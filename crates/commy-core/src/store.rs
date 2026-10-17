use crate::action::{self, SignedEvent};
use crate::event::ServerEvent;
use crate::identity::{IdentityKey, Signature};
use crate::ids::{ChannelId, MessageId, UserId};
use crate::message::{
    Message, Reaction, TombstoneProof, MAX_MESSAGE_BYTES, MAX_RELAYED_REACTIONS, MAX_RELAYED_REACTORS,
};
use crate::server::{ChannelKind, Member, Server};
use crate::user::{Presence, UserProfile};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StoreError {
    #[error("unknown channel")]
    UnknownChannel,
    #[error("unknown message")]
    UnknownMessage,
    #[error("duplicate message id")]
    DuplicateMessage,
    #[error("event does not belong to this server")]
    WrongServer,
    #[error("profile carries a different identity key than the pinned one")]
    IdentityMismatch,
    #[error("no identity key known for user {0}")]
    UnknownAuthorKey(UserId),
    #[error("bad signature")]
    BadSignature,
    #[error("dated too far ahead")]
    TooFarAhead,
    #[error("message id is not bound to its author")]
    UnboundMessageId,
    #[error("message too large")]
    TooLarge,
    #[error("too many deletes not applied here from one signer")]
    ProofLimit,
    #[error("delete names another channel than the message's")]
    WrongChannel,
    #[error("that channel already has as many pinned messages as it can hold")]
    PinLimit,
    #[error("only members allowed to post announcements can write in that channel")]
    NotAnnouncer,
    #[error("the signer may not do that here")]
    NotAllowed,
    #[error("unsigned event from a member whose key is known")]
    Unsigned,
    #[error("that server already holds as many of those as it can")]
    Limit,
}

/// Days a delete proof is kept, well past the catch-up window.
pub const TOMBSTONE_KEEP_DAYS: i64 = 60;

/// Most delete proofs kept from one signer for messages not held here.
/// Every replica keeps the same best ranked ones, the earliest first: a
/// better one pushes out the signer's worst, a worse one is refused.
/// Catch-up lists no more of a signer's proofs either, applied here or
/// not, see [`ServerState::tombstones_since`].
pub const MAX_UNAPPLIED_PROOFS: usize = 2000;

/// How far ahead of our clock a signed time may be, in ms.
pub const MAX_AHEAD_MS: i64 = 86_400_000;

pub const MAX_ACTION_TIMES: usize = 16384;
pub const MAX_ACTION_AHEAD_MS: i64 = 3_600_000;
pub const MAX_ACTION_FLOOR_MS: i64 = 900_000;

pub const MAX_MEMBERS: usize = 10_000;
pub const MAX_PROFILES: usize = 2 * MAX_MEMBERS;
pub const MAX_CHANNELS: usize = 500;
pub const MAX_ROLES: usize = 250;
pub const MAX_BANS: usize = 10_000;
pub const MAX_REACTIONS_PER_MESSAGE: usize = MAX_RELAYED_REACTIONS;
pub const MAX_REACTORS_PER_REACTION: usize = MAX_RELAYED_REACTORS;
pub const MAX_LIVE_ENTRIES: usize = MAX_MEMBERS;
pub const MAX_RSVPS_PER_EVENT: usize = MAX_MEMBERS;
pub const MAX_TOMBSTONED_IDS: usize = 5_000;

/// A delete proof as held here. `accepted_for` is the author of the
/// message the proof removed here, or kept out when it arrived later. A
/// proof without it is for a message not held here.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredProof {
    proof: TombstoneProof,
    accepted_for: Option<UserId>,
}

/// Where a proof ranks among its signer's, best first: the earliest,
/// then the lowest signature. Message and channel find it.
type UnappliedKey = (DateTime<Utc>, [u8; 64], MessageId, ChannelId);

/// Replicated state of one server, held in memory by every member.
/// Events are idempotent where possible so that gossip redelivery
/// does not corrupt state.
#[derive(Debug, Clone)]
pub struct ServerState {
    pub server: Server,
    pub profiles: HashMap<UserId, UserProfile>,
    messages: HashMap<ChannelId, Vec<Message>>,
    /// Channel of each message held, by id.
    index: HashMap<MessageId, ChannelId>,
    /// user -> voice channel they are connected to.
    pub voice_states: HashMap<UserId, ChannelId>,
    /// user -> when they joined their current voice channel, unix seconds.
    pub voice_since: HashMap<UserId, i64>,
    /// user -> (muted, deafened) as they last announced it.
    pub voice_flags: HashMap<UserId, (bool, bool)>,
    /// user -> voice channel they are sharing their screen in.
    pub screen_shares: HashMap<UserId, ChannelId>,
    /// user -> voice channel they have their camera on in.
    pub cameras: HashMap<UserId, ChannelId>,
    /// Ids of messages that stay deleted for good, so a late copy cannot
    /// bring one back: deleted before deletes were signed, or whose
    /// accepted delete proof was pruned.
    deleted_messages: HashSet<MessageId>,
    /// Signed deletes by message id, at most one per deleter and channel.
    /// They keep late copies out; catch-up relays them, see
    /// [`Self::tombstones_since`].
    tombstones: HashMap<MessageId, Vec<StoredProof>>,
    /// Per signer, their proofs not applied here, see
    /// [`MAX_UNAPPLIED_PROOFS`]. Built again when None, which every change
    /// to the proofs other than [`Self::store_proof`] sets.
    unapplied: Option<HashMap<UserId, BTreeSet<UnappliedKey>>>,
    action_times: HashMap<String, i64>,
    action_floor: i64,
    signed_actions: HashMap<String, SignedEvent>,
}

/// Durable image of a server, written to disk between sessions.
/// Transient state (voice, screen shares, presence) is not included.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub server: Server,
    pub profiles: Vec<UserProfile>,
    pub messages: Vec<(ChannelId, Vec<Message>)>,
    #[serde(default)]
    pub deleted_messages: Vec<MessageId>,
    #[serde(default)]
    pub tombstone_proofs: Vec<TombstoneProof>,
    /// (message, deleter, channel, author) of each proof this peer
    /// accepted.
    #[serde(default)]
    pub accepted: Vec<(MessageId, UserId, ChannelId, UserId)>,
    #[serde(default)]
    pub action_times: Vec<(String, i64)>,
    #[serde(default)]
    pub action_floor: i64,
    #[serde(default)]
    pub signed_actions: Vec<SignedEvent>,
}

impl ServerState {
    pub fn new(server: Server) -> Self {
        let messages = server
            .channels
            .iter()
            .map(|c| (c.id, Vec::new()))
            .collect();
        Self {
            server,
            profiles: HashMap::new(),
            messages,
            index: HashMap::new(),
            voice_states: HashMap::new(),
            voice_since: HashMap::new(),
            voice_flags: HashMap::new(),
            screen_shares: HashMap::new(),
            cameras: HashMap::new(),
            deleted_messages: Default::default(),
            tombstones: HashMap::new(),
            unapplied: None,
            action_times: HashMap::new(),
            action_floor: 0,
            signed_actions: HashMap::new(),
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            server: self.server.clone(),
            profiles: self.profiles.values().cloned().collect(),
            messages: self
                .messages
                .iter()
                .map(|(id, list)| (*id, list.clone()))
                .collect(),
            deleted_messages: self.deleted_messages.iter().copied().collect(),
            tombstone_proofs: self.stored_proofs().map(|s| s.proof.clone()).collect(),
            accepted: self
                .stored_proofs()
                .filter_map(|s| {
                    let p = &s.proof;
                    s.accepted_for.map(|author| (p.message, p.by, p.channel, author))
                })
                .collect(),
            action_times: self.action_times.iter().map(|(s, t)| (s.clone(), *t)).collect(),
            action_floor: self.action_floor,
            signed_actions: self.signed_actions.values().cloned().collect(),
        }
    }

    /// Loads a saved image as-is. Nothing is verified here: local data
    /// is trusted, including unsigned messages and unbound ids from
    /// older versions.
    pub fn from_snapshot(snapshot: Snapshot) -> Self {
        let mut state = Self::new(snapshot.server);
        for profile in snapshot.profiles {
            let mut profile = profile;
            // Nobody is online at load time until presence arrives.
            profile.presence = Presence::Offline;
            state.profiles.insert(profile.id, profile);
        }
        state.deleted_messages = snapshot.deleted_messages.into_iter().collect();
        let accepted: HashMap<(MessageId, UserId, ChannelId), UserId> = snapshot
            .accepted
            .into_iter()
            .map(|(message, by, channel, author)| ((message, by, channel), author))
            .collect();
        for proof in snapshot.tombstone_proofs {
            // Proofs for channels deleted since are of no use.
            if state.messages.contains_key(&proof.channel) {
                let accepted_for = accepted.get(&(proof.message, proof.by, proof.channel)).copied();
                state.store_proof(proof, accepted_for);
            }
        }
        for (channel, mut list) in snapshot.messages {
            // Only keep channels that still exist on the server.
            let Some(slot) = state.messages.get_mut(&channel) else { continue };
            // Ids are unique; older versions let one in twice.
            list.retain(|m| {
                let new = !state.index.contains_key(&m.id);
                if new {
                    state.index.insert(m.id, channel);
                }
                new
            });
            slot.extend(list);
        }
        state.action_times = snapshot.action_times.into_iter().collect();
        state.action_floor = snapshot.action_floor;
        for signed in snapshot.signed_actions {
            if let Some(subject) = action::subject(&signed.event) {
                if state.action_times.get(&subject) == Some(&signed.action.at) {
                    state.signed_actions.insert(subject, signed);
                }
            }
        }
        state
    }

    /// Drops messages older than each channel's retention window and
    /// returns how many were removed. Every peer runs this locally, so
    /// expired history disappears everywhere without a delete event.
    /// Delete proofs older than [`TOMBSTONE_KEEP_DAYS`] go too, as do
    /// proofs for channels that no longer exist and proofs dated more
    /// than [`MAX_AHEAD_MS`] ahead. A message such a proof removed here
    /// stays deleted.
    pub fn prune_expired(&mut self, now: DateTime<Utc>) -> usize {
        let mut removed = 0;
        for channel in &self.server.channels {
            let Some(cutoff) = retention_cutoff(channel, now) else { continue };
            if let Some(list) = self.messages.get_mut(&channel.id) {
                let before = list.len();
                list.retain(|m| {
                    let keep = m.created_at >= cutoff;
                    if !keep {
                        self.index.remove(&m.id);
                    }
                    keep
                });
                removed += before - list.len();
            }
        }
        let keep_from = now - chrono::Duration::days(TOMBSTONE_KEEP_DAYS);
        let Self { messages, tombstones, deleted_messages, unapplied, .. } = self;
        let pruned = retain_proofs(tombstones, |s| {
            let p = &s.proof;
            let channel = messages.contains_key(&p.channel);
            let keep = channel && p.at >= keep_from && !too_far_ahead(p.at, now);
            if !keep && channel && s.accepted_for.is_some() {
                deleted_messages.insert(p.message);
            }
            keep
        });
        if pruned {
            *unapplied = None;
        }
        removed
    }

    /// Oldest creation time a channel still keeps, or None when it keeps
    /// everything (or does not exist).
    pub fn retention_cutoff(&self, channel: ChannelId, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.server.channel(channel).and_then(|c| retention_cutoff(c, now))
    }

    /// Pinned identity key of a user, if one is known.
    pub fn identity_of(&self, user: UserId) -> Option<IdentityKey> {
        self.profiles.get(&user).and_then(|p| p.identity)
    }

    /// Messages in `channel` created at or after `since`, oldest first.
    pub fn messages_since(&self, channel: ChannelId, since: DateTime<Utc>) -> impl Iterator<Item = &Message> {
        self.messages(channel).iter().filter(move |m| m.created_at >= since)
    }

    /// Looks a message up by id in any channel.
    pub fn message(&self, id: MessageId) -> Option<&Message> {
        let channel = self.index.get(&id)?;
        self.messages.get(channel)?.iter().find(|m| m.id == id)
    }

    pub fn has_message(&self, id: MessageId) -> bool {
        self.message(id).is_some()
    }

