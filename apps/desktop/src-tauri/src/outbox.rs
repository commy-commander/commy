//! Offline queue. Server sends made while no member was connected wait
//! here: our own messages as pending ids, which the next catch-up with a
//! neighbor delivers, and other lasting events, re-broadcast when a
//! neighbor connects. DMs to a partner who is offline wait too and are
//! retried in order. Everything is kept in `<data>/outbox.json`.

use crate::catchup::AppAccess;
use crate::safety::{self, FriendSignal};
use crate::state::App;
use commy_core::catchup::{day_number, first_day};
use commy_core::{
    Identity, IdentityKey, Message, MessageId, Presence, ServerEvent, ServerId, ServerState, TombstoneProof, UserId,
    UserProfile,
};
use commy_net::node::IncomingDm;
use commy_net::{DmEnvelope, WirePayload};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::Path;
use std::time::{Duration, Instant};
use tauri::Manager;

pub const OUTBOX_FILE: &str = "outbox.json";

/// Most events kept per server while offline; the oldest go first.
pub const MAX_QUEUED: usize = 500;

/// First DM retry delay. It doubles after each failed try up to the max.
pub const DM_RETRY_FIRST: Duration = Duration::from_secs(30);
pub const DM_RETRY_MAX: Duration = Duration::from_secs(300);

/// Longest wait for one DM delivery attempt.
pub const DM_SEND_TIMEOUT: Duration = Duration::from_secs(20);

/// How often the DM outbox looks for retries that are due.
const DM_TICK: Duration = Duration::from_secs(5);

/// Sends that have not reached anyone yet.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Outbox {
    /// Our messages no member has received yet, per server.
    #[serde(default)]
    pending: HashMap<ServerId, Vec<MessageId>>,
    /// Other events to re-broadcast when a neighbor connects, oldest first.
    #[serde(default)]
    queued: HashMap<ServerId, Vec<ServerEvent>>,
    /// Our DMs not delivered yet, per partner, oldest first.
    #[serde(default)]
    dms: HashMap<UserId, Vec<MessageId>>,
}

/// Live state that means nothing later, so it is never queued.
pub fn is_transient(event: &ServerEvent) -> bool {
    matches!(
        event,
        ServerEvent::TypingStart { .. }
            | ServerEvent::VoiceState { .. }
            | ServerEvent::ScreenShare { .. }
            | ServerEvent::CameraState { .. }
            | ServerEvent::SoundboardPlay { .. }
            | ServerEvent::VoiceModerate { .. }
    )
}

/// True when `new` makes a queued `old` pointless: a newer profile or
/// presence of the same user.
fn supersedes(new: &ServerEvent, old: &ServerEvent) -> bool {
    match (new, old) {
        (ServerEvent::ProfileUpdate { profile: a }, ServerEvent::ProfileUpdate { profile: b }) => a.id == b.id,
        (ServerEvent::PresenceUpdate { user: a, .. }, ServerEvent::PresenceUpdate { user: b, .. }) => a == b,
        _ => false,
    }
}

impl Outbox {
    pub fn load(dir: &Path) -> Self {
        crate::persist::read_json(&dir.join(OUTBOX_FILE)).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        crate::persist::write_json(&dir.join(OUTBOX_FILE), self)
    }

    /// Records a local event after its broadcast. Our message that reached
    /// nobody becomes pending; any other lasting event that reached nobody
    /// is queued. A delete also drops its message from pending, delivered
    /// or not. Returns true when the outbox changed.
    pub fn note_local(&mut self, server: ServerId, event: &ServerEvent, delivered: bool) -> bool {
        let mut changed = false;
        if let ServerEvent::MessageDelete { message, .. } = event {
            changed |= self.clear_delivered(server, [*message]);
        }
        if delivered || is_transient(event) {
            return changed;
        }
        if let ServerEvent::MessageCreate { message } = event {
            let list = self.pending.entry(server).or_default();
            if list.contains(&message.id) {
                return changed;
            }
            list.push(message.id);
            return true;
        }
        let list = self.queued.entry(server).or_default();
        list.retain(|old| !supersedes(event, old));
        list.push(event.clone());
        if list.len() > MAX_QUEUED {
            let excess = list.len() - MAX_QUEUED;
            list.drain(..excess);
        }
        true
    }

    #[cfg(test)]
    pub fn is_pending(&self, server: ServerId, id: MessageId) -> bool {
        self.pending.get(&server).is_some_and(|list| list.contains(&id))
    }

    pub fn pending_ids(&self, server: ServerId) -> HashSet<MessageId> {
        self.pending.get(&server).map(|list| list.iter().copied().collect()).unwrap_or_default()
    }

    pub fn has_pending(&self, server: ServerId) -> bool {
        self.pending.get(&server).is_some_and(|list| !list.is_empty())
    }

    /// Forgets pending messages a peer now holds. True when any was pending.
    pub fn clear_delivered(&mut self, server: ServerId, ids: impl IntoIterator<Item = MessageId>) -> bool {
        let Some(list) = self.pending.get_mut(&server) else { return false };
        let done: HashSet<MessageId> = ids.into_iter().collect();
        let before = list.len();
        list.retain(|id| !done.contains(id));
        let changed = list.len() != before;
        if list.is_empty() {
            self.pending.remove(&server);
        }
        changed
    }

    /// Drops pending ids catch-up can never deliver: messages we no longer
    /// hold (deleted or expired) and ones older than its window.
    pub fn prune(&mut self, state: &ServerState, now: chrono::DateTime<chrono::Utc>) -> bool {
        let server = state.server.id;
        let Some(list) = self.pending.get_mut(&server) else { return false };
        let first = first_day(now);
        let before = list.len();
        list.retain(|id| state.message(*id).is_some_and(|m| day_number(m.created_at) >= first));
        let changed = list.len() != before;
        if list.is_empty() {
            self.pending.remove(&server);
        }
        changed
    }

    /// Events queued for a server, oldest first, removed from the queue.
    pub fn take_queued(&mut self, server: ServerId) -> Vec<ServerEvent> {
        self.queued.remove(&server).unwrap_or_default()
    }

    /// Puts events that still could not go out back in front of the queue.
    pub fn requeue(&mut self, server: ServerId, mut events: Vec<ServerEvent>) {
        if events.is_empty() {
            return;
        }
        events.extend(self.queued.remove(&server).unwrap_or_default());
        if events.len() > MAX_QUEUED {
            let excess = events.len() - MAX_QUEUED;
            events.drain(..excess);
        }
        self.queued.insert(server, events);
    }

    /// Everything kept for a server we left. True when anything was.
    pub fn forget_server(&mut self, server: ServerId) -> bool {
        let pending = self.pending.remove(&server).is_some();
        self.queued.remove(&server).is_some() || pending
    }

    /// Keeps a DM for a retry. False when it was kept already.
    pub fn add_dm(&mut self, partner: UserId, id: MessageId) -> bool {
        let list = self.dms.entry(partner).or_default();
        if list.contains(&id) {
            return false;
        }
        list.push(id);
        true
    }

    pub fn has_dms(&self, partner: UserId) -> bool {
        self.dms.get(&partner).is_some_and(|list| !list.is_empty())
    }

    /// A partner's waiting DMs, oldest first.
    pub fn dm_queue(&self, partner: UserId) -> Vec<MessageId> {
        self.dms.get(&partner).cloned().unwrap_or_default()
    }

    pub fn pending_dms(&self, partner: UserId) -> HashSet<MessageId> {
        self.dms.get(&partner).map(|list| list.iter().copied().collect()).unwrap_or_default()
    }

    pub fn dm_partners(&self) -> Vec<UserId> {
        self.dms.iter().filter(|(_, list)| !list.is_empty()).map(|(user, _)| *user).collect()
    }

