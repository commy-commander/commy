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

pub const MAX_QUEUED: usize = 500;

pub const DM_RETRY_FIRST: Duration = Duration::from_secs(30);
pub const DM_RETRY_MAX: Duration = Duration::from_secs(300);

pub const DM_SEND_TIMEOUT: Duration = Duration::from_secs(20);

const DM_TICK: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Outbox {
    #[serde(default)]
    pending: HashMap<ServerId, Vec<MessageId>>,
    #[serde(default)]
    queued: HashMap<ServerId, Vec<ServerEvent>>,
    #[serde(default)]
    dms: HashMap<UserId, Vec<MessageId>>,
}

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
        if let ServerEvent::MessageEdit { message, .. } = event {
            let rides_along_with_its_pending_message = self.is_pending(server, *message);
            if rides_along_with_its_pending_message {
                return changed;
            }
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

    pub fn is_pending(&self, server: ServerId, id: MessageId) -> bool {
        self.pending.get(&server).is_some_and(|list| list.contains(&id))
    }

    pub fn pending_ids(&self, server: ServerId) -> HashSet<MessageId> {
        self.pending.get(&server).map(|list| list.iter().copied().collect()).unwrap_or_default()
    }

    pub fn has_pending(&self, server: ServerId) -> bool {
        self.pending.get(&server).is_some_and(|list| !list.is_empty())
    }

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

    #[cfg(test)]
    pub fn take_queued(&mut self, server: ServerId) -> Vec<ServerEvent> {
        self.queued.remove(&server).unwrap_or_default()
    }

    pub fn take_queued_where(&mut self, server: ServerId, take: impl Fn(&ServerEvent) -> bool) -> Vec<(usize, ServerEvent)> {
        let Some(list) = self.queued.get_mut(&server) else { return Vec::new() };
        let mut taken = Vec::new();
        let mut kept = Vec::new();
        for event in std::mem::take(list) {
            if take(&event) {
                taken.push((kept.len(), event));
            } else {
                kept.push(event);
            }
        }
        if kept.is_empty() {
            self.queued.remove(&server);
        } else {
            *list = kept;
        }
        taken
    }

    pub fn requeue(&mut self, server: ServerId, events: Vec<(usize, ServerEvent)>) {
        if events.is_empty() {
            return;
        }
        let mut queued = self.queued.remove(&server).unwrap_or_default().into_iter();
        let mut list = Vec::new();
        let mut passed = 0;
        for (ahead, event) in events {
            while passed < ahead {
                let Some(next) = queued.next() else { break };
                list.push(next);
                passed += 1;
            }
            list.push(event);
        }
        list.extend(queued);
        if list.len() > MAX_QUEUED {
            let excess = list.len() - MAX_QUEUED;
            list.drain(..excess);
        }
        self.queued.insert(server, list);
    }

    pub fn forget_server(&mut self, server: ServerId) -> bool {
        let pending = self.pending.remove(&server).is_some();
        self.queued.remove(&server).is_some() || pending
    }

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

    pub fn dm_queue(&self, partner: UserId) -> Vec<MessageId> {
        self.dms.get(&partner).cloned().unwrap_or_default()
    }

    pub fn pending_dms(&self, partner: UserId) -> HashSet<MessageId> {
        self.dms.get(&partner).map(|list| list.iter().copied().collect()).unwrap_or_default()
    }

    pub fn dm_partners(&self) -> Vec<UserId> {
        self.dms.iter().filter(|(_, list)| !list.is_empty()).map(|(user, _)| *user).collect()
    }

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

#[derive(Debug, Default)]
pub struct DmBackoff {
    next: HashMap<UserId, (Instant, Duration)>,
}

impl DmBackoff {
    pub fn due(&self, partner: UserId, now: Instant) -> bool {
        self.next.get(&partner).is_none_or(|(at, _)| now >= *at)
    }

    pub fn schedule(&mut self, partner: UserId, now: Instant) {
        self.next.entry(partner).or_insert((now + DM_RETRY_FIRST, DM_RETRY_FIRST));
    }

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

    pub fn wake(&mut self, partner: UserId, now: Instant) {
        if let Some((at, _)) = self.next.get_mut(&partner) {
            *at = now;
        }
    }
}

pub fn online_user(event: &ServerEvent) -> Option<(UserId, Presence)> {
    match event {
        ServerEvent::PresenceUpdate { user, presence } => Some((*user, *presence)),
        ServerEvent::ProfileUpdate { profile } | ServerEvent::MemberJoin { profile, .. } => {
            Some((profile.id, profile.presence))
        }
        _ => None,
    }
}

pub fn key_matches_pins(key: IdentityKey, pins: impl IntoIterator<Item = Option<IdentityKey>>) -> bool {
    pins.into_iter().flatten().all(|pinned| pinned == key)
}

fn member_entries(servers: &HashMap<ServerId, ServerState>, user: UserId) -> impl Iterator<Item = &UserProfile> {
    servers.values().filter(move |s| s.server.is_member(user)).filter_map(move |s| s.profiles.get(&user))
}

pub fn member_profile(servers: &HashMap<ServerId, ServerState>, user: UserId) -> Option<UserProfile> {
    let profile = member_entries(servers, user).next()?.clone();
    let mut keys = member_entries(servers, user).filter_map(|p| p.identity);
    let identity = keys.next().filter(|first| keys.all(|key| key == *first));
    Some(UserProfile { identity, ..profile })
}

pub fn dm_sender_trusted(app: &App, envelope: &DmEnvelope) -> bool {
    let Some(key) = envelope.sender.identity.filter(|_| envelope.signed_by_sender()) else {
        return false;
    };
    let user = envelope.sender.id;
    if let Some(pinned) = app.dms.lock().unwrap().pinned_key(user) {
        return key == pinned;
    }
    key_matches_pins(key, member_entries(&app.servers.lock().unwrap(), user).map(|p| p.identity))
}

pub fn screen_dm(app: &App, dm: &IncomingDm) -> Option<UserProfile> {
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id)?;
    let envelope = &dm.envelope;
    if envelope.message.to != me || !envelope.sent_from(&dm.node) || !dm_sender_trusted(app, envelope) {
        return None;
    }
    let mut sender = envelope.sender.clone();
    sender.node = Some(data_encoding::HEXLOWER.encode(&dm.node));
    Some(sender)
}

