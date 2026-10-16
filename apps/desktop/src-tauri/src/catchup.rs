//! Catch-up with gossip neighbors, the app's side of `commy_net::catchup`:
//! the store over `App.servers`, when sessions start, what happens after
//! one, and events held until their signer's key is known. Also the one
//! snapshot a newly joined server takes.

use crate::state::App;
use chrono::Utc;
use commy_core::catchup::{digests, items, BucketItems};
use commy_core::{
    BucketDigest, BucketKey, ChannelId, Identity, IdentityKey, Message, MessageId, ServerEvent,
    ServerId, ServerState, StoreError, TombstoneProof, UserId, UserProfile,
};
use commy_net::catchup::{
    apply_items, apply_profiles, check_peer, collect, profiles_of, ApplyReport, CatchupError,
    CatchupStore, PeerCheck, SessionReport,
};
use commy_net::{SharedCatchupStore, WirePayload};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

/// The side with the higher node id waits this long, so the lower one
/// usually starts and the two do not run the same session twice.
pub const SECOND_START_DELAY: Duration = Duration::from_secs(15);

/// A delayed start is skipped if a session with that peer for that
/// server completed this recently.
pub const RECENT_SESSION: Duration = Duration::from_secs(60);

/// Each server catches up with one random neighbor this often.
pub const PERIODIC_INTERVAL: Duration = Duration::from_secs(600);

/// Servers that catch-up changed are saved together this long after the
/// first change, not once per page; a session's end saves its server at once.
pub const SAVE_DELAY: Duration = Duration::from_secs(2);

/// Events held per server for an unknown author key, and for how long.
pub const HOLD_CAP: usize = 1000;
pub const HOLD_FOR: Duration = Duration::from_secs(15 * 60);
/// Most bytes of events held per server, counted as sent.
pub const HOLD_BYTES: usize = 4 * 1024 * 1024;

type SessionKey = (ServerId, [u8; 32]);

/// An event held until its signer's key is known: when it was held, the
/// signer, and the event.
pub type Held = (Instant, UserId, ServerEvent);

/// Catch-up bookkeeping kept in `App.catchup`.
#[derive(Debug, Default)]
pub struct Catchup {
    /// Sessions we started that are still running.
    running: HashSet<SessionKey>,
    /// When the last session with a peer completed, either side.
    completed: HashMap<SessionKey, Instant>,
    /// Remote message events whose signer has no known key yet, with
    /// their size (see [`held_size`]).
    held: HashMap<ServerId, VecDeque<(Held, usize)>>,
    /// New messages from others per channel, not yet told to the webview.
    fresh: HashMap<ServerId, HashMap<ChannelId, u32>>,
    /// Servers changed since the webview was last told.
    dirty: HashSet<ServerId>,
    /// Servers changed in memory and not saved yet.
    unsaved: HashSet<ServerId>,
    /// A delayed save of `unsaved` is on its way.
    save_scheduled: bool,
}

/// What catch-up changed in one server since the last look.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub changed: bool,
    pub fresh: HashMap<ChannelId, u32>,
}

impl Catchup {
    /// Marks a session as running; false if one with that peer already is.
    pub fn begin(&mut self, key: SessionKey) -> bool {
        self.running.insert(key)
    }

    pub fn finish(&mut self, key: SessionKey, now: Instant, completed: bool) {
        self.running.remove(&key);
        if completed {
            self.record_completed(key, now);
        }
    }

    /// A session completed, possibly one the peer started.
    pub fn record_completed(&mut self, key: SessionKey, now: Instant) {
        self.completed.retain(|_, at| now.saturating_duration_since(*at) < RECENT_SESSION);
        self.completed.insert(key, now);
    }

    pub fn completed_recently(&self, key: SessionKey, now: Instant) -> bool {
        self.completed
            .get(&key)
            .is_some_and(|at| now.saturating_duration_since(*at) < RECENT_SESSION)
    }

    /// Holds an event until its signer's key arrives; the oldest go past
    /// the caps (see [`trim_held`]). One already held for the same message
    /// and kind is kept instead, or replaced by a newer edit.
    pub fn hold(&mut self, server: ServerId, signer: UserId, event: ServerEvent, now: Instant) {
        let queue = self.held.entry(server).or_default();
        if let Some(key) = hold_key(&event) {
            if let Some(slot) = queue.iter_mut().find(|((_, _, held), _)| hold_key(held) == Some(key)) {
                if let Some(size) = held_size(&event).filter(|_| newer_edit(&event, &slot.0 .2)) {
                    *slot = ((slot.0 .0, signer, event), size);
                    trim_held(queue);
                }
                return;
            }
        }
        let Some(size) = held_size(&event) else {
            tracing::debug!("not holding an event too large to send");
            return;
        };
        queue.push_back(((now, signer, event), size));
        trim_held(queue);
    }

    #[cfg(test)]
    pub fn held_count(&self, server: ServerId) -> usize {
        self.held.get(&server).map_or(0, VecDeque::len)
    }

    /// [`Self::take_held`] for every signer.
    #[cfg(test)]
    pub fn take_all_held(&mut self, server: ServerId, now: Instant) -> Vec<Held> {
        let signers = self.held.get(&server).map(|q| q.iter().map(|((_, by, _), _)| *by).collect()).unwrap_or_default();
        self.take_held(server, &signers, now)
    }

    /// The server's held events signed by one of `signers`, oldest first;
    /// the others stay held. Expired ones go.
    pub fn take_held(&mut self, server: ServerId, signers: &HashSet<UserId>, now: Instant) -> Vec<Held> {
        let Some(queue) = self.held.get_mut(&server) else { return Vec::new() };
        let (taken, kept): (VecDeque<_>, VecDeque<_>) =
            queue.drain(..).filter(|((at, _, _), _)| !expired(*at, now)).partition(|((_, by, _), _)| signers.contains(by));
        *queue = kept;
        if queue.is_empty() {
            self.held.remove(&server);
        }
        taken.into_iter().map(|(held, _)| held).collect()
    }

    /// Puts events still waiting back in front of any held meanwhile,
    /// unless the same one was held again in between.
    pub fn restore_held(&mut self, server: ServerId, events: Vec<Held>) {
        if events.is_empty() {
            return;
        }
        let queue = self.held.entry(server).or_default();
        for held in events.into_iter().rev() {
            let again = hold_key(&held.2).is_some_and(|key| queue.iter().any(|((_, _, e), _)| hold_key(e) == Some(key)));
            if let Some(size) = held_size(&held.2).filter(|_| !again) {
                queue.push_front((held, size));
            }
        }
        trim_held(queue);
    }

    /// Drops held events past [`HOLD_FOR`] in every server.
    pub fn expire_held(&mut self, now: Instant) {
        for queue in self.held.values_mut() {
            queue.retain(|((at, _, _), _)| !expired(*at, now));
        }
        self.held.retain(|_, queue| !queue.is_empty());
    }

    /// Notes that a server changed, with the channels of new messages.
    pub fn note_applied(&mut self, server: ServerId, fresh: impl IntoIterator<Item = ChannelId>) {
        self.dirty.insert(server);
        self.unsaved.insert(server);
        let counts = self.fresh.entry(server).or_default();
        for channel in fresh {
            *counts.entry(channel).or_default() += 1;
        }
        if counts.is_empty() {
            self.fresh.remove(&server);
        }
    }

    pub fn take_changes(&mut self, server: ServerId) -> Changes {
        Changes {
            changed: self.dirty.remove(&server),
            fresh: self.fresh.remove(&server).unwrap_or_default(),
        }
    }

    /// True when a server waits to be saved and no save is on its way yet;
    /// the caller then starts one (see [`SAVE_DELAY`]).
    pub fn schedule_save(&mut self) -> bool {
        if self.unsaved.is_empty() || self.save_scheduled {
            return false;
        }
        self.save_scheduled = true;
        true
    }

    /// The servers to save now; the next change schedules a save again.
    pub fn take_unsaved(&mut self) -> Vec<ServerId> {
        self.save_scheduled = false;
        self.unsaved.drain().collect()
    }

    /// True when `server` waited to be saved, which the caller now does.
    pub fn take_unsaved_server(&mut self, server: ServerId) -> bool {
        self.unsaved.remove(&server)
    }

    /// Everything about a server we left.
    pub fn forget_server(&mut self, server: ServerId) {
        self.held.remove(&server);
        self.fresh.remove(&server);
        self.dirty.remove(&server);
        self.unsaved.remove(&server);
        self.completed.retain(|(s, _), _| *s != server);
    }
}

fn expired(at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(at) >= HOLD_FOR
}

/// What a held event weighs against [`HOLD_BYTES`]: its size as sent.
/// None when it could not be sent as one gossip event, as no valid
/// message event can, so it is not worth holding.
fn held_size(event: &ServerEvent) -> Option<usize> {
    let size = commy_net::protocol::gossip_size(&WirePayload::Event(event.clone()));
    (size <= commy_net::protocol::MAX_FRAME_BYTES).then_some(size)
}

/// Drops the oldest held events past [`HOLD_CAP`] or [`HOLD_BYTES`].
fn trim_held(queue: &mut VecDeque<(Held, usize)>) {
    let mut bytes: usize = queue.iter().map(|(_, size)| size).sum();
    while queue.len() > HOLD_CAP || bytes > HOLD_BYTES {
        let Some((_, size)) = queue.pop_front() else { break };
        bytes -= size;
    }
}

/// What makes two held events the same: their kind and message, and for
/// a delete who made it, since each deleter's proof stands on its own.
fn hold_key(event: &ServerEvent) -> Option<(u8, MessageId, Option<UserId>)> {
    match event {
        ServerEvent::MessageCreate { message } => Some((0, message.id, None)),
        ServerEvent::MessageEdit { message, .. } => Some((1, *message, None)),
        ServerEvent::MessageDelete { message, by, .. } => Some((2, *message, Some(*by))),
        _ => None,
    }
}

/// True when `new` is an edit made after the held edit `old`.
fn newer_edit(new: &ServerEvent, old: &ServerEvent) -> bool {
    matches!(
        (new, old),
        (ServerEvent::MessageEdit { edited_at: a, .. }, ServerEvent::MessageEdit { edited_at: b, .. }) if a > b
    )
}

/// When to start a session with a new neighbor: at once if we have
/// pending sends or the lower node id, otherwise after a pause.
pub fn start_delay(ours: [u8; 32], peer: [u8; 32], pending: bool) -> Option<Duration> {
    (!pending && ours > peer).then_some(SECOND_START_DELAY)
}

/// Message events that wait for their signer's key instead of being lost.
pub fn holdable(event: &ServerEvent) -> bool {
    matches!(
        event,
        ServerEvent::MessageCreate { .. } | ServerEvent::MessageEdit { .. } | ServerEvent::MessageDelete { .. }
    )
}

/// Which new messages count as unread for the webview.
#[derive(Debug, Default)]
pub struct Unread {
    me: Option<UserId>,
    blocked: HashSet<UserId>,
    /// Nothing counts: history a newly joined server receives before its
    /// first snapshot.
    silent: bool,
}

impl Unread {
    /// Read from the app before its servers are locked.
    pub fn of(app: &App, server: ServerId) -> Self {
        Self {
            me: app.profile.lock().unwrap().as_ref().map(|p| p.id),
            blocked: app.safety.lock().unwrap().blocked.iter().copied().collect(),
            silent: app.awaits_snapshot(server),
        }
    }

    /// Whether a message not in `state` yet is unread once applied: someone
    /// else wrote it, not someone we blocked, and not before we joined.
    fn counts(&self, state: &ServerState, message: &Message) -> bool {
        if self.silent
            || Some(message.author) == self.me
            || self.blocked.contains(&message.author)
            || state.has_message(message.id)
        {
            return false;
        }
        let joined = self
            .me
            .and_then(|me| state.server.members.iter().find(|m| m.user == me))
            .map(|m| m.joined_at);
        joined.is_none_or(|at| message.created_at >= at)
    }
}

