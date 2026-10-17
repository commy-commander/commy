use crate::catchup::{
    admit, refuse, respond, run_initiator, CatchupError, CatchupStore, Refusal, SessionReport,
    CATCHUP_ALPN, HANDSHAKE_TIMEOUT,
};
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
use std::collections::{BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum NetEvent {
    Event {
        server: ServerId,
        sender: [u8; 32],
        event: ServerEvent,
    },
    SyncRequested { server: ServerId, sender: [u8; 32] },
    SyncReceived {
        server: ServerId,
        payload: WirePayload,
    },
    VoiceDial {
        server: ServerId,
        payload: WirePayload,
    },
    NeighborUp { server: ServerId, peer: [u8; 32] },
    CaughtUp {
        server: ServerId,
        peer: [u8; 32],
        report: SessionReport,
    },
    Wake { from: [u8; 32] },
}

pub type Connection = iroh::net::endpoint::Connection;

#[derive(Debug)]
pub struct MediaChannel {
    pub peer: [u8; 32],
    pub hello: MediaHello,
    pub incoming: mpsc::UnboundedReceiver<MediaPacket>,
    pub outgoing: mpsc::UnboundedSender<MediaPacket>,
    pub connection: iroh::net::endpoint::Connection,
}

impl MediaChannel {
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

const RELAYED_VIDEO_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

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

#[derive(Debug, Clone, PartialEq)]
pub struct IncomingDm {
    pub envelope: DmEnvelope,
    pub node: [u8; 32],
}

#[derive(Debug)]
struct DmAcceptor {
    tx: mpsc::UnboundedSender<IncomingDm>,
}

#[allow(deprecated)]
impl ProtocolHandler for DmAcceptor {
    fn accept(
        self: Arc<Self>,
        connecting: Connecting,
    ) -> futures_lite::future::Boxed<Result<()>> {
        let tx = self.tx.clone();
        Box::pin(async move {
            let connection = connecting.await?;
            let node = *iroh::net::endpoint::get_remote_node_id(&connection)?.as_bytes();
            let (mut send, mut recv) = connection.accept_bi().await?;
            while let Some(envelope) = read_frame::<_, DmEnvelope>(&mut recv)
                .await
                .map_err(|e| anyhow::anyhow!("bad dm frame: {e}"))?
            {
                let accepted = envelope.sent_from(&node) && tx.send(IncomingDm { envelope, node }).is_ok();
                write_frame(&mut send, &DmAck { accepted })
                    .await
                    .map_err(|e| anyhow::anyhow!("dm ack failed: {e}"))?;
            }
            Ok(())
        })
    }
}

pub type SharedCatchupStore = Arc<dyn CatchupStore + Send + Sync>;

type StoreSlot = Arc<RwLock<Option<SharedCatchupStore>>>;

const MAX_CATCHUP_SESSIONS: usize = 4;

const CATCHUP_SESSION_TIMEOUT: Duration = Duration::from_secs(300);

const CATCHUP_LINGER: Duration = Duration::from_secs(10);

const MAX_HANDSHAKES_PER_NODE: usize = 4;

struct Inbound {
    sessions: Arc<tokio::sync::Semaphore>,
    handshakes: Mutex<HashMap<[u8; 32], usize>>,
    running: Mutex<HashSet<([u8; 32], ServerId)>>,
    handshake: Duration,
}

impl Inbound {
    fn new(sessions: usize, handshake: Duration) -> Self {
        Self {
            sessions: Arc::new(tokio::sync::Semaphore::new(sessions)),
            handshakes: Mutex::default(),
            running: Mutex::default(),
            handshake,
        }
    }

    async fn serve<IO, S>(
        &self,
        mut io: IO,
        store: Option<&S>,
        local: [u8; 32],
        peer: [u8; 32],
    ) -> Result<(ServerId, SessionReport), CatchupError>
    where
        IO: AsyncRead + AsyncWrite + Unpin,
        S: CatchupStore + ?Sized,
    {
        let Some(waiting) = self.begin_handshake(peer) else {
            return Err(refuse(&mut io, Refusal::Busy).await);
        };
        let Some(store) = store else {
            return Err(refuse(&mut io, Refusal::Busy).await);
        };
        let admitted = tokio::time::timeout(self.handshake, admit(&mut io, store, local, peer))
            .await
            .map_err(|_| CatchupError::Timeout)??;
        drop(waiting);
        let Some(_running) = self.begin_session(peer, admitted.server) else {
            return Err(refuse(&mut io, Refusal::Busy).await);
        };
        let Ok(_permit) = self.sessions.clone().try_acquire_owned() else {
            return Err(refuse(&mut io, Refusal::Busy).await);
        };
        let session = respond(io, store, admitted, local, peer);
        tokio::time::timeout(CATCHUP_SESSION_TIMEOUT, session)
            .await
            .map_err(|_| CatchupError::Timeout)?
    }

    fn begin_handshake(&self, peer: [u8; 32]) -> Option<Handshake<'_>> {
        let mut waiting = self.handshakes.lock().unwrap_or_else(|e| e.into_inner());
        let count = waiting.entry(peer).or_default();
        if *count >= MAX_HANDSHAKES_PER_NODE {
            return None;
        }
        *count += 1;
        Some(Handshake {
            waiting: &self.handshakes,
            peer,
        })
    }

    fn begin_session(&self, peer: [u8; 32], server: ServerId) -> Option<Running<'_>> {
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        if !running.insert((peer, server)) {
            return None;
        }
        Some(Running {
            running: &self.running,
            key: (peer, server),
        })
    }
}

