use crate::handler::BotEvent;
use commy_core::{
    action, Action, Identity, Message, MessageId, ServerEvent, ServerId, ServerState, SignedEvent, StoreError,
    TombstoneProof, UserId, UserProfile,
};
use commy_net::catchup::{apply_items, apply_profiles, ApplyReport};
use commy_net::protocol::{verify_sync_response, SyncMember};
use commy_net::{ServerTicket, WirePayload};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub const SNAPSHOT_MAX_AGE_MS: i64 = 900_000;
pub const PENDING_CAP: usize = 256;
pub const PENDING_FOR: Duration = Duration::from_secs(15 * 60);

type Pending = (Instant, ServerEvent, Option<Action>);

pub struct Replica {
    pub ticket: ServerTicket,
    pub state: Option<ServerState>,
    pending: VecDeque<Pending>,
    early: VecDeque<Pending>,
}

impl Replica {
    pub fn new(ticket: ServerTicket, state: Option<ServerState>) -> Self {
        Self { ticket, state, pending: VecDeque::new(), early: VecDeque::new() }
    }

    pub fn server(&self) -> ServerId {
        self.ticket.server_id
    }

    pub fn name(&self) -> &str {
        self.state.as_ref().map_or(&self.ticket.server_name, |s| &s.server.name)
    }

    pub fn vouched(&self, payload: &WirePayload, now_ms: i64) -> bool {
        let WirePayload::SyncResponse { server, by, at, .. } = payload else { return false };
        let Some((owner, key)) = self.ticket.owner else { return false };
        server.id == self.server()
            && *by == owner
            && server.owner == owner
            && (now_ms - at).abs() <= SNAPSHOT_MAX_AGE_MS
            && verify_sync_response(payload, &key)
    }

    pub fn accept_snapshot(
        &mut self,
        payload: WirePayload,
        me: &UserProfile,
        now: Instant,
        now_ms: i64,
    ) -> Option<Vec<BotEvent>> {
        if self.state.is_some() || !self.vouched(&payload, now_ms) {
            return None;
        }
        let WirePayload::SyncResponse { server: mut remote, members, profiles, recent_messages, voice, at, .. } = payload else {
            return None;
        };
        remote.members = members.into_iter().map(member_of).collect();
        let sid = remote.id;
        let mut state = ServerState::new(remote);
        state.set_action_floor(at);
        for profile in profiles {
            state.profiles.insert(profile.id, profile.as_seen_by_others());
        }
        let entry = state.profiles.entry(me.id).or_insert_with(|| me.clone());
        entry.identity = me.identity.or(entry.identity);
        if let Err(e) = state.apply(ServerEvent::MemberJoin { server: sid, profile: me.clone() }) {
            tracing::warn!("could not join {} from its snapshot: {e}", state.server.name);
        }
        let mut fresh_ids = Vec::new();
        for message in recent_messages {
            let id = message.id;
            if state.apply(ServerEvent::MessageCreate { message }) == Ok(true) {
                fresh_ids.push(id);
            }
        }
        for entry in voice.into_iter().filter(|v| v.user != me.id) {
            let _ = state.apply(entry.into_event());
        }
        let mut fresh = vec![BotEvent::Ready];
        fresh.extend(fresh_ids.into_iter().filter_map(|id| state.message(id).cloned()).map(BotEvent::MessageCreate));
        self.state = Some(state);
        let early: VecDeque<Pending> = self.early.drain(..).filter(|(at, ..)| now.duration_since(*at) < PENDING_FOR).collect();
        fresh.extend(self.run(early, now));
        Some(fresh)
    }

    pub fn apply(&mut self, event: ServerEvent, action: Option<Action>, now: Instant) -> Vec<BotEvent> {
        if self.state.is_none() {
            keep(&mut self.early, (now, event, action));
            return Vec::new();
        }
        self.run(VecDeque::from([(now, event, action)]), now)
    }

    pub fn apply_caught_up(
        &mut self,
        messages: Vec<Message>,
        tombstones: Vec<TombstoneProof>,
        now: Instant,
    ) -> (ApplyReport, Vec<BotEvent>) {
        let Some(state) = self.state.as_mut() else {
            let rejected = messages.iter().map(|m| m.id).chain(tombstones.iter().map(|p| p.message)).collect();
            return (ApplyReport { rejected, ..ApplyReport::default() }, Vec::new());
        };
        let (report, held) = apply_items(state, messages, tombstones);
        let fresh = report.applied.iter().filter_map(|id| state.message(*id).cloned()).map(BotEvent::MessageCreate).collect();
        for event in held {
            keep(&mut self.pending, (now, event, None));
        }
        (report, fresh)
    }

    pub fn learn_profiles(&mut self, profiles: Vec<UserProfile>, now: Instant) -> Vec<BotEvent> {
        let Some(state) = self.state.as_mut() else { return Vec::new() };
        if apply_profiles(state, profiles).is_empty() {
            return Vec::new();
        }
        let waiting = self.release(now);
        self.run(waiting, now)
    }

    fn release(&mut self, now: Instant) -> VecDeque<Pending> {
        self.pending.retain(|(at, ..)| now.duration_since(*at) < PENDING_FOR);
        self.pending.drain(..).collect()
    }