    /// Best delete proof stored for a message id, if any. See
    /// [`Self::tombstones_for`] for the order.
    /// Ids of messages deleted here, with or without a proof still held.
    pub fn deleted_ids(&self) -> impl Iterator<Item = MessageId> + '_ {
        self.deleted_messages.iter().chain(self.tombstones.keys()).copied()
    }

    pub fn tombstone(&self, id: MessageId) -> Option<&TombstoneProof> {
        let list = self.tombstones.get(&id)?;
        list.iter().min_by_key(|s| proof_rank(s)).map(|s| &s.proof)
    }

    /// Every delete proof stored for a message id, one per deleter and
    /// channel, best first: accepted here, then the earliest, then the
    /// lowest signature.
    pub fn tombstones_for(&self, id: MessageId) -> Vec<&TombstoneProof> {
        let mut list: Vec<&StoredProof> = self.tombstones.get(&id).into_iter().flatten().collect();
        list.sort_by_key(|s| proof_rank(s));
        list.into_iter().map(|s| &s.proof).collect()
    }

    /// Delete proofs listed here made at or after `since`, in no
    /// particular order: each removed a message here, or is for a message
    /// not held here. Per signer only the best ranked
    /// [`MAX_UNAPPLIED_PROOFS`] of those are listed (see [`UnappliedKey`]),
    /// so every replica holding them lists the same; the rest still keep
    /// their messages out here. Catch-up lists and relays only these. A
    /// message deleted by several people has one proof for each.
    pub fn tombstones_since(&self, since: DateTime<Utc>) -> impl Iterator<Item = &TombstoneProof> {
        let listed = self.listed();
        self.stored_proofs()
            .filter(move |s| s.proof.at >= since && listed(s))
            .map(|s| &s.proof)
    }

    /// The proofs listed here (see [`Self::tombstones_since`]) for `ids`,
    /// by id in the order given, each id once and its proofs best first.
    pub fn valid_tombstones_for(&self, ids: &[MessageId]) -> Vec<&TombstoneProof> {
        let listed = self.listed();
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for id in ids.iter().filter(|id| seen.insert(**id)) {
            let mut list: Vec<&StoredProof> =
                self.tombstones.get(id).into_iter().flatten().filter(|s| listed(s)).collect();
            list.sort_by_key(|s| proof_rank(s));
            out.extend(list.into_iter().map(|s| &s.proof));
        }
        out
    }

    fn stored_proofs(&self) -> impl Iterator<Item = &StoredProof> {
        self.tombstones.values().flatten()
    }

    /// Which stored proofs are listed, see [`Self::tombstones_since`].
    fn listed(&self) -> impl Fn(&StoredProof) -> bool {
        let held = self.held_with_proofs();
        let mut by_signer: HashMap<UserId, Vec<UnappliedKey>> = HashMap::new();
        for s in self.stored_proofs().filter(|s| valid(s, &held)) {
            by_signer.entry(s.proof.by).or_default().push(unapplied_key(&s.proof));
        }
        let mut beyond: HashSet<(UserId, MessageId, ChannelId)> = HashSet::new();
        for (by, mut keys) in by_signer.into_iter().filter(|(_, keys)| keys.len() > MAX_UNAPPLIED_PROOFS) {
            keys.sort_unstable();
            beyond.extend(keys.drain(MAX_UNAPPLIED_PROOFS..).map(|(_, _, message, channel)| (by, message, channel)));
        }
        move |s: &StoredProof| {
            let p = &s.proof;
            valid(s, &held) && !beyond.contains(&(p.by, p.message, p.channel))
        }
    }

    /// Ids of messages held here that proofs are stored for. Only data
    /// saved by older versions has such proofs.
    fn held_with_proofs(&self) -> HashSet<MessageId> {
        self.tombstones.keys().filter(|id| self.has_message(**id)).copied().collect()
    }

    pub fn messages(&self, channel: ChannelId) -> &[Message] {
        self.messages.get(&channel).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Latest `limit` messages, oldest first, optionally before a given id.
    pub fn page(&self, channel: ChannelId, before: Option<MessageId>, limit: usize) -> &[Message] {
        let all = self.messages(channel);
        let end = match before {
            Some(id) => all.iter().position(|m| m.id == id).unwrap_or(all.len()),
            None => all.len(),
        };
        let start = end.saturating_sub(limit);
        &all[start..end]
    }

    pub fn admits_signed(&self, signed: &SignedEvent) -> Result<bool, StoreError> {
        let by = signed.action.by;
        let key = self.signer_key(signed).ok_or(StoreError::UnknownAuthorKey(by))?;
        if !signed.verify(self.server.id, &key) {
            return Err(StoreError::BadSignature);
        }
        if signed.action.at.saturating_sub(Utc::now().timestamp_millis()) > MAX_ACTION_AHEAD_MS {
            return Err(StoreError::TooFarAhead);
        }
        if !action::authorized(&self.server, by, &signed.event) {
            return Err(StoreError::NotAllowed);
        }
        Ok(self.fresh(action::subject(&signed.event).as_deref(), signed.action.at))
    }

    pub fn admits_unsigned(&self, event: &ServerEvent) -> Result<(), StoreError> {
        if action::self_signed(event) {
            return Ok(());
        }
        let actor = action::actor(event).filter(|user| self.identity_of(*user).is_none());
        match actor {
            Some(actor) if action::may_go_unsigned(event) && action::authorized(&self.server, actor, event) => Ok(()),
            _ => Err(StoreError::Unsigned),
        }
    }

    pub fn apply_signed(&mut self, signed: &SignedEvent) -> Result<bool, StoreError> {
        if !self.admits_signed(signed)? {
            return Ok(false);
        }
        let applied = self.apply(signed.event.clone())?;
        if applied || self.target_known(&signed.event) {
            self.note_action(action::subject(&signed.event), signed.action.at);
            self.keep_signed(signed);
        }
        Ok(applied)
    }

    pub fn apply_unsigned(&mut self, event: ServerEvent) -> Result<bool, StoreError> {
        self.admits_unsigned(&event)?;
        self.apply(event)
    }

    pub fn apply_own(&mut self, signed: &SignedEvent) -> Result<bool, StoreError> {
        if !self.fresh(action::subject(&signed.event).as_deref(), signed.action.at) {
            return Ok(false);
        }
        let applied = self.apply(signed.event.clone())?;
        if applied || self.target_known(&signed.event) {
            self.note_action(action::subject(&signed.event), signed.action.at);
            self.keep_signed(signed);
        }
        Ok(applied)
    }

    fn target_known(&self, event: &ServerEvent) -> bool {
        use ServerEvent as E;
        let server = &self.server;
        match event {
            E::ChannelDelete { channel, .. }
            | E::ChannelUpdate { channel, .. }
            | E::ChannelLayout { channel, .. }
            | E::ChannelLimit { channel, .. }
            | E::ChannelBanner { channel, .. }
            | E::ChannelColor { channel, .. }
            | E::ChannelKindSet { channel, .. } => server.channels.iter().any(|c| c.id == *channel),
            E::RoleUpdate { role, .. } => server.roles.iter().any(|r| r.id == role.id),
            E::RoleDelete { role, .. } | E::RoleAssign { role, .. } | E::RoleUnassign { role, .. } => {
                server.roles.iter().any(|r| r.id == *role)
            }
            E::MemberBan { user, .. } | E::MemberUnban { user, .. } | E::MemberLeave { user, .. } => {
                server.is_member(*user) || server.banned.contains(user) || self.profiles.contains_key(user)
            }
            E::EventDelete { event, .. } | E::EventRsvp { event, .. } => server.events.iter().any(|e| e.id == *event),
            E::EmojiRemove { name, .. } => server.emojis.iter().any(|e| e.name == *name),
            E::SoundRemove { name, .. } => server.soundboard.iter().any(|s| s.name == *name),
            E::MessagePin { message, .. } | E::ReactionSet { message, .. } | E::ReactionToggle { message, .. } => {
                self.has_message(*message)
            }
            _ => true,
        }
    }

    pub fn set_action_floor(&mut self, at: i64) {
        self.action_floor = self.action_floor.max(at);
    }

    pub fn action_floor(&self) -> i64 {
        self.action_floor
    }

    fn fresh(&self, subject: Option<&str>, at: i64) -> bool {
        let Some(subject) = subject else { return true };
        match self.action_times.get(subject) {
            Some(prev) => at > *prev,
            None => {
                !action::settles_authority(subject)
                    || action::snapshot_omits(subject)
                    || at + MAX_ACTION_FLOOR_MS >= self.action_floor
            }
        }
    }

    fn signer_key(&self, signed: &SignedEvent) -> Option<IdentityKey> {
        let by = signed.action.by;
        if let Some(key) = self.identity_of(by) {
            return Some(key);
        }
        match &signed.event {
            ServerEvent::MemberJoin { profile, .. } | ServerEvent::ProfileUpdate { profile } if profile.id == by => {
                profile.identity
            }
            _ => None,
        }
    }

    fn note_action(&mut self, subject: Option<String>, at: i64) {
        let Some(subject) = subject else { return };
        self.action_times.insert(subject, at);
        while self.action_times.len() > MAX_ACTION_TIMES {
            let oldest = |authority: bool| {
                self.action_times
                    .iter()
                    .filter(|(s, _)| action::settles_authority(s) == authority)
                    .min_by_key(|(_, t)| **t)
                    .map(|(s, _)| s.clone())
            };
            let Some(evicted) = oldest(false).or_else(|| oldest(true)) else { break };
            self.action_times.remove(&evicted);
            self.signed_actions.remove(&evicted);
        }
    }

    fn keep_signed(&mut self, signed: &SignedEvent) {
        let Some(subject) = action::subject(&signed.event) else { return };
        if !self.action_times.contains_key(&subject) {
            return;
        }
        if action::relayable(&signed.event) {
            self.signed_actions.insert(subject, signed.clone());
        } else {
            self.signed_actions.remove(&subject);
        }
    }

    pub fn signed_actions(&self) -> Vec<&SignedEvent> {
        let mut list: Vec<&SignedEvent> = self.signed_actions.values().collect();
        list.sort_by_key(|s| s.action.at);
        list
    }

    pub fn holds_signed_action(&self, subject: &str) -> bool {
        self.signed_actions.contains_key(subject)
    }

    /// Applies an event to local state. Returns Ok(false) when the event
    /// was a duplicate or no-op (safe to ignore on gossip redelivery).
    pub fn apply(&mut self, event: ServerEvent) -> Result<bool, StoreError> {
        match event {
            ServerEvent::MessageCreate { message } => self.create_message(message),
            ServerEvent::MessageEdit {
                channel,
                message,
                content,
                edited_at,
                sig,
            } => self.edit_message(channel, message, content, edited_at, sig),
            ServerEvent::MessageDelete {
                channel,
                message,
                by,
                at,
                sig,
            } => self.delete_message(TombstoneProof { channel, message, by, at, sig }),
            ServerEvent::ReactionToggle {
                channel,
                message,
                emoji,
                user,
            } => {
                let msg = self.find_message_mut(channel, message)?;
                if !reaction_room(msg, &emoji, user) {
                    return Err(StoreError::Limit);
                }
                msg.toggle_reaction(&emoji, user);
                Ok(true)
            }
            ServerEvent::ReactionSet {
                channel,
                message,
                emoji,
                user,
                active,
            } => {
                if !crate::server::valid_reaction(&emoji) {
                    return Ok(false);
                }
                let msg = self.find_message_mut(channel, message)?;
                let has = msg.reactions.iter().any(|r| r.emoji == emoji && r.users.contains(&user));
                if has == active {
                    return Ok(false);
                }
                if active && !reaction_room(msg, &emoji, user) {
                    return Err(StoreError::Limit);
                }
                msg.toggle_reaction(&emoji, user);
                Ok(true)
            }
            ServerEvent::MemberJoin { server, mut profile } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                if self.server.banned.contains(&profile.id) {
                    return Ok(false);
                }
                self.pin_identity(&mut profile)?;
                profile = profile.as_seen_by_others();
                if self.server.is_member(profile.id) {
                    self.profiles.insert(profile.id, profile);
                    return Ok(false);
                }
                if self.server.members.len() >= MAX_MEMBERS {
                    return Err(StoreError::Limit);
                }
                self.make_profile_room(profile.id)?;
                self.server.members.push(Member {
                    user: profile.id,
                    nickname: None,
                    roles: vec![],
                    joined_at: Utc::now(),
                });
                self.profiles.insert(profile.id, profile);
                Ok(true)
            }
            ServerEvent::MemberLeave { server, user } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                let before = self.server.members.len();
                self.server.members.retain(|m| m.user != user);
                self.voice_states.remove(&user);
                self.voice_since.remove(&user);
                self.voice_flags.remove(&user);
                Ok(self.server.members.len() != before)
            }
            ServerEvent::PresenceUpdate { user, presence } => {
                let presence = presence.as_seen_by_others();
                if let Some(profile) = self.profiles.get_mut(&user) {
                    if profile.presence == presence {
                        return Ok(false);
                    }
                    profile.presence = presence;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            ServerEvent::ChannelCreate { server, channel } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                if self.server.channel(channel.id).is_some()
                    || self.server.deleted_channels.contains(&channel.id)
                {
                    return Ok(false);
                }
                if self.server.channels.len() >= MAX_CHANNELS {
                    return Err(StoreError::Limit);
                }
                self.messages.insert(channel.id, Vec::new());
                self.server.channels.push(channel);
                Ok(true)
            }
            ServerEvent::ChannelDelete { server, channel } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                let before = self.server.channels.len();
                self.server.channels.retain(|c| c.id != channel);
                self.server.pins.retain(|p| p.channel != channel);
                for m in self.messages.remove(&channel).into_iter().flatten() {
                    self.index.remove(&m.id);
                }
                if retain_proofs(&mut self.tombstones, |s| s.proof.channel != channel) {
                    self.unapplied = None;
                }
                let new_tombstone = !self.server.deleted_channels.contains(&channel);
                if new_tombstone {
                    self.server.deleted_channels.push(channel);
                    if self.server.deleted_channels.len() > MAX_TOMBSTONED_IDS {
                        self.server.deleted_channels.remove(0);
                    }
                }
                if self.server.afk_channel_id == Some(channel) {
                    self.server.afk_channel_id = None;
                }
                Ok(self.server.channels.len() != before || new_tombstone)
            }
            ServerEvent::ChannelUpdate {
                server,
                channel,
                name,
                retention_days,
            } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                let name = name.trim().to_string();
                if name.is_empty() {
                    return Ok(false);
                }
                let Some(target) = self.server.channels.iter_mut().find(|c| c.id == channel) else {
                    return Err(StoreError::UnknownChannel);
                };
                let retention_days = retention_days.filter(|d| *d > 0);
                if target.name == name && target.retention_days == retention_days {
                    return Ok(false);
                }
                target.name = name;
                target.retention_days = retention_days;
                self.prune_expired(Utc::now());
                Ok(true)
            }
            ServerEvent::AfkChannelSet { server, channel } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                if let Some(id) = channel {
                    match self.server.channel(id) {
                        Some(c) if c.kind == ChannelKind::Voice => {}
                        Some(_) => return Ok(false),
                        None => return Err(StoreError::UnknownChannel),
                    }
                }
                let disabled = channel.is_none();
                if self.server.afk_channel_id == channel && self.server.afk_disabled == disabled {
                    return Ok(false);
                }
                self.server.afk_channel_id = channel;
                self.server.afk_disabled = disabled;
                Ok(true)
            }
            ServerEvent::VoiceState { user, channel, since, muted, deafened } => match channel {
                Some(ch) => {
                    if self.server.channel(ch).is_none() {
                        return Err(StoreError::UnknownChannel);
                    }
                    if !self.voice_states.contains_key(&user) && self.voice_states.len() >= MAX_LIVE_ENTRIES {
                        return Err(StoreError::Limit);
                    }
                    let moved = self.voice_states.insert(user, ch) != Some(ch);
                    let flagged = self.voice_flags.insert(user, (muted, deafened)) != Some((muted, deafened));
                    if moved || !self.voice_since.contains_key(&user) {
                        // Their own join time, unless it is missing or ahead
                        // of our clock; then the moment we heard.
                        let now = Utc::now().timestamp();
                        self.voice_since.insert(user, since.filter(|s| *s <= now).unwrap_or(now));
                    }
                    Ok(moved || flagged)
                }
                None => {
                    // Leaving voice also ends any screen share or camera.
                    self.screen_shares.remove(&user);
                    self.cameras.remove(&user);
                    self.voice_since.remove(&user);
                    self.voice_flags.remove(&user);
                    Ok(self.voice_states.remove(&user).is_some())
                }
            },
            ServerEvent::ScreenShare {
                user,
                channel,
                active,
            } => {
                if active {
                    if self.server.channel(channel).is_none() {
                        return Err(StoreError::UnknownChannel);
                    }
                    if !self.screen_shares.contains_key(&user) && self.screen_shares.len() >= MAX_LIVE_ENTRIES {
                        return Err(StoreError::Limit);
                    }
                    Ok(self.screen_shares.insert(user, channel) != Some(channel))
                } else {
                    Ok(self.screen_shares.remove(&user).is_some())
                }
            }
            ServerEvent::CameraState {
                user,
                channel,
                active,
            } => {
                if active {
                    if self.server.channel(channel).is_none() {
                        return Err(StoreError::UnknownChannel);
                    }
                    if !self.cameras.contains_key(&user) && self.cameras.len() >= MAX_LIVE_ENTRIES {
                        return Err(StoreError::Limit);
                    }
                    Ok(self.cameras.insert(user, channel) != Some(channel))
                } else {
                    Ok(self.cameras.remove(&user).is_some())
                }
            }
            ServerEvent::EmojiAdd { server, emoji } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                if !crate::server::valid_expression_name(&emoji.name)
                    || crate::server::ImageRef::parse(&emoji.blob).is_none()
                {
                    return Ok(false);
                }
                let count = self.server.emojis.len();
                match self.server.emojis.iter_mut().find(|e| e.name == emoji.name) {
                    Some(existing) if *existing == emoji => Ok(false),
                    Some(existing) => {
                        *existing = emoji;
                        Ok(true)
                    }
                    None if count >= crate::server::MAX_EMOJIS => Ok(false),
                    None => {
                        self.server.emojis.push(emoji);
                        Ok(true)
                    }
                }
            }
            ServerEvent::EmojiRemove { server, name } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                let before = self.server.emojis.len();
                self.server.emojis.retain(|e| e.name != name);
                Ok(self.server.emojis.len() != before)
            }
            ServerEvent::SoundAdd { server, sound } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                if !crate::server::valid_expression_name(&sound.name)
                    || crate::server::ImageRef::parse(&sound.blob).is_none()
                    || !(0.0..=2.0).contains(&sound.volume)
                    || sound.emoji.as_deref().is_some_and(|e| !crate::server::valid_sound_emoji(e))
                {
                    return Ok(false);
                }
                let count = self.server.soundboard.len();
                match self.server.soundboard.iter_mut().find(|s| s.name == sound.name) {
                    Some(existing) if *existing == sound => Ok(false),
                    Some(existing) => {
                        *existing = sound;
                        Ok(true)
                    }
                    None if count >= crate::server::MAX_SOUNDS => Ok(false),
                    None => {
                        self.server.soundboard.push(sound);
                        Ok(true)
                    }
                }
            }
            ServerEvent::SoundRemove { server, name } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                let before = self.server.soundboard.len();
                self.server.soundboard.retain(|s| s.name != name);
                Ok(self.server.soundboard.len() != before)
            }
            ServerEvent::ExpressionSettings { server, emojis_shareable, soundboard_open } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                if self.server.emojis_shareable == emojis_shareable && self.server.soundboard_open == soundboard_open {
                    return Ok(false);
                }
                self.server.emojis_shareable = emojis_shareable;
                self.server.soundboard_open = soundboard_open;
                Ok(true)
            }
            ServerEvent::PublicSet { server, public } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                if self.server.public == public {
                    return Ok(false);
                }
                self.server.public = public;
                Ok(true)
            }
            // Played live by whoever is in the channel; nothing to store.
            ServerEvent::SoundboardPlay { .. } => Ok(false),
            ServerEvent::GameTagsSet { server, tags, meta } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                let Some(tags) = crate::server::clean_game_tags(&tags) else { return Ok(false) };
                let meta = crate::server::clean_game_tag_meta(&tags, &meta);
                if self.server.game_tags == tags && self.server.game_tag_meta == meta {
                    return Ok(false);
                }
                self.server.game_tags = tags;
                self.server.game_tag_meta = meta;
                Ok(true)
            }
            ServerEvent::RulesSet { server, rules } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                if rules.as_deref().is_some_and(|r| !crate::server::valid_rules(r)) || self.server.rules == rules {
                    return Ok(false);
                }
                self.server.rules = rules;
                Ok(true)
            }
            ServerEvent::RulesAccept { server, user } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                if self.server.rules_accepted.contains(&user) || !self.server.members.iter().any(|m| m.user == user) {
                    return Ok(false);
                }
                self.server.rules_accepted.push(user);
                Ok(true)
            }
            ServerEvent::EventUpsert { server, event } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                if !crate::server::valid_event(&event) || self.server.deleted_events.contains(&event.id) {
                    return Ok(false);
                }
                match self.server.events.iter_mut().find(|e| e.id == event.id) {
                    Some(existing) => {
                        // RSVPs are tracked by their own events; keep ours.
                        let updated = crate::server::ScheduledEvent { rsvps: existing.rsvps.clone(), ..event };
                        if *existing == updated {
                            return Ok(false);
                        }
                        *existing = updated;
                    }
                    None => {
                        if self.server.events.len() >= crate::server::MAX_EVENTS {
                            return Ok(false);
                        }
                        self.server.events.push(crate::server::ScheduledEvent { rsvps: Vec::new(), ..event });
                    }
                }
                self.server.events.sort_by_key(|e| e.start);
                Ok(true)
            }
            ServerEvent::EventDelete { server, event } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                let before = self.server.events.len();
                self.server.events.retain(|e| e.id != event);
                let new_tombstone = !self.server.deleted_events.contains(&event);
                if new_tombstone && event.len() <= 64 {
                    self.server.deleted_events.push(event);
                    if self.server.deleted_events.len() > MAX_TOMBSTONED_IDS {
                        self.server.deleted_events.remove(0);
                    }
                }
                Ok(self.server.events.len() != before)
            }
            ServerEvent::EventRsvp { server, event, user, rsvp } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                let Some(target) = self.server.events.iter_mut().find(|e| e.id == event) else { return Ok(false) };
                let current = target.rsvps.iter().find(|(u, _)| *u == user).map(|(_, r)| *r);
                if current == rsvp {
                    return Ok(false);
                }
                if current.is_none() && target.rsvps.len() >= MAX_RSVPS_PER_EVENT {
                    return Err(StoreError::Limit);
                }
                target.rsvps.retain(|(u, _)| *u != user);
                if let Some(rsvp) = rsvp {
                    target.rsvps.push((user, rsvp));
                }
                Ok(true)
            }
            ServerEvent::ChannelLayout { server, channel, parent, position } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                // A parent must be an existing category, and categories do not nest.
                let parent_ok = parent.is_none_or(|p| {
                    self.server.channel(p).is_some_and(|c| c.kind == crate::server::ChannelKind::Category)
                });
                let Some(target) = self.server.channels.iter_mut().find(|c| c.id == channel) else { return Ok(false) };
                if !parent_ok || (parent.is_some() && target.kind == crate::server::ChannelKind::Category) {
                    return Ok(false);
                }
                if target.parent == parent && target.position == position {
                    return Ok(false);
                }
                target.parent = parent;
                target.position = position;
                Ok(true)
            }
            ServerEvent::ChannelLimit { server, channel, user_limit } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                let Some(target) = self.server.channels.iter_mut().find(|c| c.id == channel) else { return Ok(false) };
                if target.kind != crate::server::ChannelKind::Voice
                    || user_limit > crate::server::MAX_USER_LIMIT
                    || target.user_limit == user_limit
                {
                    return Ok(false);
                }
                target.user_limit = user_limit;
                Ok(true)
            }
            ServerEvent::VoiceModerate { server, user, muted, deafened } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                // The owner cannot be muted or deafened by anyone.
                if user == self.server.owner {
                    return Ok(false);
                }
                let set = |list: &mut Vec<UserId>, on: bool| {
                    let had = list.contains(&user);
                    if on && !had {
                        list.push(user);
                    } else if !on && had {
                        list.retain(|u| *u != user);
                    }
                    had != on
                };
                let a = set(&mut self.server.server_muted, muted);
                let b = set(&mut self.server.server_deafened, deafened);
                Ok(a || b)
            }
            ServerEvent::MemberBan { server, user } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                if user == self.server.owner || self.server.banned.contains(&user) {
                    return Ok(false);
                }
                if self.server.banned.len() >= MAX_BANS {
                    return Err(StoreError::Limit);
                }
                self.server.banned.push(user);
                self.server.members.retain(|m| m.user != user);
                self.voice_states.remove(&user);
                self.voice_since.remove(&user);
                self.voice_flags.remove(&user);
                self.screen_shares.remove(&user);
                Ok(true)
            }
            ServerEvent::AfkTimeoutSet { server, minutes } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                if minutes.is_some_and(|m| !crate::server::SERVER_AFK_MINUTES.contains(&m)) || self.server.afk_timeout_minutes == minutes {
                    return Ok(false);
                }
                self.server.afk_timeout_minutes = minutes;
                Ok(true)
            }
            // Devices keep doorbells outside the replicated state.
            ServerEvent::DoorbellSet { .. } => Ok(false),
            ServerEvent::MemberUnban { server, user } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                let before = self.server.banned.len();
                self.server.banned.retain(|u| *u != user);
                Ok(self.server.banned.len() != before)
            }
            ServerEvent::RoleUpdate { server, role } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                if role.banner.as_deref().is_some_and(|b| crate::server::ImageRef::parse(b).is_none()) {
                    return Ok(false);
                }
                if role.tag.as_deref().is_some_and(|t| !crate::server::valid_role_tag(t)) || role.name.trim().is_empty() {
                    return Ok(false);
                }
                let Some(existing) = self.server.roles.iter_mut().find(|r| r.id == role.id) else { return Ok(false) };
                // The everyone role keeps its name and cannot be self-assigned.
                if existing.position == 0 && (role.name != existing.name || role.self_assignable) {
                    return Ok(false);
                }
                if *existing == role {
                    return Ok(false);
                }
                *existing = role;
                Ok(true)
            }
            ServerEvent::ChannelBanner { server, channel, banner } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                // Malformed refs from a peer are dropped rather than stored.
                let banner = banner.filter(|r| crate::server::ImageRef::parse(r).is_some());
                let Some(target) = self.server.channels.iter_mut().find(|c| c.id == channel) else {
                    return Err(StoreError::UnknownChannel);
                };
                if target.banner == banner {
                    return Ok(false);
                }
                target.banner = banner;
                Ok(true)
            }
            ServerEvent::ChannelColor { server, channel, color } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                let color = color
                    .map(|c| c.to_ascii_lowercase())
                    .filter(|c| c.len() == 6 && c.chars().all(|ch| ch.is_ascii_hexdigit()));
                let Some(target) = self.server.channels.iter_mut().find(|c| c.id == channel) else {
                    return Err(StoreError::UnknownChannel);
                };
                if target.color == color {
                    return Ok(false);
                }
                target.color = color;
                Ok(true)
            }
            ServerEvent::MessagePin { server, channel, message, by, at, pinned } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                if self.server.channel(channel).is_none() {
                    return Err(StoreError::UnknownChannel);
                }
                let held = self.server.pins.iter().position(|p| p.message == message);
                match (pinned, held) {
                    // The first pin stands: a redelivered or later copy keeps
                    // every replica showing the same pinner and time.
                    (true, Some(_)) => Ok(false),
                    (false, None) => Ok(false),
                    (false, Some(pos)) => {
                        self.server.pins.remove(pos);
                        Ok(true)
                    }
                    (true, None) => {
                        // Pins of a message held here must name its channel; a
                        // message not held yet may still arrive, so it is kept.
                        if self.index.get(&message).is_some_and(|c| *c != channel) {
                            return Err(StoreError::WrongChannel);
                        }
                        if self.deleted_messages.contains(&message) || self.tombstones.contains_key(&message) {
                            return Ok(false);
                        }
                        if too_far_ahead(at, Utc::now()) {
                            return Err(StoreError::TooFarAhead);
                        }
                        let count = self.server.pins.iter().filter(|p| p.channel == channel).count();
                        if count >= crate::server::MAX_PINS_PER_CHANNEL {
                            return Err(StoreError::PinLimit);
                        }
                        self.server.pins.push(crate::server::Pin { channel, message, by, at });
                        Ok(true)
                    }
                }
            }
            ServerEvent::ChannelKindSet { server, channel, kind } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                let Some(target) = self.server.channels.iter_mut().find(|c| c.id == channel) else {
                    return Err(StoreError::UnknownChannel);
                };
                // Only text channels become announcement channels and back.
                let text_like = |k: ChannelKind| matches!(k, ChannelKind::Text | ChannelKind::Announcement);
                if !text_like(kind) || !text_like(target.kind) || target.kind == kind {
                    return Ok(false);
                }
                target.kind = kind;
                Ok(true)
            }
            ServerEvent::ServerImages { server, icon, banner } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                // Malformed refs from a peer are dropped rather than stored.
                let clean = |v: Option<String>| v.filter(|r| crate::server::ImageRef::parse(r).is_some());
                let (icon, banner) = (clean(icon), clean(banner));
                if self.server.icon == icon && self.server.banner == banner {
                    return Ok(false);
                }
                self.server.icon = icon;
                self.server.banner = banner;
                Ok(true)
            }
            ServerEvent::ServerRename { server, name } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                if self.server.name == name || name.trim().is_empty() {
                    return Ok(false);
                }
                self.server.name = name;
                Ok(true)
            }
            ServerEvent::ProfileUpdate { mut profile } => {
                if !self.server.is_member(profile.id) {
                    return Ok(false);
                }
                self.pin_identity(&mut profile)?;
                // State holds presence as others see it, so a snapshot or
                // catch-up served from it never says who appears offline.
                profile = profile.as_seen_by_others();
                match self.profiles.get(&profile.id) {
                    Some(existing) if *existing == profile => Ok(false),
                    _ => {
                        self.profiles.insert(profile.id, profile);
                        Ok(true)
                    }
                }
            }
            ServerEvent::RoleCreate { server, role } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                if self.server.roles.iter().any(|r| r.id == role.id) {
                    return Ok(false);
                }
                // Position 0 is reserved for the implicit everyone role.
                if role.position == 0 {
                    return Ok(false);
                }
                if self.server.roles.len() >= MAX_ROLES {
                    return Err(StoreError::Limit);
                }
                self.server.roles.push(role);
                Ok(true)
            }
            ServerEvent::RoleDelete { server, role } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                let is_everyone = self
                    .server
                    .roles
                    .iter()
                    .any(|r| r.id == role && r.position == 0);
                if is_everyone {
                    return Ok(false);
                }
                let before = self.server.roles.len();
                self.server.roles.retain(|r| r.id != role);
                for member in &mut self.server.members {
                    member.roles.retain(|r| *r != role);
                }
                Ok(self.server.roles.len() != before)
            }
            ServerEvent::RoleAssign { server, user, role } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                if !self.server.roles.iter().any(|r| r.id == role) {
                    return Ok(false);
                }
                let Some(member) = self.server.members.iter_mut().find(|m| m.user == user)
                else {
                    return Ok(false);
                };
                if member.roles.contains(&role) {
                    return Ok(false);
                }
                member.roles.push(role);
                Ok(true)
            }
            ServerEvent::RoleUnassign { server, user, role } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                let Some(member) = self.server.members.iter_mut().find(|m| m.user == user)
                else {
                    return Ok(false);
                };
                let before = member.roles.len();
                member.roles.retain(|r| *r != role);
                Ok(member.roles.len() != before)
            }
            // Typing indicators are transient UI state, not stored.
            ServerEvent::TypingStart { .. } => Ok(true),
        }
    }

    /// Signed create with an id bound to its author, no larger than
    /// [`MAX_MESSAGE_BYTES`]. A copy of a message we already hold is taken
    /// only as a newer signed edit of it, see [`supersedes`]. A late copy
    /// kept out by a delete returns Ok(true) when checking it changed the
    /// stored proofs, so callers save them.
    fn create_message(&mut self, mut message: Message) -> Result<bool, StoreError> {
        // Otherwise a member could claim an id another member made first.
        if !message.id.is_bound_to(message.author) {
            return Err(StoreError::UnboundMessageId);
        }
        if message.authored_size() > MAX_MESSAGE_BYTES {
            return Err(StoreError::TooLarge);
        }
        let key = self
            .identity_of(message.author)
            .ok_or(StoreError::UnknownAuthorKey(message.author))?;
        if !message.verify(self.server.id, &key) {
            return Err(StoreError::BadSignature);
        }
        let now = Utc::now();
        if too_far_ahead(message.created_at, now) || message.edited_at.is_some_and(|t| too_far_ahead(t, now)) {
            return Err(StoreError::TooFarAhead);
        }
        if !self.messages.contains_key(&message.channel) {
            return Err(StoreError::UnknownChannel);
        }
        // Announcement channels take posts from announcers only; an edit
        // of a post already held is the author's and goes through.
        if !self.has_message(message.id)
            && self.server.channel(message.channel).is_some_and(|c| c.kind == ChannelKind::Announcement)
            && !self.server.can_post_in(message.channel, message.author)
        {
            return Err(StoreError::NotAnnouncer);
        }
        if !self.has_message(message.id) {
            if let Some(changed) = self.bind_proofs(&message) {
                return Ok(changed);
            }
        }
        if self.deleted_messages.contains(&message.id) {
            return Ok(false);
        }
        // A late copy of an already-expired message stays gone.
        if self.retention_cutoff(message.channel, now).is_some_and(|cutoff| message.created_at < cutoff) {
            return Ok(false);
        }
        let server = self.server.id;
        if let Some(existing) = self.message_mut(message.id) {
            let Some(sig) = &message.sig else { return Ok(false) };
            if !supersedes(existing, message.edited_at, sig) {
                return Ok(false);
            }
            // Signed fields other than these must match what we hold.
            let updated = Message {
                content: message.content,
                edited_at: message.edited_at,
                attachments: message.attachments,
                sig: message.sig,
                ..existing.clone()
            };
            if !updated.verify(server, &key) {
                return Ok(false);
            }
            *existing = updated;
            return Ok(true);
        }
        // None of its proofs kept it out: each names another channel, or
        // was accepted for another author. They count no more.
        if self.tombstones.remove(&message.id).is_some() {
            self.unapplied = None;
        }
        let list = self
            .messages
            .get_mut(&message.channel)
            .ok_or(StoreError::UnknownChannel)?;
        message.reactions = relayed_reactions(std::mem::take(&mut message.reactions));
        self.index.insert(message.id, message.channel);
        // Keep messages sorted by creation time for display.
        let pos = list.partition_point(|m| m.created_at <= message.created_at);
        list.insert(pos, message);
        Ok(true)
    }

    /// Checks the proofs stored for a message not held here. Each naming
    /// its channel keeps it out, unless accepted here for another author
    /// (only data saved before ids were bound can hold such a message),
    /// and so becomes accepted for its author; the others then go.
    /// Returns whether the stored proofs changed when one kept it out, or
    /// None when none did.
    fn bind_proofs(&mut self, message: &Message) -> Option<bool> {
        let list = self.tombstones.get_mut(&message.id)?;
        let author = message.author;
        let before = list.len();
        let (mut blocked, mut bound) = (false, false);
        for s in list.iter_mut() {
            if s.proof.channel == message.channel && s.accepted_for.is_none_or(|a| a == author) {
                bound |= s.accepted_for.is_none();
                s.accepted_for = Some(author);
                blocked = true;
            }
        }
        if !blocked {
            return None;
        }
        list.retain(|s| s.accepted_for.is_some());
        let changed = bound || list.len() != before;
        if changed {
            self.unapplied = None;
        }
        Some(changed)
    }

    /// Signed edit by the author; older edits than the stored one lose,
    /// see [`supersedes`]. The edited message must stay within
    /// [`MAX_MESSAGE_BYTES`].
    fn edit_message(
        &mut self,
        channel: ChannelId,
        id: MessageId,
        content: String,
        edited_at: DateTime<Utc>,
        sig: Signature,
    ) -> Result<bool, StoreError> {
        let server = self.server.id;
        let author = self.find_message_mut(channel, id)?.author;
        let key = self.identity_of(author).ok_or(StoreError::UnknownAuthorKey(author))?;
        let msg = self.find_message_mut(channel, id)?;
        let edited = Message { content, edited_at: Some(edited_at), sig: Some(sig), ..msg.without_reactions() };
        // Catch-up would carry it whole, as a create.
        if edited.authored_size() > MAX_MESSAGE_BYTES {
            return Err(StoreError::TooLarge);
        }
        if !msg.verify_edit(server, &edited.content, edited_at, &sig, &key) {
            return Err(StoreError::BadSignature);
        }
        if too_far_ahead(edited_at, Utc::now()) {
            return Err(StoreError::TooFarAhead);
        }
        if !supersedes(msg, Some(edited_at), &sig) {
            return Ok(false);
        }
        msg.content = edited.content;
        msg.edited_at = edited.edited_at;
        msg.sig = edited.sig;
        Ok(true)
    }

    /// Signed delete from anyone whose key is pinned here. Who may delete
    /// what is checked where a delete is made: roles and membership are
    /// not signed, so replicas could not agree on them here. A delete
    /// naming the channel of a message held here removes it; one naming
    /// another channel is refused. Without the message at hand the proof
    /// is kept, and checked against the message if it turns up later,
    /// within [`MAX_UNAPPLIED_PROOFS`] of its signer's, see
    /// [`Self::store_unapplied`].
    fn delete_message(&mut self, proof: TombstoneProof) -> Result<bool, StoreError> {
        let key = self.identity_of(proof.by).ok_or(StoreError::UnknownAuthorKey(proof.by))?;
        if !proof.verify(self.server.id, &key) {
            return Err(StoreError::BadSignature);
        }
        if too_far_ahead(proof.at, Utc::now()) {
            return Err(StoreError::TooFarAhead);
        }
        if !self.messages.contains_key(&proof.channel) {
            return Err(StoreError::UnknownChannel);
        }
        let held = self.index.get(&proof.message).and_then(|channel| {
            let list = self.messages.get(channel)?;
            let pos = list.iter().position(|m| m.id == proof.message)?;
            Some((*channel, pos, list[pos].author))
        });
        if let Some((channel, pos, author)) = held {
            // Peers lacking the message could not tell it apart from the
            // real one, and those holding it would never take it.
            if channel != proof.channel {
                return Err(StoreError::WrongChannel);
            }

            if let Some(list) = self.messages.get_mut(&channel) {
                list.remove(pos);
            }
            self.index.remove(&proof.message);
            self.server.pins.retain(|p| p.message != proof.message);
            self.store_proof(proof, Some(author));
            return Ok(true);
        }
        // Another copy of one kept already takes no new place.
        let kept = self
            .tombstones
            .get(&proof.message)
            .is_some_and(|list| list.iter().any(|s| s.proof.by == proof.by && s.proof.channel == proof.channel));
        self.store_unapplied(proof, kept)
    }

    /// Keeps a proof for a message not held here. Every replica keeps the
    /// same ones of each signer: up to [`MAX_UNAPPLIED_PROOFS`], the best
    /// ranked (see [`UnappliedKey`]). A better one than the signer's worst
    /// pushes that out; a worse one is refused, unless another copy of it
    /// is `kept` already.
    fn store_unapplied(&mut self, proof: TombstoneProof, kept: bool) -> Result<bool, StoreError> {
        let (by, key) = (proof.by, unapplied_key(&proof));
        let full = |set: &BTreeSet<UnappliedKey>| set.len() >= MAX_UNAPPLIED_PROOFS;
        let worst = self.unapplied_index().get(&by).filter(|set| full(set)).and_then(|set| set.last().copied());
        if !kept && worst.is_some_and(|worst| key > worst) {
            return Err(StoreError::ProofLimit);
        }
        let changed = self.store_proof(proof, None);
        loop {
            let over = self.unapplied_index().get(&by).filter(|set| set.len() > MAX_UNAPPLIED_PROOFS);
            let Some(worst) = over.and_then(|set| set.last().copied()) else { break };
            self.remove_unapplied(by, worst);
        }
        Ok(changed)
    }

    /// Per signer, their proofs not applied here.
    fn unapplied_index(&mut self) -> &mut HashMap<UserId, BTreeSet<UnappliedKey>> {
        let Self { tombstones, unapplied, .. } = self;
        unapplied.get_or_insert_with(|| {
            let mut index: HashMap<UserId, BTreeSet<UnappliedKey>> = HashMap::new();
            for (by, key) in tombstones.values().flat_map(|list| unapplied_of(Some(list))) {
                index.entry(by).or_default().insert(key);
            }
            index
        })
    }

    /// Drops one proof of `by` not applied here.
    fn remove_unapplied(&mut self, by: UserId, key: UnappliedKey) {
        let (_, sig, id, channel) = key;
        if let Some(list) = self.tombstones.get_mut(&id) {
            list.retain(|s| {
                let p = &s.proof;
                s.accepted_for.is_some() || p.by != by || p.channel != channel || p.sig.0 != sig
            });
            if list.is_empty() {
                self.tombstones.remove(&id);
            }
        }
        if let Some(set) = self.unapplied.as_mut().and_then(|index| index.get_mut(&by)) {
            set.remove(&key);
        }
    }

    /// Keeps a proof next to the others for its message, one per deleter
    /// and channel, the better ranked of two (see [`Self::tombstones_for`]).
    /// Returns whether anything changed.
    fn store_proof(&mut self, proof: TombstoneProof, accepted_for: Option<UserId>) -> bool {
        let id = proof.message;
        let before = unapplied_of(self.tombstones.get(&id));
        let changed = self.put_proof(proof, accepted_for);
        let Self { tombstones, unapplied, .. } = self;
        if let Some(index) = unapplied.as_mut() {
            for (by, key) in before {
                if let Some(set) = index.get_mut(&by) {
                    set.remove(&key);
                }
            }
            for (by, key) in unapplied_of(tombstones.get(&id)) {
                index.entry(by).or_default().insert(key);
            }
        }
        changed
    }

    /// [`Self::store_proof`] without keeping the index of unapplied
    /// proofs.
    fn put_proof(&mut self, proof: TombstoneProof, accepted_for: Option<UserId>) -> bool {
        let mut new = StoredProof { proof, accepted_for };
        let (by, channel) = (new.proof.by, new.proof.channel);
        let list = self.tombstones.entry(new.proof.message).or_default();
        let Some(old) = list.iter_mut().find(|s| s.proof.by == by && s.proof.channel == channel) else {
            list.push(new);
            return true;
        };
        // The same delete made twice (two devices, a retry) keeps the
        // acceptance either one had.
        new.accepted_for = new.accepted_for.or(old.accepted_for);
        let changed = old.accepted_for != new.accepted_for;
        old.accepted_for = new.accepted_for;
        if proof_rank(&new) < proof_rank(old) {
            *old = new;
            return true;
        }
        changed
    }

    /// Pins the first identity key seen for a user. A profile without a
    /// key keeps the pinned one; a different key is refused.
    fn make_profile_room(&mut self, user: UserId) -> Result<(), StoreError> {
        if self.profiles.contains_key(&user) || self.profiles.len() < MAX_PROFILES {
            return Ok(());
        }
        let members: HashSet<UserId> = self.server.members.iter().map(|m| m.user).collect();
        self.profiles.retain(|id, _| members.contains(id));
        if self.profiles.len() < MAX_PROFILES {
            Ok(())
        } else {
            Err(StoreError::Limit)
        }
    }

    pub fn media_peer_ok(&self, user: UserId, node_hex: &str, channel: ChannelId, voice: bool) -> bool {
        if !self.server.is_member(user) || self.server.banned.contains(&user) {
            return false;
        }
        let node_ok = self
            .profiles
            .get(&user)
            .and_then(|p| p.node.as_deref())
            .is_some_and(|node| node.eq_ignore_ascii_case(node_hex));
        node_ok && (!voice || self.voice_states.get(&user) == Some(&channel))
    }

    fn pin_identity(&self, profile: &mut UserProfile) -> Result<(), StoreError> {
        match (self.identity_of(profile.id), profile.identity) {
            (Some(pinned), Some(key)) if pinned != key => Err(StoreError::IdentityMismatch),
            (Some(pinned), None) => {
                profile.identity = Some(pinned);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn message_mut(&mut self, id: MessageId) -> Option<&mut Message> {
        let channel = self.index.get(&id)?;
        self.messages.get_mut(channel)?.iter_mut().find(|m| m.id == id)
    }

    fn find_message_mut(
        &mut self,
        channel: ChannelId,
        id: MessageId,
    ) -> Result<&mut Message, StoreError> {
        self.messages
            .get_mut(&channel)
            .ok_or(StoreError::UnknownChannel)?
            .iter_mut()
            .find(|m| m.id == id)
            .ok_or(StoreError::UnknownMessage)
    }

    /// Whether a member in voice is (muted, deafened), as they announced.
    pub fn voice_flags(&self, user: UserId) -> (bool, bool) {
        self.voice_flags.get(&user).copied().unwrap_or((false, false))
    }

    /// When a member joined the voice channel they are in, unix seconds.
    pub fn voice_since(&self, user: UserId) -> Option<i64> {
        self.voice_since.get(&user).copied()
    }

    /// Members currently in a given voice channel.
    pub fn voice_members(&self, channel: ChannelId) -> Vec<UserId> {
        self.voice_states
            .iter()
            .filter(|(_, ch)| **ch == channel)
            .map(|(u, _)| *u)
            .collect()
    }

    /// Profiles of members, online first, for the member list.
    pub fn member_list(&self) -> Vec<&UserProfile> {
        let mut list: Vec<&UserProfile> = self
            .server
            .members
            .iter()
            .filter_map(|m| self.profiles.get(&m.user))
            .collect();
        list.sort_by_key(|p| (!p.presence.is_visible_online(), p.username.clone()));
        list
    }
}

fn retention_cutoff(channel: &crate::server::Channel, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let days = channel.retention_days.filter(|d| *d > 0)?;
    // A retention reaching back before the earliest date keeps everything.
    now.checked_sub_signed(chrono::TimeDelta::try_days(days.into())?)
}

/// Whether a stored proof is valid here, see
/// [`ServerState::tombstones_since`]. Proofs for a message held here that
/// did not remove it come only from data saved by older versions.
fn valid(stored: &StoredProof, held: &HashSet<MessageId>) -> bool {
    stored.accepted_for.is_some() || !held.contains(&stored.proof.message)
}

/// Preference between proofs for one message, lowest first: accepted
/// here, then the earliest, then the lowest signature, so peers holding
/// the same proofs keep the same ones.
fn proof_rank(stored: &StoredProof) -> (bool, DateTime<Utc>, [u8; 64]) {
    (stored.accepted_for.is_none(), stored.proof.at, stored.proof.sig.0)
}

/// Reactions of a relayed new message as taken: valid ones, each emoji
/// and each of its users once, within [`MAX_RELAYED_REACTIONS`] and
/// [`MAX_RELAYED_REACTORS`]. The rest is dropped.
fn relayed_reactions(reactions: Vec<Reaction>) -> Vec<Reaction> {
    let mut out: Vec<Reaction> = Vec::new();
    let mut emojis: HashSet<String> = HashSet::new();
    let mut users = 0;
    for reaction in reactions {
        if out.len() == MAX_RELAYED_REACTIONS || users == MAX_RELAYED_REACTORS {
            break;
        }
        if emojis.contains(&reaction.emoji) || !crate::server::valid_reaction(&reaction.emoji) {
            continue;
        }
        let mut seen = HashSet::new();
        let list: Vec<UserId> = reaction
            .users
            .into_iter()
            .filter(|u| seen.insert(*u))
            .take(MAX_RELAYED_REACTORS - users)
            .collect();
        if list.is_empty() {
            continue;
        }
        users += list.len();
        emojis.insert(reaction.emoji.clone());
        out.push(Reaction { emoji: reaction.emoji, users: list });
    }
    out
}

fn reaction_room(message: &Message, emoji: &str, user: UserId) -> bool {
    match message.reactions.iter().find(|r| r.emoji == emoji) {
        Some(reaction) => reaction.users.contains(&user) || reaction.users.len() < MAX_REACTORS_PER_REACTION,
        None => message.reactions.len() < MAX_REACTIONS_PER_MESSAGE,
    }
}

/// Whether a signed time lies more than [`MAX_AHEAD_MS`] past `now`.
fn too_far_ahead(t: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    t.timestamp_millis().saturating_sub(now.timestamp_millis()) > MAX_AHEAD_MS
}

/// Keeps only the proofs `keep` accepts, dropping emptied entries.
/// Returns whether any went.
fn retain_proofs(map: &mut HashMap<MessageId, Vec<StoredProof>>, mut keep: impl FnMut(&StoredProof) -> bool) -> bool {
    let mut dropped = false;
    map.retain(|_, list| {
        let before = list.len();
        list.retain(&mut keep);
        dropped |= list.len() != before;
        !list.is_empty()
    });
    dropped
}

fn unapplied_key(proof: &TombstoneProof) -> UnappliedKey {
    (proof.at, proof.sig.0, proof.message, proof.channel)
}

/// The proofs of one message not applied here, by signer.
fn unapplied_of(list: Option<&Vec<StoredProof>>) -> Vec<(UserId, UnappliedKey)> {
    let unapplied = list.into_iter().flatten().filter(|s| s.accepted_for.is_none());
    unapplied.map(|s| (s.proof.by, unapplied_key(&s.proof))).collect()
}

/// Edit time in ms; None (never edited) orders first.
fn millis(t: Option<DateTime<Utc>>) -> Option<i64> {
    t.map(|t| t.timestamp_millis())
}

/// Whether a copy of `stored` signed `sig` with edit time `edited_at`
/// replaces it: a later edit, or for the same edit time the lower
/// signature, so replicas holding two copies the author signed for one
/// time keep the same (catch-up versions order them alike, see
/// `catchup::message_version`).
fn supersedes(stored: &Message, edited_at: Option<DateTime<Utc>>, sig: &Signature) -> bool {
    match millis(edited_at).cmp(&millis(stored.edited_at)) {
        std::cmp::Ordering::Equal => stored.sig.is_none_or(|old| sig.0 < old.0),
        order => order.is_gt(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{RoleId, ServerId};
    use crate::permissions::Permissions;
    use crate::server::{Channel, Role};
    use crate::test_support::{
        delete, edit, fixture, identity_for, join, join_moderator, keyed_profile, post, post_at, proof, signed,
    };

    #[test]
    fn message_create_and_read_back() {
        let (mut state, channel, owner) = fixture();
        let msg = post(&state, channel, owner, "hello world");
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: msg.clone() }), Ok(true));
        assert_eq!(state.messages(channel), &[msg]);
    }

    #[test]
    fn duplicate_message_is_ignored() {
        let (mut state, channel, owner) = fixture();
        let msg = post(&state, channel, owner, "hello");
        state.apply(ServerEvent::MessageCreate { message: msg.clone() }).unwrap();
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: msg }), Ok(false));
        assert_eq!(state.messages(channel).len(), 1);
    }

    #[test]
    fn message_to_unknown_channel_fails() {
        let (mut state, _, owner) = fixture();
        let msg = post(&state, ChannelId::new(), owner, "lost");
        assert_eq!(
            state.apply(ServerEvent::MessageCreate { message: msg }),
            Err(StoreError::UnknownChannel)
        );
    }

    #[test]
    fn edit_updates_content_and_timestamp() {
        let (mut state, channel, owner) = fixture();
        let msg = post(&state, channel, owner, "typo");
        state.apply(ServerEvent::MessageCreate { message: msg.clone() }).unwrap();
        let edited_at = Utc::now();
        assert_eq!(state.apply(edit(&state, &msg, "fixed", edited_at)), Ok(true));
        let stored = &state.messages(channel)[0];
        assert_eq!(stored.content, "fixed");
        assert_eq!(stored.edited_at, Some(edited_at), "the author's edit time is kept");
        assert!(stored.verify(state.server.id, &identity_for(owner).public()));
    }

    #[test]
    fn delete_removes_message() {
        let (mut state, channel, owner) = fixture();
        let msg = post(&state, channel, owner, "bye");
        state.apply(ServerEvent::MessageCreate { message: msg.clone() }).unwrap();
        let del = delete(&state, &msg, owner);
        assert_eq!(state.apply(del.clone()), Ok(true));
        assert!(state.messages(channel).is_empty());
        // Redelivery of the same delete is a no-op.
        assert_eq!(state.apply(del), Ok(false));
    }

    #[test]
    fn pagination_returns_latest_window() {
        let (mut state, channel, owner) = fixture();
        for i in 0..10 {
            let msg = post(&state, channel, owner, &format!("m{i}"));
            state.apply(ServerEvent::MessageCreate { message: msg }).unwrap();
        }
        let page = state.page(channel, None, 3);
        assert_eq!(page.len(), 3);
        assert_eq!(page[2].content, "m9");

        let before = page[0].id;
        let prev = state.page(channel, Some(before), 3);
        assert_eq!(prev.len(), 3);
        assert_eq!(prev[2].content, "m6");
    }

    #[test]
    fn member_join_is_idempotent() {
        let (mut state, _, _) = fixture();
        let profile = UserProfile::new("newbie");
        let sid = state.server.id;
        assert_eq!(
            state.apply(ServerEvent::MemberJoin { server: sid, profile: profile.clone() }),
            Ok(true)
        );
        assert_eq!(
            state.apply(ServerEvent::MemberJoin { server: sid, profile }),
            Ok(false)
        );
    }

    #[test]
    fn wrong_server_event_rejected() {
        let (mut state, _, _) = fixture();
        let profile = UserProfile::new("spoof");
        assert_eq!(
            state.apply(ServerEvent::MemberJoin { server: ServerId::new(), profile }),
            Err(StoreError::WrongServer)
        );
    }

    #[test]
    fn voice_state_join_move_leave() {
        let (mut state, _, owner) = fixture();
        let voice = state.server.channels[1].id;
        assert_eq!(
            state.apply(ServerEvent::VoiceState {
                user: owner,
                channel: Some(voice),
                muted: false,
                deafened: false, since: None
            }),
            Ok(true)
        );
        assert_eq!(state.voice_members(voice), vec![owner]);
        assert_eq!(
            state.apply(ServerEvent::VoiceState {
                user: owner,
                channel: None,
                muted: false,
                deafened: false, since: None
            }),
            Ok(true)
        );
        assert!(state.voice_members(voice).is_empty());
    }

    #[test]
    fn voice_since_follows_joins_and_moves() {
        let (mut state, _, owner) = fixture();
        let voice = state.server.channels[1].id;
        let join = |ch, since| ServerEvent::VoiceState { user: owner, channel: ch, muted: false, deafened: false, since };
        let now = Utc::now().timestamp();
        // The sender's own join time is kept.
        state.apply(join(Some(voice), Some(now - 90))).unwrap();
        assert_eq!(state.voice_since(owner), Some(now - 90));
        // A repeat for the same channel does not restart the clock.
        state.apply(join(Some(voice), Some(now - 5))).unwrap();
        assert_eq!(state.voice_since(owner), Some(now - 90));
        // Moving channels does.
        let lobby = crate::server::Channel::new("Lobby", ChannelKind::Voice, 9);
        let other = lobby.id;
        state.apply(ServerEvent::ChannelCreate { server: state.server.id, channel: lobby }).unwrap();
        state.apply(join(Some(other), Some(now - 5))).unwrap();
        assert_eq!(state.voice_since(owner), Some(now - 5));
        // A time from the future, or none at all, becomes "now".
        state.apply(join(Some(voice), Some(now + 3600))).unwrap();
        assert!(state.voice_since(owner).is_some_and(|s| s >= now && s <= now + 2));
        state.apply(join(None, None)).unwrap();
        assert_eq!(state.voice_since(owner), None);
    }

    #[test]
    fn voice_flags_follow_announcements() {
        let (mut state, _, owner) = fixture();
        let voice = state.server.channels[1].id;
        let set = |muted, deafened| ServerEvent::VoiceState { user: owner, channel: Some(voice), muted, deafened, since: None };
        assert_eq!(state.apply(set(false, false)), Ok(true));
        assert_eq!(state.voice_flags(owner), (false, false));
        // A change of flags alone is news.
        assert_eq!(state.apply(set(true, false)), Ok(true));
        assert_eq!(state.apply(set(true, false)), Ok(false));
        assert_eq!(state.voice_flags(owner), (true, false));
        assert_eq!(state.apply(set(true, true)), Ok(true));
        assert_eq!(state.voice_flags(owner), (true, true));
        state
            .apply(ServerEvent::VoiceState { user: owner, channel: None, muted: true, deafened: true, since: None })
            .unwrap();
        assert_eq!(state.voice_flags(owner), (false, false));
    }

    #[test]
    fn screen_share_tracks_and_clears_on_voice_leave() {
        let (mut state, _, owner) = fixture();
        let voice = state.server.channels[1].id;
        state
            .apply(ServerEvent::VoiceState {
                user: owner,
                channel: Some(voice),
                muted: false,
                deafened: false, since: None
            })
            .unwrap();
        assert_eq!(
            state.apply(ServerEvent::ScreenShare {
                user: owner,
                channel: voice,
                active: true
            }),
            Ok(true)
        );
        assert_eq!(state.screen_shares.get(&owner), Some(&voice));
        // Leaving voice ends the share implicitly.
        state
            .apply(ServerEvent::VoiceState {
                user: owner,
                channel: None,
                muted: false,
                deafened: false, since: None
            })
            .unwrap();
        assert!(state.screen_shares.is_empty());
        // Redundant stop is a no-op.
        assert_eq!(
            state.apply(ServerEvent::ScreenShare {
                user: owner,
                channel: voice,
                active: false
            }),
            Ok(false)
        );
    }

    #[test]
    fn screen_share_rejects_unknown_channel() {
        let (mut state, _, owner) = fixture();
        assert_eq!(
            state.apply(ServerEvent::ScreenShare {
                user: owner,
                channel: ChannelId::new(),
                active: true
            }),
            Err(StoreError::UnknownChannel)
        );
    }

    #[test]
    fn snapshot_roundtrip_preserves_messages_and_profiles() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        state
            .apply(ServerEvent::MemberJoin {
                server: sid,
                profile: UserProfile::new("friend"),
            })
            .unwrap();
        for i in 0..5 {
            state
                .apply(ServerEvent::MessageCreate {
                    message: post(&state, channel, owner, &format!("m{i}")),
                })
                .unwrap();
        }
        let json = serde_json::to_string(&state.snapshot()).unwrap();
        let restored = ServerState::from_snapshot(serde_json::from_str(&json).unwrap());
        assert_eq!(restored.server.id, sid);
        assert_eq!(restored.messages(channel).len(), 5);
        assert_eq!(restored.messages(channel)[4].content, "m4");
        // Owner and friend.
        assert_eq!(restored.profiles.len(), 2);
        assert_eq!(restored.identity_of(owner), state.identity_of(owner));
        // Presence resets to offline after a restart.
        assert!(restored
            .profiles
            .values()
            .all(|p| p.presence == Presence::Offline));
    }

    #[test]
    fn snapshot_drops_messages_for_deleted_channels() {
        let (mut state, channel, owner) = fixture();
        state
            .apply(ServerEvent::MessageCreate {
                message: post(&state, channel, owner, "hi"),
            })
            .unwrap();
        let mut snapshot = state.snapshot();
        snapshot.messages.push((ChannelId::new(), vec![]));
        let restored = ServerState::from_snapshot(snapshot);
        assert_eq!(restored.messages(channel).len(), 1);
    }

    #[test]
    fn server_rename_applies_once() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        assert_eq!(
            state.apply(ServerEvent::ServerRename {
                server: sid,
                name: "New Name".to_string()
            }),
            Ok(true)
        );
        assert_eq!(state.server.name, "New Name");
        // Same name again is a no-op; empty names are rejected quietly.
        assert_eq!(
            state.apply(ServerEvent::ServerRename {
                server: sid,
                name: "New Name".to_string()
            }),
            Ok(false)
        );
        assert_eq!(
            state.apply(ServerEvent::ServerRename {
                server: sid,
                name: "  ".to_string()
            }),
            Ok(false)
        );
        assert_eq!(
            state.apply(ServerEvent::ServerRename {
                server: ServerId::new(),
                name: "x".to_string()
            }),
            Err(StoreError::WrongServer)
        );
    }

    #[test]
    fn profile_update_upserts_members_only() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let mut profile = UserProfile::new("friend");
        state
            .apply(ServerEvent::MemberJoin {
                server: sid,
                profile: profile.clone(),
            })
            .unwrap();
        profile.display_name = Some("Friendly".to_string());
        profile.custom_status = Some("hacking".to_string());
        assert_eq!(
            state.apply(ServerEvent::ProfileUpdate {
                profile: profile.clone()
            }),
            Ok(true)
        );
        assert_eq!(
            state.profiles.get(&profile.id).unwrap().visible_name(),
            "Friendly"
        );
        // Identical update is a no-op; strangers are ignored.
        assert_eq!(
            state.apply(ServerEvent::ProfileUpdate { profile }),
            Ok(false)
        );
        assert_eq!(
            state.apply(ServerEvent::ProfileUpdate {
                profile: UserProfile::new("stranger")
            }),
            Ok(false)
        );
    }

    #[test]
    fn appearing_offline_is_stored_as_offline() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let mut profile = UserProfile::new("quiet");
        profile.presence = Presence::Invisible;
        profile.activity = Some("Playing Chess".into());
        state.apply(ServerEvent::MemberJoin { server: sid, profile: profile.clone() }).unwrap();
        let stored = &state.profiles[&profile.id];
        assert_eq!(stored.presence, Presence::Offline);
        assert_eq!(stored.activity, None);
        profile.custom_status = Some("away".into());
        state.apply(ServerEvent::ProfileUpdate { profile: profile.clone() }).unwrap();
        assert_eq!(state.profiles[&profile.id].presence, Presence::Offline);
        state.apply(ServerEvent::PresenceUpdate { user: profile.id, presence: Presence::Online }).unwrap();
        state.apply(ServerEvent::PresenceUpdate { user: profile.id, presence: Presence::Invisible }).unwrap();
        assert_eq!(state.profiles[&profile.id].presence, Presence::Offline);
    }

    #[test]
    fn role_lifecycle_create_assign_unassign_delete() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let role = crate::server::Role {
            id: crate::ids::RoleId::new(),
            name: "mods".into(),
            color: Some("ff0000".into()),
            permissions: crate::Permissions::KICK_MEMBERS,
            position: 1,
            hoist: true,
            self_assignable: false,
            tag: None,
            banner: None,
        };
        assert_eq!(
            state.apply(ServerEvent::RoleCreate { server: sid, role: role.clone() }),
            Ok(true)
        );
        // Duplicate create is a no-op.
        assert_eq!(
            state.apply(ServerEvent::RoleCreate { server: sid, role: role.clone() }),
            Ok(false)
        );
        assert_eq!(
            state.apply(ServerEvent::RoleAssign { server: sid, user: owner, role: role.id }),
            Ok(true)
        );
        assert!(state
            .server
            .permissions_for(owner)
            .allows(crate::Permissions::KICK_MEMBERS));
        assert_eq!(
            state.apply(ServerEvent::RoleAssign { server: sid, user: owner, role: role.id }),
            Ok(false)
        );
        assert_eq!(
            state.apply(ServerEvent::RoleUnassign { server: sid, user: owner, role: role.id }),
            Ok(true)
        );
        // Re-assign then delete removes it from the member too.
        state
            .apply(ServerEvent::RoleAssign { server: sid, user: owner, role: role.id })
            .unwrap();
        assert_eq!(
            state.apply(ServerEvent::RoleDelete { server: sid, role: role.id }),
            Ok(true)
        );
        assert!(state.server.members[0].roles.is_empty());
    }

    #[test]
    fn everyone_role_cannot_be_deleted_or_shadowed() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let everyone = state.server.roles[0].id;
        assert_eq!(
            state.apply(ServerEvent::RoleDelete { server: sid, role: everyone }),
            Ok(false)
        );
        let fake_everyone = crate::server::Role {
            id: crate::ids::RoleId::new(),
            name: "sneaky".into(),
            color: None,
            permissions: crate::Permissions::ADMINISTRATOR,
            position: 0,
            hoist: false,
            self_assignable: false,
            tag: None,
            banner: None,
        };
        assert_eq!(
            state.apply(ServerEvent::RoleCreate { server: sid, role: fake_everyone }),
            Ok(false)
        );
    }

    #[test]
    fn role_assign_to_stranger_is_noop() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let role = crate::server::Role {
            id: crate::ids::RoleId::new(),
            name: "r".into(),
            color: None,
            permissions: crate::Permissions::NONE,
            position: 1,
            hoist: false,
            self_assignable: false,
            tag: None,
            banner: None,
        };
        let rid = role.id;
        state.apply(ServerEvent::RoleCreate { server: sid, role }).unwrap();
        assert_eq!(
            state.apply(ServerEvent::RoleAssign { server: sid, user: UserId::new(), role: rid }),
            Ok(false)
        );
        // Assigning a role that does not exist is also a no-op.
        assert_eq!(
            state.apply(ServerEvent::RoleAssign {
                server: sid,
                user: state.server.owner,
                role: crate::ids::RoleId::new()
            }),
            Ok(false)
        );
    }

    #[test]
    fn member_list_sorts_online_first() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let mut offline = UserProfile::new("aaa_offline");
        offline.presence = Presence::Offline;
        let online = UserProfile::new("zzz_online");
        state.apply(ServerEvent::MemberJoin { server: sid, profile: offline }).unwrap();
        state.apply(ServerEvent::MemberJoin { server: sid, profile: online }).unwrap();
        let names: Vec<&str> = state.member_list().iter().map(|p| p.username.as_str()).collect();
        assert_eq!(names, vec!["owner", "zzz_online", "aaa_offline"]);
    }

    #[test]
    fn deleted_channel_cannot_be_recreated_by_a_late_announce() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let channel = crate::server::Channel::new("old", ChannelKind::Text, 9);
        state
            .apply(ServerEvent::ChannelCreate { server: sid, channel: channel.clone() })
            .unwrap();
        assert_eq!(
            state.apply(ServerEvent::ChannelDelete { server: sid, channel: channel.id }),
            Ok(true)
        );
        assert_eq!(
            state.apply(ServerEvent::ChannelCreate { server: sid, channel: channel.clone() }),
            Ok(false)
        );
        assert!(state.server.channel(channel.id).is_none());
    }

    #[test]
    fn delete_of_unknown_channel_still_records_tombstone() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let ghost = ChannelId::new();
        assert_eq!(state.apply(ServerEvent::ChannelDelete { server: sid, channel: ghost }), Ok(true));
        assert_eq!(state.apply(ServerEvent::ChannelDelete { server: sid, channel: ghost }), Ok(false));
    }

    #[test]
    fn afk_channel_can_be_chosen_and_disabled() {
        let (mut state, text, _) = fixture();
        let sid = state.server.id;
        let lounge = crate::server::Channel::new("Lounge", ChannelKind::Voice, 9);
        let lounge_id = lounge.id;
        state.apply(ServerEvent::ChannelCreate { server: sid, channel: lounge }).unwrap();

        assert_eq!(
            state.apply(ServerEvent::AfkChannelSet { server: sid, channel: Some(lounge_id) }),
            Ok(true)
        );
        assert_eq!(state.server.afk_channel(), Some(lounge_id));
        // Repeats are no-ops; text channels are refused.
        assert_eq!(
            state.apply(ServerEvent::AfkChannelSet { server: sid, channel: Some(lounge_id) }),
            Ok(false)
        );
        assert_eq!(
            state.apply(ServerEvent::AfkChannelSet { server: sid, channel: Some(text) }),
            Ok(false)
        );

        assert_eq!(state.apply(ServerEvent::AfkChannelSet { server: sid, channel: None }), Ok(true));
        assert_eq!(state.server.afk_channel(), None);
    }

    #[test]
    fn deleting_the_afk_channel_falls_back_to_name() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let lounge = crate::server::Channel::new("Lounge", ChannelKind::Voice, 9);
        let lounge_id = lounge.id;
        state.apply(ServerEvent::ChannelCreate { server: sid, channel: lounge }).unwrap();
        state.apply(ServerEvent::AfkChannelSet { server: sid, channel: Some(lounge_id) }).unwrap();
        state.apply(ServerEvent::ChannelDelete { server: sid, channel: lounge_id }).unwrap();
        assert_eq!(state.server.afk_channel_id, None);
        let by_name = state
            .server
            .channels
            .iter()
            .find(|c| c.name == "AFK")
            .map(|c| c.id);
        assert_eq!(state.server.afk_channel(), by_name);
    }

    #[test]
    fn channel_update_renames_and_sets_retention() {
        let (mut state, channel, _) = fixture();
        let sid = state.server.id;
        let update = |name: &str, days: Option<u32>| ServerEvent::ChannelUpdate {
            server: sid,
            channel,
            name: name.to_string(),
            retention_days: days,
        };
        assert_eq!(state.apply(update("announcements", Some(7))), Ok(true));
        let ch = state.server.channel(channel).unwrap();
        assert_eq!((ch.name.as_str(), ch.retention_days), ("announcements", Some(7)));
        // Repeats are no-ops, blank names are ignored, 0 days means forever.
        assert_eq!(state.apply(update("announcements", Some(7))), Ok(false));
        assert_eq!(state.apply(update("  ", None)), Ok(false));
        assert_eq!(state.apply(update("announcements", Some(0))), Ok(true));
        assert_eq!(state.server.channel(channel).unwrap().retention_days, None);
        assert_eq!(
            state.apply(ServerEvent::ChannelUpdate {
                server: sid,
                channel: ChannelId::new(),
                name: "x".into(),
                retention_days: None
            }),
            Err(StoreError::UnknownChannel)
        );
    }

    #[test]
    fn channel_banner_sets_and_clears() {
        let (mut state, channel, _) = fixture();
        let sid = state.server.id;
        let set = |banner: Option<&str>| ServerEvent::ChannelBanner { server: sid, channel, banner: banner.map(String::from) };
        assert_eq!(state.apply(set(Some("abc123@provider1"))), Ok(true));
        assert_eq!(state.server.channel(channel).unwrap().banner.as_deref(), Some("abc123@provider1"));
        assert_eq!(state.apply(set(Some("abc123@provider1"))), Ok(false));
        // A malformed ref clears rather than stores garbage.
        assert_eq!(state.apply(set(Some("not a ref"))), Ok(true));
        assert_eq!(state.server.channel(channel).unwrap().banner, None);
        assert_eq!(state.apply(set(None)), Ok(false));
        assert_eq!(
            state.apply(ServerEvent::ChannelBanner { server: sid, channel: ChannelId::new(), banner: None }),
            Err(StoreError::UnknownChannel)
        );
    }

    #[test]
    fn channel_color_sets_and_clears() {
        let (mut state, channel, _) = fixture();
        let sid = state.server.id;
        let set = |color: Option<&str>| ServerEvent::ChannelColor { server: sid, channel, color: color.map(String::from) };
        assert_eq!(state.apply(set(Some("9B7BFF"))), Ok(true));
        assert_eq!(state.server.channel(channel).unwrap().color.as_deref(), Some("9b7bff"));
        assert_eq!(state.apply(set(Some("9b7bff"))), Ok(false));
        // Anything but six hex digits clears rather than stores garbage.
        assert_eq!(state.apply(set(Some("#9b7bff"))), Ok(true));
        assert_eq!(state.server.channel(channel).unwrap().color, None);
        assert_eq!(state.apply(set(None)), Ok(false));
        assert_eq!(
            state.apply(ServerEvent::ChannelColor { server: sid, channel: ChannelId::new(), color: None }),
            Err(StoreError::UnknownChannel)
        );
    }

    fn pin_event(state: &ServerState, channel: ChannelId, message: MessageId, by: UserId, pinned: bool) -> ServerEvent {
        ServerEvent::MessagePin { server: state.server.id, channel, message, by, at: Utc::now(), pinned }
    }

    #[test]
    fn pins_set_once_and_clear() {
        let (mut state, channel, owner) = fixture();
        let msg = post(&state, channel, owner, "keep this");
        state.apply(ServerEvent::MessageCreate { message: msg.clone() }).unwrap();
        assert_eq!(state.apply(pin_event(&state, channel, msg.id, owner, true)), Ok(true));
        assert!(state.server.is_pinned(msg.id));
        let first = state.server.pins[0].clone();
        // A second pin, even by someone else, changes nothing.
        let other = join(&mut state, "other");
        assert_eq!(state.apply(pin_event(&state, channel, msg.id, other, true)), Ok(false));
        assert_eq!(state.server.pins, vec![first]);
        assert_eq!(state.apply(pin_event(&state, channel, msg.id, owner, false)), Ok(true));
        assert!(!state.server.is_pinned(msg.id));
        assert_eq!(state.apply(pin_event(&state, channel, msg.id, owner, false)), Ok(false));
        assert_eq!(
            state.apply(pin_event(&state, ChannelId::new(), msg.id, owner, true)),
            Err(StoreError::UnknownChannel)
        );
        assert_eq!(
            state.apply(ServerEvent::MessagePin { server: ServerId::new(), channel, message: msg.id, by: owner, at: Utc::now(), pinned: true }),
            Err(StoreError::WrongServer)
        );
    }

    #[test]
    fn pin_must_name_the_messages_channel_and_respects_the_cap() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let other = Channel::new("other", ChannelKind::Text, 5);
        let other_id = other.id;
        state.apply(ServerEvent::ChannelCreate { server: sid, channel: other }).unwrap();
        let msg = post(&state, channel, owner, "here");
        state.apply(ServerEvent::MessageCreate { message: msg.clone() }).unwrap();
        assert_eq!(state.apply(pin_event(&state, other_id, msg.id, owner, true)), Err(StoreError::WrongChannel));
        // A pin can arrive before its message does.
        let later = MessageId::for_author(owner);
        assert_eq!(state.apply(pin_event(&state, channel, later, owner, true)), Ok(true));
        for _ in 1..crate::server::MAX_PINS_PER_CHANNEL {
            assert_eq!(state.apply(pin_event(&state, channel, MessageId::for_author(owner), owner, true)), Ok(true));
        }
        assert_eq!(state.apply(pin_event(&state, channel, msg.id, owner, true)), Err(StoreError::PinLimit));
        // Another channel has its own room.
        let m2 = post(&state, other_id, owner, "there");
        state.apply(ServerEvent::MessageCreate { message: m2.clone() }).unwrap();
        assert_eq!(state.apply(pin_event(&state, other_id, m2.id, owner, true)), Ok(true));
        assert_eq!(state.server.pins_in(other_id).len(), 1);
    }

    #[test]
    fn deleting_a_message_or_its_channel_drops_the_pin() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let msg = post(&state, channel, owner, "gone soon");
        state.apply(ServerEvent::MessageCreate { message: msg.clone() }).unwrap();
        state.apply(pin_event(&state, channel, msg.id, owner, true)).unwrap();
        assert_eq!(state.apply(delete(&state, &msg, owner)), Ok(true));
        assert!(!state.server.is_pinned(msg.id));
        // Nor does a pin of a deleted message come back.
        assert_eq!(state.apply(pin_event(&state, channel, msg.id, owner, true)), Ok(false));
        let other = Channel::new("other", ChannelKind::Text, 5);
        let other_id = other.id;
        state.apply(ServerEvent::ChannelCreate { server: sid, channel: other }).unwrap();
        let m2 = post(&state, other_id, owner, "x");
        state.apply(ServerEvent::MessageCreate { message: m2.clone() }).unwrap();
        state.apply(pin_event(&state, other_id, m2.id, owner, true)).unwrap();
        state.apply(ServerEvent::ChannelDelete { server: sid, channel: other_id }).unwrap();
        assert!(state.server.pins.is_empty());
    }

    #[test]
    fn pins_survive_a_snapshot() {
        let (mut state, channel, owner) = fixture();
        let msg = post(&state, channel, owner, "pinned");
        state.apply(ServerEvent::MessageCreate { message: msg.clone() }).unwrap();
        state.apply(pin_event(&state, channel, msg.id, owner, true)).unwrap();
        let back = ServerState::from_snapshot(state.snapshot());
        assert_eq!(back.server.pins, state.server.pins);
    }

    #[test]
    fn announcement_channels_take_posts_from_announcers_only() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let news = Channel::new("news", ChannelKind::Announcement, 5);
        let news_id = news.id;
        state.apply(ServerEvent::ChannelCreate { server: sid, channel: news }).unwrap();
        let plain = join(&mut state, "plain");
        let manager = join_moderator(&mut state, "mods");
        // Manage Messages is not enough; Manage Channels or Post Announcements is.
        let blocked = post(&state, news_id, plain, "psst");
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: blocked }), Err(StoreError::NotAnnouncer));
        let blocked = post(&state, news_id, manager, "psst");
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: blocked }), Err(StoreError::NotAnnouncer));
        let role = Role {
            id: RoleId::new(),
            name: "news".into(),
            color: None,
            permissions: Permissions::POST_ANNOUNCEMENTS,
            position: 2,
            hoist: false,
            self_assignable: false,
            tag: None,
            banner: None,
        };
        let rid = role.id;
        state.apply(ServerEvent::RoleCreate { server: sid, role }).unwrap();
        state.apply(ServerEvent::RoleAssign { server: sid, user: plain, role: rid }).unwrap();
        let allowed = post(&state, news_id, plain, "Patch day!");
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: allowed.clone() }), Ok(true));
        let owners = post(&state, news_id, owner, "Welcome");
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: owners }), Ok(true));
        // Losing the role later does not reject an edit of an existing post.
        state.apply(ServerEvent::RoleUnassign { server: sid, user: plain, role: rid }).unwrap();
        let later = Utc::now() + chrono::Duration::seconds(1);
        assert_eq!(state.apply(edit(&state, &allowed, "Patch day, 6pm", later)), Ok(true));
    }

    #[test]
    fn text_channels_switch_to_announcements_and_back() {
        let (mut state, channel, _) = fixture();
        let sid = state.server.id;
        let voice = state.server.channels[1].id;
        let set = |c: ChannelId, kind: ChannelKind| ServerEvent::ChannelKindSet { server: sid, channel: c, kind };
        assert_eq!(state.apply(set(channel, ChannelKind::Announcement)), Ok(true));
        assert_eq!(state.server.channel(channel).unwrap().kind, ChannelKind::Announcement);
        assert_eq!(state.apply(set(channel, ChannelKind::Announcement)), Ok(false));
        assert_eq!(state.apply(set(channel, ChannelKind::Text)), Ok(true));
        assert_eq!(state.apply(set(voice, ChannelKind::Announcement)), Ok(false));
        assert_eq!(state.apply(set(channel, ChannelKind::Voice)), Ok(false));
        assert_eq!(state.apply(set(ChannelId::new(), ChannelKind::Text)), Err(StoreError::UnknownChannel));
    }

    #[test]
    fn retention_prunes_only_old_messages() {
        let (mut state, channel, owner) = fixture();
        let now = Utc::now();
        for age_days in [0i64, 2, 10] {
            let mut msg = Message::new(channel, owner, format!("{age_days}d"));
            msg.created_at = now - chrono::Duration::days(age_days);
            let msg = signed(&state, msg);
            state.apply(ServerEvent::MessageCreate { message: msg }).unwrap();
        }
        // Forever keeps everything.
        assert_eq!(state.prune_expired(now), 0);
        state.server.channels.iter_mut().find(|c| c.id == channel).unwrap().retention_days = Some(7);
        assert_eq!(state.prune_expired(now), 1);
        let left: Vec<_> = state.messages(channel).iter().map(|m| m.content.as_str()).collect();
        assert_eq!(left, vec!["2d", "0d"]);
    }

    #[test]
    fn deleted_message_stays_deleted_after_late_copy_and_restart() {
        let (mut state, channel, owner) = fixture();
        let msg = post(&state, channel, owner, "oops");
        state.apply(ServerEvent::MessageCreate { message: msg.clone() }).unwrap();
        let del = delete(&state, &msg, owner);
        assert_eq!(state.apply(del.clone()), Ok(true));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: msg.clone() }), Ok(false));
        assert!(state.messages(channel).is_empty());
        // Repeat deletes are no-ops; tombstones survive a save and load.
        assert_eq!(state.apply(del), Ok(false));
        let json = serde_json::to_string(&state.snapshot()).unwrap();
        let mut restored = ServerState::from_snapshot(serde_json::from_str(&json).unwrap());
        assert!(restored.tombstone(msg.id).is_some());
        assert_eq!(restored.apply(ServerEvent::MessageCreate { message: msg }), Ok(false));
    }

    #[test]
    fn delete_that_arrives_first_still_wins() {
        let (mut state, channel, owner) = fixture();
        let msg = post(&state, channel, owner, "racy");
        assert_eq!(state.apply(delete(&state, &msg, owner)), Ok(true));
        // The first late copy binds the proof, which needs saving.
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: msg.clone() }), Ok(true));
        assert!(!state.has_message(msg.id));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: msg }), Ok(false));
    }

    #[test]
    fn camera_tracks_and_clears_on_voice_leave() {
        let (mut state, _, owner) = fixture();
        let voice = state.server.channels[1].id;
        let join = |ch| ServerEvent::VoiceState { user: owner, channel: ch, muted: false, deafened: false, since: None };
        state.apply(join(Some(voice))).unwrap();
        let cam = |active| ServerEvent::CameraState { user: owner, channel: voice, active };
        assert_eq!(state.apply(cam(true)), Ok(true));
        assert_eq!(state.apply(cam(true)), Ok(false));
        assert_eq!(state.cameras.get(&owner), Some(&voice));
        assert_eq!(state.apply(cam(false)), Ok(true));
        assert!(state.cameras.is_empty());
        state.apply(cam(true)).unwrap();
        state.apply(join(None)).unwrap();
        assert!(state.cameras.is_empty(), "leaving voice turns the camera off");
        assert_eq!(
            state.apply(ServerEvent::CameraState { user: owner, channel: ChannelId::new(), active: true }),
            Err(StoreError::UnknownChannel)
        );
    }

    #[test]
    fn server_images_set_clear_and_reject_garbage() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let set = |icon: Option<&str>, banner: Option<&str>| ServerEvent::ServerImages {
            server: sid,
            icon: icon.map(String::from),
            banner: banner.map(String::from),
        };
        assert_eq!(state.apply(set(Some("aa11@bb22"), Some("cc33@bb22"))), Ok(true));
        assert_eq!(state.server.icon.as_deref(), Some("aa11@bb22"));
        assert_eq!(state.apply(set(Some("aa11@bb22"), Some("cc33@bb22"))), Ok(false));
        assert_eq!(state.apply(set(Some("../evil"), None)), Ok(true));
        assert_eq!((state.server.icon.clone(), state.server.banner.clone()), (None, None));
    }

    #[test]
    fn reaction_set_is_idempotent_and_validated() {
        let (mut state, channel, owner) = fixture();
        let msg = post(&state, channel, owner, "nice");
        let id = msg.id;
        state.apply(ServerEvent::MessageCreate { message: msg }).unwrap();
        let set = |emoji: &str, active| ServerEvent::ReactionSet {
            channel,
            message: id,
            emoji: emoji.to_string(),
            user: owner,
            active,
        };
        assert_eq!(state.apply(set("🔥", true)), Ok(true));
        assert_eq!(state.apply(set("🔥", true)), Ok(false), "redelivery does not toggle off");
        assert_eq!(state.messages(channel)[0].reactions[0].users, vec![owner]);
        assert_eq!(state.apply(set("🔥", false)), Ok(true));
        assert!(state.messages(channel)[0].reactions.is_empty());
        assert_eq!(state.apply(set("not an emoji", true)), Ok(false));
    }

    #[test]
    fn emojis_and_sounds_add_replace_remove_and_cap() {
        use crate::server::{CustomEmoji, SoundboardSound, MAX_EMOJIS};
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let emoji = |name: &str, blob: &str| ServerEvent::EmojiAdd {
            server: sid,
            emoji: CustomEmoji { name: name.into(), blob: blob.into() },
        };
        assert_eq!(state.apply(emoji("party", "aa@bb")), Ok(true));
        assert_eq!(state.apply(emoji("party", "aa@bb")), Ok(false));
        assert_eq!(state.apply(emoji("party", "cc@bb")), Ok(true), "same name replaces");
        assert_eq!(state.server.emojis.len(), 1);
        assert_eq!(state.apply(emoji("Bad Name", "aa@bb")), Ok(false));
        assert_eq!(state.apply(emoji("evil", "../x")), Ok(false));
        for i in 0..MAX_EMOJIS + 5 {
            let _ = state.apply(emoji(&format!("e{i:02}"), "aa@bb"));
        }
        assert_eq!(state.server.emojis.len(), MAX_EMOJIS);
        assert_eq!(state.apply(ServerEvent::EmojiRemove { server: sid, name: "party".into() }), Ok(true));

        let sound = SoundboardSound { name: "airhorn".into(), blob: "dd@bb".into(), volume: 1.0, emoji: None };
        assert_eq!(state.apply(ServerEvent::SoundAdd { server: sid, sound: sound.clone() }), Ok(true));
        let loud = SoundboardSound { volume: 9.0, ..sound };
        assert_eq!(state.apply(ServerEvent::SoundAdd { server: sid, sound: loud }), Ok(false));
        assert_eq!(state.apply(ServerEvent::SoundRemove { server: sid, name: "airhorn".into() }), Ok(true));
    }

    #[test]
    fn sound_emoji_is_validated_and_replaceable() {
        use crate::server::SoundboardSound;
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let clip = |emoji: Option<&str>| SoundboardSound {
            name: "rimshot".into(),
            blob: "dd@bb".into(),
            volume: 1.0,
            emoji: emoji.map(String::from),
        };
        assert_eq!(state.apply(ServerEvent::SoundAdd { server: sid, sound: clip(Some("🥁")) }), Ok(true));
        assert_eq!(state.server.soundboard[0].emoji.as_deref(), Some("🥁"));
        assert_eq!(state.apply(ServerEvent::SoundAdd { server: sid, sound: clip(Some("🎺")) }), Ok(true));
        assert_eq!(state.apply(ServerEvent::SoundAdd { server: sid, sound: clip(Some("🎺")) }), Ok(false));
        assert_eq!(state.apply(ServerEvent::SoundAdd { server: sid, sound: clip(None) }), Ok(true));
        assert_eq!(state.server.soundboard[0].emoji, None);
        for bad in ["rim", "a🥁", "🥁 🥁", "", "🥁🥁🥁🥁🥁🥁🥁🥁🥁🥁🥁🥁🥁"] {
            assert_eq!(state.apply(ServerEvent::SoundAdd { server: sid, sound: clip(Some(bad)) }), Ok(false), "{bad:?}");
        }
        assert_eq!(state.server.soundboard[0].emoji, None);
    }

    #[test]
    fn sound_emoji_accepts_custom_tokens() {
        assert!(crate::server::valid_sound_emoji("<:hacker:aa11@bb22>"));
        assert!(!crate::server::valid_sound_emoji("<:Bad:aa11@bb22>"));
        assert!(crate::server::valid_sound_emoji("👨‍👩‍👧"));
    }

    #[test]
    fn old_sounds_without_an_emoji_still_load() {
        use crate::server::SoundboardSound;
        let json = r#"{"name":"airhorn","blob":"dd@bb","volume":1.0}"#;
        let sound: SoundboardSound = serde_json::from_str(json).unwrap();
        assert_eq!(sound.emoji, None);
    }

    #[test]
    fn soundboard_play_changes_nothing() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let voice = state.server.channels[1].id;
        assert_eq!(
            state.apply(ServerEvent::SoundboardPlay { server: sid, channel: voice, user: owner, name: "x".into() }),
            Ok(false)
        );
    }

    #[test]
    fn custom_reactions_are_accepted() {
        let (mut state, channel, owner) = fixture();
        let msg = post(&state, channel, owner, "nice");
        let id = msg.id;
        state.apply(ServerEvent::MessageCreate { message: msg }).unwrap();
        let react = ServerEvent::ReactionSet { channel, message: id, emoji: ":party:".into(), user: owner, active: true };
        assert_eq!(state.apply(react), Ok(true));
    }

    #[test]
    fn game_tags_and_rules() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let tags = vec!["Valorant".to_string(), " valorant ".to_string()];
        assert_eq!(state.apply(ServerEvent::GameTagsSet { server: sid, tags: tags.clone(), meta: vec![] }), Ok(true));
        assert_eq!(state.server.game_tags, vec!["Valorant"]);
        assert_eq!(state.apply(ServerEvent::GameTagsSet { server: sid, tags: tags.clone(), meta: vec![] }), Ok(false));
        let meta = vec![
            crate::server::GameTagMeta { name: "Valorant".into(), igdb_id: 126459, cover: Some("co2mvt".into()) },
            crate::server::GameTagMeta { name: "Not tagged".into(), igdb_id: 1, cover: None },
            crate::server::GameTagMeta { name: "Valorant".into(), igdb_id: 2, cover: Some("../x".into()) },
        ];
        assert_eq!(state.apply(ServerEvent::GameTagsSet { server: sid, tags, meta }), Ok(true));
        assert_eq!(state.server.game_tag_meta.len(), 1, "only tagged games, first wins");
        assert_eq!(state.server.game_tag_meta[0].cover.as_deref(), Some("co2mvt"));
        assert_eq!(state.apply(ServerEvent::RulesSet { server: sid, rules: Some("Be kind".into()) }), Ok(true));
        assert_eq!(state.apply(ServerEvent::RulesSet { server: sid, rules: Some(" ".into()) }), Ok(false));
        assert_eq!(state.apply(ServerEvent::RulesAccept { server: sid, user: owner }), Ok(true));
        assert_eq!(state.apply(ServerEvent::RulesAccept { server: sid, user: owner }), Ok(false));
        assert_eq!(state.apply(ServerEvent::RulesAccept { server: sid, user: UserId::new() }), Ok(false), "non-members");
    }

    #[test]
    fn events_upsert_rsvp_and_stay_deleted() {
        use crate::server::{Rsvp, ScheduledEvent};
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let event = ScheduledEvent {
            id: "e1".into(),
            title: "Raid night".into(),
            description: String::new(),
            start: chrono::Utc::now(),
            end: None,
            channel: None,
            creator: owner,
            rsvps: vec![],
        };
        assert_eq!(state.apply(ServerEvent::EventUpsert { server: sid, event: event.clone() }), Ok(true));
        let rsvp = |r| ServerEvent::EventRsvp { server: sid, event: "e1".into(), user: owner, rsvp: r };
        assert_eq!(state.apply(rsvp(Some(Rsvp::Going))), Ok(true));
        assert_eq!(state.apply(rsvp(Some(Rsvp::Going))), Ok(false));
        let renamed = ScheduledEvent { title: "Raid night 2".into(), ..event.clone() };
        assert_eq!(state.apply(ServerEvent::EventUpsert { server: sid, event: renamed }), Ok(true));
        assert_eq!(state.server.events[0].rsvps.len(), 1, "edits keep RSVPs");
        assert_eq!(state.apply(rsvp(None)), Ok(true));
        assert_eq!(state.apply(ServerEvent::EventDelete { server: sid, event: "e1".into() }), Ok(true));
        assert_eq!(state.apply(ServerEvent::EventUpsert { server: sid, event }), Ok(false), "tombstoned");
    }

    #[test]
    fn channel_layout_and_limits() {
        use crate::server::Channel;
        let (mut state, text, _) = fixture();
        let sid = state.server.id;
        let category = Channel::new("Games", ChannelKind::Category, 5);
        let cat = category.id;
        state.apply(ServerEvent::ChannelCreate { server: sid, channel: category }).unwrap();
        let layout = |channel, parent| ServerEvent::ChannelLayout { server: sid, channel, parent, position: 3 };
        assert_eq!(state.apply(layout(text, Some(cat))), Ok(true));
        assert_eq!(state.server.channel(text).unwrap().parent, Some(cat));
        assert_eq!(state.apply(layout(text, Some(text))), Ok(false), "parent must be a category");
        assert_eq!(state.apply(layout(cat, Some(cat))), Ok(false), "categories do not nest");
        let voice = state.server.channels.iter().find(|c| c.kind == ChannelKind::Voice).unwrap().id;
        let limit = |channel, n| ServerEvent::ChannelLimit { server: sid, channel, user_limit: n };
        assert_eq!(state.apply(limit(voice, 4)), Ok(true));
        assert_eq!(state.apply(limit(voice, 500)), Ok(false));
        assert_eq!(state.apply(limit(text, 4)), Ok(false), "text channels have no limit");
    }

    #[test]
    fn role_updates() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let everyone = state.server.roles[0].clone();
        let pickable = crate::server::Role { self_assignable: true, ..everyone.clone() };
        assert_eq!(state.apply(ServerEvent::RoleUpdate { server: sid, role: pickable }), Ok(false), "everyone is not pickable");
        let tagged = crate::server::Role { tag: Some("GG".into()), ..everyone.clone() };
        assert_eq!(state.apply(ServerEvent::RoleUpdate { server: sid, role: tagged }), Ok(true));
        let custom = crate::server::Role { tag: Some("<:gg:ab12@cd34>".into()), banner: Some("ab@cd".into()), ..everyone.clone() };
        assert_eq!(state.apply(ServerEvent::RoleUpdate { server: sid, role: custom }), Ok(true), "custom emoji tag and banner");
        let bad_banner = crate::server::Role { banner: Some("../x".into()), ..everyone.clone() };
        assert_eq!(state.apply(ServerEvent::RoleUpdate { server: sid, role: bad_banner }), Ok(false));
        let long = crate::server::Role { tag: Some("much too long tag".into()), ..everyone };
        assert_eq!(state.apply(ServerEvent::RoleUpdate { server: sid, role: long }), Ok(false));
    }

    #[test]
    fn server_mute_and_deafen() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let other = UserProfile::new("other");
        let uid = other.id;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: other }).unwrap();
        let moderate = |muted, deafened| ServerEvent::VoiceModerate { server: sid, user: uid, muted, deafened };
        assert_eq!(state.apply(moderate(true, false)), Ok(true));
        assert_eq!(state.apply(moderate(true, false)), Ok(false));
        assert_eq!(state.apply(moderate(true, true)), Ok(true));
        assert!(state.server.server_muted.contains(&uid) && state.server.server_deafened.contains(&uid));
        assert_eq!(state.apply(moderate(false, false)), Ok(true));
        assert!(state.server.server_muted.is_empty() && state.server.server_deafened.is_empty());
        let owner_mute = ServerEvent::VoiceModerate { server: sid, user: owner, muted: true, deafened: false };
        assert_eq!(state.apply(owner_mute), Ok(false), "owner is exempt");
    }

    #[test]
    fn server_afk_timeout_accepts_listed_values() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        assert_eq!(state.apply(ServerEvent::AfkTimeoutSet { server: sid, minutes: Some(15) }), Ok(true));
        assert_eq!(state.apply(ServerEvent::AfkTimeoutSet { server: sid, minutes: Some(7) }), Ok(false));
        assert_eq!(state.server.afk_timeout_minutes, Some(15));
        assert_eq!(state.apply(ServerEvent::AfkTimeoutSet { server: sid, minutes: None }), Ok(true));
    }

    #[test]
    fn bans_remove_and_block_rejoin() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let other = UserProfile::new("other");
        let uid = other.id;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: other.clone() }).unwrap();
        assert_eq!(state.apply(ServerEvent::MemberBan { server: sid, user: uid }), Ok(true));
        assert!(!state.server.is_member(uid));
        assert_eq!(state.apply(ServerEvent::MemberJoin { server: sid, profile: other.clone() }), Ok(false));
        assert!(!state.server.is_member(uid), "banned users cannot rejoin");
        assert_eq!(state.apply(ServerEvent::MemberBan { server: sid, user: owner }), Ok(false));
        assert_eq!(state.apply(ServerEvent::MemberUnban { server: sid, user: uid }), Ok(true));
        assert_eq!(state.apply(ServerEvent::MemberJoin { server: sid, profile: other }), Ok(true));
    }

    fn create(state: &mut ServerState, message: &Message) -> Result<bool, StoreError> {
        state.apply(ServerEvent::MessageCreate { message: message.clone() })
    }

    #[test]
    fn profiles_pin_the_first_identity_key() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let mut profile = UserProfile::new("legacy");
        let uid = profile.id;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: profile.clone() }).unwrap();
        assert_eq!(state.identity_of(uid), None);

        let key = identity_for(uid).public();
        profile.identity = Some(key);
        assert_eq!(state.apply(ServerEvent::ProfileUpdate { profile: profile.clone() }), Ok(true));
        assert_eq!(state.identity_of(uid), Some(key), "first key seen is pinned");

        // A profile without a key keeps the pinned one.
        let keyless = UserProfile { identity: None, custom_status: Some("busy".into()), ..profile.clone() };
        assert_eq!(state.apply(ServerEvent::ProfileUpdate { profile: keyless.clone() }), Ok(true));
        assert_eq!(state.identity_of(uid), Some(key));
        assert_eq!(state.profiles[&uid].custom_status.as_deref(), Some("busy"));
        assert_eq!(state.apply(ServerEvent::ProfileUpdate { profile: keyless }), Ok(false));

        // A different key is refused and changes nothing.
        let imposter = UserProfile {
            identity: Some(crate::identity::Identity::generate().public()),
            display_name: Some("evil".into()),
            ..profile.clone()
        };
        assert_eq!(
            state.apply(ServerEvent::ProfileUpdate { profile: imposter.clone() }),
            Err(StoreError::IdentityMismatch)
        );
        assert_eq!(
            state.apply(ServerEvent::MemberJoin { server: sid, profile: imposter.clone() }),
            Err(StoreError::IdentityMismatch)
        );
        assert_eq!(state.identity_of(uid), Some(key));
        assert_eq!(state.profiles[&uid].display_name, None);

        // The pin outlives leaving the server.
        state.apply(ServerEvent::MemberLeave { server: sid, user: uid }).unwrap();
        assert_eq!(
            state.apply(ServerEvent::MemberJoin { server: sid, profile: imposter }),
            Err(StoreError::IdentityMismatch)
        );
        assert!(!state.server.is_member(uid));
        assert_eq!(state.apply(ServerEvent::MemberJoin { server: sid, profile }), Ok(true));
    }

    #[test]
    fn messages_need_a_known_key_and_a_valid_signature() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let unsigned = Message::new(channel, owner, "hi");
        assert_eq!(create(&mut state, &unsigned), Err(StoreError::BadSignature));

        let mut tampered = post(&state, channel, owner, "hi");
        tampered.content = "bye".into();
        assert_eq!(create(&mut state, &tampered), Err(StoreError::BadSignature));

        let mut forged = Message::new(channel, owner, "hi");
        forged.sign(sid, &crate::identity::Identity::generate());
        assert_eq!(create(&mut state, &forged), Err(StoreError::BadSignature));

        let mut elsewhere = Message::new(channel, owner, "hi");
        elsewhere.sign(ServerId::new(), &identity_for(owner));
        assert_eq!(create(&mut state, &elsewhere), Err(StoreError::BadSignature));

        let stranger = UserId::new();
        let from_stranger = post(&state, channel, stranger, "hi");
        assert_eq!(create(&mut state, &from_stranger), Err(StoreError::UnknownAuthorKey(stranger)));

        let keyless = UserProfile::new("keyless");
        let kid = keyless.id;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: keyless }).unwrap();
        let from_keyless = post(&state, channel, kid, "hi");
        assert_eq!(create(&mut state, &from_keyless), Err(StoreError::UnknownAuthorKey(kid)));
        assert!(state.messages(channel).is_empty());

        // Once the key arrives the same message is accepted.
        let keyed = UserProfile { identity: Some(identity_for(kid).public()), ..state.profiles[&kid].clone() };
        state.apply(ServerEvent::ProfileUpdate { profile: keyed }).unwrap();
        assert_eq!(create(&mut state, &from_keyless), Ok(true));
    }

    #[test]
    fn newer_signed_copy_upserts_an_edit() {
        let (mut state, channel, owner) = fixture();
        let original = post(&state, channel, owner, "v1");
        assert_eq!(create(&mut state, &original), Ok(true));
        let react = ServerEvent::ReactionSet { channel, message: original.id, emoji: ":party:".into(), user: owner, active: true };
        state.apply(react).unwrap();

        let now = Utc::now();
        let mut v2 = Message { content: "v2".into(), edited_at: Some(now), ..original.clone() };
        v2.attachments.push(crate::message::Attachment {
            blob: "abc".into(),
            filename: "a.txt".into(),
            size_bytes: 3,
            mime_type: "text/plain".into(),
            provider: None,
        });
        let v2 = signed(&state, v2);
        assert_eq!(create(&mut state, &v2), Ok(true));
        let stored = state.message(original.id).unwrap();
        assert_eq!((stored.content.as_str(), stored.edited_at), ("v2", Some(now)));
        assert_eq!(stored.attachments.len(), 1);
        assert_eq!(stored.sig, v2.sig);
        assert_eq!(stored.reactions.len(), 1, "local reactions are kept");
        assert_eq!(state.messages(channel).len(), 1);

        // Older or equal versions are duplicates.
        assert_eq!(create(&mut state, &original), Ok(false));
        assert_eq!(create(&mut state, &v2), Ok(false));
        let older = signed(&state, Message { content: "v1.5".into(), edited_at: Some(now - chrono::Duration::minutes(1)), ..original.clone() });
        assert_eq!(create(&mut state, &older), Ok(false));
        assert_eq!(state.message(original.id).unwrap().content, "v2");
    }

    #[test]
    fn same_id_from_another_author_cannot_overwrite() {
        let (mut state, channel, owner) = fixture();
        let intruder = join(&mut state, "intruder");
        let original = post(&state, channel, owner, "mine");
        create(&mut state, &original).unwrap();
        let mut fake = Message::new(channel, intruder, "hijacked");
        fake.id = original.id;
        fake.edited_at = Some(Utc::now());
        let fake = signed(&state, fake);
        assert_eq!(create(&mut state, &fake), Err(StoreError::UnboundMessageId));
        assert_eq!(state.message(original.id).unwrap().content, "mine");

        // Ids are unique across channels too, even for their author.
        let voice = state.server.channels[1].id;
        let mut copy = Message::new(voice, owner, "dup");
        copy.id = original.id;
        copy.edited_at = Some(Utc::now());
        let copy = signed(&state, copy);
        assert_eq!(create(&mut state, &copy), Ok(false));
        assert!(state.messages(voice).is_empty());
        assert_eq!(state.message(original.id).unwrap().content, "mine");
    }

    #[test]
    fn another_members_id_is_refused_where_the_real_message_is_missing() {
        let (mut state, channel, owner) = fixture();
        let intruder = join(&mut state, "intruder");
        let real = post(&state, channel, owner, "real");
        let mut fake = Message::new(channel, intruder, "fake");
        fake.id = real.id;
        let fake = signed(&state, fake);
        assert_eq!(create(&mut state, &fake), Err(StoreError::UnboundMessageId));
        assert!(!state.has_message(real.id));

        // Refused before its key is looked up, so it is never held.
        let stranger = UserId::new();
        let mut unkeyed = Message::new(channel, stranger, "fake");
        unkeyed.id = real.id;
        let unkeyed = signed(&state, unkeyed);
        assert_eq!(create(&mut state, &unkeyed), Err(StoreError::UnboundMessageId));

        assert_eq!(create(&mut state, &real), Ok(true));
        assert_eq!(state.message(real.id).unwrap().content, "real");
    }

    #[test]
    fn edits_apply_in_author_time_order() {
        let (mut state, channel, owner) = fixture();
        let msg = post(&state, channel, owner, "v1");
        create(&mut state, &msg).unwrap();
        let t1 = Utc::now();
        let t2 = t1 + chrono::Duration::seconds(1);
        assert_eq!(state.apply(edit(&state, &msg, "v3", t2)), Ok(true));
        assert_eq!(state.apply(edit(&state, &msg, "v2", t1)), Ok(false), "older edit loses");
        assert_eq!(state.apply(edit(&state, &msg, "v3", t2)), Ok(false), "redelivery");
        let stored = state.message(msg.id).unwrap();
        assert_eq!((stored.content.as_str(), stored.edited_at), ("v3", Some(t2)));
        assert!(stored.verify(state.server.id, &identity_for(owner).public()));
    }

    #[test]
    fn edits_must_be_signed_by_the_author() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let friend = join(&mut state, "friend");
        let msg = post(&state, channel, owner, "v1");
        create(&mut state, &msg).unwrap();
        let now = Utc::now();

        let by_friend = msg.edit_event(sid, "evil", now, &identity_for(friend));
        assert_eq!(state.apply(by_friend), Err(StoreError::BadSignature));

        let ServerEvent::MessageEdit { channel: c, message, edited_at, sig, .. } = edit(&state, &msg, "ok", now) else {
            panic!("expected an edit");
        };
        let swapped = ServerEvent::MessageEdit { channel: c, message, content: "changed".into(), edited_at, sig };
        assert_eq!(state.apply(swapped), Err(StoreError::BadSignature));

        let never_sent = post(&state, channel, owner, "never sent");
        assert_eq!(state.apply(edit(&state, &never_sent, "x", now)), Err(StoreError::UnknownMessage));
        assert_eq!(state.message(msg.id).unwrap().content, "v1");
    }

    #[test]
    fn any_members_signed_delete_is_honored() {
        let (mut state, channel, _) = fixture();
        let sid = state.server.id;
        let author = join(&mut state, "author");
        let member = join(&mut state, "member");
        let [a, b, c] = ["a", "b", "c"].map(|text| post(&state, channel, author, text));
        for m in [&a, &b, &c] {
            create(&mut state, m).unwrap();
        }

        // Who may delete what is checked where the delete is made.
        let by_member = proof(&state, &a, member, Utc::now());
        assert_eq!(state.apply(by_member.clone().into_event()), Ok(true));
        assert!(!state.has_message(a.id));
        assert_eq!(state.tombstones_for(a.id), vec![&by_member]);
        assert_eq!(state.snapshot().accepted, vec![(a.id, member, channel, author)]);
        assert_eq!(state.apply(by_member.into_event()), Ok(false), "redelivery");
        assert_eq!(create(&mut state, &a), Ok(false), "a late copy stays out");
        // Nor is the time it was made checked against the message.
        let backdated = proof(&state, &b, member, b.created_at - chrono::Duration::hours(1));
        assert_eq!(state.apply(backdated.into_event()), Ok(true));
        assert!(!state.has_message(b.id));

        let keyless = UserProfile::new("keyless");
        let kid = keyless.id;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: keyless }).unwrap();
        assert_eq!(state.apply(delete(&state, &c, kid)), Err(StoreError::UnknownAuthorKey(kid)));
        let forged = TombstoneProof::sign(sid, channel, c.id, author, Utc::now(), &identity_for(member));
        assert_eq!(state.apply(forged.into_event()), Err(StoreError::BadSignature));
        let unknown_channel = TombstoneProof::sign(sid, ChannelId::new(), c.id, author, Utc::now(), &identity_for(author));
        assert_eq!(state.apply(unknown_channel.into_event()), Err(StoreError::UnknownChannel));
        assert!(state.has_message(c.id));
        // Someone no longer a member still signs with their pinned key.
        state.apply(ServerEvent::MemberLeave { server: sid, user: member }).unwrap();
        assert_eq!(state.apply(delete(&state, &c, member)), Ok(true));
        assert!(!state.has_message(c.id));
    }

    #[test]
    fn early_tombstone_is_checked_against_the_late_message() {
        let (mut state, channel, _) = fixture();
        let sid = state.server.id;
        let author = join(&mut state, "author");
        let member = join(&mut state, "member");

        // A member's proof keeps it out.
        let two = post(&state, channel, author, "two");
        assert_eq!(state.apply(delete(&state, &two, member)), Ok(true));
        assert_eq!(create(&mut state, &two), Ok(true), "the proof is bound");
        assert!(!state.has_message(two.id));
        assert_eq!(create(&mut state, &two), Ok(false));

        // A proof naming another channel does not apply.
        let voice = state.server.channels[1].id;
        let three = post(&state, channel, author, "three");
        let elsewhere = TombstoneProof::sign(sid, voice, three.id, author, Utc::now(), &identity_for(author));
        assert_eq!(state.apply(elsewhere.into_event()), Ok(true));
        assert_eq!(create(&mut state, &three), Ok(true));
        assert!(state.tombstone(three.id).is_none(), "and goes once the message is in");

        // Each deleter's proof is kept, the earliest ranking first.
        let four = post(&state, channel, author, "four");
        let now = Utc::now();
        assert_eq!(state.apply(proof(&state, &four, author, now).into_event()), Ok(true));
        let earlier = proof(&state, &four, member, now - chrono::Duration::seconds(1));
        assert_eq!(state.apply(earlier.into_event()), Ok(true));
        assert_eq!(state.tombstone(four.id).map(|p| p.by), Some(member));
        assert_eq!(create(&mut state, &four), Ok(true));
        assert!(!state.has_message(four.id));
        let left: Vec<UserId> = state.tombstones_for(four.id).iter().map(|p| p.by).collect();
        assert_eq!(left, vec![member, author]);
    }

    fn reload(state: &ServerState) -> ServerState {
        let json = serde_json::to_string(&state.snapshot()).unwrap();
        ServerState::from_snapshot(serde_json::from_str(&json).unwrap())
    }

    fn mods_role(state: &ServerState, user: UserId) -> crate::ids::RoleId {
        state.server.member(user).unwrap().roles[0]
    }

    #[test]
    fn accepted_delete_outlives_the_deleters_role() {
        let (mut state, channel, _) = fixture();
        let sid = state.server.id;
        let author = join(&mut state, "author");
        type Demote = fn(ServerId, UserId, crate::ids::RoleId) -> ServerEvent;
        let demotions: [(&str, Demote); 3] = [
            ("leave", |server, user, _| ServerEvent::MemberLeave { server, user }),
            ("unassign", |server, user, role| ServerEvent::RoleUnassign { server, user, role }),
            ("ban", |server, user, _| ServerEvent::MemberBan { server, user }),
        ];
        for (how, demote) in demotions {
            let moderator = join_moderator(&mut state, "moderator");
            let msg = post(&state, channel, author, how);
            create(&mut state, &msg).unwrap();
            assert_eq!(state.apply(delete(&state, &msg, moderator)), Ok(true));
            assert_eq!(state.apply(demote(sid, moderator, mods_role(&state, moderator))), Ok(true));
            assert!(!state.server.permissions_for(moderator).allows(crate::Permissions::MANAGE_MESSAGES), "{how}");
            assert_eq!(create(&mut state, &msg), Ok(false), "{how}");
            assert_eq!(state.tombstone(msg.id).map(|p| p.by), Some(moderator), "{how}");
            assert_eq!(create(&mut reload(&state), &msg), Ok(false), "{how}, after a restart");
        }
    }

    #[test]
    fn acceptance_does_not_cover_another_author_reusing_the_id() {
        let (mut state, channel, _) = fixture();
        let author = join(&mut state, "author");
        let intruder = join(&mut state, "intruder");
        let real = post(&state, channel, author, "real");
        let mut decoy = Message::new(channel, intruder, "decoy");
        decoy.id = real.id;
        let decoy = signed(&state, decoy);
        assert_eq!(create(&mut state, &decoy), Err(StoreError::UnboundMessageId));
        // Only data saved before ids were bound can still hold one.
        let mut saved = state.snapshot();
        saved.messages.retain(|(id, _)| *id != channel);
        saved.messages.push((channel, vec![decoy.clone()]));
        let mut state = ServerState::from_snapshot(saved);
        assert!(state.has_message(real.id));
        assert_eq!(state.apply(delete(&state, &decoy, intruder)), Ok(true));
        assert!(!state.has_message(real.id));
        assert_eq!(create(&mut state, &real), Ok(true));
        assert_eq!(state.message(real.id).unwrap().content, "real");
    }

    #[test]
    fn snapshot_without_accepted_still_loads() {
        let (mut state, channel, _) = fixture();
        let author = join(&mut state, "author");
        let moderator = join_moderator(&mut state, "moderator");
        let msg = post(&state, channel, author, "rude");
        create(&mut state, &msg).unwrap();
        state.apply(delete(&state, &msg, moderator)).unwrap();
        let mut json = serde_json::to_value(state.snapshot()).unwrap();
        assert_eq!(json["accepted"].as_array().map(Vec::len), Some(1));
        json.as_object_mut().unwrap().remove("accepted");
        let mut restored = ServerState::from_snapshot(serde_json::from_value(json).unwrap());
        assert_eq!(restored.tombstone(msg.id), state.tombstone(msg.id));
        assert_eq!(create(&mut restored, &msg), Ok(true), "bound again");
        assert!(!restored.has_message(msg.id));
    }

    #[test]
    fn a_blocked_late_copy_that_binds_a_proof_asks_for_a_save() {
        let (mut state, channel, _) = fixture();
        let author = join(&mut state, "author");
        let moderator = join_moderator(&mut state, "moderator");
        let msg = post(&state, channel, author, "late");
        state.apply(delete(&state, &msg, moderator)).unwrap();
        assert!(state.snapshot().accepted.is_empty());
        assert_eq!(create(&mut state, &msg), Ok(true));
        assert_eq!(state.snapshot().accepted, vec![(msg.id, moderator, channel, author)]);
        assert_eq!(create(&mut state, &msg), Ok(false), "nothing left to bind");
        assert!(!state.has_message(msg.id));
    }

    #[test]
    fn times_more_than_a_day_ahead_are_refused() {
        let (mut state, channel, owner) = fixture();
        let now = Utc::now();
        let ahead = |hours: i64| now + chrono::Duration::hours(hours);
        let dated = |state: &ServerState, at| signed(state, Message { created_at: at, ..Message::new(channel, owner, "soon") });
        let far = dated(&state, ahead(25));
        assert_eq!(create(&mut state, &far), Err(StoreError::TooFarAhead));
        let near = dated(&state, ahead(23));
        assert_eq!(create(&mut state, &near), Ok(true));

        let msg = post(&state, channel, owner, "edit me");
        create(&mut state, &msg).unwrap();
        assert_eq!(state.apply(edit(&state, &msg, "later", ahead(25))), Err(StoreError::TooFarAhead));
        let copy = signed(&state, Message { content: "later".into(), edited_at: Some(ahead(25)), ..msg.clone() });
        assert_eq!(create(&mut state, &copy), Err(StoreError::TooFarAhead));
        assert_eq!(state.message(msg.id).unwrap().content, "edit me");
        assert_eq!(state.apply(edit(&state, &msg, "soon", ahead(23))), Ok(true));

        let gone = post(&state, channel, owner, "gone");
        assert_eq!(state.apply(proof(&state, &gone, owner, ahead(25)).into_event()), Err(StoreError::TooFarAhead));
        assert!(state.tombstone(gone.id).is_none());
        assert_eq!(state.apply(proof(&state, &gone, owner, ahead(23)).into_event()), Ok(true));
    }

    #[test]
    fn prune_drops_proofs_dated_far_ahead() {
        let (mut state, channel, owner) = fixture();
        let now = Utc::now();
        let (a, b) = (post(&state, channel, owner, "a"), post(&state, channel, owner, "b"));
        // Saved by a version that took any date.
        let mut snapshot = state.snapshot();
        snapshot.tombstone_proofs.push(proof(&state, &a, owner, now + chrono::Duration::days(3)));
        snapshot.tombstone_proofs.push(proof(&state, &b, owner, now + chrono::Duration::hours(1)));
        state = ServerState::from_snapshot(snapshot);
        assert!(state.tombstone(a.id).is_some());
        state.prune_expired(now);
        assert!(state.tombstone(a.id).is_none());
        assert!(state.tombstone(b.id).is_some());
    }

    #[test]
    fn the_same_delete_made_twice_settles_on_the_earliest() {
        let (mut a, channel, _) = fixture();
        let author = join(&mut a, "author");
        let mut b = a.clone();
        let midnight = Utc::now().date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
        let msg = post_at(&a, channel, author, "twice", midnight - chrono::Duration::minutes(2));
        create(&mut a, &msg).unwrap();
        // Deleted from two devices either side of UTC midnight.
        let early = proof(&a, &msg, author, midnight - chrono::Duration::minutes(1));
        let late = proof(&a, &msg, author, midnight + chrono::Duration::minutes(1));
        assert_eq!(a.apply(late.clone().into_event()), Ok(true));
        assert_eq!(b.apply(early.clone().into_event()), Ok(true));
        assert_eq!(a.apply(early.clone().into_event()), Ok(true));
        assert_eq!(b.apply(late.into_event()), Ok(false));
        for state in [&a, &b] {
            assert_eq!(state.tombstones_for(msg.id), vec![&early]);
        }
        assert_eq!(a.tombstones[&msg.id][0].accepted_for, Some(author), "acceptance is kept");
    }

    #[test]
    fn deleting_a_channel_drops_its_proofs() {
        use crate::catchup::{digests, mismatched};
        let (mut a, text, owner) = fixture();
        let sid = a.server.id;
        let doomed = crate::server::Channel::new("doomed", ChannelKind::Text, 5);
        let doomed_id = doomed.id;
        a.apply(ServerEvent::ChannelCreate { server: sid, channel: doomed }).unwrap();
        let mut b = a.clone();
        let gone = post(&a, doomed_id, owner, "gone");
        let kept = post(&a, text, owner, "kept");
        let now = Utc::now();
        a.apply(proof(&a, &gone, owner, now).into_event()).unwrap();
        let kept_proof = proof(&a, &kept, owner, now);
        for state in [&mut a, &mut b] {
            state.apply(kept_proof.clone().into_event()).unwrap();
            state.apply(ServerEvent::ChannelDelete { server: sid, channel: doomed_id }).unwrap();
        }
        assert!(a.tombstone(gone.id).is_none());
        assert_eq!(a.tombstone(kept.id), Some(&kept_proof));
        assert!(mismatched(&digests(&a, now), &digests(&b, now)).is_empty());
    }

    #[test]
    fn load_and_prune_drop_proofs_for_missing_channels() {
        let (mut state, text, owner) = fixture();
        let sid = state.server.id;
        let doomed = crate::server::Channel::new("doomed", ChannelKind::Text, 5);
        let doomed_id = doomed.id;
        state.apply(ServerEvent::ChannelCreate { server: sid, channel: doomed }).unwrap();
        let gone = post(&state, doomed_id, owner, "gone");
        let kept = post(&state, text, owner, "kept");
        state.apply(delete(&state, &gone, owner)).unwrap();
        state.apply(delete(&state, &kept, owner)).unwrap();

        // Saved by a version that kept proofs after their channel went.
        let mut snapshot = state.snapshot();
        snapshot.server.channels.retain(|c| c.id != doomed_id);
        let restored = ServerState::from_snapshot(snapshot);
        assert!(restored.tombstone(gone.id).is_none());
        assert!(restored.tombstone(kept.id).is_some());

        state.server.channels.retain(|c| c.id != doomed_id);
        state.messages.remove(&doomed_id);
        assert!(state.tombstone(gone.id).is_some());
        state.prune_expired(Utc::now());
        assert!(state.tombstone(gone.id).is_none());
        assert!(state.tombstone(kept.id).is_some());
    }

    #[test]
    fn legacy_data_loads_as_is() {
        let (state, channel, owner) = fixture();
        // Older versions made ids at random.
        let legacy = Message { id: MessageId::new(), ..Message::new(channel, owner, "old") };
        assert!(!legacy.id.is_bound_to(owner));
        let gone = post(&state, channel, owner, "gone");
        let mut snapshot = state.snapshot();
        for (c, list) in snapshot.messages.iter_mut() {
            if *c == channel {
                list.push(legacy.clone());
            }
        }
        snapshot.deleted_messages.push(gone.id);
        // Files written before proofs existed lack the field.
        let mut json = serde_json::to_value(&snapshot).unwrap();
        json.as_object_mut().unwrap().remove("tombstone_proofs");
        json.as_object_mut().unwrap().remove("accepted");
        let mut restored = ServerState::from_snapshot(serde_json::from_value(json).unwrap());
        assert_eq!(restored.messages(channel), std::slice::from_ref(&legacy));
        assert_eq!(create(&mut restored, &gone), Ok(false), "unproven tombstone still holds");
        // The author can still edit an unsigned message; the edit signs it.
        assert_eq!(restored.apply(edit(&restored, &legacy, "new", Utc::now())), Ok(true));
        let stored = restored.message(legacy.id).unwrap();
        assert!(stored.verify(restored.server.id, &identity_for(owner).public()));
    }

    #[test]
    fn query_helpers() {
        let (mut state, channel, owner) = fixture();
        let now = Utc::now();
        let mut old = Message::new(channel, owner, "old");
        old.created_at = now - chrono::Duration::days(3);
        let old = signed(&state, old);
        let new = post(&state, channel, owner, "new");
        create(&mut state, &new).unwrap();
        create(&mut state, &old).unwrap();

        let recent: Vec<MessageId> = state.messages_since(channel, now - chrono::Duration::days(1)).map(|m| m.id).collect();
        assert_eq!(recent, vec![new.id]);
        let all: Vec<MessageId> = state.messages_since(channel, now - chrono::Duration::days(10)).map(|m| m.id).collect();
        assert_eq!(all, vec![old.id, new.id], "oldest first");
        assert_eq!(state.messages_since(ChannelId::new(), now - chrono::Duration::days(10)).count(), 0);
        assert_eq!(state.message(old.id).map(|m| m.content.as_str()), Some("old"));
        assert!(state.has_message(new.id) && !state.has_message(MessageId::new()));

        state.apply(proof(&state, &old, owner, now - chrono::Duration::days(2)).into_event()).unwrap();
        state.apply(proof(&state, &new, owner, Utc::now()).into_event()).unwrap();
        let recent: Vec<MessageId> = state.tombstones_since(now - chrono::Duration::days(1)).map(|p| p.message).collect();
        assert_eq!(recent, vec![new.id]);
        assert_eq!(state.tombstones_since(now - chrono::Duration::days(5)).count(), 2);
        assert_eq!(state.tombstone(old.id).map(|p| p.by), Some(owner));
        assert!(!state.has_message(old.id));
    }

    #[test]
    fn a_pruned_delete_keeps_its_message_out() {
        let (mut state, channel, _) = fixture();
        let author = join(&mut state, "author");
        let msg = post(&state, channel, author, "gone for good");
        create(&mut state, &msg).unwrap();
        state.apply(delete(&state, &msg, author)).unwrap();
        state.prune_expired(Utc::now() + chrono::Duration::days(TOMBSTONE_KEEP_DAYS + 1));
        assert!(state.tombstone(msg.id).is_none());
        assert_eq!(create(&mut state, &msg), Ok(false));
        assert!(!state.has_message(msg.id));
        let mut restored = reload(&state);
        assert_eq!(create(&mut restored, &msg), Ok(false));
        assert!(!restored.has_message(msg.id));
    }

    #[test]
    fn a_delete_naming_another_channel_than_the_messages_is_refused() {
        let (mut state, channel, _) = fixture();
        let sid = state.server.id;
        let author = join(&mut state, "author");
        let voice = state.server.channels[1].id;
        let msg = post(&state, channel, author, "here");
        create(&mut state, &msg).unwrap();
        let elsewhere = TombstoneProof::sign(sid, voice, msg.id, author, Utc::now(), &identity_for(author));
        assert_eq!(state.apply(elsewhere.into_event()), Err(StoreError::WrongChannel));
        assert!(state.has_message(msg.id) && state.tombstone(msg.id).is_none());
    }

    #[test]
    fn only_proofs_valid_here_are_relayed() {
        let (mut state, channel, _) = fixture();
        let sid = state.server.id;
        let author = join(&mut state, "author");
        let moderator = join_moderator(&mut state, "moderator");
        let role = mods_role(&state, moderator);
        let [held, applied, unseen] = ["held", "applied", "unseen"].map(|text| post(&state, channel, author, text));
        create(&mut state, &held).unwrap();
        create(&mut state, &applied).unwrap();
        let now = Utc::now();
        let accepted = proof(&state, &applied, moderator, now);
        let orphan = proof(&state, &unseen, moderator, now);
        for p in [&accepted, &orphan] {
            assert_eq!(state.apply(p.clone().into_event()), Ok(true));
        }
        // Left by an older version that kept deletes it did not apply.
        let mut saved = state.snapshot();
        let waiting = proof(&state, &held, author, now);
        saved.tombstone_proofs.push(waiting);
        let mut state = ServerState::from_snapshot(saved);
        assert!(state.has_message(held.id) && state.tombstone(held.id).is_some());
        let ids = [held.id, applied.id, unseen.id];
        let relayed = |state: &ServerState| {
            let mut list: Vec<TombstoneProof> = state.tombstones_since(now - chrono::Duration::days(1)).cloned().collect();
            list.sort_by_key(|p| ids.iter().position(|id| *id == p.message));
            list
        };
        assert_eq!(relayed(&state), vec![accepted.clone(), orphan.clone()]);
        assert_eq!(state.valid_tombstones_for(&ids), vec![&accepted, &orphan]);
        // What its signer may do now changes nothing.
        state.apply(ServerEvent::RoleUnassign { server: sid, user: moderator, role }).unwrap();
        state.apply(ServerEvent::MemberLeave { server: sid, user: moderator }).unwrap();
        assert_eq!(relayed(&state), vec![accepted, orphan]);
    }

    #[test]
    fn prune_drops_tombstones_after_sixty_days() {
        let (mut state, channel, owner) = fixture();
        let now = Utc::now();
        let (a, b) = (post(&state, channel, owner, "a"), post(&state, channel, owner, "b"));
        let days = |n| now - chrono::Duration::days(n);
        state.apply(proof(&state, &a, owner, days(TOMBSTONE_KEEP_DAYS + 1)).into_event()).unwrap();
        state.apply(proof(&state, &b, owner, days(TOMBSTONE_KEEP_DAYS - 1)).into_event()).unwrap();
        assert_eq!(state.prune_expired(now), 0);
        assert!(state.tombstone(a.id).is_none());
        assert!(state.tombstone(b.id).is_some());
        assert_eq!(state.snapshot().tombstone_proofs.len(), 1);
    }

    #[test]
    fn snapshot_with_proofs_roundtrips_through_postcard() {
        let (mut state, channel, owner) = fixture();
        let kept = post(&state, channel, owner, "kept");
        let gone = post(&state, channel, owner, "gone");
        create(&mut state, &kept).unwrap();
        create(&mut state, &gone).unwrap();
        state.apply(delete(&state, &gone, owner)).unwrap();
        let bytes = postcard::to_stdvec(&state.snapshot()).unwrap();
        let mut restored = ServerState::from_snapshot(postcard::from_bytes(&bytes).unwrap());
        assert_eq!(restored.messages(channel), &[kept]);
        assert_eq!(restored.tombstone(gone.id), state.tombstone(gone.id));
        assert_eq!(restored.identity_of(owner), Some(identity_for(owner).public()));
        assert_eq!(create(&mut restored, &gone), Ok(false));
    }

    #[test]
    fn a_delete_of_a_message_not_held_is_kept_from_any_pinned_key() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let author = join(&mut state, "author");
        let member = join(&mut state, "member");
        let banned = join(&mut state, "banned");
        let former = join(&mut state, "former");
        state.apply(ServerEvent::MemberBan { server: sid, user: banned }).unwrap();
        state.apply(ServerEvent::MemberLeave { server: sid, user: former }).unwrap();
        // Known here only from a relayed profile.
        let outsider = keyed_profile("outsider");
        let stranger = outsider.id;
        state.profiles.insert(stranger, outsider);
        // Membership is not signed, so replicas could not agree on it.
        for by in [banned, former, stranger] {
            let theirs = post(&state, channel, author, "theirs");
            assert_eq!(state.apply(delete(&state, &theirs, by)), Ok(true));
            assert_eq!(create(&mut state, &theirs), Ok(true), "the proof is bound");
            assert!(!state.has_message(theirs.id));
        }
        let unknown = UserId::new();
        let other = post(&state, channel, author, "other");
        assert_eq!(state.apply(delete(&state, &other, unknown)), Err(StoreError::UnknownAuthorKey(unknown)));
        // Each signer's is kept, and taken again once its signer has left.
        let by_member = proof(&state, &other, member, Utc::now());
        assert_eq!(state.apply(by_member.clone().into_event()), Ok(true));
        assert_eq!(state.apply(delete(&state, &other, owner)), Ok(true));
        state.apply(ServerEvent::MemberLeave { server: sid, user: member }).unwrap();
        assert_eq!(state.apply(by_member.into_event()), Ok(false), "redelivery");
        assert_eq!(state.tombstones_for(other.id).len(), 2);
        assert_eq!(create(&mut state, &other), Ok(true), "the proofs are bound");
        assert!(!state.has_message(other.id));
    }

    #[test]
    fn an_authors_delete_of_a_message_not_held_keeps_the_late_copy_out() {
        let (mut state, channel, _) = fixture();
        let author = join(&mut state, "author");
        let msg = post(&state, channel, author, "regretted");
        assert_eq!(state.apply(delete(&state, &msg, author)), Ok(true));
        assert_eq!(create(&mut state, &msg), Ok(true), "the proof is bound");
        assert!(!state.has_message(msg.id));
        assert_eq!(create(&mut reload(&state), &msg), Ok(false));
    }

    #[test]
    fn unapplied_proofs_keep_the_earliest_per_signer() {
        let (mut state, channel, _) = fixture();
        let author = join(&mut state, "author");
        let other = join(&mut state, "other");
        let base = Utc::now() - chrono::Duration::hours(1);
        let at = |i: usize| base + chrono::Duration::seconds(i as i64);
        // Never sent, so they need no signature.
        let written = base - chrono::Duration::hours(1);
        let unseen: Vec<Message> = (0..MAX_UNAPPLIED_PROOFS)
            .map(|i| Message { created_at: written, ..Message::new(channel, author, format!("m{i}")) })
            .collect();
        for (i, m) in unseen.iter().enumerate() {
            assert_eq!(state.apply(proof(&state, m, author, at(i)).into_event()), Ok(true));
        }
        let one_more = post(&state, channel, author, "one more");
        let late = proof(&state, &one_more, author, at(MAX_UNAPPLIED_PROOFS));
        assert_eq!(state.apply(late.into_event()), Err(StoreError::ProofLimit));
        assert!(state.tombstone(one_more.id).is_none());
        // Redelivery and other signers are not held back.
        assert_eq!(state.apply(proof(&state, &unseen[0], author, at(0)).into_event()), Ok(false));
        let others = post(&state, channel, other, "others");
        assert_eq!(state.apply(delete(&state, &others, other)), Ok(true));
        // An earlier one pushes out the latest.
        let early = proof(&state, &one_more, author, base - chrono::Duration::seconds(1));
        assert_eq!(state.apply(early.into_event()), Ok(true));
        let last = &unseen[MAX_UNAPPLIED_PROOFS - 1];
        assert!(state.tombstone(one_more.id).is_some() && state.tombstone(last.id).is_none());
        // A message that turns up binds its proof, which frees a place.
        let arrived = signed(&state, unseen[0].clone());
        assert_eq!(create(&mut state, &arrived), Ok(true));
        let again = proof(&state, last, author, at(MAX_UNAPPLIED_PROOFS - 1));
        assert_eq!(state.apply(again.into_event()), Ok(true));
    }

    #[test]
    fn replicas_keep_the_same_unapplied_proofs_whatever_they_saw_first() {
        let (mut a, channel, _) = fixture();
        let author = join(&mut a, "author");
        let mut b = a.clone();
        let base = Utc::now() - chrono::Duration::hours(1);
        let at = |i: usize| base + chrono::Duration::seconds(i as i64);
        let ids: Vec<Message> =
            (0..MAX_UNAPPLIED_PROOFS + 20).map(|i| Message::new(channel, author, format!("m{i}"))).collect();
        let proofs: Vec<TombstoneProof> = ids.iter().enumerate().map(|(i, m)| proof(&a, m, author, at(i))).collect();
        for p in &proofs {
            let _ = a.apply(p.clone().into_event());
        }
        for p in proofs.iter().rev() {
            let _ = b.apply(p.clone().into_event());
        }
        let kept = |state: &ServerState| {
            let mut list: Vec<TombstoneProof> = state.snapshot().tombstone_proofs;
            list.sort_by_key(|p| p.message);
            list
        };
        assert_eq!(kept(&a), kept(&b));
        let mut expected: Vec<TombstoneProof> = proofs[..MAX_UNAPPLIED_PROOFS].to_vec();
        expected.sort_by_key(|p| p.message);
        assert_eq!(kept(&a), expected);
    }

    #[test]
    fn leaving_or_a_ban_keeps_the_signers_deletes() {
        let (mut state, channel, _) = fixture();
        let sid = state.server.id;
        let author = join(&mut state, "author");
        for how in ["leave", "ban"] {
            let member = join(&mut state, how);
            let held = post(&state, channel, author, "held");
            let unseen = post(&state, channel, author, "unseen");
            create(&mut state, &held).unwrap();
            for m in [&held, &unseen] {
                assert_eq!(state.apply(delete(&state, m, member)), Ok(true), "{how}");
            }
            let out = match how {
                "ban" => ServerEvent::MemberBan { server: sid, user: member },
                _ => ServerEvent::MemberLeave { server: sid, user: member },
            };
            assert_eq!(state.apply(out), Ok(true), "{how}");
            // Replicas hearing of it later take both too, so both stay.
            assert_eq!(state.unapplied_index().get(&member).map_or(0, BTreeSet::len), 1, "{how}");
            let relayed: Vec<MessageId> = state.tombstones_since(DateTime::<Utc>::MIN_UTC).map(|p| p.message).collect();
            assert!(relayed.contains(&held.id) && relayed.contains(&unseen.id), "{how}");
            assert_eq!(create(&mut state, &held), Ok(false), "{how}");
            assert_eq!(create(&mut state, &unseen), Ok(true), "{how}: the proof is bound");
            assert!(!state.has_message(unseen.id), "{how}");
        }
    }

    #[test]
    fn listing_keeps_the_best_ranked_proofs_per_signer_applied_or_not() {
        let (mut state, channel, _) = fixture();
        let author = join(&mut state, "author");
        let moderator = join_moderator(&mut state, "moderator");
        let base = Utc::now() - chrono::Duration::hours(1);
        let at = |i: usize| base + chrono::Duration::seconds(i as i64);
        let written = base - chrono::Duration::hours(1);
        // Every other one deletes a message held here.
        let all: Vec<Message> = (0..MAX_UNAPPLIED_PROOFS + 10)
            .map(|i| post_at(&state, channel, author, &format!("m{i}"), written))
            .collect();
        for (i, m) in all.iter().enumerate() {
            if i % 2 == 0 {
                create(&mut state, m).unwrap();
            }
            assert_eq!(state.apply(proof(&state, m, moderator, at(i)).into_event()), Ok(true));
        }
        let listed: HashSet<MessageId> = state.tombstones_since(DateTime::<Utc>::MIN_UTC).map(|p| p.message).collect();
        let best: HashSet<MessageId> = all[..MAX_UNAPPLIED_PROOFS].iter().map(|m| m.id).collect();
        assert_eq!(listed, best);
        let ids: Vec<MessageId> = all.iter().map(|m| m.id).collect();
        assert_eq!(state.valid_tombstones_for(&ids).len(), MAX_UNAPPLIED_PROOFS);
        // Those left unlisted still keep their messages out here.
        let last = all.last().unwrap();
        assert!(state.tombstone(last.id).is_some());
        assert_eq!(create(&mut state, &all[all.len() - 2]), Ok(false));
        assert!(!state.has_message(all[all.len() - 2].id));
    }

    /// Content that brings signed `msg` to an authored size of `size`.
    fn filling(msg: &Message, size: usize) -> String {
        let probe = Message { content: "x".repeat(200), ..msg.clone() };
        "x".repeat(200 + size - probe.authored_size())
    }

    /// A signed message from `author` whose authored size is `size`.
    fn sized(state: &ServerState, channel: ChannelId, author: UserId, size: usize) -> Message {
        let base = signed(state, Message::new(channel, author, ""));
        let msg = signed(state, Message { content: filling(&base, size), ..base });
        assert_eq!(msg.authored_size(), size);
        msg
    }

    #[test]
    fn messages_larger_than_one_gossip_create_are_refused() {
        let (mut state, channel, owner) = fixture();
        let over = sized(&state, channel, owner, MAX_MESSAGE_BYTES + 1);
        assert_eq!(create(&mut state, &over), Err(StoreError::TooLarge));
        assert!(!state.has_message(over.id));
        // Reactions do not count.
        let mut at_limit = sized(&state, channel, owner, MAX_MESSAGE_BYTES);
        at_limit.reactions.push(Reaction { emoji: ":party:".into(), users: (0..50).map(|_| UserId::new()).collect() });
        assert_eq!(create(&mut state, &at_limit), Ok(true));

        // Edits may not grow a message past it either, by event or by copy.
        let small = post(&state, channel, owner, "small");
        create(&mut state, &small).unwrap();
        let later = Utc::now() + chrono::Duration::seconds(1);
        let fits = filling(&Message { edited_at: Some(later), ..small.clone() }, MAX_MESSAGE_BYTES);
        let too_long = format!("{fits}x");
        assert_eq!(state.apply(edit(&state, &small, &too_long, later)), Err(StoreError::TooLarge));
        let copy = signed(&state, Message { content: too_long, edited_at: Some(later), ..small.clone() });
        assert_eq!(create(&mut state, &copy), Err(StoreError::TooLarge));
        assert_eq!(state.message(small.id).unwrap().content, "small");
        assert_eq!(state.apply(edit(&state, &small, &fits, later)), Ok(true));
        assert_eq!(state.message(small.id).unwrap().authored_size(), MAX_MESSAGE_BYTES);
    }

    #[test]
    fn reactions_taken_with_a_relayed_message_are_capped() {
        let (mut state, channel, owner) = fixture();
        let user = UserId::new();
        let reaction = |emoji: &str, users: Vec<UserId>| Reaction { emoji: emoji.into(), users };
        let mut kinds = Message::new(channel, owner, "kinds");
        kinds.reactions = vec![
            reaction("not an emoji", vec![user]),
            reaction(":party:", vec![user, user, owner]),
            reaction(":party:", vec![owner]),
            reaction(":empty:", vec![]),
        ];
        kinds.reactions.extend((0..MAX_RELAYED_REACTIONS + 5).map(|i| reaction(&format!(":e{i:02}:"), vec![user])));
        let kinds = signed(&state, kinds);
        assert_eq!(create(&mut state, &kinds), Ok(true));
        let stored = &state.message(kinds.id).unwrap().reactions;
        assert_eq!(stored.len(), MAX_RELAYED_REACTIONS);
        assert_eq!(stored[0], reaction(":party:", vec![user, owner]));
        assert_eq!(stored[1].emoji, ":e00:");

        let crowd: Vec<UserId> = (0..MAX_RELAYED_REACTORS + 5).map(|_| UserId::new()).collect();
        let mut users = Message::new(channel, owner, "users");
        users.reactions = vec![reaction(":one:", vec![owner]), reaction(":two:", crowd.clone()), reaction(":three:", vec![owner])];
        let users = signed(&state, users);
        assert_eq!(create(&mut state, &users), Ok(true));
        let stored = &state.message(users.id).unwrap().reactions;
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[1], reaction(":two:", crowd[..MAX_RELAYED_REACTORS - 1].to_vec()));
    }

    /// Checks the index of messages by id against a full scan.
    fn assert_indexed(state: &ServerState) {
        let mut scanned = HashMap::new();
        for (channel, list) in &state.messages {
            for m in list {
                assert!(scanned.insert(m.id, *channel).is_none(), "ids are unique");
            }
        }
        assert_eq!(state.index, scanned);
    }

    #[test]
    fn the_index_of_messages_by_id_stays_right() {
        let (mut state, text, owner) = fixture();
        let sid = state.server.id;
        let doomed = crate::server::Channel::new("doomed", ChannelKind::Text, 5);
        let doomed_id = doomed.id;
        state.apply(ServerEvent::ChannelCreate { server: sid, channel: doomed }).unwrap();
        let old = post_at(&state, text, owner, "old", Utc::now() - chrono::Duration::days(10));
        let [kept, gone] = ["kept", "gone"].map(|t| post(&state, text, owner, t));
        let elsewhere = post(&state, doomed_id, owner, "elsewhere");
        for m in [&old, &kept, &gone, &elsewhere] {
            assert_eq!(create(&mut state, m), Ok(true));
        }
        assert_indexed(&state);
        assert_eq!(state.message(elsewhere.id).map(|m| m.channel), Some(doomed_id));

        state.apply(delete(&state, &gone, owner)).unwrap();
        assert!(!state.has_message(gone.id));
        assert_indexed(&state);

        let name = state.server.channel(text).unwrap().name.clone();
        let retention = ServerEvent::ChannelUpdate { server: sid, channel: text, name, retention_days: Some(5) };
        assert_eq!(state.apply(retention), Ok(true));
        assert!(!state.has_message(old.id) && state.has_message(kept.id));
        assert_indexed(&state);

        state.apply(ServerEvent::ChannelDelete { server: sid, channel: doomed_id }).unwrap();
        assert!(!state.has_message(elsewhere.id));
        assert_indexed(&state);

        let mut restored = reload(&state);
        assert_indexed(&restored);
        assert!(restored.has_message(kept.id));
        assert_eq!(restored.apply(edit(&restored, &kept, "edited", Utc::now())), Ok(true));
        assert_eq!(restored.message(kept.id).unwrap().content, "edited");
        assert_eq!(restored.apply(delete(&restored, &kept, owner)), Ok(true));
        assert!(!restored.has_message(kept.id));
        assert_indexed(&restored);

        // An id an older version let in twice is loaded once.
        let voice = state.server.channels[1].id;
        let mut saved = state.snapshot();
        for (c, list) in saved.messages.iter_mut() {
            if *c == voice {
                list.push(Message { channel: voice, ..kept.clone() });
            }
        }
        let restored = ServerState::from_snapshot(saved);
        assert_indexed(&restored);
        assert!(restored.has_message(kept.id));
    }

    #[test]
    fn two_copies_signed_for_one_time_settle_on_the_lower_signature() {
        let (state, channel, owner) = fixture();
        let original = post(&state, channel, owner, "original");
        let lower = |a: &Message, b: &Message| if a.sig.unwrap().0 < b.sig.unwrap().0 { a.clone() } else { b.clone() };
        let at = Utc::now();
        for edited_at in [None, Some(at)] {
            let one = signed(&state, Message { content: "one".into(), edited_at, ..original.clone() });
            let two = signed(&state, Message { content: "two".into(), edited_at, ..original.clone() });
            let winner = lower(&one, &two);
            for order in [[&one, &two], [&two, &one]] {
                let mut s = state.clone();
                for m in order {
                    create(&mut s, m).unwrap();
                }
                assert_eq!(s.message(original.id).unwrap().content, winner.content, "{edited_at:?}");
                assert_eq!(create(&mut s, &winner), Ok(false), "redelivery");
            }
        }
        // The same for edit events.
        let sig_of = |e: &ServerEvent| match e {
            ServerEvent::MessageEdit { sig, .. } => *sig,
            _ => panic!("expected an edit"),
        };
        let (one, two) = (edit(&state, &original, "one", at), edit(&state, &original, "two", at));
        let winner = if sig_of(&one).0 < sig_of(&two).0 { "one" } else { "two" };
        for order in [[&one, &two], [&two, &one]] {
            let mut s = state.clone();
            create(&mut s, &original).unwrap();
            for e in order {
                s.apply(e.clone()).unwrap();
            }
            assert_eq!(s.message(original.id).unwrap().content, winner);
            assert!(s.message(original.id).unwrap().verify(s.server.id, &identity_for(owner).public()));
        }
    }

    #[test]
    fn a_retention_reaching_past_the_earliest_date_keeps_everything() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let msg = post(&state, channel, owner, "kept");
        create(&mut state, &msg).unwrap();
        let name = state.server.channel(channel).unwrap().name.clone();
        let forever = ServerEvent::ChannelUpdate { server: sid, channel, name, retention_days: Some(u32::MAX) };
        assert_eq!(state.apply(forever), Ok(true));
        assert_eq!(state.retention_cutoff(channel, Utc::now()), None);
        // After a restart too.
        let mut restored = reload(&state);
        assert_eq!(restored.prune_expired(Utc::now()), 0);
        let late = post(&state, channel, owner, "late");
        assert_eq!(create(&mut restored, &late), Ok(true));
        assert!(restored.has_message(msg.id));
        assert_eq!(crate::catchup::digests(&restored, Utc::now()).len(), 1);
    }

    fn filler_member() -> Member {
        Member { user: UserId::new(), nickname: None, roles: vec![], joined_at: Utc::now() }
    }

    #[test]
    fn a_full_server_takes_no_more_members() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        while state.server.members.len() < MAX_MEMBERS {
            state.server.members.push(filler_member());
        }
        let newcomer = keyed_profile("late");
        let join = ServerEvent::MemberJoin { server: sid, profile: newcomer.clone() };
        assert_eq!(state.apply(join.clone()), Err(StoreError::Limit));
        assert!(!state.profiles.contains_key(&newcomer.id));
        let again = ServerEvent::MemberJoin { server: sid, profile: state.profiles[&owner].clone() };
        assert_eq!(state.apply(again), Ok(false), "a member already in stays");
        let gone = state.server.members[5].user;
        assert_eq!(state.apply(ServerEvent::MemberLeave { server: sid, user: gone }), Ok(true));
        assert_eq!(state.apply(join), Ok(true));
    }

    #[test]
    fn profiles_of_former_members_make_room_for_a_newcomer() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        while state.profiles.len() < MAX_PROFILES {
            let left = UserProfile::new("left");
            state.profiles.insert(left.id, left);
        }
        let newcomer = keyed_profile("new");
        assert_eq!(state.apply(ServerEvent::MemberJoin { server: sid, profile: newcomer.clone() }), Ok(true));
        assert!(state.profiles.contains_key(&newcomer.id) && state.profiles.contains_key(&owner));
        assert_eq!(state.profiles.len(), 2);
    }

    #[test]
    fn channels_and_roles_are_capped() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        while state.server.channels.len() < MAX_CHANNELS {
            state.server.channels.push(Channel::new("more", ChannelKind::Text, 1));
        }
        let channel = Channel::new("one more", ChannelKind::Text, 1);
        assert_eq!(state.apply(ServerEvent::ChannelCreate { server: sid, channel }), Err(StoreError::Limit));
        assert_eq!(state.server.channels.len(), MAX_CHANNELS);
        while state.server.roles.len() < MAX_ROLES {
            state.server.roles.push(Role {
                id: RoleId::new(),
                name: "r".into(),
                color: None,
                permissions: Permissions::NONE,
                position: 1,
                hoist: false,
                self_assignable: false,
                tag: None,
                banner: None,
            });
        }
        let role = Role {
            id: RoleId::new(),
            name: "last".into(),
            color: None,
            permissions: Permissions::NONE,
            position: 1,
            hoist: false,
            self_assignable: false,
            tag: None,
            banner: None,
        };
        assert_eq!(state.apply(ServerEvent::RoleCreate { server: sid, role }), Err(StoreError::Limit));
        assert_eq!(state.server.roles.len(), MAX_ROLES);
    }

    #[test]
    fn reactions_per_message_and_reactors_per_emoji_are_capped() {
        let (mut state, channel, owner) = fixture();
        let msg = post(&state, channel, owner, "popular");
        create(&mut state, &msg).unwrap();
        let react = |emoji: &str, user| ServerEvent::ReactionToggle { channel, message: msg.id, emoji: emoji.into(), user };
        let set = |emoji: &str, user, active| ServerEvent::ReactionSet { channel, message: msg.id, emoji: emoji.into(), user, active };
        {
            let held = state.find_message_mut(channel, msg.id).unwrap();
            for i in 0..MAX_REACTIONS_PER_MESSAGE {
                held.reactions.push(Reaction { emoji: format!(":e{i}:"), users: vec![UserId::new()] });
            }
        }
        assert_eq!(state.apply(react(":new:", owner)), Err(StoreError::Limit));
        assert_eq!(state.apply(set(":new:", owner, true)), Err(StoreError::Limit));
        assert_eq!(state.apply(react(":e0:", owner)), Ok(true), "an emoji already there takes more people");
        assert_eq!(state.apply(react(":e0:", owner)), Ok(true), "and lets them go");
        {
            let held = state.find_message_mut(channel, msg.id).unwrap();
            let first = &mut held.reactions[0];
            while first.users.len() < MAX_REACTORS_PER_REACTION {
                first.users.push(UserId::new());
            }
        }
        assert_eq!(state.apply(react(":e0:", owner)), Err(StoreError::Limit));
        assert_eq!(state.apply(set(":e0:", owner, true)), Err(StoreError::Limit));
        let someone = state.message(msg.id).unwrap().reactions[0].users[3];
        assert_eq!(state.apply(set(":e0:", someone, false)), Ok(true), "leaving is always possible");
        assert_eq!(state.apply(react(":e0:", owner)), Ok(true));
        assert_eq!(state.message(msg.id).unwrap().reactions[0].users.len(), MAX_REACTORS_PER_REACTION);
    }

    #[test]
    fn live_voice_share_and_camera_entries_are_capped() {
        let (mut state, _, owner) = fixture();
        let voice = state.server.channels[1].id;
        for _ in 0..MAX_LIVE_ENTRIES {
            let user = UserId::new();
            state.voice_states.insert(user, voice);
            state.screen_shares.insert(user, voice);
            state.cameras.insert(user, voice);
        }
        let inside = *state.voice_states.keys().next().unwrap();
        let share = |user, active| ServerEvent::ScreenShare { user, channel: voice, active };
        assert_eq!(state.apply(share(owner, true)), Err(StoreError::Limit));
        assert_eq!(state.apply(share(inside, true)), Ok(false), "someone in already is unaffected");
        assert_eq!(state.apply(share(inside, false)), Ok(true));
        assert_eq!(state.apply(share(owner, true)), Ok(true));
        let camera = |user, active| ServerEvent::CameraState { user, channel: voice, active };
        assert_eq!(state.apply(camera(owner, true)), Err(StoreError::Limit));
        assert_eq!(state.apply(camera(inside, false)), Ok(true));
        assert_eq!(state.apply(camera(owner, true)), Ok(true));
        let join = |user, channel| ServerEvent::VoiceState { user, channel, muted: false, deafened: false, since: None };
        assert_eq!(state.apply(join(owner, Some(voice))), Err(StoreError::Limit));
        assert!(state.apply(join(inside, Some(voice))).is_ok());
        assert_eq!(state.apply(join(inside, None)), Ok(true));
        assert_eq!(state.apply(join(owner, Some(voice))), Ok(true));
    }

    #[test]
    fn rsvps_and_bans_are_capped() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let event = crate::server::ScheduledEvent {
            id: "night".into(),
            title: "Game night".into(),
            description: String::new(),
            start: Utc::now(),
            end: None,
            channel: None,
            creator: owner,
            rsvps: (0..MAX_RSVPS_PER_EVENT).map(|_| (UserId::new(), crate::server::Rsvp::Going)).collect(),
        };
        state.server.events.push(event);
        let rsvp = |user, rsvp| ServerEvent::EventRsvp { server: sid, event: "night".into(), user, rsvp };
        assert_eq!(state.apply(rsvp(owner, Some(crate::server::Rsvp::Going))), Err(StoreError::Limit));
        let someone = state.server.events[0].rsvps[0].0;
        assert_eq!(state.apply(rsvp(someone, Some(crate::server::Rsvp::Interested))), Ok(true), "changing an answer is fine");
        assert_eq!(state.apply(rsvp(someone, None)), Ok(true));
        assert_eq!(state.apply(rsvp(owner, Some(crate::server::Rsvp::Going))), Ok(true));
        while state.server.banned.len() < MAX_BANS {
            state.server.banned.push(UserId::new());
        }
        let user = join(&mut state, "troll");
        assert_eq!(state.apply(ServerEvent::MemberBan { server: sid, user }), Err(StoreError::Limit));
        assert!(state.server.is_member(user));
        let pardoned = state.server.banned[0];
        assert_eq!(state.apply(ServerEvent::MemberUnban { server: sid, user: pardoned }), Ok(true));
        assert_eq!(state.apply(ServerEvent::MemberBan { server: sid, user }), Ok(true));
    }

    #[test]
    fn tombstoned_ids_keep_only_the_newest() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let oldest = ChannelId::new();
        state.server.deleted_channels.push(oldest);
        while state.server.deleted_channels.len() < MAX_TOMBSTONED_IDS {
            state.server.deleted_channels.push(ChannelId::new());
        }
        let gone = ChannelId::new();
        assert_eq!(state.apply(ServerEvent::ChannelDelete { server: sid, channel: gone }), Ok(true));
        assert_eq!(state.server.deleted_channels.len(), MAX_TOMBSTONED_IDS);
        assert!(state.server.deleted_channels.contains(&gone) && !state.server.deleted_channels.contains(&oldest));
        state.server.deleted_events = (0..MAX_TOMBSTONED_IDS).map(|i| format!("e{i}")).collect();
        assert_eq!(state.apply(ServerEvent::EventDelete { server: sid, event: "last".into() }), Ok(false));
        assert_eq!(state.server.deleted_events.len(), MAX_TOMBSTONED_IDS);
        assert_eq!(state.server.deleted_events.last().map(String::as_str), Some("last"));
        assert_ne!(state.server.deleted_events[0], "e0");
    }

    #[test]
    fn media_peers_must_be_members_on_their_own_node() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let voice = state.server.channels[1].id;
        let lobby = Channel::new("Lobby", ChannelKind::Voice, 9);
        let other = lobby.id;
        state.apply(ServerEvent::ChannelCreate { server: sid, channel: lobby }).unwrap();
        let node = "ab".repeat(32);
        let member = join(&mut state, "friend");
        assert!(!state.media_peer_ok(member, &node, voice, false), "no node announced yet");
        state.profiles.get_mut(&member).unwrap().node = Some(node.clone());
        assert!(state.media_peer_ok(member, &node, voice, false));
        assert!(state.media_peer_ok(member, &node.to_uppercase(), voice, false));
        assert!(!state.media_peer_ok(member, &"cd".repeat(32), voice, false), "another node");
        assert!(!state.media_peer_ok(member, &node, voice, true), "not in voice");
        let in_voice = |user, channel| ServerEvent::VoiceState { user, channel, muted: false, deafened: false, since: None };
        state.apply(in_voice(member, Some(other))).unwrap();
        assert!(!state.media_peer_ok(member, &node, voice, true), "in another channel");
        state.apply(in_voice(member, Some(voice))).unwrap();
        assert!(state.media_peer_ok(member, &node, voice, true));
        let stranger = UserProfile::new("stranger");
        state.profiles.insert(stranger.id, UserProfile { node: Some(node.clone()), ..stranger.clone() });
        state.voice_states.insert(stranger.id, voice);
        assert!(!state.media_peer_ok(stranger.id, &node, voice, true), "not a member");
        assert!(!state.media_peer_ok(owner, &node, voice, false), "owner is on another node");
        state.apply(ServerEvent::MemberBan { server: sid, user: member }).unwrap();
        assert!(!state.media_peer_ok(member, &node, voice, false));
    }
}