struct Handshake<'a> {
    waiting: &'a Mutex<HashMap<[u8; 32], usize>>,
    peer: [u8; 32],
}

impl Drop for Handshake<'_> {
    fn drop(&mut self) {
        let mut waiting = self.waiting.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = waiting.get_mut(&self.peer) {
            *count -= 1;
            if *count == 0 {
                waiting.remove(&self.peer);
            }
        }
    }
}

struct Running<'a> {
    running: &'a Mutex<HashSet<([u8; 32], ServerId)>>,
    key: ([u8; 32], ServerId),
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        running.remove(&self.key);
    }
}

struct CatchupAcceptor {
    store: StoreSlot,
    events: mpsc::UnboundedSender<NetEvent>,
    local: [u8; 32],
    inbound: Inbound,
}

impl std::fmt::Debug for CatchupAcceptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatchupAcceptor").finish_non_exhaustive()
    }
}

#[allow(deprecated)]
impl ProtocolHandler for CatchupAcceptor {
    fn accept(
        self: Arc<Self>,
        connecting: Connecting,
    ) -> futures_lite::future::Boxed<Result<()>> {
        Box::pin(async move {
            let connection = connecting.await?;
            let peer = *iroh::net::endpoint::get_remote_node_id(&connection)?.as_bytes();
            let Ok(opened) =
                tokio::time::timeout(self.inbound.handshake, connection.accept_bi()).await
            else {
                connection.close(0u32.into(), b"timeout");
                return Ok(());
            };
            let (send, recv) = opened?;
            let mut io = tokio::io::join(recv, send);
            let store = self.store.read().unwrap_or_else(|e| e.into_inner()).clone();
            let outcome = self
                .inbound
                .serve(&mut io, store.as_deref(), self.local, peer)
                .await;
            let timed_out = matches!(outcome, Err(CatchupError::Timeout));
            match outcome {
                Ok((server, report)) => {
                    let _ = self.events.send(NetEvent::CaughtUp {
                        server,
                        peer,
                        report,
                    });
                }
                Err(e) => tracing::debug!("catch-up from peer ended: {e}"),
            }
            if timed_out {
                connection.close(0u32.into(), b"timeout");
                return Ok(());
            }
            let (_, mut send) = io.into_inner();
            send.finish().ok();
            let _ = tokio::time::timeout(CATCHUP_LINGER, connection.closed()).await;
            Ok(())
        })
    }
}

type TopicSink = Pin<Box<dyn Sink<GossipCommand, Error = anyhow::Error> + Send>>;

struct TopicHandle {
    sender: tokio::sync::Mutex<TopicSink>,
    neighbors: tokio::sync::watch::Receiver<BTreeSet<[u8; 32]>>,
    reader: tokio::task::AbortHandle,
}

