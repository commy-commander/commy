use crate::catchup::{
    admit, refuse, respond, run_initiator, CatchupError, CatchupStore, Refusal, SessionReport,
    CATCHUP_ALPN, HANDSHAKE_TIMEOUT,
};
use crate::dm::{DmAck, DmEnvelope, LegacyDmEnvelope, DM_ALPN, LEGACY_DM_ALPN};
use crate::media::{read_frame, read_frame_bytes, write_frame, MediaError, MediaHello, MediaPacket, MAX_MEDIA_FRAME, MEDIA_ALPN};
use crate::protocol::{Envelope, WirePayload};
use crate::ticket::ServerTicket;
use anyhow::{Context, Result};
use commy_core::identity::IdentityKey;
use commy_core::{ServerEvent, ServerId, UserId};
use futures_lite::StreamExt;
use iroh::address_lookup::memory::MemoryLookup;
pub use iroh::endpoint::Connection;
use iroh::endpoint::{RecvStream, SendStream};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayUrl};
use iroh_blobs::store::mem::MemStore;
use iroh_gossip::api::{Event as GossipEvent, GossipSender};
use iroh_gossip::net::Gossip;
use iroh_gossip::proto::TopicId;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::future::Future;
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
        action: Option<commy_core::Action>,
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

#[derive(Debug, Clone, PartialEq)]
pub struct LinkPath {
    pub kind: &'static str,
    pub relay: Option<String>,
    pub addr: Option<String>,
}

pub fn link_path(connection: &Connection) -> LinkPath {
    let paths = connection.paths();
    let relay = paths.iter().find_map(|p| match p.remote_addr() {
        iroh::TransportAddr::Relay(url) => Some(url.to_string()),
        _ => None,
    });
    let selected = paths.iter().find(|p| p.is_selected());
    let addr = selected.as_ref().and_then(|p| match p.remote_addr() {
        iroh::TransportAddr::Ip(addr) => Some(addr.to_string()),
        _ => None,
    });
    let kind = match &selected {
        Some(p) if p.is_relay() => "relay",
        Some(_) if paths.iter().any(|p| p.is_relay()) => "mixed",
        Some(_) => "direct",
        None => "none",
    };
    LinkPath { kind, relay, addr }
}

pub(crate) fn accept_error(e: anyhow::Error) -> AcceptError {
    AcceptError::from_boxed(e.into())
}

#[derive(Debug)]
pub struct MediaChannel {
    pub peer: [u8; 32],
    pub hello: MediaHello,
    pub incoming: mpsc::UnboundedReceiver<MediaPacket>,
    pub outgoing: mpsc::UnboundedSender<MediaPacket>,
    pub connection: Connection,
}