#[derive(Debug, PartialEq)]
pub enum Received {
    Dropped,
    Taken,
    Report { from: UserId, content: String },
    Reply { to: UserId, signal: FriendSignal },
}

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

pub fn needs_settled_key(event: &ServerEvent, me: Option<UserId>) -> bool {
    use ServerEvent as E;
    match event {
        E::MessageCreate { .. } | E::MessageEdit { .. } | E::MessageDelete { .. } | E::DoorbellSet { .. } => true,
        E::MemberJoin { profile, .. } | E::ProfileUpdate { profile } => me.is_none_or(|me| me == profile.id),
        E::ReactionToggle { .. }
        | E::MemberLeave { .. }
        | E::PresenceUpdate { .. }
        | E::ChannelCreate { .. }
        | E::ChannelDelete { .. }
        | E::VoiceState { .. }
        | E::TypingStart { .. }
        | E::ScreenShare { .. }
        | E::ServerRename { .. }
        | E::RoleCreate { .. }
        | E::RoleDelete { .. }
        | E::RoleAssign { .. }
        | E::RoleUnassign { .. }
        | E::AfkChannelSet { .. }
        | E::ChannelUpdate { .. }
        | E::CameraState { .. }
        | E::ServerImages { .. }
        | E::ReactionSet { .. }
        | E::EmojiAdd { .. }
        | E::EmojiRemove { .. }
        | E::SoundAdd { .. }
        | E::SoundRemove { .. }
        | E::ExpressionSettings { .. }
        | E::PublicSet { .. }
        | E::SoundboardPlay { .. }
        | E::GameTagsSet { .. }
        | E::RulesSet { .. }
        | E::RulesAccept { .. }
        | E::EventUpsert { .. }
        | E::EventDelete { .. }
        | E::EventRsvp { .. }
        | E::ChannelLayout { .. }
        | E::ChannelLimit { .. }
        | E::ChannelBanner { .. }
        | E::ChannelColor { .. }
        | E::MessagePin { .. }
        | E::ChannelKindSet { .. }
        | E::RoleUpdate { .. }
        | E::VoiceModerate { .. }
        | E::MemberBan { .. }
        | E::MemberUnban { .. }
        | E::AfkTimeoutSet { .. } => false,
    }
}

