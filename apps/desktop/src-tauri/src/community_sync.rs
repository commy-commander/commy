use crate::lock_or_recover::LockOrRecover;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::Utc;
use commy_core::channel_key::{ChannelSecret, KeyId, KeyScope, Keyring};
use commy_core::{MessageId, Presence, ServerEvent, ServerId, SignedEvent, UserId, UserProfile};
use commy_net::catchup::CatchupStore;
use commy_net::checkpoint::{self, Assembler};
use commy_net::community_streams::{self as streams, rings, wake_point, Lane, Want};
pub use commy_net::sync_streams::SharedKey;
use commy_net::voice_gate::{link_of, StateLink};
use commy_net::{NetEvent, WirePayload};
use commy_sync::wire::{ErrorCode, WATCH_SINCE};
use commy_sync::{AppendError, StreamId, SyncClient, SyncEvent};
use commy_view::away::Place;
use commy_view::sync_host::HostRemovals;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;

use crate::state::App;
use crate::sync_link::{self, Pool};

pub const FILE: &str = "community-sync.json";
const REFRESH_EVERY: Duration = Duration::from_secs(30);
const REFRESH_SOON: Duration = Duration::from_millis(800);
const SAVE_DELAY: Duration = Duration::from_secs(5);
const FIRST_OPEN_SPREAD_MS: u64 = 8_000;
const MAX_ECHOES: usize = 512;
const MAX_INVITED: usize = 32;
const MAX_BACKFILL_PER_STREAM: usize = 1_000;
const MAX_PARKED: usize = 2_000;
const SNAPSHOT_EVERY_MS: i64 = 6 * 60 * 60 * 1000;
const MAX_UNSENT: usize = 500;
const UNSENT_WITHOUT_SERVICE_MS: i64 = 10 * 60 * 1000;
const PUBLISH_WAIT: Duration = Duration::from_secs(8);
const UNSENT_FOR_MS: i64 = 7 * 24 * 60 * 60 * 1000;
const READ_BACK: u64 = 2_000;
const GREET_GAP: Duration = Duration::from_secs(120);
const GREET_SPREAD_MS: u64 = 3_000;
const CHECKPOINT_PATIENCE: Duration = Duration::from_secs(20);
const DOZE_AFTER: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Cursors {
    #[serde(default)]
    pub after: BTreeMap<String, u64>,
    #[serde(default)]
    pub backfilled: BTreeSet<String>,
    #[serde(default)]
    pub announced: BTreeMap<String, String>,
    #[serde(default)]
    pub unsent: Vec<Unsent>,
    #[serde(default)]
    pub read_back: BTreeSet<String>,
    #[serde(default)]
    pub older_before: BTreeMap<String, u64>,
    #[serde(default)]
    pub hosts: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Unsent {
    pub server: ServerId,
    pub payload: WirePayload,
    pub at: i64,
}

pub fn hold_unsent(unsent: &mut Vec<Unsent>, held: Unsent, now: i64) {
    unsent.retain(|old| now - old.at < UNSENT_FOR_MS && !(old.server == held.server && old.payload == held.payload));
    if now - held.at < UNSENT_FOR_MS {
        unsent.push(held);
    }
    let excess = unsent.len().saturating_sub(MAX_UNSENT);
    unsent.drain(..excess);
}

impl Cursors {
    pub fn after(&self, stream: StreamId) -> u64 {
        self.after.get(&stream.to_hex()).copied().unwrap_or(0)
    }

    pub fn advance(&mut self, stream: StreamId, seq: u64) -> bool {
        let slot = self.after.entry(stream.to_hex()).or_default();
        let moved = seq > *slot;
        *slot = (*slot).max(seq);
        moved
    }

    pub fn start_at(&mut self, stream: StreamId, seq: u64) {
        let hex = stream.to_hex();
        let start = self.after.get(&hex).map_or(seq, |held| (*held).min(seq));
        self.after.insert(hex, start);
    }

    pub fn keep_only(&mut self, streams: &HashSet<String>) {
        self.after.retain(|stream, _| streams.contains(stream));
        self.backfilled.retain(|stream| streams.contains(stream));
        self.read_back.retain(|stream| streams.contains(stream));
        self.older_before.retain(|stream, _| streams.contains(stream));
        self.announced.retain(|stream, _| streams.contains(stream));
    }

    pub fn keep_hosts_of(&mut self, servers: &HashSet<ServerId>) {
        self.hosts.retain(|server, _| server.parse::<ServerId>().is_ok_and(|sid| servers.contains(&sid)));
    }

    pub fn settle_host(&mut self, server: ServerId, url: &str, account_url: &str, streams: &[StreamId]) -> bool {
        let previous = self.hosts.insert(server.to_string(), url.to_string());
        let moved = previous.as_deref().unwrap_or(account_url) != url;
        if moved {
            for stream in streams {
                let hex = stream.to_hex();
                self.after.remove(&hex);
                self.backfilled.remove(&hex);
                self.read_back.remove(&hex);
                self.older_before.remove(&hex);
                self.announced.remove(&hex);
            }
        }
        moved
    }

    pub fn announce(&mut self, stream: StreamId, profile: &str) -> bool {
        self.announced.insert(stream.to_hex(), profile.to_string()).as_deref() != Some(profile)
    }

    pub fn unannounce(&mut self, stream: StreamId, profile: &str) {
        if self.announced.get(&stream.to_hex()).is_some_and(|known| known == profile) {
            self.announced.remove(&stream.to_hex());
        }
    }
}

struct Reading {
    stream: StreamId,
    _client: SyncClient,
    task: tauri::async_runtime::JoinHandle<()>,
}

struct CheckpointRead {
    server: ServerId,
    state: commy_sync::StreamKeys,
    assembler: Assembler,
    looked_back: bool,
}

struct Community {
    url: String,
    pool: Pool,
    wants: HashMap<StreamId, Want>,
    dormant: HashSet<StreamId>,
    baseline: HashMap<StreamId, u64>,
    heard: HashMap<StreamId, Instant>,
    watchless: bool,
    owned: bool,
}

impl Community {
    fn new(url: &str, events: mpsc::UnboundedSender<SyncEvent>, owned: bool) -> Self {
        Community {
            pool: Pool::with_events(url, events),
            url: url.to_string(),
            wants: HashMap::new(),
            dormant: HashSet::new(),
            baseline: HashMap::new(),
            heard: HashMap::new(),
            watchless: false,
            owned,
        }
    }

    fn lazy(&self, server: ServerId) -> bool {
        !self.owned && !self.watchless && *FOCUS.lock_or_recover() != Some(server)
    }

    fn place(&mut self, server: ServerId, group: OpenGroup) {
        let lazy = self.lazy(server);
        let (quiet, open): (OpenGroup, OpenGroup) = group.into_iter().partition(|(lane, _, _)| lazy && matches!(lane, Lane::Channel(_)));
        let now = Instant::now();
        for (lane, keys, _) in &open {
            if matches!(lane, Lane::Channel(_)) {
                self.heard.insert(keys.id, now);
            }
        }
        follow_group(&mut self.pool, open);
        let quiet: Vec<commy_sync::StreamKeys> = quiet.into_iter().map(|(_, keys, _)| keys).collect();
        self.dormant.extend(quiet.iter().map(|keys| keys.id));
        if !quiet.is_empty() {
            self.pool.watch_at(&self.url, quiet);
        }
    }

    fn wake(&mut self, stream: StreamId, cursors: &Cursors) -> bool {
        if !self.dormant.remove(&stream) {
            return false;
        }
        self.baseline.remove(&stream);
        let Some(want) = self.wants.get(&stream) else { return false };
        self.heard.insert(stream, Instant::now());
        follow_group(&mut self.pool, vec![(want.lane, want.keys.clone(), cursors.after(stream))]);
        true
    }

    fn doze(&mut self, streams: &[StreamId]) {
        let keys: Vec<commy_sync::StreamKeys> = streams.iter().filter_map(|stream| Some(self.wants.get(stream)?.keys.clone())).collect();
        for keys in &keys {
            self.pool.unfollow(keys.id);
            self.heard.remove(&keys.id);
            self.dormant.insert(keys.id);
        }
        if !keys.is_empty() {
            self.pool.watch_at(&self.url, keys);
        }
    }

    fn forget(&mut self, stream: StreamId) {
        self.pool.unfollow(stream);
        self.pool.unwatch(stream);
        self.dormant.remove(&stream);
        self.baseline.remove(&stream);
        self.heard.remove(&stream);
    }
}

fn changed(after: u64, baseline: &mut Option<u64>, head: u64) -> bool {
    if after > 0 {
        return head != after;
    }
    match *baseline {
        Some(first) => head != first,
        None => {
            *baseline = Some(head);
            false
        }
    }
}

struct Live {
    handle: AppHandle,
    url: String,
    dir: PathBuf,
    me: UserId,
    node: [u8; 32],
    events: mpsc::UnboundedSender<SyncEvent>,
    net: mpsc::UnboundedSender<NetEvent>,
    communities: HashMap<ServerId, Community>,
    caught_up: HashSet<StreamId>,
    current: HashSet<StreamId>,
    lost: HashSet<StreamId>,
    refused: HashSet<StreamId>,
    removed: HostRemovals,
    seen: HashMap<StreamId, HashSet<MessageId>>,
    backfilling: HashSet<StreamId>,
    snapshots: HashMap<ServerId, WirePayload>,
    parked: HashMap<ServerId, VecDeque<WirePayload>>,
    published: HashMap<ServerId, i64>,
    retried: HashSet<MessageId>,
    echoes: VecDeque<[u8; 32]>,
    fresh: HashSet<StreamId>,
    reading: Option<Reading>,
    woken: HashSet<StreamId>,
    opening: HashMap<ServerId, OpenGroup>,
    checkpoints: HashMap<StreamId, CheckpointRead>,
    checkpoint_tried: HashSet<StreamId>,
    greeted: HashMap<ServerId, Instant>,
    say_again: HashSet<ServerId>,
    cursors: Cursors,
    task: tauri::async_runtime::JoinHandle<()>,
}

impl Live {
    fn route(&self, stream: StreamId) -> Option<(ServerId, Want)> {
        self.communities.iter().find_map(|(server, c)| c.wants.get(&stream).map(|want| (*server, want.clone())))
    }

    fn take_echo(&mut self, digest: &[u8; 32]) -> bool {
        match self.echoes.iter().position(|d| d == digest) {
            Some(at) => {
                self.echoes.remove(at);
                true
            }
            None => false,
        }
    }

    fn note_echo(&mut self, digest: [u8; 32]) {
        self.echoes.push_back(digest);
        while self.echoes.len() > MAX_ECHOES {
            self.echoes.pop_front();
        }
    }
}

static LIVE: Mutex<Option<Live>> = Mutex::new(None);

fn newly_removed(live: &mut Live, stream: StreamId) -> Option<ServerId> {
    let (server, _) = live.route(stream)?;
    let url = live.communities.get(&server)?.url.clone();
    live.removed.note(server, &url).then_some(server)
}

fn tell_host_removed(handle: &AppHandle, server: ServerId) {
    let name = handle.state::<App>().servers.lock_or_recover().get(&server).map(|state| state.server.name.clone());
    tracing::warn!("the sync server of {server} removed it; new messages there no longer reach anyone");
    let _ = handle.emit("host-removed", serde_json::json!({ "server": server.to_string(), "name": name }));
    let _ = handle.emit("servers-changed", ());
}

pub fn host_removed(sid: ServerId) -> bool {
    LIVE.lock_or_recover().as_ref().is_some_and(|live| live.communities.get(&sid).is_some_and(|c| live.removed.holds(sid, &c.url)))
}

pub fn removed_by_host() -> HashSet<ServerId> {
    let guard = LIVE.lock_or_recover();
    let Some(live) = guard.as_ref() else { return HashSet::new() };
    live.communities.iter().filter(|(sid, c)| live.removed.holds(**sid, &c.url)).map(|(sid, _)| *sid).collect()
}

pub fn greeting_of(sid: ServerId) -> Option<commy_sync::wire::Greeting> {
    LIVE.lock_or_recover().as_ref()?.communities.get(&sid)?.pool.greeting()
}

pub fn room_relay_of(sid: ServerId) -> commy_view::sync_host::RoomRelay {
    let live = LIVE.lock_or_recover();
    let Some(community) = live.as_ref().and_then(|live| live.communities.get(&sid).map(|community| (live, community))) else {
        return commy_view::sync_host::RoomRelay::Ours;
    };
    let (live, community) = community;
    commy_view::sync_host::room_relay(&community.url, &live.url, community.pool.greeting().as_ref())
}
static INVITED: Mutex<Vec<(ServerId, KeyId, ChannelSecret)>> = Mutex::new(Vec::new());
static INVITED_HOSTS: Mutex<Vec<(ServerId, String)>> = Mutex::new(Vec::new());
static SAVE_PENDING: AtomicBool = AtomicBool::new(false);
static REFRESH_PENDING: AtomicBool = AtomicBool::new(false);
static UNSENT: Mutex<Vec<Unsent>> = Mutex::new(Vec::new());
static FOCUS: Mutex<Option<ServerId>> = Mutex::new(None);

type OpenGroup = Vec<(Lane, commy_sync::StreamKeys, u64)>;

fn digest(plain: &[u8]) -> [u8; 32] {
    Sha256::digest(plain).into()
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub fn remember_invite_key(server: ServerId, key: KeyId, secret: ChannelSecret) {
    let mut invited = INVITED.lock_or_recover();
    invited.retain(|(s, _, _)| *s != server);
    invited.push((server, key, secret));
    let excess = invited.len().saturating_sub(MAX_INVITED);
    invited.drain(..excess);
}

pub fn remember_invite_host(server: ServerId, host: Option<String>) {
    let mut invited = INVITED_HOSTS.lock_or_recover();
    invited.retain(|(s, _)| *s != server);
    if let Some(host) = host.filter(|h| commy_core::sync_host::is_sync_host(h)) {
        invited.push((server, host));
    }
    let excess = invited.len().saturating_sub(MAX_INVITED);
    invited.drain(..excess);
}

pub fn invited_host(server: ServerId) -> Option<String> {
    INVITED_HOSTS.lock_or_recover().iter().find(|(s, _)| *s == server).map(|(_, host)| host.clone())
}

fn invited_key(server: ServerId) -> Option<(KeyId, ChannelSecret)> {
    INVITED.lock_or_recover().iter().find(|(s, _, _)| *s == server).map(|(_, key, secret)| (*key, secret.clone()))
}

pub fn keep_invite_key(app: &App, server: ServerId, owner: Option<(UserId, commy_core::IdentityKey)>) {
    let Some((key, secret)) = invited_key(server) else { return };
    if crate::account_log::shared_community_key(app, server) != Some(key) {
        crate::account_log::share_community_key(app, server, key, &secret, owner);
    }
}

pub fn invite_key(app: &App, server: ServerId) -> Option<(KeyId, ChannelSecret)> {
    let holder = crate::channel_lock::holder_of(app);
    let servers = app.servers.lock_or_recover();
    let state = servers.get(&server)?;
    let ring = holder.community_ring(state, None)?;
    ring.newest_in(state, commy_core::channel_key::KeyScope::Community).map(|(id, secret)| (id, secret.clone()))
}

fn load(dir: &Path) -> Cursors {
    crate::entry_list::read(&dir.join(FILE)).unwrap_or_default()
}

fn save_soon() {
    if SAVE_PENDING.swap(true, Ordering::SeqCst) {
        return;
    }
    crate::account_session::spawn(async {
        tokio::time::sleep(SAVE_DELAY).await;
        SAVE_PENDING.store(false, Ordering::SeqCst);
        save();
    });
}

fn save() {
    let snapshot = LIVE.lock_or_recover().as_ref().map(|live| (live.dir.clone(), live.cursors.clone()));
    if let Some((dir, mut cursors)) = snapshot {
        cursors.unsent = UNSENT.lock_or_recover().clone();
        crate::entry_list::write(&dir.join(FILE), &cursors);
    }
}

fn hold(held: Unsent) {
    hold_unsent(&mut UNSENT.lock_or_recover(), held, now_ms());
    save_soon();
}

fn worth_retrying(error: &AppendError) -> bool {
    match error {
        AppendError::NotFollowed | AppendError::Stopped => true,
        AppendError::Refused(code) => code.is_retryable(),
        AppendError::Seal(_) => false,
    }
}

fn send_unsent() {
    let due: Vec<Unsent> = {
        let guard = LIVE.lock_or_recover();
        let Some(live) = guard.as_ref() else { return };
        let mut unsent = UNSENT.lock_or_recover();
        let now = now_ms();
        unsent.retain(|held| now - held.at < UNSENT_FOR_MS);
        let (ready, waiting): (Vec<Unsent>, Vec<Unsent>) = std::mem::take(&mut *unsent)
            .into_iter()
            .partition(|held| {
                let Some(lane) = streams::lane_of_payload(&held.payload) else { return true };
                let key_event = streams::event_of(&held.payload).is_none_or(|(event, _)| streams::is_key_event(event));
                !targets(live, held.server, lane, key_event).is_empty()
            });
        *unsent = waiting;
        ready
    };
    if due.is_empty() {
        return;
    }
    tracing::info!("sending {} server changes the sync service has not stored yet", due.len());
    for held in due {
        publish_held(held);
    }
    save_soon();
}

pub fn stop() {
    save();
    if let Some(old) = LIVE.lock_or_recover().take() {
        old.task.abort();
        if let Some(reading) = old.reading {
            reading.task.abort();
        }
        UNSENT.lock_or_recover().clear();
    }
}

pub fn nudge() {
    if let Some(live) = LIVE.lock_or_recover().as_ref() {
        for community in live.communities.values() {
            community.pool.nudge();
        }
    }
}

pub async fn run(handle: AppHandle) {
    loop {
        tend(&handle).await;
        tokio::time::sleep(REFRESH_EVERY).await;
    }
}

pub fn refresh_soon(handle: &AppHandle) {
    if REFRESH_PENDING.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = handle.clone();
    crate::account_session::spawn(async move {
        tokio::time::sleep(REFRESH_SOON).await;
        REFRESH_PENDING.store(false, Ordering::SeqCst);
        tend(&handle).await;
    });
}

async fn tend(handle: &AppHandle) {
    let app = handle.state::<App>();
    let Some(url) = sync_link::url() else {
        stop();
        UNSENT.lock_or_recover().retain(|held| now_ms() - held.at < UNSENT_WITHOUT_SERVICE_MS);
        return;
    };
    let (Some(dir), Some(me)) = (app.data_dir().cloned(), app.profile.lock_or_recover().as_ref().map(|p| p.id)) else { return };
    if !app.key_settled() || app.identity().is_none() {
        return;
    }
    let same = LIVE.lock_or_recover().as_ref().is_some_and(|l| l.url == url && l.dir == dir && l.me == me);
    if !same {
        let Some(crate::state::SyncFeed { events: net, node }) = app.sync_feed.lock_or_recover().clone() else { return };
        stop();
        let (events, received) = mpsc::unbounded_channel();
        let task = crate::account_session::spawn(listen(handle.clone(), received));
        let mut cursors = load(&dir);
        {
            let mut unsent = UNSENT.lock_or_recover();
            let now = now_ms();
            for held in std::mem::take(&mut cursors.unsent) {
                hold_unsent(&mut unsent, held, now);
            }
        }
        *LIVE.lock_or_recover() = Some(Live {
            handle: handle.clone(),
            url,
            dir,
            me,
            node,
            events,
            net,
            communities: HashMap::new(),
            caught_up: HashSet::new(),
            current: HashSet::new(),
            lost: HashSet::new(),
            refused: HashSet::new(),
            removed: HostRemovals::default(),
            seen: HashMap::new(),
            backfilling: HashSet::new(),
            snapshots: HashMap::new(),
            parked: HashMap::new(),
            published: HashMap::new(),
            retried: HashSet::new(),
            echoes: VecDeque::new(),
            fresh: HashSet::new(),
            reading: None,
            woken: HashSet::new(),
            opening: HashMap::new(),
            checkpoints: HashMap::new(),
            checkpoint_tried: HashSet::new(),
            greeted: HashMap::new(),
            say_again: HashSet::new(),
            cursors,
            task,
        });
    }
    refresh(handle);
    record_hosts(&app);
    crate::sync_eraser::tend(&app);
    crate::dm_sync::tend(handle);
    share_keys(&app);
    release_parked(handle);
    publish_snapshots(handle);
    store_pending(handle);
    send_unsent();
    read_back(handle);
    if let Some(live) = LIVE.lock_or_recover().as_mut() {
        doze_quiet_channels(live);
    }
}

fn read_back(handle: &AppHandle) {
    let next = {
        let mut guard = LIVE.lock_or_recover();
        let Some(live) = guard.as_mut() else { return };
        if live.reading.as_ref().is_some_and(|r| live.route(r.stream).is_some() && !r.task.inner().is_finished()) {
            return;
        }
        if let Some(stale) = live.reading.take() {
            stale.task.abort();
        }
        let mut waiting: Vec<(StreamId, ServerId, Want)> = live
            .communities
            .iter()
            .flat_map(|(server, c)| c.wants.iter().map(move |(stream, want)| (*stream, *server, want.clone())))
            .filter(|(stream, _, want)| {
                let hex = stream.to_hex();
                matches!(want.lane, Lane::Channel(_))
                    && live.caught_up.contains(stream)
                    && !live.cursors.read_back.contains(&hex)
                    && (live.fresh.contains(stream) || live.cursors.older_before.contains_key(&hex))
            })
            .collect();
        waiting.sort_by_key(|(stream, _, _)| stream.to_hex());
        let mut pick = None;
        for (stream, server, want) in waiting {
            match live.cursors.older_before.get(&stream.to_hex()).copied() {
                Some(until) if until > 1 => {
                    pick = Some((stream, server, want, until));
                    break;
                }
                _ => {
                    live.cursors.older_before.remove(&stream.to_hex());
                    live.cursors.read_back.insert(stream.to_hex());
                }
            }
        }
        pick.and_then(|pick| Some((pick.clone(), live.communities.get(&pick.1)?.url.clone())))
    };
    let Some(((stream, server, want, until), url)) = next else { return };
    let (client, events) = SyncClient::spawn(url);
    client.follow(want.keys.clone(), until.saturating_sub(READ_BACK + 1));
    let task = crate::account_session::spawn(read_until(handle.clone(), server, stream, want.lane, until, events));
    if let Some(live) = LIVE.lock_or_recover().as_mut() {
        live.reading = Some(Reading { stream, _client: client, task });
    }
}

async fn read_until(handle: AppHandle, server: ServerId, stream: StreamId, lane: Lane, until: u64, mut events: mpsc::UnboundedReceiver<SyncEvent>) {
    let mut older = Vec::new();
    while let Some(event) = events.recv().await {
        match event {
            SyncEvent::Entry { seq, plain, .. } if seq < until => older.extend(streams::decode_entry(&plain, lane)),
            SyncEvent::Entry { .. } | SyncEvent::CaughtUp { .. } => break,
            SyncEvent::Refused { code, .. } => {
                tracing::debug!("the sync service refused reading older history: {code:?}");
                return;
            }
            _ => {}
        }
    }
    tracing::info!("read {} older entries for a channel of {server}", older.len());
    apply_older(&handle, server, older);
    if let Some(live) = LIVE.lock_or_recover().as_mut() {
        live.cursors.older_before.remove(&stream.to_hex());
        live.cursors.read_back.insert(stream.to_hex());
    }
    save_soon();
    refresh_soon(&handle);
}

fn apply_older(handle: &AppHandle, server: ServerId, older: Vec<WirePayload>) {
    if handle.state::<App>().awaits_snapshot(server) {
        return;
    }
    let Some(net) = LIVE.lock_or_recover().as_ref().map(|live| live.net.clone()) else { return };
    let store = crate::catchup::AppStore(handle.clone());
    let mut messages = Vec::new();
    for payload in older {
        let Some((event, action)) = streams::event_of(&payload).map(|(event, action)| (event.clone(), action)) else { continue };
        match event {
            ServerEvent::MessageCreate { message } => messages.push(message),
            event => {
                if !messages.is_empty() {
                    store.apply(server, std::mem::take(&mut messages), Vec::new());
                }
                let _ = net.send(NetEvent::Event { server, sender: streams::SERVICE_SENDER, event, action });
            }
        }
    }
    if !messages.is_empty() {
        store.apply(server, messages, Vec::new());
    }
}

fn store_pending(handle: &AppHandle) {
    let app = handle.state::<App>();
    let ready: Vec<ServerId> = {
        let guard = LIVE.lock_or_recover();
        let Some(live) = guard.as_ref() else { return };
        live.communities.iter().filter(|(_, c)| c.wants.values().any(|w| matches!(w.lane, Lane::Channel(_)) && w.writes)).map(|(sid, _)| *sid).collect()
    };
    for server in ready {
        let pending = app.outbox.lock_or_recover().pending_ids(server);
        let fresh: Vec<MessageId> = {
            let mut guard = LIVE.lock_or_recover();
            let Some(live) = guard.as_mut() else { return };
            pending.into_iter().filter(|id| live.retried.insert(*id)).collect()
        };
        if fresh.is_empty() {
            continue;
        }
        let messages: Vec<commy_core::Message> = {
            let servers = app.servers.lock_or_recover();
            let Some(state) = servers.get(&server) else { continue };
            fresh.iter().filter_map(|id| state.message(*id).cloned()).collect()
        };
        for message in messages {
            publish(server, &WirePayload::Event(ServerEvent::MessageCreate { message }));
        }
    }
}

fn stored(handle: &AppHandle, server: ServerId, message: MessageId) {
    let app = handle.state::<App>();
    let cleared = app.outbox.lock_or_recover().clear_delivered(server, [message]);
    if cleared {
        app.persist_outbox();
        let _ = handle.emit("server-event", server.to_string());
    }
}

pub(crate) fn given_key(app: &App, server: ServerId) -> Option<(KeyId, ChannelSecret)> {
    invited_key(server).or_else(|| crate::account_log::community_key(app, server))
}

fn share_keys(app: &App) {
    let holder = crate::channel_lock::holder_of(app);
    let sids: Vec<ServerId> = app.servers.lock_or_recover().keys().copied().filter(|sid| !app.awaits_snapshot(*sid)).collect();
    let stored: HashMap<ServerId, KeyId> = sids.iter().filter_map(|sid| Some((*sid, crate::account_log::shared_community_key(app, *sid)?))).collect();
    let owners: HashMap<ServerId, (UserId, commy_core::IdentityKey)> = sids.iter().filter_map(|sid| Some((*sid, app.vouched_ticket(*sid)?.owner?))).collect();
    let unvouched: HashSet<ServerId> = owners.keys().copied().filter(|sid| crate::account_log::shared_owner(app, *sid).is_none()).collect();
    let due: Vec<(ServerId, KeyId, ChannelSecret)> = {
        let servers = app.servers.lock_or_recover();
        sids.iter()
            .filter_map(|sid| {
                let state = servers.get(sid)?;
                let ring = holder.community_ring(state, None)?;
                let (newest, secret) = ring.newest_in(state, KeyScope::Community)?;
                let held = ring.held_records(state, KeyScope::Community);
                let at = |id: KeyId| held.iter().find(|(record, _)| record.id == id).map(|(record, _)| record.at);
                let behind = match stored.get(sid) {
                    None => true,
                    Some(id) if *id == newest => unvouched.contains(sid),
                    Some(id) => at(*id).zip(at(newest)).is_some_and(|(old, new)| old < new),
                };
                behind.then(|| (*sid, newest, secret.clone()))
            })
            .collect()
    };
    for (sid, key, secret) in due {
        tracing::info!("sharing the key of {sid} with your other devices");
        crate::account_log::share_community_key(app, sid, key, &secret, owners.get(&sid).copied());
    }
}

pub fn adopt_owner(handle: &AppHandle, server: ServerId) {
    let app = handle.state::<App>();
    let Some(owner) = crate::account_log::shared_owner(&app, server) else { return };
    let Some(raw) = app.invites.lock_or_recover().get(&server).cloned() else { return };
    let Ok(mut ticket) = commy_net::ServerTicket::decode(&raw) else { return };
    let live = app.servers.lock_or_recover().contains_key(&server) && !app.awaits_snapshot(server);
    if ticket.owner == Some(owner) || (ticket.owner.is_some() && live) {
        return;
    }
    ticket.owner = Some(owner);
    app.invites.lock_or_recover().insert(server, ticket.encode());
    app.persist_invites();
}

fn wanted_now(app: &App, me: UserId) -> HashMap<ServerId, Vec<Want>> {
    let Some(identity) = app.identity() else { return HashMap::new() };
    let holder = crate::channel_lock::holder_of(app);
    let sids: Vec<ServerId> = app.servers.lock_or_recover().keys().copied().collect();
    let awaiting: HashSet<ServerId> = sids.iter().copied().filter(|sid| app.awaits_snapshot(*sid)).collect();
    let mut given: HashMap<ServerId, (KeyId, ChannelSecret)> = sids.iter().filter_map(|sid| Some((*sid, given_key(app, *sid)?))).collect();
    let now = now_ms();
    let servers = app.servers.lock_or_recover();
    sids.into_iter()
        .filter_map(|sid| {
            let state = servers.get(&sid)?;
            let given = given.remove(&sid);
            if awaiting.contains(&sid) {
                let (key, secret) = given?;
                return Some((sid, vec![streams::bootstrap(sid, key, &secret)]));
            }
            let community = holder.community_ring(state, given.clone())?;
            let locked = |channel| Keyring::unlock(state, channel, me, &identity);
            let mut wants = streams::wanted(state, me, &community, locked, now);
            let stranded = !wants.iter().any(|w| w.lane == Lane::State) && state.server.is_current_member(me) && !state.is_deleted();
            if let Some((key, secret)) = given.filter(|_| stranded) {
                wants.push(streams::bootstrap(sid, key, &secret));
            }
            (!wants.is_empty()).then_some((sid, wants))
        })
        .collect()
}

fn known_hosts(app: &App) -> Vec<(ServerId, Option<String>, bool)> {
    let sids: Vec<ServerId> = app.servers.lock_or_recover().keys().copied().collect();
    let awaiting: HashSet<ServerId> = sids.into_iter().filter(|sid| app.awaits_snapshot(*sid)).collect();
    let servers = app.servers.lock_or_recover();
    servers.iter().map(|(sid, state)| (*sid, state.server.sync_host.clone(), awaiting.contains(sid))).collect()
}

fn chosen_hosts(app: &App) -> HashMap<ServerId, Option<String>> {
    known_hosts(app)
        .into_iter()
        .map(|(sid, host, awaiting)| {
            let chosen = if awaiting { host.or_else(|| invited_host(sid)).or_else(|| crate::account_log::community_host(app, sid)) } else { host };
            (sid, chosen)
        })
        .collect()
}

fn record_hosts(app: &App) {
    let Some(account_url) = LIVE.lock_or_recover().as_ref().map(|live| live.url.clone()) else { return };
    let settled: Vec<(ServerId, Option<String>)> = known_hosts(app)
        .into_iter()
        .filter(|(_, host, awaiting)| !awaiting || host.is_some())
        .filter_map(|(sid, host, _)| Some((sid, commy_view::away::remembered_host(host.as_deref(), place_of(sid).as_deref(), &account_url)?)))
        .collect();
    crate::account_log::record_sync_hosts(app, settled);
}

fn owners_homes(app: &App) -> HashMap<ServerId, String> {
    let Some(mine) = crate::away_homes::own_home(app) else { return HashMap::new() };
    let servers = app.servers.lock_or_recover();
    servers.iter().filter_map(|(sid, state)| Some((*sid, commy_view::away::owners_away_home(state, &mine)?))).collect()
}

fn community_place(chosen: Option<&str>, owners_home: Option<&str>, account_url: &str) -> Option<Place> {
    let place = commy_view::away::community_place(chosen, owners_home, account_url, crate::away_homes::known_sync)?;
    match &place {
        Place::At(url) if crate::server_guard::blocks_origin(url) => None,
        Place::Unknown(home) if crate::server_guard::blocks_origin(home) => None,
        _ => Some(place),
    }
}

pub fn place_of(server: ServerId) -> Option<String> {
    LIVE.lock_or_recover().as_ref()?.communities.get(&server).map(|community| community.url.clone())
}

pub fn community_url(chosen: Option<&str>, account_url: &str) -> Option<String> {
    commy_core::sync_host::chosen_socket(chosen, Some(account_url)).filter(|url| !crate::server_guard::blocks_origin(url))
}

fn drop_community(live: &mut Live, sid: ServerId) {
    live.opening.remove(&sid);
    live.checkpoints.retain(|_, read| read.server != sid);
    if let Some(community) = live.communities.remove(&sid) {
        if live.reading.as_ref().is_some_and(|r| community.wants.contains_key(&r.stream)) {
            if let Some(stale) = live.reading.take() {
                stale.task.abort();
            }
        }
        for stream in community.wants.keys() {
            live.woken.remove(stream);
            live.fresh.remove(stream);
            live.caught_up.remove(stream);
            live.current.remove(stream);
            live.lost.remove(stream);
            live.refused.remove(stream);
            live.seen.remove(stream);
            live.backfilling.remove(stream);
        }
    }
    live.snapshots.remove(&sid);
    live.published.remove(&sid);
}

fn refresh(handle: &AppHandle) {
    let app = handle.state::<App>();
    let Some(me) = LIVE.lock_or_recover().as_ref().map(|l| l.me) else { return };
    let mut wanted = wanted_now(&app, me);
    let hosts = chosen_hosts(&app);
    let owners = owners_homes(&app);
    let owned: HashSet<ServerId> = app.servers.lock_or_recover().iter().filter(|(_, state)| state.server.owner == me).map(|(sid, _)| *sid).collect();
    let mut unknown_homes = Vec::new();
    let mut fresh_communities = Vec::new();
    let mut promoted = Vec::new();
    let mut guard = LIVE.lock_or_recover();
    let Some(live) = guard.as_mut() else { return };
    let gone: Vec<ServerId> = live.communities.keys().filter(|sid| !wanted.contains_key(sid)).copied().collect();
    for sid in gone {
        drop_community(live, sid);
        live.removed.forget(sid);
    }
    let everything: HashSet<String> = wanted.values().flatten().map(|w| w.keys.id.to_hex()).collect();
    live.cursors.keep_only(&everything);
    live.cursors.keep_hosts_of(&wanted.keys().copied().collect());
    for (sid, wants) in wanted.drain() {
        let chosen = hosts.get(&sid).cloned().flatten();
        let place = community_place(chosen.as_deref(), owners.get(&sid).map(String::as_str), &live.url);
        let url = match place {
            Some(Place::At(url)) => url,
            Some(Place::Unknown(home)) => {
                unknown_homes.push(home);
                drop_community(live, sid);
                continue;
            }
            None => {
                drop_community(live, sid);
                continue;
            }
        };
        if live.communities.get(&sid).is_some_and(|community| community.url != url) {
            tracing::info!("{sid} moved to another sync host");
            drop_community(live, sid);
        }
        let streams: Vec<StreamId> = wants.iter().map(|w| w.keys.id).collect();
        let account_url = live.url.clone();
        if live.cursors.settle_host(sid, &url, &account_url, &streams) {
            save_soon();
        }
        let fresh = !live.communities.contains_key(&sid);
        let events = live.events.clone();
        let community = live.communities.entry(sid).or_insert_with(|| Community::new(&url, events, owned.contains(&sid)));
        community.owned = owned.contains(&sid);
        let keep: HashSet<StreamId> = wants.iter().map(|w| w.keys.id).collect();
        let dropped: Vec<StreamId> = community.wants.keys().filter(|id| !keep.contains(id)).copied().collect();
        for stream in dropped {
            community.wants.remove(&stream);
            community.forget(stream);
            live.woken.remove(&stream);
            live.caught_up.remove(&stream);
            live.current.remove(&stream);
            live.lost.remove(&stream);
            live.refused.remove(&stream);
            live.seen.remove(&stream);
        }
        let mut group = Vec::new();
        for want in wants {
            let stream = want.keys.id;
            if let Some(held) = community.wants.get_mut(&stream) {
                if want.writes && !held.writes && held.lane == Lane::State && live.caught_up.contains(&stream) {
                    promoted.push(sid);
                }
                held.writes = want.writes;
                continue;
            }
            let after = live.cursors.after(stream);
            if after == 0 && matches!(want.lane, Lane::Channel(_)) {
                live.fresh.insert(stream);
            }
            group.push((want.lane, want.keys.clone(), after));
            community.wants.insert(stream, want);
        }
        if group.is_empty() {
            continue;
        }
        if fresh {
            tracing::info!("following {} sync streams for {sid}", group.len());
            fresh_communities.push((sid, group));
        } else {
            community.place(sid, group);
        }
    }
    drop(guard);
    if !unknown_homes.is_empty() {
        crate::away_homes::look_up(handle, unknown_homes);
    }
    for sid in promoted {
        announce_me(handle, sid);
    }
    let opening = fresh_communities.len();
    let focus = *FOCUS.lock_or_recover();
    for (sid, group) in fresh_communities {
        let spread = first_open_delay(sid, opening, focus);
        if let Some(live) = LIVE.lock_or_recover().as_mut() {
            live.opening.insert(sid, group);
        }
        if spread.is_zero() {
            open_waiting(sid);
            continue;
        }
        crate::account_session::spawn(async move {
            tokio::time::sleep(spread).await;
            open_waiting(sid);
        });
    }
    crate::push_wake::register(handle);
}

fn first_open_delay(sid: ServerId, opening: usize, focus: Option<ServerId>) -> Duration {
    if opening <= 1 || focus == Some(sid) {
        return Duration::ZERO;
    }
    Duration::from_millis(std::hash::BuildHasher::hash_one(&std::collections::hash_map::RandomState::new(), sid) % FIRST_OPEN_SPREAD_MS)
}

fn open_waiting(server: ServerId) {
    let mut guard = LIVE.lock_or_recover();
    let Some(live) = guard.as_mut() else { return };
    let Some(group) = live.opening.remove(&server) else { return };
    let awaiting = live.handle.state::<App>().awaits_snapshot(server);
    let Some(community) = live.communities.get_mut(&server) else { return };
    let still: OpenGroup = group.into_iter().filter(|(_, keys, _)| community.wants.contains_key(&keys.id)).collect();
    let (first_checkpoint, rest): (OpenGroup, OpenGroup) = still
        .into_iter()
        .partition(|(lane, keys, _)| awaiting && *lane == Lane::State && !live.checkpoint_tried.contains(&keys.id));
    community.place(server, rest);
    for (_, state, _) in first_checkpoint {
        let keys = checkpoint::keys_for(&state);
        community.pool.follow_together_tail(vec![(keys.clone(), 0)], Some(checkpoint::FIRST_LOOK));
        live.checkpoints.insert(keys.id, CheckpointRead { server, state, assembler: Assembler::default(), looked_back: false });
        let handle = live.handle.clone();
        crate::account_session::spawn(async move {
            tokio::time::sleep(CHECKPOINT_PATIENCE).await;
            end_checkpoint_read(&handle, keys.id);
        });
    }
}

fn on_checkpoint_event(handle: &AppHandle, event: &SyncEvent) -> bool {
    let stream = match event {
        SyncEvent::Entry { stream, .. } | SyncEvent::Unreadable { stream, .. } | SyncEvent::CaughtUp { stream, .. } | SyncEvent::Refused { stream, .. } => *stream,
        _ => return false,
    };
    let caught_up = {
        let mut guard = LIVE.lock_or_recover();
        let Some(live) = guard.as_mut() else { return false };
        let Some(read) = live.checkpoints.get_mut(&stream) else { return false };
        match event {
            SyncEvent::Entry { seq, plain, .. } => {
                read.assembler.take(*seq, plain);
                return true;
            }
            SyncEvent::CaughtUp { .. } => match (read.looked_back, read.assembler.read_again_from()) {
                (false, Some(from)) => {
                    read.looked_back = true;
                    if let Some(community) = live.communities.get_mut(&read.server) {
                        community.pool.unfollow(stream);
                        community.pool.follow_together_tail(vec![(checkpoint::keys_for(&read.state), from)], None);
                    }
                    false
                }
                _ => true,
            },
            SyncEvent::Refused { .. } => true,
            _ => false,
        }
    };
    if caught_up {
        end_checkpoint_read(handle, stream);
    }
    true
}

fn end_checkpoint_read(handle: &AppHandle, stream: StreamId) {
    let Some(read) = LIVE.lock_or_recover().as_mut().and_then(|live| live.checkpoints.remove(&stream)) else { return };
    let server = read.server;
    let start = adopt_checkpoint(handle, &read);
    {
        let mut guard = LIVE.lock_or_recover();
        let Some(live) = guard.as_mut() else { return };
        live.checkpoint_tried.insert(read.state.id);
        let Some(community) = live.communities.get_mut(&server) else { return };
        community.pool.unfollow(stream);
        if !community.wants.contains_key(&read.state.id) {
            return;
        }
        match start {
            Some(after) => {
                live.cursors.start_at(read.state.id, after);
                community.pool.follow_together_tail(vec![(read.state, after)], None);
            }
            None => {
                let after = live.cursors.after(read.state.id);
                community.pool.follow_together_tail(vec![(read.state, after)], Some(streams::STATE_TAIL));
            }
        }
    }
    save_soon();
    release_parked(handle);
    refresh_soon(handle);
}

fn adopt_checkpoint(handle: &AppHandle, read: &CheckpointRead) -> Option<u64> {
    let app = handle.state::<App>();
    let (_, owner) = app.ticket_owner(read.server)?;
    let found = read.assembler.newest(read.server, &owner)?;
    let after = found.state_after();
    if !crate::catchup::apply_service_snapshot(handle, read.server, found.snapshot) {
        return None;
    }
    tracing::info!("took the owner's full copy of {} from its checkpoint on the sync service", read.server);
    Some(after)
}

pub fn focus(server: ServerId) {
    *FOCUS.lock_or_recover() = Some(server);
    crate::account_session::spawn(async move {
        open_waiting(server);
        wake_all(server);
    });
}

fn wake_all(server: ServerId) {
    let mut guard = LIVE.lock_or_recover();
    let Some(live) = guard.as_mut() else { return };
    let Some(community) = live.communities.get_mut(&server) else { return };
    let dormant: Vec<StreamId> = community.dormant.iter().copied().collect();
    for stream in dormant {
        community.wake(stream, &live.cursors);
    }
}

fn wake_lane(live: &mut Live, server: ServerId, lane: Lane) {
    let Some(community) = live.communities.get_mut(&server) else { return };
    let dormant: Vec<StreamId> = community.dormant.iter().filter(|stream| community.wants.get(stream).is_some_and(|want| want.lane == lane)).copied().collect();
    for stream in dormant {
        community.wake(stream, &live.cursors);
    }
}

fn on_head(stream: StreamId, head: u64) {
    let mut guard = LIVE.lock_or_recover();
    let Some(live) = guard.as_mut() else { return };
    let Some(community) = live.communities.values_mut().find(|community| community.dormant.contains(&stream)) else { return };
    let mut baseline = community.baseline.get(&stream).copied();
    let wake = changed(live.cursors.after(stream), &mut baseline, head);
    if let Some(first) = baseline {
        community.baseline.insert(stream, first);
    }
    if wake && community.wake(stream, &live.cursors) {
        live.woken.insert(stream);
    }
}

fn on_cannot_watch(streams: Vec<StreamId>) {
    let mut guard = LIVE.lock_or_recover();
    let Some(live) = guard.as_mut() else { return };
    for stream in streams {
        if let Some(community) = live.communities.values_mut().find(|community| community.dormant.contains(&stream)) {
            community.watchless = true;
            community.wake(stream, &live.cursors);
        }
    }
}

fn doze_quiet_channels(live: &mut Live) {
    let focus = *FOCUS.lock_or_recover();
    let reading = live.reading.as_ref().map(|reading| reading.stream);
    for (server, community) in live.communities.iter_mut() {
        if community.owned || community.watchless || focus == Some(*server) {
            continue;
        }
        if !community.pool.greeting_at(&community.url).is_some_and(|greeting| greeting.protocol >= WATCH_SINCE) {
            continue;
        }
        let now = Instant::now();
        let quiet: Vec<StreamId> = community
            .wants
            .iter()
            .filter(|(stream, want)| {
                matches!(want.lane, Lane::Channel(_))
                    && !community.dormant.contains(*stream)
                    && live.caught_up.contains(*stream)
                    && !live.backfilling.contains(*stream)
                    && reading != Some(**stream)
            })
            .map(|(stream, _)| *stream)
            .filter(|stream| now.duration_since(*community.heard.entry(*stream).or_insert(now)) >= DOZE_AFTER)
            .collect();
        community.doze(&quiet);
        for stream in quiet {
            live.caught_up.remove(&stream);
            live.current.remove(&stream);
            live.lost.remove(&stream);
        }
    }
}

fn follow_group(pool: &mut Pool, group: OpenGroup) {
    let (lasting, texts): (Vec<_>, Vec<_>) = group.into_iter().partition(|(lane, _, _)| *lane == Lane::State);
    for (part, tail) in [(lasting, streams::STATE_TAIL), (texts, streams::FRESH_TAIL)] {
        let part: Vec<_> = part.into_iter().map(|(_, keys, after)| (keys, after)).collect();
        if !part.is_empty() {
            pool.follow_together_tail(part, Some(tail));
        }
    }
}

fn targets(live: &Live, server: ServerId, lane: Lane, key_event: bool) -> Vec<(StreamId, Option<SyncClient>)> {
    let Some(community) = live.communities.get(&server) else { return Vec::new() };
    community
        .wants
        .iter()
        .filter(|(stream, want)| live.caught_up.contains(*stream) && if key_event { want.lane == Lane::State } else { want.lane == lane && want.writes })
        .map(|(stream, _)| (*stream, community.pool.client_for(*stream)))
        .collect()
}

pub fn publish(server: ServerId, payload: &WirePayload) {
    if streams::lane_of_payload(payload).is_none() {
        signal(server, payload);
        return;
    }
    publish_held(Unsent { server, payload: payload.clone(), at: now_ms() });
    if streams::event_of(payload).is_some_and(|(event, _)| streams::changes_lanes(event)) {
        let handle = LIVE.lock_or_recover().as_ref().map(|live| live.handle.clone());
        if let Some(handle) = handle {
            refresh_soon(&handle);
        }
    }
}

fn publish_held(held: Unsent) {
    let (server, payload) = (held.server, &held.payload);
    let Some(lane) = streams::lane_of_payload(payload) else { return };
    let key_event = streams::event_of(payload).is_none_or(|(event, _)| streams::is_key_event(event));
    let Some(plain) = streams::encode_entry(payload) else { return };
    let message = match streams::event_of(payload) {
        Some((ServerEvent::MessageCreate { message }, _)) => Some(message.id),
        _ => None,
    };
    let found = {
        let mut guard = LIVE.lock_or_recover();
        guard.as_mut().map(|live| {
            let sends = targets(live, server, lane, key_event);
            if !sends.is_empty() {
                live.note_echo(digest(&plain));
            } else if matches!(lane, Lane::Channel(_)) {
                wake_lane(live, server, lane);
            }
            (sends, live.handle.clone())
        })
    };
    let Some((sends, handle)) = found.filter(|(sends, _)| !sends.is_empty()) else {
        hold(held);
        return;
    };
    crate::account_session::spawn(async move {
        let mut retry = false;
        for (stream, client) in sends {
            match sync_link::append(client, stream, plain.clone()).await {
                Ok(_) => {
                    if let Some(message) = message {
                        stored(&handle, server, message);
                    }
                }
                Err(e) => {
                    tracing::debug!("could not add a server change to the sync service: {e}");
                    retry |= worth_retrying(&e);
                }
            }
        }
        if retry {
            hold(held);
        }
    });
}

pub fn signal(server: ServerId, payload: &WirePayload) {
    let Some((event, _)) = streams::event_of(payload) else { return };
    let lane = streams::signal_lane(event);
    let again = {
        let guard = LIVE.lock_or_recover();
        let Some(live) = guard.as_ref() else { return };
        let Some(plain) = streams::encode_signal(live.node, payload) else { return };
        for (stream, client) in targets(live, server, lane, false) {
            if let Some(client) = client {
                client.signal(stream, plain.clone());
            }
        }
        let online = matches!(event, ServerEvent::PresenceUpdate { user, presence } if *user == live.me && presence.is_visible_online());
        online.then(|| live.handle.clone())
    };
    if let Some(handle) = again {
        leave_word(&handle.state::<App>(), server);
    }
}

pub fn leave_word(app: &App, server: ServerId) {
    let Some(me) = app.profile.lock_or_recover().as_ref().map(|p| p.id) else { return };
    let payload = app.wire_at(server, ServerEvent::PresenceUpdate { user: me, presence: Presence::Offline }, now_ms() + 1);
    let guard = LIVE.lock_or_recover();
    let Some(live) = guard.as_ref().filter(|live| live.me == me) else { return };
    let Some(plain) = streams::encode_signal(live.node, &payload) else { return };
    for (stream, client) in targets(live, server, Lane::State, false) {
        if let Some(client) = client {
            client.will(stream, plain.clone());
        }
    }
}

fn say_here(handle: &AppHandle, server: ServerId) {
    if let Some(live) = LIVE.lock_or_recover().as_mut() {
        live.greeted.insert(server, Instant::now());
    }
    for payload in crate::liveness::here(&handle.state::<App>(), server) {
        signal(server, &payload);
    }
}

fn greet(handle: &AppHandle, server: ServerId) {
    {
        let mut guard = LIVE.lock_or_recover();
        let Some(live) = guard.as_mut() else { return };
        let now = Instant::now();
        if live.greeted.get(&server).is_some_and(|at| now.duration_since(*at) < GREET_GAP) {
            return;
        }
        live.greeted.insert(server, now);
    }
    let handle = handle.clone();
    crate::account_session::spawn(async move {
        tokio::time::sleep(Duration::from_millis(rand::random::<u64>() % GREET_SPREAD_MS)).await;
        say_here(&handle, server);
    });
}

pub async fn publish_now(server: ServerId, payload: &WirePayload) -> Vec<(StreamId, u64)> {
    let Some(lane) = streams::lane_of_payload(payload) else { return Vec::new() };
    let key_event = streams::event_of(payload).is_none_or(|(event, _)| streams::is_key_event(event));
    let Some(plain) = streams::encode_entry(payload) else { return Vec::new() };
    let sends = {
        let mut guard = LIVE.lock_or_recover();
        let Some(live) = guard.as_mut() else { return Vec::new() };
        let sends = targets(live, server, lane, key_event);
        if !sends.is_empty() {
            live.note_echo(digest(&plain));
        }
        sends
    };
    let mut stored = Vec::new();
    for (stream, client) in sends {
        if let Ok(Ok(seq)) = tokio::time::timeout(PUBLISH_WAIT, sync_link::append(client, stream, plain.clone())).await {
            stored.push((stream, seq));
        }
    }
    stored
}

pub fn community_hosts() -> HashMap<ServerId, String> {
    let guard = LIVE.lock_or_recover();
    let Some(live) = guard.as_ref() else { return HashMap::new() };
    live.communities.iter().map(|(sid, community)| (*sid, community.url.clone())).collect()
}

pub fn followed_streams() -> HashMap<ServerId, Vec<commy_sync::StreamKeys>> {
    let guard = LIVE.lock_or_recover();
    let Some(live) = guard.as_ref() else { return HashMap::new() };
    live.communities
        .iter()
        .map(|(sid, community)| {
            let checkpoints = community.wants.values().filter(|want| want.lane == Lane::State).map(|want| checkpoint::keys_for(&want.keys));
            (*sid, community.wants.values().map(|want| want.keys.clone()).chain(checkpoints).collect())
        })
        .collect()
}

pub fn wake_keys(server: ServerId, private: Option<commy_core::ChannelId>) -> Option<commy_sync::StreamKeys> {
    let guard = LIVE.lock_or_recover();
    let community = guard.as_ref()?.communities.get(&server)?;
    community.wants.values().find(|want| wake_point(want) == Some(private)).map(|want| want.keys.clone())
}

pub fn wake_points() -> Vec<(ServerId, Option<commy_core::ChannelId>, commy_sync::StreamKeys)> {
    let guard = LIVE.lock_or_recover();
    let Some(live) = guard.as_ref() else { return Vec::new() };
    live.communities
        .iter()
        .flat_map(|(server, community)| {
            community.wants.values().filter_map(|want| wake_point(want).map(|channel| (*server, channel, want.keys.clone())))
        })
        .collect()
}

pub fn profile_digest(profile: &UserProfile) -> Option<String> {
    let mut lasting = profile.clone();
    lasting.presence = Default::default();
    let bytes = serde_json::to_vec(&lasting).ok()?;
    Some(digest(&bytes).iter().map(|b| format!("{b:02x}")).collect())
}

pub fn announce_profile(server: ServerId, profile: &UserProfile, payload: &WirePayload) {
    announce(server, profile, payload);
}

fn announce(server: ServerId, profile: &UserProfile, payload: &WirePayload) -> Vec<tauri::async_runtime::JoinHandle<bool>> {
    let Some(profile) = profile_digest(profile) else { return Vec::new() };
    let Some(plain) = streams::encode_entry(payload) else { return Vec::new() };
    let sends = {
        let mut guard = LIVE.lock_or_recover();
        let Some(live) = guard.as_mut() else { return Vec::new() };
        let fresh: Vec<_> = targets(live, server, Lane::State, false)
            .into_iter()
            .filter(|(stream, _)| live.cursors.announce(*stream, &profile))
            .collect();
        if !fresh.is_empty() {
            live.note_echo(digest(&plain));
        }
        fresh
    };
    if sends.is_empty() {
        return Vec::new();
    }
    save_soon();
    sends
        .into_iter()
        .map(|(stream, client)| {
            let (plain, profile) = (plain.clone(), profile.clone());
            crate::account_session::spawn(async move {
                let Err(e) = sync_link::append(client, stream, plain).await else { return true };
                tracing::debug!("could not tell the sync service where this device is: {e}");
                if let Some(live) = LIVE.lock_or_recover().as_mut() {
                    live.cursors.unannounce(stream, &profile);
                }
                save_soon();
                false
            })
        })
        .collect()
}

fn announce_me(handle: &AppHandle, server: ServerId) {
    let app = handle.state::<App>();
    let Some(me) = app.profile.lock_or_recover().clone() else { return };
    let mut profile = app.announced(me);
    {
        let servers = app.servers.lock_or_recover();
        let Some(state) = servers.get(&server).filter(|s| s.server.is_current_member(profile.id) && !s.is_deleted()) else { return };
        crate::devices::point_at_voice(state, &mut profile);
    }
    let payload = app.wire_for(server, ServerEvent::MemberJoin { server, profile: profile.clone() });
    let sends = announce(server, &profile, &payload);
    if sends.is_empty() {
        return;
    }
    let handle = handle.clone();
    crate::account_session::spawn(async move {
        let mut stored = false;
        for send in sends {
            stored |= send.await.unwrap_or(false);
        }
        if stored {
            say_here(&handle, server);
        }
    });
}

async fn listen(handle: AppHandle, mut events: mpsc::UnboundedReceiver<SyncEvent>) {
    while let Some(event) = events.recv().await {
        on_event(&handle, event);
    }
}

fn on_event(handle: &AppHandle, event: SyncEvent) {
    if on_checkpoint_event(handle, &event) {
        return;
    }
    match event {
        SyncEvent::Entry { stream, seq, plain, .. } => on_entry(handle, stream, seq, &plain),
        SyncEvent::Unreadable { stream, seq } => {
            if let Some(live) = LIVE.lock_or_recover().as_mut() {
                live.cursors.advance(stream, seq);
            }
            save_soon();
        }
        SyncEvent::CaughtUp { stream, seq } => on_caught_up(handle, stream, seq),
        SyncEvent::Signal { stream, plain } => on_signal(handle, stream, &plain),
        SyncEvent::Head { stream, head } => on_head(stream, head),
        SyncEvent::CannotWatch { streams } => on_cannot_watch(streams),
        SyncEvent::Refused { stream, code } => {
            tracing::debug!("the sync service refused a server stream {stream}: {code:?}");
            let removed = LIVE.lock_or_recover().as_mut().and_then(|live| {
                if let Some(community) = live.communities.values_mut().find(|community| community.dormant.contains(&stream)) {
                    community.pool.forget_watch(stream);
                    if code != ErrorCode::Removed {
                        community.watchless = true;
                        community.wake(stream, &live.cursors);
                    }
                }
                live.current.remove(&stream);
                live.refused.insert(stream);
                (code == ErrorCode::Removed).then(|| newly_removed(live, stream)).flatten()
            });
            if let Some(server) = removed {
                tell_host_removed(handle, server);
            }
        }
        SyncEvent::Connected => {
            if let Some(live) = LIVE.lock_or_recover().as_mut() {
                let servers: Vec<ServerId> = live.communities.keys().copied().collect();
                live.say_again.extend(servers);
            }
        }
        SyncEvent::Disconnected { streams } => {
            if let Some(live) = LIVE.lock_or_recover().as_mut() {
                let ours: Vec<StreamId> = streams.into_iter().filter(|stream| live.route(*stream).is_some()).collect();
                for stream in ours {
                    live.current.remove(&stream);
                    live.lost.insert(stream);
                }
            }
        }
    }
}

fn on_entry(handle: &AppHandle, stream: StreamId, seq: u64, plain: &[u8]) {
    let found = {
        let mut guard = LIVE.lock_or_recover();
        let Some(live) = guard.as_mut() else { return };
        let Some((server, want)) = live.route(stream) else { return };
        live.cursors.advance(stream, seq);
        if live.fresh.contains(&stream) && !live.cursors.read_back.contains(&stream.to_hex()) {
            live.cursors.older_before.entry(stream.to_hex()).or_insert(seq);
        }
        let echo = live.take_echo(&digest(plain));
        let live_now = live.caught_up.contains(&stream) || live.woken.contains(&stream);
        if matches!(want.lane, Lane::Channel(_)) {
            if let Some(community) = live.communities.get_mut(&server) {
                community.heard.insert(stream, Instant::now());
            }
        }
        let decoded = (!echo).then(|| streams::decode_entry(plain, want.lane)).flatten();
        if let Some((ServerEvent::MessageCreate { message }, _)) = decoded.as_ref().and_then(streams::event_of) {
            if !live_now {
                live.seen.entry(stream).or_default().insert(message.id);
            }
        }
        decoded.map(|payload| (server, payload, live_now, live.net.clone()))
    };
    save_soon();
    let Some((server, payload, live_now, net)) = found else { return };
    if handle.state::<App>().awaits_snapshot(server) {
        park(server, payload);
        if live_now {
            adopt_snapshot(handle, server);
        }
        return;
    }
    if snapshot_time(&payload).is_some() {
        crate::catchup::learn_service_profiles(handle, server, &payload);
        return;
    }
    let Some((event, action)) = streams::event_of(&payload).map(|(event, action)| (event.clone(), action)) else { return };
    let lanes_changed = streams::changes_lanes(&event) || matches!(event, ServerEvent::SyncHostSet { .. });
    match event {
        ServerEvent::MessageCreate { message } if !rings(live_now, message.created_at, Utc::now()) => {
            crate::catchup::AppStore(handle.clone()).apply(server, vec![message], Vec::new());
        }
        event => {
            let _ = net.send(NetEvent::Event { server, sender: streams::SERVICE_SENDER, event, action });
        }
    }
    if lanes_changed {
        refresh_soon(handle);
    }
}

fn on_signal(handle: &AppHandle, stream: StreamId, plain: &[u8]) {
    let found = {
        let guard = LIVE.lock_or_recover();
        let Some(live) = guard.as_ref() else { return };
        let Some((server, want)) = live.route(stream) else { return };
        let Some((node, payload)) = streams::decode_signal(plain, want.lane) else { return };
        (node != live.node).then(|| (server, node, payload, live.net.clone()))
    };
    let Some((server, node, payload, net)) = found else { return };
    let app = handle.state::<App>();
    if app.awaits_snapshot(server) {
        return;
    }
    let Some((event, action)) = streams::event_of(&payload).map(|(event, action)| (event.clone(), action)) else { return };
    let (admitted, returning) = {
        let servers = app.servers.lock_or_recover();
        let Some(state) = servers.get(&server) else { return };
        let admitted = match action {
            Some(action) => state.admits_signed(&SignedEvent { action, event: event.clone() }).is_ok(),
            None => state.admits_unsigned(&event).is_ok(),
        };
        let returning = match &event {
            ServerEvent::PresenceUpdate { user, presence } => {
                presence.is_visible_online() && !state.profiles.get(user).is_some_and(|p| p.presence.is_visible_online())
            }
            _ => false,
        };
        (admitted, returning)
    };
    if !admitted {
        return;
    }
    if let Some(user) = crate::liveness::speaker(&event, action.as_ref()) {
        crate::liveness::heard_through_service(&app, server, user, Instant::now());
    }
    let _ = net.send(NetEvent::Event { server, sender: node, event, action });
    if returning {
        greet(handle, server);
    }
}

fn snapshot_time(payload: &WirePayload) -> Option<i64> {
    match payload {
        WirePayload::SyncResponse { at, .. } => Some(*at),
        _ => None,
    }
}

fn park(server: ServerId, payload: WirePayload) {
    let mut guard = LIVE.lock_or_recover();
    let Some(live) = guard.as_mut() else { return };
    match snapshot_time(&payload) {
        Some(at) => {
            let newer = live.snapshots.get(&server).and_then(snapshot_time).is_none_or(|held| at > held);
            if newer {
                live.snapshots.insert(server, payload);
            }
        }
        None => {
            let parked = live.parked.entry(server).or_default();
            parked.push_back(payload);
            while parked.len() > MAX_PARKED {
                parked.pop_front();
            }
        }
    }
}

fn adopt_snapshot(handle: &AppHandle, server: ServerId) {
    let Some(snapshot) = LIVE.lock_or_recover().as_mut().and_then(|live| live.snapshots.remove(&server)) else { return };
    if crate::catchup::apply_service_snapshot(handle, server, snapshot) {
        tracing::info!("took the owner's copy of {server} from the sync service");
        release_parked(handle);
        refresh_soon(handle);
    }
}

fn release_parked(handle: &AppHandle) {
    let app = handle.state::<App>();
    let (released, net) = {
        let mut guard = LIVE.lock_or_recover();
        let Some(live) = guard.as_mut() else { return };
        let settled: Vec<ServerId> = live.parked.keys().copied().filter(|sid| !app.awaits_snapshot(*sid)).collect();
        let released: Vec<(ServerId, VecDeque<WirePayload>)> = settled.into_iter().filter_map(|sid| Some((sid, live.parked.remove(&sid)?))).collect();
        (released, live.net.clone())
    };
    for (server, payloads) in released {
        for payload in payloads {
            if let Some((event, action)) = streams::event_of(&payload) {
                let _ = net.send(NetEvent::Event { server, sender: streams::SERVICE_SENDER, event: event.clone(), action });
            }
        }
    }
}

fn publish_snapshots(handle: &AppHandle) {
    let app = handle.state::<App>();
    let now = now_ms();
    let (me, ready) = {
        let guard = LIVE.lock_or_recover();
        let Some(live) = guard.as_ref() else { return };
        let ready: Vec<ServerId> = live
            .communities
            .iter()
            .filter(|(sid, community)| {
                let caught_up = community.wants.iter().any(|(stream, want)| want.lane == Lane::State && want.writes && live.caught_up.contains(stream));
                caught_up && live.published.get(sid).is_none_or(|at| now - at >= SNAPSHOT_EVERY_MS)
            })
            .map(|(sid, _)| *sid)
            .collect();
        (live.me, ready)
    };
    for server in ready {
        let keys = app.servers.lock_or_recover().get(&server).filter(|state| state.server.owner == me).map(key_records);
        let snapshot = keys.is_some().then(|| crate::catchup::service_snapshot_for(&app, server)).flatten();
        if keys.is_some() && snapshot.is_none() {
            continue;
        }
        if let Some(live) = LIVE.lock_or_recover().as_mut() {
            live.published.insert(server, now);
        }
        for payload in snapshot.into_iter().chain(keys.unwrap_or_default()) {
            publish(server, &payload);
        }
        if app.servers.lock_or_recover().get(&server).is_some_and(|state| state.server.owner == me) {
            publish_checkpoints(handle, server);
        }
    }
}

fn publish_checkpoints(handle: &AppHandle, server: ServerId) {
    let app = handle.state::<App>();
    let (Some(identity), Some(snapshot)) = (app.identity(), crate::catchup::checkpoint_snapshot_for(&app, server)) else { return };
    let writes: Vec<(commy_sync::StreamKeys, u64, SyncClient, String)> = {
        let mut guard = LIVE.lock_or_recover();
        let Some(live) = guard.as_mut() else { return };
        let Some(community) = live.communities.get_mut(&server) else { return };
        let states: Vec<(commy_sync::StreamKeys, u64)> = community
            .wants
            .iter()
            .filter(|(stream, want)| want.lane == Lane::State && live.caught_up.contains(*stream))
            .map(|(stream, want)| (checkpoint::keys_for(&want.keys), live.cursors.after(*stream)))
            .collect();
        states
            .into_iter()
            .filter_map(|(keys, covers)| {
                community.pool.follow_together_tail(vec![(keys.clone(), 0)], Some(0));
                let client = community.pool.client_for(keys.id)?;
                Some((keys, covers, client, community.url.clone()))
            })
            .collect()
    };
    for (keys, covers, client, url) in writes {
        let Some(records) = checkpoint::records(&snapshot, covers, &identity) else {
            stop_writing_checkpoint(server, keys.id);
            continue;
        };
        let root = identity.to_bytes();
        crate::account_session::spawn(async move {
            let written = write_checkpoint(&client, keys.id, records).await;
            stop_writing_checkpoint(server, keys.id);
            let Some(first) = written else { return };
            tracing::info!("wrote the checkpoint copy of {server} to the sync service");
            if first > 1 {
                let work = commy_sync::errands::owner_work(&root, &[keys], |_| commy_sync::errands::Errand::Erase { through: first - 1 });
                if let Err(e) = commy_sync::errands::run(&url, work).await {
                    tracing::debug!("older checkpoint copies stay until they expire: {e}");
                }
            }
        });
    }
}

async fn write_checkpoint(client: &SyncClient, stream: StreamId, records: Vec<Vec<u8>>) -> Option<u64> {
    let mut first = None;
    for plain in records {
        match tokio::time::timeout(PUBLISH_WAIT, client.append(stream, plain)).await {
            Ok(Ok(seq)) => {
                first.get_or_insert(seq);
            }
            Ok(Err(e)) => {
                tracing::debug!("could not write a checkpoint copy: {e}");
                return None;
            }
            Err(_) => {
                tracing::debug!("the sync service took too long to store a checkpoint copy");
                return None;
            }
        }
    }
    first
}

fn stop_writing_checkpoint(server: ServerId, stream: StreamId) {
    if let Some(community) = LIVE.lock_or_recover().as_mut().and_then(|live| live.communities.get_mut(&server)) {
        community.pool.unfollow(stream);
    }
}

fn on_caught_up(handle: &AppHandle, stream: StreamId, seq: u64) {
    let found = {
        let mut guard = LIVE.lock_or_recover();
        let Some(live) = guard.as_mut() else { return };
        let Some((server, want)) = live.route(stream) else { return };
        live.cursors.advance(stream, seq);
        live.woken.remove(&stream);
        let first = live.caught_up.insert(stream);
        live.current.insert(stream);
        live.lost.remove(&stream);
        if first && want.lane == Lane::State {
            tracing::info!("{server} is up to date from the sync service {} ms after launch", crate::startup::since_launch().as_millis());
        }
        let held = live.snapshots.contains_key(&server);
        let due = want.writes && !live.cursors.backfilled.contains(&stream.to_hex()) && !live.backfilling.contains(&stream);
        let again = want.lane == Lane::State && want.writes && live.say_again.remove(&server);
        (server, want, due, live.me, first, held, again)
    };
    save_soon();
    let (server, want, due, me, first, held, again) = found;
    if first {
        send_unsent();
    }
    let empty = first && !held;
    let app = handle.state::<App>();
    if app.awaits_snapshot(server) {
        if empty {
            tracing::info!("{server} waits for its owner's copy on the sync service");
        }
        adopt_snapshot(handle, server);
        return;
    }
    if want.lane == Lane::State && want.writes {
        if first {
            announce_me(handle, server);
        }
        if first || again {
            say_here(handle, server);
        }
        publish_snapshots(handle);
    }
    if !due {
        return;
    }
    let seen = LIVE.lock_or_recover().as_mut().and_then(|live| live.seen.remove(&stream)).unwrap_or_default();
    let payloads = {
        let servers = app.servers.lock_or_recover();
        servers.get(&server).filter(|s| s.server.owner == me).map(|state| backfill_for(state, want.lane, &seen))
    };
    let Some(payloads) = payloads else { return };
    if !LIVE.lock_or_recover().as_mut().is_some_and(|live| live.backfilling.insert(stream)) {
        return;
    }
    crate::account_session::spawn(async move {
        let done = fill(stream, payloads).await;
        if let Some(live) = LIVE.lock_or_recover().as_mut() {
            live.backfilling.remove(&stream);
            if done {
                live.cursors.backfilled.insert(stream.to_hex());
            }
        }
        save_soon();
    });
}

pub fn state_link(server: ServerId) -> Option<StateLink> {
    let guard = LIVE.lock_or_recover();
    let live = guard.as_ref()?;
    let Some(community) = live.communities.get(&server) else { return Some(StateLink::NotUsed) };
    let streams: Vec<StreamId> = community.wants.iter().filter(|(_, want)| want.lane == Lane::State).map(|(stream, _)| *stream).collect();
    let usable: Vec<StreamId> = streams.into_iter().filter(|stream| !live.refused.contains(stream)).collect();
    Some(link_of(&usable, &live.current, &live.lost))
}

pub fn backfill_for(state: &commy_core::ServerState, lane: Lane, seen: &HashSet<MessageId>) -> Vec<WirePayload> {
    let payloads: Vec<WirePayload> = match lane {
        Lane::Channel(channel) => streams::backfill_messages(state, channel, seen, now_ms()),
        Lane::State => state
            .signed_actions()
            .into_iter()
            .filter(|signed| streams::lane_of(&signed.event) == Some(Lane::State))
            .map(|signed| WirePayload::Signed(signed.clone()))
            .collect(),
    };
    payloads.into_iter().take(MAX_BACKFILL_PER_STREAM).collect()
}

pub fn key_records(state: &commy_core::ServerState) -> Vec<WirePayload> {
    state
        .signed_actions()
        .into_iter()
        .filter(|signed| matches!(signed.event, ServerEvent::CommunityKeyAdd { .. }))
        .map(|signed| WirePayload::Signed(signed.clone()))
        .collect()
}

async fn fill(stream: StreamId, payloads: Vec<WirePayload>) -> bool {
    for payload in payloads {
        let Some(plain) = streams::encode_entry(&payload) else { continue };
        let client = {
            let mut guard = LIVE.lock_or_recover();
            let Some(live) = guard.as_mut() else { return false };
            let Some(client) = live.communities.values().find_map(|c| c.wants.contains_key(&stream).then(|| c.pool.client_for(stream)).flatten())
            else {
                return false;
            };
            live.note_echo(digest(&plain));
            client
        };
        match client.append(stream, plain).await {
            Ok(_) | Err(AppendError::Seal(_)) => {}
            Err(e) => {
                tracing::debug!("backfilling a server stream stopped: {e}");
                return false;
            }
        }
    }
    true
}

pub fn forget() {
    INVITED.lock_or_recover().clear();
    INVITED_HOSTS.lock_or_recover().clear();
    UNSENT.lock_or_recover().clear();
    *FOCUS.lock_or_recover() = None;
    SAVE_PENDING.store(false, Ordering::SeqCst);
    REFRESH_PENDING.store(false, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Identity, Message, Server, ServerState};

    const UNREACHABLE_SYNC_URL: &str = "wss://sync.invalid";

    fn one_state_stream() -> (commy_sync::StreamId, OpenGroup) {
        let keys = commy_sync::StreamSecret::random().keys();
        (keys.id, vec![(Lane::State, keys, 0)])
    }

    #[test]
    #[should_panic]
    fn a_fresh_pool_cannot_open_its_first_link_outside_the_async_runtime() {
        let (events, _received) = mpsc::unbounded_channel();
        let mut pool = Pool::with_events(UNREACHABLE_SYNC_URL, events);
        let (_, group) = one_state_stream();
        follow_group(&mut pool, group);
    }

    #[test]
    fn a_fresh_pool_opens_its_first_link_on_the_async_runtime() {
        let (events, _received) = mpsc::unbounded_channel();
        let (stream, group) = one_state_stream();
        let followed = tauri::async_runtime::block_on(crate::account_session::spawn(async move {
            let mut pool = Pool::with_events(UNREACHABLE_SYNC_URL, events);
            follow_group(&mut pool, group);
            pool.follows(stream)
        }));
        assert!(followed.unwrap());
    }

    #[test]
    fn one_new_server_opens_at_once_and_many_spread_out() {
        let sid = ServerId::new();
        assert_eq!(first_open_delay(sid, 1, None), Duration::ZERO, "a server made or joined just now is not held back");
        let spread: Vec<Duration> = (0..20).map(|_| first_open_delay(ServerId::new(), 20, None)).collect();
        assert!(spread.iter().all(|d| *d < Duration::from_millis(FIRST_OPEN_SPREAD_MS)));
        assert!(spread.iter().any(|d| *d > Duration::ZERO));
    }

    #[test]
    fn the_server_on_screen_opens_at_once_while_the_rest_spread_out() {
        let shown = ServerId::new();
        assert!((0..20).all(|_| first_open_delay(shown, 20, Some(shown)) == Duration::ZERO));
        let others: Vec<Duration> = (0..20).map(|_| first_open_delay(ServerId::new(), 20, Some(shown))).collect();
        assert!(others.iter().any(|d| *d > Duration::ZERO));
    }

    #[test]
    fn changes_the_service_missed_wait_once_each_and_not_forever() {
        let (a, b) = (ServerId::new(), ServerId::new());
        let first = WirePayload::Event(ServerEvent::PublicSet { server: a, public: true });
        let second = WirePayload::Event(ServerEvent::PublicSet { server: b, public: false });
        let now = 10 * UNSENT_FOR_MS;
        let mut unsent = Vec::new();
        hold_unsent(&mut unsent, Unsent { server: a, payload: first.clone(), at: now - 10 }, now);
        hold_unsent(&mut unsent, Unsent { server: a, payload: first.clone(), at: now }, now);
        assert_eq!(unsent.len(), 1, "the same change waits once");
        hold_unsent(&mut unsent, Unsent { server: b, payload: second.clone(), at: now - UNSENT_FOR_MS }, now);
        assert_eq!(unsent.len(), 1, "a week-old change is dropped");
        hold_unsent(&mut unsent, Unsent { server: b, payload: second, at: now + 10 }, now + UNSENT_FOR_MS);
        assert_eq!(unsent.iter().map(|held| held.server).collect::<Vec<_>>(), vec![b], "older ones expire as new ones arrive");
        for n in 0..MAX_UNSENT + 3 {
            let payload = WirePayload::Event(ServerEvent::PublicSet { server: ServerId::new(), public: n % 2 == 0 });
            hold_unsent(&mut unsent, Unsent { server: a, payload, at: now }, now);
        }
        assert_eq!(unsent.len(), MAX_UNSENT);
        let mut cursors = Cursors { unsent: unsent.clone(), ..Cursors::default() };
        let json = serde_json::to_string(&cursors).unwrap();
        assert_eq!(serde_json::from_str::<Cursors>(&json).unwrap().unsent, unsent);
        cursors.unsent.clear();
        assert_eq!(serde_json::from_str::<Cursors>(r#"{"after":{}}"#).unwrap(), cursors);
    }

    #[test]
    fn a_quiet_channel_wakes_only_when_it_changes() {
        let mut baseline = None;
        assert!(!changed(12, &mut baseline, 12), "nothing new since this device last read it");
        assert!(changed(12, &mut baseline, 15), "new messages wake it");
        assert!(changed(12, &mut baseline, 3), "a stream that started over wakes it");
        assert_eq!(baseline, None);
        assert!(!changed(0, &mut baseline, 40), "a channel this device never read stays asleep at first");
        assert_eq!(baseline, Some(40));
        assert!(!changed(0, &mut baseline, 40));
        assert!(changed(0, &mut baseline, 41), "and wakes on its first new message");
    }

    #[test]
    fn channels_of_servers_off_screen_wait_for_changes_while_the_rest_open() {
        let (events, _received) = mpsc::unbounded_channel();
        let state = commy_sync::StreamSecret::random().keys();
        let channel = commy_sync::StreamSecret::random().keys();
        let (state_id, channel_id) = (state.id, channel.id);
        let group: OpenGroup = vec![(Lane::State, state, 0), (Lane::Channel(commy_core::ChannelId::new()), channel, 7)];
        let (lazy, eager) = tauri::async_runtime::block_on(crate::account_session::spawn(async move {
            let server = ServerId::new();
            let mut lazy = Community::new(UNREACHABLE_SYNC_URL, events.clone(), false);
            lazy.place(server, group.clone());
            let mut owned = Community::new(UNREACHABLE_SYNC_URL, events, true);
            owned.place(server, group);
            let seen = |c: &Community| (c.pool.follows(state_id), c.pool.follows(channel_id), c.pool.watches(channel_id), c.dormant.contains(&channel_id));
            (seen(&lazy), seen(&owned))
        }))
        .unwrap();
        assert_eq!(lazy, (true, false, true, true), "a quiet channel is only watched");
        assert_eq!(eager, (true, true, false, false), "an owner keeps every channel open");
    }

    #[test]
    fn a_checkpoint_moves_a_state_cursor_back_but_never_forward() {
        let stream = commy_sync::StreamSecret::random().keys().id;
        let mut cursors = Cursors::default();
        cursors.start_at(stream, 40);
        assert_eq!(cursors.after(stream), 40, "a new device starts where the checkpoint says");
        cursors.advance(stream, 90);
        cursors.start_at(stream, 60);
        assert_eq!(cursors.after(stream), 60, "entries parked before a restart are read again");
        cursors.start_at(stream, 75);
        assert_eq!(cursors.after(stream), 60);
    }

    #[test]
    fn cursors_only_move_forward_and_forget_streams_we_left() {
        let mut cursors = Cursors::default();
        let a = commy_sync::StreamSecret::random().keys().id;
        let b = commy_sync::StreamSecret::random().keys().id;
        assert!(cursors.advance(a, 5));
        assert!(!cursors.advance(a, 3));
        assert_eq!(cursors.after(a), 5);
        cursors.advance(b, 2);
        cursors.backfilled.insert(b.to_hex());
        cursors.read_back.insert(b.to_hex());
        cursors.older_before.insert(b.to_hex(), 201);
        cursors.older_before.insert(a.to_hex(), 801);
        cursors.keep_only(&HashSet::from([a.to_hex()]));
        assert_eq!(cursors.after(b), 0);
        assert!(cursors.backfilled.is_empty());
        assert!(cursors.read_back.is_empty());
        assert_eq!(cursors.older_before, BTreeMap::from([(a.to_hex(), 801)]), "a read back still to do survives a restart");
        let json = serde_json::to_string(&cursors).unwrap();
        assert_eq!(serde_json::from_str::<Cursors>(&json).unwrap(), cursors);
        assert_eq!(serde_json::from_str::<Cursors>("{}").unwrap(), Cursors::default());
    }

    #[test]
    fn moving_a_community_to_another_host_starts_its_streams_over_there() {
        let account = "wss://commy.chat/sync";
        let away = "wss://sync.example.org/sync";
        let (sid, other) = (ServerId::new(), ServerId::new());
        let a = commy_sync::StreamSecret::random().keys().id;
        let b = commy_sync::StreamSecret::random().keys().id;
        let mut cursors = Cursors::default();
        cursors.advance(a, 40);
        cursors.advance(b, 9);
        cursors.backfilled.insert(a.to_hex());
        cursors.announced.insert(a.to_hex(), "p".into());
        assert!(!cursors.settle_host(sid, account, account, &[a]), "a community never moved stays where it was");
        assert_eq!(cursors.after(a), 40);
        assert!(cursors.settle_host(sid, away, account, &[a]));
        assert_eq!(cursors.after(a), 0, "the new host numbers its entries from the start");
        assert!(!cursors.backfilled.contains(&a.to_hex()), "the owner copies the history over again");
        assert!(cursors.announced.is_empty());
        assert_eq!(cursors.after(b), 9, "other communities keep their place");
        assert!(!cursors.settle_host(sid, away, account, &[a]));
        assert!(!cursors.settle_host(other, away, away, &[b]), "an account on that host saw no move");
        cursors.advance(a, 3);
        let json = serde_json::to_string(&cursors).unwrap();
        let mut back = serde_json::from_str::<Cursors>(&json).unwrap();
        assert!(!back.settle_host(sid, away, account, &[a]), "the host is remembered across restarts");
        assert!(back.settle_host(sid, account, account, &[a]), "moving back home starts over too");
        back.keep_hosts_of(&HashSet::from([other]));
        assert_eq!(back.hosts.keys().cloned().collect::<Vec<_>>(), vec![other.to_string()]);
    }

    #[test]
    fn a_community_follows_its_chosen_host_or_the_account_servers() {
        assert_eq!(community_url(None, "wss://commy.chat/sync").as_deref(), Some("wss://commy.chat/sync"));
        assert_eq!(community_url(Some("https://sync.example.org"), "wss://commy.chat/sync").as_deref(), Some("wss://sync.example.org/sync"));
        assert_eq!(community_url(Some("https://10.0.0.1"), "wss://commy.chat/sync"), None);
    }

    #[test]
    fn an_invite_remembers_where_its_community_syncs() {
        let sid = ServerId::new();
        remember_invite_host(sid, Some("https://sync.example.org".into()));
        assert_eq!(invited_host(sid).as_deref(), Some("https://sync.example.org"));
        remember_invite_host(sid, Some("http://sync.example.org".into()));
        assert_eq!(invited_host(sid), None, "a bad host is dropped, not kept from before");
        remember_invite_host(sid, None);
        assert_eq!(invited_host(sid), None);
    }

    #[test]
    fn a_device_tells_each_stream_where_it_is_once_until_that_changes() {
        let mut cursors = Cursors::default();
        let a = commy_sync::StreamSecret::random().keys().id;
        let b = commy_sync::StreamSecret::random().keys().id;
        let mut me = UserProfile::new("me");
        me.node = Some("ab".repeat(32));
        let here = profile_digest(&me).unwrap();
        me.presence = commy_core::Presence::Idle;
        assert_eq!(profile_digest(&me).unwrap(), here, "presence is not news");
        assert!(cursors.announce(a, &here));
        assert!(!cursors.announce(a, &here), "the same place twice is quiet");
        assert!(cursors.announce(b, &here), "a new key's stream hears it again");
        me.node = Some("cd".repeat(32));
        let moved = profile_digest(&me).unwrap();
        assert_ne!(moved, here, "a reinstalled device has a new node");
        assert!(cursors.announce(a, &moved));
        cursors.unannounce(a, &here);
        assert!(!cursors.announce(a, &moved), "a stale failure keeps the newer note");
        cursors.unannounce(a, &moved);
        assert!(cursors.announce(a, &moved), "a failed send is tried again");
        let json = serde_json::to_string(&cursors).unwrap();
        assert_eq!(serde_json::from_str::<Cursors>(&json).unwrap(), cursors);
        cursors.keep_only(&HashSet::from([b.to_hex()]));
        assert_eq!(cursors.announced.len(), 1);
    }

    #[test]
    fn history_from_the_service_never_rings() {
        let now = Utc::now();
        assert!(rings(true, now - chrono::Duration::seconds(5), now));
        assert!(!rings(false, now - chrono::Duration::seconds(5), now), "catching up stays quiet");
        assert!(!rings(true, now - chrono::Duration::hours(1), now), "a backfilled message stays quiet");
    }

    #[test]
    fn the_owner_backfills_messages_and_lasting_server_changes() {
        let identity = Identity::generate();
        let mut profile = UserProfile::new("owner");
        profile.identity = Some(identity.public());
        let owner = profile.id;
        let mut state = ServerState::new(Server::new("s", owner));
        state.profiles.insert(owner, profile);
        let server = state.server.id;
        let channel = state.server.channels[0].id;
        let mut message = Message::new(channel, owner, "hello");
        message.sign(server, &identity);
        state.apply(ServerEvent::MessageCreate { message: message.clone() }).unwrap();
        let rename = commy_core::action::SignedEvent::sign(server, owner, now_ms(), ServerEvent::ServerRename { server, name: "new".into() }, &identity);
        state.apply_signed(&rename).unwrap();
        let texts = backfill_for(&state, Lane::Channel(channel), &HashSet::new());
        assert_eq!(texts, vec![WirePayload::Event(ServerEvent::MessageCreate { message: message.clone() })]);
        assert!(backfill_for(&state, Lane::Channel(channel), &HashSet::from([message.id])).is_empty());
        let lasting = backfill_for(&state, Lane::State, &HashSet::new());
        assert!(lasting.contains(&WirePayload::Signed(rename)));
    }

    #[test]
    fn the_owner_republishes_key_records_but_not_grants() {
        let identity = Identity::generate();
        let mut profile = UserProfile::new("owner");
        profile.identity = Some(identity.public());
        let owner = profile.id;
        let mut state = ServerState::new(Server::new("s", owner));
        state.profiles.insert(owner, profile);
        let server = state.server.id;
        let mut ring = Keyring::unlock_community(&state, owner, &identity);
        let task = commy_core::channel_key::community_key_task(&state, owner, &ring).unwrap();
        for event in ring.task_events(&state, KeyScope::Community, task, now_ms()) {
            let signed = commy_core::action::SignedEvent::sign(server, owner, now_ms(), event, &identity);
            state.apply_signed(&signed).unwrap();
        }
        let records = key_records(&state);
        assert_eq!(records.len(), 1);
        assert!(matches!(&records[0], WirePayload::Signed(signed) if matches!(signed.event, ServerEvent::CommunityKeyAdd { .. })));
        let (key, secret) = Keyring::unlock_community(&state, owner, &identity).newest_in(&state, KeyScope::Community).map(|(k, s)| (k, s.clone())).unwrap();
        let shared = SharedKey::of(key, &secret);
        let (back, opened) = shared.open().unwrap();
        assert_eq!(back, key);
        assert_eq!(opened.bytes(), secret.bytes());
        assert!(SharedKey { id: "zz".into(), secret: shared.secret.clone() }.open().is_none());
    }

    #[test]
    fn invite_keys_are_kept_per_server_and_bounded() {
        let first = ServerId::new();
        remember_invite_key(first, KeyId::random(), ChannelSecret::random());
        let key = KeyId::random();
        remember_invite_key(first, key, ChannelSecret::random());
        assert_eq!(invited_key(first).map(|(k, _)| k), Some(key));
        for _ in 0..MAX_INVITED {
            remember_invite_key(ServerId::new(), KeyId::random(), ChannelSecret::random());
        }
        assert!(invited_key(first).is_none());
        assert!(INVITED.lock().unwrap().len() <= MAX_INVITED);
    }
}