impl MediaChannel {
    fn spawn(
        peer: [u8; 32],
        hello: MediaHello,
        connection: Connection,
        mut send: SendStream,
        mut recv: RecvStream,
    ) -> Self {
        let (in_tx, in_rx) = mpsc::unbounded_channel();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<MediaPacket>();

        let stats_handle = connection.clone();
        n0_future::task::spawn(async move {
            let path = connection.clone();
            let _keep_connection_alive_for_both_pumps = connection;
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
                while let Some(packet) = out_rx.recv().await {
                    let (packet, after) = skip_to_newest_queued_video(packet, &mut out_rx);
                    let paced = matches!(packet, MediaPacket::Video { .. }) && !pace.allow(&path);
                    let mut failed = None;
                    for frame in (!paced).then_some(packet).into_iter().chain(after) {
                        if let Err(e) = write_frame(&mut send, &frame).await {
                            failed = Some(e);
                            break;
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

fn skip_to_newest_queued_video(
    packet: MediaPacket,
    queue: &mut mpsc::UnboundedReceiver<MediaPacket>,
) -> (MediaPacket, Option<MediaPacket>) {
    let mut newest = packet;
    if !matches!(newest, MediaPacket::Video { .. }) {
        return (newest, None);
    }
    while let Ok(next) = queue.try_recv() {
        if matches!(next, MediaPacket::Video { .. }) {
            newest = next;
        } else {
            return (newest, Some(next));
        }
    }
    (newest, None)
}

const RELAYED_VIDEO_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

const RECHECK_PATH_FOR_HOLE_PUNCH_UPGRADE: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Default)]
struct VideoPacer {
    relayed: Option<bool>,
    checked: Option<n0_future::time::Instant>,
    last_sent: Option<n0_future::time::Instant>,
}

impl VideoPacer {
    fn allow(&mut self, connection: &Connection) -> bool {
        let now = n0_future::time::Instant::now();
        if self.checked.map_or(true, |t| now.duration_since(t) > RECHECK_PATH_FOR_HOLE_PUNCH_UPGRADE) {
            self.checked = Some(now);
            let relayed = connection.paths().iter().any(|p| p.is_selected() && p.is_relay());
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

pub type MediaGate = Arc<dyn Fn([u8; 32], &MediaHello) -> bool + Send + Sync>;

type GateSlot = Arc<RwLock<Option<MediaGate>>>;

#[cfg(not(wasm_browser))]
type ShelfSlot = Arc<RwLock<Option<crate::shelf::Shelf>>>;

#[cfg(not(wasm_browser))]
#[derive(Debug, Clone)]
struct FilesAcceptor {
    shelf: ShelfSlot,
}

#[cfg(not(wasm_browser))]
impl ProtocolHandler for FilesAcceptor {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let protocol = self.shelf.read().unwrap_or_else(|e| e.into_inner()).as_ref().map(crate::shelf::Shelf::protocol);
        match protocol {
            Some(protocol) => protocol.accept(connection).await,
            None => Ok(()),
        }
    }
}

const MEDIA_GATE_WAIT: Duration = Duration::from_secs(5);

const MEDIA_GATE_POLL: Duration = Duration::from_millis(250);

struct MediaAcceptor {
    tx: mpsc::UnboundedSender<MediaChannel>,
    gate: GateSlot,
}

impl std::fmt::Debug for MediaAcceptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaAcceptor").finish_non_exhaustive()
    }
}

impl MediaAcceptor {
    async fn admitted(&self, peer: [u8; 32], hello: &MediaHello) -> bool {
        let started = n0_future::time::Instant::now();
        loop {
            let allowed = self
                .gate
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .is_some_and(|gate| gate(peer, hello));
            if allowed {
                return true;
            }
            if started.elapsed() >= MEDIA_GATE_WAIT {
                return false;
            }
            n0_future::time::sleep(MEDIA_GATE_POLL).await;
        }
    }
}

impl ProtocolHandler for MediaAcceptor {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        self.answer(connection).await.map_err(accept_error)
    }
}

impl MediaAcceptor {
    async fn answer(&self, connection: Connection) -> Result<()> {
        let peer = connection.remote_id();
        let (send, mut recv) = connection.accept_bi().await?;
        let hello: MediaHello = read_frame(&mut recv)
            .await
            .map_err(|e| anyhow::anyhow!("bad media hello: {e}"))?
            .context("peer closed before hello")?;
        if !self.admitted(*peer.as_bytes(), &hello).await {
            tracing::debug!("refused a media stream from a node that is not a member here");
            connection.close(1u32.into(), b"not a member here");
            return Ok(());
        }
        let channel = MediaChannel::spawn(*peer.as_bytes(), hello, connection, send, recv);
        self.tx.send(channel).ok();
        Ok(())
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
    legacy: bool,
}

impl ProtocolHandler for DmAcceptor {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        self.answer(connection).await.map_err(accept_error)
    }
}

impl DmAcceptor {
    async fn answer(&self, connection: Connection) -> Result<()> {
        let node = *connection.remote_id().as_bytes();
        let (mut send, mut recv) = connection.accept_bi().await?;
        loop {
            let envelope = if self.legacy {
                read_frame::<_, LegacyDmEnvelope>(&mut recv).await.map(|frame| frame.map(LegacyDmEnvelope::upgrade))
            } else {
                read_dm_frame(&mut recv).await
            };
            let Some(envelope) = envelope.map_err(|e| anyhow::anyhow!("bad dm frame: {e}"))? else {
                return Ok(());
            };
            let accepted = envelope.sent_from(&node) && self.tx.send(IncomingDm { envelope, node }).is_ok();
            write_frame(&mut send, &DmAck { accepted })
                .await
                .map_err(|e| anyhow::anyhow!("dm ack failed: {e}"))?;
        }
    }
}

async fn read_dm_frame<R: AsyncRead + Unpin>(recv: &mut R) -> Result<Option<DmEnvelope>, MediaError> {
    let Some(bytes) = read_frame_bytes(recv, MAX_MEDIA_FRAME).await? else {
        return Ok(None);
    };
    DmEnvelope::decode(&bytes).map(Some).ok_or(MediaError::Malformed)
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
        let admitted = n0_future::time::timeout(self.handshake, admit(&mut io, store, local, peer))
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
        n0_future::time::timeout(CATCHUP_SESSION_TIMEOUT, session)
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

impl ProtocolHandler for CatchupAcceptor {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        self.answer(connection).await.map_err(accept_error)
    }
}

impl CatchupAcceptor {
    async fn answer(&self, connection: Connection) -> Result<()> {
        let peer = *connection.remote_id().as_bytes();
        let Ok(opened) =
            n0_future::time::timeout(self.inbound.handshake, connection.accept_bi()).await
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
        let peer_went_quiet = matches!(outcome, Err(CatchupError::Timeout));
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
        if peer_went_quiet {
            connection.close(0u32.into(), b"timeout");
            return Ok(());
        }
        let (_, mut send) = io.into_inner();
        send.finish().ok();
        let _ = n0_future::time::timeout(CATCHUP_LINGER, connection.closed()).await;
        Ok(())
    }
}

struct TopicHandle {
    sender: GossipSender,
    neighbors: tokio::sync::watch::Receiver<BTreeSet<[u8; 32]>>,
    reader: n0_future::task::AbortHandle,
}

impl Drop for TopicHandle {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

fn track_neighbors(neighbors: &mut BTreeSet<[u8; 32]>, event: &GossipEvent) -> Vec<[u8; 32]> {
    match event {
        GossipEvent::NeighborUp(peer) => {
            neighbors.insert(*peer.as_bytes());
            vec![*peer.as_bytes()]
        }
        GossipEvent::NeighborDown(peer) => {
            neighbors.remove(peer.as_bytes());
            vec![]
        }
        GossipEvent::Received(_) | GossipEvent::Lagged => vec![],
    }
}

pub fn topic_for(server_id: ServerId) -> [u8; 32] {
    let uuid = server_id.0.as_bytes();
    let mut topic = [0u8; 32];
    topic[..16].copy_from_slice(uuid);
    topic[16..].copy_from_slice(uuid);
    topic
}

#[cfg(any(wasm_browser, test))]
pub(crate) fn reachable_from_browser(mut addr: EndpointAddr, relays: &[RelayUrl]) -> EndpointAddr {
    addr.addrs.retain(|a| matches!(a, iroh::TransportAddr::Relay(url) if relays.contains(url)));
    if addr.addrs.is_empty() {
        addr.addrs.extend(relays.iter().cloned().map(iroh::TransportAddr::Relay));
    }
    addr
}

pub(crate) fn addr_via_relays(node: EndpointId, relays: &[RelayUrl]) -> EndpointAddr {
    EndpointAddr::from_parts(node, relays.iter().cloned().map(iroh::TransportAddr::Relay))
}

fn hint_relays(lookup: &MemoryLookup, endpoint: &Endpoint, node: EndpointId, relays: &[RelayUrl]) {
    if node != endpoint.id() {
        lookup.add_endpoint_info(addr_via_relays(node, relays));
    }
}

pub const MEDIA_DIAL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct MediaDialer {
    endpoint: Endpoint,
    lookup: MemoryLookup,
    relays: Vec<RelayUrl>,
}

impl MediaDialer {
    pub async fn connect(&self, node_hex: &str, hello: MediaHello) -> Result<MediaChannel> {
        let node_id = parse_node_hex(node_hex)?;
        hint_relays(&self.lookup, &self.endpoint, node_id, &self.relays);
        let dial = async {
            let connection = self
                .endpoint
                .connect(node_id, MEDIA_ALPN)
                .await
                .context("media dial failed")?;
            let (mut send, recv) = connection.open_bi().await?;
            write_frame(&mut send, &hello)
                .await
                .map_err(|e| anyhow::anyhow!("sending hello failed: {e}"))?;
            Ok(MediaChannel::spawn(
                *node_id.as_bytes(),
                hello,
                connection,
                send,
                recv,
            ))
        };
        n0_future::time::timeout(MEDIA_DIAL_TIMEOUT, dial)
            .await
            .map_err(|_| anyhow::anyhow!("that person could not be reached"))?
    }
}

pub const MAX_BLOB_BYTES: u64 = 20 * 1024 * 1024;

pub fn blob_hash(bytes: &[u8]) -> String {
    iroh_blobs::Hash::new(bytes).to_string()
}

pub fn is_blob_of(blob: &str, bytes: &[u8]) -> bool {
    blob.parse::<iroh_blobs::Hash>().is_ok_and(|hash| hash == iroh_blobs::Hash::new(bytes))
}

pub const BLOB_UNAVAILABLE: &str = "nobody who has this file is online right now";

const BLOB_DIAL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
#[error("that file is larger than {} MB", .max / (1024 * 1024))]
pub struct BlobTooLarge {
    max: u64,
}

pub fn check_blob_size(size: u64, max: u64) -> Result<()> {
    anyhow::ensure!(size <= max, BlobTooLarge { max });
    Ok(())
}

pub fn blob_sources(provider: &str, neighbors: Vec<[u8; 32]>) -> Vec<String> {
    let mut sources: Vec<String> = Vec::new();
    for node in neighbors.iter().map(|n| data_encoding::HEXLOWER.encode(n)) {
        if !sources.contains(&node) {
            sources.push(node);
        }
    }
    match sources.iter().position(|n| n == provider) {
        Some(at) => sources.swap(0, at),
        None if !provider.is_empty() => sources.push(provider.to_string()),
        None => {}
    }
    sources
}

#[derive(Clone)]
pub struct BlobFetcher {
    blobs: MemStore,
    endpoint: Endpoint,
    lookup: MemoryLookup,
    relays: Vec<RelayUrl>,
    #[cfg(not(wasm_browser))]
    shelf: ShelfSlot,
}

pub const BIG_FILE_BYTES: u64 = 100 * 1024 * 1024;

pub const MAX_IN_MEMORY_FILE_BYTES: u64 = 512 * 1024 * 1024;

pub fn is_big_file(size: u64) -> bool {
    size > BIG_FILE_BYTES
}

#[cfg(not(wasm_browser))]
impl BlobFetcher {
    pub async fn download_to<'a>(
        &self,
        hash: &str,
        sources: impl IntoIterator<Item = &'a str>,
        max: u64,
        target: &std::path::Path,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<u64> {
        let shelf = self.shelf.read().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(shelf) = &shelf {
            if let Some(shared) = shelf.shared(hash).await {
                check_blob_size(shared.size, max)?;
                return shelf.copy_out(hash, target).await;
            }
        }
        let hash: iroh_blobs::Hash = hash.parse().context("invalid blob hash")?;
        if let Ok(iroh_blobs::api::blobs::BlobStatus::Complete { size }) = self.blobs.status(hash).await {
            check_blob_size(size, max)?;
            if let Ok(bytes) = self.blobs.get_bytes(hash).await {
                tokio::fs::write(target, &bytes).await.context("could not save the file")?;
                return Ok(size);
            }
        }
        for source in sources {
            let Ok(node_id) = parse_node_hex(source) else { continue };
            if node_id == self.endpoint.id() {
                continue;
            }
            hint_relays(&self.lookup, &self.endpoint, node_id, &self.relays);
            for alpn in [crate::shelf::FILES_ALPN, iroh_blobs::ALPN] {
                match stream_to_file(&self.endpoint, node_id, alpn, hash, max, target, &mut progress).await {
                    Ok(size) => {
                        if let Some(shelf) = &shelf {
                            if let Err(e) = shelf.share(target).await {
                                tracing::info!("downloaded file is not shared on: {e:#}");
                            }
                        }
                        return Ok(size);
                    }
                    Err(e) if e.is::<BlobTooLarge>() => return Err(e),
                    Err(e) => {
                        tracing::info!("file {} not streamed from {}: {e:#}", hash.fmt_short(), &source[..source.len().min(10)]);
                        if e.is::<Unreachable>() {
                            break;
                        }
                    }
                }
            }
        }
        anyhow::bail!(BLOB_UNAVAILABLE)
    }
}

#[cfg(not(wasm_browser))]
const PROGRESS_STEPS: u64 = 200;

#[cfg(not(wasm_browser))]
#[derive(Debug, thiserror::Error)]
#[error("that member could not be reached")]
struct Unreachable;

#[cfg(not(wasm_browser))]
async fn stream_to_file(
    endpoint: &Endpoint,
    node_id: EndpointId,
    alpn: &[u8],
    hash: iroh_blobs::Hash,
    max: u64,
    target: &std::path::Path,
    progress: &mut impl FnMut(u64, u64),
) -> Result<u64> {
    let part = crate::shelf::part_path(target);
    let streamed = stream_into(endpoint, node_id, alpn, hash, max, &part, progress).await;
    let finished = match streamed {
        Ok(size) => replace_file(&part, target).await.map(|_| size),
        Err(e) => Err(e),
    };
    if finished.is_err() {
        let _ = tokio::fs::remove_file(&part).await;
    }
    finished
}

#[cfg(not(wasm_browser))]
async fn replace_file(from: &std::path::Path, to: &std::path::Path) -> Result<()> {
    if tokio::fs::metadata(to).await.is_ok() {
        tokio::fs::remove_file(to).await.context("could not replace the existing file")?;
    }
    tokio::fs::rename(from, to).await.context("could not save the file")
}

#[cfg(not(wasm_browser))]
async fn stream_into(
    endpoint: &Endpoint,
    node_id: EndpointId,
    alpn: &[u8],
    hash: iroh_blobs::Hash,
    max: u64,
    part: &std::path::Path,
    progress: &mut impl FnMut(u64, u64),
) -> Result<u64> {
    use bao_tree::io::fsm::BaoContentItem;
    use iroh_blobs::get::fsm::{self, BlobContentNext, ConnectedNext, EndBlobNext};
    use tokio::io::{AsyncSeekExt, AsyncWriteExt};
    let connection = n0_future::time::timeout(BLOB_DIAL_TIMEOUT, endpoint.connect(node_id, alpn))
        .await
        .map_err(|_| Unreachable)?
        .context("file download failed")?;
    let request = iroh_blobs::protocol::GetRequest::blob(hash);
    let connected = fsm::start(connection, request, Default::default()).next().await?;
    let ConnectedNext::StartRoot(start) = connected.next().await? else {
        anyhow::bail!("file download failed: unexpected response");
    };
    let (mut content, size) = start.next().next().await?;
    check_blob_size(size, max)?;
    let mut file = tokio::fs::File::create(part).await.context("could not create the file")?;
    let step = (size / PROGRESS_STEPS).max(1);
    let (mut written, mut reported) = (0u64, 0u64);
    progress(0, size);
    let end = loop {
        match content.next().await {
            BlobContentNext::More((next, item)) => {
                content = next;
                if let BaoContentItem::Leaf(leaf) = item? {
                    file.seek(std::io::SeekFrom::Start(leaf.offset)).await?;
                    file.write_all(&leaf.data).await?;
                    written += leaf.data.len() as u64;
                    if written - reported >= step {
                        reported = written;
                        progress(written.min(size), size);
                    }
                }
            }
            BlobContentNext::Done(end) => break end,
        }
    };
    file.flush().await?;
    file.sync_all().await?;
    drop(file);
    let EndBlobNext::Closing(closing) = end.next() else {
        anyhow::bail!("file download failed: unexpected response");
    };
    closing.next().await?;
    progress(size, size);
    Ok(size)
}

impl BlobFetcher {
    pub async fn fetch(&self, hash: &str, provider_hex: Option<&str>, max: u64) -> Result<Vec<u8>> {
        self.fetch_from(hash, provider_hex, max).await
    }

    pub async fn fetch_from<'a>(&self, hash: &str, sources: impl IntoIterator<Item = &'a str>, max: u64) -> Result<Vec<u8>> {
        let hash: iroh_blobs::Hash = hash.parse().context("invalid blob hash")?;
        if let Ok(iroh_blobs::api::blobs::BlobStatus::Complete { size }) = self.blobs.status(hash).await {
            check_blob_size(size, max)?;
            if let Ok(bytes) = self.blobs.get_bytes(hash).await {
                return Ok(bytes.to_vec());
            }
        }
        for source in sources {
            let Ok(node_id) = parse_node_hex(source) else { continue };
            if node_id == self.endpoint.id() {
                continue;
            }
            hint_relays(&self.lookup, &self.endpoint, node_id, &self.relays);
            match fetch_capped(&self.endpoint, node_id, hash, max).await {
                Ok(bytes) => {
                    if let Err(e) = self.blobs.add_bytes(bytes.clone()).await {
                        tracing::debug!("fetched blob was not kept locally: {e}");
                    }
                    return Ok(bytes);
                }
                Err(e) if e.is::<BlobTooLarge>() => return Err(e),
                Err(e) => tracing::info!("file {} not fetched from {}: {e:#}", hash.fmt_short(), &source[..source.len().min(10)]),
            }
        }
        anyhow::bail!(BLOB_UNAVAILABLE)
    }
}

async fn fetch_capped(endpoint: &Endpoint, node_id: EndpointId, hash: iroh_blobs::Hash, max: u64) -> Result<Vec<u8>> {
    use iroh_blobs::get::fsm::{self, ConnectedNext, EndBlobNext};
    let connection = n0_future::time::timeout(BLOB_DIAL_TIMEOUT, endpoint.connect(node_id, iroh_blobs::ALPN))
        .await
        .map_err(|_| anyhow::anyhow!("blob download timed out"))?
        .context("blob download failed")?;
    let request = iroh_blobs::protocol::GetRequest::blob(hash);
    let connected = fsm::start(connection, request, Default::default()).next().await?;
    let ConnectedNext::StartRoot(start) = connected.next().await? else {
        anyhow::bail!("blob download failed: unexpected response");
    };
    let (content, size) = start.next().next().await?;
    check_blob_size(size, max)?;
    let (end, bytes) = content.concatenate_into_vec().await?;
    let EndBlobNext::Closing(closing) = end.next() else {
        anyhow::bail!("blob download failed: unexpected response");
    };
    closing.next().await?;
    Ok(bytes)
}

pub struct CommyNode {
    router: Router,
    gossip: Gossip,
    blobs: MemStore,
    #[cfg(not(wasm_browser))]
    shelf: ShelfSlot,
    lookup: MemoryLookup,
    relays: Vec<RelayUrl>,
    can_discover_address: bool,
    topics: HashMap<ServerId, TopicHandle>,
    events_tx: mpsc::UnboundedSender<NetEvent>,
    entry_key_requests: Option<mpsc::UnboundedReceiver<crate::entry_key::EntryKeyAsk>>,
    mirror_requests: Option<mpsc::UnboundedReceiver<crate::mirror::MirrorAsk>>,
    knock_requests: Option<mpsc::UnboundedReceiver<crate::knock::KnockAsk>>,
    catchup_store: StoreSlot,
    media_gate: GateSlot,
}

const ADDRESS_WAIT: Duration = Duration::from_secs(5);

const PATH_IDLE_TIMEOUT: Duration = Duration::from_secs(6);

const PATH_KEEP_ALIVE: Duration = Duration::from_secs(2);

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
        Self::spawn_on_port(secret, None).await
    }

    pub async fn spawn_on_port(
        secret: Option<[u8; 32]>,
        port: Option<u16>,
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
        let fallback = relays.as_ref().map(crate::relays::fallback_relays).unwrap_or_default();
        let spawned = Self::spawn_inner(secret, relays, port).await?;
        if !fallback.is_empty() {
            n0_future::task::spawn(crate::relays::fall_back_when_unreachable(spawned.0.endpoint(), fallback));
        }
        Ok(spawned)
    }

    pub async fn spawn_with(
        secret: Option<[u8; 32]>,
        relays: Option<iroh::RelayMap>,
    ) -> Result<(
        Self,
        mpsc::UnboundedReceiver<NetEvent>,
        mpsc::UnboundedReceiver<MediaChannel>,
        mpsc::UnboundedReceiver<IncomingDm>,
    )> {
        Self::spawn_inner(secret, relays, None).await
    }

    async fn spawn_inner(
        secret: Option<[u8; 32]>,
        relays: Option<iroh::RelayMap>,
        port: Option<u16>,
    ) -> Result<(
        Self,
        mpsc::UnboundedReceiver<NetEvent>,
        mpsc::UnboundedReceiver<MediaChannel>,
        mpsc::UnboundedReceiver<IncomingDm>,
    )> {
        let (media_tx, media_rx) = mpsc::unbounded_channel();
        let (dm_tx, dm_rx) = mpsc::unbounded_channel();
        let (entry_key_tx, entry_key_rx) = mpsc::unbounded_channel();
        let (mirror_tx, mirror_rx) = mpsc::unbounded_channel();
        let (knock_tx, knock_rx) = mpsc::unbounded_channel();
        let lookup = MemoryLookup::new();
        let relay_mode = relays.map_or_else(iroh::endpoint::default_relay_mode, iroh::RelayMode::Custom);
        let can_discover_address = relay_mode.relay_map().relays::<Vec<Arc<iroh::RelayConfig>>>().iter().any(|relay| relay.quic.is_some());
        let relays = relay_mode.relay_map().urls();
        let build = |port: Option<u16>| -> Result<iroh::endpoint::Builder> {
            #[cfg(not(wasm_browser))]
            let builder = Endpoint::builder(iroh::endpoint::presets::N0);
            #[cfg(wasm_browser)]
            let builder = Endpoint::builder(iroh::endpoint::presets::Minimal);
            let transport = iroh::endpoint::QuicTransportConfig::builder()
                .default_path_max_idle_timeout(PATH_IDLE_TIMEOUT)
                .default_path_keep_alive_interval(PATH_KEEP_ALIVE)
                .build();
            let builder = builder.address_lookup(lookup.clone()).transport_config(transport);
            let mut builder = builder.relay_mode(relay_mode.clone());
            if let Some(bytes) = secret {
                builder = builder.secret_key(iroh::SecretKey::from_bytes(&bytes));
            }
            #[cfg(not(wasm_browser))]
            if let Some(port) = port.filter(|p| *p != 0) {
                builder = builder
                    .bind_addr(std::net::SocketAddr::from((std::net::Ipv4Addr::UNSPECIFIED, port)))?
                    .bind_addr(std::net::SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, port)))?;
            }
            #[cfg(wasm_browser)]
            let _ = port;
            Ok(builder)
        };
        let endpoint = match build(port)?.bind().await {
            Ok(endpoint) => endpoint,
            Err(e) if port.is_some() => {
                tracing::warn!("could not keep the previous network port, picking a new one: {e:#}");
                build(None)?.bind().await.context("failed to build iroh node")?
            }
            Err(e) => return Err(e).context("failed to build iroh node"),
        };
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let catchup_store = StoreSlot::default();
        let media_gate = GateSlot::default();
        let local = *endpoint.id().as_bytes();
        let catchup = CatchupAcceptor {
            store: catchup_store.clone(),
            events: events_tx.clone(),
            local,
            inbound: Inbound::new(MAX_CATCHUP_SESSIONS, HANDSHAKE_TIMEOUT),
        };
        let gossip = Gossip::builder().spawn(endpoint.clone());
        let router = Router::builder(endpoint).accept(iroh_gossip::ALPN, gossip.clone());
        let blobs = MemStore::new();
        let router = router.accept(iroh_blobs::ALPN, iroh_blobs::BlobsProtocol::new(&blobs, None));
        #[cfg(not(wasm_browser))]
        let shelf = ShelfSlot::default();
        #[cfg(not(wasm_browser))]
        let router = router.accept(crate::shelf::FILES_ALPN, FilesAcceptor { shelf: shelf.clone() });
        let router = router
            .accept(MEDIA_ALPN, MediaAcceptor { tx: media_tx, gate: media_gate.clone() })
            .accept(DM_ALPN, DmAcceptor { tx: dm_tx.clone(), legacy: false })
            .accept(LEGACY_DM_ALPN, DmAcceptor { tx: dm_tx, legacy: true })
            .accept(crate::entry_key::ENTRY_KEY_ALPN, crate::entry_key::EntryKeyAcceptor { tx: entry_key_tx })
            .accept(CATCHUP_ALPN, catchup)
            .accept(crate::mirror::MIRROR_ALPN, crate::mirror::MirrorAcceptor { tx: mirror_tx, local })
            .accept(crate::wake::WAKE_ALPN, crate::wake::WakeAcceptor::new(events_tx.clone()))
            .accept(crate::knock::KNOCK_ALPN, crate::knock::KnockAcceptor { tx: knock_tx })
            .spawn();
        let _ = n0_future::time::timeout(ADDRESS_WAIT, router.endpoint().online()).await;
        Ok((
            Self {
                router,
                gossip,
                blobs,
                #[cfg(not(wasm_browser))]
                shelf,
                lookup,
                relays,
                can_discover_address,
                topics: HashMap::new(),
                events_tx,
                entry_key_requests: Some(entry_key_rx),
                mirror_requests: Some(mirror_rx),
                knock_requests: Some(knock_rx),
                catchup_store,
                media_gate,
            },
            events_rx,
            media_rx,
            dm_rx,
        ))
    }