/// Applies one catch-up page. Returns the report, the events to hold
/// with their signer, and the channels of new messages that count as
/// unread.
pub fn apply_page(
    state: &mut ServerState,
    unread: &Unread,
    messages: Vec<Message>,
    tombstones: Vec<TombstoneProof>,
) -> (ApplyReport, Vec<(UserId, ServerEvent)>, Vec<ChannelId>) {
    let new: HashMap<MessageId, ChannelId> = messages
        .iter()
        .filter(|m| unread.counts(state, m))
        .map(|m| (m.id, m.channel))
        .collect();
    let (report, held) = apply_items(state, messages, tombstones);
    let held = held.into_iter().filter_map(|event| Some((item_signer(&event)?, event))).collect();
    let fresh = report
        .applied
        .iter()
        .filter(|id| state.has_message(**id))
        .filter_map(|id| new.get(id).copied())
        .collect();
    (report, held, fresh)
}

/// Who signed a catch-up item: a message's author, a delete's deleter.
fn item_signer(event: &ServerEvent) -> Option<UserId> {
    match event {
        ServerEvent::MessageCreate { message } => Some(message.author),
        ServerEvent::MessageDelete { by, .. } => Some(*by),
        _ => None,
    }
}

/// Outcome of retrying held events.
#[derive(Debug, Default)]
pub struct Retried {
    pub changed: bool,
    pub fresh: Vec<ChannelId>,
    /// Events whose signer is still unknown.
    pub still: Vec<Held>,
}

/// Re-applies held events in order; those still missing a key stay held.
pub fn retry_held(state: &mut ServerState, unread: &Unread, events: Vec<Held>) -> Retried {
    let mut out = Retried::default();
    for (at, _, event) in events {
        let new_message = match &event {
            ServerEvent::MessageCreate { message } if unread.counts(state, message) => Some((message.id, message.channel)),
            _ => None,
        };
        match state.apply(event.clone()) {
            Ok(true) => {
                out.changed = true;
                if let Some((id, channel)) = new_message {
                    if state.has_message(id) {
                        out.fresh.push(channel);
                    }
                }
            }
            Ok(false) => {}
            Err(StoreError::UnknownAuthorKey(signer)) => out.still.push((at, signer, event)),
            Err(e) => tracing::debug!("held event rejected: {e}"),
        }
    }
    out
}

/// Applies a live event from a peer. Returns the store's answer; for a
/// create, the message when it is new here, absent before and present
/// after (a late copy kept out by a stored delete may return Ok(true)
/// because its proof changed, but it is no new message); and for a join
/// or profile, the user when it pinned their key, so what was held for
/// it may go in.
pub fn apply_live(
    state: &mut ServerState,
    event: ServerEvent,
) -> (Result<bool, StoreError>, Option<Message>, Option<UserId>) {
    let created = match &event {
        ServerEvent::MessageCreate { message } if !state.has_message(message.id) => Some(message.clone()),
        _ => None,
    };
    let keyless = match &event {
        ServerEvent::MemberJoin { profile, .. } | ServerEvent::ProfileUpdate { profile } => {
            Some(profile.id).filter(|user| state.identity_of(*user).is_none())
        }
        _ => None,
    };
    let applied = state.apply(event);
    let created = created.filter(|m| matches!(applied, Ok(true)) && state.has_message(m.id));
    let pinned = keyless.filter(|user| applied.is_ok() && state.identity_of(*user).is_some());
    (applied, created, pinned)
}

/// Payload of `messages-caught-up`: new messages per channel, so the
/// webview can bump unread counts without a toast per message.
pub fn caught_up_payload(server: ServerId, fresh: &HashMap<ChannelId, u32>) -> serde_json::Value {
    let channels: serde_json::Map<String, serde_json::Value> =
        fresh.iter().map(|(channel, count)| (channel.to_string(), (*count).into())).collect();
    serde_json::json!({ "server": server.to_string(), "channels": channels })
}

/// One neighbor picked by `seed`, for the periodic session.
fn pick(neighbors: &[[u8; 32]], seed: u64) -> Option<[u8; 32]> {
    (!neighbors.is_empty()).then(|| neighbors[(seed % neighbors.len() as u64) as usize])
}

// ---------- The store ----------

/// Reaches the app state and the webview from a tauri handle, or from a
/// plain `App` in tests.
pub trait AppAccess: Send + Sync + 'static {
    fn app(&self) -> &App;
    /// Tells the webview something changed.
    fn emit(&self, event: &str, payload: serde_json::Value);
    /// Saves the servers changed meanwhile after [`SAVE_DELAY`], in the
    /// background (see [`save_unsaved`]).
    fn save_later(&self);
}

impl AppAccess for tauri::AppHandle {
    fn app(&self) -> &App {
        self.state::<App>().inner()
    }

    fn emit(&self, event: &str, payload: serde_json::Value) {
        let _ = Emitter::emit(self, event, payload);
    }

    fn save_later(&self) {
        let handle = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(SAVE_DELAY).await;
            save_unsaved(handle.app());
        });
    }
}

/// Saves every server catch-up changed since the last save.
pub fn save_unsaved(app: &App) {
    let servers = app.catchup.lock().unwrap().take_unsaved();
    for server in servers {
        app.persist_server(server);
    }
}

/// An `App` without a webview that records what it would have been told.
/// Delayed saves run on a thread of their own.
#[cfg(test)]
#[derive(Clone, Default)]
pub struct TestHandle {
    pub app: Arc<App>,
    events: Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>>,
    pub saves_scheduled: Arc<std::sync::atomic::AtomicUsize>,
}

#[cfg(test)]
impl TestHandle {
    pub fn new(app: Arc<App>) -> Self {
        Self { app, events: Default::default(), saves_scheduled: Default::default() }
    }

    /// Events emitted since the last call, oldest first.
    pub fn take_events(&self) -> Vec<(String, serde_json::Value)> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }
}

#[cfg(test)]
impl AppAccess for TestHandle {
    fn app(&self) -> &App {
        &self.app
    }

    fn emit(&self, event: &str, payload: serde_json::Value) {
        self.events.lock().unwrap().push((event.to_string(), payload));
    }

    fn save_later(&self) {
        self.saves_scheduled.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let app = self.app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(SAVE_DELAY);
            save_unsaved(&app);
        });
    }
}

/// [`CatchupStore`] over `App.servers`. Every call locks briefly.
pub struct AppStore<A>(pub A);

impl<A: AppAccess> AppStore<A> {
    fn with_state<T>(&self, server: ServerId, f: impl FnOnce(&ServerState) -> T) -> Option<T> {
        self.0.app().servers.lock().unwrap().get(&server).map(f)
    }
}

impl<A: AppAccess> CatchupStore for AppStore<A> {
    fn local_identity(&self, server: ServerId) -> Option<(UserId, Identity)> {
        let app = self.0.app();
        // A key that may still be replaced is not shown to anyone yet.
        if !app.key_settled() {
            return None;
        }
        let me = app.profile.lock().unwrap().as_ref().map(|p| p.id)?;
        if !self.with_state(server, |s| s.server.is_member(me))? {
            return None;
        }
        app.identity().map(|identity| (me, identity))
    }

    fn verify_peer(&self, server: ServerId, user: UserId, key: &IdentityKey) -> PeerCheck {
        self.with_state(server, |s| check_peer(s, user, key)).unwrap_or(PeerCheck::Denied)
    }

    fn digests(&self, server: ServerId) -> Vec<BucketDigest> {
        self.with_state(server, |s| digests(s, Utc::now())).unwrap_or_default()
    }

    fn items(&self, server: ServerId, keys: &[BucketKey]) -> Vec<BucketItems> {
        self.with_state(server, |s| items(s, keys, Utc::now())).unwrap_or_default()
    }

    fn get(&self, server: ServerId, ids: &[MessageId]) -> (Vec<Message>, Vec<TombstoneProof>) {
        self.with_state(server, |s| collect(s, ids)).unwrap_or_default()
    }

    fn profiles(&self, server: ServerId, users: &[UserId]) -> Vec<UserProfile> {
        self.with_state(server, |s| profiles_of(s, users)).unwrap_or_default()
    }

    fn apply_profiles(&self, server: ServerId, profiles: Vec<UserProfile>) {
        let app = self.0.app();
        let learned = {
            let mut servers = app.servers.lock().unwrap();
            let Some(state) = servers.get_mut(&server) else { return };
            apply_profiles(state, profiles)
        };
        if learned.is_empty() {
            return;
        }
        app.catchup.lock().unwrap().note_applied(server, []);
        // Items held for these keys can go in now.
        release_held(&self.0, server, &learned);
    }

    fn apply(&self, server: ServerId, messages: Vec<Message>, tombstones: Vec<TombstoneProof>) -> ApplyReport {
        let app = self.0.app();
        let unread = Unread::of(app, server);
        let (report, held, fresh) = {
            let mut servers = app.servers.lock().unwrap();
            let Some(state) = servers.get_mut(&server) else {
                // Left meanwhile: nothing was taken, so the peer counts none of it.
                let rejected = messages.iter().map(|m| m.id).chain(tombstones.iter().map(|p| p.message));
                return ApplyReport { rejected: dedup(rejected), ..Default::default() };
            };
            apply_page(state, &unread, messages, tombstones)
        };
        let now = Instant::now();
        {
            let mut catchup = app.catchup.lock().unwrap();
            for (signer, event) in held {
                catchup.hold(server, signer, event, now);
            }
            if !report.applied.is_empty() {
                catchup.note_applied(server, fresh);
            }
        }
        // Shown at once and saved soon, however the session ends.
        if !report.applied.is_empty() {
            flush(&self.0, server, false);
        }
        report
    }
}

/// Ids in their first order, each once.
fn dedup(ids: impl IntoIterator<Item = MessageId>) -> Vec<MessageId> {
    let mut seen = HashSet::new();
    ids.into_iter().filter(|id| seen.insert(*id)).collect()
}

/// The catch-up store handed to the node.
pub fn store(handle: &tauri::AppHandle) -> SharedCatchupStore {
    Arc::new(AppStore(handle.clone()))
}

// ---------- Scheduling ----------

fn our_node(app: &App) -> Option<[u8; 32]> {
    let hex = app.node_hex.lock().unwrap().clone()?;
    data_encoding::HEXLOWER.decode(hex.as_bytes()).ok()?.try_into().ok()
}

/// A gossip neighbor came up: schedule a session with it, see [`start_delay`].
pub fn on_neighbor_up(handle: &tauri::AppHandle, server: ServerId, peer: [u8; 32]) {
    let app = handle.state::<App>();
    let Some(ours) = our_node(&app) else { return };
    let pending = app.outbox.lock().unwrap().has_pending(server);
    spawn_session(handle.clone(), server, peer, start_delay(ours, peer, pending), true);
}

/// Runs one session with `peer` after `delay`, at most one per server and
/// peer at a time. A delayed one is skipped when a session with that peer
/// completed meanwhile. With `retry`, a refusal that names a delay is
/// tried once more after it.
pub fn spawn_session(handle: tauri::AppHandle, server: ServerId, peer: [u8; 32], delay: Option<Duration>, retry: bool) {
    tauri::async_runtime::spawn(async move {
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
            let recent = handle.state::<App>().catchup.lock().unwrap().completed_recently((server, peer), Instant::now());
            if recent {
                return;
            }
        }
        run_session(&handle, server, peer, retry).await;
    });
}

