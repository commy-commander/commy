//! Catch-up with gossip neighbors, the app's side of `commy_net::catchup`:
//! the store over `App.servers`, when sessions start, what happens after
//! one, and events held until their signer's key is known.

use crate::state::App;
use chrono::Utc;
use commy_core::catchup::{digests, items, BucketItems};
use commy_core::{
    BucketDigest, BucketKey, ChannelId, Identity, IdentityKey, Message, MessageId, ServerEvent,
    ServerId, ServerState, StoreError, TombstoneProof, UserId,
};
use commy_net::catchup::{
    apply_items, check_peer, collect, ApplyReport, CatchupError, CatchupStore, PeerCheck,
    SessionReport,
};
use commy_net::SharedCatchupStore;
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

/// Events held per server for an unknown author key, and for how long.
pub const HOLD_CAP: usize = 1000;
pub const HOLD_FOR: Duration = Duration::from_secs(15 * 60);

type SessionKey = (ServerId, [u8; 32]);

/// Catch-up bookkeeping kept in `App.catchup`.
#[derive(Debug, Default)]
pub struct Catchup {
    /// Sessions we started that are still running.
    running: HashSet<SessionKey>,
    /// When the last session with a peer completed, either side.
    completed: HashMap<SessionKey, Instant>,
    /// Remote message events whose signer has no known key yet.
    held: HashMap<ServerId, VecDeque<(Instant, ServerEvent)>>,
    /// New messages from others per channel, not yet told to the webview.
    fresh: HashMap<ServerId, HashMap<ChannelId, u32>>,
    /// Servers changed in memory and not saved yet.
    dirty: HashSet<ServerId>,
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

    /// Holds an event until its signer's key arrives; the oldest go past the cap.
    pub fn hold(&mut self, server: ServerId, event: ServerEvent, now: Instant) {
        let queue = self.held.entry(server).or_default();
        queue.push_back((now, event));
        while queue.len() > HOLD_CAP {
            queue.pop_front();
        }
    }

    #[cfg(test)]
    pub fn held_count(&self, server: ServerId) -> usize {
        self.held.get(&server).map_or(0, VecDeque::len)
    }

    /// The server's held events, oldest first, without expired ones.
    pub fn take_held(&mut self, server: ServerId, now: Instant) -> Vec<(Instant, ServerEvent)> {
        self.held
            .remove(&server)
            .map(|queue| queue.into_iter().filter(|(at, _)| !expired(*at, now)).collect())
            .unwrap_or_default()
    }

    /// Puts events still waiting back in front of any held meanwhile.
    pub fn restore_held(&mut self, server: ServerId, events: Vec<(Instant, ServerEvent)>) {
        if events.is_empty() {
            return;
        }
        let queue = self.held.entry(server).or_default();
        for item in events.into_iter().rev() {
            queue.push_front(item);
        }
        while queue.len() > HOLD_CAP {
            queue.pop_front();
        }
    }

    /// Drops held events past [`HOLD_FOR`] in every server.
    pub fn expire_held(&mut self, now: Instant) {
        for queue in self.held.values_mut() {
            queue.retain(|(at, _)| !expired(*at, now));
        }
        self.held.retain(|_, queue| !queue.is_empty());
    }

