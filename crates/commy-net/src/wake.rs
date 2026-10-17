#![allow(deprecated, reason = "iroh 0.28 deprecates its networking layer in favour of a renamed crate")]

use crate::node::NetEvent;
use anyhow::{Context, Result};
use iroh::net::endpoint::Connecting;
use iroh::router::ProtocolHandler;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub const WAKE_ALPN: &[u8] = b"commy/wake/1";

pub const WAKE_BYTE: u8 = 1;

pub const WAKE_TIMEOUT: Duration = Duration::from_secs(15);

pub const WAKE_SPACING: Duration = Duration::from_secs(10);

const MAX_REMEMBERED_SENDERS: usize = 4096;

#[derive(Debug)]
pub(crate) struct WakeAcceptor {
    pub(crate) events: mpsc::UnboundedSender<NetEvent>,
    pub(crate) recent: Mutex<HashMap<[u8; 32], Instant>>,
}

impl WakeAcceptor {
    pub(crate) fn new(events: mpsc::UnboundedSender<NetEvent>) -> Self {
        Self { events, recent: Mutex::new(HashMap::new()) }
    }

    fn admit(&self, node: [u8; 32], now: Instant) -> bool {
        let mut recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
        if recent.get(&node).is_some_and(|at| now.duration_since(*at) < WAKE_SPACING) {
            return false;
        }
        if recent.len() >= MAX_REMEMBERED_SENDERS {
            recent.retain(|_, at| now.duration_since(*at) < WAKE_SPACING);
            if recent.len() >= MAX_REMEMBERED_SENDERS {
                return false;
            }
        }
        recent.insert(node, now);
        true
    }
}

impl ProtocolHandler for WakeAcceptor {
    fn accept(self: Arc<Self>, connecting: Connecting) -> futures_lite::future::Boxed<Result<()>> {
        Box::pin(async move {
            let connection = connecting.await?;
            let node = *iroh::net::endpoint::get_remote_node_id(&connection)?.as_bytes();
            let read = async {
                let (mut send, mut recv) = connection.accept_bi().await?;
                let mut byte = [0u8; 1];
                recv.read_exact(&mut byte).await?;
                anyhow::ensure!(byte[0] == WAKE_BYTE, "unknown wake");
                send.write_all(&[WAKE_BYTE]).await?;
                send.finish()?;
                anyhow::Ok(send)
            };
            let mut send = tokio::time::timeout(WAKE_TIMEOUT, read).await.context("wake timed out")??;
            if self.admit(node, Instant::now()) {
                let _ = self.events.send(NetEvent::Wake { from: node });
            }
            let _ = tokio::time::timeout(Duration::from_secs(5), send.stopped()).await;
            Ok(())
        })
    }
}

pub(crate) async fn send(
    endpoint: iroh::net::Endpoint,
    node: iroh::net::NodeId,
    relay: Option<iroh::net::relay::RelayUrl>,
) -> Result<()> {
    let addr = crate::node::addr_with_relay(node, relay);
    let wake = async {
        let connection = endpoint.connect(addr, WAKE_ALPN).await.context("device is unreachable")?;
        let (mut send, mut recv) = connection.open_bi().await?;
        send.write_all(&[WAKE_BYTE]).await?;
        send.finish()?;
        let mut byte = [0u8; 1];
        recv.read_exact(&mut byte).await.context("device did not answer the wake")?;
        connection.close(0u32.into(), b"done");
        anyhow::ensure!(byte[0] == WAKE_BYTE, "unexpected wake reply");
        anyhow::Ok(())
    };
    tokio::time::timeout(WAKE_TIMEOUT, wake).await.context("wake timed out")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_wake_per_node_per_window() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let acceptor = WakeAcceptor::new(tx);
        let now = Instant::now();
        assert!(acceptor.admit([1; 32], now));
        assert!(!acceptor.admit([1; 32], now + Duration::from_secs(3)));
        assert!(acceptor.admit([2; 32], now + Duration::from_secs(3)), "other nodes are not held back");
        assert!(acceptor.admit([1; 32], now + WAKE_SPACING));
    }

    #[test]
    fn the_sender_table_stays_bounded() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let acceptor = WakeAcceptor::new(tx);
        let now = Instant::now();
        for i in 0..MAX_REMEMBERED_SENDERS {
            let mut node = [0u8; 32];
            node[..8].copy_from_slice(&(i as u64).to_be_bytes());
            assert!(acceptor.admit(node, now));
        }
        assert!(!acceptor.admit([0xff; 32], now), "full of fresh senders: refused");
        assert!(acceptor.admit([0xff; 32], now + WAKE_SPACING), "old entries make room");
        assert!(acceptor.recent.lock().unwrap().len() <= 1);
    }
}