impl Drop for TopicHandle {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

fn track_neighbors(neighbors: &mut BTreeSet<[u8; 32]>, event: &GossipEvent) -> Vec<[u8; 32]> {
    match event {
        GossipEvent::Joined(peers) => {
            let up: Vec<[u8; 32]> = peers.iter().map(|p| *p.as_bytes()).collect();
            neighbors.extend(up.iter().copied());
            up
        }
        GossipEvent::NeighborUp(peer) => {
            neighbors.insert(*peer.as_bytes());
            vec![*peer.as_bytes()]
        }
        GossipEvent::NeighborDown(peer) => {
            neighbors.remove(peer.as_bytes());
            vec![]
        }
        GossipEvent::Received(_) => vec![],
    }
}

pub fn topic_for(server_id: ServerId) -> [u8; 32] {
    let uuid = server_id.0.as_bytes();
    let mut topic = [0u8; 32];
    topic[..16].copy_from_slice(uuid);
    topic[16..].copy_from_slice(uuid);
    topic
}

pub struct CommyNode {
    node: iroh::node::MemNode,
    topics: HashMap<ServerId, TopicHandle>,
    events_tx: mpsc::UnboundedSender<NetEvent>,
    entry_key_requests: Option<mpsc::UnboundedReceiver<crate::entry_key::EntryKeyAsk>>,
    catchup_store: StoreSlot,
}

impl CommyNode {
    pub fn generate_secret() -> [u8; 32] {
        rand::random()
    }

    pub async fn spawn(
        secret: Option<[u8; 32]>,
    ) -> Result<(
        Self,
        mpsc::UnboundedReceiver<NetEvent>,
        mpsc::UnboundedReceiver<MediaChannel>,
        mpsc::UnboundedReceiver<IncomingDm>,
    )> {
        let relays = match crate::relays::relay_map(&crate::relays::own_relays()) {
            Ok(map) => Some(map),
            Err(e) => {
                tracing::warn!("using default relays: {e:#}");
                None
            }
        };
        Self::spawn_with(secret, relays).await
    }

    pub async fn spawn_with(
        secret: Option<[u8; 32]>,
        relays: Option<iroh::net::relay::RelayMap>,
    ) -> Result<(
        Self,
        mpsc::UnboundedReceiver<NetEvent>,
        mpsc::UnboundedReceiver<MediaChannel>,
        mpsc::UnboundedReceiver<IncomingDm>,
    )> {
        let (media_tx, media_rx) = mpsc::unbounded_channel();
        let (dm_tx, dm_rx) = mpsc::unbounded_channel();
        let (entry_key_tx, entry_key_rx) = mpsc::unbounded_channel();
        let mut builder = iroh::node::Node::memory();
        if let Some(map) = relays {
            builder = builder.relay_mode(iroh::net::relay::RelayMode::Custom(map));
        }
        if let Some(bytes) = secret {
            builder = builder.secret_key(iroh::net::key::SecretKey::from_bytes(&bytes));
        }
        let protocols = builder.build().await.context("failed to build iroh node")?;
        let endpoint = protocols.endpoint().clone();
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let catchup_store = StoreSlot::default();
        #[allow(deprecated)]
        let local = *endpoint.node_id().as_bytes();
        let catchup = CatchupAcceptor {
            store: catchup_store.clone(),
            events: events_tx.clone(),
            local,
            inbound: Inbound::new(MAX_CATCHUP_SESSIONS, HANDSHAKE_TIMEOUT),
        };
        let node = protocols
            .accept(MEDIA_ALPN.to_vec(), Arc::new(MediaAcceptor { tx: media_tx, endpoint }))
            .accept(DM_ALPN.to_vec(), Arc::new(DmAcceptor { tx: dm_tx }))
            .accept(
                crate::entry_key::ENTRY_KEY_ALPN.to_vec(),
                Arc::new(crate::entry_key::EntryKeyAcceptor { tx: entry_key_tx }),
            )
            .accept(CATCHUP_ALPN.to_vec(), Arc::new(catchup))
            .accept(crate::wake::WAKE_ALPN.to_vec(), Arc::new(crate::wake::WakeAcceptor::new(events_tx.clone())))
            .spawn()
            .await
            .context("failed to start iroh node")?;
        Ok((
            Self {
                node,
                topics: HashMap::new(),
                events_tx,
                entry_key_requests: Some(entry_key_rx),
                catchup_store,
            },
            events_rx,
            media_rx,
            dm_rx,
        ))
    }

    pub fn take_entry_key_requests(&mut self) -> Option<mpsc::UnboundedReceiver<crate::entry_key::EntryKeyAsk>> {
        self.entry_key_requests.take()
    }

    pub fn endpoint(&self) -> iroh::net::Endpoint {
        self.node.endpoint().clone()
    }

    pub fn send_dm(
        &self,
        node_hex: &str,
        envelope: &DmEnvelope,
    ) -> impl Future<Output = Result<()>> + Send + 'static {
        let endpoint = self.node.endpoint().clone();
        let node_id = parse_node_hex(node_hex);
        let envelope = envelope.clone();
        async move {
            let connection = endpoint
                .connect(NodeAddr::new(node_id?), DM_ALPN)
                .await
                .context("peer is unreachable")?;
            let (mut send, mut recv) = connection.open_bi().await?;
            write_frame(&mut send, &envelope)
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
    }