    /// Forgets a DM that was delivered (or no longer exists).
    pub fn dm_done(&mut self, partner: UserId, id: MessageId) -> bool {
        let Some(list) = self.dms.get_mut(&partner) else { return false };
        let before = list.len();
        list.retain(|m| *m != id);
        let changed = list.len() != before;
        if list.is_empty() {
            self.dms.remove(&partner);
        }
        changed
    }
}

/// When each partner's waiting DMs are tried next.
#[derive(Debug, Default)]
pub struct DmBackoff {
    next: HashMap<UserId, (Instant, Duration)>,
}

impl DmBackoff {
    pub fn due(&self, partner: UserId, now: Instant) -> bool {
        self.next.get(&partner).is_none_or(|(at, _)| now >= *at)
    }

    /// First failure for a partner: try again after [`DM_RETRY_FIRST`].
    /// An existing schedule is kept.
    pub fn schedule(&mut self, partner: UserId, now: Instant) {
        self.next.entry(partner).or_insert((now + DM_RETRY_FIRST, DM_RETRY_FIRST));
    }

    /// A try failed: wait twice as long as last time, up to [`DM_RETRY_MAX`].
    pub fn failed(&mut self, partner: UserId, now: Instant) {
        let delay = self
            .next
            .get(&partner)
            .map_or(DM_RETRY_FIRST, |(_, last)| (*last * 2).min(DM_RETRY_MAX));
        self.next.insert(partner, (now + delay, delay));
    }

    pub fn succeeded(&mut self, partner: UserId) {
        self.next.remove(&partner);
    }

    /// Makes a partner due now; a failure still backs off from the last delay.
    pub fn wake(&mut self, partner: UserId, now: Instant) {
        if let Some((at, _)) = self.next.get_mut(&partner) {
            *at = now;
        }
    }
}

/// The user an event shows as online, if it is about presence at all.
pub fn online_user(event: &ServerEvent) -> Option<(UserId, Presence)> {
    match event {
        ServerEvent::PresenceUpdate { user, presence } => Some((*user, *presence)),
        ServerEvent::ProfileUpdate { profile } | ServerEvent::MemberJoin { profile, .. } => {
            Some((profile.id, profile.presence))
        }
        _ => None,
    }
}

/// True when every key we pinned for a sender, if any, is `key`.
pub fn key_matches_pins(key: IdentityKey, pins: impl IntoIterator<Item = Option<IdentityKey>>) -> bool {
    pins.into_iter().flatten().all(|pinned| pinned == key)
}

/// Whether an incoming DM is signed by its sender with the key we hold
/// for them: the one pinned for the contact and in any server we share.
pub fn dm_sender_trusted(app: &App, envelope: &DmEnvelope) -> bool {
    let Some(key) = envelope.sender.identity.filter(|_| envelope.signed_by_sender()) else {
        return false;
    };
    let user = envelope.sender.id;
    let mut pins = vec![app.dms.lock().unwrap().pinned_key(user)];
    pins.extend(app.servers.lock().unwrap().values().map(|s| s.identity_of(user)));
    key_matches_pins(key, pins)
}

/// Checks an incoming DM before anything else looks at it. It must be
/// addressed to us, name the node it came from, and be signed by its
/// sender with the key we hold for them; the signature covers that node,
/// so a copy sent on from another node is dropped. Returns the sender's
/// profile to cache as a contact, with that node as theirs, or None to
/// drop the DM.
pub fn screen_dm(app: &App, dm: &IncomingDm) -> Option<UserProfile> {
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id)?;
    let envelope = &dm.envelope;
    if envelope.message.to != me || !envelope.sent_from(&dm.node) || !dm_sender_trusted(app, envelope) {
        return None;
    }
    let mut sender = envelope.sender.clone();
    // The profile's own node is not signed; the one it came from is.
    sender.node = Some(data_encoding::HEXLOWER.encode(&dm.node));
    Some(sender)
}

/// What became of an incoming DM, for the parts that need the network.
#[derive(Debug, PartialEq)]
pub enum Received {
    /// Not for us, a replayed friend signal, or filtered out.
    Dropped,
    /// Stored, or a friend signal applied with nothing to answer.
    Taken,
    /// A report about a member, for the caller to check and keep.
    Report { from: UserId, content: String },
    /// A friend signal applied that asks for this answer.
    Reply { to: UserId, signal: FriendSignal },
}

/// Takes one incoming DM: screens it (see [`screen_dm`]), applies a friend
/// signal unless it was seen before, and files a message in the inbox,
/// the requests or spam, telling the webview.
pub fn receive_dm<A: AppAccess>(access: &A, dm: IncomingDm) -> Received {
    let app = access.app();
    let Some(sender) = screen_dm(app, &dm) else {
        tracing::warn!("dropped a DM not for us, not from the node it names, or not signed by its sender");
        return Received::Dropped;
    };
    let message = dm.envelope.message;
    let partner = sender.id;
    if message.content.starts_with(safety::REPORT_PREFIX) {
        return Received::Report { from: partner, content: message.content };
    }
    if let Some(signal) = FriendSignal::decode(&message.content) {
        // A signed signal sent to us again must not undo a later one.
        let fresh = safety::note_friend_signal(&mut app.safety.lock().unwrap(), partner, message.id, message.created_at);
        if !fresh {
            tracing::debug!("ignored a friend signal from {partner} seen before");
            return Received::Dropped;
        }
        let reply = safety::apply_friend_signal(&mut app.safety.lock().unwrap(), partner, signal);
        app.dms.lock().unwrap().remember_contact(sender.clone());
        app.persist_dms();
        app.persist_safety();
        access.emit(
            "friends-changed",
            serde_json::json!({ "user": partner.to_string(), "name": sender.visible_name(), "signal": signal }),
        );
        return reply.map_or(Received::Taken, |signal| Received::Reply { to: partner, signal });
    }
    let Some(me) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { return Received::Dropped };
    let verdict = {
        let shares_server = app
            .servers
            .lock()
            .unwrap()
            .values()
            .any(|s| s.server.members.iter().any(|m| m.user == partner));
        let already_talking = app.dms.lock().unwrap().has_sent_to(me, partner);
        let safety = app.safety.lock().unwrap();
        let from = safety::Sender {
            blocked: safety.is_blocked(partner),
            accepted: already_talking || safety.accepted.contains(&partner) || safety.friends.contains(&partner),
            shares_server,
        };
        app.dm_filter.lock().unwrap().classify(
            safety.dm_policy,
            safety.spam_filter,
            &from,
            partner,
            &message.content,
            chrono::Utc::now().timestamp(),
        )
    };
    if verdict == safety::Verdict::Drop {
        tracing::debug!("dropped a DM from {partner}");
        return Received::Dropped;
    }
    if verdict == safety::Verdict::Spam && app.safety.lock().unwrap().spam.insert(partner) {
        app.persist_safety();
    }
    let notice = serde_json::json!({
        "user": partner.to_string(),
        "name": sender.visible_name(),
        "content": message.content.clone(),
        "request": verdict != safety::Verdict::Inbox,
        "spam": verdict == safety::Verdict::Spam,
    });
    let inserted = {
        let mut dms = app.dms.lock().unwrap();
        dms.remember_contact(sender);
        dms.insert(partner, message)
    };
    if inserted {
        app.persist_dms();
        access.emit("dm-changed", partner.to_string().into());
        access.emit("dm-received", notice);
    }
    Received::Taken
}

// ---------- Network glue ----------

/// Whether a local event goes out now: someone is connected, and our key
/// is settled. Until then whatever we sign might carry a key about to be
/// replaced, so it waits here and is signed again first if it is. Live
/// state is never kept, so it goes out regardless.
pub fn may_send(app: &App, neighbors: usize, event: &ServerEvent) -> bool {
    neighbors > 0 && (app.key_settled() || is_transient(event))
}

