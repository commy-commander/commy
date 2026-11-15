use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use commy_sync::wire::{ClientFrame, ServerFrame};
use commy_sync::{StreamKeys, StreamSecret};
use futures_util::{SinkExt, StreamExt};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::CertificateDer;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

const WAIT: Duration = Duration::from_secs(20);
const PROBE: &[u8] = b"commy sync check";

#[derive(Debug, PartialEq, Eq)]
pub struct Checked {
    pub protocol: u32,
    pub seq: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    #[error("the address must start with wss:// or ws://")]
    BadUrl,
    #[error("could not read the certificate file {0}: {1}")]
    Roots(String, String),
    #[error("could not connect: {0}")]
    Connect(String),
    #[error("the sync server did not answer within {} seconds", WAIT.as_secs())]
    Timeout,
    #[error("the sync server closed the connection")]
    Closed,
    #[error("the sync server refused the check: {0}")]
    Refused(String),
    #[error("the entry read back did not match the one written")]
    Mismatch,
}

pub async fn run(url: &str, extra_roots: Option<&Path>) -> Result<Checked, CheckError> {
    if !url.starts_with("wss://") && !url.starts_with("ws://") {
        return Err(CheckError::BadUrl);
    }
    let connector = connector(extra_roots)?;
    let keys = StreamSecret::random().keys();

    let mut writer = Link::connect(url, connector.clone()).await?;
    let protocol = writer.hello().await?;
    writer.open(&keys).await?;
    let sealed = keys.seal(PROBE).map_err(|e| CheckError::Refused(e.to_string()))?;
    writer.send(&ClientFrame::append(&keys, rand::random(), sealed)).await?;
    let seq = writer
        .wait_for(|frame| match frame {
            ServerFrame::Appended { stream, seq, .. } if *stream == keys.id => Some(*seq),
            _ => None,
        })
        .await?;
    writer.close().await;

    let mut reader = Link::connect(url, connector).await?;
    reader.hello().await?;
    reader.open(&keys).await?;
    let body = reader
        .wait_for(|frame| match frame {
            ServerFrame::Entries { stream, entries, .. } if *stream == keys.id => {
                entries.iter().find(|entry| entry.seq == seq).map(|entry| entry.body.0.clone())
            }
            _ => None,
        })
        .await?;
    reader.close().await;

    match keys.open(&body) {
        Ok(plain) if plain == PROBE => Ok(Checked { protocol, seq }),
        _ => Err(CheckError::Mismatch),
    }
}

fn connector(extra_roots: Option<&Path>) -> Result<Connector, CheckError> {
    let mut roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    if let Some(path) = extra_roots {
        let shown = path.display().to_string();
        let certs = CertificateDer::pem_file_iter(path)
            .map_err(|e| CheckError::Roots(shown.clone(), e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CheckError::Roots(shown.clone(), e.to_string()))?;
        if certs.is_empty() {
            return Err(CheckError::Roots(shown, "no certificates in it".into()));
        }
        for cert in certs {
            roots.add(cert).map_err(|e| CheckError::Roots(shown.clone(), e.to_string()))?;
        }
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| CheckError::Connect(e.to_string()))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Connector::Rustls(Arc::new(config)))
}

struct Link(WebSocketStream<MaybeTlsStream<TcpStream>>);

impl Link {
    async fn connect(url: &str, connector: Connector) -> Result<Link, CheckError> {
        let tls = url.starts_with("wss://").then_some(connector);
        let attempt = tokio_tungstenite::connect_async_tls_with_config(url, None, false, tls);
        let (socket, _) = tokio::time::timeout(WAIT, attempt)
            .await
            .map_err(|_| CheckError::Timeout)?
            .map_err(|e| CheckError::Connect(e.to_string()))?;
        Ok(Link(socket))
    }

    async fn hello(&mut self) -> Result<u32, CheckError> {
        self.wait_for(|frame| match frame {
            ServerFrame::Hello { protocol } => Some(*protocol),
            _ => None,
        })
        .await
    }

    async fn open(&mut self, keys: &StreamKeys) -> Result<(), CheckError> {
        self.send(&ClientFrame::open(keys, 0)).await?;
        self.wait_for(|frame| matches!(frame, ServerFrame::Opened { stream, .. } if *stream == keys.id).then_some(()))
            .await
    }

    async fn send(&mut self, frame: &ClientFrame) -> Result<(), CheckError> {
        let text = serde_json::to_string(frame).map_err(|e| CheckError::Refused(e.to_string()))?;
        self.0.send(Message::Text(text.into())).await.map_err(|_| CheckError::Closed)
    }

    async fn wait_for<T>(&mut self, mut wanted: impl FnMut(&ServerFrame) -> Option<T>) -> Result<T, CheckError> {
        let waiting = async {
            loop {
                let frame = match self.0.next().await {
                    Some(Ok(Message::Text(text))) => serde_json::from_str::<ServerFrame>(&text).ok(),
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return Err(CheckError::Closed),
                    Some(Ok(_)) => None,
                };
                let Some(frame) = frame else { continue };
                if let ServerFrame::Error { code, .. } = &frame {
                    return Err(CheckError::Refused(format!("{code:?}")));
                }
                if let Some(found) = wanted(&frame) {
                    return Ok(found);
                }
            }
        };
        tokio::time::timeout(WAIT, waiting).await.map_err(|_| CheckError::Timeout)?
    }

    async fn close(&mut self) {
        let _ = self.0.close(None).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::store::Store;
    use crate::{serve, Service};

    async fn local_service() -> String {
        let config = Config::from_env(|_| None).unwrap();
        let service = Service::new(Store::memory(), &config);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(serve(listener, service));
        format!("ws://{address}/sync")
    }

    #[tokio::test]
    async fn writes_an_entry_and_reads_it_back() {
        let url = local_service().await;
        let checked = run(&url, None).await.unwrap();
        assert_eq!(checked.protocol, commy_sync::wire::PROTOCOL);
        assert!(checked.seq >= 1);
    }

    #[tokio::test]
    async fn refuses_addresses_that_are_not_websockets() {
        assert!(matches!(run("https://example.org/sync", None).await, Err(CheckError::BadUrl)));
    }

    #[tokio::test]
    async fn reports_an_unreadable_certificate_file() {
        let missing = Path::new("/nonexistent/root.crt");
        assert!(matches!(run("wss://example.org/sync", Some(missing)).await, Err(CheckError::Roots(..))));
    }

    #[tokio::test]
    async fn says_when_nothing_answers() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("ws://{}/sync", listener.local_addr().unwrap());
        drop(listener);
        assert!(matches!(run(&url, None).await, Err(CheckError::Connect(_))));
    }
}
