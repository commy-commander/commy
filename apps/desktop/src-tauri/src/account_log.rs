use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use commy_core::channel_key::{ChannelSecret, KeyId};
use commy_core::{ServerId, UserId, UserProfile};
use commy_net::entry_list::EntryList;
use commy_net::sync_streams;
use commy_sync::{AppendError, StreamId, SyncEvent};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::community_sync::SharedKey;
use crate::entry_list;
use crate::read_state::ReadMark;
use crate::saved::SavedMessage;
use crate::state::App;
use crate::sync_link::{self, Pool};

pub const FILE: &str = "sync-state.json";
const MAX_RECORD_BYTES: usize = 48 * 1024;
const MAX_MARKS_PER_RECORD: usize = 300;
const MAX_OUTBOX: usize = 400;
const MAX_PREF_KEY: usize = 200;
const MAX_PREFS_PER_CALL: usize = 500;
const FLUSH_DELAY: Duration = Duration::from_secs(2);
const SERVERS_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrefPiece {
    #[serde(default)]
    pub value: serde_json::Value,
    pub at: i64,
}

impl PrefPiece {
    fn beats(&self, other: Option<&PrefPiece>) -> bool {
        other.is_none_or(|old| (self.at, self.value.to_string()) > (old.at, old.value.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedOp {
    pub message: String,
    pub at: i64,
    #[serde(default)]
    pub saved: Option<SavedMessage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "snake_case")]
pub enum Record {
    Servers { list: EntryList },
    Read { marks: BTreeMap<String, ReadMark> },
    Saved { ops: Vec<SavedOp> },
    Prefs { set: BTreeMap<String, PrefPiece> },
    DmPartner { profile: UserProfile, at: i64 },
    CommunityKey { server: ServerId, key: SharedKey, at: i64 },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Queued {
    pub id: u64,
    pub record: Record,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SyncState {
    #[serde(default)]
    pub stream: String,
    #[serde(default)]
    pub after: u64,
    #[serde(default)]
    pub seeded: bool,
    #[serde(default)]
    pub servers: EntryList,
    #[serde(default)]
    pub prefs: BTreeMap<String, PrefPiece>,
    #[serde(default)]
    pub saved: BTreeMap<String, i64>,
    #[serde(default)]
    pub partners: BTreeMap<UserId, i64>,
    #[serde(default)]
    pub dm_cursors: BTreeMap<String, u64>,
    #[serde(default)]
    pub community: BTreeMap<ServerId, (SharedKey, i64)>,
    #[serde(default)]
    pub outbox: Vec<Queued>,
    #[serde(default)]
    pub next_id: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Nothing,
    Servers,
    Read(BTreeMap<String, ReadMark>),
    Saved(Vec<SavedOp>),
    Prefs(BTreeMap<String, PrefPiece>),
    Partner(Box<UserProfile>),
    CommunityKey,
}

impl SyncState {
    pub fn start_stream(&mut self, stream: &str) {
        if self.stream != stream {
            *self = SyncState { stream: stream.to_string(), prefs: std::mem::take(&mut self.prefs), dm_cursors: std::mem::take(&mut self.dm_cursors), ..Default::default() };
        }
    }

    pub fn fold(&mut self, record: Record) -> Change {
        match record {
            Record::Servers { mut list } => {
                list.identity = None;
                let next = EntryList::merge(&self.servers, &list);
                if next == self.servers {
                    return Change::Nothing;
                }
                self.servers = next;
                Change::Servers
            }
            Record::Read { marks } if !marks.is_empty() => Change::Read(marks),
            Record::Saved { ops } => {
                let taken: Vec<SavedOp> = ops
                    .into_iter()
                    .filter(|op| op.saved.as_ref().is_none_or(|s| s.entry.message == op.message))
                    .filter(|op| {
                        let newer = self.saved.get(&op.message).is_none_or(|at| op.at > *at);
                        if newer {
                            self.saved.insert(op.message.clone(), op.at);
                        }
                        newer
                    })
                    .collect();
                if taken.is_empty() { Change::Nothing } else { Change::Saved(taken) }
            }
            Record::Prefs { set } => {
                let taken: BTreeMap<String, PrefPiece> = set.into_iter().filter(|(key, piece)| piece.beats(self.prefs.get(key))).collect();
                for (key, piece) in &taken {
                    self.prefs.insert(key.clone(), piece.clone());
                }
                if taken.is_empty() { Change::Nothing } else { Change::Prefs(taken) }
            }
            Record::DmPartner { profile, at } if profile.identity.is_some() => {
                let newer = self.partners.get(&profile.id).is_none_or(|seen| at > *seen);
                if !newer {
                    return Change::Nothing;
                }
                self.partners.insert(profile.id, at);
                Change::Partner(Box::new(profile))
            }
            Record::CommunityKey { server, key, at } => {
                let known = self.community.get(&server);
                if known.is_some_and(|(_, seen)| at <= *seen) {
                    return Change::Nothing;
                }
                let same = known.is_some_and(|(held, _)| *held == key);
                self.community.insert(server, (key, at));
                if same { Change::Nothing } else { Change::CommunityKey }
            }
            _ => Change::Nothing,
        }
    }

    pub fn enqueue(&mut self, record: Record, busy: Option<u64>) {
        if let (Record::Read { marks }, Some(last)) = (&record, self.outbox.last_mut()) {
            if let Record::Read { marks: queued } = &mut last.record {
                if Some(last.id) != busy && queued.len() + marks.len() <= MAX_MARKS_PER_RECORD {
                    queued.extend(marks.clone());
                    return;
                }
            }
        }
        self.next_id += 1;
        self.outbox.push(Queued { id: self.next_id, record });
        let excess = self.outbox.len().saturating_sub(MAX_OUTBOX);
        self.outbox.drain(..excess);
    }

    pub fn forget_queued(&mut self, id: u64) {
        self.outbox.retain(|q| q.id != id);
    }
}

fn record_bytes(record: &Record) -> usize {
    serde_json::to_vec(record).map_or(usize::MAX, |bytes| bytes.len())
}

fn is_empty(list: &EntryList) -> bool {
    list.profile.is_none() && list.servers.is_empty() && list.left.is_empty()
}

pub fn servers_delta(log: &EntryList, merged: &EntryList) -> Vec<Record> {
    let mut delta = EntryList::default();
    if merged.profile.is_some() && (log.profile.is_none() || merged.profile_at > log.profile_at) {
        delta.profile = merged.profile.clone();
        delta.profile_at = merged.profile_at;
    }
    for (id, entry) in &merged.servers {
        if log.servers.get(id) != Some(entry) {
            delta.servers.insert(*id, entry.clone());
        }
    }
    for (id, at) in &merged.left {
        if log.left.get(id).is_none_or(|known| at > known) {
            delta.left.insert(*id, *at);
        }
    }
    if is_empty(&delta) {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut current = EntryList { profile: delta.profile.take(), profile_at: delta.profile_at, ..Default::default() };
    let over = |list: &EntryList| record_bytes(&Record::Servers { list: list.clone() }) > MAX_RECORD_BYTES;
    for (id, entry) in delta.servers {
        current.servers.insert(id, entry.clone());
        if over(&current) && current.servers.len() + current.left.len() > 1 {
            current.servers.remove(&id);
            out.push(std::mem::take(&mut current));
            current.servers.insert(id, entry);
        }
    }
    for (id, at) in delta.left {
        current.left.insert(id, at);
        if over(&current) && current.servers.len() + current.left.len() > 1 {
            current.left.remove(&id);
            out.push(std::mem::take(&mut current));
            current.left.insert(id, at);
        }
    }
    if !is_empty(&current) {
        out.push(current);
    }
    out.into_iter().map(|list| Record::Servers { list }).collect()
}

pub fn read_records(marks: BTreeMap<String, ReadMark>) -> Vec<Record> {
    let mut out = Vec::new();
    let mut current = BTreeMap::new();
    for (channel, mark) in marks {
        current.insert(channel, mark);
        if current.len() >= MAX_MARKS_PER_RECORD {
            out.push(Record::Read { marks: std::mem::take(&mut current) });
        }
    }
    if !current.is_empty() {
        out.push(Record::Read { marks: current });
    }
    out
}

pub fn saved_records(ops: Vec<SavedOp>) -> Vec<Record> {
    let mut out = Vec::new();
    let mut current: Vec<SavedOp> = Vec::new();
    for op in ops {
        current.push(op);
        if current.len() > 1 && record_bytes(&Record::Saved { ops: current.clone() }) > MAX_RECORD_BYTES {
            let last = current.pop().expect("just pushed");
            out.push(Record::Saved { ops: std::mem::take(&mut current) });
            current.push(last);
        }
    }
    if !current.is_empty() {
        out.push(Record::Saved { ops: current });
    }
    out
}

struct Live {
    url: String,
    dir: PathBuf,
    stream: StreamId,
    pool: Pool,
    caught_up: bool,
    task: tauri::async_runtime::JoinHandle<()>,
}

static HANDLE: OnceLock<AppHandle> = OnceLock::new();
static LIVE: Mutex<Option<Live>> = Mutex::new(None);
static STATE: Mutex<Option<(PathBuf, SyncState)>> = Mutex::new(None);
static IN_FLIGHT: Mutex<Option<u64>> = Mutex::new(None);
static FLUSH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static FLUSH_PENDING: AtomicBool = AtomicBool::new(false);
static SERVERS_PENDING: AtomicBool = AtomicBool::new(false);

fn load(dir: &Path) -> SyncState {
    entry_list::read(&dir.join(FILE)).unwrap_or_default()
}

pub fn with_state<R>(app: &App, change: impl FnOnce(&mut SyncState) -> R) -> Option<R> {
    let dir = app.data_dir()?.clone();
    let mut guard = STATE.lock().unwrap();
    if guard.as_ref().is_none_or(|(loaded, _)| *loaded != dir) {
        *guard = Some((dir.clone(), load(&dir)));
    }
    guard.as_mut().map(|(_, state)| change(state))
}

fn persist(app: &App) {
    let guard = STATE.lock().unwrap();
    if let Some((dir, state)) = guard.as_ref() {
        if app.data_dir() == Some(dir) {
            entry_list::write(&dir.join(FILE), state);
        }
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn ensure(handle: &AppHandle) {
    let _ = HANDLE.set(handle.clone());
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move { start(&handle) });
}

pub fn stop() {
    if let Some(old) = LIVE.lock().unwrap().take() {
        old.task.abort();
    }
    crate::dm_sync::stop();
}

pub fn nudge() {
    if let Some(live) = LIVE.lock().unwrap().as_ref() {
        live.pool.nudge();
    }
}

fn start(handle: &AppHandle) {
    let app = handle.state::<App>();
    let (Some(url), Some(s)) = (sync_link::url(), entry_list::session(&app)) else {
        stop();
        return;
    };
    let Some(dir) = app.data_dir().cloned() else { return };
    let Some(key) = entry_list::load_key(&dir, &s.server, &s.account) else { return };
    let keys = sync_streams::account_stream(&key);
    let mut live = LIVE.lock().unwrap();
    if live.as_ref().is_some_and(|l| l.url == url && l.dir == dir && l.stream == keys.id) {
        return;
    }
    if let Some(old) = live.take() {
        old.task.abort();
    }
    let stream = keys.id;
    let Some(after) = with_state(&app, |state| {
        state.start_stream(&stream.to_hex());
        state.after
    }) else {
        return;
    };
    persist(&app);
    let (mut pool, events) = Pool::new(&url);
    pool.follow(keys, after);
    let task = tauri::async_runtime::spawn(run(handle.clone(), events, stream));
    *live = Some(Live { url, dir, stream, pool, caught_up: false, task });
    drop(live);
    crate::dm_sync::ensure(handle);
}

async fn run(handle: AppHandle, mut events: tokio::sync::mpsc::UnboundedReceiver<SyncEvent>, stream: StreamId) {
    while let Some(event) = events.recv().await {
        let app = handle.state::<App>();
        match event {
            SyncEvent::Entry { stream: from, seq, plain, .. } if from == stream => {
                let record = serde_json::from_slice::<Record>(&plain).unwrap_or(Record::Unknown);
                let change = with_state(&app, |state| {
                    state.after = seq;
                    state.fold(record)
                });
                if let Some(change) = change {
                    apply(&handle, change);
                }
                if caught_up() {
                    persist(&app);
                }
            }
            SyncEvent::Unreadable { stream: from, seq } if from == stream => {
                with_state(&app, |state| state.after = seq);
            }
            SyncEvent::CaughtUp { stream: from, seq } if from == stream => {
                let first = with_state(&app, |state| {
                    state.after = seq;
                    !std::mem::replace(&mut state.seeded, true)
                })
                .unwrap_or(false);
                persist(&app);
                let was = LIVE.lock().unwrap().as_mut().map(|l| std::mem::replace(&mut l.caught_up, true));
                if was == Some(false) {
                    if first {
                        seed(&handle);
                    }
                    schedule_servers(&handle);
                    flush_soon();
                }
            }
            SyncEvent::Connected => flush_soon(),
            SyncEvent::Refused { stream: from, code } if from == stream => {
                tracing::warn!("the sync service refused the account log: {code:?}");
            }
            _ => {}
        }
    }
}

pub fn is_live(app: &App) -> bool {
    let dir = app.data_dir();
    LIVE.lock().unwrap().as_ref().is_some_and(|l| Some(&l.dir) == dir)
}

fn caught_up() -> bool {
    LIVE.lock().unwrap().as_ref().is_some_and(|l| l.caught_up)
}

fn apply(handle: &AppHandle, change: Change) {
    match change {
        Change::Nothing => {}
        Change::Servers => schedule_servers(handle),
        Change::Read(marks) => crate::read_state::absorb_remote(handle, marks),
        Change::Saved(ops) => crate::saved::apply_remote(handle, ops),
        Change::Prefs(set) => {
            let _ = handle.emit("sync-prefs-changed", set);
        }
        Change::Partner(profile) => crate::dm_sync::adopt_partner(handle, *profile),
        Change::CommunityKey => crate::community_sync::refresh_soon(handle),
    }
}

fn seed(handle: &AppHandle) {
    let app = handle.state::<App>();
    let marks = crate::read_state::all_marks(&app);
    let saved = app.data_dir().map(|dir| crate::saved::SavedStore::load(dir).saved).unwrap_or_default();
    with_state(&app, |state| {
        for record in read_records(marks) {
            state.enqueue(record, None);
        }
        let ops: Vec<SavedOp> = saved
            .into_iter()
            .filter(|s| state.saved.get(&s.entry.message).is_none_or(|at| s.saved_at > *at))
            .map(|s| SavedOp { message: s.entry.message.clone(), at: s.saved_at, saved: Some(s) })
            .collect();
        for record in saved_records(ops) {
            state.fold(record.clone());
            state.enqueue(record, None);
        }
        if !state.prefs.is_empty() {
            state.enqueue(Record::Prefs { set: state.prefs.clone() }, None);
        }
    });
    persist(&app);
    crate::dm_sync::seed(handle);
}

pub fn servers(app: &App) -> Option<EntryList> {
    let dir = app.data_dir()?;
    let live = LIVE.lock().unwrap();
    let ready = live.as_ref().is_some_and(|l| l.caught_up && &l.dir == dir);
    drop(live);
    if !ready {
        return None;
    }
    with_state(app, |state| state.servers.clone())
}

pub fn publish_servers(app: &App, log: &EntryList, merged: &EntryList) {
    let records = servers_delta(log, merged);
    if records.is_empty() {
        return;
    }
    for record in records {
        queue(app, record);
    }
}

pub fn queue(app: &App, record: Record) {
    let offered = sync_link::url().is_some();
    let keeps_settings = matches!(record, Record::Prefs { .. });
    let busy = *IN_FLIGHT.lock().unwrap();
    let Some(enabled) = with_state(app, |state| {
        let enabled = offered || !state.stream.is_empty();
        if enabled || keeps_settings {
            state.fold(record.clone());
        }
        if enabled {
            state.enqueue(record, busy);
        }
        enabled
    }) else {
        return;
    };
    if enabled || keeps_settings {
        persist(app);
    }
    if enabled {
        flush_soon();
    }
}

pub fn note_partner(app: &App, mut profile: UserProfile) {
    if profile.identity.is_none() {
        return;
    }
    let now = now_ms();
    let fresh = with_state(app, |state| state.partners.get(&profile.id).is_none_or(|at| now - at > PARTNER_RENOTE_MS)).unwrap_or(false);
    if !fresh {
        return;
    }
    profile.activity = None;
    profile.activity_cover = None;
    profile.activity_since = None;
    queue(app, Record::DmPartner { profile, at: now });
}

pub fn community_key(app: &App, server: ServerId) -> Option<(KeyId, ChannelSecret)> {
    with_state(app, |state| state.community.get(&server).and_then(|(key, _)| key.open())).flatten()
}

pub fn shared_community_key(app: &App, server: ServerId) -> Option<KeyId> {
    with_state(app, |state| state.community.get(&server).and_then(|(key, _)| key.open()).map(|(id, _)| id)).flatten()
}

pub fn share_community_key(app: &App, server: ServerId, key: KeyId, secret: &ChannelSecret) {
    queue(app, Record::CommunityKey { server, key: SharedKey::of(key, secret), at: now_ms() });
}

pub const PARTNER_RENOTE_MS: i64 = 30 * 24 * 60 * 60 * 1000;

pub fn dm_cursor(app: &App, stream: &str) -> u64 {
    with_state(app, |state| state.dm_cursors.get(stream).copied()).flatten().unwrap_or(0)
}

pub fn set_dm_cursor(app: &App, stream: String, seq: u64, save: bool) {
    with_state(app, |state| state.dm_cursors.insert(stream, seq));
    if save {
        persist(app);
    }
}

pub fn partners(app: &App) -> BTreeMap<UserId, i64> {
    with_state(app, |state| state.partners.clone()).unwrap_or_default()
}

fn schedule_servers(handle: &AppHandle) {
    if SERVERS_PENDING.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SERVERS_DELAY).await;
        SERVERS_PENDING.store(false, Ordering::SeqCst);
        entry_list::schedule(&handle);
    });
}

fn flush_soon() {
    let Some(handle) = HANDLE.get().cloned() else { return };
    if FLUSH_PENDING.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FLUSH_DELAY).await;
        FLUSH_PENDING.store(false, Ordering::SeqCst);
        flush(&handle).await;
    });
}

async fn flush(handle: &AppHandle) {
    let _guard = FLUSH.lock().await;
    loop {
        let app = handle.state::<App>();
        let Some(next) = with_state(&app, |state| state.outbox.first().cloned()).flatten() else { return };
        let target = LIVE.lock().unwrap().as_ref().map(|l| (l.stream, l.pool.client_for(l.stream)));
        let Some((stream, client)) = target else { return };
        let Ok(plain) = serde_json::to_vec(&next.record) else {
            with_state(&app, |state| state.forget_queued(next.id));
            continue;
        };
        *IN_FLIGHT.lock().unwrap() = Some(next.id);
        let result = sync_link::append(client, stream, plain).await;
        *IN_FLIGHT.lock().unwrap() = None;
        match result {
            Ok(_) => {}
            Err(AppendError::Refused(code)) if code.is_retryable() => return,
            Err(AppendError::Seal(e)) => tracing::warn!("dropped an account log entry: {e}"),
            Err(AppendError::Refused(code)) => tracing::warn!("the sync service refused an account log entry: {code:?}"),
            Err(_) => return,
        }
        with_state(&app, |state| state.forget_queued(next.id));
        persist(&app);
    }
}

#[tauri::command]
pub fn sync_prefs_load(app: tauri::State<'_, App>) -> BTreeMap<String, PrefPiece> {
    with_state(&app, |state| state.prefs.clone()).unwrap_or_default()
}

#[tauri::command]
pub fn sync_prefs_set(app: tauri::State<'_, App>, changes: BTreeMap<String, serde_json::Value>) {
    let at = now_ms();
    let set: BTreeMap<String, PrefPiece> = changes
        .into_iter()
        .filter(|(key, _)| !key.is_empty() && key.len() <= MAX_PREF_KEY)
        .take(MAX_PREFS_PER_CALL)
        .map(|(key, value)| (key, PrefPiece { value, at }))
        .collect();
    if !set.is_empty() {
        queue(&app, Record::Prefs { set });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use commy_core::ServerId;
    use commy_net::entry_list::Entry;

    #[test]
    fn the_newest_community_key_wins_and_its_echo_stays_quiet() {
        let mut state = SyncState::default();
        let server = ServerId::new();
        let first = SharedKey::of(KeyId::random(), &ChannelSecret::random());
        let second = SharedKey::of(KeyId::random(), &ChannelSecret::random());
        assert_eq!(state.fold(Record::CommunityKey { server, key: first.clone(), at: 1 }), Change::CommunityKey);
        assert_eq!(state.fold(Record::CommunityKey { server, key: first.clone(), at: 1 }), Change::Nothing);
        assert_eq!(state.fold(Record::CommunityKey { server, key: second.clone(), at: 0 }), Change::Nothing);
        assert_eq!(state.fold(Record::CommunityKey { server, key: first, at: 2 }), Change::Nothing);
        assert_eq!(state.fold(Record::CommunityKey { server, key: second.clone(), at: 3 }), Change::CommunityKey);
        assert_eq!(state.community.get(&server).map(|(key, _)| key), Some(&second));
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(serde_json::from_str::<SyncState>(&json).unwrap(), state);
        let record = serde_json::to_string(&Record::CommunityKey { server, key: second, at: 3 }).unwrap();
        assert!(record.contains("\"k\":\"community_key\""));
    }

    fn entry(name: &str, minute: i64) -> Entry {
        Entry { name: name.into(), ticket: format!("ticket-{name}"), since: Utc.timestamp_opt(1_760_000_000 + minute * 60, 0).unwrap() }
    }

    fn mark(at: i64) -> ReadMark {
        ReadMark { id: Some(format!("m{at}")), at, set: at, back: false }
    }

    fn saved(message: &str, at: i64) -> SavedMessage {
        SavedMessage { saved_at: at, entry: crate::saved::Entry { message: message.into(), content: "hi".into(), ..Default::default() } }
    }

    #[test]
    fn records_keep_their_shape_and_unknown_kinds_are_skipped() {
        let record = Record::Prefs { set: BTreeMap::from([("notify.level".to_string(), PrefPiece { value: "mentions".into(), at: 5 })]) };
        let json = serde_json::to_string(&record).unwrap();
        assert_eq!(json, r#"{"k":"prefs","set":{"notify.level":{"value":"mentions","at":5}}}"#);
        assert_eq!(serde_json::from_str::<Record>(&json).unwrap(), record);
        assert_eq!(serde_json::from_str::<Record>(r#"{"k":"from_a_newer_build","x":1}"#).unwrap(), Record::Unknown);
        let mut state = SyncState::default();
        assert_eq!(state.fold(Record::Unknown), Change::Nothing);
    }

    #[test]
    fn server_lists_fold_like_the_vault_and_never_carry_the_identity() {
        let (a, b) = (ServerId::new(), ServerId::new());
        let mut state = SyncState::default();
        let mut list = EntryList::default();
        list.servers.insert(a, entry("a", 1));
        list.identity = Some("secret".into());
        assert_eq!(state.fold(Record::Servers { list: list.clone() }), Change::Servers);
        assert_eq!(state.servers.identity, None);
        assert_eq!(state.fold(Record::Servers { list }), Change::Nothing, "folding twice changes nothing");

        let mut later = EntryList::default();
        later.servers.insert(b, entry("b", 2));
        later.left.insert(a, Utc.timestamp_opt(1_760_000_000 + 600, 0).unwrap());
        assert_eq!(state.fold(Record::Servers { list: later }), Change::Servers);
        assert!(!state.servers.servers.contains_key(&a));
        assert!(state.servers.servers.contains_key(&b));
    }

    #[test]
    fn only_what_the_log_lacks_is_published() {
        let (a, b, c) = (ServerId::new(), ServerId::new(), ServerId::new());
        let mut log = EntryList::default();
        log.servers.insert(a, entry("a", 1));
        let mut merged = log.clone();
        merged.servers.insert(b, entry("b", 2));
        merged.left.insert(c, Utc.timestamp_opt(1_760_000_000, 0).unwrap());
        let records = servers_delta(&log, &merged);
        assert_eq!(records.len(), 1);
        let Record::Servers { list } = &records[0] else { panic!("servers record") };
        assert_eq!(list.servers.keys().collect::<Vec<_>>(), vec![&b]);
        assert_eq!(list.left.keys().collect::<Vec<_>>(), vec![&c]);
        assert!(list.profile.is_none());
        assert!(servers_delta(&merged, &merged).is_empty());
    }

    #[test]
    fn big_lists_are_split_under_the_entry_size() {
        let mut merged = EntryList::default();
        for n in 0..400 {
            let mut e = entry(&format!("server {n}"), n);
            e.ticket = "t".repeat(400);
            merged.servers.insert(ServerId::new(), e);
        }
        let records = servers_delta(&EntryList::default(), &merged);
        assert!(records.len() > 1);
        let mut state = SyncState::default();
        for record in records {
            assert!(record_bytes(&record) <= MAX_RECORD_BYTES);
            state.fold(record);
        }
        assert_eq!(state.servers.servers.len(), 400);
    }

    #[test]
    fn saved_messages_follow_the_latest_change() {
        let mut state = SyncState::default();
        let save = |at| Record::Saved { ops: vec![SavedOp { message: "m".into(), at, saved: Some(saved("m", at)) }] };
        let unsave = |at| Record::Saved { ops: vec![SavedOp { message: "m".into(), at, saved: None }] };
        assert!(matches!(state.fold(save(10)), Change::Saved(_)));
        assert!(matches!(state.fold(unsave(20)), Change::Saved(_)));
        assert_eq!(state.fold(save(15)), Change::Nothing, "an older save does not undo a later removal");
        assert!(matches!(state.fold(save(30)), Change::Saved(_)));
        let mismatched = Record::Saved { ops: vec![SavedOp { message: "x".into(), at: 99, saved: Some(saved("m", 99)) }] };
        assert_eq!(state.fold(mismatched), Change::Nothing);
    }

    #[test]
    fn settings_take_the_latest_write_per_key() {
        let mut state = SyncState::default();
        let set = |key: &str, value: &str, at| Record::Prefs { set: BTreeMap::from([(key.to_string(), PrefPiece { value: value.into(), at })]) };
        assert!(matches!(state.fold(set("notify.level", "all", 5)), Change::Prefs(_)));
        assert_eq!(state.fold(set("notify.level", "none", 4)), Change::Nothing);
        let Change::Prefs(taken) = state.fold(set("notify.level", "mentions", 6)) else { panic!("newer wins") };
        assert_eq!(taken["notify.level"].value, "mentions");
        assert_eq!(state.fold(set("notify.level", "mentions", 6)), Change::Nothing);
    }

    #[test]
    fn partners_need_an_identity_and_are_noted_once_per_change() {
        let mut state = SyncState::default();
        let mut friend = UserProfile::new("friend");
        assert_eq!(state.fold(Record::DmPartner { profile: friend.clone(), at: 1 }), Change::Nothing);
        friend.identity = Some(commy_core::Identity::generate().public());
        assert!(matches!(state.fold(Record::DmPartner { profile: friend.clone(), at: 1 }), Change::Partner(_)));
        assert_eq!(state.fold(Record::DmPartner { profile: friend, at: 1 }), Change::Nothing);
    }

    #[test]
    fn the_outbox_merges_read_positions_but_not_one_already_sending() {
        let mut state = SyncState::default();
        state.enqueue(Record::Read { marks: BTreeMap::from([("a".to_string(), mark(1))]) }, None);
        state.enqueue(Record::Read { marks: BTreeMap::from([("b".to_string(), mark(2))]) }, None);
        assert_eq!(state.outbox.len(), 1);
        let sending = state.outbox[0].id;
        state.enqueue(Record::Read { marks: BTreeMap::from([("c".to_string(), mark(3))]) }, Some(sending));
        assert_eq!(state.outbox.len(), 2);
        state.forget_queued(sending);
        assert_eq!(state.outbox.len(), 1);
        for n in 0..(MAX_OUTBOX + 10) {
            state.enqueue(Record::Prefs { set: BTreeMap::from([(format!("k{n}"), PrefPiece { value: 1.into(), at: 1 })]) }, None);
        }
        assert_eq!(state.outbox.len(), MAX_OUTBOX);
    }

    #[test]
    fn a_new_entry_key_starts_a_new_log_but_keeps_settings_and_dm_cursors() {
        let mut state = SyncState { stream: "old".into(), after: 9, seeded: true, ..Default::default() };
        state.prefs.insert("notify.level".into(), PrefPiece { value: "all".into(), at: 1 });
        state.dm_cursors.insert("dm".into(), 4);
        state.saved.insert("m".into(), 1);
        state.start_stream("new");
        assert_eq!((state.stream.as_str(), state.after, state.seeded), ("new", 0, false));
        assert_eq!(state.prefs.len(), 1);
        assert_eq!(state.dm_cursors["dm"], 4);
        assert!(state.saved.is_empty());
        state.start_stream("new");
        assert_eq!(state.stream, "new");
    }

    #[test]
    fn read_and_saved_batches_stay_small() {
        let marks: BTreeMap<String, ReadMark> = (0..700).map(|n| (format!("c{n}"), mark(n))).collect();
        let records = read_records(marks);
        assert_eq!(records.len(), 3);
        let ops: Vec<SavedOp> = (0..500).map(|n| SavedOp { message: format!("m{n}"), at: n, saved: Some(saved(&format!("m{n}"), n)) }).collect();
        for record in saved_records(ops) {
            assert!(record_bytes(&record) <= MAX_RECORD_BYTES);
        }
    }
}