async fn run_session(handle: &tauri::AppHandle, server: ServerId, peer: [u8; 32], retry: bool) {
    let app = handle.state::<App>();
    if !app.catchup.lock().unwrap().begin((server, peer)) {
        return;
    }
    // The session future owns what it needs, so the network lock is
    // released before it runs.
    let session = {
        let net = app.net.lock().await;
        net.as_ref().map(|node| node.catch_up(server, peer, store(handle)))
    };
    let result = match session {
        Some(session) => session.await,
        None => Err(anyhow::anyhow!("network is not ready")),
    };
    app.catchup.lock().unwrap().finish((server, peer), Instant::now(), result.is_ok());
    match result {
        Ok(report) => {
            tracing::info!(
                "caught up on {server}: delivered {}, received {}, held {}",
                report.delivered.len(),
                report.received.len(),
                report.held
            );
            after_session(handle, server, Some(&report));
        }
        Err(e) => {
            tracing::debug!("catch-up on {server} did not complete: {e:#}");
            // Pages applied before the failure still count.
            after_session(handle, server, None);
            let wait = e.downcast_ref::<CatchupError>().and_then(CatchupError::retry_after_ms);
            if let (true, Some(ms)) = (retry, wait) {
                spawn_session(handle.clone(), server, peer, Some(Duration::from_millis(ms)), false);
            }
        }
    }
}

/// A peer finished a session it started with us.
pub fn on_caught_up(handle: &tauri::AppHandle, server: ServerId, peer: [u8; 32], report: &SessionReport) {
    handle.state::<App>().catchup.lock().unwrap().record_completed((server, peer), Instant::now());
    after_session(handle, server, Some(report));
}

/// Clears pending sends the peer confirmed it holds, then saves and
/// announces what the session changed. A session that fails before this
/// runs is saved by the delayed save its pages scheduled.
pub fn after_session<A: AppAccess>(access: &A, server: ServerId, report: Option<&SessionReport>) {
    let app = access.app();
    let mut outbox_changed = false;
    if let Some(report) = report {
        let delivered = report.delivered.iter().copied();
        outbox_changed = app.outbox.lock().unwrap().clear_delivered(server, delivered);
    }
    {
        let servers = app.servers.lock().unwrap();
        if let Some(state) = servers.get(&server) {
            outbox_changed |= app.outbox.lock().unwrap().prune(state, Utc::now());
        }
    }
    if outbox_changed {
        app.persist_outbox();
    }
    if app.catchup.lock().unwrap().take_unsaved_server(server) {
        app.persist_server(server);
    }
    flush(access, server, outbox_changed);
}

/// Tells the webview what catch-up or released events changed in a
/// server, `server-event` to redraw and `messages-caught-up` for unread
/// counts, and schedules saving it (see [`SAVE_DELAY`]).
fn flush<A: AppAccess>(access: &A, server: ServerId, redraw: bool) {
    let app = access.app();
    let (changes, save) = {
        let mut catchup = app.catchup.lock().unwrap();
        (catchup.take_changes(server), catchup.schedule_save())
    };
    if save {
        access.save_later();
    }
    if changes.changed || redraw {
        access.emit("server-event", server.to_string().into());
    }
    if !changes.fresh.is_empty() {
        access.emit("messages-caught-up", caught_up_payload(server, &changes.fresh));
    }
}

/// Holds a remote message event whose signer has no known key yet.
pub fn hold(app: &App, server: ServerId, signer: UserId, event: ServerEvent) {
    app.catchup.lock().unwrap().hold(server, signer, event, Instant::now());
}

/// Retries a server's held events signed by `signers`, whose keys just
/// became known; the rest stay held. New messages bump unread counts
/// silently.
pub fn release_held<A: AppAccess>(access: &A, server: ServerId, signers: &[UserId]) {
    let app = access.app();
    let signers: HashSet<UserId> = signers.iter().copied().collect();
    let events = if signers.is_empty() {
        Vec::new()
    } else {
        app.catchup.lock().unwrap().take_held(server, &signers, Instant::now())
    };
    if events.is_empty() {
        flush(access, server, false);
        return;
    }
    let unread = Unread::of(app, server);
    let retried = {
        let mut servers = app.servers.lock().unwrap();
        let Some(state) = servers.get_mut(&server) else { return };
        retry_held(state, &unread, events)
    };
    {
        let mut catchup = app.catchup.lock().unwrap();
        catchup.restore_held(server, retried.still);
        if retried.changed {
            catchup.note_applied(server, retried.fresh);
        }
    }
    flush(access, server, false);
}

// ---------- The first snapshot ----------

/// The snapshot we answer a newcomer's request with, none for a server
/// still waiting for its own: what we hold there is a placeholder. While
/// our key may still be replaced, it stays out of it, and so does what we
/// signed with it: our messages arrive by catch-up once it settled.
pub fn snapshot_for(app: &App, server: ServerId) -> Option<WirePayload> {
    if app.awaits_snapshot(server) {
        return None;
    }
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let settled = app.key_settled();
    let mut payload = commy_net::protocol::build_sync_response(app.servers.lock().unwrap().get(&server)?, 50);
    if let WirePayload::SyncResponse { profiles, recent_messages, .. } = &mut payload {
        if !settled {
            for profile in profiles.iter_mut().filter(|p| Some(p.id) == me) {
                profile.identity = None;
            }
            recent_messages.retain(|m| Some(m.author) != me);
        }
    }
    Some(payload)
}

/// What we send when someone on `server`'s topic asks for a snapshot:
/// ours, if we have one to give (see [`snapshot_for`]), then our own join
/// either way, so a newcomer whose snapshot was too large to name every
/// member still learns of each one online. Nothing for a server we are
/// not in.
pub fn sync_replies(app: &App, server: ServerId) -> Vec<WirePayload> {
    let Some(me) = app.profile.lock().unwrap().clone() else { return Vec::new() };
    if !app.servers.lock().unwrap().contains_key(&server) {
        return Vec::new();
    }
    let join = ServerEvent::MemberJoin { server, profile: app.announced(me) };
    snapshot_for(app, server).into_iter().chain([WirePayload::Event(join)]).collect()
}

/// Marks a server whose placeholder was just placed from a ticket as
/// waiting for its first snapshot, and returns what to send there: our
/// join, then a request for a snapshot. Joining asked for one already,
/// but an answer that came before the wait was marked was dropped.
pub fn await_snapshot(app: &App, server: ServerId, me: UserProfile) -> Vec<WirePayload> {
    app.set_awaiting_snapshot(server, true);
    let join = ServerEvent::MemberJoin { server, profile: app.announced(me) };
    vec![WirePayload::Event(join), WirePayload::SyncRequest]
}

/// Takes a `SyncResponse` that arrived on `server`'s topic. Only a server
/// joined from a ticket that still waits for its first snapshot takes
/// one (see `App::awaiting_snapshot`), and only a snapshot of that same
/// server. It replaces the provisional state and trusts the keys it
/// carries: keys pinned meanwhile are provisional, so only those it
/// leaves out stay, and our own. What came meanwhile is kept: members,
/// messages with their edits, and deletes. Every other state keeps its
/// pinned keys and delete proofs; catch-up brings what it lacks. Returns
/// true when the snapshot was taken.
pub fn apply_snapshot<A: AppAccess>(access: &A, server: ServerId, payload: WirePayload) -> bool {
    let app = access.app();
    let WirePayload::SyncResponse { server: mut remote_server, members, profiles, recent_messages } = payload else {
        return false;
    };
    if !app.awaits_snapshot(server) || remote_server.id != server {
        return false;
    }
    remote_server.members = members.into_iter().map(member_of).collect();
    let me = app.profile.lock().unwrap().clone();
    let me_id = me.as_ref().map(|p| p.id);
    let mut held = Vec::new();
    let learned: Vec<UserId>;
    {
        let mut servers = app.servers.lock().unwrap();
        // A server left meanwhile stays gone.
        let Some(old) = servers.remove(&server) else { return false };
        let pinned: HashMap<UserId, IdentityKey> = old.profiles.values().filter_map(|p| Some((p.id, p.identity?))).collect();
        let mut state = ServerState::new(remote_server);
        // Snapshots carry no emojis or sounds; keep any announced already.
        if old.server.rules.is_some() {
            let _ = state.apply(ServerEvent::RulesSet { server, rules: old.server.rules.clone() });
        }
        for event in old.server.events.iter().cloned() {
            let rsvps = event.rsvps.clone();
            let id = event.id.clone();
            let _ = state.apply(ServerEvent::EventUpsert { server, event });
            for (user, rsvp) in rsvps {
                let _ = state.apply(ServerEvent::EventRsvp { server, event: id.clone(), user, rsvp: Some(rsvp) });
            }
        }
        for emoji in old.server.emojis.iter().cloned() {
            let _ = state.apply(ServerEvent::EmojiAdd { server, emoji });
        }
        for sound in old.server.soundboard.iter().cloned() {
            let _ = state.apply(ServerEvent::SoundAdd { server, sound });
        }
        for profile in profiles {
            state.profiles.insert(profile.id, profile);
        }
        // A key pinned here that the snapshot left out stays.
        for profile in old.profiles.values().filter(|p| p.identity.is_some()) {
            let entry = state.profiles.entry(profile.id).or_insert_with(|| profile.clone());
            entry.identity = entry.identity.or(profile.identity);
        }
        // So do members whose joins came first: a snapshot too large to
        // name every member leaves some out.
        let heard = old.server.members.iter().filter_map(|m| old.profiles.get(&m.user));
        for profile in heard.filter(|p| Some(p.id) != me_id) {
            let profile = state.profiles.get(&profile.id).cloned().unwrap_or_else(|| profile.clone());
            let _ = state.apply(ServerEvent::MemberJoin { server, profile });
        }
        // Keep our own profile visible, with the key we sign with, and
        // join as startup does: the snapshot may predate our join.
        if let Some(me) = me {
            let key = me.identity;
            let entry = state.profiles.entry(me.id).or_insert_with(|| me.clone());
            entry.identity = key.or(entry.identity);
            if let Err(e) = state.apply(ServerEvent::MemberJoin { server, profile: me }) {
                tracing::warn!("could not join {server} from its snapshot: {e}");
            }
        }
        for message in recent_messages {
            let event = ServerEvent::MessageCreate { message };
            if let Err(StoreError::UnknownAuthorKey(signer)) = state.apply(event.clone()) {
                held.push((signer, event));
            }
        }
        // Then what came while we waited, our pending messages among it,
        // wherever its channel and signer's key hold.
        let waited = old.snapshot();
        for message in waited.messages.into_iter().flat_map(|(_, list)| list) {
            let _ = state.apply(ServerEvent::MessageCreate { message });
        }
        for proof in waited.tombstone_proofs {
            let _ = state.apply(proof.into_event());
        }
        learned = state
            .profiles
            .values()
            .filter(|p| p.identity.is_some() && p.identity != pinned.get(&p.id).copied())
            .map(|p| p.id)
            .collect();
        servers.insert(server, state);
    }
    for (signer, event) in held {
        hold(app, server, signer, event);
    }
    app.persist_server(server);
    // Still a newcomer here, so released history is not counted as unread.
    release_held(access, server, &learned);
    app.set_awaiting_snapshot(server, false);
    access.emit("server-event", server.to_string().into());
    access.emit("servers-changed", serde_json::Value::Null);
    true
}

/// A member as a snapshot carried them.
fn member_of(m: commy_net::protocol::SyncMember) -> commy_core::Member {
    let joined_at = chrono::DateTime::from_timestamp_millis(m.joined_ms).unwrap_or_default();
    commy_core::Member { user: m.user, nickname: m.nickname, roles: m.roles, joined_at }
}

/// The nodes a stored invite bootstraps from, or None when it is unreadable.
pub fn invite_nodes(raw: &str) -> Option<Vec<[u8; 32]>> {
    let ticket = commy_net::ServerTicket::decode(raw).ok()?;
    Some(ticket.bootstrap.iter().map(|addr| *addr.node_id.as_bytes()).collect())
}