fn held_for_key(app: &App, event: &ServerEvent) -> bool {
    if app.key_settled() {
        return false;
    }
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    needs_settled_key(event, me)
}

pub fn may_send(app: &App, neighbors: usize, event: &ServerEvent) -> bool {
    neighbors > 0 && !held_for_key(app, event)
}

pub async fn broadcast_local(app: &App, server: ServerId, event: ServerEvent) -> bool {
    crate::wake::unrest(app).await;
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

pub async fn flush_queued(app: &App, server: ServerId) {
    let Ok(_signing) = app.signing.try_read() else { return };
    let settled = app.key_settled();
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let events = app.outbox.lock().unwrap().take_queued_where(server, |e| settled || !needs_settled_key(e, me));
    if events.is_empty() {
        return;
    }
    let total = events.len();
    let mut left = Vec::new();
    for (i, (_, event)) in events.iter().enumerate() {
        let sent = {
            let net = app.net.lock().await;
            match net.as_ref() {
                Some(node) if may_send(app, node.neighbor_count(server), event) => {
                    node.broadcast(server, WirePayload::Event(event.clone())).await
                }
                Some(_) => Err(anyhow::anyhow!("no neighbor is connected")),
                None => Err(anyhow::anyhow!("network is not ready")),
            }
        };
        if let Err(e) = sent {
            tracing::warn!("re-sending queued events stopped: {e}");
            left.extend_from_slice(&events[i..]);
            break;
        }
    }
    tracing::info!("re-sent {} of {total} queued events that may go now", total - left.len());
    app.outbox.lock().unwrap().requeue(server, left);
    app.persist_outbox();
}

pub fn resign_ours(state: &mut ServerState, me: UserId, identity: &Identity) -> bool {
    let server = state.server.id;
    let key = identity.public();
    let stale = |m: &Message| m.author == me && m.sig.is_some() && !m.verify(server, &key);
    let mut snapshot = state.snapshot();
    let mut changed = false;
    for message in snapshot.messages.iter_mut().flat_map(|(_, list)| list.iter_mut()).filter(|m| stale(m)) {
        message.sign(server, identity);
        changed = true;
    }
    for proof in snapshot.tombstone_proofs.iter_mut().filter(|p| p.by == me && !p.verify(server, &key)) {
        *proof = TombstoneProof::sign(server, proof.channel, proof.message, me, proof.at, identity);
        changed = true;
    }
    if changed {
        let mut fresh = ServerState::from_snapshot(snapshot);
        fresh.profiles = std::mem::take(&mut state.profiles);
        fresh.voice_states = std::mem::take(&mut state.voice_states);
        fresh.voice_since = std::mem::take(&mut state.voice_since);
        fresh.screen_shares = std::mem::take(&mut state.screen_shares);
        fresh.cameras = std::mem::take(&mut state.cameras);
        *state = fresh;
    }
    changed
}

impl Outbox {
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

pub fn resign_held(app: &App) {
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let (Some(me), Some(identity)) = (me, app.identity()) else { return };
    let servers: Vec<ServerId> = app.servers.lock().unwrap().keys().copied().collect();
    for server in servers {
        let (kept, queued) = {
            let mut servers = app.servers.lock().unwrap();
            let Some(state) = servers.get_mut(&server) else { continue };
            let kept = resign_ours(state, me, &identity);
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

pub fn queue_dm(app: &App, partner: UserId, id: MessageId) {
    if app.outbox.lock().unwrap().add_dm(partner, id) {
        app.dm_backoff.lock().unwrap().schedule(partner, Instant::now());
        app.persist_outbox();
    }
}

pub fn wake_dm(app: &App, partner: UserId) {
    if !app.outbox.lock().unwrap().has_dms(partner) {
        return;
    }
    app.dm_backoff.lock().unwrap().wake(partner, Instant::now());
    app.dm_wake.notify_one();
}

pub fn wake_all_dms(app: &App) {
    let partners = app.outbox.lock().unwrap().dm_partners();
    for partner in partners {
        wake_dm(app, partner);
    }
}

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

async fn deliver_dms(handle: &tauri::AppHandle, partner: UserId) {
    let app = handle.state::<App>();
    deliver_dms_with(handle, partner, |envelope| {
        let app = &app;
        async move { try_send_dm(app, partner, &envelope).await }
    })
    .await;
}

async fn deliver_dms_with<A, F, Fut>(access: &A, partner: UserId, send: F)
where
    A: AppAccess,
    F: Fn(DmEnvelope) -> Fut,
    Fut: Future<Output = bool>,
{
    let app = access.app();
    let node = app.node_hex.lock().unwrap().clone();
    let (Ok(_), Some(_), Some(node)) = (crate::current_profile(app), app.identity(), node) else { return };
    if app.safety.lock().unwrap().is_blocked(partner) {
        return;
    }
    let mut changed = false;
    let mut failed = false;
    let queue = app.outbox.lock().unwrap().dm_queue(partner);
    for id in queue {
        let found = app.dms.lock().unwrap().conversation(partner).iter().find(|m| m.id == id).cloned();
        let Some(mut message) = found else {
            changed |= app.outbox.lock().unwrap().dm_done(partner, id);
            continue;
        };
        let _signing = app.signing.read().await;
        let (Ok(me), Some(identity)) = (crate::current_profile(app), app.identity()) else {
            failed = true;
            break;
        };
        message.sign(&node, &identity);
        let envelope = DmEnvelope { message, sender: me.as_seen_by_others() };
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
        assert!(!outbox.note_local(sid, &edit, false), "an edit of a pending message rides along with it");
        assert!(outbox.clear_delivered(sid, [message.id]));
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
            ServerEvent::VoiceState { user, channel: Some(channel), muted: false, deafened: false, since: None },
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

        outbox.note_local(sid, &rename(sid, "later"), false);
        outbox.requeue(sid, vec![(0, rename(sid, "first")), (0, rename(sid, "second"))]);
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
        let (a, b) = (MessageId::for_author(message.author), MessageId::for_author(message.author));
        for id in [a, b, message.id] {
            outbox.note_local(sid, &ServerEvent::MessageCreate { message: Message { id, ..message.clone() } }, false);
        }
        assert!(outbox.clear_delivered(sid, [a, MessageId::new()]));
        assert!(!outbox.clear_delivered(sid, [a]), "already cleared");
        assert!(!outbox.clear_delivered(ServerId::new(), [b]));
        assert_eq!(outbox.pending_ids(sid), HashSet::from([b, message.id]));

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
        let gone = MessageId::for_author(message.author);
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
        assert_eq!(loaded.clone().take_queued(sid).len(), 2, "the edit rides along with its pending message");

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

        let impostor = Identity::generate();
        let mut forged = envelope.clone();
        forged.sender.identity = Some(impostor.public());
        forged.message.sign(&hex(SENDER_NODE), &impostor);
        assert!(forged.signed_by_sender());
        assert!(dm_sender_trusted(&app, &forged), "nothing pinned yet");
        app.dms.lock().unwrap().remember_contact(sender.clone());
        assert!(dm_sender_trusted(&app, &envelope));
        assert!(!dm_sender_trusted(&app, &forged), "the contact pin wins");

        let app = App::default();
        let mut state = ServerState::new(Server::new("s", sender.id));
        state.profiles.insert(sender.id, sender.clone());
        app.servers.lock().unwrap().insert(state.server.id, state);
        assert!(dm_sender_trusted(&app, &envelope));
        assert!(!dm_sender_trusted(&app, &forged));
    }

    fn server_with(app: &App, user: &UserProfile, member: bool, key: IdentityKey) -> ServerId {
        let owner = if member { user.id } else { UserId::new() };
        let mut state = ServerState::new(Server::new("s", owner));
        state.profiles.insert(user.id, UserProfile { identity: Some(key), ..user.clone() });
        let sid = state.server.id;
        app.servers.lock().unwrap().insert(sid, state);
        sid
    }

    #[test]
    fn a_key_planted_for_a_non_member_cuts_off_nobody() {
        let real = Identity::generate();
        let planted = Identity::generate().public();
        let mut sender = UserProfile::new("sender");
        sender.identity = Some(real.public());
        let mut message = DmMessage::new(sender.id, UserId::new(), "hi");
        message.sign(&hex(SENDER_NODE), &real);
        let envelope = DmEnvelope { message, sender: sender.clone() };

        let app = App::default();
        app.dms.lock().unwrap().remember_contact(sender.clone());
        server_with(&app, &sender, false, planted);
        assert!(dm_sender_trusted(&app, &envelope));
        server_with(&app, &sender, true, planted);
        assert!(dm_sender_trusted(&app, &envelope));

        let app = App::default();
        server_with(&app, &sender, false, planted);
        assert!(dm_sender_trusted(&app, &envelope));
        let member = server_with(&app, &sender, true, real.public());
        assert!(dm_sender_trusted(&app, &envelope));
        server_with(&app, &sender, true, planted);
        assert!(!dm_sender_trusted(&app, &envelope));
        app.servers.lock().unwrap().remove(&member);
        assert!(!dm_sender_trusted(&app, &envelope));
    }

    #[test]
    fn a_contact_is_taken_from_a_server_they_are_in() {
        let real = Identity::generate().public();
        let planted = Identity::generate().public();
        let sender = UserProfile::new("sender");
        let app = App::default();
        server_with(&app, &sender, false, planted);
        let servers = |app: &App| app.servers.lock().unwrap().clone();
        assert_eq!(member_profile(&servers(&app), sender.id), None, "not a member anywhere");

        let member = server_with(&app, &sender, true, real);
        assert_eq!(member_profile(&servers(&app), sender.id).unwrap().identity, Some(real));
        server_with(&app, &sender, true, planted);
        let profile = member_profile(&servers(&app), sender.id).unwrap();
        assert_eq!((profile.id, profile.identity), (sender.id, None));

        let mut dms = commy_core::DmStore::new();
        dms.remember_contact(UserProfile { identity: Some(real), ..sender.clone() });
        app.servers.lock().unwrap().remove(&member);
        let other = member_profile(&servers(&app), sender.id).unwrap();
        assert_eq!(other.identity, Some(planted));
        dms.remember_contact(other);
        assert_eq!(dms.pinned_key(sender.id), Some(real));
    }

    const SENDER_NODE: [u8; 32] = [1; 32];
    const OTHER_NODE: [u8; 32] = [2; 32];

    fn hex(node: [u8; 32]) -> String {
        data_encoding::HEXLOWER.encode(&node)
    }

    fn sender_on(claimed: [u8; 32]) -> (UserProfile, Identity) {
        let identity = Identity::generate();
        let mut sender = UserProfile::new("sender");
        sender.identity = Some(identity.public());
        sender.node = Some(hex(claimed));
        (sender, identity)
    }

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

        let someone_else = UserId::new();
        assert_eq!(screen_dm(&app, &dm_from(&sender, &identity, someone_else, "hi", SENDER_NODE, SENDER_NODE)), None);

        assert_eq!(screen_dm(&app, &dm_from(&sender, &identity, me.id, "hi", SENDER_NODE, OTHER_NODE)), None);
        let (mut moved, _) = sender_on(OTHER_NODE);
        moved.id = sender.id;
        moved.identity = sender.identity;
        assert_eq!(screen_dm(&app, &dm_from(&moved, &identity, me.id, "hi", SENDER_NODE, OTHER_NODE)), None);
        let mut renamed = dm_from(&sender, &identity, me.id, "hi", SENDER_NODE, OTHER_NODE);
        renamed.envelope.message.from_node = hex(OTHER_NODE);
        assert_eq!(screen_dm(&app, &renamed), None);
        assert_eq!(app.dms.lock().unwrap().contacts[&sender.id].node, Some(hex(SENDER_NODE)));

        let (claims_other, identity) = sender_on(OTHER_NODE);
        let fresh = app_as(&me);
        let screened = screen_dm(&fresh, &dm_from(&claims_other, &identity, me.id, "hi", SENDER_NODE, SENDER_NODE)).unwrap();
        assert_eq!(screened.node, Some(hex(SENDER_NODE)));

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
        assert_eq!(receive_dm(&handle, request.clone()), Received::Dropped);
        assert_eq!(receive_dm(&handle, accept.clone()), Received::Taken);
        assert!(app.safety.lock().unwrap().friends.contains(&sender.id));
        assert_eq!(receive_dm(&handle, remove), Received::Taken);
        assert!(!app.safety.lock().unwrap().friends.contains(&sender.id));

        app.safety.lock().unwrap().friend_out.insert(sender.id);
        assert_eq!(receive_dm(&handle, request), Received::Dropped);
        assert_eq!(receive_dm(&handle, accept), Received::Dropped);
        let safety = app.safety.lock().unwrap().clone();
        assert!(safety.friend_in.is_empty() && safety.friends.is_empty());
        assert!(safety.friend_out.contains(&sender.id), "our own request still stands");
        let names: Vec<String> = handle.take_events().into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["friends-changed"; 3]);

        let mut relayed = signal(FriendSignal::Request, 3);
        relayed.node = OTHER_NODE;
        assert_eq!(receive_dm(&handle, relayed.clone()), Received::Dropped);
        assert!(handle.take_events().is_empty());
        assert!(app.safety.lock().unwrap().friends.is_empty());
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
    async fn events_waiting_for_our_key_keep_their_place() {
        let (state, identity, message) = keyed_server();
        let sid = state.server.id;
        let app = app_as(&state.profiles[&message.author]);
        app.hold_key(Instant::now() + Duration::from_secs(120));
        let edit = message.edit_event(sid, "edited", chrono::Utc::now(), &identity);
        for event in [&edit, &rename(sid, "a"), &rename(sid, "b")] {
            app.outbox.lock().unwrap().note_local(sid, event, false);
        }
        flush_queued(&app, sid).await;
        assert_eq!(app.outbox.lock().unwrap().clone().take_queued(sid), vec![edit, rename(sid, "a"), rename(sid, "b")]);
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

        deliver_dms_with(&handle, partner, send).await;
        assert!(sent.lock().unwrap().is_empty() && app.outbox.lock().unwrap().has_dms(partner));

        *app.node_hex.lock().unwrap() = Some(hex(SENDER_NODE));
        deliver_dms_with(&handle, partner, send).await;
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].message.id, message.id, "the same message");
        assert!(sent[0].sent_from(&SENDER_NODE) && sent[0].signed_by_sender());
    }

    #[tokio::test]
    async fn waiting_dms_sent_while_our_key_swaps_carry_the_new_one() {
        let dir = temp_dir("dm-swap");
        let write_key = |identity: &Identity| {
            let hex = data_encoding::HEXLOWER.encode(&identity.to_bytes());
            std::fs::write(dir.join(commy_core::identity::IDENTITY_FILE), hex).unwrap();
        };
        let (held, account) = (Identity::generate(), Identity::generate());
        write_key(&held);
        let app = Arc::new(App::default());
        app.data_dir.set(dir.clone()).unwrap();
        let me = UserProfile::new("me");
        *app.profile.lock().unwrap() = Some(me.clone());
        app.reload_identity().unwrap();
        *app.node_hex.lock().unwrap() = Some(hex(SENDER_NODE));
        app.mark_key_unsettled();
        app.hold_key(Instant::now());
        let partner = UserId::new();
        for content in ["one", "two"] {
            let message = DmMessage::new(me.id, partner, content);
            app.dms.lock().unwrap().insert(partner, message.clone());
            app.outbox.lock().unwrap().add_dm(partner, message.id);
        }
        write_key(&account);

        let sent = Arc::new(Mutex::new(Vec::new()));
        let (on_wire, wired) = tokio::sync::oneshot::channel();
        let on_wire = Arc::new(Mutex::new(Some(on_wire)));
        let go = Arc::new(tokio::sync::Notify::new());
        let send = {
            let (sent, go) = (sent.clone(), go.clone());
            move |envelope: DmEnvelope| {
                let (sent, on_wire, go) = (sent.clone(), on_wire.clone(), go.clone());
                async move {
                    let first = {
                        let mut sent = sent.lock().unwrap();
                        sent.push(envelope);
                        sent.len() == 1
                    };
                    if first {
                        let tx = on_wire.lock().unwrap().take();
                        tx.map(|tx| tx.send(()));
                        go.notified().await;
                    }
                    true
                }
            }
        };
        let delivering = tokio::spawn({
            let handle = TestHandle::new(app.clone());
            async move { deliver_dms_with(&handle, partner, send).await }
        });
        wired.await.unwrap();
        let settling = tokio::spawn({
            let app = app.clone();
            async move {
                let swap = app.signing.write().await;
                crate::settle::settle_loaded_key(&app, &swap)
            }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(app.settle_pending(), "the key is not swapped under a send");
        assert_eq!(app.identity().unwrap().public(), held.public());
        go.notify_one();
        delivering.await.unwrap();
        assert!(settling.await.unwrap().unwrap());

        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert!(sent.iter().all(DmEnvelope::signed_by_sender));
        assert_eq!(sent[0].sender.identity, Some(held.public()));
        assert_eq!(sent[1].sender.identity, Some(account.public()), "signed after the swap");
        std::fs::remove_dir_all(&dir).ok();
    }

    fn signed_and_unsigned(me: &UserProfile, identity: &Identity) -> (Vec<ServerEvent>, Vec<ServerEvent>) {
        let server = Server::new("s", me.id);
        let (sid, channel) = (server.id, server.channels[0].id);
        let mut message = Message::new(channel, me.id, "hi");
        message.sign(sid, identity);
        let other = UserId::new();
        let role = commy_core::RoleId::new();
        let signed = vec![
            ServerEvent::MessageCreate { message: message.clone() },
            message.edit_event(sid, "edited", chrono::Utc::now(), identity),
            TombstoneProof::sign(sid, channel, message.id, me.id, chrono::Utc::now(), identity).into_event(),
            ServerEvent::MemberJoin { server: sid, profile: me.clone() },
            ServerEvent::ProfileUpdate { profile: me.clone() },
        ];
        let unsigned = vec![
            rename(sid, "renamed"),
            ServerEvent::MemberLeave { server: sid, user: other },
            ServerEvent::MemberBan { server: sid, user: other },
            ServerEvent::MemberUnban { server: sid, user: other },
            ServerEvent::RoleAssign { server: sid, user: other, role },
            ServerEvent::RoleDelete { server: sid, role },
            ServerEvent::ChannelCreate { server: sid, channel: commy_core::Channel::new("new", commy_core::ChannelKind::Text, 3) },
            ServerEvent::ChannelDelete { server: sid, channel },
            ServerEvent::ChannelUpdate { server: sid, channel, name: "general".into(), retention_days: Some(7) },
            ServerEvent::ReactionSet { channel, message: message.id, emoji: "x".into(), user: me.id, active: true },
            ServerEvent::PresenceUpdate { user: me.id, presence: Presence::Idle },
            ServerEvent::ProfileUpdate { profile: UserProfile::new("member") },
            ServerEvent::VoiceModerate { server: sid, user: other, muted: true, deafened: false },
        ];
        (signed, unsigned)
    }

    #[test]
    fn only_what_carries_our_key_waits_while_it_is_held() {
        let identity = Identity::generate();
        let mut me = UserProfile::new("me");
        me.identity = Some(identity.public());
        let app = app_as(&me);
        let (signed, unsigned) = signed_and_unsigned(&me, &identity);
        for event in signed.iter().chain(&unsigned) {
            assert!(!may_send(&app, 0, event), "nobody to send to: {event:?}");
            assert!(may_send(&app, 1, event), "settled: {event:?}");
        }
        app.hold_key(Instant::now() + Duration::from_secs(120));
        for event in &signed {
            assert!(!may_send(&app, 3, event), "waits for our key: {event:?}");
        }
        for event in &unsigned {
            assert!(may_send(&app, 3, event), "goes at once: {event:?}");
            assert!(!may_send(&app, 0, event));
        }
        app.settle_key();
        assert!(signed.iter().all(|event| may_send(&app, 3, event)));
    }

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
        let (state, identity, message) = keyed_server();
        let sid = state.server.id;
        let (a, b) = connected_pair(sid).await;
        let app = app_as(&state.profiles[&message.author]);
        *app.net.lock().await = Some(a);
        app.hold_key(Instant::now() + Duration::from_secs(120));

        assert!(!broadcast_local(&app, sid, ServerEvent::MessageCreate { message: message.clone() }).await);
        assert!(app.outbox.lock().unwrap().is_pending(sid, message.id));
        app.outbox.lock().unwrap().clear_delivered(sid, [message.id]);
        let edit = message.edit_event(sid, "held", chrono::Utc::now(), &identity);
        assert!(!broadcast_local(&app, sid, edit.clone()).await);
        assert!(broadcast_local(&app, sid, rename(sid, "now")).await);
        app.outbox.lock().unwrap().note_local(sid, &rename(sid, "offline"), false);
        flush_queued(&app, sid).await;
        assert_eq!(app.outbox.lock().unwrap().clone().take_queued(sid), vec![edit]);

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
        let taken = app.outbox.lock().unwrap().take_queued_where(sid, |_| true);
        app.persist_outbox();
        app.outbox.lock().unwrap().requeue(sid, taken);
        assert!(Outbox::load(&dir).take_queued(sid).is_empty());
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
        let waiting = post(me.id, &held, "waiting");
        let edited = post(me.id, &held, "first");
        let theirs = post(other.id, &member, "theirs");
        for message in [&waiting, &edited, &theirs] {
            state.apply(ServerEvent::MessageCreate { message: (*message).clone() }).unwrap();
        }
        let edit = edited.edit_event(sid, "second", chrono::Utc::now(), &held);
        state.apply(edit.clone()).unwrap();
        let delete = TombstoneProof::sign(sid, channel, theirs.id, me.id, chrono::Utc::now(), &held).into_event();
        let removed = post(me.id, &held, "removed");
        state.apply(ServerEvent::MessageCreate { message: removed.clone() }).unwrap();
        state.apply(TombstoneProof::sign(sid, channel, removed.id, me.id, chrono::Utc::now(), &held).into_event()).unwrap();

        let app = App::default();
        *app.profile.lock().unwrap() = Some(me.clone());
        app.servers.lock().unwrap().insert(sid, state);
        {
            let mut outbox = app.outbox.lock().unwrap();
            outbox.note_local(sid, &ServerEvent::MessageCreate { message: waiting.clone() }, false);
            outbox.note_local(sid, &edit, false);
            outbox.note_local(sid, &delete, false);
        }

        *app.identity.lock().unwrap() = Some(held.clone());
        let kept = |app: &App| {
            let snapshot = app.servers.lock().unwrap()[&sid].snapshot();
            (snapshot.messages, snapshot.tombstone_proofs)
        };
        let before = kept(&app);
        resign_held(&app);
        assert_eq!(kept(&app), before);

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
        assert_eq!(app.outbox.lock().unwrap().pending_ids(sid), HashSet::from([waiting.id]), "same ids");
    }
}
