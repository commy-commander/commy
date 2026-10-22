use std::sync::Arc;

use n0_future::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

use super::CONNECT_TIMEOUT;
use crate::wire::MAX_FRAME_BYTES;

pub enum Incoming {
    Text(String),
    Other,
    Closed,
}

pub struct Socket(tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>);

impl Socket {
    pub async fn connect(url: &str) -> Result<Socket, String> {
        let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME_BYTES))
            .max_frame_size(Some(MAX_FRAME_BYTES));
        let connector = url.starts_with("wss://").then(tls);
        let attempt = tokio_tungstenite::connect_async_tls_with_config(url, Some(config), false, connector);
        let (socket, _) = n0_future::time::timeout(CONNECT_TIMEOUT, attempt)
            .await
            .map_err(|_| "timed out".to_string())?
            .map_err(|e| e.to_string())?;
        Ok(Socket(socket))
    }

    pub async fn send(&mut self, text: String) -> Result<(), ()> {
        self.0.send(Message::Text(text.into())).await.map_err(|_| ())
    }

    pub async fn next(&mut self) -> Incoming {
        match self.0.next().await {
            Some(Ok(Message::Text(text))) => Incoming::Text(text.to_string()),
            Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Binary(_) | Message::Frame(_))) => Incoming::Other,
            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => Incoming::Closed,
        }
    }

    pub async fn close(&mut self) {
        let _ = self.0.close(None).await;
    }
}

fn tls() -> tokio_tungstenite::Connector {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    tokio_tungstenite::Connector::Rustls(Arc::new(config))
}