/// At startup, whether a server an earlier version joined still waits for
/// its first snapshot, which that version did not record: its state is the
/// placeholder made on joining (ours, with our profile only and no
/// messages), and its stored invite names nodes, none of them ours, so we
/// did not create it.
pub fn joined_placeholder(state: &ServerState, me: UserId, invite: Option<&[[u8; 32]]>, our_node: [u8; 32]) -> bool {
    let placeholder = state.server.owner == me
        && state.profiles.keys().all(|id| *id == me)
        && state.server.channels.iter().all(|c| state.messages(c.id).is_empty());
    let joined = invite.is_some_and(|nodes| !nodes.is_empty() && !nodes.contains(&our_node));
    placeholder && joined
}

/// Marks the servers [`joined_placeholder`] finds as awaiting a snapshot.
pub fn mark_joined_placeholders(app: &App, invites: &HashMap<ServerId, String>, our_node: [u8; 32]) {
    let Some(me) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { return };
    let found: Vec<ServerId> = app
        .servers
        .lock()
        .unwrap()
        .values()
        .filter(|state| {
            let nodes = invites.get(&state.server.id).and_then(|raw| invite_nodes(raw));
            joined_placeholder(state, me, nodes.as_deref(), our_node)
        })
        .map(|state| state.server.id)
        .collect();
    for server in found {
        if !app.awaits_snapshot(server) {
            tracing::info!("{server} still waits for its first snapshot");
            app.set_awaiting_snapshot(server, true);
        }
    }
}

/// One session for each server and each of its neighbors.
pub fn every_neighbor(neighbors: Vec<(ServerId, Vec<[u8; 32]>)>) -> Vec<SessionKey> {
    neighbors
        .into_iter()
        .flat_map(|(server, peers)| peers.into_iter().map(move |peer| (server, peer)))
        .collect()
}

