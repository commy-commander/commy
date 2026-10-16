#![allow(deprecated)] // iroh 0.28 networking layer, as elsewhere here
//! Wake-ups for resting devices. A device that rests leaves every
//! server's gossip mesh but keeps its relay link, so it stays reachable by
//! node id. A member who sends a message dials it on the `commy/wake/1`
//! ALPN and writes one byte; the device then rejoins the mesh and catches
//! up with that member. The wake names no server and carries nothing
//! else: the receiver works out which servers it shares with the sender
//! from the sender's node id, which the QUIC handshake authenticates. The
//! relay sees a few encrypted bytes between two nodes, as it does for any
//! other peer traffic.

use crate::node::NetEvent;
use anyhow::{Context, Result};
use iroh::net::endpoint::Connecting;
use iroh::router::ProtocolHandler;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub const WAKE_ALPN: &[u8] = b"commy/wake/1";

/// The whole wake, and the receiver's reply to it.
pub const WAKE_BYTE: u8 = 1;

/// How long a wake may take end to end, dial included.
pub const WAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// One wake per sending node in this window reaches the app; the rest
/// are answered and dropped.
pub const WAKE_SPACING: Duration = Duration::from_secs(10);

/// Senders remembered for [`WAKE_SPACING`], so a flood of node ids cannot
/// grow the table without bound.
const MAX_RECENT: usize = 4096;

/// Accepts wakes and passes each sending node to the app as
/// [`NetEvent::Wake`]. Whether the sender is anyone we know is the app's
/// call; here we only keep one node from waking us more than once per
/// [`WAKE_SPACING`].
#[derive(Debug)]
pub(crate) struct WakeAcceptor {
    pub(crate) events: mpsc::UnboundedSender<NetEvent>,
    pub(crate) recent: Mutex<HashMap<[u8; 32], Instant>>,
}

impl WakeAcceptor {
    pub(crate) fn new(events: mpsc::UnboundedSender<NetEvent>) -> Self {
        Self { events, recent: Mutex::new(HashMap::new()) }
    }

    /// True when a wake from `node` should reach the app now.
    fn admit(&self, node: [u8; 32], now: Instant) -> bool {
        let mut recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
        if recent.get(&node).is_some_and(|at| now.duration_since(*at) < WAKE_SPACING) {
            return false;
        }
        if recent.len() >= MAX_RECENT {
            recent.retain(|_, at| now.duration_since(*at) < WAKE_SPACING);
            if recent.len() >= MAX_RECENT {
                return false;
            }
        }
        recent.insert(node, now);
        true
    }
}

// iroh 0.28 marks its whole networking layer deprecated in favour of a
// renamed crate; this is the same API the other acceptors use.
#[allow(deprecated)]
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
            // The sender hangs up once it has our reply.
            let _ = tokio::time::timeout(Duration::from_secs(5), send.stopped()).await;
            Ok(())
        })
    }
}

/// Wakes a node: dials it (through `relay` when given, else however
/// discovery finds it), writes the wake byte and waits for the reply.
#[allow(deprecated)] // iroh 0.28 networking layer, as elsewhere here
pub(crate) async fn send(
    endpoint: iroh::net::Endpoint,
    node: iroh::net::NodeId,
    relay: Option<iroh::net::relay::RelayUrl>,
) -> Result<()> {
    let mut addr = iroh::net::NodeAddr::new(node);
    if let Some(relay) = relay {
        addr = addr.with_relay_url(relay);
    }
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
        for i in 0..MAX_RECENT {
            let mut node = [0u8; 32];
            node[..8].copy_from_slice(&(i as u64).to_be_bytes());
            assert!(acceptor.admit(node, now));
        }
        assert!(!acceptor.admit([0xff; 32], now), "full of fresh senders: refused");
        assert!(acceptor.admit([0xff; 32], now + WAKE_SPACING), "old entries make room");
        assert!(acceptor.recent.lock().unwrap().len() <= 1);
    }
}