    pub fn take_entry_key_requests(&mut self) -> Option<mpsc::UnboundedReceiver<crate::entry_key::EntryKeyAsk>> {
        self.entry_key_requests.take()
    }

    pub fn take_mirror_requests(&mut self) -> Option<mpsc::UnboundedReceiver<crate::mirror::MirrorAsk>> {
        self.mirror_requests.take()
    }

    pub fn take_knock_requests(&mut self) -> Option<mpsc::UnboundedReceiver<crate::knock::KnockAsk>> {
        self.knock_requests.take()
    }

    pub fn knock(&self, ticket: &ServerTicket, request: crate::knock::KnockRequest) -> impl Future<Output = Option<crate::knock::KnockReply>> + Send + 'static {
        let addrs: Vec<EndpointAddr> = ticket.bootstrap.iter().filter_map(|a| a.endpoint_addr()).collect();
        let nodes: Vec<EndpointId> = addrs.iter().map(|a| a.id).filter(|id| *id != self.router.endpoint().id()).collect();
        for addr in addrs {
            self.learn(addr);
        }
        let endpoint = self.endpoint();
        async move {
            let mut pending: futures_util::stream::FuturesUnordered<_> =
                nodes.into_iter().map(|node| crate::knock::knock_one(endpoint.clone(), node, request.clone())).collect();
            while let Some(reply) = futures_util::StreamExt::next(&mut pending).await {
                match reply {
                    Ok(reply) if reply.decisive() => return Some(reply),
                    Ok(_) => {}
                    Err(e) => tracing::debug!("knock went unanswered: {e:#}"),
                }
            }
            None
        }
    }

    pub fn endpoint(&self) -> Endpoint {
        self.router.endpoint().clone()
    }

    pub fn send_dm(
        &self,
        node_hex: &str,
        envelope: &DmEnvelope,
    ) -> impl Future<Output = Result<()>> + Send + 'static {
        let endpoint = self.endpoint();
        let node_id = parse_node_hex(node_hex);
        let envelope = envelope.clone();
        async move {
            let node_id = node_id?;
            let connection = match dial(&endpoint, node_id, DM_ALPN).await {
                Ok(connection) => connection,
                Err(e) => {
                    let Some(legacy) = LegacyDmEnvelope::downgrade(&envelope) else {
                        return Err(e).context("peer is unreachable");
                    };
                    let connection = dial(&endpoint, node_id, LEGACY_DM_ALPN)
                        .await
                        .context("peer is unreachable")?;
                    return confirm_dm(connection, &legacy).await;
                }
            };
            confirm_dm(connection, &envelope).await
        }
    }

    #[cfg(not(wasm_browser))]
    pub fn bound_port(&self) -> Option<u16> {
        let sockets = self.router.endpoint().bound_sockets();
        sockets
            .iter()
            .find(|addr| addr.is_ipv4())
            .or(sockets.first())
            .map(|addr| addr.port())
    }
}

