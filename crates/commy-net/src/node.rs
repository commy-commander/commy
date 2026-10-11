use crate::dm::{DmAck, DmEnvelope, DM_ALPN};
use crate::media::{read_frame, write_frame, MediaHello, MediaPacket, MEDIA_ALPN};
use crate::protocol::{Envelope, WirePayload};
use crate::ticket::ServerTicket;
use anyhow::{Context, Result};
use commy_core::{ServerEvent, ServerId};
use futures_lite::StreamExt;
use futures_util::{Sink, SinkExt};
use iroh::net::endpoint::Connecting;
use iroh::net::{NodeAddr, NodeId};
use iroh::router::ProtocolHandler;
use iroh_gossip::net::{Command as GossipCommand, Event as GossipNetEvent, GossipEvent};
use iroh_gossip::proto::TopicId;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Events surfaced to the application layer from the gossip plane.
#[derive(Debug)]
pub enum NetEvent {
    /// A server event arrived from a peer and should be applied.
    Event {
        server: ServerId,
        sender: [u8; 32],
        event: ServerEvent,
    },
    /// A newcomer asked for state; the app should reply with a snapshot.
    SyncRequested { server: ServerId, sender: [u8; 32] },
    /// A peer sent us a state snapshot.
    SyncReceived {
        server: ServerId,
        payload: WirePayload,
    },
    /// A peer wants to start a voice session with us.
    VoiceDial {
        server: ServerId,
        payload: WirePayload,
    },
    /// A new gossip neighbor connected on a server topic. Peers use
    /// this to re-announce themselves and request missed state.
    NeighborUp { server: ServerId },
}

/// A peer QUIC connection, re-exported for link statistics.
pub type Connection = iroh::net::endpoint::Connection;

/// One live media stream (voice or screen share) with a peer,
/// abstracted to packet channels so the app never touches QUIC.
#[derive(Debug)]
pub struct MediaChannel {
    pub peer: [u8; 32],
    pub hello: MediaHello,
    pub incoming: mpsc::UnboundedReceiver<MediaPacket>,
    pub outgoing: mpsc::UnboundedSender<MediaPacket>,
    /// The QUIC connection, for link statistics.
    pub connection: iroh::net::endpoint::Connection,
}

impl MediaChannel {
    /// Spawns reader and writer pumps over an open bi stream.
    fn spawn(
        endpoint: iroh::net::Endpoint,
        peer: [u8; 32],
        hello: MediaHello,
        connection: iroh::net::endpoint::Connection,
        mut send: iroh::net::endpoint::SendStream,
        mut recv: iroh::net::endpoint::RecvStream,
    ) -> Self {
        let (in_tx, in_rx) = mpsc::unbounded_channel();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<MediaPacket>();

        let stats_handle = connection.clone();
        tokio::spawn(async move {
            // Holding the connection keeps it alive for both pumps.
            let _connection = connection;
            let reader = async {
                loop {
                    match read_frame::<_, MediaPacket>(&mut recv).await {
                        Ok(Some(packet)) => {
                            if in_tx.send(packet).is_err() {
                                break;
                            }
                        }
                        Ok(None) => break,
                        Err(e) => {
                            tracing::debug!("media read ended: {e}");
                            break;
                        }
                    }
                }
            };
            let writer = async {
                let mut pace = VideoPacer::default();
                while let Some(mut packet) = out_rx.recv().await {
                    // On a slow link, send only the newest video frame
                    // instead of falling further behind. Audio is never skipped.
                    let mut after = None;
                    if matches!(packet, MediaPacket::Video { .. }) {
                        while let Ok(next) = out_rx.try_recv() {
                            if matches!(next, MediaPacket::Video { .. }) {
                                packet = next;
                            } else {
                                after = Some(next);
                                break;
                            }
                        }
                    }
                    if matches!(packet, MediaPacket::Video { .. }) && !pace.allow(&endpoint, peer) {
                        if let Some(next) = after {
                            if let Err(e) = write_frame(&mut send, &next).await {
                                tracing::debug!("media write ended: {e}");
                                break;
                            }
                        }
                        continue;
                    }
                    let mut failed = write_frame(&mut send, &packet).await.err();
                    if failed.is_none() {
                        if let Some(next) = after {
                            failed = write_frame(&mut send, &next).await.err();
                        }
                    }
                    if let Some(e) = failed {
                        tracing::debug!("media write ended: {e}");
                        break;
                    }
                }
            };
            tokio::join!(reader, writer);
        });

        Self {
            peer,
            hello,
            incoming: in_rx,
            outgoing: out_tx,
            connection: stats_handle,
        }
    }
}

