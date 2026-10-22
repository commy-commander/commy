use crate::state::App;
use chrono::Utc;
use commy_core::catchup::{apply_reaction_changes, digests, items, reaction_changes, BucketItems, ReactionChange};
use commy_core::{
    BucketDigest, BucketKey, ChannelId, Identity, IdentityKey, Message, MessageId, ServerEvent,
    ServerId, ServerState, SignedEvent, StoreError, TombstoneProof, UserId, UserProfile,
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

pub const HIGHER_NODE_START_DELAY: Duration = Duration::from_secs(15);

pub const RECENT_SESSION: Duration = Duration::from_secs(60);

pub const PERIODIC_INTERVAL: Duration = Duration::from_secs(600);

pub const SWEEP_INTERVAL: Duration = Duration::from_secs(600);
pub const STARTUP_SWEEP_DELAY: Duration = Duration::from_secs(3);
pub const MAX_SWEEP_PEERS: usize = 16;

pub const SAVE_DELAY: Duration = Duration::from_secs(2);

pub const HOLD_CAP: usize = 1000;
pub const HOLD_FOR: Duration = Duration::from_secs(15 * 60);
pub const HOLD_BYTES: usize = 4 * 1024 * 1024;
pub const HOLD_ACTIONS_CAP: usize = 512;
pub const HOLD_ACTIONS_PER_SIGNER: usize = 32;
pub const SNAPSHOT_MAX_AGE_MS: i64 = 900_000;

type SessionKey = (ServerId, [u8; 32]);

pub type Held = (Instant, UserId, ServerEvent);

#[derive(Debug, Default)]
pub struct Catchup {
    running: HashSet<SessionKey>,
    completed: HashMap<SessionKey, Instant>,
    swept: HashMap<ServerId, Instant>,
    held: HashMap<ServerId, VecDeque<(Held, usize)>>,
    held_actions: HashMap<ServerId, VecDeque<(Instant, SignedEvent)>>,
    fresh: HashMap<ServerId, HashMap<ChannelId, FreshChannel>>,
    dirty: HashSet<ServerId>,
    unsaved: HashSet<ServerId>,
    save_scheduled: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub changed: bool,
    pub fresh: HashMap<ChannelId, FreshChannel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fresh {
    pub channel: ChannelId,
    pub author: UserId,
    pub content: String,
    pub has_attachments: bool,
    pub voice_message: bool,
    pub at: i64,
    pub mentioned: bool,
}

impl Fresh {
    pub fn of(message: &Message, names: &[&str]) -> Self {
        Self {
            channel: message.channel,
            author: message.author,
            content: message.content.clone(),
            has_attachments: message.files().next().is_some(),
            voice_message: commy_core::message::only_a_voice_message(&message.content, message.attachments.iter()),
            at: message.created_at.timestamp(),
            mentioned: mentions(&message.content, names),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreshChannel {
    pub count: u32,
    pub latest: Fresh,
    pub mentioned: bool,
}

pub fn mentions(content: &str, names: &[&str]) -> bool {
    names_mentioned(content, names)
}

pub use commy_core::mention::names_mentioned;

pub fn mention_names(profile: &commy_core::UserProfile) -> Vec<String> {
    [Some(profile.username.clone()), profile.display_name.clone()].into_iter().flatten().collect()
}

impl Catchup {
    pub fn begin(&mut self, key: SessionKey) -> bool {
        self.running.insert(key)
    }

    pub fn finish(&mut self, key: SessionKey, now: Instant, completed: bool) {
        self.running.remove(&key);
        if completed {
            self.record_completed(key, now);
        }
    }

    pub fn record_completed(&mut self, key: SessionKey, now: Instant) {
        self.completed.retain(|_, at| now.saturating_duration_since(*at) < RECENT_SESSION);
        self.completed.insert(key, now);
    }

    pub fn completed_recently(&self, key: SessionKey, now: Instant) -> bool {
        self.completed
            .get(&key)
            .is_some_and(|at| now.saturating_duration_since(*at) < RECENT_SESSION)
    }

    pub fn sweep_due(&mut self, server: ServerId, now: Instant) -> bool {
        if self.swept.get(&server).is_some_and(|at| now.saturating_duration_since(*at) < SWEEP_INTERVAL) {
            return false;
        }
        self.swept.insert(server, now);
        true
    }

    pub fn needs_session(&self, key: SessionKey, now: Instant) -> bool {
        !self.running.contains(&key) && !self.completed_recently(key, now)
    }

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

    #[cfg(test)]
    pub fn take_all_held(&mut self, server: ServerId, now: Instant) -> Vec<Held> {
        let signers = self.held.get(&server).map(|q| q.iter().map(|((_, by, _), _)| *by).collect()).unwrap_or_default();
        self.take_held(server, &signers, now)
    }

    pub fn take_held(&mut self, server: ServerId, signers: &HashSet<UserId>, now: Instant) -> Vec<Held> {
        let live = |((at, _, _), _): &(Held, usize)| !expired(*at, now);
        let taken = take_where(&mut self.held, server, live, |((_, by, _), _)| signers.contains(by));
        taken.into_iter().map(|(held, _)| held).collect()
    }

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

    pub fn expire_held(&mut self, now: Instant) {
        expire_queues(&mut self.held, |((at, _, _), _)| !expired(*at, now));
        expire_queues(&mut self.held_actions, |(at, _)| !expired(*at, now));
    }

    pub fn hold_action(&mut self, server: ServerId, signed: SignedEvent, now: Instant) {
        let queue = self.held_actions.entry(server).or_default();
        let by = signed.action.by;
        let same = queue.iter().filter(|(_, held)| held.action.by == by).count();
        if same >= HOLD_ACTIONS_PER_SIGNER {
            if let Some(oldest) = queue.iter().position(|(_, held)| held.action.by == by) {
                queue.remove(oldest);
            }
        }
        queue.push_back((now, signed));
        while queue.len() > HOLD_ACTIONS_CAP {
            queue.pop_front();
        }
    }

    pub fn take_held_actions(&mut self, server: ServerId, signers: &HashSet<UserId>, now: Instant) -> Vec<(Instant, SignedEvent)> {
        let live = |(at, _): &(Instant, SignedEvent)| !expired(*at, now);
        let taken = take_where(&mut self.held_actions, server, live, |(_, signed)| signers.contains(&signed.action.by));
        taken.into_iter().collect()
    }

    pub fn restore_held_actions(&mut self, server: ServerId, events: Vec<(Instant, SignedEvent)>) {
        if events.is_empty() {
            return;
        }
        let queue = self.held_actions.entry(server).or_default();
        for held in events.into_iter().rev() {
            queue.push_front(held);
        }
        while queue.len() > HOLD_ACTIONS_CAP {
            queue.pop_front();
        }
    }

    pub fn note_applied(&mut self, server: ServerId, fresh: impl IntoIterator<Item = Fresh>) {
        self.dirty.insert(server);
        self.unsaved.insert(server);
        let channels = self.fresh.entry(server).or_default();
        for message in fresh {
            match channels.get_mut(&message.channel) {
                Some(channel) => {
                    channel.count += 1;
                    channel.mentioned |= message.mentioned;
                    if message.at >= channel.latest.at {
                        channel.latest = message;
                    }
                }
                None => {
                    let mentioned = message.mentioned;
                    channels.insert(message.channel, FreshChannel { count: 1, latest: message, mentioned });
                }
            }
        }
        if channels.is_empty() {
            self.fresh.remove(&server);
        }
    }

    pub fn take_changes(&mut self, server: ServerId) -> Changes {
        Changes {
            changed: self.dirty.remove(&server),
            fresh: self.fresh.remove(&server).unwrap_or_default(),
        }
    }

    pub fn schedule_save(&mut self) -> bool {
        if self.unsaved.is_empty() || self.save_scheduled {
            return false;
        }
        self.save_scheduled = true;
        true
    }

    pub fn take_unsaved(&mut self) -> Vec<ServerId> {
        self.save_scheduled = false;
        self.unsaved.drain().collect()
    }

    pub fn take_unsaved_server(&mut self, server: ServerId) -> bool {
        self.unsaved.remove(&server)
    }

    pub fn forget_server(&mut self, server: ServerId) {
        self.held.remove(&server);
        self.held_actions.remove(&server);
        self.fresh.remove(&server);
        self.dirty.remove(&server);
        self.unsaved.remove(&server);
        self.completed.retain(|(s, _), _| *s != server);
    }
}

fn expired(at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(at) >= HOLD_FOR
}

fn take_where<T>(
    map: &mut HashMap<ServerId, VecDeque<T>>,
    server: ServerId,
    live: impl Fn(&T) -> bool,
    take: impl Fn(&T) -> bool,
) -> VecDeque<T> {
    let Some(queue) = map.get_mut(&server) else { return VecDeque::new() };
    let (taken, kept) = queue.drain(..).filter(|item| live(item)).partition(|item| take(item));
    *queue = kept;
    if queue.is_empty() {
        map.remove(&server);
    }
    taken
}

fn expire_queues<T>(map: &mut HashMap<ServerId, VecDeque<T>>, live: impl Fn(&T) -> bool) {
    for queue in map.values_mut() {
        queue.retain(&live);
    }
    map.retain(|_, queue| !queue.is_empty());
}

fn held_size(event: &ServerEvent) -> Option<usize> {
    let size = commy_net::protocol::gossip_size(&WirePayload::Event(event.clone()));
    (size <= commy_net::protocol::MAX_FRAME_BYTES).then_some(size)
}

fn trim_held(queue: &mut VecDeque<(Held, usize)>) {
    let mut bytes: usize = queue.iter().map(|(_, size)| size).sum();
    while queue.len() > HOLD_CAP || bytes > HOLD_BYTES {
        let Some((_, size)) = queue.pop_front() else { break };
        bytes -= size;
    }
}

fn hold_key(event: &ServerEvent) -> Option<(u8, MessageId, Option<UserId>)> {
    match event {
        ServerEvent::MessageCreate { message } => Some((0, message.id, None)),
        ServerEvent::MessageEdit { message, .. } => Some((1, *message, None)),
        ServerEvent::MessageDelete { message, by, .. } => Some((2, *message, Some(*by))),
        _ => None,
    }
}

fn newer_edit(new: &ServerEvent, old: &ServerEvent) -> bool {
    matches!(
        (new, old),
        (ServerEvent::MessageEdit { edited_at: a, .. }, ServerEvent::MessageEdit { edited_at: b, .. }) if a > b
    )
}

pub fn start_delay(ours: [u8; 32], peer: [u8; 32], pending: bool) -> Option<Duration> {
    (!pending && ours > peer).then_some(HIGHER_NODE_START_DELAY)
}

pub fn holdable(event: &ServerEvent) -> bool {
    matches!(
        event,
        ServerEvent::MessageCreate { .. } | ServerEvent::MessageEdit { .. } | ServerEvent::MessageDelete { .. }
    )
}

#[derive(Debug, Default)]
pub struct Unread {
    me: Option<UserId>,
    names: Vec<String>,
    blocked: HashSet<UserId>,
    silent: bool,
    keys: crate::channel_lock::KeyHolder,
}

impl Unread {
    pub fn of(app: &App, server: ServerId) -> Self {
        let profile = app.profile.lock().unwrap().clone();
        Self {
            me: profile.as_ref().map(|p| p.id),
            names: profile.as_ref().map(mention_names).unwrap_or_default(),
            blocked: app.safety.lock().unwrap().blocked.iter().copied().collect(),
            silent: app.awaits_snapshot(server),
            keys: crate::channel_lock::holder_of(app),
        }
    }

    fn fresh(&self, state: &ServerState, message: &Message) -> Fresh {
        let names: Vec<&str> = self.names.iter().map(String::as_str).collect();
        let Some(opened) = self.keys.open(state, message) else {
            return Fresh::of(&Message { content: String::new(), ..message.clone() }, &names);
        };
        let mut fresh = Fresh::of(&opened, &names);
        fresh.mentioned = self
            .me
            .is_some_and(|me| state.server.mentioned_by(opened.author, &opened.content, me, &names));
        fresh
    }

    fn counts(&self, state: &ServerState, message: &Message) -> bool {
        if self.silent
            || Some(message.author) == self.me
            || self.blocked.contains(&message.author)
            || state.has_message(message.id)
            || self.me.is_none_or(|me| !state.server.can_view(message.channel, me))
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

pub fn apply_page(
    state: &mut ServerState,
    unread: &Unread,
    messages: Vec<Message>,
    tombstones: Vec<TombstoneProof>,
) -> (ApplyReport, Vec<(UserId, ServerEvent)>, Vec<Fresh>) {
    let new: HashMap<MessageId, Fresh> = messages
        .iter()
        .filter(|m| unread.counts(state, m))
        .map(|m| (m.id, unread.fresh(state, m)))
        .collect();
    let (report, held) = apply_items(state, messages, tombstones);
    let held = held.into_iter().filter_map(|event| Some((item_signer(&event)?, event))).collect();
    let fresh = report
        .applied
        .iter()
        .filter(|id| shown_to_members(state, **id))
        .filter_map(|id| new.get(id).cloned())
        .collect();
    (report, held, fresh)
}

fn shown_to_members(state: &ServerState, id: MessageId) -> bool {
    state.message(id).is_some_and(|m| state.hold(m).is_none())
}

fn item_signer(event: &ServerEvent) -> Option<UserId> {
    match event {
        ServerEvent::MessageCreate { message } => Some(message.author),
        ServerEvent::MessageDelete { by, .. } => Some(*by),
        _ => None,
    }
}

#[derive(Debug, Default)]
pub struct Retried {
    pub changed: bool,
    pub fresh: Vec<Fresh>,
    pub still: Vec<Held>,
}

pub fn retry_held(state: &mut ServerState, unread: &Unread, events: Vec<Held>) -> Retried {
    let mut out = Retried::default();
    for (at, _, event) in events {
        let new_message = match &event {
            ServerEvent::MessageCreate { message } if unread.counts(state, message) => Some((message.id, unread.fresh(state, message))),
            _ => None,
        };
        match state.apply(event.clone()) {
            Ok(true) => {
                out.changed = true;
                if let Some((id, fresh)) = new_message {
                    if shown_to_members(state, id) {
                        out.fresh.push(fresh);
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

pub use commy_net::catchup::apply_live;

pub fn caught_up_payload(server: ServerId, fresh: &HashMap<ChannelId, FreshChannel>, state: Option<&ServerState>) -> serde_json::Value {
    let channels: serde_json::Map<String, serde_json::Value> = fresh
        .iter()
        .map(|(channel, fresh)| {
            let latest = &fresh.latest;
            let found = state.and_then(|s| s.server.channel(*channel));
            let value = serde_json::json!({
                "count": fresh.count,
                "mentioned": fresh.mentioned,
                "channel_name": found.map(|c| c.name.clone()).unwrap_or_default(),
                "announcement": found.is_some_and(|c| c.kind == commy_core::ChannelKind::Announcement),
                "sensitive": found.is_some_and(|c| c.sensitive),
                "author": latest.author.to_string(),
                "author_name": state
                    .and_then(|s| crate::view::known_name_of(s, latest.author))
                    .unwrap_or_else(|| "Someone".into()),
                "content": latest.content,
                "has_attachments": latest.has_attachments,
                "voice_message": latest.voice_message,
                "at": latest.at,
            });
            (channel.to_string(), value)
        })
        .collect();
    serde_json::json!({
        "server": server.to_string(),
        "server_name": state.map(|s| s.server.name.clone()).unwrap_or_default(),
        "channels": channels,
    })
}

pub fn sweep_targets(state: &ServerState, me: Option<UserId>, ours: [u8; 32]) -> Vec<[u8; 32]> {
    let mut found: Vec<(bool, [u8; 32])> = state
        .profiles
        .values()
        .filter(|p| Some(p.id) != me && state.server.is_current_member(p.id))
        .filter_map(|p| {
            let node: [u8; 32] = data_encoding::HEXLOWER.decode(p.node.as_ref()?.as_bytes()).ok()?.try_into().ok()?;
            Some((p.presence == commy_core::Presence::Offline, node))
        })
        .filter(|(_, node)| *node != ours)
        .collect();
    found.sort();
    let mut seen = HashSet::new();
    found
        .into_iter()
        .map(|(_, node)| node)
        .filter(|node| seen.insert(*node))
        .take(MAX_SWEEP_PEERS)
        .collect()
}

pub fn sweep_all<A: AppAccess>(access: &A) {
    let servers: Vec<ServerId> = access.app().servers.lock().unwrap().keys().copied().collect();
    for server in servers {
        sweep(access, server, true);
    }
}

fn sweep<A: AppAccess>(access: &A, server: ServerId, always: bool) {
    let app = access.app();
    let Some(ours) = our_node(app) else { return };
    let now = Instant::now();
    if !app.catchup.lock().unwrap().sweep_due(server, now) && !always {
        return;
    }
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let targets = {
        let servers = app.servers.lock().unwrap();
        let Some(state) = servers.get(&server) else { return };
        sweep_targets(state, me, ours)
    };
    let targets: Vec<[u8; 32]> = {
        let catchup = app.catchup.lock().unwrap();
        targets.into_iter().filter(|peer| catchup.needs_session((server, *peer), now)).collect()
    };
    for peer in targets {
        access.catch_up(server, peer);
    }
}

fn pick(neighbors: &[[u8; 32]], seed: u64) -> Option<[u8; 32]> {
    (!neighbors.is_empty()).then(|| neighbors[(seed % neighbors.len() as u64) as usize])
}

pub trait AppAccess: Send + Sync + 'static {
    fn app(&self) -> &App;
    fn emit(&self, event: &str, payload: serde_json::Value);
    fn save_later(&self);
    fn catch_up(&self, server: ServerId, peer: [u8; 32]);
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

    fn catch_up(&self, server: ServerId, peer: [u8; 32]) {
        spawn_session(self.clone(), server, peer, None, false);
    }
}

pub fn save_unsaved(app: &App) {
    let servers = app.catchup.lock().unwrap().take_unsaved();
    for server in servers {
        app.persist_server(server);
    }
}

#[cfg(test)]
#[derive(Clone, Default)]
pub struct TestHandle {
    pub app: Arc<App>,
    events: Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>>,
    pub saves_scheduled: Arc<std::sync::atomic::AtomicUsize>,
    pub catch_ups: Arc<std::sync::Mutex<Vec<SessionKey>>>,
}

#[cfg(test)]
impl TestHandle {
    pub fn new(app: Arc<App>) -> Self {
        Self { app, events: Default::default(), saves_scheduled: Default::default(), catch_ups: Default::default() }
    }

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

    fn catch_up(&self, server: ServerId, peer: [u8; 32]) {
        self.catch_ups.lock().unwrap().push((server, peer));
    }
}

pub struct AppStore<A>(pub A);

impl<A: AppAccess> AppStore<A> {
    fn with_state<T>(&self, server: ServerId, f: impl FnOnce(&ServerState) -> T) -> Option<T> {
        self.0.app().servers.lock().unwrap().get(&server).map(f)
    }
}

impl<A: AppAccess> CatchupStore for AppStore<A> {
    fn local_identity(&self, server: ServerId) -> Option<(UserId, Identity)> {
        let app = self.0.app();
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
        release_held(&self.0, server, &learned);
    }

    fn apply(&self, server: ServerId, messages: Vec<Message>, tombstones: Vec<TombstoneProof>) -> ApplyReport {
        let app = self.0.app();
        let unread = Unread::of(app, server);
        let (report, held, fresh) = {
            let mut servers = app.servers.lock().unwrap();
            let Some(state) = servers.get_mut(&server) else {
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
        if !report.applied.is_empty() {
            flush(&self.0, server, false);
        }
        report
    }

    fn reaction_changes(&self, server: ServerId, user: UserId) -> Vec<ReactionChange> {
        self.with_state(server, |s| reaction_changes(s, user, Utc::now())).unwrap_or_default()
    }

    fn apply_reaction_changes(&self, server: ServerId, user: UserId, changes: Vec<ReactionChange>) -> usize {
        let app = self.0.app();
        let changed = {
            let mut servers = app.servers.lock().unwrap();
            let Some(state) = servers.get_mut(&server) else { return 0 };
            apply_reaction_changes(state, user, changes, Utc::now())
        };
        if changed > 0 {
            app.catchup.lock().unwrap().note_applied(server, []);
            flush(&self.0, server, false);
        }
        changed
    }
}

fn dedup(ids: impl IntoIterator<Item = MessageId>) -> Vec<MessageId> {
    let mut seen = HashSet::new();
    ids.into_iter().filter(|id| seen.insert(*id)).collect()
}

pub fn store(handle: &tauri::AppHandle) -> SharedCatchupStore {
    Arc::new(AppStore(handle.clone()))
}

fn our_node(app: &App) -> Option<[u8; 32]> {
    let hex = app.node_hex.lock().unwrap().clone()?;
    data_encoding::HEXLOWER.decode(hex.as_bytes()).ok()?.try_into().ok()
}

pub fn on_neighbor_up(handle: &tauri::AppHandle, server: ServerId, peer: [u8; 32]) {
    let app = handle.state::<App>();
    let Some(ours) = our_node(&app) else { return };
    let pending = app.outbox.lock().unwrap().has_pending(server);
    spawn_session(handle.clone(), server, peer, start_delay(ours, peer, pending), true);
}

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
                "caught up on {server}: delivered {}, received {}, held {}, reactions {}",
                report.delivered.len(),
                report.received.len(),
                report.held,
                report.reactions
            );
            after_session(handle, server, Some(&report));
        }
        Err(e) => {
            tracing::debug!("catch-up on {server} did not complete: {e:#}");
            after_session(handle, server, None);
            let wait = e.downcast_ref::<CatchupError>().and_then(CatchupError::retry_after_ms);
            if let (true, Some(ms)) = (retry, wait) {
                spawn_session(handle.clone(), server, peer, Some(Duration::from_millis(ms)), false);
            }
        }
    }
}

pub fn on_caught_up(handle: &tauri::AppHandle, server: ServerId, peer: [u8; 32], report: &SessionReport) {
    handle.state::<App>().catchup.lock().unwrap().record_completed((server, peer), Instant::now());
    after_session(handle, server, Some(report));
}

pub fn after_session<A: AppAccess>(access: &A, server: ServerId, report: Option<&SessionReport>) {
    let app = access.app();
    let outbox_changed = {
        let servers = app.servers.lock().unwrap();
        let mut outbox = app.outbox.lock().unwrap();
        let delivered = report.is_some_and(|r| outbox.clear_delivered(server, r.delivered.iter().copied()));
        let pruned = servers.get(&server).is_some_and(|state| outbox.prune(state, Utc::now()));
        delivered | pruned
    };
    if outbox_changed {
        app.persist_outbox();
    }
    if app.catchup.lock().unwrap().take_unsaved_server(server) {
        app.persist_server(server);
    }
    crate::doorbell::settle(access, server);
    flush(access, server, outbox_changed);
    if report.is_some() {
        sweep(access, server, false);
    }
}

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
        let payload = {
            let servers = app.servers.lock().unwrap();
            caught_up_payload(server, &changes.fresh, servers.get(&server))
        };
        access.emit("messages-caught-up", payload);
    }
}

pub fn hold(app: &App, server: ServerId, signer: UserId, event: ServerEvent) {
    app.catchup.lock().unwrap().hold(server, signer, event, Instant::now());
}

pub fn release_held<A: AppAccess>(access: &A, server: ServerId, signers: &[UserId]) {
    let app = access.app();
    let signers: HashSet<UserId> = signers.iter().copied().collect();
    let (events, actions) = if signers.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        let mut catchup = app.catchup.lock().unwrap();
        let now = Instant::now();
        (catchup.take_held(server, &signers, now), catchup.take_held_actions(server, &signers, now))
    };
    if !actions.is_empty() {
        let mut still = Vec::new();
        let mut changed = false;
        {
            let mut servers = app.servers.lock().unwrap();
            if let Some(state) = servers.get_mut(&server) {
                for (at, signed) in actions {
                    match state.apply_signed(&signed) {
                        Ok(true) => changed = true,
                        Ok(false) => {}
                        Err(StoreError::UnknownAuthorKey(_)) => still.push((at, signed)),
                        Err(e) => tracing::debug!("held action rejected: {e}"),
                    }
                }
            }
        }
        let mut catchup = app.catchup.lock().unwrap();
        catchup.restore_held_actions(server, still);
        if changed {
            catchup.note_applied(server, []);
        }
    }
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

pub fn snapshot_for(app: &App, server: ServerId) -> Option<WirePayload> {
    if app.awaits_snapshot(server) {
        return None;
    }
    if !app.key_settled() {
        return None;
    }
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id)?;
    let identity = app.identity()?;
    let mut payload = commy_net::protocol::build_sync_response(app.servers.lock().unwrap().get(&server)?, 50, me);
    commy_net::protocol::sign_sync_response(&mut payload, Utc::now().timestamp_millis(), &identity);
    Some(payload)
}

pub fn service_snapshot_for(app: &App, server: ServerId) -> Option<WirePayload> {
    if app.awaits_snapshot(server) || !app.key_settled() {
        return None;
    }
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id)?;
    let identity = app.identity()?;
    let budget = commy_net::community_streams::SNAPSHOT_BYTES;
    let mut payload = commy_net::protocol::build_sync_response_within(app.servers.lock().unwrap().get(&server)?, 0, me, budget);
    commy_net::protocol::sign_sync_response(&mut payload, Utc::now().timestamp_millis(), &identity);
    Some(payload)
}

pub fn learn_service_profiles(handle: &tauri::AppHandle, server: ServerId, payload: &WirePayload) {
    if !snapshot_vouched(handle.app(), server, payload, commy_net::community_streams::RETENTION_MS) {
        return;
    }
    if let WirePayload::SyncResponse { profiles, .. } = payload {
        AppStore(handle.clone()).apply_profiles(server, profiles.clone());
    }
}

pub fn sync_replies(app: &App, server: ServerId) -> Vec<WirePayload> {
    let Some(me) = app.profile.lock().unwrap().clone() else { return Vec::new() };
    if !app.servers.lock().unwrap().contains_key(&server) {
        return Vec::new();
    }
    let join = app.wire_for(server, ServerEvent::MemberJoin { server, profile: app.announced(me) });
    snapshot_for(app, server).into_iter().chain([join]).collect()
}

pub fn await_snapshot(app: &App, server: ServerId, me: UserProfile) -> Vec<WirePayload> {
    app.set_awaiting_snapshot(server, true);
    let join = app.wire_for(server, ServerEvent::MemberJoin { server, profile: app.announced(me) });
    vec![join, WirePayload::SyncRequest]
}

pub fn apply_snapshot<A: AppAccess>(access: &A, server: ServerId, payload: WirePayload) -> bool {
    apply_snapshot_within(access, server, payload, SNAPSHOT_MAX_AGE_MS)
}

pub fn apply_service_snapshot<A: AppAccess>(access: &A, server: ServerId, payload: WirePayload) -> bool {
    apply_snapshot_within(access, server, payload, commy_net::community_streams::RETENTION_MS)
}

fn apply_snapshot_within<A: AppAccess>(access: &A, server: ServerId, payload: WirePayload, max_age_ms: i64) -> bool {
    let app = access.app();
    if !app.awaits_snapshot(server) {
        return false;
    }
    if !snapshot_vouched(app, server, &payload, max_age_ms) {
        tracing::warn!("dropping a snapshot of {server} that its owner did not sign");
        return false;
    }
    let WirePayload::SyncResponse { server: mut remote_server, members, profiles, recent_messages, voice, at, .. } = payload else {
        return false;
    };
    if remote_server.id != server {
        return false;
    }
    remote_server.members = members.into_iter().map(member_of).collect();
    let me = app.profile.lock().unwrap().clone();
    let me_id = me.as_ref().map(|p| p.id);
    let mut held = Vec::new();
    let learned: Vec<UserId>;
    {
        let mut servers = app.servers.lock().unwrap();
        let Some(old) = servers.remove(&server) else { return false };
        let pinned: HashMap<UserId, IdentityKey> = old.profiles.values().filter_map(|p| Some((p.id, p.identity?))).collect();
        let mut state = ServerState::new(remote_server);
        state.set_action_floor(at);
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
        for sticker in old.server.stickers.iter().cloned() {
            let _ = state.apply(ServerEvent::StickerAdd { server, sticker });
        }
        for emoticon in old.server.emoticons.iter().cloned() {
            let _ = state.apply(ServerEvent::EmoticonAdd { server, emoticon });
        }
        for sound in old.server.soundboard.iter().cloned() {
            let _ = state.apply(ServerEvent::SoundAdd { server, sound });
        }
        for pin in old.server.pins.iter().cloned() {
            let event = ServerEvent::MessagePin { server, channel: pin.channel, message: pin.message, by: pin.by, at: pin.at, pinned: true };
            let _ = state.apply(event);
        }
        for profile in profiles {
            state.profiles.insert(profile.id, profile.as_seen_by_others());
        }
        for profile in old.profiles.values().filter(|p| p.identity.is_some()) {
            let entry = state.profiles.entry(profile.id).or_insert_with(|| profile.clone());
            entry.identity = entry.identity.or(profile.identity);
        }
        let heard = old.server.members.iter().filter_map(|m| old.profiles.get(&m.user));
        for profile in heard.filter(|p| Some(p.id) != me_id) {
            let profile = state.profiles.get(&profile.id).cloned().unwrap_or_else(|| profile.clone());
            let _ = state.apply(ServerEvent::MemberJoin { server, profile });
        }
        for user in old.bots() {
            let _ = state.apply(ServerEvent::BotSet { server, user, bot: true });
        }
        state.carry_bot_commands(&old);
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
        for entry in voice.into_iter().filter(|v| Some(v.user) != me_id) {
            let _ = state.apply(entry.into_event());
        }
        let waited = old.snapshot();
        for message in waited.messages.into_iter().flat_map(|(_, list)| list) {
            let _ = state.apply(ServerEvent::MessageCreate { message });
        }
        for proof in waited.tombstone_proofs {
            let _ = state.apply(proof.into_event());
        }
        state.carry_polls(&old);
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
    release_held(access, server, &learned);
    app.set_awaiting_snapshot(server, false);
    access.emit("server-event", server.to_string().into());
    access.emit("servers-changed", serde_json::Value::Null);
    true
}

fn snapshot_vouched(app: &App, server: ServerId, payload: &WirePayload, max_age_ms: i64) -> bool {
    let WirePayload::SyncResponse { server: remote, by, at, .. } = payload else { return false };
    if remote.id != server {
        return false;
    }
    let Some((owner, key)) = app.ticket_owner(server) else { return false };
    if *by != owner || remote.owner != owner {
        return false;
    }
    if (Utc::now().timestamp_millis() - *at).abs() > max_age_ms {
        return false;
    }
    commy_net::protocol::verify_sync_response(payload, &key)
}

fn member_of(m: commy_net::protocol::SyncMember) -> commy_core::Member {
    let joined_at = chrono::DateTime::from_timestamp_millis(m.joined_ms).unwrap_or_default();
    commy_core::Member { user: m.user, nickname: m.nickname, roles: m.roles, joined_at, timeout: None }
}

pub fn invite_nodes(raw: &str) -> Option<Vec<[u8; 32]>> {
    let ticket = commy_net::ServerTicket::decode(raw).ok()?;
    Some(ticket.bootstrap.iter().map(|addr| addr.node_id).collect())
}

pub fn joined_placeholder(state: &ServerState, me: UserId, invite: Option<&[[u8; 32]]>, our_node: [u8; 32]) -> bool {
    let placeholder = state.server.owner == me
        && state.profiles.keys().all(|id| *id == me)
        && state.server.channels.iter().all(|c| state.messages(c.id).is_empty());
    let joined = invite.is_some_and(|nodes| !nodes.is_empty() && !nodes.contains(&our_node));
    placeholder && joined
}

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

pub fn settled_own_server(state: &ServerState, me: UserId, invite: Option<&str>) -> bool {
    let named_owner = invite
        .and_then(|raw| commy_net::ServerTicket::decode(raw).ok())
        .and_then(|ticket| ticket.owner)
        .is_some_and(|(owner, _)| owner == me);
    let heard_others = state.server.channels.iter().any(|c| state.messages(c.id).iter().any(|m| m.author != me));
    state.server.owner == me && named_owner && heard_others
}

pub fn release_own_servers(app: &App, invites: &HashMap<ServerId, String>) {
    let Some(me) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { return };
    let awaiting = app.awaiting_snapshot.lock().unwrap().clone();
    let settled: Vec<ServerId> = {
        let servers = app.servers.lock().unwrap();
        awaiting
            .into_iter()
            .filter(|id| servers.get(id).is_some_and(|state| settled_own_server(state, me, invites.get(id).map(String::as_str))))
            .collect()
    };
    for server in settled {
        tracing::warn!("{server} is ours and already has history; it no longer waits for a snapshot");
        app.set_awaiting_snapshot(server, false);
    }
}

pub fn every_neighbor(neighbors: Vec<(ServerId, Vec<[u8; 32]>)>) -> Vec<SessionKey> {
    neighbors
        .into_iter()
        .flat_map(|(server, peers)| peers.into_iter().map(move |peer| (server, peer)))
        .collect()
}

pub async fn run_periodic(handle: tauri::AppHandle) {
    tokio::time::sleep(STARTUP_SWEEP_DELAY).await;
    sweep_all(&handle);
    loop {
        tokio::time::sleep(PERIODIC_INTERVAL).await;
        handle.state::<App>().catchup.lock().unwrap().expire_held(Instant::now());
        let neighbors = crate::settle::KeyNet::neighbors(&handle).await;
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

    struct Member {
        profile: UserProfile,
        identity: Identity,
    }

    impl Member {
        fn new(name: &str) -> Self {
            let identity = Identity::generate();
            let mut profile = UserProfile::new(name);
            profile.identity = Some(identity.public());
            OWNERS.lock().unwrap().insert(profile.id, identity.clone());
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

    fn join_without_key(state: &mut ServerState, member: &Member) {
        let server = state.server.id;
        let profile = UserProfile { identity: None, ..member.profile.clone() };
        state.apply(ServerEvent::MemberJoin { server, profile }).unwrap();
    }

    fn unread_for(member: &Member) -> Unread {
        Unread { me: Some(member.id()), ..Default::default() }
    }

    #[test]
    fn lower_node_starts_unless_we_have_pending_sends() {
        let (low, high) = ([1u8; 32], [2u8; 32]);
        assert_eq!(start_delay(low, high, false), None);
        assert_eq!(start_delay(high, low, false), Some(HIGHER_NODE_START_DELAY));
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
        let post = |i: usize, reactors: usize| {
            let mut message = stranger.post(&state, channel, &format!("{i:04} {}", "x".repeat(3000)));
            message.reactions.push(commy_core::Reaction { emoji: ":up".into(), users: (0..reactors).map(|_| UserId::new()).collect() });
            ServerEvent::MessageCreate { message }
        };
        let events: Vec<ServerEvent> = (0..800).map(|i| post(i, 300)).collect();
        let size = |e: &ServerEvent| held_size(e).unwrap();
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
        catchup.hold(sid, by, post(800, 5000), t0);
        let held: Vec<ServerEvent> = catchup.take_all_held(sid, t0).into_iter().map(|(_, _, e)| e).collect();
        assert!(held.iter().map(size).sum::<usize>() <= HOLD_BYTES);
        assert_eq!(held.len(), fits);
        assert_eq!(held, events[events.len() - fits..], "the oldest were dropped");

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
        let author = UserId::new();
        let fresh = |channel: ChannelId, content: &str, at: i64| Fresh {
            channel,
            author,
            content: content.into(),
            has_attachments: false,
            voice_message: false,
            at,
            mentioned: mentions(content, &["me"]),
        };
        catchup.note_applied(sid, [fresh(a, "@me later", 20), fresh(b, "other", 5), fresh(a, "earlier", 10)]);
        catchup.note_applied(sid, []);
        let changes = catchup.take_changes(sid);
        assert!(changes.changed);
        assert_eq!(changes.fresh[&a], FreshChannel { count: 2, latest: fresh(a, "@me later", 20), mentioned: true });
        assert_eq!(changes.fresh[&b], FreshChannel { count: 1, latest: fresh(b, "other", 5), mentioned: false });
        assert_eq!(catchup.take_changes(sid), Changes::default());

        let payload = caught_up_payload(sid, &changes.fresh, None);
        assert_eq!(payload["server"], sid.to_string());
        let in_a = &payload["channels"][a.to_string()];
        assert_eq!((in_a["count"].as_u64(), in_a["mentioned"].as_bool()), (Some(2), Some(true)));
        assert_eq!((in_a["content"].as_str(), in_a["at"].as_i64()), (Some("@me later"), Some(20)));
        assert_eq!(in_a["author_name"], "Someone", "no state to name the author");
        assert_eq!(payload["channels"][b.to_string()]["count"], 1);
    }

    #[test]
    fn mentions_match_whole_names_only() {
        assert!(mentions("hey @sam, lunch?", &["sam"]));
        assert!(mentions("HEY @Sam", &["sam"]));
        assert!(!mentions("hey @samantha", &["sam"]));
        assert!(!mentions("@everyone look", &[]), "everyone and here depend on who sent them");
        assert!(!mentions("ping @here", &["x"]));
        assert!(!mentions("mail sam@example.org", &["sam"]));
        assert!(!mentions("@", &[""]));
        assert!(mentions("@Sam Smith is here", &["Sam Smith"]));
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
        assert_eq!(
            retried.fresh.iter().map(|f| f.content.as_str()).collect::<Vec<_>>(),
            ["first", "second"],
            "the edit is not a new message"
        );
        assert_eq!(state.message(first.id).unwrap().content, "first, edited");
        assert!(state.has_message(second.id));

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
        assert_eq!(fresh, vec![Fresh::of(&new, &[])], "only the friend's new message is unread");
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

        assert_eq!(apply_live(&mut state, delete.into_event(), None), (Ok(true), None, None));
        let create = ServerEvent::MessageCreate { message: message.clone() };
        assert_eq!(apply_live(&mut state, create.clone(), None), (Ok(true), None, None), "only its proof changed");
        assert!(!state.has_message(message.id));
        assert_eq!(apply_live(&mut state, create, None), (Ok(false), None, None));

        let fresh = friend.post(&state, channel, "hello");
        let create = ServerEvent::MessageCreate { message: fresh.clone() };
        assert_eq!(apply_live(&mut state, create.clone(), None), (Ok(true), Some(fresh.clone()), None));
        assert_eq!(apply_live(&mut state, create, None), (Ok(false), None, None));
        let mut edited = fresh.clone();
        edited.content = "hello again".into();
        edited.edited_at = Some(Utc::now() + chrono::Duration::seconds(1));
        edited.sign(sid, &friend.identity);
        assert_eq!(apply_live(&mut state, ServerEvent::MessageCreate { message: edited }, None), (Ok(true), None, None));
        let rename = ServerEvent::ServerRename { server: sid, name: "renamed".into() };
        assert_eq!(apply_live(&mut state, rename.clone(), None), (Err(StoreError::Unsigned), None, None));
        let by_friend = SignedEvent::sign(sid, friend.id(), 1, rename.clone(), &friend.identity);
        assert_eq!(apply_live(&mut state, rename.clone(), Some(by_friend.action)), (Err(StoreError::NotAllowed), None, None));
        let by_owner = SignedEvent::sign(sid, owner.id(), 1, rename.clone(), &owner.identity);
        assert_eq!(apply_live(&mut state, rename, Some(by_owner.action)), (Ok(true), None, None));
        assert_eq!(state.server.name, "renamed");
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

    fn invited_by(app: &App, sid: ServerId, owner: &Member) {
        let mut ticket = commy_net::ServerTicket::new(sid, "s", [0; 32], vec![]);
        ticket.owner = Some((owner.id(), owner.identity.public()));
        app.invites.lock().unwrap().insert(sid, ticket.encode());
    }

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
        let caught_up = {
            let servers = app.servers.lock().unwrap();
            let fresh = FreshChannel { count: 1, latest: Fresh::of(&theirs, &[]), mentioned: false };
            caught_up_payload(sid, &HashMap::from([(channel, fresh)]), servers.get(&sid))
        };
        assert_eq!(caught_up["channels"][channel.to_string()]["author_name"], friend.profile.visible_name());
        assert_eq!(
            handle.take_events(),
            vec![("server-event".into(), sid.to_string().into()), ("messages-caught-up".into(), caught_up)]
        );

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
        assert_eq!(retried.fresh, vec![Fresh::of(&early, &[])]);
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

        let first = friend.post(&app.servers.lock().unwrap()[&sid], channel, "first page");
        let second = friend.post(&app.servers.lock().unwrap()[&sid], channel, "second page");
        store.apply(sid, vec![first.clone()], vec![]);
        let names: Vec<String> = handle.take_events().into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["server-event", "messages-caught-up"]);
        store.apply(sid, vec![second.clone()], vec![]);
        assert_eq!(handle.take_events().len(), 2);
        assert!(!saved_has(&dir, sid, first.id), "not saved per page");
        assert_eq!(handle.saves_scheduled.load(std::sync::atomic::Ordering::SeqCst), 1, "one save for both");

        let started = Instant::now();
        while !saved_has(&dir, sid, second.id) && started.elapsed() < SAVE_DELAY * 5 {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(started.elapsed() >= SAVE_DELAY - Duration::from_millis(500));
        assert!(saved_has(&dir, sid, first.id) && saved_has(&dir, sid, second.id));
        assert_eq!(app.catchup.lock().unwrap().take_changes(sid), Changes::default(), "nothing left over");

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

        let absent = Member::new("absent");
        join_without_key(app.servers.lock().unwrap().get_mut(&sid).unwrap(), &absent);
        let message = absent.post(&app.servers.lock().unwrap()[&sid], channel, "while you were away");
        store.apply(sid, vec![message.clone()], vec![]);
        assert_eq!(app.catchup.lock().unwrap().held_count(sid), 1);

        store.apply_profiles(sid, vec![absent.profile.clone()]);
        let state = app.servers.lock().unwrap()[&sid].clone();
        assert!(state.has_message(message.id));
        assert_eq!(state.identity_of(absent.id()), Some(absent.identity.public()));
        assert_eq!(app.catchup.lock().unwrap().held_count(sid), 0);
        let fresh = FreshChannel { count: 1, latest: Fresh::of(&message, &[]), mentioned: false };
        let caught_up = caught_up_payload(sid, &HashMap::from([(channel, fresh)]), Some(&state));
        assert!(handle.take_events().contains(&("messages-caught-up".into(), caught_up)));

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
        app.servers.lock().unwrap().get_mut(&sid).unwrap().profiles.get_mut(&b.id()).unwrap().identity =
            Some(b.identity.public());
        handle.take_events();
        let live = |event: ServerEvent| {
            let pinned = apply_live(app.servers.lock().unwrap().get_mut(&sid).unwrap(), event, None).2;
            release_held(&handle, sid, pinned.as_slice());
            pinned
        };
        let signed_live = |member: &Member, event: ServerEvent| {
            let signed = SignedEvent::sign(sid, member.id(), 1, event, &member.identity);
            let pinned = apply_live(app.servers.lock().unwrap().get_mut(&sid).unwrap(), signed.event, Some(signed.action)).2;
            release_held(&handle, sid, pinned.as_slice());
            pinned
        };
        let has = |m: &Message| app.servers.lock().unwrap()[&sid].has_message(m.id);

        let keyless = UserProfile { identity: None, ..a.profile.clone() };
        assert_eq!(live(ServerEvent::MemberJoin { server: sid, profile: keyless }), None);
        assert_eq!(signed_live(&friend, ServerEvent::ProfileUpdate { profile: friend.profile.clone() }), None);
        assert!(!has(&by_a) && !has(&by_b));
        assert_eq!(app.catchup.lock().unwrap().held_count(sid), 2);
        assert!(handle.take_events().is_empty());

        assert_eq!(signed_live(&a, ServerEvent::ProfileUpdate { profile: a.profile.clone() }), Some(a.id()));
        assert!(has(&by_a) && !has(&by_b));
        assert_eq!(app.catchup.lock().unwrap().held_count(sid), 1);
        let caught_up = {
            let servers = app.servers.lock().unwrap();
            let fresh = FreshChannel { count: 1, latest: Fresh::of(&by_a, &[]), mentioned: false };
            caught_up_payload(sid, &HashMap::from([(channel, fresh)]), servers.get(&sid))
        };
        assert!(handle.take_events().contains(&("messages-caught-up".into(), caught_up)));

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
        let by = owner.id();
        for _ in 0..3 {
            catchup.hold(sid, by, ServerEvent::MessageCreate { message: message.clone() }, t0);
        }
        assert_eq!(catchup.held_count(sid), 1);

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

        let other = Member::new("other");
        let by_owner = TombstoneProof::sign(sid, channel, message.id, owner.id(), Utc::now(), &owner.identity);
        let by_other = TombstoneProof::sign(sid, channel, message.id, other.id(), Utc::now(), &other.identity);
        for proof in [&by_owner, &by_other, &by_owner] {
            catchup.hold(sid, proof.by, proof.clone().into_event(), t0);
        }
        assert_eq!(catchup.held_count(sid), 2);

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
        let unread = Unread { me: Some(owner.id()), blocked: HashSet::from([pest.id()]), ..Default::default() };

        let page = pest.post(&state, channel, "caught up");
        let (report, _, fresh) = apply_page(&mut state, &unread, vec![page.clone()], vec![]);
        assert_eq!(report.applied, vec![page.id], "still applied");
        assert!(fresh.is_empty());

        let released = pest.post(&state, channel, "released");
        let retried = retry_held(&mut state, &unread, vec![(Instant::now(), pest.id(), ServerEvent::MessageCreate { message: released.clone() })]);
        assert!(retried.changed && state.has_message(released.id));
        assert!(retried.fresh.is_empty());

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
        assert_eq!(fresh, vec![Fresh::of(&new, &[])], "only the message after our join");
    }

    #[test]
    fn reactions_from_catch_up_are_drawn_and_offered_back() {
        let owner = Member::new("owner");
        let friend = Member::new("friend");
        let (mut state, channel) = server(&owner);
        join(&mut state, &friend);
        let held = owner.post(&state, channel, "held by both");
        state.apply(ServerEvent::MessageCreate { message: held.clone() }).unwrap();
        let sid = state.server.id;
        let app = app_with(&owner, state);
        let handle = TestHandle::new(app.clone());
        let store = AppStore(handle.clone());

        let change = |emoji: &str, active| ReactionChange { message: held.id, emoji: emoji.into(), at: 1_000, active };
        assert_eq!(store.apply_reaction_changes(sid, friend.id(), vec![change("👍", true)]), 1);
        let reactions = app.servers.lock().unwrap()[&sid].message(held.id).unwrap().reactions.clone();
        assert_eq!(reactions[0].users, vec![friend.id()]);
        assert_eq!(handle.take_events(), vec![("server-event".into(), sid.to_string().into())]);

        assert_eq!(store.apply_reaction_changes(sid, friend.id(), vec![change("👍", true)]), 0);
        assert!(handle.take_events().is_empty(), "nothing to redraw");
        assert_eq!(store.reaction_changes(sid, friend.id()), vec![change("👍", true)]);
        assert!(store.reaction_changes(sid, owner.id()).is_empty());
    }

    #[test]
    fn a_finished_session_catches_up_with_every_other_member() {
        let node = |n: u8| [n; 32];
        let hex = |n: u8| data_encoding::HEXLOWER.encode(&node(n));
        let owner = Member::new("owner");
        let (mut state, _) = server(&owner);
        let mut members = Vec::new();
        for (name, n, presence) in [
            ("away", 2, commy_core::Presence::Offline),
            ("first", 3, commy_core::Presence::Online),
            ("second", 4, commy_core::Presence::Idle),
            ("banned", 5, commy_core::Presence::Online),
        ] {
            let mut member = Member::new(name);
            member.profile.node = Some(hex(n));
            member.profile.presence = presence;
            join(&mut state, &member);
            members.push(member);
        }
        let mut nodeless = Member::new("nodeless");
        nodeless.profile.node = Some("not hex".into());
        join(&mut state, &nodeless);
        state.server.banned.push(members[3].id());
        let sid = state.server.id;
        let app = app_with(&owner, state);
        *app.node_hex.lock().unwrap() = Some(hex(1));
        let handle = TestHandle::new(app.clone());

        after_session(&handle, sid, None);
        assert!(handle.catch_ups.lock().unwrap().is_empty(), "only a finished session sweeps");

        app.catchup.lock().unwrap().record_completed((sid, node(3)), Instant::now());
        after_session(&handle, sid, Some(&SessionReport::default()));
        assert_eq!(std::mem::take(&mut *handle.catch_ups.lock().unwrap()), vec![(sid, node(4)), (sid, node(2))]);

        after_session(&handle, sid, Some(&SessionReport::default()));
        assert!(handle.catch_ups.lock().unwrap().is_empty(), "one sweep per interval");

        let later = Instant::now() + SWEEP_INTERVAL;
        assert!(app.catchup.lock().unwrap().sweep_due(sid, later));
    }

    #[test]
    fn launching_catches_up_with_every_member_of_every_server() {
        let hex = |n: u8| data_encoding::HEXLOWER.encode(&[n; 32]);
        let owner = Member::new("owner");
        let app = Arc::new(App::default());
        *app.profile.lock().unwrap() = Some(owner.profile.clone());
        *app.node_hex.lock().unwrap() = Some(hex(1));
        let mut expected = Vec::new();
        for n in [2, 3] {
            let (mut state, _) = server(&owner);
            let mut friend = Member::new("friend");
            friend.profile.node = Some(hex(n));
            join(&mut state, &friend);
            expected.push((state.server.id, [n; 32]));
            app.servers.lock().unwrap().insert(state.server.id, state);
        }
        let handle = TestHandle::new(app);

        expected.sort();
        for _ in 0..2 {
            sweep_all(&handle);
            let mut asked = std::mem::take(&mut *handle.catch_ups.lock().unwrap());
            asked.sort();
            assert_eq!(asked, expected, "every launch sweeps");
        }

        after_session(&handle, expected[0].0, Some(&SessionReport::default()));
        assert!(handle.catch_ups.lock().unwrap().is_empty(), "the launch sweep counts");
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

        let stranger = Member::new("stranger");
        join_without_key(app.servers.lock().unwrap().get_mut(&sid).unwrap(), &stranger);
        let held = stranger.post(&app.servers.lock().unwrap()[&sid], channel, "held");
        store.apply(sid, vec![held.clone()], vec![]);
        store.apply_profiles(sid, vec![stranger.profile.clone()]);
        assert!(app.servers.lock().unwrap()[&sid].has_message(held.id));
        assert!(handle.take_events().iter().all(|(name, _)| name != "messages-caught-up"));

        app.set_awaiting_snapshot(sid, false);
        let live = friend.post(&app.servers.lock().unwrap()[&sid], channel, "live");
        store.apply(sid, vec![live], vec![]);
        assert!(handle.take_events().iter().any(|(name, _)| name == "messages-caught-up"));
    }

    #[test]
    fn a_snapshot_is_taken_only_from_the_owner_the_invite_names() {
        let owner = Member::new("owner");
        let friend = Member::new("friend");
        let me = Member::new("me");
        let (mut real, _) = server(&owner);
        join(&mut real, &friend);
        join(&mut real, &me);
        let sid = real.server.id;
        let app = app_with(&me, placeholder(&me, sid));
        let handle = TestHandle::new(app.clone());
        app.set_awaiting_snapshot(sid, true);
        let mut ticket = commy_net::ServerTicket::new(sid, "s", [0; 32], vec![]);
        ticket.owner = Some((owner.id(), owner.identity.public()));
        app.invites.lock().unwrap().insert(sid, ticket.encode());

        assert!(!apply_snapshot(&handle, sid, snapshot_by(&real, &friend)), "a member's word is not enough");
        let mut forged = real.clone();
        forged.server.owner = friend.id();
        assert!(!apply_snapshot(&handle, sid, snapshot_by(&forged, &friend)), "nor a claim to be the owner");
        let mut unsigned = snapshot_by(&real, &owner);
        if let WirePayload::SyncResponse { server, .. } = &mut unsigned {
            server.name = "tampered".into();
        }
        assert!(!apply_snapshot(&handle, sid, unsigned), "nor the owner's snapshot changed on the way");
        assert!(app.awaits_snapshot(sid));
        assert_eq!(app.servers.lock().unwrap()[&sid].server.owner, me.id());

        assert!(apply_snapshot(&handle, sid, snapshot_by(&real, &owner)));
        assert!(!app.awaits_snapshot(sid));
        assert_eq!(app.servers.lock().unwrap()[&sid].server.owner, owner.id());

        let other = Member::new("other");
        let (mut legacy, _) = server(&other);
        join(&mut legacy, &me);
        let lid = legacy.server.id;
        let app = app_with(&me, placeholder(&me, lid));
        let handle = TestHandle::new(app.clone());
        app.set_awaiting_snapshot(lid, true);
        app.invites.lock().unwrap().insert(lid, commy_net::ServerTicket::new(lid, "s", [0; 32], vec![]).encode());
        assert!(!apply_snapshot(&handle, lid, snapshot_by(&legacy, &other)), "an invite without an owner vouches for nobody");
        assert!(app.awaits_snapshot(lid));
    }

    #[test]
    fn a_snapshot_from_another_day_is_not_taken() {
        let owner = Member::new("owner");
        let me = Member::new("me");
        let (mut real, _) = server(&owner);
        join(&mut real, &me);
        let sid = real.server.id;
        let app = app_with(&me, placeholder(&me, sid));
        let handle = TestHandle::new(app.clone());
        app.set_awaiting_snapshot(sid, true);
        let mut ticket = commy_net::ServerTicket::new(sid, "s", [0; 32], vec![]);
        ticket.owner = Some((owner.id(), owner.identity.public()));
        app.invites.lock().unwrap().insert(sid, ticket.encode());
        let mut old = commy_net::protocol::build_sync_response(&real, 50, owner.id());
        let then = Utc::now().timestamp_millis() - 2 * SNAPSHOT_MAX_AGE_MS;
        commy_net::protocol::sign_sync_response(&mut old, then, &owner.identity);
        assert!(!apply_snapshot(&handle, sid, old));
        assert!(app.awaits_snapshot(sid));
        assert!(apply_snapshot(&handle, sid, snapshot_by(&real, &owner)));
        let floor = app.servers.lock().unwrap()[&sid].action_floor();
        assert!(floor > then && floor <= Utc::now().timestamp_millis());
    }

    #[test]
    fn one_signer_cannot_crowd_out_what_others_have_waiting() {
        let sid = ServerId::new();
        let loud = Member::new("loud");
        let quiet = Member::new("quiet");
        let mut catchup = Catchup::default();
        let now = Instant::now();
        let signed = |who: &Member, at: i64| {
            SignedEvent::sign(sid, who.id(), at, ServerEvent::ServerRename { server: sid, name: format!("{at}") }, &who.identity)
        };
        catchup.hold_action(sid, signed(&quiet, 1), now);
        for at in 0..(HOLD_ACTIONS_PER_SIGNER as i64 + 8) {
            catchup.hold_action(sid, signed(&loud, at), now);
        }
        let signers: HashSet<UserId> = [loud.id(), quiet.id()].into_iter().collect();
        let taken = catchup.take_held_actions(sid, &signers, now);
        assert_eq!(taken.iter().filter(|(_, s)| s.action.by == loud.id()).count(), HOLD_ACTIONS_PER_SIGNER);
        assert!(taken.iter().any(|(_, s)| s.action.by == quiet.id()), "the quiet one's action is still there");
        assert!(taken.iter().all(|(_, s)| s.action.by != loud.id() || s.action.at >= 8), "the oldest of the loud one's went");
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
        let report = SessionReport { delivered: vec![taken.id], ..Default::default() };
        after_session(&handle, sid, Some(&report));
        assert_eq!(app.outbox.lock().unwrap().pending_ids(sid), HashSet::from([refused.id]));
        assert_eq!(handle.take_events(), vec![("server-event".into(), sid.to_string().into())]);
    }

    fn snapshot_of(state: &ServerState) -> WirePayload {
        snapshot_by(state, &owner_of(state))
    }

    fn snapshot_by(state: &ServerState, member: &Member) -> WirePayload {
        let mut payload = commy_net::protocol::build_sync_response(state, 50, member.id());
        commy_net::protocol::sign_sync_response(&mut payload, Utc::now().timestamp_millis(), &member.identity);
        payload
    }

    fn owner_of(state: &ServerState) -> Member {
        let profile = state.profiles[&state.server.owner].clone();
        let identity = OWNERS.lock().unwrap().get(&profile.id).cloned().expect("an owner made with Member::new");
        Member { profile, identity }
    }

    static OWNERS: std::sync::Mutex<std::collections::BTreeMap<UserId, Identity>> = std::sync::Mutex::new(std::collections::BTreeMap::new());

    #[test]
    fn only_a_server_waiting_for_it_takes_a_snapshot() {
        let owner = Member::new("owner");
        let me = Member::new("me");
        let (mut real, channel) = server(&owner);
        join(&mut real, &me);
        let sid = real.server.id;
        let hello = owner.post(&real, channel, "hello");
        real.apply(ServerEvent::MessageCreate { message: hello.clone() }).unwrap();

        let mut provisional = commy_core::Server::new("s", me.id());
        provisional.id = sid;
        let mut placeholder = ServerState::new(provisional);
        placeholder.profiles.insert(me.id(), me.profile.clone());
        let app = app_with(&me, placeholder);
        let handle = TestHandle::new(app.clone());

        assert!(!apply_snapshot(&handle, sid, snapshot_of(&real)));
        assert!(!app.servers.lock().unwrap()[&sid].has_message(hello.id));

        app.set_awaiting_snapshot(sid, true);

        invited_by(&app, sid, &owner);
        let (other, _) = server(&owner);
        assert!(!apply_snapshot(&handle, sid, snapshot_of(&other)));
        assert!(app.awaits_snapshot(sid));
        assert!(handle.take_events().is_empty());

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

        let mut forged = real.clone();
        let impostor = Identity::generate();
        forged.profiles.get_mut(&owner.id()).unwrap().identity = Some(impostor.public());
        assert!(!apply_snapshot(&handle, sid, snapshot_of(&forged)));
        assert_eq!(app.servers.lock().unwrap()[&sid].identity_of(owner.id()), Some(owner.identity.public()));

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
        let snapshot = snapshot_of(&real);
        let mut provisional = commy_core::Server::new("s", me.id());
        provisional.id = sid;
        let mut placeholder = ServerState::new(provisional);
        placeholder.profiles.insert(me.id(), me.profile.clone());
        let app = app_with(&me, placeholder);
        app.set_awaiting_snapshot(sid, true);
        invited_by(&app, sid, &owner);
        let handle = TestHandle::new(app.clone());
        assert!(apply_snapshot(&handle, sid, snapshot));
        assert!(!app.awaits_snapshot(sid));
        assert!(app.servers.lock().unwrap()[&sid].server.is_member(me.id()));
        let store = AppStore(handle);
        let (user, identity) = store.local_identity(sid).expect("a member, so catch-up may run");
        assert_eq!((user, identity.public()), (me.id(), me.identity.public()));

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
        let mut ours = placeholder(&me, sid);
        ours.server.members.iter_mut().for_each(|m| m.joined_at = Utc::now() - chrono::Duration::days(30));
        join(&mut ours, &owner);
        let dir = temp_dir("awaiting");
        for how in ["session", "restart", "startup"] {
            let app = app_with(&me, ours.clone());
            let handle = TestHandle::new(app.clone());
            app.set_awaiting_snapshot(sid, true);
            invited_by(&app, sid, &owner);
            match how {
                "session" => after_session(&handle, sid, Some(&SessionReport::default())),
                "restart" => {
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
        let mut ours = placeholder(&me, sid);
        let planted = UserProfile { identity: Some(Identity::generate().public()), ..owner.profile.clone() };
        ours.apply(ServerEvent::MemberJoin { server: sid, profile: planted }).unwrap();
        let owner_join = ServerEvent::MemberJoin { server: sid, profile: owner.profile.clone() };
        assert_eq!(ours.apply(owner_join), Err(StoreError::IdentityMismatch));
        join(&mut ours, &friend);
        let app = app_with(&me, ours);
        app.set_awaiting_snapshot(sid, true);
        invited_by(&app, sid, &owner);
        let handle = TestHandle::new(app.clone());

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
        invited_by(&app, sid, &owner);
        let handle = TestHandle::new(app.clone());

        assert!(apply_snapshot(&handle, sid, snapshot_of(&real)));
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
        invited_by(&app, sid, &owner);
        let handle = TestHandle::new(app.clone());
        let theirs = app_with(&owner, real);
        let deliver = |sent: Vec<WirePayload>| {
            for _ in sent.iter().filter(|p| matches!(p, WirePayload::SyncRequest)) {
                for reply in sync_replies(&theirs, sid) {
                    if matches!(reply, WirePayload::SyncResponse { .. }) {
                        apply_snapshot(&handle, sid, reply);
                    }
                }
            }
        };
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
        invited_by(&app, sid, &owner);
        assert!(snapshot_for(&app, sid).is_none(), "still waiting for ours");
        after_session(&handle, sid, Some(&SessionReport::default()));
        assert!(snapshot_for(&app, sid).is_none(), "after a session");
        let dir = temp_dir("handed-out");
        crate::persist::save_awaiting(&dir, &app.awaiting_snapshot.lock().unwrap()).unwrap();
        *app.awaiting_snapshot.lock().unwrap() = crate::persist::load_awaiting(&dir);
        std::fs::remove_dir_all(&dir).ok();
        assert!(snapshot_for(&app, sid).is_none(), "after a restart");
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
            matches!(payload, WirePayload::Signed(SignedEvent { action, event: ServerEvent::MemberJoin { profile, .. } })
                if profile.id == who.id() && profile.identity == Some(who.identity.public()) && action.by == who.id())
        };
        let from_owner = replies_of(&owner, false);
        assert!(matches!(from_owner.as_slice(), [WirePayload::SyncResponse { .. }, join] if joined(join, &owner)));
        let from_friend = replies_of(&friend, true);
        assert!(matches!(from_friend.as_slice(), [join] if joined(join, &friend)));
        assert!(sync_replies(&app_with(&owner, real.clone()), ServerId::new()).is_empty());

        let mut trimmed = from_owner[0].clone();
        if let WirePayload::SyncResponse { members, profiles, .. } = &mut trimmed {
            members.retain(|m| m.user != friend.id());
            profiles.retain(|p| p.id != friend.id());
        }
        commy_net::protocol::sign_sync_response(&mut trimmed, Utc::now().timestamp_millis(), &owner.identity);
        let WirePayload::Signed(SignedEvent { event: friend_join, .. }) = from_friend[0].clone() else { panic!("not an event") };
        for join_first in [true, false] {
            let app = app_with(&me, placeholder(&me, sid));
            app.set_awaiting_snapshot(sid, true);
            invited_by(&app, sid, &owner);
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
    fn pins_announced_before_a_snapshot_survive_it_and_stay_out_of_ours() {
        let owner = Member::new("owner");
        let me = Member::new("me");
        let (mut state, channel) = server(&owner);
        join(&mut state, &me);
        let sid = state.server.id;
        let post = owner.post(&state, channel, "pinned early");
        state.apply(ServerEvent::MessageCreate { message: post.clone() }).unwrap();
        let pin = ServerEvent::MessagePin { server: sid, channel, message: post.id, by: owner.id(), at: chrono::Utc::now(), pinned: true };
        state.apply(pin.clone()).unwrap();
        let snapshot = snapshot_for(&app_with(&owner, state), sid).unwrap();
        let WirePayload::SyncResponse { server, .. } = &snapshot else { panic!("not a snapshot") };
        assert!(server.pins.is_empty());
        let app = app_with(&me, placeholder(&me, sid));
        app.set_awaiting_snapshot(sid, true);
        invited_by(&app, sid, &owner);
        let early = app.servers.lock().unwrap().get_mut(&sid).unwrap().apply(ServerEvent::ChannelCreate {
            server: sid,
            channel: commy_core::Channel { id: channel, ..commy_core::Channel::new("general", commy_core::ChannelKind::Text, 0) },
        });
        assert!(early.is_ok());
        app.servers.lock().unwrap().get_mut(&sid).unwrap().apply(pin).unwrap();
        assert!(apply_snapshot(&TestHandle::new(app.clone()), sid, snapshot));
        let state = app.servers.lock().unwrap()[&sid].clone();
        assert!(state.server.is_pinned(post.id));
        assert!(state.has_message(post.id));
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
        assert!(snapshot_for(&app, sid).is_none(), "nothing to sign it with yet");
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
        let sent = |app: &App| match snapshot_for(app, sid) {
            Some(WirePayload::SyncResponse { recent_messages, .. }) => recent_messages.into_iter().map(|m| m.id).collect::<HashSet<_>>(),
            Some(_) => panic!("not a snapshot"),
            None => HashSet::new(),
        };
        app.hold_key(Instant::now() + Duration::from_secs(120));
        assert_eq!(sent(&app), HashSet::new());
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
        app.hold_key(Instant::now());
        assert!(store.local_identity(sid).is_some());
    }

    #[test]
    fn servers_joined_before_the_upgrade_still_wait_for_their_snapshot() {
        let me = Member::new("member");
        let owner = Member::new("owner");
        let (ours, inviter) = ([1u8; 32], [2u8; 32]);
        let (placeholder, channel) = server(&me);
        assert!(joined_placeholder(&placeholder, me.id(), Some(&[inviter]), ours));

        assert!(!joined_placeholder(&placeholder, me.id(), Some(&[ours]), ours));
        assert!(!joined_placeholder(&placeholder, me.id(), Some(&[inviter, ours]), ours));
        assert!(!joined_placeholder(&placeholder, me.id(), Some(&[]), ours));
        assert!(!joined_placeholder(&placeholder, me.id(), None, ours));

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

    #[test]
    fn an_owner_with_history_stops_waiting_for_a_snapshot_nobody_else_can_sign() {
        let me = Member::new("owner");
        let friend = Member::new("friend");
        let (mut real, channel) = server(&me);
        join(&mut real, &friend);
        let sid = real.server.id;
        let mut hello = Message::new(channel, friend.id(), "hi");
        hello.sign(sid, &friend.identity);
        real.apply(ServerEvent::MessageCreate { message: hello }).unwrap();
        let invite = |owner: &Member| {
            let mut ticket = commy_net::ServerTicket::new(sid, "s", [7; 32], vec![]);
            ticket.owner = Some((owner.id(), owner.identity.public()));
            HashMap::from([(sid, ticket.encode())])
        };

        let app = app_with(&me, real.clone());
        app.set_awaiting_snapshot(sid, true);
        release_own_servers(&app, &HashMap::new());
        assert!(app.awaits_snapshot(sid));
        release_own_servers(&app, &invite(&friend));
        assert!(app.awaits_snapshot(sid));
        release_own_servers(&app, &invite(&me));
        assert!(!app.awaits_snapshot(sid));

        let mut ours = placeholder(&me, sid);
        let own_channel = ours.server.channels[0].id;
        let mut note = Message::new(own_channel, me.id(), "only me so far");
        note.sign(sid, &me.identity);
        ours.apply(ServerEvent::MessageCreate { message: note }).unwrap();
        let app = app_with(&me, ours);
        app.set_awaiting_snapshot(sid, true);
        release_own_servers(&app, &invite(&me));
        assert!(app.awaits_snapshot(sid));
    }

    #[tokio::test]
    async fn startup_marks_servers_joined_from_someone_elses_invite() {
        let (mut inviter, _, _, _) = commy_net::CommyNode::spawn(None).await.unwrap();
        let me = Member::new("member");
        let (joined, _) = server(&me);
        let (created, _) = server(&me);
        let (sid, created_id) = (joined.server.id, created.server.id);
        let ticket = tokio::time::timeout(Duration::from_secs(30), inviter.host_server(sid, "s", None)).await.unwrap().unwrap();
        let theirs = inviter.node_id_bytes();
        assert_eq!(invite_nodes(&ticket.encode()), Some(vec![theirs]));
        assert_eq!(invite_nodes("not an invite"), None);

        let app = app_with(&me, joined);
        app.servers.lock().unwrap().insert(created_id, created);
        let mut own = ticket.clone();
        own.server_id = created_id;
        let invites = HashMap::from([(sid, ticket.encode()), (created_id, own.encode())]);
        mark_joined_placeholders(&app, &invites, theirs);
        assert!(app.awaiting_snapshot.lock().unwrap().is_empty());
        mark_joined_placeholders(&app, &HashMap::from([(sid, ticket.encode())]), [9; 32]);
        assert_eq!(*app.awaiting_snapshot.lock().unwrap(), HashSet::from([sid]));
        assert!(app.awaits_snapshot(sid));
        inviter.shutdown().await.ok();
    }
}