    /// Notes that a server changed, with the channels of new messages.
    pub fn note_applied(&mut self, server: ServerId, fresh: impl IntoIterator<Item = ChannelId>) {
        self.dirty.insert(server);
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

    /// Everything about a server we left.
    pub fn forget_server(&mut self, server: ServerId) {
        self.held.remove(&server);
        self.fresh.remove(&server);
        self.dirty.remove(&server);
        self.completed.retain(|(s, _), _| *s != server);
    }
}

fn expired(at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(at) >= HOLD_FOR
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

/// Applies one catch-up page. Returns the report, the events to hold,
/// and the channels of messages from others that are new to us.
pub fn apply_page(
    state: &mut ServerState,
    me: Option<UserId>,
    messages: Vec<Message>,
    tombstones: Vec<TombstoneProof>,
) -> (ApplyReport, Vec<ServerEvent>, Vec<ChannelId>) {
    let new: HashMap<MessageId, ChannelId> = messages
        .iter()
        .filter(|m| Some(m.author) != me && !state.has_message(m.id))
        .map(|m| (m.id, m.channel))
        .collect();
    let (report, held) = apply_items(state, messages, tombstones);
    let fresh = report
        .applied
        .iter()
        .filter(|id| state.has_message(**id))
        .filter_map(|id| new.get(id).copied())
        .collect();
    (report, held, fresh)
}

/// Outcome of retrying held events.
#[derive(Debug, Default)]
pub struct Retried {
    pub changed: bool,
    pub fresh: Vec<ChannelId>,
    /// Events whose signer is still unknown, with their hold time.
    pub still: Vec<(Instant, ServerEvent)>,
}

/// Re-applies held events in order; those still missing a key stay held.
pub fn retry_held(state: &mut ServerState, me: Option<UserId>, events: Vec<(Instant, ServerEvent)>) -> Retried {
    let mut out = Retried::default();
    for (at, event) in events {
        let new_message = match &event {
            ServerEvent::MessageCreate { message } if Some(message.author) != me && !state.has_message(message.id) => {
                Some((message.id, message.channel))
            }
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
            Err(StoreError::UnknownAuthorKey(_)) => out.still.push((at, event)),
            Err(e) => tracing::debug!("held event rejected: {e}"),
        }
    }
    out
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

/// Reaches the app state from a tauri handle, or from a plain `App` in tests.
pub trait AppAccess: Send + Sync + 'static {
    fn app(&self) -> &App;
}

impl AppAccess for tauri::AppHandle {
    fn app(&self) -> &App {
        self.state::<App>().inner()
    }
}

#[cfg(test)]
impl AppAccess for Arc<App> {
    fn app(&self) -> &App {
        self
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

    fn apply(&self, server: ServerId, messages: Vec<Message>, tombstones: Vec<TombstoneProof>) -> ApplyReport {
        let app = self.0.app();
        let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
        let (report, held, fresh) = {
            let mut servers = app.servers.lock().unwrap();
            let Some(state) = servers.get_mut(&server) else { return ApplyReport::default() };
            apply_page(state, me, messages, tombstones)
        };
        let now = Instant::now();
        let mut catchup = app.catchup.lock().unwrap();
        for event in held {
            catchup.hold(server, event, now);
        }
        if !report.applied.is_empty() {
            catchup.note_applied(server, fresh);
        }
        report
    }
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
                "caught up on {server}: sent {}, received {}, held {}",
                report.pushed.len(),
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

/// Clears pending sends the peer now holds, then saves and announces
/// what the session changed.
pub fn after_session(handle: &tauri::AppHandle, server: ServerId, report: Option<&SessionReport>) {
    let app = handle.state::<App>();
    let mut outbox_changed = false;
    if let Some(report) = report {
        let delivered = report.pushed.iter().chain(&report.peer_had).copied();
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
    flush(handle, server, outbox_changed);
}

/// Saves a server that catch-up or released events changed and tells the
/// webview: `server-event` to redraw, `messages-caught-up` for unread counts.
fn flush(handle: &tauri::AppHandle, server: ServerId, redraw: bool) {
    let app = handle.state::<App>();
    let changes = app.catchup.lock().unwrap().take_changes(server);
    if changes.changed {
        app.persist_server(server);
    }
    if changes.changed || redraw {
        let _ = handle.emit("server-event", &server.to_string());
    }
    if !changes.fresh.is_empty() {
        let _ = handle.emit("messages-caught-up", caught_up_payload(server, &changes.fresh));
    }
}

/// Holds a remote message event whose signer has no known key yet.
pub fn hold(app: &App, server: ServerId, event: ServerEvent) {
    app.catchup.lock().unwrap().hold(server, event, Instant::now());
}

/// Retries a server's held events, after a member's profile (and so
/// perhaps their key) arrived. New messages bump unread counts silently.
pub fn release_held(handle: &tauri::AppHandle, server: ServerId) {
    let app = handle.state::<App>();
    let events = app.catchup.lock().unwrap().take_held(server, Instant::now());
    if events.is_empty() {
        return;
    }
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let retried = {
        let mut servers = app.servers.lock().unwrap();
        let Some(state) = servers.get_mut(&server) else { return };
        retry_held(state, me, events)
    };
    {
        let mut catchup = app.catchup.lock().unwrap();
        catchup.restore_held(server, retried.still);
        if retried.changed {
            catchup.note_applied(server, retried.fresh);
        }
    }
    flush(handle, server, false);
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
        let t0 = Instant::now();
        for i in 0..HOLD_CAP + 2 {
            catchup.hold(sid, typing(i), t0);
        }
        assert_eq!(catchup.held_count(sid), HOLD_CAP);
        let held = catchup.take_held(sid, t0);
        assert_eq!(held.len(), HOLD_CAP);
        assert_eq!(held[0].1, typing(2), "the oldest were dropped");
        assert_eq!(catchup.held_count(sid), 0);

        catchup.hold(sid, typing(0), t0);
        catchup.hold(sid, typing(1), t0 + Duration::from_secs(60));
        let later = t0 + HOLD_FOR + Duration::from_secs(1);
        assert_eq!(catchup.take_held(sid, later), vec![(t0 + Duration::from_secs(60), typing(1))]);

        catchup.hold(sid, typing(3), t0);
        catchup.expire_held(later);
        assert_eq!(catchup.held_count(sid), 0);

        // Restored events go in front of ones held meanwhile.
        catchup.hold(sid, typing(9), t0);
        catchup.restore_held(sid, vec![(t0, typing(1)), (t0, typing(2))]);
        let order: Vec<ServerEvent> = catchup.take_held(sid, t0).into_iter().map(|(_, e)| e).collect();
        assert_eq!(order, vec![typing(1), typing(2), typing(9)]);
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
            catchup.hold(sid, event, t0);
        }
        let retried = retry_held(&mut state, Some(owner.id()), catchup.take_held(sid, t0));
        assert!(!retried.changed && retried.fresh.is_empty());
        assert_eq!(retried.still.len(), 2, "still no key");
        catchup.restore_held(sid, retried.still);

        join(&mut state, &newcomer);
        catchup.hold(sid, edit, t0);
        let retried = retry_held(&mut state, Some(owner.id()), catchup.take_held(sid, t0));
        assert!(retried.changed && retried.still.is_empty());
        assert_eq!(retried.fresh, vec![channel, channel], "the edit is not a new message");
        assert_eq!(state.message(first.id).unwrap().content, "first, edited");
        assert!(state.has_message(second.id));

        // Our own messages coming back are not unread.
        let mine = owner.post(&state, channel, "mine");
        let retried = retry_held(&mut state, Some(owner.id()), vec![(t0, ServerEvent::MessageCreate { message: mine })]);
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
            Some(owner.id()),
            vec![known.clone(), new.clone(), mine.clone(), theirs.clone(), deleted.clone()],
            vec![proof],
        );
        assert!(report.applied.contains(&new.id) && report.applied.contains(&mine.id));
        assert!(!report.applied.contains(&known.id));
        assert_eq!(fresh, vec![channel], "only the friend's new message is unread");
        assert_eq!(held, vec![ServerEvent::MessageCreate { message: theirs }]);
        assert_eq!(report.held, 1);
        assert!(!state.has_message(deleted.id));
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
        let store = AppStore(app.clone());

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
        let changes = app.catchup.lock().unwrap().take_changes(sid);
        assert!(changes.changed);
        assert_eq!(changes.fresh, HashMap::from([(channel, 1)]));

        // A message from someone whose join has not arrived waits in the queue.
        let late = Member::new("late");
        let early = late.post(&app.servers.lock().unwrap()[&sid], channel, "early");
        let report = store.apply(sid, vec![early.clone()], vec![]);
        assert_eq!((report.applied.len(), report.held), (0, 1));
        assert_eq!(app.catchup.lock().unwrap().held_count(sid), 1);
        assert_eq!(app.catchup.lock().unwrap().take_changes(sid), Changes::default());

        join(app.servers.lock().unwrap().get_mut(&sid).unwrap(), &late);
        let held = app.catchup.lock().unwrap().take_held(sid, Instant::now());
        let retried = retry_held(app.servers.lock().unwrap().get_mut(&sid).unwrap(), Some(owner.id()), held);
        assert_eq!(retried.fresh, vec![channel]);
        assert!(app.servers.lock().unwrap()[&sid].has_message(early.id));
    }

    #[test]
    fn store_without_identity_cannot_start_sessions() {
        let owner = Member::new("owner");
        let (state, _) = server(&owner);
        let sid = state.server.id;
        let app = app_with(&owner, state);
        *app.identity.lock().unwrap() = None;
        assert!(AppStore(app).local_identity(sid).is_none());
    }
}
