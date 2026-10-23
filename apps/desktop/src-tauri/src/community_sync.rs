use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::Utc;
use commy_core::channel_key::{ChannelSecret, KeyId, KeyScope, Keyring};
use commy_core::{MessageId, ServerEvent, ServerId, SignedEvent, UserId, UserProfile};
use commy_net::catchup::CatchupStore;
use commy_net::community_streams::{self as streams, rings, Lane, Want};
pub use commy_net::sync_streams::SharedKey;
use commy_net::{NetEvent, WirePayload};
use commy_sync::{AppendError, StreamId, SyncClient, SyncEvent};
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

    pub fn keep_only(&mut self, streams: &HashSet<String>) {
        self.after.retain(|stream, _| streams.contains(stream));
        self.backfilled.retain(|stream| streams.contains(stream));
        self.read_back.retain(|stream| streams.contains(stream));
        self.older_before.retain(|stream, _| streams.contains(stream));
        self.announced.retain(|stream, _| streams.contains(stream));
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

struct Community {
    pool: Pool,
    wants: HashMap<StreamId, Want>,
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
    seen: HashMap<StreamId, HashSet<MessageId>>,
    backfilling: HashSet<StreamId>,
    snapshots: HashMap<ServerId, WirePayload>,
    parked: HashMap<ServerId, VecDeque<WirePayload>>,
    published: HashMap<ServerId, i64>,
    retried: HashSet<MessageId>,
    echoes: VecDeque<[u8; 32]>,
    fresh: HashSet<StreamId>,
    reading: Option<Reading>,
    greeted: HashMap<ServerId, Instant>,
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
static INVITED: Mutex<Vec<(ServerId, KeyId, ChannelSecret)>> = Mutex::new(Vec::new());
static SAVE_PENDING: AtomicBool = AtomicBool::new(false);
static REFRESH_PENDING: AtomicBool = AtomicBool::new(false);
static UNSENT: Mutex<Vec<Unsent>> = Mutex::new(Vec::new());

fn digest(plain: &[u8]) -> [u8; 32] {
    Sha256::digest(plain).into()
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub fn remember_invite_key(server: ServerId, key: KeyId, secret: ChannelSecret) {
    let mut invited = INVITED.lock().unwrap();
    invited.retain(|(s, _, _)| *s != server);
    invited.push((server, key, secret));
    let excess = invited.len().saturating_sub(MAX_INVITED);
    invited.drain(..excess);
}

fn invited_key(server: ServerId) -> Option<(KeyId, ChannelSecret)> {
    INVITED.lock().unwrap().iter().find(|(s, _, _)| *s == server).map(|(_, key, secret)| (*key, secret.clone()))
}

pub fn invite_key(app: &App, server: ServerId) -> Option<(KeyId, ChannelSecret)> {
    let holder = crate::channel_lock::holder_of(app);
    let servers = app.servers.lock().unwrap();
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
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(SAVE_DELAY).await;
        SAVE_PENDING.store(false, Ordering::SeqCst);
        save();
    });
}

fn save() {
    let snapshot = LIVE.lock().unwrap().as_ref().map(|live| (live.dir.clone(), live.cursors.clone()));
    if let Some((dir, mut cursors)) = snapshot {
        cursors.unsent = UNSENT.lock().unwrap().clone();
        crate::entry_list::write(&dir.join(FILE), &cursors);
    }
}

fn hold(held: Unsent) {
    hold_unsent(&mut UNSENT.lock().unwrap(), held, now_ms());
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
        let guard = LIVE.lock().unwrap();
        let Some(live) = guard.as_ref() else { return };
        let mut unsent = UNSENT.lock().unwrap();
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
    if let Some(old) = LIVE.lock().unwrap().take() {
        old.task.abort();
        if let Some(reading) = old.reading {
            reading.task.abort();
        }
        UNSENT.lock().unwrap().clear();
    }
}

pub fn nudge() {
    if let Some(live) = LIVE.lock().unwrap().as_ref() {
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
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(REFRESH_SOON).await;
        REFRESH_PENDING.store(false, Ordering::SeqCst);
        tend(&handle).await;
    });
}