/// Broadcasts an event we already applied locally. When it may not go
/// out yet (see [`may_send`]) or the send fails, it waits here (see
/// [`Outbox::note_local`]). Returns true when it went out.
pub async fn broadcast_local(app: &App, server: ServerId, event: ServerEvent) -> bool {
    let delivered = {
        let net = app.net.lock().await;
        match net.as_ref() {
            Some(node) if may_send(app, node.neighbor_count(server), &event) => {
                match node.broadcast(server, WirePayload::Event(event.clone())).await {
                    Ok(()) => true,
                    Err(e) => {
                        tracing::warn!("broadcast failed, keeping it for later: {e}");
                        false
                    }
                }
            }
            _ => false,
        }
    };
    if app.outbox.lock().unwrap().note_local(server, &event, delivered) {
        app.persist_outbox();
    }
    delivered
}

/// Re-broadcasts a server's queued events, oldest first, now that a
/// neighbor is connected. An event leaves the queue only once it went out
/// (see [`may_send`]); the rest stay queued for the next time.
pub async fn flush_queued(app: &App, server: ServerId) {
    if !app.key_settled() {
        return;
    }
    let events = app.outbox.lock().unwrap().take_queued(server);
    if events.is_empty() {
        return;
    }
    let total = events.len();
    let mut left = Vec::new();
    for (i, event) in events.iter().enumerate() {
        // Locked per event so other sends are not held up behind a long queue.
        let sent = {
            let net = app.net.lock().await;
            match net.as_ref() {
                // A neighbor may have left since the event that woke us.
                Some(node) if may_send(app, node.neighbor_count(server), event) => {
                    node.broadcast(server, WirePayload::Event(event.clone())).await
                }
                Some(_) => Err(anyhow::anyhow!("no neighbor is connected")),
                None => Err(anyhow::anyhow!("network is not ready")),
            }
        };
        if let Err(e) = sent {
            tracing::warn!("re-sending queued events stopped: {e}");
            left = events[i..].to_vec();
            break;
        }
    }
    tracing::info!("re-sent {} of {total} queued events", total - left.len());
    app.outbox.lock().unwrap().requeue(server, left);
    // Always saved: another save while the events were out of the queue
    // left them out of the file.
    app.persist_outbox();
}

/// Signs again with `identity` what of ours in `state` it does not sign:
/// our pending messages and our kept delete proofs, made while our key
/// was held back, when another key settled. Ids and contents stay. True
/// when any changed.
pub fn resign_ours(state: &mut ServerState, me: UserId, pending: &HashSet<MessageId>, identity: &Identity) -> bool {
    let server = state.server.id;
    let key = identity.public();
    let stale = |m: &Message| m.author == me && pending.contains(&m.id) && !m.verify(server, &key);
    let mut snapshot = state.snapshot();
    let mut changed = false;
    for message in snapshot.messages.iter_mut().flat_map(|(_, list)| list.iter_mut()).filter(|m| stale(m)) {
        message.sign(server, identity);
        changed = true;
    }
    // Catch-up relays the proofs as kept here.
    for proof in snapshot.tombstone_proofs.iter_mut().filter(|p| p.by == me && !p.verify(server, &key)) {
        *proof = TombstoneProof::sign(server, proof.channel, proof.message, me, proof.at, identity);
        changed = true;
    }
    if changed {
        // Rebuilt from its saved image; live state the image leaves out stays.
        let mut fresh = ServerState::from_snapshot(snapshot);
        fresh.profiles = std::mem::take(&mut state.profiles);
        fresh.voice_states = std::mem::take(&mut state.voice_states);
        fresh.screen_shares = std::mem::take(&mut state.screen_shares);
        fresh.cameras = std::mem::take(&mut state.cameras);
        *state = fresh;
    }
    changed
}

impl Outbox {
    /// Signs again with `identity` our queued edits and deletes in
    /// `state`'s server that it does not sign, as [`resign_ours`] does
    /// for what the state keeps. True when any changed.
    pub fn resign_queued(&mut self, state: &ServerState, me: UserId, identity: &Identity) -> bool {
        let server = state.server.id;
        let key = identity.public();
        let Some(list) = self.queued.get_mut(&server) else { return false };
        let mut changed = false;
        for event in list.iter_mut() {
            let resigned = match event {
                ServerEvent::MessageEdit { message, content, edited_at, sig, .. } => state
                    .message(*message)
                    .filter(|m| m.author == me && !m.verify_edit(server, content, *edited_at, sig, &key))
                    .map(|m| m.edit_event(server, content.clone(), *edited_at, identity)),
                ServerEvent::MessageDelete { channel, message, by, at, sig } => {
                    let proof = TombstoneProof { channel: *channel, message: *message, by: *by, at: *at, sig: *sig };
                    (*by == me && !proof.verify(server, &key))
                        .then(|| TombstoneProof::sign(server, *channel, *message, me, *at, identity).into_event())
                }
                _ => None,
            };
            if let Some(resigned) = resigned {
                *event = resigned;
                changed = true;
            }
        }
        changed
    }
}

/// Once our key settled: signs again with it whatever of ours still waits
/// here that another key signed, see [`resign_ours`] and
/// [`Outbox::resign_queued`]. Nothing of it reached anyone, since nothing
/// went out while the key was held back.
pub fn resign_held(app: &App) {
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let (Some(me), Some(identity)) = (me, app.identity()) else { return };
    let servers: Vec<ServerId> = app.servers.lock().unwrap().keys().copied().collect();
    for server in servers {
        let pending = app.outbox.lock().unwrap().pending_ids(server);
        let (kept, queued) = {
            let mut servers = app.servers.lock().unwrap();
            let Some(state) = servers.get_mut(&server) else { continue };
            let kept = resign_ours(state, me, &pending, &identity);
            (kept, app.outbox.lock().unwrap().resign_queued(state, me, &identity))
        };
        if kept {
            tracing::info!("signed what we wrote in {server} meanwhile again with our settled key");
            app.persist_server(server);
        }
        if queued {
            app.persist_outbox();
        }
    }
}

/// Sends one DM now. False when the partner has no known address, the
/// network is not up, delivery failed, or our key may still change (the
/// partner would pin the one we are about to drop).
pub async fn try_send_dm(app: &App, partner: UserId, envelope: &DmEnvelope) -> bool {
    if !app.key_settled() {
        return false;
    }
    let Some(hex) = crate::node_hex_for_user(app, partner) else { return false };
    let send = {
        let net = app.net.lock().await;
        match net.as_ref() {
            Some(node) => node.send_dm(&hex, envelope),
            None => return false,
        }
    };
    match tokio::time::timeout(DM_SEND_TIMEOUT, send).await {
        Ok(Ok(())) => true,
        Ok(Err(e)) => {
            tracing::debug!("dm not delivered yet: {e:#}");
            false
        }
        Err(_) => false,
    }
}

/// Keeps a DM we could not deliver for the outbox task to retry.
pub fn queue_dm(app: &App, partner: UserId, id: MessageId) {
    if app.outbox.lock().unwrap().add_dm(partner, id) {
        app.dm_backoff.lock().unwrap().schedule(partner, Instant::now());
        app.persist_outbox();
    }
}

/// Retries a partner's waiting DMs right away.
pub fn wake_dm(app: &App, partner: UserId) {
    if !app.outbox.lock().unwrap().has_dms(partner) {
        return;
    }
    app.dm_backoff.lock().unwrap().wake(partner, Instant::now());
    app.dm_wake.notify_one();
}

/// Retries every partner's waiting DMs right away.
pub fn wake_all_dms(app: &App) {
    let partners = app.outbox.lock().unwrap().dm_partners();
    for partner in partners {
        wake_dm(app, partner);
    }
}

