use std::sync::Arc;

use anyhow::{Context, Result};
use iroh::net::endpoint::Connecting;
use iroh::router::ProtocolHandler;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

use crate::media::{read_frame, write_frame};
use iroh::net::NodeAddr;

pub const ENTRY_KEY_ALPN: &[u8] = b"commy/entry-key/1";

pub const ANSWER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

const REPLY_DELIVERY_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryKeyRequest {
    pub server: String,
    pub account: String,
    pub device_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EntryKeyReply {
    Key(String),
    Denied,
    Unknown,
}

#[derive(Debug)]
pub struct EntryKeyAsk {
    pub peer: [u8; 32],
    pub request: EntryKeyRequest,
    pub reply: oneshot::Sender<EntryKeyReply>,
}

#[derive(Debug)]
pub(crate) struct EntryKeyAcceptor {
    pub(crate) tx: mpsc::UnboundedSender<EntryKeyAsk>,
}

impl ProtocolHandler for EntryKeyAcceptor {
    fn accept(self: Arc<Self>, connecting: Connecting) -> futures_lite::future::Boxed<Result<()>> {
        let tx = self.tx.clone();
        Box::pin(async move {
            let connection = connecting.await?;
            let peer = iroh::net::endpoint::get_remote_node_id(&connection)?;
            let (mut send, mut recv) = connection.accept_bi().await?;
            let request: EntryKeyRequest = read_frame(&mut recv)
                .await
                .map_err(|e| anyhow::anyhow!("bad entry key request: {e}"))?
                .context("peer closed before asking")?;
            let (reply_tx, reply_rx) = oneshot::channel();
            let reply = if tx
                .send(EntryKeyAsk {
                    peer: *peer.as_bytes(),
                    request,
                    reply: reply_tx,
                })
                .is_ok()
            {
                tokio::time::timeout(ANSWER_TIMEOUT, reply_rx)
                    .await
                    .ok()
                    .and_then(|r| r.ok())
                    .unwrap_or(EntryKeyReply::Denied)
            } else {
                EntryKeyReply::Unknown
            };
            write_frame(&mut send, &reply)
                .await
                .map_err(|e| anyhow::anyhow!("entry key reply failed: {e}"))?;
            send.finish()?;
            let _ = tokio::time::timeout(REPLY_DELIVERY_GRACE, connection.closed()).await;
            Ok(())
        })
    }
}

pub async fn request(
    endpoint: iroh::net::Endpoint,
    node_hex: &str,
    request: &EntryKeyRequest,
) -> Result<EntryKeyReply> {
    let node_id = crate::node::parse_node_hex(node_hex)?;
    let connection = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        endpoint.connect(NodeAddr::new(node_id), ENTRY_KEY_ALPN),
    )
    .await
    .context("device did not answer in time")?
    .context("device is unreachable")?;
    let (mut send, mut recv) = connection.open_bi().await?;
    write_frame(&mut send, request)
        .await
        .map_err(|e| anyhow::anyhow!("entry key request failed: {e}"))?;
    let reply = tokio::time::timeout(
        ANSWER_TIMEOUT + std::time::Duration::from_secs(10),
        read_frame::<_, EntryKeyReply>(&mut recv),
    )
    .await
    .context("no answer in time")?
    .map_err(|e| anyhow::anyhow!("entry key reply failed: {e}"))?
    .context("device closed before answering")?;
    connection.close(0u32.into(), b"done");
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn request_and_reply_roundtrip() {
        let (mut a, mut b) = tokio::io::duplex(16 * 1024);
        let request = EntryKeyRequest {
            server: "https://commy.chat".into(),
            account: "acct".into(),
            device_name: "Laptop".into(),
        };
        write_frame(&mut a, &request).await.unwrap();
        let got: EntryKeyRequest = read_frame(&mut b).await.unwrap().unwrap();
        assert_eq!(got, request);
        write_frame(&mut b, &EntryKeyReply::Key("k".into()))
            .await
            .unwrap();
        let reply: EntryKeyReply = read_frame(&mut a).await.unwrap().unwrap();
        assert_eq!(reply, EntryKeyReply::Key("k".into()));
    }
}