/// Relays carry everyone's traffic with limited bandwidth; a full-rate
/// screen share through one starves gossip and voice. Video to a peer we
/// only reach via a relay is held to this frame interval.
const RELAYED_VIDEO_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// Decides per video frame whether to send it, based on how the peer is
/// reached. The path is rechecked every couple of seconds since iroh
/// upgrades relayed connections to direct ones once hole punching works.
#[derive(Default)]
struct VideoPacer {
    relayed: Option<bool>,
    checked: Option<std::time::Instant>,
    last_sent: Option<std::time::Instant>,
}

impl VideoPacer {
    fn allow(&mut self, endpoint: &iroh::net::Endpoint, peer: [u8; 32]) -> bool {
        let now = std::time::Instant::now();
        if self.checked.map_or(true, |t| now.duration_since(t) > std::time::Duration::from_secs(2)) {
            self.checked = Some(now);
            let relayed = iroh::net::NodeId::from_bytes(&peer)
                .ok()
                .and_then(|id| endpoint.remote_info(id))
                .map_or(false, |info| !matches!(info.conn_type, iroh::net::endpoint::ConnectionType::Direct(_)));
            if self.relayed != Some(relayed) {
                tracing::info!("video to peer is {}", if relayed { "relayed, pacing to 10 fps" } else { "direct, full rate" });
                self.relayed = Some(relayed);
            }
        }
        if self.relayed == Some(true) && self.last_sent.is_some_and(|t| now.duration_since(t) < RELAYED_VIDEO_INTERVAL) {
            return false;
        }
        self.last_sent = Some(now);
        true
    }
}

/// Accepts inbound media connections and hands them to the app.
#[derive(Debug)]
struct MediaAcceptor {
    tx: mpsc::UnboundedSender<MediaChannel>,
    endpoint: iroh::net::Endpoint,
}

impl ProtocolHandler for MediaAcceptor {
    fn accept(
        self: Arc<Self>,
        connecting: Connecting,
    ) -> futures_lite::future::Boxed<Result<()>> {
        let tx = self.tx.clone();
        Box::pin(async move {
            let connection = connecting.await?;
            let peer = iroh::net::endpoint::get_remote_node_id(&connection)?;
            let (send, mut recv) = connection.accept_bi().await?;
            let hello: MediaHello = read_frame(&mut recv)
                .await
                .map_err(|e| anyhow::anyhow!("bad media hello: {e}"))?
                .context("peer closed before hello")?;
            let channel = MediaChannel::spawn(self.endpoint.clone(), *peer.as_bytes(), hello, connection, send, recv);
            tx.send(channel).ok();
            Ok(())
        })
    }
}

/// Accepts inbound DM connections: reads envelopes, acks each one,
/// and forwards them to the app.
#[derive(Debug)]
struct DmAcceptor {
    tx: mpsc::UnboundedSender<DmEnvelope>,
}

impl ProtocolHandler for DmAcceptor {
    fn accept(
        self: Arc<Self>,
        connecting: Connecting,
    ) -> futures_lite::future::Boxed<Result<()>> {
        let tx = self.tx.clone();
        Box::pin(async move {
            let connection = connecting.await?;
            let (mut send, mut recv) = connection.accept_bi().await?;
            while let Some(envelope) = read_frame::<_, DmEnvelope>(&mut recv)
                .await
                .map_err(|e| anyhow::anyhow!("bad dm frame: {e}"))?
            {
                let accepted = tx.send(envelope).is_ok();
                write_frame(&mut send, &DmAck { accepted })
                    .await
                    .map_err(|e| anyhow::anyhow!("dm ack failed: {e}"))?;
            }
            Ok(())
        })
    }
}