    pub fn set_catchup_store(&self, store: SharedCatchupStore) {
        *self
            .catchup_store
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(store);
    }

    #[allow(deprecated)]
    pub fn catch_up(
        &self,
        server: ServerId,
        peer: [u8; 32],
        store: SharedCatchupStore,
    ) -> impl Future<Output = Result<SessionReport>> + Send + 'static {
        let endpoint = self.node.endpoint().clone();
        let local = self.node_id_bytes();
        async move {
            let (me, identity) = store
                .local_identity(server)
                .context("not a member of that server")?;
            let node_id = NodeId::from_bytes(&peer).context("invalid node key")?;
            let connection = endpoint
                .connect(NodeAddr::new(node_id), CATCHUP_ALPN)
                .await
                .context("peer is unreachable")?;
            let (send, recv) = connection.open_bi().await?;
            let mut io = tokio::io::join(recv, send);
            let session = run_initiator(&mut io, &*store, &identity, server, me, local, peer);
            let result = tokio::time::timeout(CATCHUP_SESSION_TIMEOUT, session).await;
            let (_, mut send) = io.into_inner();
            if send.finish().is_ok() {
                let _ = tokio::time::timeout(CATCHUP_LINGER, send.stopped()).await;
            }
            connection.close(0u32.into(), b"done");
            let report = result.context("catch-up timed out")??;
            Ok(report)
        }
    }

    pub fn node_id_bytes(&self) -> [u8; 32] {
        *self.node.node_id().as_bytes()
    }

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

    pub fn node_id_hex(&self) -> String {
        data_encoding::HEXLOWER.encode(&self.node_id_bytes())
    }