async fn tend(handle: &AppHandle) {
    let app = handle.state::<App>();
    let Some(url) = sync_link::url() else {
        stop();
        UNSENT.lock().unwrap().retain(|held| now_ms() - held.at < UNSENT_WITHOUT_SERVICE_MS);
        return;
    };
    let (Some(dir), Some(me)) = (app.data_dir().cloned(), app.profile.lock().unwrap().as_ref().map(|p| p.id)) else { return };
    if !app.key_settled() || app.identity().is_none() {
        return;
    }
    let same = LIVE.lock().unwrap().as_ref().is_some_and(|l| l.url == url && l.dir == dir && l.me == me);
    if !same {
        let Some((net, node)) = app.net.lock().await.as_ref().map(|node| (node.local_events(), node.node_id_bytes())) else { return };
        stop();
        let (events, received) = mpsc::unbounded_channel();
        let task = tauri::async_runtime::spawn(listen(handle.clone(), received));
        let mut cursors = load(&dir);
        {
            let mut unsent = UNSENT.lock().unwrap();
            let now = now_ms();
            for held in std::mem::take(&mut cursors.unsent) {
                hold_unsent(&mut unsent, held, now);
            }
        }
        *LIVE.lock().unwrap() = Some(Live {
            handle: handle.clone(),
            url,
            dir,
            me,
            node,
            events,
            net,
            communities: HashMap::new(),
            caught_up: HashSet::new(),
            seen: HashMap::new(),
            backfilling: HashSet::new(),
            snapshots: HashMap::new(),
            parked: HashMap::new(),
            published: HashMap::new(),
            retried: HashSet::new(),
            echoes: VecDeque::new(),
            fresh: HashSet::new(),
            reading: None,
            greeted: HashMap::new(),
            cursors,
            task,
        });
    }
    refresh(handle);
    share_keys(&app);
    release_parked(handle);
    publish_snapshots(handle);
    store_pending(handle);
    send_unsent();
    read_back(handle);
}