    fn run(&mut self, mut queue: VecDeque<Pending>, now: Instant) -> Vec<BotEvent> {
        let mut fresh = Vec::new();
        let Some(state) = self.state.as_mut() else { return fresh };
        while let Some((at, event, action)) = queue.pop_front() {
            let (applied, happened, pinned) = apply_live(state, event.clone(), action);
            match applied {
                Err(StoreError::UnknownAuthorKey(_)) => keep(&mut self.pending, (at, event, action)),
                Err(_) if awaits_create(&self.pending, &event) => keep(&mut self.pending, (at, event, action)),
                Err(e) => tracing::debug!("rejected an event: {e}"),
                Ok(_) => {}
            }
            fresh.extend(happened);
            if pinned {
                self.pending.retain(|(at, ..)| now.duration_since(*at) < PENDING_FOR);
                queue.extend(self.pending.drain(..));
            }
        }
        fresh
    }

    pub fn has(&self, id: MessageId) -> bool {
        self.state.as_ref().is_some_and(|s| s.has_message(id))
    }

    pub fn signed(&self, me: &UserProfile, identity: &Identity, event: ServerEvent, at_ms: i64) -> WirePayload {
        if action::self_signed(&event) {
            return WirePayload::Event(event);
        }
        WirePayload::Signed(SignedEvent::sign(self.server(), me.id, at_ms, event, identity))
    }

    pub fn apply_own(&mut self, payload: &WirePayload) {
        let Some(state) = self.state.as_mut() else { return };
        let result = match payload {
            WirePayload::Signed(signed) => state.apply_own(signed),
            WirePayload::Event(event) => state.apply(event.clone()),
            _ => return,
        };
        if let Err(e) = result {
            tracing::debug!("our own event did not apply: {e}");
        }
    }

    pub fn banned(&self, me: commy_core::UserId) -> bool {
        self.state.as_ref().is_some_and(|s| s.server.banned.contains(&me))
    }

    pub fn pending(&self) -> usize {
        self.pending.len() + self.early.len()
    }
}

fn apply_live(state: &mut ServerState, event: ServerEvent, action: Option<Action>) -> (Result<bool, StoreError>, Option<BotEvent>, bool) {
    let before = Before::of(state, &event);
    let keyless = match &event {
        ServerEvent::MemberJoin { profile, .. } | ServerEvent::ProfileUpdate { profile } => {
            Some(profile.id).filter(|user| state.identity_of(*user).is_none())
        }
        _ => None,
    };
    let applied = match action {
        Some(action) => state.apply_signed(&SignedEvent { action, event }),
        None => state.apply_unsigned(event),
    };
    let pinned = keyless.is_some_and(|user| applied.is_ok() && state.identity_of(user).is_some());
    let happened = matches!(applied, Ok(true)).then(|| before.after(state)).flatten();
    (applied, happened, pinned)
}

enum Before {
    Nothing,
    Created(MessageId),
    Edited(MessageId),
    Deleted(commy_core::ChannelId, MessageId),
    Reacted(commy_core::ChannelId, MessageId, UserId, String),
    Joined(UserId),
    Left(UserId),
}

impl Before {
    fn of(state: &ServerState, event: &ServerEvent) -> Self {
        use ServerEvent as E;
        let server = &state.server;
        match event {
            E::MessageCreate { message } if !state.has_message(message.id) => Before::Created(message.id),
            E::MessageEdit { message, .. } => Before::Edited(*message),
            E::MessageDelete { channel, message, .. } if state.has_message(*message) => Before::Deleted(*channel, *message),
            E::ReactionSet { channel, message, emoji, user, .. } | E::ReactionToggle { channel, message, emoji, user } => {
                Before::Reacted(*channel, *message, *user, emoji.clone())
            }
            E::MemberJoin { profile, .. } if !server.is_current_member(profile.id) => Before::Joined(profile.id),
            E::MemberLeave { user, .. } | E::MemberBan { user, .. } if server.is_current_member(*user) => Before::Left(*user),
            _ => Before::Nothing,
        }
    }

    fn after(self, state: &ServerState) -> Option<BotEvent> {
        Some(match self {
            Before::Created(id) => BotEvent::MessageCreate(state.message(id)?.clone()),
            Before::Edited(id) => BotEvent::MessageUpdate(state.message(id)?.clone()),
            Before::Deleted(channel, message) => BotEvent::MessageDelete { channel, message },
            Before::Reacted(channel, message, user, emoji) => {
                let on = state.message(message)?.reactions.iter().any(|r| r.emoji == emoji && r.users.contains(&user));
                match on {
                    true => BotEvent::ReactionAdd { channel, message, user, emoji },
                    false => BotEvent::ReactionRemove { channel, message, user, emoji },
                }
            }
            Before::Joined(user) if state.server.is_current_member(user) => BotEvent::MemberJoin(user),
            Before::Left(user) if !state.server.is_current_member(user) => BotEvent::MemberLeave(user),
            Before::Nothing | Before::Joined(_) | Before::Left(_) => return None,
        })
    }
}

fn keep(queue: &mut VecDeque<Pending>, item: Pending) {
    if queue.len() >= PENDING_CAP {
        queue.pop_front();
    }
    queue.push_back(item);
}

fn awaits_create(pending: &VecDeque<Pending>, event: &ServerEvent) -> bool {
    let ServerEvent::MessageEdit { message: id, .. } = event else { return false };
    pending.iter().any(|(_, e, _)| matches!(e, ServerEvent::MessageCreate { message } if message.id == *id))
}

fn member_of(m: SyncMember) -> commy_core::Member {
    let joined_at = chrono::DateTime::from_timestamp_millis(m.joined_ms).unwrap_or_default();
    commy_core::Member { user: m.user, nickname: m.nickname, roles: m.roles, joined_at, timeout: None }
}