/// Every [`PERIODIC_INTERVAL`], each server catches up with one random
/// neighbor (cheap when already in sync), and stale held events expire.
pub async fn run_periodic(handle: tauri::AppHandle) {
    loop {
        tokio::time::sleep(PERIODIC_INTERVAL).await;
        let app = handle.state::<App>();
        app.catchup.lock().unwrap().expire_held(Instant::now());
        let servers: Vec<ServerId> = app.servers.lock().unwrap().keys().copied().collect();
        let neighbors: Vec<(ServerId, Vec<[u8; 32]>)> = {
            let net = app.net.lock().await;
            let Some(node) = net.as_ref() else { continue };
            servers.into_iter().map(|sid| (sid, node.neighbors(sid))).collect()
        };
        let nanos = std::time::UNIX_EPOCH.elapsed().map(|d| d.subsec_nanos() as u64).unwrap_or(0);
        for (i, (sid, list)) in neighbors.into_iter().enumerate() {
            if let Some(peer) = pick(&list, nanos.wrapping_add(i as u64)) {
                spawn_session(handle.clone(), sid, peer, None, true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Server, UserProfile};

    /// A member with a key, signing as themselves.
    struct Member {
        profile: UserProfile,
        identity: Identity,
    }

    impl Member {
        fn new(name: &str) -> Self {
            let identity = Identity::generate();
            let mut profile = UserProfile::new(name);
            profile.identity = Some(identity.public());
            Self { profile, identity }
        }

        fn id(&self) -> UserId {
            self.profile.id
        }

        fn post(&self, state: &ServerState, channel: ChannelId, content: &str) -> Message {
            let mut message = Message::new(channel, self.id(), content);
            message.sign(state.server.id, &self.identity);
            message
        }
    }

    /// A server owned by `owner`, with its first text channel.
    fn server(owner: &Member) -> (ServerState, ChannelId) {
        let server = Server::new("s", owner.id());
        let channel = server.channels[0].id;
        let mut state = ServerState::new(server);
        state.profiles.insert(owner.id(), owner.profile.clone());
        (state, channel)
    }

    fn join(state: &mut ServerState, member: &Member) {
        let server = state.server.id;
        state.apply(ServerEvent::MemberJoin { server, profile: member.profile.clone() }).unwrap();
    }

    /// `member` joins without a key, as an announcement made while their
    /// key was held back.
    fn join_without_key(state: &mut ServerState, member: &Member) {
        let server = state.server.id;
        let profile = UserProfile { identity: None, ..member.profile.clone() };
        state.apply(ServerEvent::MemberJoin { server, profile }).unwrap();
    }

    /// Unread counting as `member` sees it, blocking nobody.
    fn unread_for(member: &Member) -> Unread {
        Unread { me: Some(member.id()), ..Default::default() }
    }

    #[test]
    fn lower_node_starts_unless_we_have_pending_sends() {
        let (low, high) = ([1u8; 32], [2u8; 32]);
        assert_eq!(start_delay(low, high, false), None);
        assert_eq!(start_delay(high, low, false), Some(SECOND_START_DELAY));
        assert_eq!(start_delay(high, low, true), None, "pending sends go at once");
    }

    #[test]
    fn one_session_per_peer_and_recent_ones_count() {
        let mut catchup = Catchup::default();
        let key = (ServerId::new(), [7u8; 32]);
        let t0 = Instant::now();
        assert!(catchup.begin(key));
        assert!(!catchup.begin(key), "already running");
        assert!(!catchup.completed_recently(key, t0));
        catchup.finish(key, t0, false);
        assert!(!catchup.completed_recently(key, t0), "a failed session does not count");
        assert!(catchup.begin(key));
        catchup.finish(key, t0, true);
        assert!(catchup.completed_recently(key, t0 + Duration::from_secs(59)));
        assert!(!catchup.completed_recently(key, t0 + RECENT_SESSION));

        // A session the peer started counts, and does not touch ours.
        let other = (key.0, [8u8; 32]);
        assert!(catchup.begin(other));
        catchup.record_completed(other, t0);
        assert!(catchup.completed_recently(other, t0));
        assert!(!catchup.begin(other), "ours is still running");
    }

    #[test]
    fn held_events_are_capped_and_expire() {
        let mut catchup = Catchup::default();
        let sid = ServerId::new();
        let channel = ChannelId::new();
        let users: Vec<UserId> = (0..HOLD_CAP + 10).map(|_| UserId::new()).collect();
        let typing = |i: usize| ServerEvent::TypingStart { channel, user: users[i] };
        let who = UserId::new();
        let t0 = Instant::now();
        for i in 0..HOLD_CAP + 2 {
            catchup.hold(sid, who, typing(i), t0);
        }
        assert_eq!(catchup.held_count(sid), HOLD_CAP);
        let held = catchup.take_all_held(sid, t0);
        assert_eq!(held.len(), HOLD_CAP);
        assert_eq!(held[0].2, typing(2), "the oldest were dropped");
        assert_eq!(catchup.held_count(sid), 0);

        catchup.hold(sid, who, typing(0), t0);
        catchup.hold(sid, who, typing(1), t0 + Duration::from_secs(60));
        let later = t0 + HOLD_FOR + Duration::from_secs(1);
        assert_eq!(catchup.take_all_held(sid, later), vec![(t0 + Duration::from_secs(60), who, typing(1))]);

        catchup.hold(sid, who, typing(3), t0);
        catchup.expire_held(later);
        assert_eq!(catchup.held_count(sid), 0);

        // Restored events go in front of ones held meanwhile.
        catchup.hold(sid, who, typing(9), t0);
        catchup.restore_held(sid, vec![(t0, who, typing(1)), (t0, who, typing(2))]);
        let order: Vec<ServerEvent> = catchup.take_all_held(sid, t0).into_iter().map(|(_, _, e)| e).collect();
        assert_eq!(order, vec![typing(1), typing(2), typing(9)]);
    }

    #[test]
    fn held_events_are_capped_by_size() {
        let owner = Member::new("owner");
        let stranger = Member::new("stranger");
        let (state, channel) = server(&owner);
        let sid = state.server.id;
        // Large as relayed messages get: full content, many reactions.
        let post = |i: usize, reactors: usize| {
            let mut message = stranger.post(&state, channel, &format!("{i:04} {}", "x".repeat(3000)));
            message.reactions.push(commy_core::Reaction { emoji: ":up".into(), users: (0..reactors).map(|_| UserId::new()).collect() });
            ServerEvent::MessageCreate { message }
        };
        let events: Vec<ServerEvent> = (0..800).map(|i| post(i, 300)).collect();
        let size = |e: &ServerEvent| held_size(e).unwrap();
        // The newest that fit together.
        let mut total = 0;
        let fits = events.iter().rev().take_while(|e| {
            total += size(e);
            total <= HOLD_BYTES
        });
        let fits = fits.count();
        assert!(fits < events.len() && events.len() < HOLD_CAP);
        let mut catchup = Catchup::default();
        let t0 = Instant::now();
        let by = stranger.id();
        for event in &events {
            catchup.hold(sid, by, event.clone(), t0);
        }
        // One too large to send is not held, and pushes nothing out.
        catchup.hold(sid, by, post(800, 5000), t0);
        let held: Vec<ServerEvent> = catchup.take_all_held(sid, t0).into_iter().map(|(_, _, e)| e).collect();
        assert!(held.iter().map(size).sum::<usize>() <= HOLD_BYTES);
        assert_eq!(held.len(), fits);
        assert_eq!(held, events[events.len() - fits..], "the oldest were dropped");

        // Put back in front of newer ones, the oldest go again.
        catchup.hold(sid, by, events[0].clone(), t0);
        catchup.restore_held(sid, held.iter().map(|e| (t0, by, e.clone())).collect());
        let again: Vec<ServerEvent> = catchup.take_all_held(sid, t0).into_iter().map(|(_, _, e)| e).collect();
        assert!(again.iter().map(size).sum::<usize>() <= HOLD_BYTES);
        assert!(!again.contains(&held[0]));
        assert_eq!(again.last(), Some(&events[0]));
    }

    #[test]
    fn changes_are_reported_once() {
        let mut catchup = Catchup::default();
        let sid = ServerId::new();
        let (a, b) = (ChannelId::new(), ChannelId::new());
        assert_eq!(catchup.take_changes(sid), Changes::default());
        catchup.note_applied(sid, [a, b, a]);
        catchup.note_applied(sid, []);
        let changes = catchup.take_changes(sid);
        assert!(changes.changed);
        assert_eq!(changes.fresh, HashMap::from([(a, 2), (b, 1)]));
        assert_eq!(catchup.take_changes(sid), Changes::default());

        let payload = caught_up_payload(sid, &changes.fresh);
        assert_eq!(payload["server"], sid.to_string());
        assert_eq!(payload["channels"][a.to_string()], 2);
        assert_eq!(payload["channels"][b.to_string()], 1);
    }

    #[test]
    fn only_message_events_are_held() {
        let owner = Member::new("owner");
        let (state, channel) = server(&owner);
        let message = owner.post(&state, channel, "hi");
        let edit = message.edit_event(state.server.id, "edited", Utc::now(), &owner.identity);
        let delete = TombstoneProof::sign(state.server.id, channel, message.id, owner.id(), Utc::now(), &owner.identity);
        assert!(holdable(&ServerEvent::MessageCreate { message }));
        assert!(holdable(&edit));
        assert!(holdable(&delete.into_event()));
        assert!(!holdable(&ServerEvent::ServerRename { server: state.server.id, name: "x".into() }));
    }

    #[test]
    fn held_messages_apply_once_the_author_key_arrives() {
        let owner = Member::new("owner");
        let newcomer = Member::new("newcomer");
        let (mut state, channel) = server(&owner);
        let sid = state.server.id;
        let first = newcomer.post(&state, channel, "first");
        let second = newcomer.post(&state, channel, "second");
        let edit = first.edit_event(sid, "first, edited", Utc::now(), &newcomer.identity);
        let t0 = Instant::now();

        // Their messages beat their join: the key is unknown, so they are held.
        let mut catchup = Catchup::default();
        for event in [
            ServerEvent::MessageCreate { message: first.clone() },
            ServerEvent::MessageCreate { message: second.clone() },
        ] {
            assert_eq!(state.apply(event.clone()), Err(StoreError::UnknownAuthorKey(newcomer.id())));
            catchup.hold(sid, newcomer.id(), event, t0);
        }
        let retried = retry_held(&mut state, &unread_for(&owner), catchup.take_all_held(sid, t0));
        assert!(!retried.changed && retried.fresh.is_empty());
        assert_eq!(retried.still.len(), 2, "still no key");
        catchup.restore_held(sid, retried.still);

        join(&mut state, &newcomer);
        catchup.hold(sid, newcomer.id(), edit, t0);
        let retried = retry_held(&mut state, &unread_for(&owner), catchup.take_all_held(sid, t0));
        assert!(retried.changed && retried.still.is_empty());
        assert_eq!(retried.fresh, vec![channel, channel], "the edit is not a new message");
        assert_eq!(state.message(first.id).unwrap().content, "first, edited");
        assert!(state.has_message(second.id));

        // Our own messages coming back are not unread.
        let mine = owner.post(&state, channel, "mine");
        let retried = retry_held(&mut state, &unread_for(&owner), vec![(t0, owner.id(), ServerEvent::MessageCreate { message: mine })]);
        assert!(retried.changed && retried.fresh.is_empty());
    }

    #[test]
    fn apply_page_counts_new_messages_from_others() {
        let owner = Member::new("owner");
        let friend = Member::new("friend");
        let stranger = Member::new("stranger");
        let (mut state, channel) = server(&owner);
        join(&mut state, &friend);
        let known = friend.post(&state, channel, "known");
        state.apply(ServerEvent::MessageCreate { message: known.clone() }).unwrap();

        let new = friend.post(&state, channel, "new");
        let mine = owner.post(&state, channel, "mine");
        let theirs = stranger.post(&state, channel, "who?");
        let deleted = friend.post(&state, channel, "deleted");
        let proof = TombstoneProof::sign(state.server.id, channel, deleted.id, friend.id(), Utc::now(), &friend.identity);
        let (report, held, fresh) = apply_page(
            &mut state,
            &unread_for(&owner),
            vec![known.clone(), new.clone(), mine.clone(), theirs.clone(), deleted.clone()],
            vec![proof],
        );
        assert!(report.applied.contains(&new.id) && report.applied.contains(&mine.id));
        assert!(!report.applied.contains(&known.id));
        assert_eq!(fresh, vec![channel], "only the friend's new message is unread");
        assert_eq!(report.held, vec![theirs.id]);
        assert_eq!(held, vec![(stranger.id(), ServerEvent::MessageCreate { message: theirs })]);
        assert!(!state.has_message(deleted.id));
    }

    #[test]
    fn a_create_kept_out_by_an_earlier_delete_is_no_new_message() {
        let owner = Member::new("owner");
        let friend = Member::new("friend");
        let (mut state, channel) = server(&owner);
        join(&mut state, &friend);
        let sid = state.server.id;
        let message = friend.post(&state, channel, "soon gone");
        let delete = TombstoneProof::sign(sid, channel, message.id, friend.id(), Utc::now(), &friend.identity);

        // The delete overtook the create.
        assert_eq!(apply_live(&mut state, delete.into_event()), (Ok(true), None, None));
        let create = ServerEvent::MessageCreate { message: message.clone() };
        assert_eq!(apply_live(&mut state, create.clone()), (Ok(true), None, None), "only its proof changed");
        assert!(!state.has_message(message.id));
        assert_eq!(apply_live(&mut state, create), (Ok(false), None, None));

        // A new message is news once; a newer copy of it is not.
        let fresh = friend.post(&state, channel, "hello");
        let create = ServerEvent::MessageCreate { message: fresh.clone() };
        assert_eq!(apply_live(&mut state, create.clone()), (Ok(true), Some(fresh.clone()), None));
        assert_eq!(apply_live(&mut state, create), (Ok(false), None, None));
        let mut edited = fresh.clone();
        edited.content = "hello again".into();
        edited.edited_at = Some(Utc::now() + chrono::Duration::seconds(1));
        edited.sign(sid, &friend.identity);
        assert_eq!(apply_live(&mut state, ServerEvent::MessageCreate { message: edited }), (Ok(true), None, None));
        // Other events bring no message.
        let rename = ServerEvent::ServerRename { server: sid, name: "renamed".into() };
        assert_eq!(apply_live(&mut state, rename), (Ok(true), None, None));
    }

    #[test]
    fn a_settled_key_catches_up_with_every_neighbor() {
        let (a, b) = (ServerId::new(), ServerId::new());
        let (one, two) = ([1u8; 32], [2u8; 32]);
        let sessions = every_neighbor(vec![(a, vec![one, two]), (b, vec![]), (b, vec![two])]);
        assert_eq!(sessions, vec![(a, one), (a, two), (b, two)]);
    }

    #[test]
    fn neighbor_pick_wraps_around() {
        assert_eq!(pick(&[], 5), None);
        let list = [[1u8; 32], [2u8; 32], [3u8; 32]];
        assert_eq!(pick(&list, 0), Some(list[0]));
        assert_eq!(pick(&list, 4), Some(list[1]));
    }

    /// The store over a real `App`, as the node sees it.
    fn app_with(owner: &Member, state: ServerState) -> Arc<App> {
        let app = Arc::new(App::default());
        *app.profile.lock().unwrap() = Some(owner.profile.clone());
        *app.identity.lock().unwrap() = Some(owner.identity.clone());
        app.servers.lock().unwrap().insert(state.server.id, state);
        app
    }

    #[test]
    fn store_serves_and_applies_over_app_state() {
        let owner = Member::new("owner");
        let friend = Member::new("friend");
        let (mut state, channel) = server(&owner);
        join(&mut state, &friend);
        let sid = state.server.id;
        let ours = owner.post(&state, channel, "ours");
        state.apply(ServerEvent::MessageCreate { message: ours.clone() }).unwrap();
        let app = app_with(&owner, state);
        let handle = TestHandle::new(app.clone());
        let store = AppStore(handle.clone());

        let (me, identity) = store.local_identity(sid).unwrap();
        assert_eq!((me, identity.public()), (owner.id(), owner.identity.public()));
        assert!(store.local_identity(ServerId::new()).is_none(), "not in that server");

        assert_eq!(store.verify_peer(sid, friend.id(), &friend.identity.public()), PeerCheck::Allowed);
        assert_eq!(store.verify_peer(sid, friend.id(), &owner.identity.public()), PeerCheck::Denied);
        assert_eq!(store.verify_peer(sid, UserId::new(), &friend.identity.public()), PeerCheck::Unknown);
        assert_eq!(store.verify_peer(ServerId::new(), friend.id(), &friend.identity.public()), PeerCheck::Denied);

        let digest = store.digests(sid);
        assert_eq!(digest.len(), 1);
        let listed = store.items(sid, &[digest[0].key]);
        assert_eq!(listed[0].1.len(), 1);
        let (messages, proofs) = store.get(sid, &[ours.id, MessageId::new()]);
        assert_eq!((messages, proofs.len()), (vec![ours.clone()], 0));
        assert!(store.digests(ServerId::new()).is_empty());

        let theirs = friend.post(&app.servers.lock().unwrap()[&sid], channel, "theirs");
        let report = store.apply(sid, vec![theirs.clone()], vec![]);
        assert_eq!(report.applied, vec![theirs.id]);
        assert!(app.servers.lock().unwrap()[&sid].has_message(theirs.id));
        let caught_up = caught_up_payload(sid, &HashMap::from([(channel, 1)]));
        assert_eq!(
            handle.take_events(),
            vec![("server-event".into(), sid.to_string().into()), ("messages-caught-up".into(), caught_up)]
        );

        // A message from someone whose join has not arrived waits in the queue.
        let late = Member::new("late");
        let early = late.post(&app.servers.lock().unwrap()[&sid], channel, "early");
        let report = store.apply(sid, vec![early.clone()], vec![]);
        assert_eq!((report.applied.len(), report.held), (0, vec![early.id]));
        assert_eq!(app.catchup.lock().unwrap().held_count(sid), 1);
        assert_eq!(app.catchup.lock().unwrap().take_changes(sid), Changes::default());
        assert!(handle.take_events().is_empty(), "nothing changed");

        join(app.servers.lock().unwrap().get_mut(&sid).unwrap(), &late);
        let held = app.catchup.lock().unwrap().take_all_held(sid, Instant::now());
        let retried = retry_held(app.servers.lock().unwrap().get_mut(&sid).unwrap(), &unread_for(&owner), held);
        assert_eq!(retried.fresh, vec![channel]);
        assert!(app.servers.lock().unwrap()[&sid].has_message(early.id));
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos();
        let dir = std::env::temp_dir().join("commy-catchup-tests").join(format!("{tag}-{nanos:x}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn saved_has(dir: &std::path::Path, sid: ServerId, id: MessageId) -> bool {
        crate::persist::load_servers(dir).iter().any(|s| s.server.id == sid && s.has_message(id))
    }

    #[test]
    fn applied_pages_are_shown_at_once_and_saved_together() {
        let owner = Member::new("owner");
        let friend = Member::new("friend");
        let (mut state, channel) = server(&owner);
        join(&mut state, &friend);
        let sid = state.server.id;
        let app = app_with(&owner, state);
        let dir = temp_dir("pages");
        app.data_dir.set(dir.clone()).unwrap();
        let handle = TestHandle::new(app.clone());
        let store = AppStore(handle.clone());

        // Pages applied in a session that never finishes, as when a peer's
        // session with us fails: shown at once, not saved per page.
        let first = friend.post(&app.servers.lock().unwrap()[&sid], channel, "first page");
        let second = friend.post(&app.servers.lock().unwrap()[&sid], channel, "second page");
        store.apply(sid, vec![first.clone()], vec![]);
        let names: Vec<String> = handle.take_events().into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["server-event", "messages-caught-up"]);
        store.apply(sid, vec![second.clone()], vec![]);
        assert_eq!(handle.take_events().len(), 2);
        assert!(!saved_has(&dir, sid, first.id), "not saved per page");
        assert_eq!(handle.saves_scheduled.load(std::sync::atomic::Ordering::SeqCst), 1, "one save for both");

        // The delayed save writes both, not before its delay.
        let started = Instant::now();
        while !saved_has(&dir, sid, second.id) && started.elapsed() < SAVE_DELAY * 5 {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(started.elapsed() >= SAVE_DELAY - Duration::from_millis(500));
        assert!(saved_has(&dir, sid, first.id) && saved_has(&dir, sid, second.id));
        assert_eq!(app.catchup.lock().unwrap().take_changes(sid), Changes::default(), "nothing left over");

        // A page that changes nothing says nothing and saves nothing.
        store.apply(sid, vec![second], vec![]);
        assert!(handle.take_events().is_empty());
        assert_eq!(handle.saves_scheduled.load(std::sync::atomic::Ordering::SeqCst), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_session_that_ends_saves_its_server_at_once() {
        let owner = Member::new("owner");
        let friend = Member::new("friend");
        let (mut state, channel) = server(&owner);
        join(&mut state, &friend);
        let sid = state.server.id;
        let app = app_with(&owner, state);
        let dir = temp_dir("session-end");
        app.data_dir.set(dir.clone()).unwrap();
        let handle = TestHandle::new(app.clone());
        let page = friend.post(&app.servers.lock().unwrap()[&sid], channel, "page");
        AppStore(handle.clone()).apply(sid, vec![page.clone()], vec![]);
        assert!(!saved_has(&dir, sid, page.id));
        after_session(&handle, sid, Some(&SessionReport::default()));
        assert!(saved_has(&dir, sid, page.id));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_server_left_meanwhile_takes_nothing() {
        let owner = Member::new("owner");
        let (state, channel) = server(&owner);
        let message = owner.post(&state, channel, "for a server we left");
        let proof = TombstoneProof::sign(state.server.id, channel, MessageId::new(), owner.id(), Utc::now(), &owner.identity);
        let app = app_with(&owner, state);
        let gone = ServerId::new();
        let report = AppStore(TestHandle::new(app)).apply(gone, vec![message.clone(), message.clone()], vec![proof.clone()]);
        assert_eq!(report, ApplyReport { rejected: vec![message.id, proof.message], ..Default::default() });
    }

    #[test]
    fn profiles_from_a_session_let_held_items_in() {
        let owner = Member::new("owner");
        let (state, channel) = server(&owner);
        let sid = state.server.id;
        let app = app_with(&owner, state);
        let handle = TestHandle::new(app.clone());
        let store = AppStore(handle.clone());

        // Relayed history from a member whose key we lack waits for it.
        let absent = Member::new("absent");
        join_without_key(app.servers.lock().unwrap().get_mut(&sid).unwrap(), &absent);
        let message = absent.post(&app.servers.lock().unwrap()[&sid], channel, "while you were away");
        store.apply(sid, vec![message.clone()], vec![]);
        assert_eq!(app.catchup.lock().unwrap().held_count(sid), 1);

        // Their profile comes along in a later session.
        store.apply_profiles(sid, vec![absent.profile.clone()]);
        let state = app.servers.lock().unwrap()[&sid].clone();
        assert!(state.has_message(message.id));
        assert_eq!(state.identity_of(absent.id()), Some(absent.identity.public()));
        assert_eq!(app.catchup.lock().unwrap().held_count(sid), 0);
        let caught_up = caught_up_payload(sid, &HashMap::from([(channel, 1)]));
        assert!(handle.take_events().contains(&("messages-caught-up".into(), caught_up)));

        // Only profiles with a key are taken, and only members' passed on.
        let unkeyed = UserProfile::new("unkeyed");
        store.apply_profiles(sid, vec![unkeyed.clone()]);
        assert!(!app.servers.lock().unwrap()[&sid].profiles.contains_key(&unkeyed.id));
        let asked = [absent.id(), unkeyed.id, owner.id(), owner.id()];
        let absent_here = app.servers.lock().unwrap()[&sid].profiles[&absent.id()].clone();
        assert_eq!(store.profiles(sid, &asked), vec![absent_here, owner.profile.clone()]);
    }

    #[test]
    fn only_what_a_key_just_pinned_signed_is_released() {
        let owner = Member::new("owner");
        let friend = Member::new("friend");
        let (a, b) = (Member::new("a"), Member::new("b"));
        let (mut state, channel) = server(&owner);
        join(&mut state, &friend);
        for member in [&a, &b] {
            join_without_key(&mut state, member);
        }
        let sid = state.server.id;
        let app = app_with(&owner, state);
        let handle = TestHandle::new(app.clone());
        let store = AppStore(handle.clone());
        let post = |m: &Member, text: &str| m.post(&app.servers.lock().unwrap()[&sid], channel, text);
        let (by_a, by_b) = (post(&a, "from a"), post(&b, "from b"));
        store.apply(sid, vec![by_a.clone(), by_b.clone()], vec![]);
        assert_eq!(app.catchup.lock().unwrap().held_count(sid), 2);
        // b's key is known by now, though nothing named b since.
        app.servers.lock().unwrap().get_mut(&sid).unwrap().profiles.get_mut(&b.id()).unwrap().identity =
            Some(b.identity.public());
        handle.take_events();
        // What the net loop does with a live event.
        let live = |event: ServerEvent| {
            let pinned = apply_live(app.servers.lock().unwrap().get_mut(&sid).unwrap(), event).2;
            release_held(&handle, sid, pinned.as_slice());
            pinned
        };
        let has = |m: &Message| app.servers.lock().unwrap()[&sid].has_message(m.id);

        // Joins and profiles that pin no key release nothing.
        let keyless = UserProfile { identity: None, ..a.profile.clone() };
        assert_eq!(live(ServerEvent::MemberJoin { server: sid, profile: keyless }), None);
        assert_eq!(live(ServerEvent::ProfileUpdate { profile: friend.profile.clone() }), None);
        assert!(!has(&by_a) && !has(&by_b));
        assert_eq!(app.catchup.lock().unwrap().held_count(sid), 2);
        assert!(handle.take_events().is_empty());

        // a's key arrives: only a's message goes in.
        assert_eq!(live(ServerEvent::ProfileUpdate { profile: a.profile.clone() }), Some(a.id()));
        assert!(has(&by_a) && !has(&by_b));
        assert_eq!(app.catchup.lock().unwrap().held_count(sid), 1);
        let caught_up = caught_up_payload(sid, &HashMap::from([(channel, 1)]));
        assert!(handle.take_events().contains(&("messages-caught-up".into(), caught_up)));

        // A Profiles frame that pins nothing new releases nothing either.
        store.apply_profiles(sid, vec![a.profile.clone(), b.profile.clone(), friend.profile.clone()]);
        assert!(!has(&by_b));
        assert_eq!(app.catchup.lock().unwrap().held_count(sid), 1);
    }

    #[test]
    fn the_same_held_event_is_kept_once() {
        let owner = Member::new("owner");
        let (state, channel) = server(&owner);
        let sid = state.server.id;
        let message = owner.post(&state, channel, "hi");
        let t0 = Instant::now();
        let mut catchup = Catchup::default();
        // Every session pushes the same message again.
        let by = owner.id();
        for _ in 0..3 {
            catchup.hold(sid, by, ServerEvent::MessageCreate { message: message.clone() }, t0);
        }
        assert_eq!(catchup.held_count(sid), 1);

        // Of two edits of one message, the later one stays.
        let first = message.edit_event(sid, "one", Utc::now(), &owner.identity);
        let second = message.edit_event(sid, "two", Utc::now() + chrono::Duration::seconds(1), &owner.identity);
        catchup.hold(sid, by, second.clone(), t0);
        catchup.hold(sid, by, first.clone(), t0);
        assert_eq!(catchup.held_count(sid), 2);
        let held: Vec<ServerEvent> = catchup.take_all_held(sid, t0).into_iter().map(|(_, _, e)| e).collect();
        assert_eq!(held, vec![ServerEvent::MessageCreate { message: message.clone() }, second.clone()]);
        catchup.hold(sid, by, first, t0);
        catchup.hold(sid, by, second.clone(), t0);
        assert_eq!(catchup.take_all_held(sid, t0), vec![(t0, by, second)]);

        // Each deleter's proof is its own.
        let other = Member::new("other");
        let by_owner = TombstoneProof::sign(sid, channel, message.id, owner.id(), Utc::now(), &owner.identity);
        let by_other = TombstoneProof::sign(sid, channel, message.id, other.id(), Utc::now(), &other.identity);
        for proof in [&by_owner, &by_other, &by_owner] {
            catchup.hold(sid, proof.by, proof.clone().into_event(), t0);
        }
        assert_eq!(catchup.held_count(sid), 2);

        // Restoring does not bring back one held again meanwhile.
        let again = catchup.take_all_held(sid, t0);
        catchup.hold(sid, by, by_owner.clone().into_event(), t0);
        catchup.restore_held(sid, again);
        assert_eq!(catchup.held_count(sid), 2);
    }

    #[test]
    fn blocked_authors_are_never_unread() {
        let owner = Member::new("owner");
        let pest = Member::new("pest");
        let (mut state, channel) = server(&owner);
        join(&mut state, &pest);
        let unread = Unread { me: Some(owner.id()), blocked: HashSet::from([pest.id()]), silent: false };

        let page = pest.post(&state, channel, "caught up");
        let (report, _, fresh) = apply_page(&mut state, &unread, vec![page.clone()], vec![]);
        assert_eq!(report.applied, vec![page.id], "still applied");
        assert!(fresh.is_empty());

        let released = pest.post(&state, channel, "released");
        let retried = retry_held(&mut state, &unread, vec![(Instant::now(), pest.id(), ServerEvent::MessageCreate { message: released.clone() })]);
        assert!(retried.changed && state.has_message(released.id));
        assert!(retried.fresh.is_empty());

        // Through the store, the blocked list comes from the safety settings.
        let sid = state.server.id;
        let app = app_with(&owner, state);
        app.safety.lock().unwrap().blocked.push(pest.id());
        let handle = TestHandle::new(app.clone());
        let late = pest.post(&app.servers.lock().unwrap()[&sid], channel, "late");
        AppStore(handle.clone()).apply(sid, vec![late], vec![]);
        let names: Vec<String> = handle.take_events().into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["server-event"], "redrawn, but nothing unread");
    }

    #[test]
    fn history_before_we_joined_is_not_unread() {
        let owner = Member::new("owner");
        let friend = Member::new("friend");
        let newcomer = Member::new("newcomer");
        let (mut state, channel) = server(&owner);
        join(&mut state, &friend);
        let old = friend.post(&state, channel, "before you came");
        let mut older = old.clone();
        older.created_at = Utc::now() - chrono::Duration::days(3);
        older.sign(state.server.id, &friend.identity);
        join(&mut state, &newcomer);
        let new = friend.post(&state, channel, "welcome");
        let (report, _, fresh) = apply_page(&mut state, &unread_for(&newcomer), vec![older.clone(), new.clone()], vec![]);
        assert_eq!(report.applied.len(), 2);
        assert_eq!(fresh, vec![channel], "only the message after our join");
    }

    #[test]
    fn nothing_is_unread_before_the_first_snapshot() {
        let owner = Member::new("owner");
        let friend = Member::new("friend");
        let (mut state, channel) = server(&owner);
        join(&mut state, &friend);
        let sid = state.server.id;
        let app = app_with(&owner, state);
        app.set_awaiting_snapshot(sid, true);
        let handle = TestHandle::new(app.clone());
        let store = AppStore(handle.clone());

        let history = friend.post(&app.servers.lock().unwrap()[&sid], channel, "history");
        store.apply(sid, vec![history.clone()], vec![]);
        assert!(app.servers.lock().unwrap()[&sid].has_message(history.id));
        let names: Vec<String> = handle.take_events().into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["server-event"]);

        // Released from the hold queue: not counted either.
        let stranger = Member::new("stranger");
        join_without_key(app.servers.lock().unwrap().get_mut(&sid).unwrap(), &stranger);
        let held = stranger.post(&app.servers.lock().unwrap()[&sid], channel, "held");
        store.apply(sid, vec![held.clone()], vec![]);
        store.apply_profiles(sid, vec![stranger.profile.clone()]);
        assert!(app.servers.lock().unwrap()[&sid].has_message(held.id));
        assert!(handle.take_events().iter().all(|(name, _)| name != "messages-caught-up"));

        // Once the snapshot arrived, new messages count again.
        app.set_awaiting_snapshot(sid, false);
        let live = friend.post(&app.servers.lock().unwrap()[&sid], channel, "live");
        store.apply(sid, vec![live], vec![]);
        assert!(handle.take_events().iter().any(|(name, _)| name == "messages-caught-up"));
    }

    #[test]
    fn only_what_the_peer_confirmed_stops_pending() {
        let owner = Member::new("owner");
        let (mut state, channel) = server(&owner);
        let sid = state.server.id;
        let taken = owner.post(&state, channel, "taken");
        let refused = owner.post(&state, channel, "refused");
        for message in [&taken, &refused] {
            state.apply(ServerEvent::MessageCreate { message: message.clone() }).unwrap();
        }
        let app = app_with(&owner, state);
        {
            let mut outbox = app.outbox.lock().unwrap();
            for message in [&taken, &refused] {
                outbox.note_local(sid, &ServerEvent::MessageCreate { message: message.clone() }, false);
            }
        }
        let handle = TestHandle::new(app.clone());
        // Both were pushed, the peer only took one.
        let report = SessionReport { delivered: vec![taken.id], ..Default::default() };
        after_session(&handle, sid, Some(&report));
        assert_eq!(app.outbox.lock().unwrap().pending_ids(sid), HashSet::from([refused.id]));
        assert_eq!(handle.take_events(), vec![("server-event".into(), sid.to_string().into())]);
    }

    /// A snapshot as a member of `state` would answer one.
    fn snapshot_of(state: &ServerState) -> WirePayload {
        commy_net::protocol::build_sync_response(state, 50)
    }

    #[test]
    fn only_a_server_waiting_for_it_takes_a_snapshot() {
        let owner = Member::new("owner");
        let me = Member::new("me");
        let (mut real, channel) = server(&owner);
        join(&mut real, &me);
        let sid = real.server.id;
        let hello = owner.post(&real, channel, "hello");
        real.apply(ServerEvent::MessageCreate { message: hello.clone() }).unwrap();

        // Our provisional state from a ticket.
        let mut provisional = commy_core::Server::new("s", me.id());
        provisional.id = sid;
        let mut placeholder = ServerState::new(provisional);
        placeholder.profiles.insert(me.id(), me.profile.clone());
        let app = app_with(&me, placeholder);
        let handle = TestHandle::new(app.clone());

        // Not marked as waiting: nothing is replaced.
        assert!(!apply_snapshot(&handle, sid, snapshot_of(&real)));
        assert!(!app.servers.lock().unwrap()[&sid].has_message(hello.id));

        app.set_awaiting_snapshot(sid, true);
        // A snapshot of another server on this topic is refused.
        let (other, _) = server(&owner);
        assert!(!apply_snapshot(&handle, sid, snapshot_of(&other)));
        assert!(app.awaits_snapshot(sid));
        assert!(handle.take_events().is_empty());

        // The owner's live message beat the snapshot that brings their key.
        let early = owner.post(&real, channel, "before the snapshot");
        hold(&app, sid, owner.id(), ServerEvent::MessageCreate { message: early.clone() });
        assert!(apply_snapshot(&handle, sid, snapshot_of(&real)));
        let state = app.servers.lock().unwrap()[&sid].clone();
        assert_eq!(state.server.owner, owner.id());
        let ms = |m: &commy_core::Member| (m.user, m.roles.clone(), m.joined_at.timestamp_millis());
        assert_eq!(state.server.members.iter().map(ms).collect::<Vec<_>>(), real.server.members.iter().map(ms).collect::<Vec<_>>());
        assert!(state.has_message(hello.id) && state.has_message(early.id));
        assert_eq!(app.catchup.lock().unwrap().held_count(sid), 0);
        assert_eq!(state.identity_of(me.id()), Some(me.identity.public()));
        assert!(!app.awaits_snapshot(sid), "one snapshot only");
        let names: Vec<String> = handle.take_events().into_iter().map(|(name, _)| name).collect();
        assert!(names.contains(&"servers-changed".to_string()));

        // From now on the state is established: a forged snapshot that
        // claims another key for the owner changes nothing.
        let mut forged = real.clone();
        let impostor = Identity::generate();
        forged.profiles.get_mut(&owner.id()).unwrap().identity = Some(impostor.public());
        assert!(!apply_snapshot(&handle, sid, snapshot_of(&forged)));
        assert_eq!(app.servers.lock().unwrap()[&sid].identity_of(owner.id()), Some(owner.identity.public()));

        // A server left while its snapshot was on the way stays gone.
        let (gone, _) = server(&owner);
        app.set_awaiting_snapshot(gone.server.id, true);
        assert!(!apply_snapshot(&handle, gone.server.id, snapshot_of(&gone)));
        assert!(!app.servers.lock().unwrap().contains_key(&gone.server.id));
    }

    #[tokio::test]
    async fn a_snapshot_from_before_our_join_still_makes_us_a_member() {
        let owner = Member::new("owner");
        let me = Member::new("me");
        let (mut real, channel) = server(&owner);
        let sid = real.server.id;
        // Answered before our join reached the owner: we are not in it.
        let snapshot = snapshot_of(&real);
        let mut provisional = commy_core::Server::new("s", me.id());
        provisional.id = sid;
        let mut placeholder = ServerState::new(provisional);
        placeholder.profiles.insert(me.id(), me.profile.clone());
        let app = app_with(&me, placeholder);
        app.set_awaiting_snapshot(sid, true);
        let handle = TestHandle::new(app.clone());
        assert!(apply_snapshot(&handle, sid, snapshot));
        assert!(!app.awaits_snapshot(sid));
        assert!(app.servers.lock().unwrap()[&sid].server.is_member(me.id()));
        let store = AppStore(handle);
        let (user, identity) = store.local_identity(sid).expect("a member, so catch-up may run");
        assert_eq!((user, identity.public()), (me.id(), me.identity.public()));

        // Our join arrives there, then a session brings what came since.
        join(&mut real, &me);
        let later = owner.post(&real, channel, "since the snapshot");
        real.apply(ServerEvent::MessageCreate { message: later.clone() }).unwrap();
        let theirs = AppStore(TestHandle::new(app_with(&owner, real)));
        let (ours_io, theirs_io) = tokio::io::duplex(1 << 20);
        let (our_node, their_node) = ([1u8; 32], [2u8; 32]);
        let (initiated, responded) = tokio::join!(
            commy_net::catchup::run_initiator(ours_io, &store, &identity, sid, user, our_node, their_node),
            commy_net::catchup::run_responder(theirs_io, &theirs, their_node, our_node),
        );
        assert_eq!(initiated.unwrap().received, vec![later.id]);
        assert_eq!(responded.unwrap().0, sid);
        assert!(app.servers.lock().unwrap()[&sid].has_message(later.id));
    }

    /// The provisional state of `me` joining server `sid` from a ticket.
    fn placeholder(me: &Member, sid: ServerId) -> ServerState {
        let mut provisional = commy_core::Server::new("s", me.id());
        provisional.id = sid;
        let mut state = ServerState::new(provisional);
        state.profiles.insert(me.id(), me.profile.clone());
        state
    }

    #[test]
    fn a_placeholder_joined_long_ago_still_takes_the_real_snapshot() {
        let owner = Member::new("owner");
        let me = Member::new("me");
        let (mut real, _) = server(&owner);
        join(&mut real, &me);
        let sid = real.server.id;
        // Joined a month ago; the owner's join pinned their key since.
        let mut ours = placeholder(&me, sid);
        ours.server.members.iter_mut().for_each(|m| m.joined_at = Utc::now() - chrono::Duration::days(30));
        join(&mut ours, &owner);
        let dir = temp_dir("awaiting");
        for how in ["session", "restart", "startup"] {
            let app = app_with(&me, ours.clone());
            let handle = TestHandle::new(app.clone());
            app.set_awaiting_snapshot(sid, true);
            match how {
                "session" => after_session(&handle, sid, Some(&SessionReport::default())),
                "restart" => {
                    // Saved by an earlier build with a window long closed.
                    let timed = HashMap::from([(sid, 0i64)]);
                    crate::persist::write_json(&dir.join("awaiting-snapshot.json"), &timed).unwrap();
                    *app.awaiting_snapshot.lock().unwrap() = crate::persist::load_awaiting(&dir);
                }
                _ => mark_joined_placeholders(&app, &HashMap::new(), [9; 32]),
            }
            assert!(app.awaits_snapshot(sid), "{how}");
            assert!(apply_snapshot(&handle, sid, snapshot_of(&real)), "{how}");
            let state = app.servers.lock().unwrap()[&sid].clone();
            assert_eq!(state.server.owner, owner.id(), "{how}");
            assert!(state.server.is_member(me.id()), "{how}");
            assert!(!app.awaits_snapshot(sid), "{how}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_key_pinned_while_waiting_gives_way_to_the_snapshot() {
        let owner = Member::new("owner");
        let friend = Member::new("friend");
        let me = Member::new("me");
        let (mut real, channel) = server(&owner);
        join(&mut real, &friend);
        join(&mut real, &me);
        let sid = real.server.id;
        let hello = owner.post(&real, channel, "hello");
        real.apply(ServerEvent::MessageCreate { message: hello.clone() }).unwrap();
        // A relayed profile planted a key for the owner here before the
        // snapshot, so the owner's own join is refused; the friend's
        // join pinned their real key.
        let mut ours = placeholder(&me, sid);
        let planted = UserProfile { identity: Some(Identity::generate().public()), ..owner.profile.clone() };
        ours.apply(ServerEvent::MemberJoin { server: sid, profile: planted }).unwrap();
        let owner_join = ServerEvent::MemberJoin { server: sid, profile: owner.profile.clone() };
        assert_eq!(ours.apply(owner_join), Err(StoreError::IdentityMismatch));
        join(&mut ours, &friend);
        let app = app_with(&me, ours);
        app.set_awaiting_snapshot(sid, true);
        let handle = TestHandle::new(app.clone());

        // The snapshot's keys win, except for us; one it leaves out stays.
        let mut sent = real.clone();
        sent.profiles.get_mut(&friend.id()).unwrap().identity = None;
        sent.profiles.get_mut(&me.id()).unwrap().identity = Some(Identity::generate().public());
        assert!(apply_snapshot(&handle, sid, snapshot_of(&sent)));
        let state = app.servers.lock().unwrap()[&sid].clone();
        assert_eq!(state.server.owner, owner.id());
        assert_eq!(state.identity_of(owner.id()), Some(owner.identity.public()));
        assert_eq!(state.identity_of(friend.id()), Some(friend.identity.public()));
        assert_eq!(state.identity_of(me.id()), Some(me.identity.public()));
        assert!(state.has_message(hello.id));

        // Once one is taken, a snapshot that changes a key is refused.
        let mut forged = real.clone();
        forged.profiles.get_mut(&owner.id()).unwrap().identity = Some(Identity::generate().public());
        assert!(!apply_snapshot(&handle, sid, snapshot_of(&forged)));
        assert_eq!(app.servers.lock().unwrap()[&sid].identity_of(owner.id()), Some(owner.identity.public()));
    }

    #[test]
    fn what_came_while_waiting_outlives_the_snapshot() {
        let owner = Member::new("owner");
        let me = Member::new("me");
        let (mut real, channel) = server(&owner);
        join(&mut real, &me);
        let sid = real.server.id;
        let edited = owner.post(&real, channel, "first");
        let deleted = owner.post(&real, channel, "gone");
        for message in [&edited, &deleted] {
            real.apply(ServerEvent::MessageCreate { message: message.clone() }).unwrap();
        }
        // While we wait the channel is announced, the owner joins, edits
        // one message and deletes another, and we post one nobody has yet.
        let mut ours = placeholder(&me, sid);
        let announced = real.server.channel(channel).unwrap().clone();
        ours.apply(ServerEvent::ChannelCreate { server: sid, channel: announced }).unwrap();
        join(&mut ours, &owner);
        ours.apply(ServerEvent::MessageCreate { message: edited.clone() }).unwrap();
        ours.apply(edited.edit_event(sid, "second", Utc::now(), &owner.identity)).unwrap();
        let delete = TombstoneProof::sign(sid, channel, deleted.id, owner.id(), Utc::now(), &owner.identity);
        ours.apply(delete.into_event()).unwrap();
        let mine = me.post(&ours, channel, "while waiting");
        let create = ServerEvent::MessageCreate { message: mine.clone() };
        ours.apply(create.clone()).unwrap();
        let app = app_with(&me, ours);
        app.outbox.lock().unwrap().note_local(sid, &create, false);
        app.set_awaiting_snapshot(sid, true);
        let handle = TestHandle::new(app.clone());

        assert!(apply_snapshot(&handle, sid, snapshot_of(&real)));
        // What runs after any session drops pending ids we no longer hold.
        after_session(&handle, sid, None);
        let state = app.servers.lock().unwrap()[&sid].clone();
        assert_eq!(state.server.owner, owner.id());
        assert!(state.has_message(mine.id));
        assert!(app.outbox.lock().unwrap().is_pending(sid, mine.id), "still pending");
        assert_eq!(state.message(edited.id).map(|m| m.content.as_str()), Some("second"));
        assert!(!state.has_message(deleted.id));
        assert!(state.tombstone(deleted.id).is_some());
    }

    #[test]
    fn a_snapshot_answered_while_joining_is_asked_for_again() {
        let owner = Member::new("owner");
        let me = Member::new("me");
        let (mut real, _) = server(&owner);
        join(&mut real, &me);
        let sid = real.server.id;
        let app = app_with(&me, placeholder(&me, sid));
        let handle = TestHandle::new(app.clone());
        let theirs = app_with(&owner, real);
        // What we send reaches the owner, whose snapshot reaches us.
        let deliver = |sent: Vec<WirePayload>| {
            for _ in sent.iter().filter(|p| matches!(p, WirePayload::SyncRequest)) {
                for reply in sync_replies(&theirs, sid) {
                    if matches!(reply, WirePayload::SyncResponse { .. }) {
                        apply_snapshot(&handle, sid, reply);
                    }
                }
            }
        };
        // Joining asks first, and that answer beats the wait being marked.
        deliver(vec![WirePayload::SyncRequest]);
        assert_eq!(app.servers.lock().unwrap()[&sid].server.owner, me.id(), "dropped");
        deliver(await_snapshot(&app, sid, me.profile.clone()));
        assert!(!app.awaits_snapshot(sid), "taken");
        assert_eq!(app.servers.lock().unwrap()[&sid].server.owner, owner.id());
    }

    #[test]
    fn a_placeholder_is_never_handed_out() {
        let owner = Member::new("owner");
        let me = Member::new("me");
        let (mut real, _) = server(&owner);
        join(&mut real, &me);
        let sid = real.server.id;
        let app = app_with(&me, placeholder(&me, sid));
        let handle = TestHandle::new(app.clone());
        app.set_awaiting_snapshot(sid, true);
        assert!(snapshot_for(&app, sid).is_none(), "still waiting for ours");
        // Nor once a session completed, nor after a restart.
        after_session(&handle, sid, Some(&SessionReport::default()));
        assert!(snapshot_for(&app, sid).is_none(), "after a session");
        let dir = temp_dir("handed-out");
        crate::persist::save_awaiting(&dir, &app.awaiting_snapshot.lock().unwrap()).unwrap();
        *app.awaiting_snapshot.lock().unwrap() = crate::persist::load_awaiting(&dir);
        std::fs::remove_dir_all(&dir).ok();
        assert!(snapshot_for(&app, sid).is_none(), "after a restart");
        // Once a snapshot is taken, we answer with what we hold.
        assert!(apply_snapshot(&handle, sid, snapshot_of(&real)));
        let Some(WirePayload::SyncResponse { server, .. }) = snapshot_for(&app, sid) else { panic!("no snapshot") };
        assert_eq!(server.owner, owner.id());
    }

    #[test]
    fn each_member_answering_a_sync_request_announces_its_join() {
        let owner = Member::new("owner");
        let friend = Member::new("friend");
        let me = Member::new("me");
        let (mut real, _) = server(&owner);
        join(&mut real, &friend);
        let sid = real.server.id;
        let replies_of = |member: &Member, awaiting: bool| {
            let app = app_with(member, real.clone());
            app.set_awaiting_snapshot(sid, awaiting);
            sync_replies(&app, sid)
        };
        let joined = |payload: &WirePayload, who: &Member| {
            matches!(payload, WirePayload::Event(ServerEvent::MemberJoin { profile, .. })
                if profile.id == who.id() && profile.identity == Some(who.identity.public()))
        };
        let from_owner = replies_of(&owner, false);
        assert!(matches!(from_owner.as_slice(), [WirePayload::SyncResponse { .. }, join] if joined(join, &owner)));
        // One still waiting for its own snapshot sends its join only.
        let from_friend = replies_of(&friend, true);
        assert!(matches!(from_friend.as_slice(), [join] if joined(join, &friend)));
        assert!(sync_replies(&app_with(&owner, real.clone()), ServerId::new()).is_empty());

        // The owner's snapshot was too large to name the friend. Whether
        // the friend's join comes first or last, we learn of them.
        let mut trimmed = from_owner[0].clone();
        if let WirePayload::SyncResponse { members, profiles, .. } = &mut trimmed {
            members.retain(|m| m.user != friend.id());
            profiles.retain(|p| p.id != friend.id());
        }
        let WirePayload::Event(friend_join) = from_friend[0].clone() else { panic!("not an event") };
        for join_first in [true, false] {
            let app = app_with(&me, placeholder(&me, sid));
            app.set_awaiting_snapshot(sid, true);
            let apply_join = || app.servers.lock().unwrap().get_mut(&sid).unwrap().apply(friend_join.clone()).unwrap();
            if join_first {
                apply_join();
            }
            assert!(apply_snapshot(&TestHandle::new(app.clone()), sid, trimmed.clone()));
            if !join_first {
                apply_join();
            }
            let state = app.servers.lock().unwrap()[&sid].clone();
            assert_eq!(state.server.owner, owner.id(), "{join_first}");
            assert!(state.server.is_member(friend.id()) && state.server.is_member(me.id()), "{join_first}");
            assert_eq!(state.identity_of(friend.id()), Some(friend.identity.public()), "{join_first}");
        }
    }

    #[test]
    fn our_snapshot_leaves_out_a_key_held_back() {
        let owner = Member::new("owner");
        let (state, _) = server(&owner);
        let sid = state.server.id;
        let app = app_with(&owner, state);
        let our_key = |payload: WirePayload| match payload {
            WirePayload::SyncResponse { profiles, .. } => {
                profiles.into_iter().find(|p| p.id == owner.id()).and_then(|p| p.identity)
            }
            _ => panic!("not a snapshot"),
        };
        assert_eq!(our_key(snapshot_for(&app, sid).unwrap()), Some(owner.identity.public()));
        app.hold_key(Instant::now() + Duration::from_secs(120));
        assert_eq!(our_key(snapshot_for(&app, sid).unwrap()), None);
        assert!(snapshot_for(&app, ServerId::new()).is_none());
    }

    #[test]
    fn our_snapshot_leaves_out_what_a_held_key_signed() {
        let owner = Member::new("owner");
        let friend = Member::new("friend");
        let (mut state, channel) = server(&owner);
        join(&mut state, &friend);
        let ours = owner.post(&state, channel, "signed with a key held back");
        let theirs = friend.post(&state, channel, "theirs");
        for message in [&ours, &theirs] {
            state.apply(ServerEvent::MessageCreate { message: message.clone() }).unwrap();
        }
        let sid = state.server.id;
        let app = app_with(&owner, state);
        let sent = |app: &App| match snapshot_for(app, sid).unwrap() {
            WirePayload::SyncResponse { recent_messages, .. } => recent_messages.into_iter().map(|m| m.id).collect::<HashSet<_>>(),
            _ => panic!("not a snapshot"),
        };
        app.hold_key(Instant::now() + Duration::from_secs(120));
        assert_eq!(sent(&app), HashSet::from([theirs.id]));
        assert!(app.settle_key());
        assert_eq!(sent(&app), HashSet::from([ours.id, theirs.id]));
    }

    #[test]
    fn store_without_identity_cannot_start_sessions() {
        let owner = Member::new("owner");
        let (state, _) = server(&owner);
        let sid = state.server.id;
        let app = app_with(&owner, state);
        *app.identity.lock().unwrap() = None;
        assert!(AppStore(TestHandle::new(app)).local_identity(sid).is_none());
    }

    #[test]
    fn a_key_not_settled_yet_starts_no_sessions() {
        let owner = Member::new("owner");
        let (state, _) = server(&owner);
        let sid = state.server.id;
        let app = app_with(&owner, state);
        let store = AppStore(TestHandle::new(app.clone()));
        app.hold_key(Instant::now() + Duration::from_secs(120));
        assert!(store.local_identity(sid).is_none());
        assert!(app.settle_key());
        assert!(store.local_identity(sid).is_some());
        // A hold that ran out counts as settled.
        app.hold_key(Instant::now());
        assert!(store.local_identity(sid).is_some());
    }

    #[test]
    fn servers_joined_before_the_upgrade_still_wait_for_their_snapshot() {
        let me = Member::new("member");
        let owner = Member::new("owner");
        let (ours, inviter) = ([1u8; 32], [2u8; 32]);
        // What joining leaves until the snapshot: ours, with only us in it.
        let (placeholder, channel) = server(&me);
        assert!(joined_placeholder(&placeholder, me.id(), Some(&[inviter]), ours));

        // An invite of our own is a server we created; without a readable
        // invite we cannot tell, so nothing changes.
        assert!(!joined_placeholder(&placeholder, me.id(), Some(&[ours]), ours));
        assert!(!joined_placeholder(&placeholder, me.id(), Some(&[inviter, ours]), ours));
        assert!(!joined_placeholder(&placeholder, me.id(), Some(&[]), ours));
        assert!(!joined_placeholder(&placeholder, me.id(), None, ours));

        // History, someone else, or another owner: the state is real.
        let mut talked = placeholder.clone();
        talked.apply(ServerEvent::MessageCreate { message: me.post(&placeholder, channel, "hi") }).unwrap();
        assert!(!joined_placeholder(&talked, me.id(), Some(&[inviter]), ours));
        let mut met = placeholder.clone();
        join(&mut met, &owner);
        assert!(!joined_placeholder(&met, me.id(), Some(&[inviter]), ours));
        let mut theirs = ServerState::new(Server::new("s", owner.id()));
        theirs.profiles.insert(me.id(), me.profile.clone());
        assert!(!joined_placeholder(&theirs, me.id(), Some(&[inviter]), ours));
    }

    #[tokio::test]
    async fn startup_marks_servers_joined_from_someone_elses_invite() {
        let (mut inviter, _, _, _) = commy_net::CommyNode::spawn(None).await.unwrap();
        let me = Member::new("member");
        let (joined, _) = server(&me);
        let (created, _) = server(&me);
        let (sid, created_id) = (joined.server.id, created.server.id);
        let ticket = tokio::time::timeout(Duration::from_secs(30), inviter.host_server(sid, "s")).await.unwrap().unwrap();
        let theirs = inviter.node_id_bytes();
        assert_eq!(invite_nodes(&ticket.encode()), Some(vec![theirs]));
        assert_eq!(invite_nodes("not an invite"), None);

        let app = app_with(&me, joined);
        app.servers.lock().unwrap().insert(created_id, created);
        let mut own = ticket.clone();
        own.server_id = created_id;
        let invites = HashMap::from([(sid, ticket.encode()), (created_id, own.encode())]);
        // Seen from the inviter's node, both invites are its own.
        mark_joined_placeholders(&app, &invites, theirs);
        assert!(app.awaiting_snapshot.lock().unwrap().is_empty());
        mark_joined_placeholders(&app, &HashMap::from([(sid, ticket.encode())]), [9; 32]);
        assert_eq!(*app.awaiting_snapshot.lock().unwrap(), HashSet::from([sid]));
        assert!(app.awaits_snapshot(sid));
        inviter.shutdown().await.ok();
    }
}