type TopicSink = Pin<Box<dyn Sink<GossipCommand, Error = anyhow::Error> + Send>>;

struct TopicHandle {
    sender: tokio::sync::Mutex<TopicSink>,
    /// Live count of direct gossip neighbors on this topic.
    neighbors: tokio::sync::watch::Receiver<usize>,
}

/// Stable topic id derived from the server id, so invites keep working
/// across host restarts. The id is already shared with every invitee.
pub fn topic_for(server_id: ServerId) -> [u8; 32] {
    let uuid = server_id.0.as_bytes();
    let mut topic = [0u8; 32];
    topic[..16].copy_from_slice(uuid);
    topic[16..].copy_from_slice(uuid);
    topic
}

/// Wraps an in-process iroh node: one gossip topic per joined server,
/// a blob store for file transfer, and a media ALPN for voice and
/// screen share connections.
pub struct CommyNode {
    node: iroh::node::MemNode,
    topics: HashMap<ServerId, TopicHandle>,
    events_tx: mpsc::UnboundedSender<NetEvent>,
}

impl CommyNode {
    /// Generates a fresh node secret key for first launch.
    pub fn generate_secret() -> [u8; 32] {
        rand::random()
    }

    /// Spawns an iroh node with default n0 discovery and relays.
    /// Pass the persisted secret key so the node keeps the same
    /// identity across restarts; a fresh key invalidates every invite
    /// and DM address that peers have stored.
    /// Returns gossip events, inbound media channels, and inbound DMs.
    pub async fn spawn(
        secret: Option<[u8; 32]>,
    ) -> Result<(
        Self,
        mpsc::UnboundedReceiver<NetEvent>,
        mpsc::UnboundedReceiver<MediaChannel>,
        mpsc::UnboundedReceiver<DmEnvelope>,
    )> {
        let (media_tx, media_rx) = mpsc::unbounded_channel();
        let (dm_tx, dm_rx) = mpsc::unbounded_channel();
        let mut builder = iroh::node::Node::memory();
        if let Some(bytes) = secret {
            builder = builder.secret_key(iroh::net::key::SecretKey::from_bytes(&bytes));
        }
        let protocols = builder.build().await.context("failed to build iroh node")?;
        let endpoint = protocols.endpoint().clone();
        let node = protocols
            .accept(MEDIA_ALPN.to_vec(), Arc::new(MediaAcceptor { tx: media_tx, endpoint }))
            .accept(DM_ALPN.to_vec(), Arc::new(DmAcceptor { tx: dm_tx }))
            .spawn()
            .await
            .context("failed to start iroh node")?;
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        Ok((
            Self {
                node,
                topics: HashMap::new(),
                events_tx,
            },
            events_rx,
            media_rx,
            dm_rx,
        ))
    }