/// A node became a gossip neighbor: retry DMs to whoever is on it.
pub fn wake_dms_for_node(app: &App, peer: [u8; 32]) {
    let hex = data_encoding::HEXLOWER.encode(&peer);
    let on_node = |p: &commy_core::UserProfile| p.node.as_deref() == Some(hex.as_str());
    let mut users: HashSet<UserId> = app
        .servers
        .lock()
        .unwrap()
        .values()
        .flat_map(|s| s.profiles.values().filter(|p| on_node(p)).map(|p| p.id))
        .collect();
    users.extend(app.dms.lock().unwrap().contacts.values().filter(|p| on_node(p)).map(|p| p.id));
    for user in users {
        wake_dm(app, user);
    }
}

/// Retries waiting DMs per partner, in order: every 30 seconds at first,
/// backing off to 5 minutes, and at once when [`wake_dm`] is called.
pub async fn run_dm_outbox(handle: tauri::AppHandle) {
    loop {
        {
            let app = handle.state::<App>();
            let _ = tokio::time::timeout(DM_TICK, app.dm_wake.notified()).await;
        }
        let app = handle.state::<App>();
        let now = Instant::now();
        let partners = app.outbox.lock().unwrap().dm_partners();
        let due: Vec<UserId> = {
            let backoff = app.dm_backoff.lock().unwrap();
            partners.into_iter().filter(|p| backoff.due(*p, now)).collect()
        };
        for partner in due {
            deliver_dms(&handle, partner).await;
        }
    }
}

/// Tries a partner's waiting DMs in order, stopping at the first failure.
async fn deliver_dms(handle: &tauri::AppHandle, partner: UserId) {
    let app = handle.state::<App>();
    deliver_dms_with(handle, partner, |envelope| {
        let app = &app;
        async move { try_send_dm(app, partner, &envelope).await }
    })
    .await;
}