fn read_back(handle: &AppHandle) {
    let next = {
        let mut guard = LIVE.lock().unwrap();
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
        pick.map(|pick| (pick, live.url.clone()))
    };
    let Some(((stream, server, want, until), url)) = next else { return };
    let (client, events) = SyncClient::spawn(url);
    client.follow(want.keys.clone(), until.saturating_sub(READ_BACK + 1));
    let task = tauri::async_runtime::spawn(read_until(handle.clone(), server, stream, want.lane, until, events));
    if let Some(live) = LIVE.lock().unwrap().as_mut() {
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
    if let Some(live) = LIVE.lock().unwrap().as_mut() {
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
    let Some(net) = LIVE.lock().unwrap().as_ref().map(|live| live.net.clone()) else { return };
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
        let guard = LIVE.lock().unwrap();
        let Some(live) = guard.as_ref() else { return };
        live.communities.iter().filter(|(_, c)| c.wants.values().any(|w| matches!(w.lane, Lane::Channel(_)) && w.writes)).map(|(sid, _)| *sid).collect()
    };
    for server in ready {
        let pending = app.outbox.lock().unwrap().pending_ids(server);
        let fresh: Vec<MessageId> = {
            let mut guard = LIVE.lock().unwrap();
            let Some(live) = guard.as_mut() else { return };
            pending.into_iter().filter(|id| live.retried.insert(*id)).collect()
        };
        if fresh.is_empty() {
            continue;
        }
        let messages: Vec<commy_core::Message> = {
            let servers = app.servers.lock().unwrap();
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
    let cleared = app.outbox.lock().unwrap().clear_delivered(server, [message]);
    if cleared {
        app.persist_outbox();
        let _ = handle.emit("server-event", server.to_string());
    }
}

fn given_key(app: &App, server: ServerId) -> Option<(KeyId, ChannelSecret)> {
    invited_key(server).or_else(|| crate::account_log::community_key(app, server))
}

fn share_keys(app: &App) {
    let holder = crate::channel_lock::holder_of(app);
    let sids: Vec<ServerId> = app.servers.lock().unwrap().keys().copied().filter(|sid| !app.awaits_snapshot(*sid)).collect();
    let stored: HashMap<ServerId, KeyId> = sids.iter().filter_map(|sid| Some((*sid, crate::account_log::shared_community_key(app, *sid)?))).collect();
    let owners: HashMap<ServerId, (UserId, commy_core::IdentityKey)> = sids.iter().filter_map(|sid| Some((*sid, app.vouched_ticket(*sid)?.owner?))).collect();
    let unvouched: HashSet<ServerId> = owners.keys().copied().filter(|sid| crate::account_log::shared_owner(app, *sid).is_none()).collect();
    let due: Vec<(ServerId, KeyId, ChannelSecret)> = {
        let servers = app.servers.lock().unwrap();
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
    let Some(raw) = app.invites.lock().unwrap().get(&server).cloned() else { return };
    let Ok(mut ticket) = commy_net::ServerTicket::decode(&raw) else { return };
    if ticket.owner.is_some() {
        return;
    }
    ticket.owner = Some(owner);
    app.invites.lock().unwrap().insert(server, ticket.encode());
    app.persist_invites();
}

fn wanted_now(app: &App, me: UserId) -> HashMap<ServerId, Vec<Want>> {
    let Some(identity) = app.identity() else { return HashMap::new() };
    let holder = crate::channel_lock::holder_of(app);
    let sids: Vec<ServerId> = app.servers.lock().unwrap().keys().copied().collect();
    let awaiting: HashSet<ServerId> = sids.iter().copied().filter(|sid| app.awaits_snapshot(*sid)).collect();
    let mut given: HashMap<ServerId, (KeyId, ChannelSecret)> = sids.iter().filter_map(|sid| Some((*sid, given_key(app, *sid)?))).collect();
    let now = now_ms();
    let servers = app.servers.lock().unwrap();
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

fn refresh(handle: &AppHandle) {
    let app = handle.state::<App>();
    let Some(me) = LIVE.lock().unwrap().as_ref().map(|l| l.me) else { return };
    let mut wanted = wanted_now(&app, me);
    let mut fresh_communities = Vec::new();
    let mut promoted = Vec::new();
    let mut guard = LIVE.lock().unwrap();
    let Some(live) = guard.as_mut() else { return };
    let gone: Vec<ServerId> = live.communities.keys().filter(|sid| !wanted.contains_key(sid)).copied().collect();
    for sid in gone {
        if let Some(community) = live.communities.remove(&sid) {
            for stream in community.wants.keys() {
                live.caught_up.remove(stream);
                live.seen.remove(stream);
            }
        }
    }
    let everything: HashSet<String> = wanted.values().flatten().map(|w| w.keys.id.to_hex()).collect();
    live.cursors.keep_only(&everything);
    for (sid, wants) in wanted.drain() {
        let fresh = !live.communities.contains_key(&sid);
        let events = live.events.clone();
        let url = live.url.clone();
        let community = live.communities.entry(sid).or_insert_with(|| Community { pool: Pool::with_events(&url, events), wants: HashMap::new() });
        let keep: HashSet<StreamId> = wants.iter().map(|w| w.keys.id).collect();
        let dropped: Vec<StreamId> = community.wants.keys().filter(|id| !keep.contains(id)).copied().collect();
        for stream in dropped {
            community.wants.remove(&stream);
            community.pool.unfollow(stream);
            live.caught_up.remove(&stream);
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
            follow_group(&mut community.pool, group);
        }
    }
    drop(guard);
    for sid in promoted {
        announce_me(handle, sid);
    }
    for (sid, group) in fresh_communities {
        let spread = Duration::from_millis(std::hash::BuildHasher::hash_one(&std::collections::hash_map::RandomState::new(), sid) % FIRST_OPEN_SPREAD_MS);
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(spread).await;
            if let Some(community) = LIVE.lock().unwrap().as_mut().and_then(|l| l.communities.get_mut(&sid)) {
                let still: Vec<_> = group.into_iter().filter(|(_, keys, _)| community.wants.contains_key(&keys.id)).collect();
                follow_group(&mut community.pool, still);
            }
        });
    }
    crate::push_wake::register(handle);
}

fn follow_group(pool: &mut Pool, group: Vec<(Lane, commy_sync::StreamKeys, u64)>) {
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
        .filter(|(_, want)| if key_event { want.lane == Lane::State } else { want.lane == lane && want.writes })
        .map(|(stream, _)| (*stream, community.pool.client_for(*stream)))
        .collect()
}

pub fn publish(server: ServerId, payload: &WirePayload) {
    if streams::lane_of_payload(payload).is_none() {
        signal(server, payload);
        return;
    }
    publish_held(Unsent { server, payload: payload.clone(), at: now_ms() });
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
        let mut guard = LIVE.lock().unwrap();
        guard.as_mut().map(|live| {
            let sends = targets(live, server, lane, key_event);
            if !sends.is_empty() {
                live.note_echo(digest(&plain));
            }
            (sends, live.handle.clone())
        })
    };
    let Some((sends, handle)) = found.filter(|(sends, _)| !sends.is_empty()) else {
        hold(held);
        return;
    };
    tauri::async_runtime::spawn(async move {
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
    let guard = LIVE.lock().unwrap();
    let Some(live) = guard.as_ref() else { return };
    let Some(plain) = streams::encode_signal(live.node, payload) else { return };
    for (stream, client) in targets(live, server, lane, false) {
        if let Some(client) = client {
            client.signal(stream, plain.clone());
        }
    }
}

fn say_here(handle: &AppHandle, server: ServerId) {
    if let Some(live) = LIVE.lock().unwrap().as_mut() {
        live.greeted.insert(server, Instant::now());
    }
    for payload in crate::liveness::here(&handle.state::<App>(), server) {
        signal(server, &payload);
    }
}

fn greet(handle: &AppHandle, server: ServerId) {
    {
        let mut guard = LIVE.lock().unwrap();
        let Some(live) = guard.as_mut() else { return };
        let now = Instant::now();
        if live.greeted.get(&server).is_some_and(|at| now.duration_since(*at) < GREET_GAP) {
            return;
        }
        live.greeted.insert(server, now);
    }
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(rand::random::<u64>() % GREET_SPREAD_MS)).await;
        say_here(&handle, server);
    });
}

pub async fn publish_now(server: ServerId, payload: &WirePayload) -> bool {
    let Some(lane) = streams::lane_of_payload(payload) else { return false };
    let key_event = streams::event_of(payload).is_none_or(|(event, _)| streams::is_key_event(event));
    let Some(plain) = streams::encode_entry(payload) else { return false };
    let sends = {
        let mut guard = LIVE.lock().unwrap();
        let Some(live) = guard.as_mut() else { return false };
        let sends = targets(live, server, lane, key_event);
        if !sends.is_empty() {
            live.note_echo(digest(&plain));
        }
        sends
    };
    let mut stored = !sends.is_empty();
    for (stream, client) in sends {
        let sent = tokio::time::timeout(PUBLISH_WAIT, sync_link::append(client, stream, plain.clone())).await;
        stored &= matches!(sent, Ok(Ok(_)));
    }
    stored
}

fn wake_point(want: &Want) -> Option<Option<commy_core::ChannelId>> {
    if !want.writes {
        return None;
    }
    match (want.scope, want.lane) {
        (KeyScope::Community, Lane::State) => Some(None),
        (KeyScope::Channel(channel), _) => Some(Some(channel)),
        _ => None,
    }
}

pub fn wake_keys(server: ServerId, private: Option<commy_core::ChannelId>) -> Option<commy_sync::StreamKeys> {
    let guard = LIVE.lock().unwrap();
    let community = guard.as_ref()?.communities.get(&server)?;
    community.wants.values().find(|want| wake_point(want) == Some(private)).map(|want| want.keys.clone())
}

pub fn wake_points() -> Vec<(ServerId, Option<commy_core::ChannelId>, commy_sync::StreamKeys)> {
    let guard = LIVE.lock().unwrap();
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
    let Some(profile) = profile_digest(profile) else { return };
    let Some(plain) = streams::encode_entry(payload) else { return };
    let sends = {
        let mut guard = LIVE.lock().unwrap();
        let Some(live) = guard.as_mut() else { return };
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
        return;
    }
    save_soon();
    for (stream, client) in sends {
        let (plain, profile) = (plain.clone(), profile.clone());
        tauri::async_runtime::spawn(async move {
            if let Err(e) = sync_link::append(client, stream, plain).await {
                tracing::debug!("could not tell the sync service where this device is: {e}");
                if let Some(live) = LIVE.lock().unwrap().as_mut() {
                    live.cursors.unannounce(stream, &profile);
                }
                save_soon();
            }
        });
    }
}

fn announce_me(handle: &AppHandle, server: ServerId) {
    let app = handle.state::<App>();
    let Some(me) = app.profile.lock().unwrap().clone() else { return };
    let mut profile = app.announced(me);
    {
        let servers = app.servers.lock().unwrap();
        let Some(state) = servers.get(&server).filter(|s| s.server.is_current_member(profile.id) && !s.is_deleted()) else { return };
        crate::devices::point_at_voice(state, &mut profile);
    }
    let payload = app.wire_for(server, ServerEvent::MemberJoin { server, profile: profile.clone() });
    announce_profile(server, &profile, &payload);
}

async fn listen(handle: AppHandle, mut events: mpsc::UnboundedReceiver<SyncEvent>) {
    while let Some(event) = events.recv().await {
        on_event(&handle, event);
    }
}

fn on_event(handle: &AppHandle, event: SyncEvent) {
    match event {
        SyncEvent::Entry { stream, seq, plain, .. } => on_entry(handle, stream, seq, &plain),
        SyncEvent::Unreadable { stream, seq } => {
            if let Some(live) = LIVE.lock().unwrap().as_mut() {
                live.cursors.advance(stream, seq);
            }
            save_soon();
        }
        SyncEvent::CaughtUp { stream, seq } => on_caught_up(handle, stream, seq),
        SyncEvent::Signal { stream, plain } => on_signal(handle, stream, &plain),
        SyncEvent::Refused { stream, code } => tracing::debug!("the sync service refused a server stream {stream}: {code:?}"),
        SyncEvent::Connected | SyncEvent::Disconnected => {}
    }
}

fn on_entry(handle: &AppHandle, stream: StreamId, seq: u64, plain: &[u8]) {
    let found = {
        let mut guard = LIVE.lock().unwrap();
        let Some(live) = guard.as_mut() else { return };
        let Some((server, want)) = live.route(stream) else { return };
        live.cursors.advance(stream, seq);
        if live.fresh.contains(&stream) && !live.cursors.read_back.contains(&stream.to_hex()) {
            live.cursors.older_before.entry(stream.to_hex()).or_insert(seq);
        }
        let echo = live.take_echo(&digest(plain));
        let live_now = live.caught_up.contains(&stream);
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
    let key_event = streams::is_key_event(&event);
    match event {
        ServerEvent::MessageCreate { message } if !rings(live_now, message.created_at, Utc::now()) => {
            crate::catchup::AppStore(handle.clone()).apply(server, vec![message], Vec::new());
        }
        event => {
            let _ = net.send(NetEvent::Event { server, sender: streams::SERVICE_SENDER, event, action });
        }
    }
    if key_event {
        refresh_soon(handle);
    }
}

fn on_signal(handle: &AppHandle, stream: StreamId, plain: &[u8]) {
    let found = {
        let guard = LIVE.lock().unwrap();
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
        let servers = app.servers.lock().unwrap();
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
    let mut guard = LIVE.lock().unwrap();
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
    let Some(snapshot) = LIVE.lock().unwrap().as_mut().and_then(|live| live.snapshots.remove(&server)) else { return };
    if crate::catchup::apply_service_snapshot(handle, server, snapshot) {
        tracing::info!("took the owner's copy of {server} from the sync service");
        release_parked(handle);
        refresh_soon(handle);
    }
}

fn release_parked(handle: &AppHandle) {
    let app = handle.state::<App>();
    let (released, net) = {
        let mut guard = LIVE.lock().unwrap();
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
        let guard = LIVE.lock().unwrap();
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
        let keys = app.servers.lock().unwrap().get(&server).filter(|state| state.server.owner == me).map(key_records);
        let snapshot = keys.is_some().then(|| crate::catchup::service_snapshot_for(&app, server)).flatten();
        if keys.is_some() && snapshot.is_none() {
            continue;
        }
        if let Some(live) = LIVE.lock().unwrap().as_mut() {
            live.published.insert(server, now);
        }
        for payload in snapshot.into_iter().chain(keys.unwrap_or_default()) {
            publish(server, &payload);
        }
    }
}

fn on_caught_up(handle: &AppHandle, stream: StreamId, seq: u64) {
    let found = {
        let mut guard = LIVE.lock().unwrap();
        let Some(live) = guard.as_mut() else { return };
        let Some((server, want)) = live.route(stream) else { return };
        live.cursors.advance(stream, seq);
        let first = live.caught_up.insert(stream);
        let held = live.snapshots.contains_key(&server);
        let due = want.writes && !live.cursors.backfilled.contains(&stream.to_hex()) && !live.backfilling.contains(&stream);
        (server, want, due, live.me, first, held)
    };
    save_soon();
    let (server, want, due, me, first, held) = found;
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
            say_here(handle, server);
        }
        publish_snapshots(handle);
    }
    if !due {
        return;
    }
    let seen = LIVE.lock().unwrap().as_mut().and_then(|live| live.seen.remove(&stream)).unwrap_or_default();
    let payloads = {
        let servers = app.servers.lock().unwrap();
        servers.get(&server).filter(|s| s.server.owner == me).map(|state| backfill_for(state, want.lane, &seen))
    };
    let Some(payloads) = payloads else { return };
    if !LIVE.lock().unwrap().as_mut().is_some_and(|live| live.backfilling.insert(stream)) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let done = fill(stream, payloads).await;
        if let Some(live) = LIVE.lock().unwrap().as_mut() {
            live.backfilling.remove(&stream);
            if done {
                live.cursors.backfilled.insert(stream.to_hex());
            }
        }
        save_soon();
    });
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
            let mut guard = LIVE.lock().unwrap();
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

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Identity, Message, Server, ServerState};

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
