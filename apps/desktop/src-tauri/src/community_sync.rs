use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use chrono::Utc;
use commy_core::channel_key::{ChannelSecret, KeyId, Keyring};
use commy_core::{MessageId, ServerEvent, ServerId, UserId, UserProfile};
use commy_net::catchup::CatchupStore;
use commy_net::community_streams::{self as streams, Lane, Want};
use commy_net::{NetEvent, WirePayload};
use commy_sync::{AppendError, StreamId, SyncClient, SyncEvent};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};
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
const RINGS_FOR_SECS: i64 = 10 * 60;
const MAX_BACKFILL_PER_STREAM: usize = 1_000;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Cursors {
    #[serde(default)]
    pub after: BTreeMap<String, u64>,
    #[serde(default)]
    pub backfilled: BTreeSet<String>,
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
    }
}

struct Community {
    pool: Pool,
    wants: HashMap<StreamId, Want>,
}

struct Live {
    url: String,
    dir: PathBuf,
    me: UserId,
    events: mpsc::UnboundedSender<SyncEvent>,
    net: mpsc::UnboundedSender<NetEvent>,
    communities: HashMap<ServerId, Community>,
    caught_up: HashSet<StreamId>,
    seen: HashMap<StreamId, HashSet<MessageId>>,
    echoes: VecDeque<[u8; 32]>,
    announced: HashMap<ServerId, [u8; 32]>,
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
    if let Some((dir, cursors)) = snapshot {
        crate::entry_list::write(&dir.join(FILE), &cursors);
    }
}

pub fn stop() {
    save();
    if let Some(old) = LIVE.lock().unwrap().take() {
        old.task.abort();
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
        return;
    };
    let (Some(dir), Some(me)) = (app.data_dir().cloned(), app.profile.lock().unwrap().as_ref().map(|p| p.id)) else { return };
    if !app.key_settled() || app.identity().is_none() {
        return;
    }
    let same = LIVE.lock().unwrap().as_ref().is_some_and(|l| l.url == url && l.dir == dir && l.me == me);
    if !same {
        let Some(net) = app.net.lock().await.as_ref().map(|node| node.local_events()) else { return };
        stop();
        let (events, received) = mpsc::unbounded_channel();
        let task = tauri::async_runtime::spawn(listen(handle.clone(), received));
        let cursors = load(&dir);
        *LIVE.lock().unwrap() = Some(Live {
            url,
            dir,
            me,
            events,
            net,
            communities: HashMap::new(),
            caught_up: HashSet::new(),
            seen: HashMap::new(),
            echoes: VecDeque::new(),
            announced: HashMap::new(),
            cursors,
            task,
        });
    }
    refresh(handle);
}

fn wanted_now(app: &App, me: UserId) -> HashMap<ServerId, Vec<Want>> {
    let Some(identity) = app.identity() else { return HashMap::new() };
    let holder = crate::channel_lock::holder_of(app);
    let sids: Vec<ServerId> = app.servers.lock().unwrap().keys().copied().collect();
    let settled: Vec<ServerId> = sids.into_iter().filter(|sid| !app.awaits_snapshot(*sid)).collect();
    let now = now_ms();
    let servers = app.servers.lock().unwrap();
    settled
        .into_iter()
        .filter_map(|sid| {
            let state = servers.get(&sid)?;
            let community = holder.community_ring(state, invited_key(sid))?;
            let locked = |channel| Keyring::unlock(state, channel, me, &identity);
            let wants = streams::wanted(state, me, &community, locked, now);
            (!wants.is_empty()).then_some((sid, wants))
        })
        .collect()
}

fn refresh(handle: &AppHandle) {
    let app = handle.state::<App>();
    let Some(me) = LIVE.lock().unwrap().as_ref().map(|l| l.me) else { return };
    let mut wanted = wanted_now(&app, me);
    let mut fresh_communities = Vec::new();
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
                held.writes = want.writes;
                continue;
            }
            group.push((want.keys.clone(), live.cursors.after(stream)));
            community.wants.insert(stream, want);
        }
        if group.is_empty() {
            continue;
        }
        if fresh {
            fresh_communities.push((sid, group));
        } else {
            community.pool.follow_together_tail(group, Some(streams::FRESH_TAIL));
        }
    }
    drop(guard);
    for (sid, group) in fresh_communities {
        let spread = Duration::from_millis(std::hash::BuildHasher::hash_one(&std::collections::hash_map::RandomState::new(), sid) % FIRST_OPEN_SPREAD_MS);
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(spread).await;
            if let Some(community) = LIVE.lock().unwrap().as_mut().and_then(|l| l.communities.get_mut(&sid)) {
                let still: Vec<_> = group.into_iter().filter(|(keys, _)| community.wants.contains_key(&keys.id)).collect();
                community.pool.follow_together_tail(still, Some(streams::FRESH_TAIL));
            }
        });
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
    let Some((event, _)) = streams::event_of(payload) else { return };
    let Some(lane) = streams::lane_of(event) else { return };
    let key_event = streams::is_key_event(event);
    let Some(plain) = streams::encode_entry(payload) else { return };
    let sends = {
        let mut guard = LIVE.lock().unwrap();
        let Some(live) = guard.as_mut() else { return };
        let sends = targets(live, server, lane, key_event);
        if !sends.is_empty() {
            live.note_echo(digest(&plain));
        }
        sends
    };
    for (stream, client) in sends {
        let plain = plain.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = sync_link::append(client, stream, plain).await {
                tracing::debug!("could not add a server change to the sync service: {e}");
            }
        });
    }
}