/// [`deliver_dms`] over any transport. A delivery tells the webview with
/// `dm-sent`, not `dm-changed`: nothing arrived, ours went out.
async fn deliver_dms_with<A, F, Fut>(access: &A, partner: UserId, send: F)
where
    A: AppAccess,
    F: Fn(DmEnvelope) -> Fut,
    Fut: Future<Output = bool>,
{
    let app = access.app();
    let node = app.node_hex.lock().unwrap().clone();
    let (Ok(me), Some(identity), Some(node)) = (crate::current_profile(app), app.identity(), node) else { return };
    if app.safety.lock().unwrap().is_blocked(partner) {
        return;
    }
    let mut changed = false;
    let mut failed = false;
    let queue = app.outbox.lock().unwrap().dm_queue(partner);
    for id in queue {
        let found = app.dms.lock().unwrap().conversation(partner).iter().find(|m| m.id == id).cloned();
        let Some(mut message) = found else {
            // The conversation was removed; nothing left to send.
            changed |= app.outbox.lock().unwrap().dm_done(partner, id);
            continue;
        };
        // Signed again with the current key and node, either may have changed.
        message.sign(&node, &identity);
        let envelope = DmEnvelope { message, sender: me.clone() };
        if !send(envelope).await {
            failed = true;
            break;
        }
        changed |= app.outbox.lock().unwrap().dm_done(partner, id);
    }
    {
        let mut backoff = app.dm_backoff.lock().unwrap();
        if failed {
            backoff.failed(partner, Instant::now());
        } else {
            backoff.succeeded(partner);
        }
    }
    if changed {
        app.persist_outbox();
        access.emit("dm-sent", partner.to_string().into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catchup::TestHandle;
    use commy_core::{DmMessage, Identity, Message, Server, TombstoneProof};
    use std::sync::{Arc, Mutex};

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos();
        let dir = std::env::temp_dir().join("commy-outbox-tests").join(format!("{tag}-{nanos:x}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A server whose owner holds a key, plus a signed message of theirs.
    fn keyed_server() -> (ServerState, Identity, Message) {
        let identity = Identity::generate();
        let mut owner = UserProfile::new("owner");
        owner.identity = Some(identity.public());
        let server = Server::new("s", owner.id);
        let channel = server.channels[0].id;
        let mut state = ServerState::new(server);
        state.profiles.insert(owner.id, owner.clone());
        let mut message = Message::new(channel, owner.id, "hello");
        message.sign(state.server.id, &identity);
        state.apply(ServerEvent::MessageCreate { message: message.clone() }).unwrap();
        (state, identity, message)
    }

    fn rename(server: ServerId, name: &str) -> ServerEvent {
        ServerEvent::ServerRename { server, name: name.into() }
    }

    #[test]
    fn undelivered_messages_pend_and_other_events_queue() {
        let (state, identity, message) = keyed_server();
        let sid = state.server.id;
        let mut outbox = Outbox::default();
        let create = ServerEvent::MessageCreate { message: message.clone() };

        assert!(!outbox.note_local(sid, &create, true), "delivered sends leave no trace");
        assert!(outbox.note_local(sid, &create, false));
        assert!(!outbox.note_local(sid, &create, false), "pending once");
        assert!(outbox.is_pending(sid, message.id) && outbox.has_pending(sid));
        assert_eq!(outbox.pending_ids(sid), HashSet::from([message.id]));
        assert!(outbox.take_queued(sid).is_empty(), "messages travel by catch-up");

        let edit = message.edit_event(sid, "edited", chrono::Utc::now(), &identity);
        assert!(outbox.note_local(sid, &rename(sid, "a"), false));
        assert!(outbox.note_local(sid, &edit, false));
        assert_eq!(outbox.take_queued(sid), vec![rename(sid, "a"), edit]);
        assert!(outbox.take_queued(sid).is_empty(), "taking clears the queue");
    }

    #[test]
    fn transient_events_are_never_queued() {
        let sid = ServerId::new();
        let user = UserId::new();
        let channel = commy_core::ChannelId::new();
        let mut outbox = Outbox::default();
        let transient = [
            ServerEvent::TypingStart { channel, user },
            ServerEvent::VoiceState { user, channel: Some(channel), muted: false, deafened: false },
            ServerEvent::ScreenShare { user, channel, active: true },
            ServerEvent::CameraState { user, channel, active: true },
            ServerEvent::SoundboardPlay { server: sid, channel, user, name: "boop".into() },
            ServerEvent::VoiceModerate { server: sid, user, muted: true, deafened: false },
        ];
        for event in &transient {
            assert!(is_transient(event));
            assert!(!outbox.note_local(sid, event, false));
        }
        assert_eq!(outbox, Outbox::default());
        assert!(!is_transient(&rename(sid, "x")));
    }

    #[test]
    fn newer_profile_replaces_a_queued_one() {
        let sid = ServerId::new();
        let mut outbox = Outbox::default();
        let mut me = UserProfile::new("me");
        let other = UserProfile::new("other");
        outbox.note_local(sid, &ServerEvent::ProfileUpdate { profile: me.clone() }, false);
        outbox.note_local(sid, &ServerEvent::ProfileUpdate { profile: other.clone() }, false);
        outbox.note_local(sid, &rename(sid, "x"), false);
        me.custom_status = Some("away".into());
        outbox.note_local(sid, &ServerEvent::ProfileUpdate { profile: me.clone() }, false);
        assert_eq!(
            outbox.take_queued(sid),
            vec![
                ServerEvent::ProfileUpdate { profile: other },
                rename(sid, "x"),
                ServerEvent::ProfileUpdate { profile: me },
            ]
        );
    }

    #[test]
    fn queue_keeps_the_newest_events() {
        let sid = ServerId::new();
        let mut outbox = Outbox::default();
        for i in 0..MAX_QUEUED + 3 {
            outbox.note_local(sid, &rename(sid, &i.to_string()), false);
        }
        let queued = outbox.take_queued(sid);
        assert_eq!(queued.len(), MAX_QUEUED);
        assert_eq!(queued[0], rename(sid, "3"));

        // Requeued events go back in front of anything queued meanwhile.
        outbox.note_local(sid, &rename(sid, "later"), false);
        outbox.requeue(sid, vec![rename(sid, "first"), rename(sid, "second")]);
        assert_eq!(
            outbox.take_queued(sid),
            vec![rename(sid, "first"), rename(sid, "second"), rename(sid, "later")]
        );
    }

    #[test]
    fn delivery_reports_and_deletes_clear_pending() {
        let (state, identity, message) = keyed_server();
        let sid = state.server.id;
        let mut outbox = Outbox::default();
        let (a, b) = (MessageId::new(), MessageId::new());
        for id in [a, b, message.id] {
            outbox.note_local(sid, &ServerEvent::MessageCreate { message: Message { id, ..message.clone() } }, false);
        }
        assert!(outbox.clear_delivered(sid, [a, MessageId::new()]));
        assert!(!outbox.clear_delivered(sid, [a]), "already cleared");
        assert!(!outbox.clear_delivered(ServerId::new(), [b]));
        assert_eq!(outbox.pending_ids(sid), HashSet::from([b, message.id]));

        // A delete drops the message from pending even when it went out.
        let delete = TombstoneProof::sign(sid, message.channel, b, message.author, chrono::Utc::now(), &identity).into_event();
        assert!(outbox.note_local(sid, &delete, true));
        assert_eq!(outbox.pending_ids(sid), HashSet::from([message.id]));
        assert!(outbox.clear_delivered(sid, [message.id]));
        assert!(!outbox.has_pending(sid));
        assert_eq!(outbox, Outbox::default(), "empty lists are dropped");
    }

    #[test]
    fn prune_drops_what_catch_up_cannot_deliver() {
        let (mut state, identity, message) = keyed_server();
        let sid = state.server.id;
        let mut outbox = Outbox::default();
        let mut old = Message::new(message.channel, message.author, "ancient");
        old.created_at = chrono::Utc::now() - chrono::Duration::days(45);
        old.sign(sid, &identity);
        state.apply(ServerEvent::MessageCreate { message: old.clone() }).unwrap();
        let gone = MessageId::new();
        for id in [message.id, old.id, gone] {
            outbox.note_local(sid, &ServerEvent::MessageCreate { message: Message { id, ..message.clone() } }, false);
        }
        assert!(outbox.prune(&state, chrono::Utc::now()));
        assert_eq!(outbox.pending_ids(sid), HashSet::from([message.id]));
        assert!(!outbox.prune(&state, chrono::Utc::now()));
    }

    #[test]
    fn outbox_survives_a_restart() {
        let dir = temp_dir("roundtrip");
        assert_eq!(Outbox::load(&dir), Outbox::default(), "missing file is empty");
        let (state, identity, message) = keyed_server();
        let sid = state.server.id;
        let mut outbox = Outbox::default();
        outbox.note_local(sid, &ServerEvent::MessageCreate { message: message.clone() }, false);
        outbox.note_local(sid, &message.edit_event(sid, "edited", chrono::Utc::now(), &identity), false);
        outbox.note_local(sid, &rename(sid, "renamed"), false);
        let mut profile = UserProfile::new("me");
        profile.identity = Some(identity.public());
        outbox.note_local(sid, &ServerEvent::ProfileUpdate { profile }, false);
        let partner = UserId::new();
        let dm = MessageId::new();
        outbox.add_dm(partner, dm);
        outbox.save(&dir).unwrap();

        let loaded = Outbox::load(&dir);
        assert_eq!(loaded, outbox);
        assert!(loaded.is_pending(sid, message.id));
        assert_eq!(loaded.dm_queue(partner), vec![dm]);
        assert_eq!(loaded.clone().take_queued(sid).len(), 3);

        std::fs::write(dir.join(OUTBOX_FILE), b"{broken").unwrap();
        assert_eq!(Outbox::load(&dir), Outbox::default(), "a corrupt file is ignored");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dms_wait_in_order_per_partner() {
        let mut outbox = Outbox::default();
        let (first, second) = (UserId::new(), UserId::new());
        let (one, two, three) = (MessageId::new(), MessageId::new(), MessageId::new());
        assert!(outbox.add_dm(first, one));
        assert!(outbox.add_dm(first, two));
        assert!(!outbox.add_dm(first, one));
        assert!(outbox.add_dm(second, three));
        assert_eq!(outbox.dm_queue(first), vec![one, two]);
        assert_eq!(outbox.pending_dms(first), HashSet::from([one, two]));
        let mut partners = outbox.dm_partners();
        partners.sort();
        let mut expected = vec![first, second];
        expected.sort();
        assert_eq!(partners, expected);

        assert!(outbox.dm_done(first, one));
        assert!(!outbox.dm_done(first, one));
        assert_eq!(outbox.dm_queue(first), vec![two]);
        assert!(outbox.dm_done(first, two));
        assert!(!outbox.has_dms(first) && outbox.has_dms(second));
        assert_eq!(outbox.dm_partners(), vec![second]);
    }

    #[test]
    fn leaving_a_server_forgets_its_outbox() {
        let (state, _, message) = keyed_server();
        let sid = state.server.id;
        let mut outbox = Outbox::default();
        outbox.note_local(sid, &ServerEvent::MessageCreate { message }, false);
        outbox.note_local(sid, &rename(sid, "x"), false);
        assert!(outbox.forget_server(sid));
        assert!(!outbox.forget_server(sid));
        assert_eq!(outbox, Outbox::default());
    }

    #[test]
    fn dm_retries_back_off_to_five_minutes() {
        let mut backoff = DmBackoff::default();
        let partner = UserId::new();
        let t0 = Instant::now();
        assert!(backoff.due(partner, t0), "nothing scheduled is due");

        backoff.schedule(partner, t0);
        assert!(!backoff.due(partner, t0 + Duration::from_secs(29)));
        assert!(backoff.due(partner, t0 + DM_RETRY_FIRST));
        // Scheduling again keeps the running schedule.
        backoff.schedule(partner, t0 + Duration::from_secs(10));
        assert!(backoff.due(partner, t0 + DM_RETRY_FIRST));

        let mut now = t0 + DM_RETRY_FIRST;
        let mut delays = Vec::new();
        for _ in 0..5 {
            backoff.failed(partner, now);
            let (at, delay) = backoff.next[&partner];
            assert_eq!(at, now + delay);
            delays.push(delay.as_secs());
            now = at;
        }
        assert_eq!(delays, vec![60, 120, 240, 300, 300]);

        // A wake makes a waiting partner due now; a failure keeps backing off.
        backoff.failed(partner, now);
        assert!(!backoff.due(partner, now + Duration::from_secs(1)));
        backoff.wake(partner, now);
        assert!(backoff.due(partner, now));
        backoff.failed(partner, now);
        assert_eq!(backoff.next[&partner].1, DM_RETRY_MAX);

        backoff.succeeded(partner);
        assert!(backoff.due(partner, now));
        backoff.failed(partner, now);
        assert_eq!(backoff.next[&partner].1, DM_RETRY_FIRST, "a fresh failure starts over");
        // Waking a partner with nothing scheduled changes nothing.
        let other = UserId::new();
        backoff.wake(other, now);
        assert!(backoff.due(other, now) && !backoff.next.contains_key(&other));
    }

    #[test]
    fn presence_events_name_their_user() {
        let mut profile = UserProfile::new("p");
        profile.presence = Presence::Online;
        let sid = ServerId::new();
        assert_eq!(
            online_user(&ServerEvent::ProfileUpdate { profile: profile.clone() }),
            Some((profile.id, Presence::Online))
        );
        assert_eq!(
            online_user(&ServerEvent::MemberJoin { server: sid, profile: profile.clone() }),
            Some((profile.id, Presence::Online))
        );
        assert_eq!(
            online_user(&ServerEvent::PresenceUpdate { user: profile.id, presence: Presence::Offline }),
            Some((profile.id, Presence::Offline))
        );
        assert_eq!(online_user(&rename(sid, "x")), None);
    }

    #[test]
    fn dm_keys_must_match_every_pin() {
        let key = Identity::generate().public();
        let other = Identity::generate().public();
        assert!(key_matches_pins(key, []));
        assert!(key_matches_pins(key, [None, None]));
        assert!(key_matches_pins(key, [Some(key), None, Some(key)]));
        assert!(!key_matches_pins(key, [Some(key), Some(other)]));
        assert!(!key_matches_pins(key, [None, Some(other)]));
    }

    #[test]
    fn incoming_dms_need_the_pinned_key() {
        let app = App::default();
        let identity = Identity::generate();
        let mut sender = UserProfile::new("sender");
        sender.identity = Some(identity.public());
        let mut message = commy_core::DmMessage::new(sender.id, UserId::new(), "hi");
        message.sign(&hex(SENDER_NODE), &identity);
        let envelope = DmEnvelope { message, sender: sender.clone() };
        assert!(dm_sender_trusted(&app, &envelope), "first key seen is trusted");

        let mut unsigned = envelope.clone();
        unsigned.message.sig = None;
        assert!(!dm_sender_trusted(&app, &unsigned));

        // Someone else signing as the same user with another key.
        let impostor = Identity::generate();
        let mut forged = envelope.clone();
        forged.sender.identity = Some(impostor.public());
        forged.message.sign(&hex(SENDER_NODE), &impostor);
        assert!(forged.signed_by_sender());
        assert!(dm_sender_trusted(&app, &forged), "nothing pinned yet");
        app.dms.lock().unwrap().remember_contact(sender.clone());
        assert!(dm_sender_trusted(&app, &envelope));
        assert!(!dm_sender_trusted(&app, &forged), "the contact pin wins");

        // A key pinned in a shared server counts too.
        let app = App::default();
        let mut state = ServerState::new(Server::new("s", UserId::new()));
        state.profiles.insert(sender.id, sender.clone());
        app.servers.lock().unwrap().insert(state.server.id, state);
        assert!(dm_sender_trusted(&app, &envelope));
        assert!(!dm_sender_trusted(&app, &forged));
    }

    const SENDER_NODE: [u8; 32] = [1; 32];
    const OTHER_NODE: [u8; 32] = [2; 32];

    fn hex(node: [u8; 32]) -> String {
        data_encoding::HEXLOWER.encode(&node)
    }

    /// A keyed sender on [`SENDER_NODE`], whose profile claims `claimed`.
    fn sender_on(claimed: [u8; 32]) -> (UserProfile, Identity) {
        let identity = Identity::generate();
        let mut sender = UserProfile::new("sender");
        sender.identity = Some(identity.public());
        sender.node = Some(hex(claimed));
        (sender, identity)
    }

    /// A DM `sender` signed for `to` as sent from `from`, arriving from `via`.
    fn dm_from(sender: &UserProfile, identity: &Identity, to: UserId, content: &str, from: [u8; 32], via: [u8; 32]) -> IncomingDm {
        let mut message = DmMessage::new(sender.id, to, content);
        message.sign(&hex(from), identity);
        IncomingDm { envelope: DmEnvelope { message, sender: sender.clone() }, node: via }
    }

    fn app_as(me: &UserProfile) -> App {
        let app = App::default();
        *app.profile.lock().unwrap() = Some(me.clone());
        app
    }

    #[test]
    fn dms_must_be_ours_and_from_the_node_they_name() {
        let me = UserProfile::new("me");
        let app = app_as(&me);
        let (sender, identity) = sender_on(SENDER_NODE);
        let direct = dm_from(&sender, &identity, me.id, "hi", SENDER_NODE, SENDER_NODE);
        assert_eq!(screen_dm(&app, &direct), Some(sender.clone()));
        app.dms.lock().unwrap().remember_contact(sender.clone());

        // Their signed DM to someone else, sent on to us: dropped.
        let someone_else = UserId::new();
        assert_eq!(screen_dm(&app, &dm_from(&sender, &identity, someone_else, "hi", SENDER_NODE, SENDER_NODE)), None);

        // A real DM to us, replayed from another node: dropped, whatever
        // node its unsigned profile claims.
        assert_eq!(screen_dm(&app, &dm_from(&sender, &identity, me.id, "hi", SENDER_NODE, OTHER_NODE)), None);
        let (mut moved, _) = sender_on(OTHER_NODE);
        moved.id = sender.id;
        moved.identity = sender.identity;
        assert_eq!(screen_dm(&app, &dm_from(&moved, &identity, me.id, "hi", SENDER_NODE, OTHER_NODE)), None);
        // Claiming the other node in the message needs the sender's key.
        let mut renamed = dm_from(&sender, &identity, me.id, "hi", SENDER_NODE, OTHER_NODE);
        renamed.envelope.message.from_node = hex(OTHER_NODE);
        assert_eq!(screen_dm(&app, &renamed), None);
        assert_eq!(app.dms.lock().unwrap().contacts[&sender.id].node, Some(hex(SENDER_NODE)));

        // Only the node it came from is cached, never the one claimed.
        let (claims_other, identity) = sender_on(OTHER_NODE);
        let fresh = app_as(&me);
        let screened = screen_dm(&fresh, &dm_from(&claims_other, &identity, me.id, "hi", SENDER_NODE, SENDER_NODE)).unwrap();
        assert_eq!(screened.node, Some(hex(SENDER_NODE)));

        // Unsigned, naming no node, or before we have a profile: dropped.
        let mut unsigned = direct.clone();
        unsigned.envelope.message.sig = None;
        assert_eq!(screen_dm(&app, &unsigned), None);
        let mut unnamed = DmMessage::new(sender.id, me.id, "hi");
        unnamed.sign("", &identity);
        let unnamed = IncomingDm { envelope: DmEnvelope { message: unnamed, sender: sender.clone() }, node: SENDER_NODE };
        assert_eq!(screen_dm(&app, &unnamed), None);
        assert_eq!(screen_dm(&App::default(), &direct), None);
    }

    #[test]
    fn replayed_friend_signals_never_reach_the_lists() {
        let me = UserProfile::new("me");
        let app = Arc::new(app_as(&me));
        let handle = TestHandle::new(app.clone());
        let (sender, identity) = sender_on(SENDER_NODE);
        // Signals written one second apart, in this order.
        let t0 = chrono::Utc::now();
        let signal = |signal: FriendSignal, second: i64| {
            let mut dm = dm_from(&sender, &identity, me.id, &signal.encode(), SENDER_NODE, SENDER_NODE);
            dm.envelope.message.created_at = t0 + chrono::Duration::seconds(second);
            dm.envelope.message.sign(&hex(SENDER_NODE), &identity);
            dm
        };
        let request = signal(FriendSignal::Request, 0);
        let accept = signal(FriendSignal::Accept, 1);
        let remove = signal(FriendSignal::Remove, 2);

        assert_eq!(receive_dm(&handle, request.clone()), Received::Taken);
        assert!(app.safety.lock().unwrap().friend_in.contains(&sender.id));
        // The same signal sent again is ignored.
        assert_eq!(receive_dm(&handle, request.clone()), Received::Dropped);
        assert_eq!(receive_dm(&handle, accept.clone()), Received::Taken);
        assert!(app.safety.lock().unwrap().friends.contains(&sender.id));
        assert_eq!(receive_dm(&handle, remove), Received::Taken);
        assert!(!app.safety.lock().unwrap().friends.contains(&sender.id));

        // Earlier signals replayed after a later one change nothing.
        app.safety.lock().unwrap().friend_out.insert(sender.id);
        assert_eq!(receive_dm(&handle, request), Received::Dropped);
        assert_eq!(receive_dm(&handle, accept), Received::Dropped);
        let safety = app.safety.lock().unwrap().clone();
        assert!(safety.friend_in.is_empty() && safety.friends.is_empty());
        assert!(safety.friend_out.contains(&sender.id), "our own request still stands");
        let names: Vec<String> = handle.take_events().into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["friends-changed"; 3]);

        // A newer signal relayed from another node never gets that far.
        let mut relayed = signal(FriendSignal::Request, 3);
        relayed.node = OTHER_NODE;
        assert_eq!(receive_dm(&handle, relayed.clone()), Received::Dropped);
        assert!(handle.take_events().is_empty());
        assert!(app.safety.lock().unwrap().friends.is_empty());
        // From the node it names, the same one meets our own request.
        relayed.node = SENDER_NODE;
        assert_eq!(receive_dm(&handle, relayed), Received::Reply { to: sender.id, signal: FriendSignal::Accept });
        assert!(app.safety.lock().unwrap().friends.contains(&sender.id));
    }

    #[test]
    fn a_friend_request_we_also_sent_is_answered() {
        let me = UserProfile::new("me");
        let app = Arc::new(app_as(&me));
        let handle = TestHandle::new(app.clone());
        let (sender, identity) = sender_on(SENDER_NODE);
        app.safety.lock().unwrap().friend_out.insert(sender.id);
        let request = dm_from(&sender, &identity, me.id, &FriendSignal::Request.encode(), SENDER_NODE, SENDER_NODE);
        assert_eq!(receive_dm(&handle, request), Received::Reply { to: sender.id, signal: FriendSignal::Accept });
        // A report is handed back to the caller.
        let report = dm_from(&sender, &identity, me.id, &format!("{}{{}}", safety::REPORT_PREFIX), SENDER_NODE, SENDER_NODE);
        assert!(matches!(receive_dm(&handle, report), Received::Report { from, .. } if from == sender.id));
    }

    #[test]
    fn incoming_messages_are_stored_once_with_their_node() {
        let me = UserProfile::new("me");
        let app = Arc::new(app_as(&me));
        app.safety.lock().unwrap().accepted.insert(UserId::new());
        let handle = TestHandle::new(app.clone());
        let (sender, identity) = sender_on(OTHER_NODE);
        app.safety.lock().unwrap().accepted.insert(sender.id);
        let dm = dm_from(&sender, &identity, me.id, "hello there", SENDER_NODE, SENDER_NODE);
        assert_eq!(receive_dm(&handle, dm.clone()), Received::Taken);
        assert_eq!(app.dms.lock().unwrap().conversation(sender.id), std::slice::from_ref(&dm.envelope.message));
        assert_eq!(app.dms.lock().unwrap().contacts[&sender.id].node, Some(hex(SENDER_NODE)));
        let names: Vec<String> = handle.take_events().into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["dm-changed", "dm-received"]);
        assert_eq!(receive_dm(&handle, dm), Received::Taken);
        assert!(handle.take_events().is_empty(), "a duplicate is not new");
    }

    #[test]
    fn concurrent_saves_leave_the_latest_outbox() {
        let dir = temp_dir("concurrent");
        let app = Arc::new(App::default());
        app.data_dir.set(dir.clone()).unwrap();
        let writers: Vec<_> = (0..8)
            .map(|_| {
                let app = app.clone();
                std::thread::spawn(move || {
                    for _ in 0..40 {
                        queue_dm(&app, UserId::new(), MessageId::new());
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        assert_eq!(app.outbox.lock().unwrap().dm_partners().len(), 8 * 40);
        assert_eq!(Outbox::load(&dir), *app.outbox.lock().unwrap(), "the file holds the last state");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn queued_events_wait_for_a_neighbor() {
        let app = App::default();
        let sid = ServerId::new();
        app.outbox.lock().unwrap().note_local(sid, &rename(sid, "offline"), false);
        flush_queued(&app, sid).await;
        assert_eq!(app.outbox.lock().unwrap().clone().take_queued(sid), vec![rename(sid, "offline")]);

        // The network is up and we are on the topic, but nobody is connected.
        let (mut node, _, _, _) = commy_net::CommyNode::spawn(None).await.unwrap();
        tokio::time::timeout(Duration::from_secs(30), node.host_server(sid, "s")).await.unwrap().unwrap();
        *app.net.lock().await = Some(node);
        flush_queued(&app, sid).await;
        assert_eq!(app.outbox.lock().unwrap().clone().take_queued(sid), vec![rename(sid, "offline")]);
        let node = app.net.lock().await.take();
        if let Some(node) = node {
            node.shutdown().await.ok();
        }
    }

    #[tokio::test]
    async fn a_waiting_dm_going_out_is_not_a_new_one() {
        let app = Arc::new(App::default());
        let identity = Identity::generate();
        let mut me = UserProfile::new("me");
        me.identity = Some(identity.public());
        *app.profile.lock().unwrap() = Some(me.clone());
        *app.identity.lock().unwrap() = Some(identity);
        *app.node_hex.lock().unwrap() = Some(hex(SENDER_NODE));
        let partner = UserId::new();
        let message = DmMessage::new(me.id, partner, "while you were away");
        app.dms.lock().unwrap().insert(partner, message.clone());
        app.outbox.lock().unwrap().add_dm(partner, message.id);
        let handle = TestHandle::new(app.clone());

        // Still unreachable: kept, and nothing to tell.
        deliver_dms_with(&handle, partner, |_| async { false }).await;
        assert!(app.outbox.lock().unwrap().has_dms(partner));
        assert!(handle.take_events().is_empty());

        let sent = Mutex::new(Vec::new());
        deliver_dms_with(&handle, partner, |envelope: DmEnvelope| {
            sent.lock().unwrap().push(envelope.message.id);
            async { true }
        })
        .await;
        assert_eq!(*sent.lock().unwrap(), vec![message.id]);
        assert!(!app.outbox.lock().unwrap().has_dms(partner));
        assert_eq!(handle.take_events(), vec![("dm-sent".to_string(), partner.to_string().into())]);
    }

    #[tokio::test]
    async fn a_waiting_dm_goes_out_signed_from_our_node_now() {
        let app = Arc::new(App::default());
        let identity = Identity::generate();
        let mut me = UserProfile::new("me");
        me.identity = Some(identity.public());
        *app.profile.lock().unwrap() = Some(me.clone());
        *app.identity.lock().unwrap() = Some(identity.clone());
        let partner = UserId::new();
        // Written on another node, or before the network was up.
        let mut message = DmMessage::new(me.id, partner, "queued");
        message.sign(&hex(OTHER_NODE), &identity);
        app.dms.lock().unwrap().insert(partner, message.clone());
        app.outbox.lock().unwrap().add_dm(partner, message.id);
        let handle = TestHandle::new(app.clone());
        let sent = Mutex::new(Vec::new());
        let send = |envelope: DmEnvelope| {
            sent.lock().unwrap().push(envelope);
            async { true }
        };

        // No node yet: nothing can go out.
        deliver_dms_with(&handle, partner, send).await;
        assert!(sent.lock().unwrap().is_empty() && app.outbox.lock().unwrap().has_dms(partner));

        *app.node_hex.lock().unwrap() = Some(hex(SENDER_NODE));
        deliver_dms_with(&handle, partner, send).await;
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].message.id, message.id, "the same message");
        assert!(sent[0].sent_from(&SENDER_NODE) && sent[0].signed_by_sender());
    }

    #[test]
    fn nothing_goes_out_while_our_key_is_held() {
        let app = App::default();
        let sid = ServerId::new();
        let lasting = rename(sid, "renamed");
        let live = ServerEvent::VoiceModerate { server: sid, user: UserId::new(), muted: true, deafened: false };
        assert!(!may_send(&app, 0, &lasting), "nobody to send to");
        assert!(may_send(&app, 1, &lasting));
        app.hold_key(Instant::now() + Duration::from_secs(120));
        assert!(!may_send(&app, 3, &lasting));
        assert!(may_send(&app, 3, &live), "live state is never kept, so it goes");
        assert!(!may_send(&app, 0, &live));
        app.settle_key();
        assert!(may_send(&app, 3, &lasting));
    }

    /// Two nodes on a server's topic; the first is connected to the second.
    async fn connected_pair(sid: ServerId) -> (commy_net::CommyNode, commy_net::CommyNode) {
        let within = Duration::from_secs;
        let (mut a, _, _, _) = commy_net::CommyNode::spawn(None).await.unwrap();
        let (mut b, _, _, _) = commy_net::CommyNode::spawn(None).await.unwrap();
        let ticket = tokio::time::timeout(within(30), a.host_server(sid, "s")).await.unwrap().unwrap();
        assert!(tokio::time::timeout(within(60), b.join_server(&ticket)).await.unwrap().unwrap());
        assert!(a.wait_for_peers(sid, within(30)).await);
        (a, b)
    }

    #[tokio::test]
    async fn a_held_key_keeps_local_sends_here() {
        let (state, _, message) = keyed_server();
        let sid = state.server.id;
        let (a, b) = connected_pair(sid).await;
        let app = App::default();
        *app.net.lock().await = Some(a);
        app.hold_key(Instant::now() + Duration::from_secs(120));

        // Someone is connected, but our key may still change.
        assert!(!broadcast_local(&app, sid, ServerEvent::MessageCreate { message: message.clone() }).await);
        assert!(app.outbox.lock().unwrap().is_pending(sid, message.id));
        assert!(!broadcast_local(&app, sid, rename(sid, "held")).await);
        flush_queued(&app, sid).await;
        assert_eq!(app.outbox.lock().unwrap().clone().take_queued(sid), vec![rename(sid, "held")]);

        // Settled: what waited goes out, and so does what comes next.
        app.settle_key();
        flush_queued(&app, sid).await;
        assert!(app.outbox.lock().unwrap().clone().take_queued(sid).is_empty());
        assert!(broadcast_local(&app, sid, rename(sid, "settled")).await);
        let a = app.net.lock().await.take().unwrap();
        a.shutdown().await.ok();
        b.shutdown().await.ok();
    }

    #[tokio::test]
    async fn requeued_events_are_saved_even_when_none_went_out() {
        let dir = temp_dir("requeue");
        let app = App::default();
        app.data_dir.set(dir.clone()).unwrap();
        let sid = ServerId::new();
        app.outbox.lock().unwrap().note_local(sid, &rename(sid, "waiting"), false);
        // Another save while the events are out of the queue, as from a
        // send in between, leaves them out of the file...
        let taken = app.outbox.lock().unwrap().take_queued(sid);
        app.persist_outbox();
        app.outbox.lock().unwrap().requeue(sid, taken);
        assert!(Outbox::load(&dir).take_queued(sid).is_empty());
        // ...until the next flush, which saves what it put back.
        flush_queued(&app, sid).await;
        assert_eq!(Outbox::load(&dir).take_queued(sid), vec![rename(sid, "waiting")]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn what_we_signed_while_held_is_signed_again_with_the_settled_key() {
        let held = Identity::generate();
        let settled = Identity::generate();
        let mut me = UserProfile::new("me");
        me.identity = Some(held.public());
        let member = Identity::generate();
        let mut other = UserProfile::new("member");
        other.identity = Some(member.public());
        let server = Server::new("s", me.id);
        let (sid, channel) = (server.id, server.channels[0].id);
        let mut state = ServerState::new(server);
        state.profiles.insert(me.id, me.clone());
        state.apply(ServerEvent::MemberJoin { server: sid, profile: other.clone() }).unwrap();
        let post = |author: UserId, identity: &Identity, content: &str| {
            let mut message = Message::new(channel, author, content);
            message.sign(sid, identity);
            message
        };
        // Written with the key held back: one waiting, one we edited.
        let waiting = post(me.id, &held, "waiting");
        let edited = post(me.id, &held, "first");
        let theirs = post(other.id, &member, "theirs");
        for message in [&waiting, &edited, &theirs] {
            state.apply(ServerEvent::MessageCreate { message: (*message).clone() }).unwrap();
        }
        let edit = edited.edit_event(sid, "second", chrono::Utc::now(), &held);
        state.apply(edit.clone()).unwrap();
        let delete = TombstoneProof::sign(sid, channel, theirs.id, me.id, chrono::Utc::now(), &held).into_event();
        // One of ours deleted here too, its proof kept for catch-up.
        let removed = post(me.id, &held, "removed");
        state.apply(ServerEvent::MessageCreate { message: removed.clone() }).unwrap();
        state.apply(TombstoneProof::sign(sid, channel, removed.id, me.id, chrono::Utc::now(), &held).into_event()).unwrap();

        let app = App::default();
        *app.profile.lock().unwrap() = Some(me.clone());
        app.servers.lock().unwrap().insert(sid, state);
        {
            let mut outbox = app.outbox.lock().unwrap();
            for message in [&waiting, &edited] {
                outbox.note_local(sid, &ServerEvent::MessageCreate { message: (*message).clone() }, false);
            }
            outbox.note_local(sid, &edit, false);
            outbox.note_local(sid, &delete, false);
        }

        // The key that settled is the same: nothing changes.
        *app.identity.lock().unwrap() = Some(held.clone());
        let kept = |app: &App| {
            let snapshot = app.servers.lock().unwrap()[&sid].snapshot();
            (snapshot.messages, snapshot.tombstone_proofs)
        };
        let before = kept(&app);
        resign_held(&app);
        assert_eq!(kept(&app), before);

        // Another key settled.
        *app.identity.lock().unwrap() = Some(settled.clone());
        app.servers.lock().unwrap().get_mut(&sid).unwrap().voice_states.insert(other.id, channel);
        resign_held(&app);
        let state = app.servers.lock().unwrap()[&sid].clone();
        let key = settled.public();
        let now_waiting = state.message(waiting.id).unwrap();
        assert!(now_waiting.verify(sid, &key));
        assert_eq!(now_waiting.content, "waiting");
        let now_edited = state.message(edited.id).unwrap();
        assert!(now_edited.verify(sid, &key) && now_edited.content == "second");
        assert_eq!(state.message(theirs.id), Some(&theirs), "only our own messages");
        assert_eq!(state.voice_states.get(&other.id), Some(&channel), "live state stays");
        let proof = state.tombstone(removed.id).unwrap();
        assert!(proof.verify(sid, &key) && proof.by == me.id);
        assert!(!state.has_message(removed.id));
        let queued = app.outbox.lock().unwrap().clone().take_queued(sid);
        let [ServerEvent::MessageEdit { content, edited_at, sig, .. }, ServerEvent::MessageDelete { channel: c, message: m, by, at, sig: proof_sig }] =
            queued.as_slice()
        else {
            panic!("unexpected queue {queued:?}");
        };
        assert!(now_edited.verify_edit(sid, content, *edited_at, sig, &key));
        let proof = TombstoneProof { channel: *c, message: *m, by: *by, at: *at, sig: *proof_sig };
        assert!(proof.verify(sid, &key) && proof.message == theirs.id);
        assert_eq!(app.outbox.lock().unwrap().pending_ids(sid), HashSet::from([waiting.id, edited.id]), "same ids");
    }
}