const REDIAL_AT: [Duration; 4] = [
    Duration::from_millis(500),
    Duration::from_millis(1000),
    Duration::from_millis(2000),
    Duration::from_millis(4000),
];

async fn dial(
    endpoint: &Endpoint,
    node_id: EndpointId,
    alpn: &'static [u8],
) -> Result<Connection, iroh::endpoint::ConnectError> {
    race_dials(|| endpoint.connect(node_id, alpn), &REDIAL_AT).await
}

async fn race_dials<F, Fut, T, E>(mut attempt: F, redial_at: &[Duration]) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let started = n0_future::time::Instant::now();
    let mut attempts = futures_util::stream::FuturesUnordered::new();
    attempts.push(attempt());
    let mut redials = redial_at.iter();
    let mut next = redials.next();
    loop {
        let redial = async {
            match next {
                Some(at) => n0_future::time::sleep_until(started + *at).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            outcome = attempts.next() => match outcome {
                Some(Ok(value)) => return Ok(value),
                Some(Err(e)) if attempts.is_empty() => return Err(e),
                Some(Err(_)) | None => {}
            },
            _ = redial => {
                attempts.push(attempt());
                next = redials.next();
            }
        }
    }
}

async fn confirm_dm<T: serde::Serialize>(connection: Connection, envelope: &T) -> Result<()> {
    let (mut send, mut recv) = connection.open_bi().await?;
    write_frame(&mut send, envelope)
        .await
        .map_err(|e| anyhow::anyhow!("dm send failed: {e}"))?;
    let ack = n0_future::time::timeout(
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

impl CommyNode {
    pub fn set_catchup_store(&self, store: SharedCatchupStore) {
        *self
            .catchup_store
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(store);
    }

    pub fn catch_up(
        &self,
        server: ServerId,
        peer: [u8; 32],
        store: SharedCatchupStore,
    ) -> impl Future<Output = Result<SessionReport>> + Send + 'static {
        let endpoint = self.endpoint();
        let local = self.node_id_bytes();
        async move {
            let (me, identity) = store
                .local_identity(server)
                .context("not a member of that server")?;
            let node_id = EndpointId::from_bytes(&peer).context("invalid node key")?;
            let connection = endpoint
                .connect(node_id, CATCHUP_ALPN)
                .await
                .context("peer is unreachable")?;
            let (send, recv) = connection.open_bi().await?;
            let mut io = tokio::io::join(recv, send);
            let session = run_initiator(&mut io, &*store, &identity, server, me, local, peer);
            let result = n0_future::time::timeout(CATCHUP_SESSION_TIMEOUT, session).await;
            let (_, send) = io.into_inner();
            let_our_applied_reach_the_peer(send).await;
            connection.close(0u32.into(), b"done");
            let report = result.context("catch-up timed out")??;
            Ok(report)
        }
    }

    pub fn node_id_bytes(&self) -> [u8; 32] {
        *self.router.endpoint().id().as_bytes()
    }

    pub fn node_id_hex(&self) -> String {
        data_encoding::HEXLOWER.encode(&self.node_id_bytes())
    }

    pub async fn node_addr(&self) -> Result<crate::ticket::NodeAddr> {
        let endpoint = self.router.endpoint();
        let _ = n0_future::time::timeout(ADDRESS_WAIT, endpoint.online()).await;
        Ok(crate::ticket::NodeAddr::from(&endpoint.addr()))
    }

    fn learn(&self, addr: EndpointAddr) {
        #[cfg(wasm_browser)]
        let addr = reachable_from_browser(addr, &self.relays);
        if addr.id != self.router.endpoint().id() {
            self.lookup.add_endpoint_info(addr);
        }
    }

    pub async fn host_server(
        &mut self,
        server_id: ServerId,
        server_name: &str,
        owner: Option<(UserId, IdentityKey)>,
    ) -> Result<ServerTicket> {
        let topic_bytes = topic_for(server_id);
        let addr = self.node_addr().await?;
        let mut ticket = ServerTicket::new(server_id, server_name, topic_bytes, vec![addr]);
        ticket.owner = owner;
        self.join_topic(server_id, topic_bytes, vec![]).await?;
        Ok(ticket)
    }

    pub async fn join_server(&mut self, ticket: &ServerTicket) -> Result<bool> {
        self.rejoin_server(ticket).await?;
        let connected = self
            .wait_for_peers(ticket.server_id, std::time::Duration::from_secs(20))
            .await;
        if connected {
            self.broadcast(ticket.server_id, WirePayload::SyncRequest)
                .await?;
        }
        Ok(connected)
    }

    pub async fn rejoin_server(&mut self, ticket: &ServerTicket) -> Result<()> {
        let addrs: Vec<EndpointAddr> = ticket.bootstrap.iter().filter_map(|a| a.endpoint_addr()).collect();
        let bootstrap: Vec<EndpointId> = addrs.iter().map(|a| a.id).collect();
        for addr in addrs {
            self.learn(addr);
        }
        self.join_topic(ticket.server_id, ticket.topic, bootstrap).await
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
        n0_future::time::timeout(timeout, async {
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
        bootstrap: Vec<EndpointId>,
    ) -> Result<()> {
        let topic = TopicId::from_bytes(topic_bytes);
        let (sender, mut stream) = self
            .gossip
            .subscribe(topic, bootstrap)
            .await
            .context("gossip is not running")?
            .split();

        let events_tx = self.events_tx.clone();
        let our_id = self.node_id_bytes();
        let (neighbors_tx, neighbors_rx) = tokio::sync::watch::channel(BTreeSet::new());
        let reader = n0_future::task::spawn(async move {
            while let Some(item) = stream.next().await {
                let Ok(event) = item else {
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
                        action: None,
                    },
                    WirePayload::Signed(signed) => NetEvent::Event {
                        server: server_id,
                        sender: envelope.sender,
                        event: signed.event,
                        action: Some(signed.action),
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
            neighbors_tx.send_modify(|set| set.clear());
        });

        self.topics.insert(
            server_id,
            TopicHandle {
                sender,
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
        let ids = self.add_peers(peers_hex);
        if ids.is_empty() {
            return Ok(0);
        }
        let count = ids.len();
        handle.sender.join_peers(ids).await?;
        Ok(count)
    }

    pub fn local_events(&self) -> mpsc::UnboundedSender<NetEvent> {
        self.events_tx.clone()
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
        handle.sender.broadcast(bytes.into()).await?;
        Ok(())
    }

    pub fn media_dialer(&self) -> MediaDialer {
        MediaDialer { endpoint: self.endpoint(), lookup: self.lookup.clone(), relays: self.relays.clone() }
    }

    pub fn connection_checker(&self) -> crate::connection_check::ConnectionChecker {
        crate::connection_check::ConnectionChecker { endpoint: self.endpoint(), can_discover_address: self.can_discover_address }
    }

    pub async fn add_blob(&self, bytes: Vec<u8>) -> Result<String> {
        Ok(self.blobs.add_bytes(bytes).await?.hash.to_string())
    }

    pub fn blob_fetcher(&self) -> BlobFetcher {
        BlobFetcher {
            blobs: self.blobs.clone(),
            endpoint: self.endpoint(),
            lookup: self.lookup.clone(),
            relays: self.relays.clone(),
            #[cfg(not(wasm_browser))]
            shelf: self.shelf.clone(),
        }
    }

    #[cfg(not(wasm_browser))]
    pub fn set_shelf(&self, shelf: crate::shelf::Shelf) {
        *self.shelf.write().unwrap_or_else(|e| e.into_inner()) = Some(shelf);
    }

    #[cfg(not(wasm_browser))]
    pub async fn share_file(&self, path: &std::path::Path) -> Result<(String, u64)> {
        let shelf = self.shelf.read().unwrap_or_else(|e| e.into_inner()).clone();
        shelf.context("big files are not ready yet")?.share(path).await
    }

    #[cfg(not(wasm_browser))]
    pub async fn shares_file(&self, hash: &str) -> bool {
        let shelf = self.shelf.read().unwrap_or_else(|e| e.into_inner()).clone();
        match shelf {
            Some(shelf) => shelf.shared(hash).await.is_some(),
            None => false,
        }
    }

    #[cfg(not(wasm_browser))]
    pub async fn forget_file(&self, hash: &str) -> Result<u64> {
        let shelf = self.shelf.read().unwrap_or_else(|e| e.into_inner()).clone();
        shelf.context("big files are not ready yet")?.forget(hash).await
    }

    pub fn set_media_gate(&self, gate: MediaGate) {
        *self.media_gate.write().unwrap_or_else(|e| e.into_inner()) = Some(gate);
    }

    pub fn leave_server(&mut self, server_id: ServerId) {
        self.topics.remove(&server_id);
    }

    fn home_relay_url(&self) -> Option<RelayUrl> {
        self.router.endpoint().addr().relay_urls().next().cloned()
    }

    pub fn home_relay(&self) -> Option<String> {
        self.home_relay_url().map(|r| r.to_string())
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
        let ids = self.add_peers(peers_hex);
        self.join_topic(server_id, topic_for(server_id), ids).await
    }

    pub fn reach_via_relays(&self, peers_hex: &[String]) -> usize {
        self.add_peers(peers_hex).len()
    }

    fn add_peers(&self, peers_hex: &[String]) -> Vec<EndpointId> {
        let our_id = self.node_id_bytes();
        let mut ids = Vec::new();
        for hex in peers_hex {
            let Ok(id) = parse_node_hex(hex) else { continue };
            if *id.as_bytes() == our_id {
                continue;
            }
            self.learn(addr_via_relays(id, &self.relays));
            ids.push(id);
        }
        ids
    }

    pub fn wake(&self, node_hex: &str) -> impl Future<Output = Result<()>> + Send + 'static {
        let endpoint = self.endpoint();
        let relays = self.relays.clone();
        let node = parse_node_hex(node_hex);
        async move { crate::wake::send(endpoint, node?, &relays).await }
    }

    pub async fn shutdown(self) -> Result<()> {
        self.router.shutdown().await.map_err(|e| anyhow::anyhow!("{e}"))
    }
}

async fn let_our_applied_reach_the_peer(mut send: SendStream) {
    if send.finish().is_ok() {
        let _ = n0_future::time::timeout(CATCHUP_LINGER, send.stopped()).await;
    }
}

pub(crate) fn parse_node_hex(hex: &str) -> Result<EndpointId> {
    let bytes = data_encoding::HEXLOWER
        .decode(hex.trim().as_bytes())
        .context("node id is not valid hex")?;
    let bytes: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .context("node id must be 32 bytes")?;
    EndpointId::from_bytes(&bytes).context("invalid node key")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browsers_keep_only_commy_relay_addresses() {
        let id = iroh::SecretKey::from_bytes(&[3; 32]).public();
        let ours: RelayUrl = "https://relay.commy.chat:8443".parse().unwrap();
        let theirs: RelayUrl = "https://usw1-1.relay.n0.iroh.link".parse().unwrap();
        let direct = iroh::TransportAddr::Ip("192.0.2.1:4433".parse().unwrap());
        let mixed = EndpointAddr::from_parts(id, [iroh::TransportAddr::Relay(theirs.clone()), iroh::TransportAddr::Relay(ours.clone()), direct.clone()]);
        let kept = reachable_from_browser(mixed, std::slice::from_ref(&ours));
        assert_eq!(kept.addrs.into_iter().collect::<Vec<_>>(), vec![iroh::TransportAddr::Relay(ours.clone())]);
        let foreign = EndpointAddr::from_parts(id, [iroh::TransportAddr::Relay(theirs), direct]);
        let kept = reachable_from_browser(foreign, std::slice::from_ref(&ours));
        assert_eq!(kept.addrs.into_iter().collect::<Vec<_>>(), vec![iroh::TransportAddr::Relay(ours)]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_stalled_dial_is_joined_by_a_fresh_one() {
        use std::pin::Pin;
        let started = n0_future::time::Instant::now();
        let mut calls = 0;
        let result: Result<u32, &str> = race_dials(
            || -> Pin<Box<dyn Future<Output = Result<u32, &str>>>> {
                calls += 1;
                match calls {
                    1 => Box::pin(std::future::pending()),
                    n => Box::pin(std::future::ready(Ok(n))),
                }
            },
            &REDIAL_AT,
        )
        .await;
        assert_eq!(result, Ok(2));
        assert_eq!(started.elapsed(), REDIAL_AT[0]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_dial_that_fails_outright_does_not_wait_for_redials() {
        use std::pin::Pin;
        let started = n0_future::time::Instant::now();
        let result: Result<u32, &str> = race_dials(
            || -> Pin<Box<dyn Future<Output = Result<u32, &str>>>> { Box::pin(std::future::ready(Err("no address"))) },
            &REDIAL_AT,
        )
        .await;
        assert_eq!(result, Err("no address"));
        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn a_late_failure_waits_for_the_dials_still_running() {
        use std::pin::Pin;
        let mut calls = 0;
        let result: Result<u32, &str> = race_dials(
            || -> Pin<Box<dyn Future<Output = Result<u32, &str>>>> {
                calls += 1;
                match calls {
                    1 => Box::pin(async {
                        n0_future::time::sleep(Duration::from_millis(700)).await;
                        Err("timed out")
                    }),
                    _ => Box::pin(async {
                        n0_future::time::sleep(Duration::from_millis(400)).await;
                        Ok(7)
                    }),
                }
            },
            &REDIAL_AT[..1],
        )
        .await;
        assert_eq!(result, Ok(7));
    }

    #[test]
    fn node_hex_roundtrip() {
        let key = iroh::SecretKey::generate().public();
        let hex = data_encoding::HEXLOWER.encode(key.as_bytes());
        let parsed = parse_node_hex(&hex).unwrap();
        assert_eq!(parsed, key);
    }

    #[test]
    fn bad_node_hex_is_rejected() {
        assert!(parse_node_hex("zz").is_err());
        assert!(parse_node_hex("abcd").is_err());
    }

    fn peer() -> EndpointId {
        iroh::SecretKey::generate().public()
    }

    #[test]
    fn neighbor_set_follows_gossip_membership() {
        let (a, b, c) = (peer(), peer(), peer());
        let mut set = BTreeSet::new();
        for p in [a, b] {
            assert_eq!(track_neighbors(&mut set, &GossipEvent::NeighborUp(p)), vec![*p.as_bytes()]);
        }
        assert!(track_neighbors(&mut set, &GossipEvent::Lagged).is_empty());
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
    async fn a_big_file_streams_from_where_it_sits_to_a_file_and_is_shared_on() {
        let within = Duration::from_secs;
        let dir = std::env::temp_dir().join(format!("commy-big-file-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("video.bin");
        let bytes: Vec<u8> = (0..3_000_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
        std::fs::write(&original, &bytes).unwrap();
        let (a, _, _, _) = CommyNode::spawn(None).await.unwrap();
        let (b, _, _, _) = CommyNode::spawn(None).await.unwrap();
        a.set_shelf(crate::shelf::Shelf::open(&dir.join("a")).await.unwrap());
        b.set_shelf(crate::shelf::Shelf::open(&dir.join("b")).await.unwrap());
        let (hash, size) = a.share_file(&original).await.unwrap();
        assert_eq!(size, bytes.len() as u64);
        let addr = tokio::time::timeout(within(30), a.node_addr()).await.unwrap().unwrap();
        b.learn(addr.endpoint_addr().unwrap());
        let target = dir.join("saved.bin");
        let mut seen = Vec::new();
        let source = a.node_id_hex();
        let fetcher = b.blob_fetcher();
        let too_small = fetcher.download_to(&hash, [source.as_str()], 1024, &target, |_, _| {}).await;
        assert!(too_small.unwrap_err().is::<BlobTooLarge>());
        let got = tokio::time::timeout(within(60), fetcher.download_to(&hash, [source.as_str()], u64::MAX, &target, |done, total| seen.push((done, total))))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got, bytes.len() as u64);
        assert_eq!(std::fs::read(&target).unwrap(), bytes);
        assert!(!crate::shelf::part_path(&target).exists());
        assert_eq!(seen.first(), Some(&(0, got)));
        assert_eq!(seen.last(), Some(&(got, got)));
        assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0));
        assert!(b.blob_fetcher().shelf.read().unwrap().as_ref().unwrap().shared(&hash).await.is_some(), "the downloader shares it on");
        let again = dir.join("again.bin");
        assert_eq!(b.blob_fetcher().download_to(&hash, std::iter::empty(), u64::MAX, &again, |_, _| {}).await.unwrap(), got);
        assert_eq!(std::fs::read(&again).unwrap(), bytes);
        a.shutdown().await.ok();
        b.shutdown().await.ok();
        let _ = std::fs::remove_dir_all(&dir);
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
        a.learn(addr.endpoint_addr().unwrap());

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
        a.learn(addr.endpoint_addr().unwrap());

        let mut sender = keyed_profile("sender");
        sender.node = Some(data_encoding::HEXLOWER.encode(&[9u8; 32]));
        let mut message = DmMessage::new(sender.id, UserId::new(), "hi");
        message.from_node = a.node_id_hex();
        let envelope = DmEnvelope::new(message, sender);
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
        a.learn(addr.endpoint_addr().unwrap());
        relay.learn(addr.endpoint_addr().unwrap());

        let sender = keyed_profile("sender");
        let mut message = DmMessage::new(sender.id, UserId::new(), "for you only");
        message.sign(&a.node_id_hex(), &identity_for(sender.id));
        let envelope = DmEnvelope::new(message, sender);
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

    #[test]
    fn blob_sizes_over_the_cap_are_refused_before_any_bytes() {
        assert!(check_blob_size(0, MAX_BLOB_BYTES).is_ok());
        assert!(check_blob_size(MAX_BLOB_BYTES, MAX_BLOB_BYTES).is_ok());
        let refused = check_blob_size(MAX_BLOB_BYTES + 1, MAX_BLOB_BYTES).unwrap_err();
        assert_eq!(refused.to_string(), "that file is larger than 20 MB");
        assert!(check_blob_size(u64::MAX, MAX_BLOB_BYTES).is_err());
        assert!(check_blob_size(512 * 1024 * 1024, 512 * 1024 * 1024).is_ok());
    }

    async fn local_node() -> CommyNode {
        CommyNode::spawn_with(None, Some(iroh::RelayMap::empty())).await.unwrap().0
    }

    async fn local_pair() -> (CommyNode, CommyNode) {
        let (host, guest) = (local_node().await, local_node().await);
        guest.learn(host.endpoint().addr());
        (host, guest)
    }

    fn picture() -> Vec<u8> {
        (0..300_000u32).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn files_come_from_the_uploader_first_then_from_members_online() {
        let (a, b) = ([1u8; 32], [2u8; 32]);
        let hex = |n: [u8; 32]| data_encoding::HEXLOWER.encode(&n);
        assert_eq!(blob_sources(&hex(b), vec![a, b, a]), vec![hex(b), hex(a)]);
        assert_eq!(blob_sources("away", vec![a]), vec![hex(a), "away".to_string()], "an absent uploader is tried last");
        assert_eq!(blob_sources("away", vec![]), vec!["away".to_string()]);
        assert_eq!(blob_sources("", vec![a]), vec![hex(a)]);
    }

    #[tokio::test]
    async fn a_member_fetches_a_picture_from_its_uploader() {
        let (host, guest) = local_pair().await;
        let bytes = picture();
        let blob = host.add_blob(bytes.clone()).await.unwrap();
        let fetched = guest.blob_fetcher().fetch(&blob, Some(&host.node_id_hex()), MAX_BLOB_BYTES).await.unwrap();
        assert_eq!(fetched, bytes);
        host.shutdown().await.ok();
        guest.shutdown().await.ok();
    }

    #[tokio::test]
    async fn an_uploader_who_no_longer_has_the_file_is_reported_in_plain_words() {
        let (host, guest) = local_pair().await;
        let blob = blob_hash(b"gone after a restart");
        let failure = guest.blob_fetcher().fetch(&blob, Some(&host.node_id_hex()), MAX_BLOB_BYTES).await.unwrap_err();
        assert_eq!(failure.to_string(), BLOB_UNAVAILABLE);
        host.shutdown().await.ok();
        guest.shutdown().await.ok();
    }

    #[tokio::test]
    async fn our_own_node_is_never_dialled_for_a_file() {
        let node = local_node().await;
        let blob = blob_hash(b"posted before a restart");
        let failure = node.blob_fetcher().fetch(&blob, Some(&node.node_id_hex()), MAX_BLOB_BYTES).await.unwrap_err();
        assert_eq!(failure.to_string(), BLOB_UNAVAILABLE);
        node.shutdown().await.ok();
    }

    #[tokio::test]
    async fn a_member_who_has_the_picture_passes_it_on_when_the_uploader_lost_it() {
        let (host, friend) = local_pair().await;
        let guest = local_node().await;
        guest.learn(host.endpoint().addr());
        guest.learn(friend.endpoint().addr());
        let bytes = picture();
        let blob = host.add_blob(bytes.clone()).await.unwrap();
        friend.blob_fetcher().fetch(&blob, Some(&host.node_id_hex()), MAX_BLOB_BYTES).await.unwrap();
        let restarted = local_node().await;
        guest.learn(restarted.endpoint().addr());
        let sources = [restarted.node_id_hex(), friend.node_id_hex()];
        let fetched = guest.blob_fetcher().fetch_from(&blob, sources.iter().map(String::as_str), MAX_BLOB_BYTES).await.unwrap();
        assert_eq!(fetched, bytes);
        for node in [host, friend, guest, restarted] {
            node.shutdown().await.ok();
        }
    }

    #[tokio::test]
    async fn a_file_over_the_limit_is_refused_without_asking_anyone_else() {
        let (host, guest) = local_pair().await;
        let blob = host.add_blob(picture()).await.unwrap();
        let source = host.node_id_hex();
        let failure = guest.blob_fetcher().fetch_from(&blob, [source.as_str()], 1024).await.unwrap_err();
        assert!(failure.is::<BlobTooLarge>(), "{failure:#}");
        host.shutdown().await.ok();
        guest.shutdown().await.ok();
    }
}