    /// Delivers one DM to a peer, waiting for its ack. Fails when the
    /// peer is offline or unreachable.
    pub async fn send_dm(&self, node_hex: &str, envelope: &DmEnvelope) -> Result<()> {
        let node_id = parse_node_hex(node_hex)?;
        let connection = self
            .node
            .endpoint()
            .connect(NodeAddr::new(node_id), DM_ALPN)
            .await
            .context("peer is unreachable")?;
        let (mut send, mut recv) = connection.open_bi().await?;
        write_frame(&mut send, envelope)
            .await
            .map_err(|e| anyhow::anyhow!("dm send failed: {e}"))?;
        let ack = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            read_frame::<_, DmAck>(&mut recv),
        )
        .await
        .context("peer did not confirm delivery in time")?
        .map_err(|e| anyhow::anyhow!("dm ack failed: {e}"))?
        .context("peer closed before confirming")?;
        if !ack.accepted {
            anyhow::bail!("peer rejected the message");
        }
        connection.close(0u32.into(), b"done");
        Ok(())
    }

    /// This peer's public identity.
    pub fn node_id_bytes(&self) -> [u8; 32] {
        *self.node.node_id().as_bytes()
    }

    /// How we reach a peer: ("direct" | "relay" | "mixed" | "none", relay
    /// url, direct address).
    pub fn peer_path(&self, node_hex: &str) -> Option<(String, Option<String>, Option<String>)> {
        use iroh::net::endpoint::ConnectionType;
        let node_id = parse_node_hex(node_hex).ok()?;
        let info = self.node.endpoint().remote_info(node_id)?;
        let relay = info.relay_url.map(|r| r.relay_url.to_string());
        Some(match info.conn_type {
            ConnectionType::Direct(addr) => ("direct".into(), relay, Some(addr.to_string())),
            ConnectionType::Relay(url) => ("relay".into(), Some(url.to_string()), None),
            ConnectionType::Mixed(addr, url) => ("mixed".into(), Some(url.to_string()), Some(addr.to_string())),
            ConnectionType::None => ("none".into(), relay, None),
        })
    }

    /// Hex form of our node id, shared in profiles for media dialing.
    pub fn node_id_hex(&self) -> String {
        data_encoding::HEXLOWER.encode(&self.node_id_bytes())
    }

    /// Our dialable address, embedded into invites we create.
    pub async fn node_addr(&self) -> Result<NodeAddr> {
        self.node.net().node_addr().await.map_err(Into::into)
    }

    /// Creates the gossip topic for a brand new server and returns an
    /// invite ticket other people can use to join.
    pub async fn host_server(
        &mut self,
        server_id: ServerId,
        server_name: &str,
    ) -> Result<ServerTicket> {
        let topic_bytes = topic_for(server_id);
        let addr = self.node_addr().await?;
        let ticket = ServerTicket::new(server_id, server_name, topic_bytes, vec![addr]);
        self.join_topic(server_id, topic_bytes, vec![]).await?;
        Ok(ticket)
    }

    /// Joins an existing server from an invite. Waits for the gossip
    /// mesh to form before requesting state sync, because broadcasts
    /// sent with zero neighbors are silently lost. Returns true when a
    /// peer was reached; false means we joined the topic but nobody
    /// was online yet (we announce again on NeighborUp).
    pub async fn join_server(&mut self, ticket: &ServerTicket) -> Result<bool> {
        let bootstrap: Vec<NodeId> = ticket.bootstrap.iter().map(|a| a.node_id).collect();
        for addr in &ticket.bootstrap {
            self.node.net().add_node_addr(addr.clone()).await.ok();
        }
        self.join_topic(ticket.server_id, ticket.topic, bootstrap)
            .await?;
        let connected = self
            .wait_for_peers(ticket.server_id, std::time::Duration::from_secs(20))
            .await;
        if connected {
            self.broadcast(ticket.server_id, WirePayload::SyncRequest)
                .await?;
        }
        Ok(connected)
    }

    /// Waits until at least one gossip neighbor is up on the topic.
    pub async fn wait_for_peers(
        &self,
        server_id: ServerId,
        timeout: std::time::Duration,
    ) -> bool {
        let Some(handle) = self.topics.get(&server_id) else {
            return false;
        };
        let mut neighbors = handle.neighbors.clone();
        tokio::time::timeout(timeout, async {
            loop {
                if *neighbors.borrow() > 0 {
                    return;
                }
                if neighbors.changed().await.is_err() {
                    return;
                }
            }
        })
        .await
        .is_ok()
            && *handle.neighbors.borrow() > 0
    }

    /// Current neighbor count for a joined server topic.
    pub fn neighbor_count(&self, server_id: ServerId) -> usize {
        self.topics
            .get(&server_id)
            .map(|h| *h.neighbors.borrow())
            .unwrap_or(0)
    }

    async fn join_topic(
        &mut self,
        server_id: ServerId,
        topic_bytes: [u8; 32],
        bootstrap: Vec<NodeId>,
    ) -> Result<()> {
        let topic = TopicId::from_bytes(topic_bytes);
        let (sink, stream) = self
            .node
            .gossip()
            .subscribe(topic, bootstrap)
            .await
            .context("failed to subscribe to server topic")?;

        let events_tx = self.events_tx.clone();
        let our_id = self.node_id_bytes();
        let (neighbors_tx, neighbors_rx) = tokio::sync::watch::channel(0usize);
        tokio::spawn(async move {
            let mut stream = Box::pin(stream);
            let mut count: usize = 0;
            while let Some(item) = stream.next().await {
                let Ok(GossipNetEvent::Gossip(event)) = item else {
                    continue;
                };
                let msg = match event {
                    GossipEvent::Received(msg) => msg,
                    GossipEvent::Joined(peers) => {
                        count = peers.len();
                        let _ = neighbors_tx.send(count);
                        let _ = events_tx.send(NetEvent::NeighborUp { server: server_id });
                        continue;
                    }
                    GossipEvent::NeighborUp(_) => {
                        count += 1;
                        let _ = neighbors_tx.send(count);
                        let _ = events_tx.send(NetEvent::NeighborUp { server: server_id });
                        continue;
                    }
                    GossipEvent::NeighborDown(_) => {
                        count = count.saturating_sub(1);
                        let _ = neighbors_tx.send(count);
                        continue;
                    }
                };
                let Ok(envelope) = Envelope::decode(&msg.content) else {
                    tracing::warn!("dropping undecodable frame");
                    continue;
                };
                if envelope.sender == our_id {
                    continue;
                }
                let net_event = match envelope.payload {
                    WirePayload::Event(event) => NetEvent::Event {
                        server: server_id,
                        sender: envelope.sender,
                        event,
                    },
                    WirePayload::SyncRequest => NetEvent::SyncRequested {
                        server: server_id,
                        sender: envelope.sender,
                    },
                    payload @ WirePayload::SyncResponse { .. } => NetEvent::SyncReceived {
                        server: server_id,
                        payload,
                    },
                    payload @ WirePayload::VoiceDial { .. } => NetEvent::VoiceDial {
                        server: server_id,
                        payload,
                    },
                };
                if events_tx.send(net_event).is_err() {
                    break;
                }
            }
        });

        self.topics.insert(
            server_id,
            TopicHandle {
                sender: tokio::sync::Mutex::new(Box::pin(sink)),
                neighbors: neighbors_rx,
            },
        );
        Ok(())
    }

    /// Asks gossip to dial these members on a server topic we already
    /// joined. Used to heal the mesh when both sides restarted and the
    /// startup dial reached nobody.
    pub async fn join_peers(&self, server_id: ServerId, peers_hex: &[String]) -> Result<usize> {
        let handle = self
            .topics
            .get(&server_id)
            .context("not joined to that server")?;
        let our_id = self.node_id_bytes();
        // Peers usually share our home relay (same region), and dialing
        // through it works even when no direct address is known. Without
        // any route hint a bare id dial times out.
        let relay_hint = self
            .node_addr()
            .await
            .ok()
            .and_then(|a| a.relay_url().cloned());
        let mut ids = Vec::new();
        for hex in peers_hex {
            let Ok(id) = parse_node_hex(hex) else { continue };
            if *id.as_bytes() == our_id {
                continue;
            }
            let mut addr = NodeAddr::new(id);
            if let Some(relay) = relay_hint.clone() {
                addr = addr.with_relay_url(relay);
            }
            self.node.net().add_node_addr(addr).await.ok();
            ids.push(id);
        }
        if ids.is_empty() {
            return Ok(0);
        }
        let count = ids.len();
        let mut sink = handle.sender.lock().await;
        sink.send(GossipCommand::JoinPeers(ids)).await?;
        Ok(count)
    }

    /// Broadcasts a payload to every member of a server.
    pub async fn broadcast(&self, server_id: ServerId, payload: WirePayload) -> Result<()> {
        let handle = self
            .topics
            .get(&server_id)
            .context("not joined to that server")?;
        let bytes = Envelope::new(self.node_id_bytes(), payload).encode()?;
        // An oversized frame makes peers drop the connection outright,
        // so refuse it here instead of breaking the mesh.
        anyhow::ensure!(
            bytes.len() <= crate::protocol::MAX_GOSSIP_BYTES,
            "message is too large to send ({} bytes, limit {})",
            bytes.len(),
            crate::protocol::MAX_GOSSIP_BYTES
        );
        let mut sink = handle.sender.lock().await;
        sink.send(GossipCommand::Broadcast(bytes.into())).await?;
        Ok(())
    }

    /// Opens a direct media connection (voice or screen) to a peer
    /// identified by its hex node id.
    pub async fn connect_media(&self, node_hex: &str, hello: MediaHello) -> Result<MediaChannel> {
        let node_id = parse_node_hex(node_hex)?;
        let connection = self
            .node
            .endpoint()
            .connect(NodeAddr::new(node_id), MEDIA_ALPN)
            .await
            .context("media dial failed")?;
        let (mut send, recv) = connection.open_bi().await?;
        write_frame(&mut send, &hello)
            .await
            .map_err(|e| anyhow::anyhow!("sending hello failed: {e}"))?;
        Ok(MediaChannel::spawn(
            self.node.endpoint().clone(),
            *node_id.as_bytes(),
            hello,
            connection,
            send,
            recv,
        ))
    }

    /// Stores a file in the local blob store, returning its hash for
    /// embedding in a message attachment.
    ///
    /// Goes through a temp file on purpose: `add_bytes` uses a bidi
    /// streaming RPC whose server side (quic-rpc 0.14) polls a finished
    /// oneshot, which current tokio turns into a panic that kills the
    /// iroh node and with it all networking. `add_from_path` is plain
    /// server streaming and does not hit that path.
    pub async fn add_blob(&self, bytes: Vec<u8>) -> Result<String> {
        let path = std::env::temp_dir().join(format!(
            "commy-blob-{}-{}.bin",
            std::process::id(),
            rand_suffix()
        ));
        tokio::fs::write(&path, &bytes).await.context("could not stage blob")?;
        let added = async {
            self.node
                .blobs()
                .add_from_path(
                    path.clone(),
                    false,
                    iroh_blobs::util::SetTagOption::Auto,
                    iroh::client::blobs::WrapOption::NoWrap,
                )
                .await?
                .finish()
                .await
        }
        .await;
        let _ = tokio::fs::remove_file(&path).await;
        Ok(added?.hash.to_string())
    }

    /// Fetches a blob by hash, trying the given provider first.
    pub async fn fetch_blob(&self, hash: &str, provider_hex: Option<&str>) -> Result<Vec<u8>> {
        let hash: iroh_blobs::Hash = hash.parse().context("invalid blob hash")?;
        // Serve from the local store when we already have it.
        if let Ok(bytes) = self.node.blobs().read_to_bytes(hash).await {
            return Ok(bytes.to_vec());
        }
        let provider = provider_hex.context("blob not local and no provider known")?;
        let node_id = parse_node_hex(provider)?;
        self.node
            .blobs()
            .download(hash, NodeAddr::new(node_id))
            .await?
            .await
            .context("blob download failed")?;
        let bytes = self.node.blobs().read_to_bytes(hash).await?;
        Ok(bytes.to_vec())
    }

    pub fn leave_server(&mut self, server_id: ServerId) {
        self.topics.remove(&server_id);
    }

    pub async fn shutdown(self) -> Result<()> {
        self.node.shutdown().await.map_err(Into::into)
    }
}

fn parse_node_hex(hex: &str) -> Result<NodeId> {
    let bytes = data_encoding::HEXLOWER
        .decode(hex.trim().as_bytes())
        .context("node id is not valid hex")?;
    let bytes: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .context("node id must be 32 bytes")?;
    NodeId::from_bytes(&bytes).context("invalid node key")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_hex_roundtrip() {
        let key = iroh::net::key::SecretKey::generate().public();
        let hex = data_encoding::HEXLOWER.encode(key.as_bytes());
        let parsed = parse_node_hex(&hex).unwrap();
        assert_eq!(parsed, key);
    }

    #[test]
    fn bad_node_hex_is_rejected() {
        assert!(parse_node_hex("zz").is_err());
        assert!(parse_node_hex("abcd").is_err());
    }

    #[test]
    fn topic_is_stable_per_server() {
        let a = ServerId::new();
        let b = ServerId::new();
        assert_eq!(topic_for(a), topic_for(a));
        assert_ne!(topic_for(a), topic_for(b));
    }
}

/// Unique-enough suffix for staging files within one process.
fn rand_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    nanos ^ COUNTER.fetch_add(1, Ordering::Relaxed).rotate_left(32)
}