    pub async fn node_addr(&self) -> Result<NodeAddr> {
        self.node.net().node_addr().await.map_err(Into::into)
    }

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
                if !neighbors.borrow().is_empty() {
                    return;
                }
                if neighbors.changed().await.is_err() {
                    return;
                }
            }
        })
        .await
        .is_ok()
            && !handle.neighbors.borrow().is_empty()
    }

    pub fn neighbor_count(&self, server_id: ServerId) -> usize {
        self.topics
            .get(&server_id)
            .map(|h| h.neighbors.borrow().len())
            .unwrap_or(0)
    }

    pub fn neighbors(&self, server_id: ServerId) -> Vec<[u8; 32]> {
        self.topics
            .get(&server_id)
            .map(|h| h.neighbors.borrow().iter().copied().collect())
            .unwrap_or_default()
    }

    async fn join_topic(
        &mut self,
        server_id: ServerId,
        topic_bytes: [u8; 32],
        bootstrap: Vec<NodeId>,
    ) -> Result<()> {
        let topic = TopicId::from_bytes(topic_bytes);
        let gossip = self
            .node
            .get_protocol::<iroh_gossip::net::Gossip>(iroh_gossip::net::GOSSIP_ALPN)
            .context("gossip is not running")?;
        let (commands, updates) = async_channel::bounded::<GossipCommand>(64);
        let stream = gossip.join_with_stream(
            topic,
            iroh_gossip::net::JoinOptions::with_bootstrap(bootstrap),
            Box::pin(updates),
        );
        let sink = futures_util::sink::unfold(commands, |commands, command: GossipCommand| async move {
            commands.send(command).await.map_err(|_| anyhow::anyhow!("gossip is gone"))?;
            anyhow::Ok(commands)
        });

        let events_tx = self.events_tx.clone();
        let our_id = self.node_id_bytes();
        let (neighbors_tx, neighbors_rx) = tokio::sync::watch::channel(BTreeSet::new());
        let reader = tokio::spawn(async move {
            let mut stream = Box::pin(stream);
            while let Some(item) = stream.next().await {
                let Ok(GossipNetEvent::Gossip(event)) = item else {
                    continue;
                };
                let msg = match event {
                    GossipEvent::Received(msg) => msg,
                    membership => {
                        let mut up = Vec::new();
                        neighbors_tx.send_modify(|set| up = track_neighbors(set, &membership));
                        for peer in up {
                            let _ = events_tx.send(NetEvent::NeighborUp {
                                server: server_id,
                                peer,
                            });
                        }
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
                reader: reader.abort_handle(),
            },
        );
        Ok(())
    }

    pub async fn join_peers(&self, server_id: ServerId, peers_hex: &[String]) -> Result<usize> {
        let handle = self
            .topics
            .get(&server_id)
            .context("not joined to that server")?;
        let our_id = self.node_id_bytes();
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

    pub async fn broadcast(&self, server_id: ServerId, payload: WirePayload) -> Result<()> {
        let handle = self
            .topics
            .get(&server_id)
            .context("not joined to that server")?;
        let bytes = Envelope::new(self.node_id_bytes(), payload).encode()?;
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

    pub async fn fetch_blob(&self, hash: &str, provider_hex: Option<&str>) -> Result<Vec<u8>> {
        let hash: iroh_blobs::Hash = hash.parse().context("invalid blob hash")?;
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

    pub fn home_relay(&self) -> Option<String> {
        self.node.endpoint().home_relay().map(|r| r.to_string())
    }

    pub fn is_joined(&self, server_id: ServerId) -> bool {
        self.topics.contains_key(&server_id)
    }

    pub fn rest(&mut self) -> Vec<ServerId> {
        self.topics.drain().map(|(id, _)| id).collect()
    }

    pub async fn rejoin(&mut self, server_id: ServerId, peers_hex: &[String]) -> Result<()> {
        if self.is_joined(server_id) {
            return Ok(());
        }
        let our_id = self.node_id_bytes();
        let relay = self.node.endpoint().home_relay();
        let mut ids = Vec::new();
        for hex in peers_hex {
            let Ok(id) = parse_node_hex(hex) else { continue };
            if *id.as_bytes() == our_id {
                continue;
            }
            let mut addr = NodeAddr::new(id);
            if let Some(relay) = relay.clone() {
                addr = addr.with_relay_url(relay);
            }
            self.node.net().add_node_addr(addr).await.ok();
            ids.push(id);
        }
        self.join_topic(server_id, topic_for(server_id), ids).await
    }

    pub fn wake(&self, node_hex: &str) -> impl Future<Output = Result<()>> + Send + 'static {
        let endpoint = self.node.endpoint().clone();
        let relay = endpoint.home_relay();
        let node = parse_node_hex(node_hex);
        async move { crate::wake::send(endpoint, node?, relay).await }
    }

    pub async fn shutdown(self) -> Result<()> {
        self.node.shutdown().await.map_err(Into::into)
    }
}

pub(crate) fn parse_node_hex(hex: &str) -> Result<NodeId> {
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

    fn peer() -> NodeId {
        iroh::net::key::SecretKey::generate().public()
    }

    #[test]
    fn neighbor_set_follows_gossip_membership() {
        let (a, b, c) = (peer(), peer(), peer());
        let mut set = BTreeSet::new();
        let up = track_neighbors(&mut set, &GossipEvent::Joined(vec![a, b]));
        assert_eq!(
            up,
            vec![*a.as_bytes(), *b.as_bytes()],
            "each joined peer is announced"
        );
        assert_eq!(set.len(), 2);
        assert_eq!(
            track_neighbors(&mut set, &GossipEvent::NeighborUp(c)),
            vec![*c.as_bytes()]
        );
        assert_eq!(set.len(), 3);
        assert_eq!(
            track_neighbors(&mut set, &GossipEvent::NeighborUp(c)),
            vec![*c.as_bytes()]
        );
        assert_eq!(set.len(), 3);
        assert!(track_neighbors(&mut set, &GossipEvent::NeighborDown(a)).is_empty());
        assert_eq!(set, BTreeSet::from([*b.as_bytes(), *c.as_bytes()]));
        assert!(track_neighbors(&mut set, &GossipEvent::NeighborDown(a)).is_empty());
        assert_eq!(set.len(), 2);
    }

    fn members() -> (
        crate::test_support::MemStore,
        crate::test_support::MemStore,
        crate::test_support::MemStore,
    ) {
        use crate::test_support::{fixture, join, MemStore};
        let (mut state, channel, owner) = fixture();
        let friend = join(&mut state, "friend");
        let other = join(&mut state, "other");
        let host = MemStore::new(state.clone(), owner);
        host.post(channel, "while you were away");
        (
            host,
            MemStore::new(state.clone(), friend),
            MemStore::new(state, other),
        )
    }

    const HOST: [u8; 32] = [1; 32];
    const GUEST: [u8; 32] = [2; 32];
    const OTHER: [u8; 32] = [3; 32];

    async fn session_through<S: CatchupStore>(
        inbound: &Inbound,
        host: &S,
        guest: &crate::test_support::MemStore,
        node: [u8; 32],
    ) -> (
        std::result::Result<SessionReport, CatchupError>,
        std::result::Result<(ServerId, SessionReport), CatchupError>,
    ) {
        let (x, y) = tokio::io::duplex(8 * 1024);
        let server = guest.server();
        tokio::join!(
            run_initiator(x, guest, &guest.identity, server, guest.me, node, HOST),
            inbound.serve(y, Some(host), HOST, node),
        )
    }

    fn waiting(inbound: &Inbound, node: [u8; 32]) -> usize {
        inbound.handshakes.lock().unwrap().get(&node).copied().unwrap_or(0)
    }

    async fn until_waiting(inbound: &Inbound, node: [u8; 32], count: usize) {
        while waiting(inbound, node) < count {
            tokio::task::yield_now().await;
        }
    }

    async fn until_running(inbound: &Inbound, node: [u8; 32], server: ServerId) {
        while !inbound.running.lock().unwrap().contains(&(node, server)) {
            tokio::task::yield_now().await;
        }
    }

    async fn say_hello(x: &mut tokio::io::DuplexStream, guest: &crate::test_support::MemStore) {
        use crate::catchup::{CatchupFrame, HelloRole};
        let hello = CatchupFrame::hello(
            &guest.identity,
            HelloRole::Initiator,
            guest.server(),
            guest.me,
            GUEST,
            HOST,
            chrono::Utc::now().timestamp_millis(),
        );
        write_frame(x, &hello).await.unwrap();
    }

    #[tokio::test]
    async fn a_node_may_have_a_few_hellos_waiting() {
        use crate::catchup::CatchupFrame;
        let (host, guest, _) = members();
        let inbound = Inbound::new(MAX_CATCHUP_SESSIONS, HANDSHAKE_TIMEOUT);

        let (mut quiet, mut held) = (Vec::new(), Vec::new());
        for _ in 0..MAX_HANDSHAKES_PER_NODE {
            let (x, y) = tokio::io::duplex(1024);
            quiet.push(x);
            held.push(inbound.serve(y, Some(&host), HOST, GUEST));
        }
        let (mut x, extra) = tokio::io::duplex(1024);
        let again = async {
            until_waiting(&inbound, GUEST, MAX_HANDSHAKES_PER_NODE).await;
            let result = inbound.serve(extra, Some(&host), HOST, GUEST).await;
            let reply: Option<CatchupFrame> = read_frame(&mut x).await.unwrap();
            quiet.clear();
            (result, reply)
        };
        let (held, (again, reply)) = tokio::join!(futures_util::future::join_all(held), again);
        assert!(matches!(
            again.unwrap_err(),
            CatchupError::Rejected(Refusal::Busy)
        ));
        assert_eq!(reply, Some(Refusal::Busy.frame()));
        for outcome in held {
            assert!(matches!(outcome.unwrap_err(), CatchupError::Closed));
        }
        assert_eq!(waiting(&inbound, GUEST), 0);

        let (rg, rh) = session_through(&inbound, &host, &guest, GUEST).await;
        assert_eq!(rg.unwrap().received.len(), 1);
        rh.unwrap();
    }

    #[tokio::test]
    async fn one_session_per_node_and_server() {
        use crate::catchup::RETRY_AFTER_MS;
        use crate::test_support::MultiStore;
        let (first, guest, _) = members();
        let (second, guest_elsewhere, _) = members();
        let server = guest.server();
        let host = MultiStore(vec![first, second]);
        let inbound = Inbound::new(MAX_CATCHUP_SESSIONS, HANDSHAKE_TIMEOUT);

        let (mut x, y) = tokio::io::duplex(8 * 1024);
        say_hello(&mut x, &guest).await;
        let held = inbound.serve(y, Some(&host), HOST, GUEST);
        let next = async {
            until_running(&inbound, GUEST, server).await;
            let same = session_through(&inbound, &host, &guest, GUEST).await;
            let other = session_through(&inbound, &host, &guest_elsewhere, GUEST).await;
            drop(x);
            (same, other)
        };
        let (held, ((rs, rh), (ro, rho))) = tokio::join!(held, next);
        assert_eq!(rs.unwrap_err().retry_after_ms(), Some(RETRY_AFTER_MS));
        assert!(matches!(
            rh.unwrap_err(),
            CatchupError::Rejected(Refusal::Busy)
        ));
        assert_eq!(ro.unwrap().received.len(), 1);
        assert_eq!(rho.unwrap().0, guest_elsewhere.server());
        assert!(matches!(held.unwrap_err(), CatchupError::Closed));
        assert!(inbound.running.lock().unwrap().is_empty());

        let (rg, rh) = session_through(&inbound, &host, &guest, GUEST).await;
        assert_eq!(rg.unwrap().received.len(), 1);
        rh.unwrap();
    }

    #[tokio::test]
    async fn parallel_sessions_for_several_servers_from_one_node_all_run() {
        use crate::test_support::MultiStore;
        let (hosts, guests): (Vec<_>, Vec<_>) = (0..3)
            .map(|_| {
                let (host, guest, _) = members();
                (host, guest)
            })
            .unzip();
        let host = MultiStore(hosts);
        let inbound = Inbound::new(MAX_CATCHUP_SESSIONS, HANDSHAKE_TIMEOUT);
        let sessions = guests
            .iter()
            .map(|guest| session_through(&inbound, &host, guest, GUEST));
        let outcomes = futures_util::future::join_all(sessions).await;
        for ((rg, rh), guest) in outcomes.into_iter().zip(&guests) {
            assert_eq!(rg.unwrap().received.len(), 1);
            assert_eq!(rh.unwrap().0, guest.server());
        }
        assert!(host.0.iter().zip(&guests).all(|(h, g)| h.digests() == g.digests()));
    }

    #[tokio::test]
    async fn silent_peer_holds_no_session_slot_and_is_let_go() {
        let (host, guest, _) = members();
        let inbound = Inbound::new(1, Duration::from_millis(200));
        let started = tokio::time::Instant::now();
        let (silent, quiet) = tokio::io::duplex(1024);
        let stalled = inbound.serve(quiet, Some(&host), HOST, OTHER);
        let member = async {
            until_waiting(&inbound, OTHER, 1).await;
            session_through(&inbound, &host, &guest, GUEST).await
        };
        let (stalled, (rg, rh)) = tokio::join!(stalled, member);
        assert_eq!(rg.unwrap().received.len(), 1);
        rh.unwrap();
        assert!(matches!(stalled.unwrap_err(), CatchupError::Timeout));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(waiting(&inbound, OTHER), 0);
        drop(silent);
    }

    #[tokio::test]
    async fn members_past_the_session_limit_are_asked_to_retry() {
        use crate::catchup::RETRY_AFTER_MS;
        let (host, guest, other) = members();
        let inbound = Inbound::new(1, HANDSHAKE_TIMEOUT);
        let (mut x, first) = tokio::io::duplex(8 * 1024);
        say_hello(&mut x, &guest).await;
        let held = inbound.serve(first, Some(&host), HOST, GUEST);
        let next = async {
            while inbound.sessions.available_permits() > 0 {
                tokio::task::yield_now().await;
            }
            let outcome = session_through(&inbound, &host, &other, OTHER).await;
            drop(x);
            outcome
        };
        let (held, (ro, rh)) = tokio::join!(held, next);
        assert_eq!(ro.unwrap_err().retry_after_ms(), Some(RETRY_AFTER_MS));
        assert!(matches!(
            rh.unwrap_err(),
            CatchupError::Rejected(Refusal::Busy)
        ));
        assert!(matches!(held.unwrap_err(), CatchupError::Closed));
    }

    #[tokio::test]
    async fn catch_up_runs_over_quic() {
        use crate::catchup::{CatchupError, RETRY_AFTER_MS};
        use crate::test_support::{fixture, join, MemStore};
        let within = Duration::from_secs;
        let (a, _a_events, _, _) = CommyNode::spawn(None).await.unwrap();
        let (b, mut b_events, _, _) = CommyNode::spawn(None).await.unwrap();
        let addr = tokio::time::timeout(within(30), b.node_addr())
            .await
            .unwrap()
            .unwrap();
        a.node.net().add_node_addr(addr).await.unwrap();

        let (mut state, channel, owner) = fixture();
        let friend = join(&mut state, "friend");
        let store_a = Arc::new(MemStore::new(state.clone(), owner));
        let store_b = Arc::new(MemStore::new(state, friend));
        let from_a = store_a.post(channel, "from a");
        let from_b = store_b.post(channel, "from b");
        let server = store_a.server();

        let early = tokio::time::timeout(
            within(30),
            a.catch_up(server, b.node_id_bytes(), store_a.clone()),
        )
        .await
        .unwrap()
        .unwrap_err();
        let early = early
            .downcast_ref::<CatchupError>()
            .expect("a catch-up refusal");
        assert_eq!(early.retry_after_ms(), Some(RETRY_AFTER_MS));
        assert!(!store_b.has(from_a.id));

        b.set_catchup_store(store_b.clone());
        let report = tokio::time::timeout(
            within(30),
            a.catch_up(server, b.node_id_bytes(), store_a.clone()),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(report.delivered, vec![from_a.id]);
        assert_eq!(report.received, vec![from_b.id]);
        assert!(store_a.has(from_b.id) && store_b.has(from_a.id));

        let event = tokio::time::timeout(within(10), b_events.recv())
            .await
            .unwrap()
            .unwrap();
        let NetEvent::CaughtUp {
            server: got,
            peer,
            report,
        } = event
        else {
            panic!("unexpected event {event:?}");
        };
        assert_eq!((got, peer), (server, a.node_id_bytes()));
        assert_eq!(report.delivered, vec![from_b.id]);
        assert_eq!(report.received, vec![from_a.id]);
        a.shutdown().await.ok();
        b.shutdown().await.ok();
    }

    #[tokio::test]
    async fn incoming_dms_carry_the_sending_node() {
        use crate::test_support::keyed_profile;
        use commy_core::{DmMessage, UserId};
        let within = Duration::from_secs;
        let (a, _, _, _) = CommyNode::spawn(None).await.unwrap();
        let (b, _, _, mut b_dms) = CommyNode::spawn(None).await.unwrap();
        let addr = tokio::time::timeout(within(30), b.node_addr())
            .await
            .unwrap()
            .unwrap();
        a.node.net().add_node_addr(addr).await.unwrap();

        let mut sender = keyed_profile("sender");
        sender.node = Some(data_encoding::HEXLOWER.encode(&[9u8; 32]));
        let mut message = DmMessage::new(sender.id, UserId::new(), "hi");
        message.from_node = a.node_id_hex();
        let envelope = DmEnvelope { message, sender };
        tokio::time::timeout(within(30), a.send_dm(&b.node_id_hex(), &envelope))
            .await
            .unwrap()
            .unwrap();
        let got = tokio::time::timeout(within(10), b_dms.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.envelope, envelope);
        assert_eq!(got.node, a.node_id_bytes());
        a.shutdown().await.ok();
        b.shutdown().await.ok();
    }

    #[tokio::test]
    async fn a_dm_sent_on_from_another_node_is_refused() {
        use crate::test_support::{identity_for, keyed_profile};
        use commy_core::{DmMessage, UserId};
        let within = Duration::from_secs;
        let (a, _, _, _) = CommyNode::spawn(None).await.unwrap();
        let (relay, _, _, _) = CommyNode::spawn(None).await.unwrap();
        let (b, _, _, mut b_dms) = CommyNode::spawn(None).await.unwrap();
        let addr = tokio::time::timeout(within(30), b.node_addr())
            .await
            .unwrap()
            .unwrap();
        a.node.net().add_node_addr(addr.clone()).await.unwrap();
        relay.node.net().add_node_addr(addr).await.unwrap();

        let sender = keyed_profile("sender");
        let mut message = DmMessage::new(sender.id, UserId::new(), "for you only");
        message.sign(&a.node_id_hex(), &identity_for(sender.id));
        let envelope = DmEnvelope { message, sender };
        assert!(envelope.signed_by_sender());

        let replay = tokio::time::timeout(within(30), relay.send_dm(&b.node_id_hex(), &envelope))
            .await
            .unwrap();
        assert!(replay.is_err(), "refused");
        tokio::time::timeout(within(30), a.send_dm(&b.node_id_hex(), &envelope))
            .await
            .unwrap()
            .unwrap();
        let got = tokio::time::timeout(within(10), b_dms.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!((got.envelope, got.node), (envelope, a.node_id_bytes()));
        assert!(b_dms.try_recv().is_err(), "only the direct copy arrived");
        a.shutdown().await.ok();
        relay.shutdown().await.ok();
        b.shutdown().await.ok();
    }

    #[test]
    fn topic_is_stable_per_server() {
        let a = ServerId::new();
        let b = ServerId::new();
        assert_eq!(topic_for(a), topic_for(a));
        assert_ne!(topic_for(a), topic_for(b));
    }
}

fn rand_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    nanos ^ COUNTER.fetch_add(1, Ordering::Relaxed).rotate_left(32)
}