pub fn announce_profile(server: ServerId, profile: &UserProfile, payload: &WirePayload) {
    let mut lasting = profile.clone();
    lasting.presence = Default::default();
    let Ok(bytes) = serde_json::to_vec(&lasting) else { return };
    let digest = digest(&bytes);
    let changed = LIVE.lock().unwrap().as_mut().is_some_and(|live| {
        let known = live.communities.get(&server).is_some_and(|c| c.wants.values().any(|w| w.lane == Lane::State));
        known && live.announced.insert(server, digest) != Some(digest)
    });
    if changed {
        publish(server, payload);
    }
}

async fn listen(handle: AppHandle, mut events: mpsc::UnboundedReceiver<SyncEvent>) {
    while let Some(event) = events.recv().await {
        on_event(&handle, event);
    }
}

pub fn rings(live_now: bool, created_at: chrono::DateTime<Utc>, now: chrono::DateTime<Utc>) -> bool {
    live_now && (now - created_at).num_seconds() < RINGS_FOR_SECS
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
        let echo = live.take_echo(&digest(plain));
        let live_now = live.caught_up.contains(&stream);
        let decoded = (!echo).then(|| streams::decode_entry(plain, want.lane)).flatten();
        if let Some((ServerEvent::MessageCreate { message }, _)) = &decoded {
            if !live_now {
                live.seen.entry(stream).or_default().insert(message.id);
            }
        }
        decoded.map(|(event, action)| (server, event, action, live_now, live.net.clone()))
    };
    save_soon();
    let Some((server, event, action, live_now, net)) = found else { return };
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

fn on_caught_up(handle: &AppHandle, stream: StreamId, seq: u64) {
    let backfill = {
        let mut guard = LIVE.lock().unwrap();
        let Some(live) = guard.as_mut() else { return };
        let Some((server, want)) = live.route(stream) else { return };
        live.cursors.advance(stream, seq);
        live.caught_up.insert(stream);
        let seen = live.seen.remove(&stream).unwrap_or_default();
        let due = want.writes && !live.cursors.backfilled.contains(&stream.to_hex());
        due.then(|| (server, want.lane, seen, live.me))
    };
    save_soon();
    let Some((server, lane, seen, me)) = backfill else { return };
    let app = handle.state::<App>();
    let payloads = {
        let servers = app.servers.lock().unwrap();
        let Some(state) = servers.get(&server).filter(|s| s.server.owner == me) else { return };
        backfill_for(state, lane, &seen)
    };
    tauri::async_runtime::spawn(async move {
        if fill(stream, payloads).await {
            if let Some(live) = LIVE.lock().unwrap().as_mut() {
                live.cursors.backfilled.insert(stream.to_hex());
            }
            save_soon();
        }
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
    fn cursors_only_move_forward_and_forget_streams_we_left() {
        let mut cursors = Cursors::default();
        let a = commy_sync::StreamSecret::random().keys().id;
        let b = commy_sync::StreamSecret::random().keys().id;
        assert!(cursors.advance(a, 5));
        assert!(!cursors.advance(a, 3));
        assert_eq!(cursors.after(a), 5);
        cursors.advance(b, 2);
        cursors.backfilled.insert(b.to_hex());
        cursors.keep_only(&HashSet::from([a.to_hex()]));
        assert_eq!(cursors.after(b), 0);
        assert!(cursors.backfilled.is_empty());
        let json = serde_json::to_string(&cursors).unwrap();
        assert_eq!(serde_json::from_str::<Cursors>(&json).unwrap(), cursors);
        assert_eq!(serde_json::from_str::<Cursors>("{}").unwrap(), Cursors::default());
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
