use crate::action::{self, Action, SignedEvent};
use crate::audit::{AuditEntry, AuditKind};
use crate::invites::TrackedInvite;
use crate::automod::{AutoMod, Hold};
use crate::channel_key::{ChannelKeys, KeyScope};
use crate::command::{BotCommand, CommandUse};
use crate::devices::{self, Device, Devices};
use crate::event::ServerEvent;
use crate::identity::{IdentityKey, Signature};
use crate::ids::{ChannelId, MessageId, ServerId, UserId};
use crate::message::{
    Attachment, Message, Reaction, TombstoneProof, MAX_MESSAGE_BYTES, MAX_RELAYED_REACTIONS, MAX_RELAYED_REACTORS,
};
use crate::poll::{Ballots, PollTally};
use crate::server::{ChannelKind, Member, Server, Timeout};
use crate::user::{Presence, UserProfile};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
    #[error("members without the send files permission cannot attach files here")]
    NoUploads,
    #[error("members without the send voice messages permission cannot send voice messages here")]
    NoVoiceMessages,
    #[error("members without the create polls permission cannot post polls here")]
    NoPolls,
    #[error("the signer may not do that here")]
    NotAllowed,
    #[error("unsigned event from a member whose key is known")]
    Unsigned,
    #[error("that server already holds as many of those as it can")]
    Limit,
    #[error("that member is timed out")]
    TimedOut,
    #[error("that thread is locked")]
    ThreadLocked,
}

const MAX_THREAD_DEPTH: usize = 64;

pub const TOMBSTONE_KEEP_DAYS: i64 = 60;

pub const MAX_UNAPPLIED_PROOFS: usize = 2000;

pub const MAX_AHEAD_MS: i64 = 86_400_000;
const STEPPED_DOWN_SKEW_MS: i64 = 600_000;

pub const MAX_ACTION_TIMES: usize = 16384;
pub const MAX_WAITING: usize = 4096;
const MAX_WAITING_PER_MESSAGE: usize = 64;
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredProof {
    proof: TombstoneProof,
    accepted_for: Option<UserId>,
}

type UnappliedKey = (DateTime<Utc>, [u8; 64], MessageId, ChannelId);

#[derive(Debug, Clone)]
pub struct ServerState {
    pub server: Server,
    pub profiles: HashMap<UserId, UserProfile>,
    messages: HashMap<ChannelId, Vec<Message>>,
    index: HashMap<MessageId, ChannelId>,
    pub voice_states: HashMap<UserId, ChannelId>,
    pub voice_since: HashMap<UserId, i64>,
    pub voice_flags: HashMap<UserId, (bool, bool)>,
    pub screen_shares: HashMap<UserId, ChannelId>,
    pub cameras: HashMap<UserId, ChannelId>,
    pub hands: HashMap<UserId, ChannelId>,
    pub activities: HashMap<ChannelId, crate::activity::Activity>,
    devices: Devices,
    deleted_messages: HashSet<MessageId>,
    tombstones: HashMap<MessageId, Vec<StoredProof>>,
    unapplied: Option<HashMap<UserId, BTreeSet<UnappliedKey>>>,
    action_times: HashMap<String, i64>,
    action_floor: i64,
    signed_actions: HashMap<String, SignedEvent>,
    local_floor: Option<DateTime<Utc>>,
    reaction_marks: HashMap<MessageId, Vec<ReactionMark>>,
    channel_keys: HashMap<ChannelId, ChannelKeys>,
    community_keys: ChannelKeys,
    automod: AutoMod,
    bots: BTreeSet<UserId>,
    bot_commands: BTreeMap<UserId, Vec<BotCommand>>,
    audit: Vec<AuditEntry>,
    deleted: bool,
    invites: BTreeMap<String, TrackedInvite>,
    invite_background: Option<String>,
    ballots: HashMap<MessageId, Ballots>,
    closed_polls: HashSet<MessageId>,
    waiting: HashMap<MessageId, Vec<Waiting>>,
}

#[derive(Debug, Clone)]
enum Waiting {
    Plain(ServerEvent),
    Signed(SignedEvent),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionMark {
    pub user: UserId,
    pub emoji: String,
    pub at: i64,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub server: Server,
    pub profiles: Vec<UserProfile>,
    pub messages: Vec<(ChannelId, Vec<Message>)>,
    #[serde(default)]
    pub deleted_messages: Vec<MessageId>,
    #[serde(default)]
    pub tombstone_proofs: Vec<TombstoneProof>,
    #[serde(default)]
    pub accepted: Vec<(MessageId, UserId, ChannelId, UserId)>,
    #[serde(default)]
    pub action_times: Vec<(String, i64)>,
    #[serde(default)]
    pub action_floor: i64,
    #[serde(default)]
    pub signed_actions: Vec<SignedEvent>,
    #[serde(default)]
    pub reaction_marks: Vec<(MessageId, ReactionMark)>,
    #[serde(default)]
    pub channel_keys: Vec<(ChannelId, ChannelKeys)>,
    #[serde(default)]
    pub community_keys: ChannelKeys,
    #[serde(default)]
    pub automod: AutoMod,
    #[serde(default)]
    pub bots: Vec<UserId>,
    #[serde(default)]
    pub bot_commands: Vec<(UserId, Vec<BotCommand>)>,
    #[serde(default)]
    pub audit: Vec<AuditEntry>,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub invites: Vec<TrackedInvite>,
    #[serde(default)]
    pub invite_background: Option<String>,
    #[serde(default)]
    pub ballots: Vec<(MessageId, UserId, Vec<u8>)>,
    #[serde(default)]
    pub closed_polls: Vec<MessageId>,
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
            hands: HashMap::new(),
            activities: HashMap::new(),
            devices: Devices::default(),
            deleted_messages: Default::default(),
            tombstones: HashMap::new(),
            unapplied: None,
            action_times: HashMap::new(),
            action_floor: 0,
            signed_actions: HashMap::new(),
            local_floor: None,
            reaction_marks: HashMap::new(),
            channel_keys: HashMap::new(),
            community_keys: ChannelKeys::default(),
            automod: AutoMod::default(),
            bots: BTreeSet::new(),
            bot_commands: BTreeMap::new(),
            audit: Vec::new(),
            deleted: false,
            invites: BTreeMap::new(),
            invite_background: None,
            ballots: HashMap::new(),
            closed_polls: HashSet::new(),
            waiting: HashMap::new(),
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
            reaction_marks: self
                .reaction_marks
                .iter()
                .filter(|(id, _)| self.index.contains_key(id))
                .flat_map(|(id, marks)| marks.iter().map(|m| (*id, m.clone())))
                .collect(),
            channel_keys: self.channel_keys.iter().map(|(c, k)| (*c, k.clone())).collect(),
            community_keys: self.community_keys.clone(),
            automod: self.automod.clone(),
            bots: self.bots.iter().copied().collect(),
            bot_commands: self.bot_commands.iter().map(|(bot, list)| (*bot, list.clone())).collect(),
            audit: self.audit.clone(),
            deleted: self.deleted,
            invites: self.invites.values().cloned().collect(),
            invite_background: self.invite_background.clone(),
            ballots: self.ballots().map(|(id, user, choices)| (id, user, choices.to_vec())).collect(),
            closed_polls: self.closed_polls.iter().filter(|id| self.index.contains_key(id)).copied().collect(),
        }
    }

    pub fn from_snapshot(snapshot: Snapshot) -> Self {
        let mut state = Self::new(snapshot.server);
        for profile in snapshot.profiles {
            let mut profile = profile;
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
            if state.messages.contains_key(&proof.channel) {
                let accepted_for = accepted.get(&(proof.message, proof.by, proof.channel)).copied();
                state.store_proof(proof, accepted_for);
            }
        }
        for (channel, mut list) in snapshot.messages {
            let Some(slot) = state.messages.get_mut(&channel) else { continue };
            list.retain(|m| {
                let new = !state.index.contains_key(&m.id);
                if new {
                    state.index.insert(m.id, channel);
                }
                new
            });
            slot.extend(list);
        }
        for (id, mark) in snapshot.reaction_marks {
            if state.index.contains_key(&id) {
                state.mark_reaction(id, mark.user, &mark.emoji, mark.at, mark.active);
            }
        }
        state.channel_keys = snapshot
            .channel_keys
            .into_iter()
            .filter(|(channel, _)| state.server.channel(*channel).is_some())
            .collect();
        state.community_keys = snapshot.community_keys;
        state.action_times = snapshot.action_times.into_iter().collect();
        state.action_floor = snapshot.action_floor;
        state.automod = snapshot.automod;
        state.bots = snapshot.bots.into_iter().filter(|user| state.server.is_member(*user)).collect();
        state.bot_commands = snapshot
            .bot_commands
            .into_iter()
            .filter(|(bot, list)| state.bots.contains(bot) && !list.is_empty() && crate::command::valid_commands(list))
            .collect();
        state.audit = crate::audit::tidy(snapshot.audit);
        state.deleted = snapshot.deleted;
        state.invites = snapshot.invites.into_iter().filter(TrackedInvite::valid).map(|i| (i.id(), i)).collect();
        state.invite_background = snapshot.invite_background;
        for (id, user, choices) in snapshot.ballots {
            if state.index.contains_key(&id) && !choices.is_empty() {
                state.ballots.entry(id).or_default().insert(user, choices);
            }
        }
        state.closed_polls = snapshot.closed_polls.into_iter().filter(|id| state.index.contains_key(id)).collect();
        for signed in snapshot.signed_actions {
            if let Some(subject) = action::subject(&signed.event) {
                if state.action_times.get(&subject) == Some(&signed.action.at) && state.still_verifies(&signed) {
                    state.signed_actions.insert(subject, signed);
                }
            }
        }
        state
    }

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
        self.prune_invites(now.timestamp_millis());
        removed
    }

    fn prune_invites(&mut self, now_ms: i64) {
        let expired: Vec<String> = self.invites.values().filter(|i| !i.live(now_ms)).map(TrackedInvite::id).collect();
        for id in expired {
            self.invites.remove(&id);
            self.signed_actions.remove(&format!("invite:{id}"));
        }
    }

    pub fn tracked_invites(&self, now_ms: i64) -> Vec<&TrackedInvite> {
        let mut live: Vec<&TrackedInvite> = self.invites.values().filter(|i| i.live(now_ms)).collect();
        live.sort_by_key(|i| std::cmp::Reverse(i.created_at));
        live
    }

    pub fn tracked_invite(&self, origin: &str, code: &str) -> Option<&TrackedInvite> {
        self.invites.get(&crate::invites::invite_id(origin, code))
    }

    pub fn invite_background(&self) -> Option<&str> {
        self.invite_background.as_deref()
    }

    pub fn roll_off(&mut self, floor: Option<DateTime<Utc>>) -> usize {
        self.local_floor = floor;
        let Some(floor) = floor else { return 0 };
        if !self.messages.values().flatten().any(|m| m.created_at < floor) {
            return 0;
        }
        let mut pinned: HashSet<MessageId> = self.server.pins.iter().map(|p| p.message).collect();
        pinned.extend(self.server.channels.iter().flat_map(|c| c.posts.keys().copied()));
        let parents: HashMap<MessageId, MessageId> =
            self.messages.values().flatten().filter_map(|m| Some((m.id, m.reply_to?))).collect();
        let mut keep: HashSet<MessageId> = self
            .messages
            .values()
            .flatten()
            .filter(|m| m.created_at >= floor || pinned.contains(&m.id))
            .map(|m| m.id)
            .collect();
        let mut unvisited: Vec<MessageId> = keep.iter().copied().collect();
        while let Some(id) = unvisited.pop() {
            if let Some(&parent) = parents.get(&id) {
                if keep.insert(parent) {
                    unvisited.push(parent);
                }
            }
        }
        let Self { messages, index, .. } = self;
        let mut removed = 0;
        for list in messages.values_mut() {
            let before = list.len();
            list.retain(|m| {
                let kept = keep.contains(&m.id);
                if !kept {
                    index.remove(&m.id);
                }
                kept
            });
            removed += before - list.len();
        }
        removed
    }

    fn rolled_off(&self, message: &Message) -> bool {
        self.local_floor.is_some_and(|floor| message.created_at < floor)
            && !self.server.is_pinned(message.id)
            && !self.has_message(message.id)
    }

    pub fn retention_cutoff(&self, channel: ChannelId, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.server.channel(channel).and_then(|c| retention_cutoff(c, now))
    }

    pub fn channel_keys(&self, channel: ChannelId) -> Option<&ChannelKeys> {
        self.channel_keys.get(&channel)
    }

    pub fn community_keys(&self) -> &ChannelKeys {
        &self.community_keys
    }

    pub fn key_chain(&self, scope: KeyScope) -> Option<&ChannelKeys> {
        match scope {
            KeyScope::Channel(channel) => self.channel_keys(channel),
            KeyScope::Community => Some(&self.community_keys),
        }
    }

    pub fn identity_of(&self, user: UserId) -> Option<IdentityKey> {
        self.profiles.get(&user).and_then(|p| p.identity)
    }

    fn asker_identity(&self, user: UserId) -> Option<IdentityKey> {
        crate::join_request::request_of(&self.server, user).and_then(|r| r.profile.identity)
    }

    fn key_of(&self, user: UserId) -> Result<IdentityKey, StoreError> {
        self.identity_of(user).ok_or(StoreError::UnknownAuthorKey(user))
    }

    pub fn messages_since(&self, channel: ChannelId, since: DateTime<Utc>) -> impl Iterator<Item = &Message> {
        self.messages(channel).iter().filter(move |m| m.created_at >= since)
    }

    pub fn message(&self, id: MessageId) -> Option<&Message> {
        let channel = self.index.get(&id)?;
        self.messages.get(channel)?.iter().rev().find(|m| m.id == id)
    }

    pub fn reaction_marks(&self, id: MessageId) -> &[ReactionMark] {
        self.reaction_marks.get(&id).map_or(&[], Vec::as_slice)
    }

    pub fn reaction_mark(&self, id: MessageId, user: UserId, emoji: &str) -> Option<&ReactionMark> {
        self.reaction_marks(id).iter().find(|m| m.user == user && m.emoji == emoji)
    }

    pub fn mark_reaction(&mut self, id: MessageId, user: UserId, emoji: &str, at: i64, active: bool) {
        let marks = self.reaction_marks.entry(id).or_default();
        match marks.iter_mut().find(|m| m.user == user && m.emoji == emoji) {
            Some(mark) => {
                mark.at = at;
                mark.active = active;
            }
            None => marks.push(ReactionMark { user, emoji: emoji.to_string(), at, active }),
        }
    }

    pub fn has_message(&self, id: MessageId) -> bool {
        self.message(id).is_some()
    }

    pub fn ballots(&self) -> impl Iterator<Item = (MessageId, UserId, &[u8])> + '_ {
        self.ballots
            .iter()
            .filter(|(id, _)| self.index.contains_key(id))
            .flat_map(|(id, votes)| votes.iter().map(|(user, choices)| (*id, *user, choices.as_slice())))
    }

    pub fn poll_choices(&self, id: MessageId, user: UserId) -> &[u8] {
        self.ballots.get(&id).and_then(|b| b.get(&user)).map_or(&[], Vec::as_slice)
    }

    pub fn poll_closed(&self, id: MessageId, now: DateTime<Utc>) -> bool {
        self.closed_polls.contains(&id) || self.message(id).and_then(|m| m.poll()).is_some_and(|p| !p.is_open_at(now))
    }

    pub fn poll_tally(&self, id: MessageId, now: DateTime<Utc>) -> Option<PollTally> {
        let poll = self.message(id)?.poll()?;
        let empty = Ballots::new();
        let ballots = self.ballots.get(&id).unwrap_or(&empty);
        Some(poll.tally(ballots, self.poll_closed(id, now)))
    }

    pub fn carry_polls(&mut self, from: &ServerState) {
        for (id, user, choices) in from.ballots() {
            if self.index.contains_key(&id) && !self.ballots.get(&id).is_some_and(|b| b.contains_key(&user)) {
                self.ballots.entry(id).or_default().insert(user, choices.to_vec());
            }
        }
        for id in from.closed_polls.iter().filter(|id| self.index.contains_key(id)) {
            self.closed_polls.insert(*id);
        }
    }

    fn vote_poll(&mut self, channel: ChannelId, id: MessageId, user: UserId, choices: Vec<u8>) -> Result<bool, StoreError> {
        let message = self.find_message_mut(channel, id)?;
        let Some(poll) = message.poll() else { return Ok(false) };
        if self.poll_closed(id, Utc::now()) {
            return Ok(false);
        }
        let Some(choices) = poll.normalize_choices(&choices) else { return Ok(false) };
        let votes = self.ballots.entry(id).or_default();
        let before = votes.get(&user).map_or(&[][..], Vec::as_slice);
        if before == choices.as_slice() {
            return Ok(false);
        }
        if choices.is_empty() {
            votes.remove(&user);
        } else {
            votes.insert(user, choices);
        }
        Ok(true)
    }

    fn close_poll(&mut self, channel: ChannelId, id: MessageId, by: UserId) -> Result<bool, StoreError> {
        let message = self.find_message_mut(channel, id)?;
        let author = message.author;
        if message.poll().is_none() {
            return Ok(false);
        }
        if author != by && !self.server.permissions_for(by).allows(crate::permissions::Permissions::MANAGE_MESSAGES) {
            return Err(StoreError::NotAllowed);
        }
        Ok(self.closed_polls.insert(id))
    }

    pub fn deleted_ids(&self) -> impl Iterator<Item = MessageId> + '_ {
        self.deleted_messages.iter().chain(self.tombstones.keys()).copied()
    }

    pub fn tombstone(&self, id: MessageId) -> Option<&TombstoneProof> {
        let list = self.tombstones.get(&id)?;
        list.iter().min_by_key(|s| proof_rank(s)).map(|s| &s.proof)
    }

    pub fn tombstones_for(&self, id: MessageId) -> Vec<&TombstoneProof> {
        let mut list: Vec<&StoredProof> = self.tombstones.get(&id).into_iter().flatten().collect();
        list.sort_by_key(|s| proof_rank(s));
        list.into_iter().map(|s| &s.proof).collect()
    }

    pub fn tombstones_since(&self, since: DateTime<Utc>) -> impl Iterator<Item = &TombstoneProof> {
        let listed = self.listed();
        self.stored_proofs()
            .filter(move |s| s.proof.at >= since && listed(s))
            .map(|s| &s.proof)
    }

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

    fn held_with_proofs(&self) -> HashSet<MessageId> {
        self.tombstones.keys().filter(|id| self.has_message(**id)).copied().collect()
    }

    pub fn messages(&self, channel: ChannelId) -> &[Message] {
        self.messages.get(&channel).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn automod(&self) -> &AutoMod {
        &self.automod
    }

    pub fn is_bot(&self, user: UserId) -> bool {
        self.bots.contains(&user)
    }

    pub fn bots(&self) -> impl Iterator<Item = UserId> + '_ {
        self.bots.iter().copied()
    }

    pub fn commands_of(&self, bot: UserId) -> &[BotCommand] {
        self.bot_commands.get(&bot).map_or(&[], Vec::as_slice)
    }

    pub fn bot_commands(&self) -> impl Iterator<Item = (UserId, &[BotCommand])> + '_ {
        self.bot_commands.iter().map(|(bot, list)| (*bot, list.as_slice()))
    }

    pub fn command_of(&self, used: &CommandUse) -> Option<&BotCommand> {
        self.commands_of(used.bot).iter().find(|c| c.name == used.command).filter(|c| used.fits(c))
    }

    pub fn carry_bot_commands(&mut self, from: &ServerState) {
        for (bot, list) in from.bot_commands() {
            if self.bots.contains(&bot) && !self.bot_commands.contains_key(&bot) {
                self.bot_commands.insert(bot, list.to_vec());
            }
        }
    }

    pub fn hold(&self, message: &Message) -> Option<Hold> {
        self.automod.hold(&self.server, self.messages(message.channel), message)
    }

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
        self.apply_noted(signed)
    }

    pub fn apply_unsigned(&mut self, event: ServerEvent) -> Result<bool, StoreError> {
        self.admits_unsigned(&event)?;
        self.apply(event)
    }

    pub fn apply_own(&mut self, signed: &SignedEvent) -> Result<bool, StoreError> {
        if !self.fresh(action::subject(&signed.event).as_deref(), signed.action.at) {
            return Ok(false);
        }
        self.apply_noted(signed)
    }

    pub fn join_as_of(&mut self, profile: UserProfile, at: i64) -> Result<bool, StoreError> {
        let event = ServerEvent::MemberJoin { server: self.server.id, profile };
        let subject = action::subject(&event).expect("a join has a subject");
        let applied = self.apply(event)?;
        if self.action_times.get(&subject).is_none_or(|prev| at > *prev) {
            self.note_action(Some(subject.clone()), at);
            self.signed_actions.remove(&subject);
        }
        Ok(applied)
    }

    fn apply_noted(&mut self, signed: &SignedEvent) -> Result<bool, StoreError> {
        let Action { by, at, sig } = signed.action;
        let entry = crate::audit::entry_for(self, by, at, sig, &signed.event);
        let moderators = self.moderators_before(&signed.event);
        let applied = match self.apply_or_wait(signed.event.clone(), false) {
            Err(StoreError::UnknownMessage) => {
                self.wait_for_message(Waiting::Signed(signed.clone()));
                return Err(StoreError::UnknownMessage);
            }
            other => other?,
        };
        if applied {
            self.note_stepped_down(moderators, at);
        }
        if let Some(entry) = entry.filter(|_| applied) {
            crate::audit::record(&mut self.audit, entry);
        }
        if applied || self.target_known(&signed.event) {
            self.note_action(action::subject(&signed.event), signed.action.at);
            self.keep_signed(signed);
        }
        if let (true, ServerEvent::MemberUnban { user, .. }) = (applied, &signed.event) {
            self.readmit_if_joined_since(*user, at);
        }
        Ok(applied)
    }

    fn last_join(&self, user: UserId) -> Option<i64> {
        let subject = format!("member:{user}");
        let at = *self.action_times.get(&subject)?;
        let left = self.signed_actions.get(&subject).is_some_and(|s| matches!(s.event, ServerEvent::MemberLeave { .. }));
        (!left).then_some(at)
    }

    fn readmit_if_joined_since(&mut self, user: UserId, unbanned_at: i64) {
        if self.server.is_member(user) || self.last_join(user).is_none_or(|joined| joined <= unbanned_at) {
            return;
        }
        let Some(profile) = self.profiles.get(&user).cloned() else { return };
        let _ = self.apply(ServerEvent::MemberJoin { server: self.server.id, profile });
    }

    fn moderators_before(&self, event: &ServerEvent) -> Vec<UserId> {
        use ServerEvent as E;
        let reshapes = matches!(
            event,
            E::RoleDelete { .. } | E::RoleUnassign { .. } | E::RoleUpdate { .. } | E::MemberLeave { .. } | E::MemberBan { .. } | E::OwnerTransfer { .. }
        );
        if !reshapes {
            return Vec::new();
        }
        let server = &self.server;
        server.members.iter().map(|m| m.user).filter(|user| server.permissions_for(*user).allows(crate::permissions::Permissions::MANAGE_MESSAGES)).collect()
    }

    fn note_stepped_down(&mut self, moderators: Vec<UserId>, at: i64) {
        for user in moderators {
            if !self.server.permissions_for(user).allows(crate::permissions::Permissions::MANAGE_MESSAGES) {
                let since = self.server.stepped_down.entry(user).or_insert(at);
                *since = (*since).max(at);
            }
        }
    }

    fn target_known(&self, event: &ServerEvent) -> bool {
        use ServerEvent as E;
        let server = &self.server;
        match event {
            E::ChannelDelete { channel, .. }
            | E::ChannelUpdate { channel, .. }
            | E::ChannelLayout { channel, .. }
            | E::ChannelLimit { channel, .. }
            | E::ChannelSlowMode { channel, .. }
            | E::ChannelBanner { channel, .. }
            | E::ChannelColor { channel, .. }
            | E::ChannelKindSet { channel, .. }
            | E::ChannelAccessSet { channel, .. }
            | E::ChannelSensitiveSet { channel, .. }
            | E::VoiceStatusSet { channel, .. }
            | E::ChannelKeyAdd { channel, .. }
            | E::ChannelKeyGrant { channel, .. }
            | E::ForumTagsSet { channel, .. }
            | E::StageSet { channel, .. } => server.channel(*channel).is_some(),
            E::RoleUpdate { role, .. } => server.roles.iter().any(|r| r.id == role.id),
            E::RoleDelete { role, .. } | E::RoleAssign { role, .. } | E::RoleUnassign { role, .. } => {
                server.roles.iter().any(|r| r.id == *role)
            }
            E::MemberBan { user, .. } | E::MemberUnban { user, .. } | E::MemberLeave { user, .. } => {
                server.is_member(*user) || server.banned.contains(user) || self.profiles.contains_key(user)
            }
            E::MemberTimeout { user, .. } => server.is_member(*user),
            E::DriveAdd { item, .. } => {
                server.channel(item.channel).is_some_and(|c| c.is_drive()) && crate::drive::parent_fits(&server.drive, item.channel, item.parent)
            }
            E::DriveEdit { item, .. } | E::DriveRemove { item, .. } | E::DriveHold { item, .. } => crate::drive::find(&server.drive, *item).is_some(),
            E::VoiceState { user, channel, .. } => self.voice_claim_fits(*user, *channel),
            E::EventDelete { event, .. } | E::EventRsvp { event, .. } => server.events.iter().any(|e| e.id == *event),
            E::EmojiRemove { name, .. } => server.emojis.iter().any(|e| e.name == *name),
            E::StickerRemove { name, .. } => server.stickers.iter().any(|s| s.name == *name),
            E::EmoticonRemove { name, .. } => server.emoticons.iter().any(|e| e.name == *name),
            E::SoundRemove { name, .. } => server.soundboard.iter().any(|s| s.name == *name),
            E::MessagePin { message, .. }
            | E::ReactionSet { message, .. }
            | E::ReactionToggle { message, .. }
            | E::PollVote { message, .. }
            | E::PollClose { message, .. }
            | E::ThreadSet { root: message, .. } => self.has_message(*message),
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
                    || self.missing_from_trimmed_snapshot(subject)
                    || at + MAX_ACTION_FLOOR_MS >= self.action_floor
            }
        }
    }

    fn missing_from_trimmed_snapshot(&self, subject: &str) -> bool {
        match subject {
            "images" => self.server.icon.is_none() && self.server.banner.is_none(),
            "game_tags" => self.server.game_tags.is_empty(),
            _ => false,
        }
    }

    fn still_verifies(&self, signed: &SignedEvent) -> bool {
        self.signer_key(signed).is_some_and(|key| signed.verify(self.server.id, &key))
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
            ServerEvent::JoinAsk { profile, .. } if profile.id == by => self.asker_identity(by).or(profile.identity),
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

    fn forget_drive_actions(&mut self, gone: &[crate::ids::DriveId]) {
        if gone.is_empty() {
            return;
        }
        let ids: std::collections::HashSet<String> = gone.iter().map(|id| id.to_string()).collect();
        let about_gone = |subject: &str| {
            let rest = subject.strip_prefix("drive:").or_else(|| subject.strip_prefix("drive.edit:")).or_else(|| subject.strip_prefix("drive.hold:"));
            rest.is_some_and(|rest| ids.contains(rest.split(':').next().unwrap_or_default()))
        };
        self.action_times.retain(|subject, _| !about_gone(subject));
        self.signed_actions.retain(|subject, _| !about_gone(subject));
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

    pub fn audit_log(&self) -> &[AuditEntry] {
        &self.audit
    }

    pub fn is_deleted(&self) -> bool {
        self.deleted
    }

    fn same_server(&self, server: ServerId) -> Result<(), StoreError> {
        if server != self.server.id {
            return Err(StoreError::WrongServer);
        }
        Ok(())
    }

    pub fn apply(&mut self, event: ServerEvent) -> Result<bool, StoreError> {
        self.apply_or_wait(event, true)
    }

    fn apply_or_wait(&mut self, event: ServerEvent, wait: bool) -> Result<bool, StoreError> {
        use ServerEvent as E;
        let grants = matches!(event, E::RoleAssign { .. } | E::RoleUpdate { .. } | E::OwnerTransfer { .. });
        let created = match &event {
            E::MessageCreate { message } => Some(message.id),
            _ => None,
        };
        let kept = (wait && waits_for(&event).is_some()).then(|| event.clone());
        let applied = match self.apply_event(event) {
            Err(StoreError::UnknownMessage) => {
                if let Some(event) = kept {
                    self.wait_for_message(Waiting::Plain(event));
                }
                return Err(StoreError::UnknownMessage);
            }
            other => other?,
        };
        if applied && grants {
            self.retry_held_proofs();
        }
        if let Some(id) = created {
            self.release_waiting(id);
        }
        Ok(applied)
    }

    fn wait_for_message(&mut self, waiting: Waiting) {
        let event = match &waiting {
            Waiting::Plain(event) => event,
            Waiting::Signed(signed) => &signed.event,
        };
        let Some(id) = waits_for(event) else { return };
        if self.tombstones.contains_key(&id) || self.deleted_messages.contains(&id) {
            return;
        }
        let total: usize = self.waiting.values().map(Vec::len).sum();
        let list = self.waiting.entry(id).or_default();
        if total < MAX_WAITING && list.len() < MAX_WAITING_PER_MESSAGE {
            list.push(waiting);
        }
    }

    fn release_waiting(&mut self, id: MessageId) {
        let Some(list) = self.waiting.remove(&id) else { return };
        if !self.has_message(id) {
            return;
        }
        for waiting in list {
            let _ = match waiting {
                Waiting::Plain(event) => self.apply_or_wait(event, false),
                Waiting::Signed(signed) => self.apply_signed(&signed),
            };
        }
    }

    fn apply_event(&mut self, event: ServerEvent) -> Result<bool, StoreError> {
        if self.deleted {
            return Ok(false);
        }
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
                let active = msg.reactions.iter().any(|r| r.emoji == emoji && r.users.contains(&user));
                self.mark_reaction(message, user, &emoji, Utc::now().timestamp_millis(), active);
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
                self.mark_reaction(message, user, &emoji, Utc::now().timestamp_millis(), active);
                Ok(true)
            }
            ServerEvent::MemberJoin { server, mut profile } => {
                self.same_server(server)?;
                if self.server.banned.contains(&profile.id) || crate::join_request::may_ask(&self.server, profile.id) {
                    return Ok(false);
                }
                self.pin_identity(&mut profile)?;
                profile = profile.as_seen_by_others();
                if self.server.is_member(profile.id) {
                    let changed = self.profiles.get(&profile.id) != Some(&profile);
                    self.profiles.insert(profile.id, profile);
                    return Ok(changed);
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
                    timeout: None,
                });
                self.profiles.insert(profile.id, profile);
                Ok(true)
            }
            ServerEvent::MemberLeave { server, user } => {
                self.same_server(server)?;
                let before = self.server.members.len();
                self.server.members.retain(|m| m.user != user);
                self.drop_voice_presence(user);
                self.devices.forget(user);
                self.bots.remove(&user);
                self.bot_commands.remove(&user);
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
                self.same_server(server)?;
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
                self.same_server(server)?;
                let before = self.server.channels.len();
                self.server.channels.retain(|c| c.id != channel);
                self.server.pins.retain(|p| p.channel != channel);
                self.channel_keys.remove(&channel);
                self.hands.retain(|_, c| *c != channel);
                for m in self.messages.remove(&channel).into_iter().flatten() {
                    self.index.remove(&m.id);
                }
                if retain_proofs(&mut self.tombstones, |s| s.proof.channel != channel) {
                    self.unapplied = None;
                }
                let new_tombstone = !self.server.deleted_channels.contains(&channel);
                if new_tombstone {
                    push_capped(&mut self.server.deleted_channels, channel, MAX_TOMBSTONED_IDS);
                }
                if self.server.afk_channel_id == Some(channel) {
                    self.server.afk_channel_id = None;
                }
                let gone = crate::drive::clear_channel(&mut self.server.drive, channel);
                self.forget_drive_actions(&gone);
                Ok(self.server.channels.len() != before || new_tombstone)
            }
            ServerEvent::ChannelUpdate {
                server,
                channel,
                name,
                retention_days,
            } => {
                self.same_server(server)?;
                let name = name.trim().to_string();
                if name.is_empty() {
                    return Ok(false);
                }
                let Some(target) = self.server.channel_mut(channel) else {
                    return Err(StoreError::UnknownChannel);
                };
                let a = replace(&mut target.name, name);
                let b = replace(&mut target.retention_days, retention_days.filter(|d| *d > 0));
                if !(a | b) {
                    return Ok(false);
                }
                self.prune_expired(Utc::now());
                Ok(true)
            }
            ServerEvent::AfkChannelSet { server, channel } => {
                self.same_server(server)?;
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
            ServerEvent::VoiceState { user, channel, .. } if !self.voice_claim_fits(user, channel) => Ok(false),
            ServerEvent::VoiceState { user, channel, since, muted, deafened } => match channel {
                Some(ch) => {
                    if self.server.channel(ch).is_none() {
                        return Err(StoreError::UnknownChannel);
                    }
                    let moved = self.seat(user, ch, since)?;
                    let flagged = self.voice_flags.insert(user, (muted, deafened)) != Some((muted, deafened));
                    Ok(moved || flagged)
                }
                None => Ok(self.end_seat(user)),
            },
            ServerEvent::ScreenShare {
                user,
                channel,
                active,
            } => {
                let known = self.server.channel(channel).is_some();
                set_live(&mut self.screen_shares, known, user, channel, active)
            }
            ServerEvent::CameraState {
                user,
                channel,
                active,
            } => {
                let known = self.server.channel(channel).is_some();
                set_live(&mut self.cameras, known, user, channel, active)
            }
            ServerEvent::EmojiAdd { server, emoji } => {
                self.same_server(server)?;
                if !crate::server::valid_expression_name(&emoji.name)
                    || crate::server::ImageRef::parse(&emoji.blob).is_none()
                {
                    return Ok(false);
                }
                Ok(upsert_by(&mut self.server.emojis, emoji, crate::server::MAX_EMOJIS, |a, b| a.name == b.name))
            }
            ServerEvent::EmojiRemove { server, name } => {
                self.same_server(server)?;
                let before = self.server.emojis.len();
                self.server.emojis.retain(|e| e.name != name);
                Ok(self.server.emojis.len() != before)
            }
            ServerEvent::SoundAdd { server, sound } => {
                self.same_server(server)?;
                if !crate::server::valid_expression_name(&sound.name)
                    || crate::server::ImageRef::parse(&sound.blob).is_none()
                    || !(0.0..=2.0).contains(&sound.volume)
                    || sound.emoji.as_deref().is_some_and(|e| !crate::server::valid_sound_emoji(e))
                {
                    return Ok(false);
                }
                Ok(upsert_by(&mut self.server.soundboard, sound, crate::server::MAX_SOUNDS, |a, b| a.name == b.name))
            }
            ServerEvent::SoundRemove { server, name } => {
                self.same_server(server)?;
                let before = self.server.soundboard.len();
                self.server.soundboard.retain(|s| s.name != name);
                Ok(self.server.soundboard.len() != before)
            }
            ServerEvent::ExpressionSettings { server, emojis_shareable, soundboard_open } => {
                self.same_server(server)?;
                let a = replace(&mut self.server.emojis_shareable, emojis_shareable);
                let b = replace(&mut self.server.soundboard_open, soundboard_open);
                Ok(a | b)
            }
            ServerEvent::PublicSet { server, public } => {
                self.same_server(server)?;
                Ok(replace(&mut self.server.public, public))
            }
            ServerEvent::SoundSharingSet { server, shareable } => {
                self.same_server(server)?;
                Ok(replace(&mut self.server.sounds_shareable, shareable))
            }
            ServerEvent::ActivitiesSet { server, enabled } => {
                self.same_server(server)?;
                Ok(replace(&mut self.server.activities_enabled, enabled))
            }
            ServerEvent::DeviceAnnounce { server, user, node, voice, since } => {
                self.same_server(server)?;
                self.note_device(user, node, voice, since)
            }
            ServerEvent::ServerDelete { server } => {
                self.same_server(server)?;
                self.deleted = true;
                Ok(true)
            }
            ServerEvent::OwnerTransfer { server, user } => {
                self.same_server(server)?;
                if user == self.server.owner || !self.server.is_member(user) {
                    return Ok(false);
                }
                self.server.owner = user;
                Ok(true)
            }
            ServerEvent::InviteTrack { server, invite } => {
                self.same_server(server)?;
                let now = Utc::now().timestamp_millis();
                if !invite.valid() || !invite.live(now) || self.invites.contains_key(&invite.id()) {
                    return Ok(false);
                }
                if self.invites.len() >= crate::invites::MAX_TRACKED_INVITES {
                    self.prune_invites(now);
                }
                if self.invites.len() >= crate::invites::MAX_TRACKED_INVITES {
                    return Err(StoreError::Limit);
                }
                self.invites.insert(invite.id(), invite);
                Ok(true)
            }
            ServerEvent::InviteRevoke { server, origin, code, creator } => {
                self.same_server(server)?;
                let id = crate::invites::invite_id(&origin, &code);
                if self.invites.get(&id).is_none_or(|i| i.creator != creator) {
                    return Ok(false);
                }
                self.invites.remove(&id);
                Ok(true)
            }
            ServerEvent::InviteBackgroundSet { server, background } => {
                self.same_server(server)?;
                if background.as_deref().is_some_and(|b| crate::server::ImageRef::parse(b).is_none()) {
                    return Ok(false);
                }
                Ok(replace(&mut self.invite_background, background))
            }
            ServerEvent::SoundboardPlay { .. } | ServerEvent::SharedSoundPlay { .. } | ServerEvent::ActivityInput { .. } => Ok(false),
            ServerEvent::ActivitySet { server, channel, activity, by } => {
                self.same_server(server)?;
                if !crate::activity::admits_set(self, by, channel, activity.as_ref()) {
                    return Ok(false);
                }
                match activity {
                    Some(next) => Ok(self.activities.insert(channel, next.clone()) != Some(next)),
                    None => Ok(self.activities.remove(&channel).is_some()),
                }
            }
            ServerEvent::StickerAdd { server, sticker } => {
                self.same_server(server)?;
                if !crate::server::valid_expression_name(&sticker.name)
                    || crate::server::ImageRef::parse(&sticker.blob).is_none()
                {
                    return Ok(false);
                }
                Ok(upsert_by(&mut self.server.stickers, sticker, crate::server::MAX_STICKERS, |a, b| a.name == b.name))
            }
            ServerEvent::StickerRemove { server, name } => {
                self.same_server(server)?;
                let before = self.server.stickers.len();
                self.server.stickers.retain(|s| s.name != name);
                Ok(self.server.stickers.len() != before)
            }
            ServerEvent::EmoticonAdd { server, emoticon } => {
                self.same_server(server)?;
                let clean = crate::server::clean_emoticon(&emoticon.text);
                if !crate::server::valid_expression_name(&emoticon.name) || clean.as_deref() != Some(emoticon.text.as_str()) {
                    return Ok(false);
                }
                Ok(upsert_by(&mut self.server.emoticons, emoticon, crate::server::MAX_EMOTICONS, |a, b| a.name == b.name))
            }
            ServerEvent::EmoticonRemove { server, name } => {
                self.same_server(server)?;
                let before = self.server.emoticons.len();
                self.server.emoticons.retain(|e| e.name != name);
                Ok(self.server.emoticons.len() != before)
            }
            ServerEvent::ExpressionSharingSet { server, stickers, emoticons } => {
                self.same_server(server)?;
                let a = replace(&mut self.server.stickers_shareable, stickers);
                let b = replace(&mut self.server.emoticons_shareable, emoticons);
                Ok(a | b)
            }
            ServerEvent::GameTagsSet { server, tags, meta } => {
                self.same_server(server)?;
                let Some(tags) = crate::server::clean_game_tags(&tags) else { return Ok(false) };
                let meta = crate::server::clean_game_tag_meta(&tags, &meta);
                let a = replace(&mut self.server.game_tags, tags);
                let b = replace(&mut self.server.game_tag_meta, meta);
                Ok(a | b)
            }
            ServerEvent::RulesSet { server, rules } => {
                self.same_server(server)?;
                if rules.as_deref().is_some_and(|r| !crate::server::valid_rules(r)) {
                    return Ok(false);
                }
                Ok(replace(&mut self.server.rules, rules))
            }
            ServerEvent::AutoModSet { server, rules } => {
                self.same_server(server)?;
                if !rules.valid() {
                    return Ok(false);
                }
                Ok(replace(&mut self.automod, rules))
            }
            ServerEvent::OnboardingSet { server, onboarding } => {
                self.same_server(server)?;
                if onboarding.as_ref().is_some_and(|o| !o.valid()) {
                    return Ok(false);
                }
                Ok(replace(&mut self.server.onboarding, onboarding))
            }
            ServerEvent::BotSet { server, user, bot } => {
                self.same_server(server)?;
                if !bot {
                    self.bot_commands.remove(&user);
                    return Ok(self.bots.remove(&user));
                }
                Ok(self.server.is_member(user) && self.bots.insert(user))
            }
            ServerEvent::BotCommandsSet { server, bot, commands, by } => {
                self.same_server(server)?;
                if by != bot || !self.bots.contains(&bot) || !self.server.is_member(bot) || !crate::command::valid_commands(&commands) {
                    return Ok(false);
                }
                if commands.is_empty() {
                    return Ok(self.bot_commands.remove(&bot).is_some());
                }
                Ok(self.bot_commands.insert(bot, commands.clone()).as_ref() != Some(&commands))
            }
            ServerEvent::PronounsSet { server, user, pronouns } => {
                self.same_server(server)?;
                if !self.server.is_member(user) {
                    return Ok(false);
                }
                match pronouns.as_deref().map(crate::user::clean_pronouns) {
                    None => Ok(self.server.pronouns.remove(&user).is_some()),
                    Some(None) => Ok(false),
                    Some(Some(text)) => Ok(self.server.pronouns.insert(user, text.clone()).as_deref() != Some(text.as_str())),
                }
            }
            ServerEvent::JoinSoundSet { server, user, sound } => {
                self.same_server(server)?;
                if !self.server.is_member(user) {
                    return Ok(false);
                }
                match sound {
                    None => Ok(self.server.join_sounds.remove(&user).is_some()),
                    Some(sound) if !crate::server::valid_join_sound(&sound) => Ok(false),
                    Some(sound) => Ok(self.server.join_sounds.insert(user, sound.clone()).as_deref() != Some(sound.as_str())),
                }
            }
            ServerEvent::JoinSoundsAllowed { server, allowed } => {
                self.same_server(server)?;
                Ok(replace(&mut self.server.join_sounds_off, !allowed))
            }
            ServerEvent::ServerTagSet { server, tag } => {
                self.same_server(server)?;
                if tag.as_ref().is_some_and(|t| !t.is_valid()) {
                    return Ok(false);
                }
                Ok(replace(&mut self.server.tag, tag))
            }
            ServerEvent::PetSet { server, pet } => {
                self.same_server(server)?;
                Ok(crate::pet::set(&mut self.server.pet, &mut self.server.pet_care, pet))
            }
            ServerEvent::PetCare { server, user, care, at } => {
                self.same_server(server)?;
                if self.server.pet.is_none() || !self.server.is_current_member(user) {
                    return Ok(false);
                }
                Ok(crate::pet::tend(&mut self.server.pet_care, user, care, at))
            }
            ServerEvent::DriveAdd { server, item } => {
                self.same_server(server)?;
                if !self.server.channel(item.channel).is_some_and(|c| c.is_drive()) {
                    return Ok(false);
                }
                Ok(crate::drive::add(&mut self.server.drive, item))
            }
            ServerEvent::DriveEdit { server, item, name, parent } => {
                self.same_server(server)?;
                Ok(crate::drive::edit(&mut self.server.drive, item, &name, parent))
            }
            ServerEvent::DriveRemove { server, item } => {
                self.same_server(server)?;
                let gone = crate::drive::remove(&mut self.server.drive, item);
                let inside: Vec<_> = gone.iter().copied().filter(|g| *g != item).collect();
                self.forget_drive_actions(&inside);
                Ok(!gone.is_empty())
            }
            ServerEvent::DriveHold { server, item, user, node } => {
                self.same_server(server)?;
                if node.is_some() && !self.server.is_current_member(user) {
                    return Ok(false);
                }
                Ok(crate::drive::hold(&mut self.server.drive, item, user, node.as_deref()))
            }
            ServerEvent::TipJarSet { server, url } => {
                self.same_server(server)?;
                if url.as_deref().is_some_and(|u| !crate::tip_jar::is_tip_jar(u)) {
                    return Ok(false);
                }
                Ok(replace(&mut self.server.tip_jar, url))
            }
            ServerEvent::JoinApprovalSet { server, on, at } => {
                self.same_server(server)?;
                if !on {
                    self.server.join_requests.clear();
                    self.server.join_approval_since = 0;
                } else if !self.server.join_approval {
                    self.server.join_approval_since = at;
                }
                Ok(replace(&mut self.server.join_approval, on))
            }
            ServerEvent::JoinAsk { server, mut profile, note, at } => {
                self.same_server(server)?;
                let Some(note) = crate::join_request::clean_note(&note) else { return Ok(false) };
                if !crate::join_request::may_ask(&self.server, profile.id) || self.server.turned_away.contains(&profile.id) {
                    return Ok(false);
                }
                let pinned = self.identity_of(profile.id).or_else(|| self.asker_identity(profile.id));
                if !profile.pin_identity(pinned) {
                    return Err(StoreError::IdentityMismatch);
                }
                let profile = profile.as_seen_by_others();
                Ok(crate::join_request::record(&mut self.server, crate::join_request::JoinRequest { profile, note, at }))
            }
            ServerEvent::JoinAnswer { server, user, admit } => {
                self.same_server(server)?;
                if !admit {
                    return Ok(crate::join_request::turn_away(&mut self.server, user));
                }
                let asked = crate::join_request::request_of(&self.server, user).map(|r| r.profile.clone());
                let before = (self.server.join_requests.len(), self.server.turned_away.len());
                self.server.join_requests.retain(|r| r.profile.id != user);
                self.server.turned_away.retain(|u| *u != user);
                let cleared = before != (self.server.join_requests.len(), self.server.turned_away.len());
                if self.server.is_member(user) || self.server.banned.contains(&user) {
                    return Ok(cleared);
                }
                if self.server.members.len() >= MAX_MEMBERS {
                    return Err(StoreError::Limit);
                }
                self.make_profile_room(user)?;
                self.server.members.push(Member { user, nickname: None, roles: vec![], joined_at: Utc::now(), timeout: None });
                if let Some(profile) = asked.filter(|_| !self.profiles.contains_key(&user)) {
                    self.profiles.insert(user, profile);
                }
                Ok(true)
            }
            ServerEvent::MemberNickname { server, user, nickname } => {
                self.same_server(server)?;
                let Some(member) = self.server.member_mut(user) else {
                    return Ok(false);
                };
                match nickname.as_deref().map(crate::server::clean_nickname) {
                    None => Ok(replace(&mut member.nickname, None)),
                    Some(None) => Ok(false),
                    Some(cleaned) => Ok(replace(&mut member.nickname, cleaned)),
                }
            }
            ServerEvent::RulesAccept { server, user } => {
                self.same_server(server)?;
                if self.server.rules_accepted.contains(&user) || !self.server.is_member(user) {
                    return Ok(false);
                }
                self.server.rules_accepted.push(user);
                Ok(true)
            }
            ServerEvent::EventUpsert { server, event } => {
                self.same_server(server)?;
                if !crate::server::valid_event(&event) || self.server.deleted_events.contains(&event.id) {
                    return Ok(false);
                }
                match self.server.events.iter_mut().find(|e| e.id == event.id) {
                    Some(existing) => {
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
                self.same_server(server)?;
                let before = self.server.events.len();
                self.server.events.retain(|e| e.id != event);
                let new_tombstone = !self.server.deleted_events.contains(&event);
                if new_tombstone && event.len() <= 64 {
                    push_capped(&mut self.server.deleted_events, event, MAX_TOMBSTONED_IDS);
                }
                Ok(self.server.events.len() != before)
            }
            ServerEvent::EventRsvp { server, event, user, rsvp } => {
                self.same_server(server)?;
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
                self.same_server(server)?;
                let parent_ok = parent.is_none_or(|p| {
                    self.server.channel(p).is_some_and(|c| c.kind == crate::server::ChannelKind::Category)
                });
                let Some(target) = self.server.channel_mut(channel) else { return Ok(false) };
                if !parent_ok || (parent.is_some() && target.kind == crate::server::ChannelKind::Category) {
                    return Ok(false);
                }
                let a = replace(&mut target.parent, parent);
                let b = replace(&mut target.position, position);
                Ok(a | b)
            }
            ServerEvent::ChannelLimit { server, channel, user_limit } => {
                self.same_server(server)?;
                let Some(target) = self.server.channel_mut(channel) else { return Ok(false) };
                if !target.kind.is_voice() || user_limit > crate::server::MAX_USER_LIMIT {
                    return Ok(false);
                }
                Ok(replace(&mut target.user_limit, user_limit))
            }
            ServerEvent::ChannelSlowMode { server, channel, seconds } => {
                self.same_server(server)?;
                let Some(target) = self.server.channel_mut(channel) else { return Ok(false) };
                if !target.kind.holds_messages() || seconds > crate::server::MAX_SLOW_MODE {
                    return Ok(false);
                }
                Ok(replace(&mut target.slow_mode, seconds))
            }
            ServerEvent::ForumTagsSet { server, channel, tags, .. } => {
                self.same_server(server)?;
                let Some(target) = self.server.channel_mut(channel) else { return Err(StoreError::UnknownChannel) };
                let Some(tags) = crate::server::clean_forum_tags(&tags).filter(|_| target.is_forum()) else { return Ok(false) };
                if target.tags == tags {
                    return Ok(false);
                }
                target.tags = tags;
                for post in target.posts.values_mut() {
                    post.tags.retain(|t| target.tags.iter().any(|known| known.id == *t));
                }
                Ok(true)
            }
            ServerEvent::ThreadSet { server, channel, root, name, archived, locked, by } => {
                self.same_server(server)?;
                if self.server.channel(channel).is_none() {
                    return Err(StoreError::UnknownChannel);
                }
                if self.index.get(&root).is_some_and(|c| *c != channel) {
                    return Err(StoreError::WrongChannel);
                }
                if self.deleted_messages.contains(&root) || self.tombstones.contains_key(&root) {
                    return Ok(false);
                }
                if name.as_deref().is_some_and(|n| !crate::server::valid_thread_name(n)) {
                    return Ok(false);
                }
                let target = self.server.channel_mut(channel).ok_or(StoreError::UnknownChannel)?;
                if !target.kind.holds_messages() {
                    return Ok(false);
                }
                let thread = crate::server::Thread { name, archived, locked, by };
                if thread.is_plain() {
                    return Ok(target.threads.remove(&root).is_some());
                }
                if target.threads.get(&root).is_some_and(|t| t.same_settings(&thread)) {
                    return Ok(false);
                }
                if !target.threads.contains_key(&root) && target.threads.len() >= crate::server::MAX_THREADS_PER_CHANNEL {
                    return Err(StoreError::Limit);
                }
                target.threads.insert(root, thread);
                Ok(true)
            }
            ServerEvent::ForumPost { server, channel, message, title, tags, by } => {
                self.same_server(server)?;
                if self.server.channel(channel).is_none() {
                    return Err(StoreError::UnknownChannel);
                }
                let root_fits = match self.index.get(&message) {
                    Some(home) => *home == channel && self.message(message).is_some_and(|m| m.reply_to.is_none()),
                    None => true,
                };
                if !root_fits || self.deleted_messages.contains(&message) || self.tombstones.contains_key(&message) {
                    return Ok(false);
                }
                let Some(target) = self.server.channel_mut(channel).filter(|c| c.is_forum()) else { return Ok(false) };
                let Some(title) = crate::server::clean_post_title(&title) else { return Ok(false) };
                let tags = crate::server::clean_post_tags(&tags, &target.tags);
                match target.posts.get_mut(&message) {
                    Some(post) => {
                        if post.title == title && post.tags == tags {
                            return Ok(false);
                        }
                        post.title = title;
                        post.tags = tags;
                        Ok(true)
                    }
                    None => {
                        if !message.is_bound_to(by) {
                            return Ok(false);
                        }
                        if target.posts.len() >= crate::server::MAX_FORUM_POSTS {
                            return Err(StoreError::Limit);
                        }
                        target.posts.insert(message, crate::server::ForumPost { title, tags, author: by });
                        Ok(true)
                    }
                }
            }
            ServerEvent::StageSet { server, channel, user, speaker, .. } => {
                self.same_server(server)?;
                if !self.server.is_member(user) {
                    return Ok(false);
                }
                let Some(target) = self.server.channel_mut(channel) else {
                    return Err(StoreError::UnknownChannel);
                };
                if !target.is_stage() {
                    return Ok(false);
                }
                let listed = target.speakers.contains(&user);
                match (speaker, listed) {
                    (true, true) | (false, false) => Ok(false),
                    (false, true) => {
                        target.speakers.retain(|u| *u != user);
                        Ok(true)
                    }
                    (true, false) => {
                        if target.speakers.len() >= crate::server::MAX_STAGE_SPEAKERS {
                            return Err(StoreError::Limit);
                        }
                        target.speakers.push(user);
                        self.hands.remove(&user);
                        Ok(true)
                    }
                }
            }
            ServerEvent::HandRaise { server, channel, user, raised } => {
                self.same_server(server)?;
                if self.server.channel(channel).is_none() {
                    return Err(StoreError::UnknownChannel);
                }
                if !raised {
                    return Ok(self.hands.remove(&user).is_some());
                }
                if self.voice_states.get(&user) != Some(&channel) || !self.server.in_audience(channel, user) {
                    return Ok(false);
                }
                live_room(&self.hands, user)?;
                Ok(self.hands.insert(user, channel) != Some(channel))
            }
            ServerEvent::VoiceModerate { server, user, muted, deafened } => {
                self.same_server(server)?;
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
            ServerEvent::MemberTimeout { server, user, by, at, until } => {
                self.same_server(server)?;
                if user == self.server.owner {
                    return Ok(false);
                }
                let timeout = match until {
                    Some(until) => match Timeout::new(at, until, by) {
                        Some(timeout) => Some(timeout),
                        None => return Ok(false),
                    },
                    None => None,
                };
                let Some(member) = self.server.member_mut(user) else { return Ok(false) };
                Ok(replace(&mut member.timeout, timeout))
            }
            ServerEvent::VoiceMove { server, user, channel } => {
                self.same_server(server)?;
                if user == self.server.owner {
                    return Ok(false);
                }
                let Some(current) = self.voice_states.get(&user).copied() else {
                    return Ok(false);
                };
                match channel {
                    Some(target) => match self.server.channel(target) {
                        Some(c) if c.kind.is_voice() => Ok(target != current),
                        Some(_) => Ok(false),
                        None => Err(StoreError::UnknownChannel),
                    },
                    None => Ok(true),
                }
            }
            ServerEvent::MemberBan { server, user } => {
                self.same_server(server)?;
                if user == self.server.owner || self.server.banned.contains(&user) {
                    return Ok(false);
                }
                if self.server.banned.len() >= MAX_BANS {
                    return Err(StoreError::Limit);
                }
                self.server.banned.push(user);
                self.server.members.retain(|m| m.user != user);
                crate::drive::forget_user(&mut self.server.drive, user);
                self.drop_voice_presence(user);
                self.devices.forget(user);
                self.screen_shares.remove(&user);
                self.bots.remove(&user);
                self.bot_commands.remove(&user);
                Ok(true)
            }
            ServerEvent::AfkTimeoutSet { server, minutes } => {
                self.same_server(server)?;
                if minutes.is_some_and(|m| !crate::server::SERVER_AFK_MINUTES.contains(&m)) {
                    return Ok(false);
                }
                Ok(replace(&mut self.server.afk_timeout_minutes, minutes))
            }
            ServerEvent::DoorbellSet { .. } => Ok(false),
            ServerEvent::MemberUnban { server, user } => {
                self.same_server(server)?;
                let before = self.server.banned.len();
                self.server.banned.retain(|u| *u != user);
                Ok(self.server.banned.len() != before)
            }
            ServerEvent::RoleUpdate { server, role } => {
                self.same_server(server)?;
                if role.banner.as_deref().is_some_and(|b| crate::server::ImageRef::parse(b).is_none()) {
                    return Ok(false);
                }
                if role.tag.as_deref().is_some_and(|t| !crate::server::valid_role_tag(t)) || role.name.trim().is_empty() {
                    return Ok(false);
                }
                let late_defaults = self.server.late_member_defaults();
                let adopting = self.server.defaults_version < crate::server::MEMBER_DEFAULTS_VERSION;
                let Some(existing) = self.server.roles.iter_mut().find(|r| r.id == role.id) else { return Ok(false) };
                if existing.is_everyone() && (role.name != existing.name || role.self_assignable || role.mentionable) {
                    return Ok(false);
                }
                let mut role = role;
                if existing.is_everyone() {
                    role.permissions |= late_defaults;
                }
                let adopted = existing.is_everyone() && adopting;
                if *existing == role && !adopted {
                    return Ok(false);
                }
                *existing = role;
                if adopted {
                    self.server.defaults_version = crate::server::MEMBER_DEFAULTS_VERSION;
                }
                Ok(true)
            }
            ServerEvent::ChannelBanner { server, channel, banner } => {
                self.same_server(server)?;
                let banner = banner.filter(|r| crate::server::ImageRef::parse(r).is_some());
                let Some(target) = self.server.channel_mut(channel) else {
                    return Err(StoreError::UnknownChannel);
                };
                Ok(replace(&mut target.banner, banner))
            }
            ServerEvent::ChannelColor { server, channel, color } => {
                self.same_server(server)?;
                let color = color
                    .map(|c| c.to_ascii_lowercase())
                    .filter(|c| crate::server::valid_hex_color(c));
                let Some(target) = self.server.channel_mut(channel) else {
                    return Err(StoreError::UnknownChannel);
                };
                Ok(replace(&mut target.color, color))
            }
            ServerEvent::ChannelSensitiveSet { server, channel, sensitive } => {
                self.same_server(server)?;
                let Some(target) = self.server.channel_mut(channel) else {
                    return Err(StoreError::UnknownChannel);
                };
                if !target.kind.holds_messages() {
                    return Ok(false);
                }
                Ok(replace(&mut target.sensitive, sensitive))
            }
            ServerEvent::VoiceStatusSet { server, channel, text, by } => {
                self.same_server(server)?;
                let in_channel = self.voice_states.get(&by) == Some(&channel);
                if !self.server.permissions_for(by).can_set_voice_status(in_channel) {
                    return Ok(false);
                }
                let Some(target) = self.server.channel_mut(channel) else {
                    return Err(StoreError::UnknownChannel);
                };
                if target.kind != ChannelKind::Voice {
                    return Ok(false);
                }
                Ok(replace(&mut target.voice_status, crate::server::clean_voice_status(&text)))
            }
            ServerEvent::MessagePin { server, channel, message, by, at, pinned } => {
                self.same_server(server)?;
                if self.server.channel(channel).is_none() {
                    return Err(StoreError::UnknownChannel);
                }
                let held = self.server.pins.iter().position(|p| p.message == message);
                match (pinned, held) {
                    (true, Some(_)) => Ok(false),
                    (false, None) => Ok(false),
                    (false, Some(pos)) => {
                        self.server.pins.remove(pos);
                        Ok(true)
                    }
                    (true, None) => {
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
                self.same_server(server)?;
                let Some(target) = self.server.channel_mut(channel) else {
                    return Err(StoreError::UnknownChannel);
                };
                let text_like = |k: ChannelKind| matches!(k, ChannelKind::Text | ChannelKind::Announcement);
                if !text_like(kind) || !text_like(target.kind) {
                    return Ok(false);
                }
                Ok(replace(&mut target.kind, kind))
            }
            ServerEvent::ChannelAccessSet { server, channel, view, send } => {
                self.same_server(server)?;
                let known: Vec<_> = self.server.roles.iter().map(|r| r.id).collect();
                let clean = |mut roles: Vec<crate::ids::RoleId>| {
                    roles.retain(|r| known.contains(r));
                    roles.sort();
                    roles.dedup();
                    roles
                };
                let (view, send) = (clean(view), clean(send));
                if view.len() > crate::server::MAX_ACCESS_ROLES || send.len() > crate::server::MAX_ACCESS_ROLES {
                    return Ok(false);
                }
                let Some(target) = self.server.channel_mut(channel) else {
                    return Err(StoreError::UnknownChannel);
                };
                if target.kind == ChannelKind::Category {
                    return Ok(false);
                }
                let a = replace(&mut target.view_roles, view);
                let b = replace(&mut target.send_roles, send);
                if a {
                    self.drop_unseen_live();
                }
                Ok(a | b)
            }
            ServerEvent::ChannelKeyAdd { server, channel, key } => {
                self.same_server(server)?;
                if self.server.channel(channel).is_none() {
                    return Err(StoreError::UnknownChannel);
                }
                Ok(self.channel_keys.entry(channel).or_default().add_key(key))
            }
            ServerEvent::ChannelKeyGrant { server, channel, user, grant } => {
                self.same_server(server)?;
                if self.server.channel(channel).is_none() {
                    return Err(StoreError::UnknownChannel);
                }
                self.channel_keys.entry(channel).or_default().set_grant(user, grant, MAX_PROFILES).ok_or(StoreError::Limit)
            }
            ServerEvent::CommunityKeyAdd { server, key } => {
                self.same_server(server)?;
                Ok(self.community_keys.add_key(key))
            }
            ServerEvent::CommunityKeyGrant { server, user, grant } => {
                self.same_server(server)?;
                if self.server.banned.contains(&user) {
                    return Ok(false);
                }
                self.community_keys.set_grant(user, grant, MAX_PROFILES).ok_or(StoreError::Limit)
            }
            ServerEvent::PollVote { server, channel, message, user, choices } => {
                self.same_server(server)?;
                self.vote_poll(channel, message, user, choices)
            }
            ServerEvent::PollClose { server, channel, message, by } => {
                self.same_server(server)?;
                self.close_poll(channel, message, by)
            }
            ServerEvent::ServerImages { server, icon, banner } => {
                self.same_server(server)?;
                let clean = |v: Option<String>| v.filter(|r| crate::server::ImageRef::parse(r).is_some());
                let a = replace(&mut self.server.icon, clean(icon));
                let b = replace(&mut self.server.banner, clean(banner));
                Ok(a | b)
            }
            ServerEvent::ServerRename { server, name } => {
                self.same_server(server)?;
                if name.trim().is_empty() {
                    return Ok(false);
                }
                Ok(replace(&mut self.server.name, name))
            }
            ServerEvent::ProfileUpdate { mut profile } => {
                if !self.server.is_member(profile.id) {
                    return Ok(false);
                }
                self.pin_identity(&mut profile)?;
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
                self.same_server(server)?;
                if self.server.roles.iter().any(|r| r.id == role.id) {
                    return Ok(false);
                }
                if role.is_everyone() {
                    return Ok(false);
                }
                if self.server.roles.len() >= MAX_ROLES {
                    return Err(StoreError::Limit);
                }
                self.server.roles.push(role);
                Ok(true)
            }
            ServerEvent::RoleDelete { server, role } => {
                self.same_server(server)?;
                let is_everyone = self
                    .server
                    .roles
                    .iter()
                    .any(|r| r.id == role && r.is_everyone());
                if is_everyone {
                    return Ok(false);
                }
                let before = self.server.roles.len();
                self.server.roles.retain(|r| r.id != role);
                for member in &mut self.server.members {
                    member.roles.retain(|r| *r != role);
                }
                self.drop_unseen_live();
                Ok(self.server.roles.len() != before)
            }
            ServerEvent::RoleAssign { server, user, role } => {
                self.same_server(server)?;
                if !self.server.roles.iter().any(|r| r.id == role) {
                    return Ok(false);
                }
                let Some(member) = self.server.member_mut(user) else {
                    return Ok(false);
                };
                if member.roles.contains(&role) {
                    return Ok(false);
                }
                member.roles.push(role);
                Ok(true)
            }
            ServerEvent::RoleUnassign { server, user, role } => {
                self.same_server(server)?;
                let Some(member) = self.server.member_mut(user) else {
                    return Ok(false);
                };
                let before = member.roles.len();
                member.roles.retain(|r| *r != role);
                let changed = member.roles.len() != before;
                if changed {
                    self.drop_unseen_live();
                }
                Ok(changed)
            }
            ServerEvent::TypingStart { .. } => Ok(true),
        }
    }

    fn drop_unseen_live(&mut self) {
        let server = &self.server;
        self.screen_shares.retain(|u, c| server.can_view(*c, *u));
        self.cameras.retain(|u, c| server.can_view(*c, *u));
        self.hands.retain(|u, c| server.can_view(*c, *u));
        let gone: Vec<UserId> =
            self.voice_states.iter().filter(|(u, c)| !server.can_view(**c, **u)).map(|(u, _)| *u).collect();
        for user in gone {
            self.drop_voice_presence(user);
        }
    }

    pub fn slow_mode_wait(&self, channel: ChannelId, author: UserId, at: DateTime<Utc>) -> Option<i64> {
        let seconds = self.server.channel(channel).map(|c| c.slow_mode).unwrap_or(0);
        if seconds == 0 || self.server.permissions_for(author).bypasses_slow_mode() {
            return None;
        }
        let last = self.messages(channel).iter().filter(|m| m.author == author && m.created_at < at).map(|m| m.created_at).max()?;
        let wait = i64::from(seconds) - (at - last).num_seconds();
        (wait > 0).then_some(wait)
    }

    fn slow_mode_blocks(&self, message: &Message) -> bool {
        self.slow_mode_wait(message.channel, message.author, message.created_at).is_some()
    }

    pub fn thread_root_of(&self, channel: ChannelId, id: MessageId) -> MessageId {
        let list = self.messages(channel);
        let mut current = id;
        for _ in 0..MAX_THREAD_DEPTH {
            let Some(parent) = list.iter().rev().find(|m| m.id == current).and_then(|m| m.reply_to) else { break };
            if !list.iter().any(|m| m.id == parent) {
                break;
            }
            current = parent;
        }
        current
    }

    pub fn thread_of(&self, channel: ChannelId, root: MessageId) -> Option<&crate::server::Thread> {
        self.server.channel(channel)?.threads.get(&root)
    }

    fn thread_locked_for(&self, message: &Message, root: MessageId) -> bool {
        self.thread_of(message.channel, root).is_some_and(|t| t.locked)
            && !self.server.permissions_for(message.author).allows(crate::permissions::Permissions::MANAGE_MESSAGES)
    }

    fn reopen_thread(&mut self, channel: ChannelId, root: MessageId) {
        let Some(target) = self.server.channel_mut(channel) else { return };
        let Some(thread) = target.threads.get_mut(&root) else { return };
        if thread.archived {
            thread.archived = false;
            if thread.is_plain() {
                target.threads.remove(&root);
            }
        }
    }

    fn create_message(&mut self, mut message: Message) -> Result<bool, StoreError> {
        if !message.id.is_bound_to(message.author) {
            return Err(StoreError::UnboundMessageId);
        }
        if message.authored_size() > MAX_MESSAGE_BYTES {
            return Err(StoreError::TooLarge);
        }
        let key = self.key_of(message.author)?;
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
        let held = self.has_message(message.id);
        if !held && self.server.timed_out_at(message.author, message.created_at) {
            return Err(StoreError::TimedOut);
        }
        if !held && !self.server.can_post_in(message.channel, message.author) {
            match self.server.channel(message.channel) {
                Some(c) if c.is_restricted() => return Err(StoreError::NotAllowed),
                Some(c) if c.kind == ChannelKind::Announcement => return Err(StoreError::NotAnnouncer),
                _ => {}
            }
        }
        let thread_root = message.reply_to.filter(|_| !held).map(|parent| self.thread_root_of(message.channel, parent));
        if thread_root.is_some_and(|root| self.thread_locked_for(&message, root)) {
            return Err(StoreError::ThreadLocked);
        }
        if !held
            && message.attachments.iter().any(Attachment::is_plain_file)
            && !self.server.permissions_for(message.author).allows(crate::permissions::Permissions::ATTACH_FILES)
        {
            return Err(StoreError::NoUploads);
        }
        if !held
            && message.attachments.iter().any(Attachment::is_voice_message)
            && !self.server.permissions_for(message.author).allows(crate::permissions::Permissions::SEND_VOICE_MESSAGES)
        {
            return Err(StoreError::NoVoiceMessages);
        }
        if !held
            && message.attachments.iter().any(Attachment::is_poll)
            && !self.server.permissions_for(message.author).can_send_polls()
        {
            return Err(StoreError::NoPolls);
        }
        if !held {
            if let Some(changed) = self.bind_proofs(&message) {
                return Ok(changed);
            }
        }
        if !held && self.slow_mode_blocks(&message) {
            return Ok(false);
        }
        if self.deleted_messages.contains(&message.id) {
            return Ok(false);
        }
        if self.retention_cutoff(message.channel, now).is_some_and(|cutoff| message.created_at < cutoff) {
            return Ok(false);
        }
        if self.rolled_off(&message) {
            return Ok(false);
        }
        let server = self.server.id;
        if let Some(existing) = self.message_mut(message.id) {
            let Some(sig) = &message.sig else { return Ok(false) };
            if !supersedes(existing, message.edited_at, sig) {
                return Ok(false);
            }
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
        if let Some(list) = self.tombstones.get_mut(&message.id) {
            list.retain(|s| s.proof.channel == message.channel);
            if list.is_empty() {
                self.tombstones.remove(&message.id);
            }
            self.unapplied = None;
        }
        let list = self
            .messages
            .get_mut(&message.channel)
            .ok_or(StoreError::UnknownChannel)?;
        message.reactions = relayed_reactions(std::mem::take(&mut message.reactions));
        self.index.insert(message.id, message.channel);
        let channel = message.channel;
        let pos = list.partition_point(|m| m.created_at <= message.created_at);
        list.insert(pos, message);
        if let Some(root) = thread_root {
            self.reopen_thread(channel, root);
        }
        Ok(true)
    }

    fn may_delete(&self, proof: &TombstoneProof, author: UserId, written: DateTime<Utc>) -> bool {
        let by = proof.by;
        let at = proof.at.timestamp_millis();
        let was_moderator = self.server.stepped_down.get(&by).is_some_and(|until| at < *until && at + STEPPED_DOWN_SKEW_MS >= written.timestamp_millis());
        by == author || self.server.permissions_for(by).allows(crate::permissions::Permissions::MANAGE_MESSAGES) || was_moderator
    }

    fn bind_proofs(&mut self, message: &Message) -> Option<bool> {
        let author = message.author;
        let list = self.tombstones.get(&message.id)?;
        let allowed: Vec<bool> = list.iter().map(|s| self.may_delete(&s.proof, author, message.created_at)).collect();
        let list = self.tombstones.get_mut(&message.id)?;
        let before = list.len();
        let (mut blocked, mut bound) = (false, false);
        for (s, allowed) in list.iter_mut().zip(allowed) {
            if s.proof.channel == message.channel && s.accepted_for.map_or(allowed, |a| a == author) {
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
        let key = self.key_of(author)?;
        let msg = self.find_message_mut(channel, id)?;
        let edited = Message { content, edited_at: Some(edited_at), sig: Some(sig), ..msg.without_reactions() };
        if edited.authored_size() > MAX_MESSAGE_BYTES {
            return Err(StoreError::TooLarge);
        }
        if !msg.verify_edit(server, &edited.content, edited_at, &sig, &key) {
            return Err(StoreError::BadSignature);
        }
        if too_far_ahead(edited_at, Utc::now()) {
            return Err(StoreError::TooFarAhead);
        }
        if self.server.timed_out_at(author, edited_at) {
            return Err(StoreError::TimedOut);
        }
        let restricted = self.server.channel(channel).is_some_and(|c| c.is_restricted());
        if restricted && !self.server.can_post_in(channel, author) {
            return Err(StoreError::NotAllowed);
        }
        let msg = self.find_message_mut(channel, id)?;
        if !supersedes(msg, Some(edited_at), &sig) {
            return Ok(false);
        }
        msg.content = edited.content;
        msg.edited_at = edited.edited_at;
        msg.sig = edited.sig;
        Ok(true)
    }

    fn delete_message(&mut self, proof: TombstoneProof) -> Result<bool, StoreError> {
        let key = self.key_of(proof.by)?;
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
            let pos = list.iter().rposition(|m| m.id == proof.message)?;
            Some((*channel, pos, list[pos].author, list[pos].created_at))
        });
        if let Some((channel, pos, author, written)) = held {
            if channel != proof.channel {
                return Err(StoreError::WrongChannel);
            }

            if !self.may_delete(&proof, author, written) {
                let kept = self.holds_proof_from(&proof);
                return self.store_unapplied(proof, kept).map(|_| false);
            }
            if author != proof.by {
                self.note_moderator_delete(&proof, author);
            }
            if let Some(list) = self.messages.get_mut(&channel) {
                list.remove(pos);
            }
            self.index.remove(&proof.message);
            self.server.pins.retain(|p| p.message != proof.message);
            self.ballots.remove(&proof.message);
            self.closed_polls.remove(&proof.message);
            if let Some(target) = self.server.channel_mut(channel) {
                target.threads.remove(&proof.message);
                target.posts.remove(&proof.message);
            }
            let id = proof.message;
            self.store_proof(proof, Some(author));
            if let Some(list) = self.tombstones.get_mut(&id) {
                list.retain(|s| s.accepted_for.is_some());
            }
            self.unapplied = None;
            return Ok(true);
        }
        let kept = self.holds_proof_from(&proof);
        self.store_unapplied(proof, kept)
    }

    fn holds_proof_from(&self, proof: &TombstoneProof) -> bool {
        self.tombstones
            .get(&proof.message)
            .is_some_and(|list| list.iter().any(|s| s.proof.by == proof.by && s.proof.channel == proof.channel))
    }

    fn retry_held_proofs(&mut self) {
        let waiting: Vec<TombstoneProof> = self
            .tombstones
            .iter()
            .filter(|(id, _)| self.index.contains_key(id))
            .flat_map(|(_, list)| list.iter().filter(|s| s.accepted_for.is_none()).map(|s| s.proof.clone()))
            .collect();
        for proof in waiting {
            let _ = self.delete_message(proof);
        }
    }

    fn note_moderator_delete(&mut self, proof: &TombstoneProof, author: UserId) {
        let entry = AuditEntry {
            at: proof.at.timestamp_millis(),
            by: proof.by,
            kind: AuditKind::MessageDelete,
            target: Some(author.to_string()),
            label: crate::audit::member_name(self, author),
            detail: self.server.channel(proof.channel).map(|c| c.name.clone()),
            sig: proof.sig,
        };
        crate::audit::record(&mut self.audit, entry);
    }

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

    fn put_proof(&mut self, proof: TombstoneProof, accepted_for: Option<UserId>) -> bool {
        let mut new = StoredProof { proof, accepted_for };
        let (by, channel) = (new.proof.by, new.proof.channel);
        let list = self.tombstones.entry(new.proof.message).or_default();
        let Some(old) = list.iter_mut().find(|s| s.proof.by == by && s.proof.channel == channel) else {
            list.push(new);
            return true;
        };
        new.accepted_for = new.accepted_for.or(old.accepted_for);
        let changed = old.accepted_for != new.accepted_for;
        old.accepted_for = new.accepted_for;
        if proof_rank(&new) < proof_rank(old) {
            *old = new;
            return true;
        }
        changed
    }

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
        if !self.server.is_current_member(user) {
            return false;
        }
        self.is_device_of(user, node_hex)
            && self.server.can_view(channel, user)
            && (!voice || self.voice_states.get(&user) == Some(&channel))
    }

    pub fn is_device_of(&self, user: UserId, node_hex: &str) -> bool {
        let announced = self.profiles.get(&user).and_then(|p| p.node.as_deref());
        announced.is_some_and(|node| node.eq_ignore_ascii_case(node_hex)) || self.devices.has(user, node_hex)
    }

    pub fn devices_of(&self, user: UserId) -> &[Device] {
        self.devices.of(user)
    }

    pub fn voice_join_stamp(&self, user: UserId, now_ms: i64) -> i64 {
        self.devices.join_stamp(user, now_ms)
    }

    pub fn voice_device(&self, user: UserId) -> Option<&str> {
        self.voice_states.get(&user).and_then(|room| self.devices.in_voice(user, *room))
    }

    pub fn voice_node(&self, user: UserId) -> Option<String> {
        self.voice_device(user).map(str::to_string).or_else(|| self.profiles.get(&user).and_then(|p| p.node.clone()))
    }

    fn note_device(&mut self, user: UserId, node: String, voice: Option<ChannelId>, since: Option<i64>) -> Result<bool, StoreError> {
        if !self.server.is_current_member(user) {
            return Ok(false);
        }
        let Some(node) = devices::node_key(&node) else { return Ok(false) };
        if let Some(room) = voice {
            match self.server.channel(room) {
                Some(c) if c.kind.is_voice() => {}
                Some(_) => return Ok(false),
                None => return Err(StoreError::UnknownChannel),
            }
        }
        if self.devices.of(user).is_empty() && self.devices.users() >= MAX_LIVE_ENTRIES {
            return Err(StoreError::Limit);
        }
        let held_by = self.devices.voice_holder(user).map(|d| d.node.clone());
        let noted = self.devices.note(user, node, voice, since);
        Ok(self.follow_voice_device(user, held_by)? || noted)
    }

    fn follow_voice_device(&mut self, user: UserId, held_by: Option<String>) -> Result<bool, StoreError> {
        let holder = self.devices.voice_holder(user).map(|d| (d.node.clone(), d.voice, d.since));
        match holder {
            Some((node, Some(room), since)) => {
                if held_by.as_deref() != Some(node.as_str()) {
                    self.action_times.remove(&format!("voice:{user}"));
                }
                self.seat(user, room, since.map(|ms| ms.div_euclid(1000)))
            }
            _ if held_by.is_some() => Ok(self.end_seat(user)),
            _ => Ok(false),
        }
    }

    fn voice_claim_fits(&self, user: UserId, channel: Option<ChannelId>) -> bool {
        self.devices.voice_holder(user).is_none_or(|held| held.voice == channel)
    }

    fn seat(&mut self, user: UserId, room: ChannelId, since: Option<i64>) -> Result<bool, StoreError> {
        live_room(&self.voice_states, user)?;
        let previous = self.voice_states.insert(user, room);
        let moved = previous != Some(room);
        if moved {
            self.hands.remove(&user);
            self.activities.retain(|at, a| a.starter != user || *at == room);
        }
        if moved || !self.voice_since.contains_key(&user) {
            let now = Utc::now().timestamp();
            self.voice_since.insert(user, since.filter(|s| *s <= now).unwrap_or(now));
        }
        if let Some(left) = previous.filter(|_| moved) {
            self.clear_empty_voice_status(left);
        }
        Ok(moved)
    }

    fn end_seat(&mut self, user: UserId) -> bool {
        self.screen_shares.remove(&user);
        self.cameras.remove(&user);
        self.drop_voice_presence(user)
    }

    pub fn carry_live_voice(&mut self, old: &ServerState) {
        self.devices = old.devices.clone();
        let carried: Vec<(UserId, ChannelId)> = old
            .voice_states
            .iter()
            .filter(|(user, room)| self.server.is_member(**user) && self.server.channel(**room).is_some())
            .map(|(user, room)| (*user, *room))
            .collect();
        for (user, room) in carried {
            self.voice_states.insert(user, room);
            carry(&old.voice_since, &mut self.voice_since, user);
            carry(&old.voice_flags, &mut self.voice_flags, user);
            carry(&old.screen_shares, &mut self.screen_shares, user);
            carry(&old.cameras, &mut self.cameras, user);
        }
    }

    pub fn end_voice(&mut self, user: UserId) -> bool {
        let stood = self.devices.stand_up(user);
        self.end_seat(user) || stood
    }

    fn pin_identity(&self, profile: &mut UserProfile) -> Result<(), StoreError> {
        if profile.pin_identity(self.identity_of(profile.id)) {
            Ok(())
        } else {
            Err(StoreError::IdentityMismatch)
        }
    }

    fn message_mut(&mut self, id: MessageId) -> Option<&mut Message> {
        let channel = self.index.get(&id)?;
        self.messages.get_mut(channel)?.iter_mut().rev().find(|m| m.id == id)
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
            .rev()
            .find(|m| m.id == id)
            .ok_or(StoreError::UnknownMessage)
    }

    pub fn voice_flags(&self, user: UserId) -> (bool, bool) {
        self.voice_flags.get(&user).copied().unwrap_or((false, false))
    }

    pub fn voice_since(&self, user: UserId) -> Option<i64> {
        self.voice_since.get(&user).copied()
    }

    pub fn hands_in(&self, channel: ChannelId) -> Vec<UserId> {
        let mut hands: Vec<UserId> = self.hands.iter().filter(|(_, ch)| **ch == channel).map(|(u, _)| *u).collect();
        hands.sort();
        hands
    }

    pub fn hand_of(&self, user: UserId) -> Option<ChannelId> {
        self.hands.get(&user).copied()
    }

    pub fn voice_members(&self, channel: ChannelId) -> Vec<UserId> {
        self.voice_states
            .iter()
            .filter(|(_, ch)| **ch == channel)
            .map(|(u, _)| *u)
            .collect()
    }

    pub fn voice_status(&self, channel: ChannelId) -> Option<&str> {
        self.server.channel(channel).and_then(|c| c.voice_status.as_deref())
    }

    pub fn priority_speakers(&self, channel: ChannelId) -> Vec<UserId> {
        self.voice_members(channel)
            .into_iter()
            .filter(|u| self.server.permissions_for(*u).is_priority_speaker())
            .collect()
    }

    fn drop_voice_presence(&mut self, user: UserId) -> bool {
        self.voice_since.remove(&user);
        self.voice_flags.remove(&user);
        self.hands.remove(&user);
        self.activities.retain(|_, a| a.starter != user);
        let left = self.voice_states.remove(&user);
        if let Some(channel) = left {
            self.clear_empty_voice_status(channel);
        }
        left.is_some()
    }

    fn clear_empty_voice_status(&mut self, channel: ChannelId) -> bool {
        if self.voice_states.values().any(|c| *c == channel) {
            return false;
        }
        self.server.channel_mut(channel).is_some_and(|c| c.voice_status.take().is_some())
    }

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
    now.checked_sub_signed(chrono::TimeDelta::try_days(days.into())?)
}

fn valid(stored: &StoredProof, held: &HashSet<MessageId>) -> bool {
    stored.accepted_for.is_some() || !held.contains(&stored.proof.message)
}

fn proof_rank(stored: &StoredProof) -> (bool, DateTime<Utc>, [u8; 64]) {
    (stored.accepted_for.is_none(), stored.proof.at, stored.proof.sig.0)
}

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

fn replace<T: PartialEq>(slot: &mut T, value: T) -> bool {
    if *slot == value {
        return false;
    }
    *slot = value;
    true
}

fn carry<T: Copy>(from: &HashMap<UserId, T>, to: &mut HashMap<UserId, T>, user: UserId) {
    if let Some(value) = from.get(&user) {
        to.insert(user, *value);
    }
}

fn live_room(map: &HashMap<UserId, ChannelId>, user: UserId) -> Result<(), StoreError> {
    if !map.contains_key(&user) && map.len() >= MAX_LIVE_ENTRIES {
        return Err(StoreError::Limit);
    }
    Ok(())
}

fn set_live(
    map: &mut HashMap<UserId, ChannelId>,
    known: bool,
    user: UserId,
    channel: ChannelId,
    active: bool,
) -> Result<bool, StoreError> {
    if !active {
        return Ok(map.remove(&user).is_some());
    }
    if !known {
        return Err(StoreError::UnknownChannel);
    }
    live_room(map, user)?;
    Ok(map.insert(user, channel) != Some(channel))
}

fn upsert_by<T: PartialEq>(list: &mut Vec<T>, value: T, cap: usize, same: impl Fn(&T, &T) -> bool) -> bool {
    let count = list.len();
    match list.iter_mut().find(|e| same(e, &value)) {
        Some(existing) if *existing == value => false,
        Some(existing) => {
            *existing = value;
            true
        }
        None if count >= cap => false,
        None => {
            list.push(value);
            true
        }
    }
}

fn push_capped<T>(list: &mut Vec<T>, value: T, cap: usize) {
    list.push(value);
    if list.len() > cap {
        list.remove(0);
    }
}

fn waits_for(event: &ServerEvent) -> Option<MessageId> {
    use ServerEvent as E;
    match event {
        E::MessageEdit { message, .. }
        | E::ReactionToggle { message, .. }
        | E::ReactionSet { message, .. }
        | E::PollVote { message, .. }
        | E::PollClose { message, .. } => Some(*message),
        _ => None,
    }
}

fn too_far_ahead(t: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    t.timestamp_millis().saturating_sub(now.timestamp_millis()) > MAX_AHEAD_MS
}

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

fn unapplied_of(list: Option<&Vec<StoredProof>>) -> Vec<(UserId, UnappliedKey)> {
    let unapplied = list.into_iter().flatten().filter(|s| s.accepted_for.is_none());
    unapplied.map(|s| (s.proof.by, unapplied_key(&s.proof))).collect()
}

fn millis(t: Option<DateTime<Utc>>) -> Option<i64> {
    t.map(|t| t.timestamp_millis())
}

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
        delete, edit, fixture, identity_for, join, join_moderator, keyed_profile, post, post_at, proof, signed, signed_event,
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
            state.apply(ServerEvent::MemberJoin { server: sid, profile: profile.clone() }),
            Ok(false)
        );
        state.apply(ServerEvent::PresenceUpdate { user: profile.id, presence: Presence::Offline }).unwrap();
        assert_eq!(
            state.apply(ServerEvent::MemberJoin { server: sid, profile }),
            Ok(true),
            "a member back online is a change"
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
        state.apply(join(Some(voice), Some(now - 90))).unwrap();
        assert_eq!(state.voice_since(owner), Some(now - 90));
        state.apply(join(Some(voice), Some(now - 5))).unwrap();
        assert_eq!(state.voice_since(owner), Some(now - 90));
        let lobby = crate::server::Channel::new("Lobby", ChannelKind::Voice, 9);
        let other = lobby.id;
        state.apply(ServerEvent::ChannelCreate { server: state.server.id, channel: lobby }).unwrap();
        state.apply(join(Some(other), Some(now - 5))).unwrap();
        assert_eq!(state.voice_since(owner), Some(now - 5));
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
        state
            .apply(ServerEvent::VoiceState {
                user: owner,
                channel: None,
                muted: false,
                deafened: false, since: None
            })
            .unwrap();
        assert!(state.screen_shares.is_empty());
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
        assert_eq!(restored.profiles.len(), 2);
        assert_eq!(restored.identity_of(owner), state.identity_of(owner));
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
            mentionable: false,
            tag: None,
            banner: None,
        };
        assert_eq!(
            state.apply(ServerEvent::RoleCreate { server: sid, role: role.clone() }),
            Ok(true)
        );
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
            mentionable: false,
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
            mentionable: false,
            tag: None,
            banner: None,
        };
        let rid = role.id;
        state.apply(ServerEvent::RoleCreate { server: sid, role }).unwrap();
        assert_eq!(
            state.apply(ServerEvent::RoleAssign { server: sid, user: UserId::new(), role: rid }),
            Ok(false)
        );
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
        assert_eq!(state.apply(set(Some("#9b7bff"))), Ok(true));
        assert_eq!(state.server.channel(channel).unwrap().color, None);
        assert_eq!(state.apply(set(None)), Ok(false));
        assert_eq!(
            state.apply(ServerEvent::ChannelColor { server: sid, channel: ChannelId::new(), color: None }),
            Err(StoreError::UnknownChannel)
        );
    }

    #[test]
    fn channel_sensitive_sets_and_clears_on_text_channels_only() {
        let (mut state, channel, _) = fixture();
        let sid = state.server.id;
        let set = |channel, sensitive| ServerEvent::ChannelSensitiveSet { server: sid, channel, sensitive };
        assert_eq!(state.apply(set(channel, true)), Ok(true));
        assert!(state.server.channel(channel).unwrap().sensitive);
        assert_eq!(state.apply(set(channel, true)), Ok(false));
        assert_eq!(state.apply(set(channel, false)), Ok(true));
        assert!(!state.server.channel(channel).unwrap().sensitive);
        let voice = Channel::new("Voice", ChannelKind::Voice, 9);
        let voice_id = voice.id;
        state.apply(ServerEvent::ChannelCreate { server: sid, channel: voice }).unwrap();
        assert_eq!(state.apply(set(voice_id, true)), Ok(false));
        assert!(!state.server.channel(voice_id).unwrap().sensitive);
        assert_eq!(state.apply(set(ChannelId::new(), true)), Err(StoreError::UnknownChannel));
    }

    fn enter_voice(state: &mut ServerState, user: UserId, channel: ChannelId) {
        let event = ServerEvent::VoiceState { user, channel: Some(channel), muted: false, deafened: false, since: None };
        state.apply(event).unwrap();
    }

    fn leave_voice(state: &mut ServerState, user: UserId) {
        let event = ServerEvent::VoiceState { user, channel: None, muted: false, deafened: false, since: None };
        state.apply(event).unwrap();
    }

    #[test]
    fn voice_status_is_set_by_people_in_the_room_and_cleaned_on_the_way_in() {
        let (mut state, text, owner) = fixture();
        let sid = state.server.id;
        let voice = state.server.channels[1].id;
        let member = join(&mut state, "member");
        let outsider = join(&mut state, "outsider");
        let set = |channel, by, t: &str| ServerEvent::VoiceStatusSet { server: sid, channel, text: t.into(), by };
        assert_eq!(state.apply(set(voice, member, "Ranked grind")), Ok(false), "not in the channel yet");
        enter_voice(&mut state, member, voice);
        assert_eq!(state.apply(set(voice, member, "  Ranked \u{7}  grind \n")), Ok(true));
        assert_eq!(state.voice_status(voice), Some("Ranked grind"));
        assert_eq!(state.apply(set(voice, member, "Ranked grind")), Ok(false));
        assert_eq!(state.apply(set(voice, outsider, "Hijack")), Ok(false), "outsiders without manage channels");
        assert_eq!(state.voice_status(voice), Some("Ranked grind"));
        assert_eq!(state.apply(set(voice, owner, "Owner says")), Ok(true), "managers set it from anywhere");
        let long = "x".repeat(200);
        assert_eq!(state.apply(set(voice, owner, &long)), Ok(true));
        assert_eq!(state.voice_status(voice).map(|s| s.chars().count()), Some(crate::server::MAX_VOICE_STATUS_CHARS));
        assert_eq!(state.apply(set(voice, owner, "   ")), Ok(true), "blank clears");
        assert_eq!(state.voice_status(voice), None);
        assert_eq!(state.apply(set(voice, owner, "")), Ok(false));
        assert_eq!(state.apply(set(text, owner, "Not a voice channel")), Ok(false));
        assert_eq!(state.apply(set(ChannelId::new(), owner, "x")), Err(StoreError::UnknownChannel));
    }

    #[test]
    fn voice_status_clears_when_the_room_empties() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let voice = state.server.channels[1].id;
        let a = join(&mut state, "a");
        let b = join(&mut state, "b");
        enter_voice(&mut state, a, voice);
        enter_voice(&mut state, b, voice);
        let set = |t: &str| ServerEvent::VoiceStatusSet { server: sid, channel: voice, text: t.into(), by: a };
        assert_eq!(state.apply(set("Ranked grind")), Ok(true));
        leave_voice(&mut state, a);
        assert_eq!(state.voice_status(voice), Some("Ranked grind"), "someone is still there");
        let lobby = Channel::new("Lobby", ChannelKind::Voice, 9);
        let lobby_id = lobby.id;
        state.apply(ServerEvent::ChannelCreate { server: sid, channel: lobby }).unwrap();
        enter_voice(&mut state, b, lobby_id);
        assert_eq!(state.voice_status(voice), None, "the last person moved away");
        enter_voice(&mut state, a, voice);
        assert_eq!(state.apply(set("Round two")), Ok(true));
        state.apply(ServerEvent::MemberLeave { server: sid, user: a }).unwrap();
        assert_eq!(state.voice_status(voice), None, "leaving the server empties the room too");
        enter_voice(&mut state, b, voice);
        assert_eq!(state.apply(ServerEvent::VoiceStatusSet { server: sid, channel: voice, text: "Three".into(), by: b }), Ok(true));
        state.apply(ServerEvent::MemberBan { server: sid, user: b }).unwrap();
        assert_eq!(state.voice_status(voice), None);
        assert_eq!(state.apply(ServerEvent::VoiceStatusSet { server: sid, channel: voice, text: "Empty".into(), by: owner }), Ok(true));
        assert_eq!(state.voice_status(voice), Some("Empty"), "a manager may label an empty room until someone leaves it");
    }

    #[test]
    fn priority_speakers_are_the_voice_members_holding_the_permission() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let voice = state.server.channels[1].id;
        let a = join(&mut state, "a");
        let b = join(&mut state, "b");
        let role = Role {
            id: RoleId::new(),
            name: "host".into(),
            color: None,
            permissions: Permissions::PRIORITY_SPEAKER,
            position: 1,
            hoist: false,
            self_assignable: false,
            mentionable: false,
            tag: None,
            banner: None,
        };
        let rid = role.id;
        state.apply(ServerEvent::RoleCreate { server: sid, role }).unwrap();
        state.apply(ServerEvent::RoleAssign { server: sid, user: a, role: rid }).unwrap();
        assert!(state.priority_speakers(voice).is_empty());
        enter_voice(&mut state, a, voice);
        enter_voice(&mut state, b, voice);
        assert_eq!(state.priority_speakers(voice), vec![a]);
        enter_voice(&mut state, owner, voice);
        let mut speakers = state.priority_speakers(voice);
        speakers.sort();
        let mut expected = vec![a, owner];
        expected.sort();
        assert_eq!(speakers, expected, "the owner holds every permission");
        leave_voice(&mut state, a);
        assert_eq!(state.priority_speakers(voice), vec![owner]);
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
        let later = MessageId::for_author(owner);
        assert_eq!(state.apply(pin_event(&state, channel, later, owner, true)), Ok(true));
        for _ in 1..crate::server::MAX_PINS_PER_CHANNEL {
            assert_eq!(state.apply(pin_event(&state, channel, MessageId::for_author(owner), owner, true)), Ok(true));
        }
        assert_eq!(state.apply(pin_event(&state, channel, msg.id, owner, true)), Err(StoreError::PinLimit));
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
            mentionable: false,
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
        state.apply(ServerEvent::RoleUnassign { server: sid, user: plain, role: rid }).unwrap();
        let later = Utc::now() + chrono::Duration::seconds(1);
        assert_eq!(state.apply(edit(&state, &allowed, "Patch day, 6pm", later)), Ok(true));
    }

    #[test]
    fn stickers_and_emoticons_add_replace_remove_cap_and_share() {
        use crate::server::{Emoticon, Sticker, MAX_EMOTICONS, MAX_STICKERS};
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let sticker = |name: &str, blob: &str| ServerEvent::StickerAdd { server: sid, sticker: Sticker { name: name.into(), blob: blob.into() } };
        let emoticon = |name: &str, text: &str| ServerEvent::EmoticonAdd { server: sid, emoticon: Emoticon { name: name.into(), text: text.into() } };
        assert_eq!(state.apply(sticker("wave", "aa@bb")), Ok(true));
        assert_eq!(state.apply(sticker("wave", "aa@bb")), Ok(false));
        assert_eq!(state.apply(sticker("wave", "cc@bb")), Ok(true));
        assert_eq!(state.apply(sticker("Bad Name", "aa@bb")), Ok(false));
        assert_eq!(state.apply(sticker("bad_ref", "nope")), Ok(false));
        for i in 0..MAX_STICKERS + 3 {
            state.apply(sticker(&format!("s{i}"), "aa@bb")).unwrap();
        }
        assert_eq!(state.server.stickers.len(), MAX_STICKERS);
        assert_eq!(state.apply(ServerEvent::StickerRemove { server: sid, name: "wave".into() }), Ok(true));
        assert_eq!(state.apply(ServerEvent::StickerRemove { server: sid, name: "wave".into() }), Ok(false));

        assert_eq!(state.apply(emoticon("shrug", "¯\\_(ツ)_/¯")), Ok(true));
        assert_eq!(state.apply(emoticon("flip", " padded ")), Ok(false));
        assert_eq!(state.apply(emoticon("flip", "two\nlines")), Ok(false));
        assert_eq!(state.apply(emoticon("flip", "")), Ok(false));
        for i in 0..MAX_EMOTICONS + 3 {
            state.apply(emoticon(&format!("e{i}"), ":)")).unwrap();
        }
        assert_eq!(state.server.emoticons.len(), MAX_EMOTICONS);
        assert_eq!(state.apply(ServerEvent::EmoticonRemove { server: sid, name: "shrug".into() }), Ok(true));

        let share = ServerEvent::ExpressionSharingSet { server: sid, stickers: true, emoticons: false };
        assert_eq!(state.apply(share.clone()), Ok(true));
        assert_eq!(state.apply(share), Ok(false));
        assert!(state.server.stickers_shareable && !state.server.emoticons_shareable);
    }

    #[test]
    fn the_first_everyone_edit_on_an_older_server_adopts_the_newer_member_defaults() {
        let (mut state, _channel, _owner) = fixture();
        let sid = state.server.id;
        let plain = join(&mut state, "plain");
        state.server.defaults_version = 0;
        state.server.roles[0].permissions.remove(Permissions::CHANGE_NICKNAME);
        assert!(state.server.permissions_for(plain).can_change_own_nickname());
        let mut everyone = state.server.roles[0].clone();
        everyone.permissions.remove(Permissions::ATTACH_FILES);
        assert_eq!(state.apply(ServerEvent::RoleUpdate { server: sid, role: everyone.clone() }), Ok(true));
        assert_eq!(state.server.defaults_version, crate::server::MEMBER_DEFAULTS_VERSION);
        let perms = state.server.permissions_for(plain);
        assert!(perms.can_change_own_nickname() && !perms.allows(Permissions::ATTACH_FILES));
        assert!(state.server.roles[0].permissions.contains(Permissions::CHANGE_NICKNAME), "the defaults are now stored");
        everyone.permissions = state.server.roles[0].permissions;
        everyone.permissions.remove(Permissions::CHANGE_NICKNAME);
        assert_eq!(state.apply(ServerEvent::RoleUpdate { server: sid, role: everyone }), Ok(true));
        assert!(!state.server.permissions_for(plain).can_change_own_nickname(), "later edits are taken as written");
    }

    #[test]
    fn a_link_card_without_a_picture_needs_no_send_files_permission() {
        let (mut state, channel, _owner) = fixture();
        let sid = state.server.id;
        let plain = join(&mut state, "plain");
        let mut everyone = state.server.roles[0].clone();
        everyone.permissions.remove(Permissions::ATTACH_FILES);
        state.apply(ServerEvent::RoleUpdate { server: sid, role: everyone }).unwrap();
        let card = |picture: Option<crate::link_preview::PreviewPicture>| {
            let preview = crate::link_preview::LinkPreview {
                url: "https://example.org/a".into(),
                site: String::new(),
                title: "A page".into(),
                description: String::new(),
                image: Some(crate::link_preview::PreviewImage { mime: "image/jpeg".into(), width: 10, height: 10 }),
            };
            let mut m = Message::new(channel, plain, "see https://example.org/a");
            m.attachments.push(preview.into_attachment(picture));
            crate::test_support::signed(&state, m)
        };
        let text_only = card(None);
        let pictured = card(Some(crate::link_preview::PreviewPicture { blob: "abc".into(), size_bytes: 10, provider: None }));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: text_only }), Ok(true));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: pictured }), Err(StoreError::NoUploads));
    }

    #[test]
    fn attachments_need_the_send_files_permission() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let plain = join(&mut state, "plain");
        let with_file = |state: &ServerState, author| {
            let mut m = Message::new(channel, author, "look");
            m.attachments.push(crate::message::Attachment {
                blob: "abc".into(),
                filename: "cat.png".into(),
                size_bytes: 10,
                mime_type: "image/png".into(),
                provider: None,
            });
            crate::test_support::signed(state, m)
        };
        let first = with_file(&state, plain);
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: first.clone() }), Ok(true));
        let mut everyone = state.server.roles[0].clone();
        everyone.permissions.remove(Permissions::ATTACH_FILES);
        state.apply(ServerEvent::RoleUpdate { server: sid, role: everyone }).unwrap();
        let blocked = with_file(&state, plain);
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: blocked }), Err(StoreError::NoUploads));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: first }), Ok(false), "held messages stay");
        let text = post(&state, channel, plain, "words are fine");
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: text }), Ok(true));
        let owners = with_file(&state, owner);
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: owners }), Ok(true));
        let role = Role {
            id: RoleId::new(),
            name: "artists".into(),
            color: None,
            permissions: Permissions::ATTACH_FILES,
            position: 1,
            hoist: false,
            self_assignable: false,
            mentionable: false,
            tag: None,
            banner: None,
        };
        let rid = role.id;
        state.apply(ServerEvent::RoleCreate { server: sid, role }).unwrap();
        state.apply(ServerEvent::RoleAssign { server: sid, user: plain, role: rid }).unwrap();
        let allowed = with_file(&state, plain);
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: allowed }), Ok(true));
    }

    #[test]
    fn voice_messages_need_their_own_permission_and_not_the_files_one() {
        let (mut state, channel, _owner) = fixture();
        let sid = state.server.id;
        let plain = join(&mut state, "plain");
        let with_voice = |state: &ServerState, author| {
            let mut m = Message::new(channel, author, "");
            m.attachments.push(crate::message::Attachment::voice_message("abc".into(), 10, None));
            crate::test_support::signed(state, m)
        };
        let mut everyone = state.server.roles[0].clone();
        everyone.permissions.remove(Permissions::ATTACH_FILES);
        state.apply(ServerEvent::RoleUpdate { server: sid, role: everyone.clone() }).unwrap();
        let first = with_voice(&state, plain);
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: first.clone() }), Ok(true), "voice needs no send files permission");
        everyone.permissions.remove(Permissions::SEND_VOICE_MESSAGES);
        everyone.permissions.insert(Permissions::ATTACH_FILES);
        state.apply(ServerEvent::RoleUpdate { server: sid, role: everyone }).unwrap();
        let blocked = with_voice(&state, plain);
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: blocked }), Err(StoreError::NoVoiceMessages));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: first }), Ok(false), "held messages stay");
        let mut file = Message::new(channel, plain, "look");
        file.attachments.push(crate::message::Attachment {
            blob: "abc".into(),
            filename: "cat.png".into(),
            size_bytes: 10,
            mime_type: "image/png".into(),
            provider: None,
        });
        let file = crate::test_support::signed(&state, file);
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: file }), Ok(true), "files still follow their own permission");
    }

    #[test]
    fn members_care_for_the_server_pet_that_managers_adopt() {
        use crate::pet::{Care, Pet, CARE_COOLDOWN_MS};
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let friend = join(&mut state, "friend");
        let care = |user, care, at| ServerEvent::PetCare { server: sid, user, care, at };
        assert_eq!(state.apply(care(friend, Care::Feed, 1)), Ok(false));
        assert!(!crate::action::authorized(&state.server, friend, &care(friend, Care::Feed, 1)));
        let adopt = ServerEvent::PetSet { server: sid, pet: Pet::new("Cosmo", "orange", 1) };
        assert!(crate::action::authorized(&state.server, owner, &adopt));
        assert!(!crate::action::authorized(&state.server, friend, &adopt));
        assert_eq!(state.apply(adopt), Ok(true));
        assert!(crate::action::authorized(&state.server, friend, &care(friend, Care::Feed, 1)));
        assert!(!crate::action::authorized(&state.server, owner, &care(friend, Care::Feed, 1)));
        assert_eq!(state.apply(care(friend, Care::Feed, 100)), Ok(true));
        assert_eq!(state.apply(care(friend, Care::Feed, 200)), Ok(false));
        assert_eq!(state.apply(care(friend, Care::Feed, 100 + CARE_COOLDOWN_MS)), Ok(true));
        assert_eq!(state.apply(care(UserId::new(), Care::Pat, 100)), Ok(false));
        assert_eq!(crate::pet::xp(&state.server.pet_care), 2);
        let json = serde_json::to_string(&state.snapshot()).unwrap();
        let back = ServerState::from_snapshot(serde_json::from_str(&json).unwrap());
        assert_eq!((back.server.pet.as_ref().map(|p| p.name.as_str()), back.server.pet_care.len()), (Some("Cosmo"), 1));
    }

    #[test]
    fn a_tip_jar_is_a_ko_fi_page_set_by_managers() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let friend = join(&mut state, "friend");
        let set = |url: Option<&str>| ServerEvent::TipJarSet { server: sid, url: url.map(str::to_string) };
        assert_eq!(state.apply(set(Some("https://ko-fi.com/rockets"))), Ok(true));
        assert_eq!(state.apply(set(Some("https://evil.example/rockets"))), Ok(false));
        assert_eq!(state.apply(set(Some("ko-fi.com/rockets"))), Ok(false));
        assert_eq!(state.server.tip_jar.as_deref(), Some("https://ko-fi.com/rockets"));
        assert!(crate::action::authorized(&state.server, owner, &set(None)));
        assert!(!crate::action::authorized(&state.server, friend, &set(None)));
        assert_eq!(state.apply(set(None)), Ok(true));
        assert_eq!(state.server.tip_jar, None);
    }

    #[test]
    fn a_server_tag_is_set_by_managers_and_worn_tags_must_be_well_formed() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let friend = join(&mut state, "friend");
        let tag = crate::ServerTag::new("rkt", Some("\u{1f680}"), "blue", "glow").unwrap();
        let set = |tag| ServerEvent::ServerTagSet { server: sid, tag };
        assert_eq!(state.apply(set(Some(tag.clone()))), Ok(true));
        assert_eq!(state.apply(set(Some(tag.clone()))), Ok(false));
        let forged = crate::ServerTag { color: "neon".into(), ..tag.clone() };
        assert_eq!(state.apply(set(Some(forged))), Ok(false));
        assert_eq!(state.server.tag.as_ref(), Some(&tag));
        assert!(crate::action::authorized(&state.server, owner, &set(None)));
        assert!(!crate::action::authorized(&state.server, friend, &set(None)));
        let json = serde_json::to_string(&state.snapshot()).unwrap();
        assert_eq!(ServerState::from_snapshot(serde_json::from_str(&json).unwrap()).server.tag, Some(tag.clone()));

        let worn = crate::WornTag::of(&state.server).unwrap();
        let mut profile = state.profiles[&friend].clone();
        profile.server_tag = Some(worn.clone());
        assert_eq!(state.apply(ServerEvent::ProfileUpdate { profile: profile.clone() }), Ok(true));
        assert_eq!(state.profiles[&friend].server_tag, Some(worn.clone()));
        profile.server_tag = Some(crate::WornTag { name: "x".repeat(40), ..worn });
        assert_eq!(state.apply(ServerEvent::ProfileUpdate { profile }), Ok(true));
        assert_eq!(state.profiles[&friend].server_tag, None, "a malformed tag is dropped, the rest of the profile kept");
        assert_eq!(state.apply(set(None)), Ok(true));
        assert_eq!(state.server.tag, None);
    }

    #[test]
    fn with_join_approval_newcomers_wait_outside_until_a_moderator_lets_them_in() {
        use crate::channel_key::{community_key_task, Keyring};
        use crate::test_support::{identity_for, keyed_profile, signed_event};
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let now = Utc::now();
        let plain = join(&mut state, "plain");
        let moderator = join(&mut state, "moderator");
        let on = signed_event(&state, owner, now, ServerEvent::JoinApprovalSet { server: sid, on: true, at: now.timestamp_millis() });
        assert_eq!(state.apply_signed(&on), Ok(true));
        let asker = keyed_profile("asker");
        assert_eq!(state.apply(ServerEvent::MemberJoin { server: sid, profile: asker.clone() }), Ok(false), "a plain join is refused");
        assert!(!state.server.is_member(asker.id));
        let ask = |at: i64, note: &str| ServerEvent::JoinAsk { server: sid, profile: asker.clone(), note: note.into(), at };
        let signed_ask = crate::action::SignedEvent::sign(sid, asker.id, now.timestamp_millis(), ask(1, "hi, Sam sent me"), &identity_for(asker.id));
        assert_eq!(state.apply_signed(&signed_ask), Ok(true));
        assert_eq!(state.server.join_requests[0].note, "hi, Sam sent me");
        assert!(!state.server.is_current_member(asker.id));
        let mut ring = Keyring::unlock_community(&state, owner, &identity_for(owner));
        let task = community_key_task(&state, owner, &ring).expect("the owner makes the first key");
        for event in ring.task_events(&state, crate::channel_key::KeyScope::Community, task, now.timestamp_millis()) {
            state.apply(event).unwrap();
        }
        ring = Keyring::unlock_community(&state, owner, &identity_for(owner));
        assert!(community_key_task(&state, owner, &ring).is_none(), "nobody hands the key to someone still waiting");
        assert!(state.community_keys().grant_for(asker.id).is_none());
        let by_plain = signed_event(&state, plain, now, ServerEvent::JoinAnswer { server: sid, user: asker.id, admit: true });
        assert_eq!(state.apply_signed(&by_plain), Err(StoreError::NotAllowed), "only moderators answer");
        let mut mods = state.server.roles[0].clone();
        mods.id = RoleId::new();
        mods.name = "mods".into();
        mods.position = 1;
        mods.permissions = Permissions::KICK_MEMBERS;
        let role = mods.id;
        state.apply(ServerEvent::RoleCreate { server: sid, role: mods }).unwrap();
        state.apply(ServerEvent::RoleAssign { server: sid, user: moderator, role }).unwrap();
        let later = now + chrono::Duration::seconds(1);
        let admit = signed_event(&state, moderator, later, ServerEvent::JoinAnswer { server: sid, user: asker.id, admit: true });
        assert_eq!(state.apply_signed(&admit), Ok(true));
        assert!(state.server.is_current_member(asker.id) && state.server.join_requests.is_empty());
        assert_eq!(state.profiles[&asker.id].username, "asker");
        assert_eq!(state.audit_log().last().unwrap().kind, crate::audit::AuditKind::JoinAdmit);
        let task = community_key_task(&state, owner, &ring);
        assert!(matches!(task, Some(crate::channel_key::KeyTask::Grant { ref to, .. }) if to.contains(&asker.id)), "now the key is theirs too");
        assert_eq!(state.apply(ServerEvent::MemberJoin { server: sid, profile: asker.clone() }), Ok(false), "their own join just refreshes the profile");
        assert_eq!(state.apply_signed(&signed_ask), Err(StoreError::NotAllowed), "members have nothing to ask");
    }

    #[test]
    fn turned_away_people_cannot_ask_again_and_switching_approval_off_lets_everyone_in() {
        use crate::test_support::{identity_for, keyed_profile, signed_event};
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let now = Utc::now();
        state.apply(ServerEvent::JoinApprovalSet { server: sid, on: true, at: 1 }).unwrap();
        let asker = keyed_profile("asker");
        let ask = |profile: &UserProfile, at: i64| ServerEvent::JoinAsk { server: sid, profile: profile.clone(), note: String::new(), at };
        assert_eq!(state.apply(ask(&asker, 1)), Ok(true));
        let wait = keyed_profile("waiting");
        assert_eq!(state.apply(ask(&wait, 2)), Ok(true));
        let no = signed_event(&state, owner, now, ServerEvent::JoinAnswer { server: sid, user: asker.id, admit: false });
        assert_eq!(state.apply_signed(&no), Ok(true));
        assert_eq!(state.audit_log().last().unwrap().kind, crate::audit::AuditKind::JoinTurnAway);
        assert_eq!(state.server.turned_away, vec![asker.id]);
        assert_eq!(state.apply(ask(&asker, 3)), Ok(false));
        let forged = ServerEvent::JoinAsk { server: sid, profile: asker.clone(), note: "x".repeat(400), at: 4 };
        assert_eq!(state.apply(forged), Ok(false));
        let mut liar = wait.clone();
        liar.identity = Some(identity_for(UserId::new()).public());
        assert_eq!(state.apply(ask(&liar, 5)), Err(StoreError::IdentityMismatch));
        assert_eq!(state.apply(ServerEvent::JoinApprovalSet { server: sid, on: false, at: 2 }), Ok(true));
        assert!(state.server.join_requests.is_empty());
        assert_eq!(state.apply(ServerEvent::MemberJoin { server: sid, profile: wait.clone() }), Ok(true));
        assert_eq!(state.apply(ask(&keyed_profile("late"), 6)), Ok(false), "nobody asks while the door is open");
    }

    #[test]
    fn join_sounds_are_the_members_own_and_follow_the_server_switch() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let friend = join(&mut state, "friend");
        state.server.soundboard.push(crate::server::SoundboardSound { name: "tada".into(), blob: "b".into(), volume: 1.0, emoji: None });
        let set = |user, sound: Option<&str>| ServerEvent::JoinSoundSet { server: sid, user, sound: sound.map(Into::into) };
        assert_eq!(state.apply(set(friend, Some("tada"))), Ok(true));
        assert_eq!(state.apply(set(friend, Some("tada"))), Ok(false));
        assert_eq!(state.apply(set(friend, Some("Not A Name!"))), Ok(false));
        assert_eq!(state.apply(set(UserId::new(), Some("tada"))), Ok(false), "strangers have none");
        assert_eq!(state.server.join_sound_of(friend), Some("tada"));
        assert_eq!(state.apply(set(owner, Some("default:airhorn"))), Ok(true));
        assert_eq!(state.server.join_sound_of(owner), Some("default:airhorn"));
        state.server.soundboard.clear();
        assert_eq!(state.server.join_sound_of(friend), None, "a removed sound stops playing");
        assert_eq!(state.apply(ServerEvent::JoinSoundsAllowed { server: sid, allowed: false }), Ok(true));
        assert_eq!(state.server.join_sound_of(owner), None);
        let json = serde_json::to_string(&state.snapshot()).unwrap();
        let restored = ServerState::from_snapshot(serde_json::from_str(&json).unwrap());
        assert!(restored.server.join_sounds_off);
        assert_eq!(restored.server.join_sounds.len(), 2);
        assert_eq!(state.apply(set(friend, None)), Ok(true));
        let signed = |by, user| crate::SignedEvent::sign(sid, by, 1, set(user, Some("tada")), &identity_for(by));
        assert!(crate::action::authorized(&state.server, friend, &signed(friend, friend).event));
        assert!(!crate::action::authorized(&state.server, owner, &signed(owner, friend).event));
        let switch = ServerEvent::JoinSoundsAllowed { server: sid, allowed: true };
        assert!(crate::action::authorized(&state.server, owner, &switch));
        assert!(!crate::action::authorized(&state.server, friend, &switch));
    }

    #[test]
    fn pronouns_are_the_members_own_and_kept_short() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let friend = join(&mut state, "friend");
        let set = |user, text: Option<&str>| ServerEvent::PronounsSet { server: sid, user, pronouns: text.map(Into::into) };
        assert_eq!(state.apply(set(friend, Some("  she / her "))), Ok(true));
        assert_eq!(state.server.pronouns.get(&friend).map(String::as_str), Some("she / her"));
        assert_eq!(state.apply(set(friend, Some("she / her"))), Ok(false));
        assert_eq!(state.apply(set(friend, Some("   "))), Ok(false));
        assert_eq!(state.apply(set(friend, Some("a very very long string of pronouns"))), Ok(false));
        assert_eq!(state.server.pronouns.get(&friend).map(String::as_str), Some("she / her"));
        assert_eq!(state.apply(set(UserId::new(), Some("they / them"))), Ok(false), "strangers have none");
        let json = serde_json::to_string(&state.snapshot()).unwrap();
        let restored = ServerState::from_snapshot(serde_json::from_str(&json).unwrap());
        assert_eq!(restored.server.pronouns.len(), 1);
        assert_eq!(state.apply(set(friend, None)), Ok(true));
        assert!(state.server.pronouns.is_empty());
        let signed = |by, user| crate::SignedEvent::sign(sid, by, 1, set(user, Some("he / him")), &identity_for(by));
        assert!(crate::action::authorized(&state.server, friend, &signed(friend, friend).event));
        assert!(!crate::action::authorized(&state.server, owner, &signed(owner, friend).event));
    }

    #[test]
    fn nicknames_are_trimmed_kept_short_and_cleared_with_none() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let friend = join(&mut state, "friend");
        let set = |user, text: Option<&str>| ServerEvent::MemberNickname { server: sid, user, nickname: text.map(Into::into) };
        let nickname_of = |state: &ServerState, user| state.server.member(user).and_then(|m| m.nickname.clone());
        assert_eq!(state.apply(set(friend, Some("  Sammy "))), Ok(true));
        assert_eq!(nickname_of(&state, friend).as_deref(), Some("Sammy"));
        assert_eq!(state.apply(set(friend, Some("Sammy"))), Ok(false), "unchanged");
        assert_eq!(state.apply(set(friend, Some("   "))), Ok(false), "blank is not a nickname");
        assert_eq!(state.apply(set(friend, Some("Sam\u{7}my"))), Ok(false), "control characters");
        assert_eq!(state.apply(set(friend, Some(&"n".repeat(33)))), Ok(false), "too long");
        assert_eq!(state.apply(set(friend, Some(&"n".repeat(32)))), Ok(true), "the longest allowed");
        assert_eq!(nickname_of(&state, friend).as_deref(), Some("n".repeat(32).as_str()));
        assert_eq!(state.apply(set(UserId::new(), Some("ghost"))), Ok(false), "strangers have none");
        assert_eq!(state.apply(set(owner, Some("Boss"))), Ok(true), "the owner is a member like any other");
        let json = serde_json::to_string(&state.snapshot()).unwrap();
        let restored = ServerState::from_snapshot(serde_json::from_str(&json).unwrap());
        assert_eq!(nickname_of(&restored, owner).as_deref(), Some("Boss"));
        assert_eq!(state.apply(set(friend, None)), Ok(true));
        assert_eq!(state.apply(set(friend, None)), Ok(false), "already cleared");
        assert_eq!(nickname_of(&state, friend), None);
        assert_eq!(state.apply(ServerEvent::MemberNickname { server: ServerId::new(), user: owner, nickname: None }), Err(StoreError::WrongServer));
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
        assert_eq!(state.prune_expired(now), 0);
        state.server.channels.iter_mut().find(|c| c.id == channel).unwrap().retention_days = Some(7);
        assert_eq!(state.prune_expired(now), 1);
        let left: Vec<_> = state.messages(channel).iter().map(|m| m.content.as_str()).collect();
        assert_eq!(left, vec!["2d", "0d"]);
    }

    fn post_aged(state: &mut ServerState, channel: ChannelId, owner: UserId, age_days: i64, reply_to: Option<MessageId>) -> Message {
        let mut msg = Message::new(channel, owner, format!("{age_days}d"));
        msg.created_at = Utc::now() - chrono::Duration::days(age_days);
        msg.reply_to = reply_to;
        let msg = signed(state, msg);
        state.apply(ServerEvent::MessageCreate { message: msg.clone() }).unwrap();
        msg
    }

    fn contents(state: &ServerState, channel: ChannelId) -> Vec<&str> {
        state.messages(channel).iter().map(|m| m.content.as_str()).collect()
    }

    #[test]
    fn roll_off_drops_messages_older_than_the_floor() {
        let (mut state, channel, owner) = fixture();
        for age in [200, 100, 10] {
            post_aged(&mut state, channel, owner, age, None);
        }
        assert_eq!(state.roll_off(None), 0);
        assert_eq!(contents(&state, channel), vec!["200d", "100d", "10d"]);
        let floor = Utc::now() - chrono::Duration::days(90);
        assert_eq!(state.roll_off(Some(floor)), 2);
        assert_eq!(contents(&state, channel), vec!["10d"]);
        assert_eq!(state.roll_off(Some(floor)), 0);
    }

    #[test]
    fn roll_off_keeps_pinned_messages_and_what_kept_replies_point_to() {
        let (mut state, channel, owner) = fixture();
        let pinned = post_aged(&mut state, channel, owner, 300, None);
        let root = post_aged(&mut state, channel, owner, 200, None);
        let middle = post_aged(&mut state, channel, owner, 150, Some(root.id));
        post_aged(&mut state, channel, owner, 120, None);
        post_aged(&mut state, channel, owner, 5, Some(middle.id));
        state.apply(pin_event(&state, channel, pinned.id, owner, true)).unwrap();
        let floor = Utc::now() - chrono::Duration::days(90);
        assert_eq!(state.roll_off(Some(floor)), 1);
        assert_eq!(contents(&state, channel), vec!["300d", "200d", "150d", "5d"]);
        assert!(state.has_message(pinned.id));
    }

    #[test]
    fn edits_and_reactions_heard_before_their_message_apply_once_it_arrives() {
        let (mut state, channel, owner) = fixture();
        let friend = join(&mut state, "friend");
        let msg = post(&state, channel, owner, "v1");
        let later = Utc::now() + chrono::Duration::seconds(1);
        let edited = edit(&state, &msg, "v2", later);
        let react = |user, emoji: &str| ServerEvent::ReactionSet { channel, message: msg.id, emoji: emoji.into(), user, active: true };
        let signed = signed_event(&state, friend, Utc::now(), react(friend, "👍"));
        assert_eq!(state.apply(edited), Err(StoreError::UnknownMessage));
        assert_eq!(state.apply_signed(&signed), Err(StoreError::UnknownMessage));
        assert_eq!(state.apply(react(owner, "🎉")), Err(StoreError::UnknownMessage));
        assert_eq!(create(&mut state, &msg), Ok(true));
        let shown = state.message(msg.id).unwrap();
        assert_eq!(shown.content, "v2");
        let mut reactions: Vec<(String, Vec<UserId>)> = shown.reactions.iter().map(|r| (r.emoji.clone(), r.users.clone())).collect();
        reactions.sort();
        assert_eq!(reactions, vec![("🎉".to_string(), vec![owner]), ("👍".to_string(), vec![friend])]);
        assert_eq!(create(&mut state, &msg), Ok(false), "nothing is replayed twice");
    }

    #[test]
    fn nothing_waits_for_a_message_that_was_deleted_and_the_wait_is_capped() {
        let (mut state, channel, owner) = fixture();
        let gone = post(&state, channel, owner, "gone");
        state.apply(delete(&state, &gone, owner)).unwrap();
        let react = |message: &Message, emoji: String| ServerEvent::ReactionSet { channel, message: message.id, emoji, user: owner, active: true };
        assert_eq!(state.apply(react(&gone, "👍".into())), Err(StoreError::UnknownMessage));
        assert!(state.waiting.is_empty());
        let unseen: Vec<Message> = (0..MAX_WAITING / MAX_WAITING_PER_MESSAGE + 2).map(|i| post(&state, channel, owner, &format!("m{i}"))).collect();
        for m in &unseen {
            for i in 0..MAX_WAITING_PER_MESSAGE + 5 {
                let _ = state.apply(react(m, format!(":e{i}:")));
            }
        }
        assert!(state.waiting.values().all(|list| list.len() <= MAX_WAITING_PER_MESSAGE));
        assert_eq!(state.waiting.values().map(Vec::len).sum::<usize>(), MAX_WAITING);
    }

    #[test]
    fn a_rolled_off_message_does_not_come_back_unless_pinned() {
        let (mut state, channel, owner) = fixture();
        let old = post_aged(&mut state, channel, owner, 200, None);
        let floor = Utc::now() - chrono::Duration::days(90);
        assert_eq!(state.roll_off(Some(floor)), 1);
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: old.clone() }), Ok(false));
        assert!(!state.has_message(old.id));
        state.apply(pin_event(&state, channel, old.id, owner, true)).unwrap();
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: old.clone() }), Ok(true));
        assert!(state.has_message(old.id));
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
    fn onboarding_questions_are_kept_when_tidy_and_survive_a_restart() {
        use crate::onboarding::{Answer, Onboarding, Question};
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let role = RoleId::new();
        let tidy = Onboarding {
            questions: vec![Question {
                title: "Pick one".into(),
                description: String::new(),
                multiple: false,
                answers: vec![Answer { label: "A".into(), roles: vec![role] }, Answer { label: "B".into(), roles: vec![] }],
            }],
        };
        let signed = crate::SignedEvent::sign(sid, owner, 1_000, ServerEvent::OnboardingSet { server: sid, onboarding: Some(tidy.clone()) }, &identity_for(owner));
        assert_eq!(state.apply_signed(&signed), Ok(true));
        assert_eq!(state.server.onboarding, Some(tidy.clone()));
        assert_eq!(state.audit_log().last().map(|e| e.kind), Some(crate::AuditKind::Onboarding));
        let restored = ServerState::from_snapshot(state.snapshot());
        assert_eq!(restored.server.onboarding, Some(tidy.clone()));
        assert_eq!(state.apply(ServerEvent::OnboardingSet { server: sid, onboarding: Some(tidy.clone()) }), Ok(false), "same again");
        let untidy = Onboarding { questions: vec![Question { title: "  spaced ".into(), ..tidy.questions[0].clone() }] };
        assert_eq!(state.apply(ServerEvent::OnboardingSet { server: sid, onboarding: Some(untidy) }), Ok(false));
        assert_eq!(state.apply(ServerEvent::OnboardingSet { server: sid, onboarding: Some(Onboarding::default()) }), Ok(false), "no questions means none");
        assert_eq!(state.server.onboarding, Some(tidy));
        let member = join(&mut state, "member");
        let by_member = crate::SignedEvent::sign(sid, member, 2_000, ServerEvent::OnboardingSet { server: sid, onboarding: None }, &identity_for(member));
        assert_eq!(state.apply_signed(&by_member), Err(StoreError::NotAllowed));
        let off = crate::SignedEvent::sign(sid, owner, 3_000, ServerEvent::OnboardingSet { server: sid, onboarding: None }, &identity_for(owner));
        assert_eq!(state.apply_signed(&off), Ok(true));
        assert_eq!(state.server.onboarding, None);
        assert_eq!(state.apply(ServerEvent::OnboardingSet { server: ServerId::new(), onboarding: None }), Err(StoreError::WrongServer));
    }

    #[test]
    fn auto_moderation_rules_hold_messages_and_survive_a_restart() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let member = join(&mut state, "member");
        let message = post(&state, channel, member, "buy cheap gold");
        create(&mut state, &message).unwrap();
        assert_eq!(state.hold(&message), None);
        let rules = crate::AutoMod { blocked_terms: vec!["gold".into()], ..Default::default() };
        let signed = crate::SignedEvent::sign(sid, owner, 1_000, ServerEvent::AutoModSet { server: sid, rules: rules.clone() }, &identity_for(owner));
        assert_eq!(state.apply_signed(&signed), Ok(true));
        assert_eq!(state.hold(&message), Some(crate::Hold::BlockedTerm), "older messages are held too");
        assert!(state.has_message(message.id), "held, not deleted");
        let restored = reload(&state);
        assert_eq!(restored.automod(), &rules);
        assert_eq!(restored.signed_actions(), vec![&signed]);
        let mut older_build = serde_json::to_value(state.snapshot()).unwrap();
        older_build.as_object_mut().unwrap().remove("automod");
        let without: crate::Snapshot = serde_json::from_value(older_build).unwrap();
        assert!(ServerState::from_snapshot(without).automod().is_off(), "snapshots from before auto moderation load");
        let untidy = crate::AutoMod { blocked_terms: vec!["Gold".into()], ..Default::default() };
        assert_eq!(state.apply(ServerEvent::AutoModSet { server: sid, rules: untidy }), Ok(false));
        let by_member = crate::SignedEvent::sign(sid, member, 2_000, ServerEvent::AutoModSet { server: sid, rules: Default::default() }, &identity_for(member));
        assert_eq!(state.apply_signed(&by_member), Err(StoreError::NotAllowed));
        let off = crate::SignedEvent::sign(sid, owner, 3_000, ServerEvent::AutoModSet { server: sid, rules: Default::default() }, &identity_for(owner));
        assert_eq!(state.apply_signed(&off), Ok(true));
        assert_eq!(state.hold(&message), None);
    }

    #[test]
    fn members_mark_only_themselves_as_bots() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let bot = join(&mut state, "bot");
        let declare = |by, at, user, bot| crate::SignedEvent::sign(sid, by, at, ServerEvent::BotSet { server: sid, user, bot }, &identity_for(by));
        assert_eq!(state.apply_signed(&declare(owner, 1_000, bot, true)), Err(StoreError::NotAllowed), "nobody else can label a member");
        assert_eq!(state.apply_unsigned(ServerEvent::BotSet { server: sid, user: bot, bot: true }), Err(StoreError::Unsigned));
        let own = declare(bot, 2_000, bot, true);
        assert_eq!(state.apply_signed(&own), Ok(true));
        assert!(state.is_bot(bot) && !state.is_bot(owner));
        assert_eq!(state.apply_signed(&own), Ok(false), "redelivery");
        assert!(state.signed_actions().contains(&&own), "relayed to members who missed it");
        let restored = reload(&state);
        assert!(restored.is_bot(bot));
        let mut older_build = serde_json::to_value(state.snapshot()).unwrap();
        older_build.as_object_mut().unwrap().remove("bots");
        let without: crate::Snapshot = serde_json::from_value(older_build).unwrap();
        assert_eq!(ServerState::from_snapshot(without).bots().count(), 0, "snapshots from before bot badges load");
        assert!(crate::action::snapshot_omits("bot:x"), "owner snapshots do not carry the badge, so its action stays fresh");
        state.apply(ServerEvent::MemberLeave { server: sid, user: bot }).unwrap();
        assert!(!state.is_bot(bot));
        assert_eq!(state.apply(ServerEvent::BotSet { server: sid, user: bot, bot: true }), Ok(false), "only members");
    }

    #[test]
    fn bots_register_their_own_commands_and_members_keep_them() {
        use crate::command::{BotCommand, CommandOption, OptionKind};
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let bot = join(&mut state, "bot");
        let roll = BotCommand {
            name: "roll".into(),
            description: "Roll dice".into(),
            options: vec![CommandOption { name: "sides".into(), description: "How many sides".into(), required: false, kind: OptionKind::Integer }],
        };
        let set = |by, at, bot, commands: Vec<BotCommand>| {
            crate::SignedEvent::sign(sid, by, at, ServerEvent::BotCommandsSet { server: sid, bot, commands, by }, &identity_for(by))
        };
        assert_eq!(state.apply_signed(&set(bot, 1_000, bot, vec![roll.clone()])), Ok(false), "no badge yet");
        let badge = crate::SignedEvent::sign(sid, bot, 1_500, ServerEvent::BotSet { server: sid, user: bot, bot: true }, &identity_for(bot));
        state.apply_signed(&badge).unwrap();
        assert_eq!(state.apply_signed(&set(owner, 2_000, bot, vec![roll.clone()])), Err(StoreError::NotAllowed), "only the bot itself");
        let own = set(bot, 3_000, bot, vec![roll.clone()]);
        assert_eq!(state.apply_signed(&own), Ok(true));
        assert_eq!(state.commands_of(bot), &[roll.clone()]);
        assert_eq!(state.apply_signed(&set(bot, 4_000, bot, vec![roll.clone()])), Ok(false), "same list again");
        assert!(state.signed_actions().iter().any(|s| s.event == own.event), "relayed to members who missed it");
        assert!(crate::action::snapshot_omits("bot_commands:x"), "owner snapshots do not carry the list, so the action stays fresh");
        let restored = reload(&state);
        assert_eq!(restored.commands_of(bot), &[roll.clone()]);
        let mut older_build = serde_json::to_value(state.snapshot()).unwrap();
        older_build.as_object_mut().unwrap().remove("bot_commands");
        let without: crate::Snapshot = serde_json::from_value(older_build).unwrap();
        assert!(ServerState::from_snapshot(without).commands_of(bot).is_empty(), "snapshots from before slash commands load");
        let bad = BotCommand { name: "Roll".into(), ..roll.clone() };
        assert_eq!(state.apply_signed(&set(bot, 5_000, bot, vec![bad])), Ok(false), "invalid lists are ignored");
        let used = CommandUse::new(bot, "roll").with("sides", "20");
        assert_eq!(state.command_of(&used), Some(&roll));
        assert_eq!(state.command_of(&CommandUse::new(bot, "roll").with("sides", "d20")), None);
        assert_eq!(state.command_of(&CommandUse::new(owner, "roll")), None);
        let mut rebuilt = ServerState::new(state.server.clone());
        rebuilt.apply(ServerEvent::BotSet { server: sid, user: bot, bot: true }).unwrap();
        rebuilt.carry_bot_commands(&state);
        assert_eq!(rebuilt.commands_of(bot), &[roll.clone()]);
        assert_eq!(state.apply_signed(&set(bot, 6_000, bot, vec![])), Ok(true), "an empty list clears them");
        assert!(state.commands_of(bot).is_empty());
        assert_eq!(state.apply_signed(&set(bot, 7_000, bot, vec![roll.clone()])), Ok(true));
        state.apply(ServerEvent::MemberLeave { server: sid, user: bot }).unwrap();
        assert!(state.commands_of(bot).is_empty(), "leaving drops them");
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

    fn forum_fixture() -> (ServerState, ChannelId, UserId) {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let forum = Channel::new("ideas", ChannelKind::Forum, 7);
        let fid = forum.id;
        state.apply(ServerEvent::ChannelCreate { server: sid, channel: forum }).unwrap();
        (state, fid, owner)
    }

    fn tag(name: &str, emoji: Option<&str>) -> crate::server::ForumTag {
        crate::server::ForumTag { id: crate::ids::TagId::new(), name: name.into(), emoji: emoji.map(String::from) }
    }

    fn post_event(state: &ServerState, channel: ChannelId, message: MessageId, title: &str, tags: Vec<crate::ids::TagId>, by: UserId) -> ServerEvent {
        ServerEvent::ForumPost { server: state.server.id, channel, message, title: title.into(), tags, by }
    }

    #[test]
    fn forum_tags_are_set_by_channel_managers_and_cleaned() {
        let (mut state, forum, owner) = forum_fixture();
        let sid = state.server.id;
        let text = state.server.channels[0].id;
        let member = join(&mut state, "plain");
        let tags = vec![tag(" Feature  request ", Some("\u{1F4A1}")), tag("feature request", None), tag("Bug", Some("nope")), tag("", None)];
        let set = |channel, tags: Vec<crate::server::ForumTag>, by| ServerEvent::ForumTagsSet { server: sid, channel, tags, by };
        assert!(!crate::action::authorized(&state.server, member, &set(forum, tags.clone(), member)), "members cannot set tags");
        assert!(!crate::action::authorized(&state.server, owner, &set(forum, tags.clone(), member)), "signed by someone else");
        assert!(crate::action::authorized(&state.server, owner, &set(forum, tags.clone(), owner)));
        assert_eq!(state.apply(set(text, tags.clone(), owner)), Ok(false), "only forums hold tags");
        assert_eq!(state.apply(set(forum, tags.clone(), owner)), Ok(true));
        let kept = &state.server.channel(forum).unwrap().tags;
        assert_eq!(kept.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), vec!["Feature request", "Bug"]);
        assert_eq!(kept[0].emoji.as_deref(), Some("\u{1F4A1}"));
        assert_eq!(kept[1].emoji, None, "alphanumeric text is not an emoji");
        assert_eq!(state.apply(set(forum, kept.clone(), owner)), Ok(false), "unchanged");
        let long = vec![tag(&"x".repeat(crate::server::MAX_FORUM_TAG_CHARS + 1), None)];
        assert_eq!(state.apply(set(forum, long, owner)), Ok(false));
        let many: Vec<_> = (0..=crate::server::MAX_FORUM_TAGS).map(|i| tag(&format!("t{i}"), None)).collect();
        assert_eq!(state.apply(set(forum, many, owner)), Ok(false));
        assert_eq!(state.apply(set(ChannelId::new(), vec![], owner)), Err(StoreError::UnknownChannel));
    }

    #[test]
    fn posting_needs_the_send_permission_in_a_forum() {
        let (mut state, forum, owner) = forum_fixture();
        let sid = state.server.id;
        let text = state.server.channels[0].id;
        let member = join(&mut state, "poster");
        let stranger = UserId::new();
        state.apply(ServerEvent::ForumTagsSet { server: sid, channel: forum, tags: vec![tag("Bug", None), tag("Idea", None)], by: owner }).unwrap();
        let tags: Vec<_> = state.server.channel(forum).unwrap().tags.iter().map(|t| t.id).collect();
        let root = post(&state, forum, member, "the body of the post");
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: root.clone() }), Ok(true));
        let event = post_event(&state, forum, root.id, "  Borrow checker   woes ", vec![tags[1], tags[1], crate::ids::TagId::new()], member);
        assert!(crate::action::authorized(&state.server, member, &event));
        assert!(!crate::action::authorized(&state.server, stranger, &event), "not a member");
        assert!(!crate::action::authorized(&state.server, owner, &event), "signed by someone else than the poster");
        let foreign = post_event(&state, forum, MessageId::for_author(member), "mine", vec![], owner);
        assert!(!crate::action::authorized(&state.server, owner, &foreign), "the root must be the poster's own message");
        let in_text = post_event(&state, text, MessageId::for_author(member), "nope", vec![], member);
        assert!(!crate::action::authorized(&state.server, member, &in_text), "text channels have no posts");
        assert_eq!(state.apply(in_text), Ok(false));
        let role = Role { id: RoleId::new(), name: "writers".into(), color: None, permissions: Permissions::NONE, position: 1, hoist: false, self_assignable: false, mentionable: false, tag: None, banner: None };
        let writers = role.id;
        state.apply(ServerEvent::RoleCreate { server: sid, role }).unwrap();
        state.apply(ServerEvent::ChannelAccessSet { server: sid, channel: forum, view: vec![], send: vec![writers] }).unwrap();
        assert!(!crate::action::authorized(&state.server, member, &event), "the send roles gate posts like messages");
        state.apply(ServerEvent::ChannelAccessSet { server: sid, channel: forum, view: vec![], send: vec![] }).unwrap();
        assert_eq!(state.apply(event.clone()), Ok(true));
        let kept = state.server.channel(forum).unwrap().posts.get(&root.id).unwrap().clone();
        assert_eq!(kept.title, "Borrow checker woes");
        assert_eq!(kept.tags, vec![tags[1]], "unknown and repeated tags drop out");
        assert_eq!(kept.author, member);
        assert_eq!(state.apply(event), Ok(false), "unchanged");
        let reply = signed(&state, Message { reply_to: Some(root.id), ..Message::new(forum, member, "a reply") });
        state.apply(ServerEvent::MessageCreate { message: reply.clone() }).unwrap();
        assert_eq!(state.apply(post_event(&state, forum, reply.id, "reply as post", vec![], member)), Ok(false), "replies are not roots");
        let early = MessageId::for_author(member);
        assert_eq!(state.apply(post_event(&state, forum, early, "arrives first", vec![], member)), Ok(true), "the post may land before its root");
        assert_eq!(state.apply(post_event(&state, forum, MessageId::for_author(owner), "not mine", vec![], member)), Ok(false));
        assert_eq!(state.apply(post_event(&state, forum, root.id, "", vec![], member)), Ok(false), "a title is needed");
        state.apply(delete(&state, &root, member)).unwrap();
        assert!(!state.server.channel(forum).unwrap().posts.contains_key(&root.id), "deleting the root removes the post");
        assert_eq!(state.apply(post_event(&state, forum, root.id, "back", vec![], member)), Ok(false), "deleted roots stay gone");
    }

    #[test]
    fn a_post_title_changes_by_its_author_or_a_moderator_only() {
        let (mut state, forum, owner) = forum_fixture();
        let sid = state.server.id;
        let author = join(&mut state, "author");
        let other = join(&mut state, "other");
        let moderator = join_moderator(&mut state, "mod");
        state.apply(ServerEvent::ForumTagsSet { server: sid, channel: forum, tags: vec![tag("Bug", None), tag("Idea", None)], by: owner }).unwrap();
        let tags: Vec<_> = state.server.channel(forum).unwrap().tags.iter().map(|t| t.id).collect();
        let root = post(&state, forum, author, "body");
        state.apply(ServerEvent::MessageCreate { message: root.clone() }).unwrap();
        state.apply(post_event(&state, forum, root.id, "First title", vec![tags[0]], author)).unwrap();
        let retitle = |by| post_event(&state, forum, root.id, "Second title", vec![tags[0], tags[1]], by);
        assert!(crate::action::authorized(&state.server, author, &retitle(author)));
        assert!(!crate::action::authorized(&state.server, other, &retitle(other)), "strangers cannot retitle");
        assert!(crate::action::authorized(&state.server, moderator, &retitle(moderator)));
        assert!(crate::action::authorized(&state.server, owner, &retitle(owner)));
        assert_eq!(state.apply(retitle(moderator)), Ok(true));
        let kept = state.server.channel(forum).unwrap().posts.get(&root.id).unwrap().clone();
        assert_eq!((kept.title.as_str(), kept.tags.len(), kept.author), ("Second title", 2, author), "the author stays the poster");
        state.apply(ServerEvent::ForumTagsSet { server: sid, channel: forum, tags: vec![state.server.channel(forum).unwrap().tags[1].clone()], by: owner }).unwrap();
        assert_eq!(state.server.channel(forum).unwrap().posts.get(&root.id).unwrap().tags, vec![tags[1]], "removed tags leave the posts");
        let snapshot = state.snapshot();
        let back = ServerState::from_snapshot(snapshot);
        assert_eq!(back.server.channel(forum).unwrap().posts, state.server.channel(forum).unwrap().posts);
    }

    #[test]
    fn slow_mode_holds_members_to_the_interval_but_not_moderators() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let voice = state.server.channels.iter().find(|c| c.kind == ChannelKind::Voice).unwrap().id;
        let slow = |channel, n| ServerEvent::ChannelSlowMode { server: sid, channel, seconds: n };
        assert_eq!(state.apply(slow(voice, 5)), Ok(false), "voice channels have no slow mode");
        assert_eq!(state.apply(slow(channel, 100_000)), Ok(false), "capped");
        assert_eq!(state.apply(slow(channel, 30)), Ok(true));
        assert_eq!(state.apply(slow(channel, 30)), Ok(false), "unchanged");
        let member = join(&mut state, "chatty");
        let t0 = Utc::now() - chrono::Duration::seconds(120);
        let first = post_at(&state, channel, member, "one", t0);
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: first }), Ok(true));
        assert_eq!(state.slow_mode_wait(channel, member, t0 + chrono::Duration::seconds(10)), Some(20));
        let quick = post_at(&state, channel, member, "two", t0 + chrono::Duration::seconds(10));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: quick }), Ok(false), "too soon");
        let later = post_at(&state, channel, member, "three", t0 + chrono::Duration::seconds(31));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: later }), Ok(true));
        assert_eq!(state.messages(channel).len(), 2);
        let a = post_at(&state, channel, owner, "mod one", t0);
        let b = post_at(&state, channel, owner, "mod two", t0 + chrono::Duration::seconds(1));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: a }), Ok(true));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: b }), Ok(true), "owners bypass");
        assert_eq!(state.slow_mode_wait(channel, owner, Utc::now()), None);
        assert_eq!(state.apply(slow(channel, 0)), Ok(true));
        assert_eq!(state.slow_mode_wait(channel, member, Utc::now()), None);
    }

    #[test]
    fn threads_are_named_archived_and_locked_by_their_starter_or_a_moderator() {
        use crate::action::authorized;
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let starter = join(&mut state, "starter");
        let other = join(&mut state, "other");
        let moderator = join_moderator(&mut state, "mod");
        let root = post(&state, channel, starter, "where should we meet");
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: root.clone() }), Ok(true));
        let set = |by, name: Option<&str>, archived, locked| ServerEvent::ThreadSet {
            server: sid,
            channel,
            root: root.id,
            name: name.map(str::to_string),
            archived,
            locked,
            by,
        };
        let rename = |by| set(by, Some("meetup"), false, false);
        assert!(authorized(&state.server, starter, &rename(starter)), "the root author names their thread");
        assert!(authorized(&state.server, owner, &rename(owner)));
        assert!(authorized(&state.server, moderator, &rename(moderator)));
        assert!(!authorized(&state.server, other, &rename(other)), "a plain member cannot");
        assert!(!authorized(&state.server, other, &rename(starter)), "nor pretend to be the author");
        assert!(!authorized(&state.server, UserId::new(), &rename(UserId::new())), "stranger");
        assert_eq!(state.apply(rename(starter)), Ok(true));
        assert_eq!(state.apply(rename(starter)), Ok(false), "unchanged");
        assert_eq!(state.thread_of(channel, root.id).and_then(|t| t.name.clone()).as_deref(), Some("meetup"));
        assert_eq!(state.apply(set(starter, Some(&"x".repeat(101)), false, false)), Ok(false), "name too long");
        assert_eq!(state.apply(set(starter, None, false, false)), Ok(true), "back to plain forgets the entry");
        assert!(state.thread_of(channel, root.id).is_none());
        let voice = state.server.channels.iter().find(|c| c.kind == ChannelKind::Voice).unwrap().id;
        let elsewhere = ServerEvent::ThreadSet { server: sid, channel: voice, root: root.id, name: None, archived: false, locked: true, by: owner };
        assert_eq!(state.apply(elsewhere), Err(StoreError::WrongChannel));
        assert_eq!(state.apply(set(moderator, Some("meetup"), false, true)), Ok(true), "locked");
        let reply = |state: &ServerState, author, to: MessageId, text: &str| {
            signed(state, Message { reply_to: Some(to), ..Message::new(channel, author, text) })
        };
        let blocked = reply(&state, other, root.id, "friday?");
        assert!(!authorized(&state.server, other, &ServerEvent::MessageCreate { message: blocked.clone() }));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: blocked }), Err(StoreError::ThreadLocked));
        let allowed = reply(&state, moderator, root.id, "settled: friday");
        assert!(authorized(&state.server, moderator, &ServerEvent::MessageCreate { message: allowed.clone() }));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: allowed.clone() }), Ok(true));
        let nested = reply(&state, other, allowed.id, "ok");
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: nested }), Err(StoreError::ThreadLocked), "deeper replies too");
        assert_eq!(state.messages(channel).len(), 2);
        assert_eq!(state.apply(set(moderator, Some("meetup"), true, false)), Ok(true), "archived and unlocked");
        let revived = reply(&state, other, allowed.id, "one more thing");
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: revived }), Ok(true));
        let thread = state.thread_of(channel, root.id).unwrap();
        assert!(!thread.archived, "a reply reopens an archived thread");
        assert_eq!(thread.name.as_deref(), Some("meetup"));
        let restored = ServerState::from_snapshot(state.snapshot());
        assert_eq!(restored.thread_of(channel, root.id), state.thread_of(channel, root.id));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: root.clone() }), Ok(false));
        let gone = delete(&state, &root, starter);
        assert_eq!(state.apply(gone), Ok(true));
        assert!(state.thread_of(channel, root.id).is_none(), "deleting the root forgets the thread");
    }

    #[test]
    fn role_updates() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let everyone = state.server.roles[0].clone();
        let pickable = crate::server::Role { self_assignable: true, ..everyone.clone() };
        assert_eq!(state.apply(ServerEvent::RoleUpdate { server: sid, role: pickable }), Ok(false), "everyone is not pickable");
        let pingable = crate::server::Role { mentionable: true, ..everyone.clone() };
        assert_eq!(state.apply(ServerEvent::RoleUpdate { server: sid, role: pingable }), Ok(false), "everyone is not a mentionable role");
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
    fn voice_moves_apply_only_to_voice_members_headed_somewhere_else() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let other = UserProfile::new("other");
        let uid = other.id;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: other }).unwrap();
        let general = state.server.channels.iter().find(|c| c.kind == ChannelKind::Voice).unwrap().id;
        let text = state.server.channels.iter().find(|c| c.kind == ChannelKind::Text).unwrap().id;
        let lobby = crate::server::Channel::new("Lobby", ChannelKind::Voice, 9);
        let lobby_id = lobby.id;
        state.apply(ServerEvent::ChannelCreate { server: sid, channel: lobby }).unwrap();
        let move_to = |user, channel| ServerEvent::VoiceMove { server: sid, user, channel };
        assert_eq!(state.apply(move_to(uid, Some(lobby_id))), Ok(false), "not in voice");
        let join = ServerEvent::VoiceState { user: uid, channel: Some(general), muted: false, deafened: false, since: None };
        state.apply(join).unwrap();
        assert_eq!(state.apply(move_to(uid, Some(lobby_id))), Ok(true));
        assert_eq!(state.voice_members(general), vec![uid], "the member's own voice state follows later");
        assert_eq!(state.apply(move_to(uid, Some(general))), Ok(false), "already there");
        assert_eq!(state.apply(move_to(uid, Some(text))), Ok(false), "not a voice channel");
        assert_eq!(state.apply(move_to(uid, Some(ChannelId::new()))), Err(StoreError::UnknownChannel));
        assert_eq!(state.apply(move_to(uid, None)), Ok(true));
        let owner_join = ServerEvent::VoiceState { user: owner, channel: Some(general), muted: false, deafened: false, since: None };
        state.apply(owner_join).unwrap();
        assert_eq!(state.apply(move_to(owner, Some(lobby_id))), Ok(false), "owner is exempt");
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

    fn by_owner(state: &ServerState, owner: UserId, at: i64, event: ServerEvent) -> SignedEvent {
        SignedEvent::sign(state.server.id, owner, at, event, &identity_for(owner))
    }

    #[test]
    fn a_kick_from_before_a_rejoin_no_longer_applies() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let me = keyed_profile("me");
        let kick = |at| by_owner(&state, owner, at, ServerEvent::MemberLeave { server: sid, user: me.id });
        let (old_kick, new_kick) = (kick(1_000), kick(3_000));
        assert_eq!(state.join_as_of(me.clone(), 2_000), Ok(true));
        assert_eq!(state.apply_signed(&old_kick), Ok(false));
        assert!(state.server.is_member(me.id), "the kick came before the rejoin");
        assert_eq!(state.apply_signed(&new_kick), Ok(true));
        assert!(!state.server.is_member(me.id), "a kick after the rejoin still holds");
    }

    #[test]
    fn an_unban_from_before_a_rejoin_lets_the_rejoined_member_back_in() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let me = keyed_profile("me");
        state.apply(ServerEvent::MemberJoin { server: sid, profile: me.clone() }).unwrap();
        let ban = by_owner(&state, owner, 1_000, ServerEvent::MemberBan { server: sid, user: me.id });
        let unban = by_owner(&state, owner, 2_000, ServerEvent::MemberUnban { server: sid, user: me.id });
        assert_eq!(state.join_as_of(me.clone(), 3_000), Ok(false));
        assert_eq!(state.apply_signed(&ban), Ok(true));
        assert!(!state.server.is_current_member(me.id));
        assert_eq!(state.apply_signed(&unban), Ok(true));
        assert!(state.server.is_current_member(me.id), "the rejoin came after the unban");
    }

    #[test]
    fn a_join_refused_while_banned_counts_once_an_earlier_unban_arrives() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let member = keyed_profile("member");
        state.apply(ServerEvent::MemberJoin { server: sid, profile: member.clone() }).unwrap();
        let ban = by_owner(&state, owner, 1_000, ServerEvent::MemberBan { server: sid, user: member.id });
        let unban = by_owner(&state, owner, 2_000, ServerEvent::MemberUnban { server: sid, user: member.id });
        let rejoin = SignedEvent::sign(sid, member.id, 3_000, ServerEvent::MemberJoin { server: sid, profile: member.clone() }, &identity_for(member.id));
        assert_eq!(state.apply_signed(&ban), Ok(true));
        assert_eq!(state.apply_signed(&rejoin), Ok(false), "still banned here");
        assert_eq!(state.apply_signed(&unban), Ok(true));
        assert!(state.server.is_current_member(member.id), "replicas agree whatever order they heard it in");
    }

    #[test]
    fn an_unban_does_not_bring_back_a_member_who_joined_before_the_ban_or_left_since() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let early = keyed_profile("early");
        let gone = keyed_profile("gone");
        assert_eq!(state.join_as_of(early.clone(), 1_000), Ok(true));
        assert_eq!(state.join_as_of(gone.clone(), 1_000), Ok(true));
        let kick = by_owner(&state, owner, 1_500, ServerEvent::MemberLeave { server: sid, user: gone.id });
        assert_eq!(state.apply_signed(&kick), Ok(true));
        for (user, at) in [(early.id, 2_000), (gone.id, 2_000)] {
            assert_eq!(state.apply_signed(&by_owner(&state, owner, at, ServerEvent::MemberBan { server: sid, user })), Ok(true));
            assert_eq!(state.apply_signed(&by_owner(&state, owner, at + 1_000, ServerEvent::MemberUnban { server: sid, user })), Ok(true));
        }
        assert!(!state.server.is_member(early.id), "they were banned after joining");
        assert!(!state.server.is_member(gone.id), "they were kicked after joining");
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

        let keyless = UserProfile { identity: None, custom_status: Some("busy".into()), ..profile.clone() };
        assert_eq!(state.apply(ServerEvent::ProfileUpdate { profile: keyless.clone() }), Ok(true));
        assert_eq!(state.identity_of(uid), Some(key));
        assert_eq!(state.profiles[&uid].custom_status.as_deref(), Some("busy"));
        assert_eq!(state.apply(ServerEvent::ProfileUpdate { profile: keyless }), Ok(false));

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
    fn only_the_author_or_a_moderator_may_delete_a_message() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let author = join(&mut state, "author");
        let member = join(&mut state, "member");
        let moderator = join_moderator(&mut state, "moderator");
        let [a, b, c, d] = ["a", "b", "c", "d"].map(|text| post(&state, channel, author, text));
        for m in [&a, &b, &c, &d] {
            create(&mut state, m).unwrap();
        }

        assert_eq!(state.apply(delete(&state, &a, member)), Ok(false));
        assert!(state.has_message(a.id), "another member cannot delete it");
        let by_moderator = proof(&state, &a, moderator, Utc::now());
        assert_eq!(state.apply(by_moderator.clone().into_event()), Ok(true));
        assert!(!state.has_message(a.id));
        assert_eq!(state.tombstones_for(a.id), vec![&by_moderator]);
        assert_eq!(state.snapshot().accepted, vec![(a.id, moderator, channel, author)]);
        assert_eq!(state.apply(by_moderator.into_event()), Ok(false), "redelivery");
        assert_eq!(create(&mut state, &a), Ok(false), "a late copy stays out");
        let backdated = proof(&state, &b, author, b.created_at - chrono::Duration::hours(1));
        assert_eq!(state.apply(backdated.into_event()), Ok(true), "the author may delete their own");
        assert!(!state.has_message(b.id));
        assert_eq!(state.apply(delete(&state, &c, owner)), Ok(true), "the owner may delete anything");

        let keyless = UserProfile::new("keyless");
        let kid = keyless.id;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: keyless }).unwrap();
        assert_eq!(state.apply(delete(&state, &d, kid)), Err(StoreError::UnknownAuthorKey(kid)));
        let forged = TombstoneProof::sign(sid, channel, d.id, author, Utc::now(), &identity_for(member));
        assert_eq!(state.apply(forged.into_event()), Err(StoreError::BadSignature));
        let unknown_channel = TombstoneProof::sign(sid, ChannelId::new(), d.id, author, Utc::now(), &identity_for(author));
        assert_eq!(state.apply(unknown_channel.into_event()), Err(StoreError::UnknownChannel));
        state.apply(ServerEvent::MemberLeave { server: sid, user: member }).unwrap();
        assert_eq!(state.apply(delete(&state, &d, member)), Ok(false), "nor once they have left");
        assert!(state.has_message(d.id));
        state.apply(ServerEvent::MemberLeave { server: sid, user: author }).unwrap();
        assert_eq!(state.apply(delete(&state, &d, author)), Ok(true), "an author who left may still delete their own");
        assert!(!state.has_message(d.id));
    }

    #[test]
    fn a_moderators_delete_still_counts_after_they_step_down() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let author = join(&mut state, "author");
        let start = Utc::now();
        let at = |s: i64| start + chrono::Duration::seconds(s);
        type StepDown = fn(ServerId, UserId, crate::ids::RoleId) -> ServerEvent;
        let step_downs: [(&str, StepDown, bool); 3] = [
            ("unassign", |server, user, role| ServerEvent::RoleUnassign { server, user, role }, false),
            ("role deleted", |server, _, role| ServerEvent::RoleDelete { server, role }, false),
            ("leave", |server, user, _| ServerEvent::MemberLeave { server, user }, true),
        ];
        for (how, step_down, by_themselves) in step_downs {
            let moderator = join_moderator(&mut state, how);
            let [held, unseen] = ["held", "unseen"].map(|text| post_at(&state, channel, author, text, at(0)));
            let later = post_at(&state, channel, author, "later", at(20 * 60));
            create(&mut state, &held).unwrap();
            let signer = if by_themselves { moderator } else { owner };
            let out = signed_event(&state, signer, at(10), step_down(sid, moderator, mods_role(&state, moderator)));
            assert_eq!(state.apply_signed(&out), Ok(true), "{how}");
            assert_eq!(state.server.stepped_down.get(&moderator), Some(&at(10).timestamp_millis()), "{how}");
            assert_eq!(state.apply(proof(&state, &held, moderator, at(5)).into_event()), Ok(true), "{how}: signed while a moderator");
            assert!(!state.has_message(held.id), "{how}");
            assert_eq!(state.apply(proof(&state, &unseen, moderator, at(5)).into_event()), Ok(true), "{how}");
            assert_eq!(create(&mut state, &unseen), Ok(true), "{how}: the proof is bound");
            assert!(!state.has_message(unseen.id), "{how}");
            create(&mut state, &later).unwrap();
            assert_eq!(state.apply(proof(&state, &later, moderator, at(11)).into_event()), Ok(false), "{how}: signed after");
            let backdated = proof(&state, &later, moderator, at(5));
            assert_eq!(state.apply(backdated.into_event()), Ok(false), "{how}: backdated past a message written later");
            assert!(state.has_message(later.id), "{how}");
            assert_eq!(reload(&state).server.stepped_down, state.server.stepped_down, "{how}, after a restart");
        }
    }

    #[test]
    fn a_late_message_comes_in_when_only_others_tried_to_delete_it() {
        let (mut state, channel, _) = fixture();
        let author = join(&mut state, "author");
        let member = join(&mut state, "member");
        let msg = post(&state, channel, author, "mine");
        assert_eq!(state.apply(delete(&state, &msg, member)), Ok(true), "kept until the message shows who wrote it");
        assert_eq!(create(&mut state, &msg), Ok(true));
        assert!(state.has_message(msg.id), "the member was not allowed to delete it");
        assert_eq!(state.apply(delete(&state, &msg, member)), Ok(false));
        assert!(state.has_message(msg.id));
    }

    #[test]
    fn a_delete_from_a_new_moderator_applies_once_their_role_arrives() {
        let (mut state, channel, _) = fixture();
        let sid = state.server.id;
        let author = join(&mut state, "author");
        let moderator = join(&mut state, "moderator");
        let [held, unseen] = ["held", "unseen"].map(|text| post(&state, channel, author, text));
        create(&mut state, &held).unwrap();
        for m in [&held, &unseen] {
            state.apply(delete(&state, m, moderator)).unwrap();
        }
        assert_eq!(create(&mut state, &unseen), Ok(true));
        assert!(state.has_message(held.id) && state.has_message(unseen.id), "their role has not arrived yet");
        let role = Role { id: RoleId::new(), name: "mods".into(), permissions: Permissions::MANAGE_MESSAGES, position: 1, ..state.server.roles[0].clone() };
        let rid = role.id;
        state.apply(ServerEvent::RoleCreate { server: sid, role }).unwrap();
        assert!(state.has_message(held.id), "a role nobody holds changes nothing");
        assert_eq!(state.apply(ServerEvent::RoleAssign { server: sid, user: moderator, role: rid }), Ok(true));
        assert!(!state.has_message(held.id) && !state.has_message(unseen.id));
        assert_eq!(state.tombstone(held.id).map(|p| p.by), Some(moderator));
    }

    #[test]
    fn early_tombstone_is_checked_against_the_late_message() {
        let (mut state, channel, _) = fixture();
        let sid = state.server.id;
        let author = join(&mut state, "author");
        let member = join_moderator(&mut state, "moderator");

        let two = post(&state, channel, author, "two");
        assert_eq!(state.apply(delete(&state, &two, member)), Ok(true));
        assert_eq!(create(&mut state, &two), Ok(true), "the proof is bound");
        assert!(!state.has_message(two.id));
        assert_eq!(create(&mut state, &two), Ok(false));

        let voice = state.server.channels[1].id;
        let three = post(&state, channel, author, "three");
        let elsewhere = TombstoneProof::sign(sid, voice, three.id, author, Utc::now(), &identity_for(author));
        assert_eq!(state.apply(elsewhere.into_event()), Ok(true));
        assert_eq!(create(&mut state, &three), Ok(true));
        assert!(state.tombstone(three.id).is_none(), "and goes once the message is in");

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
        let mut json = serde_json::to_value(&snapshot).unwrap();
        json.as_object_mut().unwrap().remove("tombstone_proofs");
        json.as_object_mut().unwrap().remove("accepted");
        let mut restored = ServerState::from_snapshot(serde_json::from_value(json).unwrap());
        assert_eq!(restored.messages(channel), std::slice::from_ref(&legacy));
        assert_eq!(create(&mut restored, &gone), Ok(false), "unproven tombstone still holds");
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
        let outsider = keyed_profile("outsider");
        let stranger = outsider.id;
        state.profiles.insert(stranger, outsider);
        for by in [banned, former, stranger] {
            let theirs = post(&state, channel, author, "theirs");
            assert_eq!(state.apply(delete(&state, &theirs, by)), Ok(true), "kept until the message shows who wrote it");
            assert_eq!(create(&mut state, &theirs), Ok(true));
            assert!(state.has_message(theirs.id), "they may not delete it");
        }
        let unknown = UserId::new();
        let other = post(&state, channel, author, "other");
        assert_eq!(state.apply(delete(&state, &other, unknown)), Err(StoreError::UnknownAuthorKey(unknown)));
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
        assert_eq!(state.apply(proof(&state, &unseen[0], author, at(0)).into_event()), Ok(false));
        let others = post(&state, channel, other, "others");
        assert_eq!(state.apply(delete(&state, &others, other)), Ok(true));
        let early = proof(&state, &one_more, author, base - chrono::Duration::seconds(1));
        assert_eq!(state.apply(early.into_event()), Ok(true));
        let last = &unseen[MAX_UNAPPLIED_PROOFS - 1];
        assert!(state.tombstone(one_more.id).is_some() && state.tombstone(last.id).is_none());
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
    fn leaving_or_a_ban_keeps_a_moderators_deletes() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let author = join(&mut state, "author");
        for how in ["leave", "ban"] {
            let member = join_moderator(&mut state, how);
            let held = post(&state, channel, author, "held");
            let unseen = post(&state, channel, author, "unseen");
            create(&mut state, &held).unwrap();
            for m in [&held, &unseen] {
                assert_eq!(state.apply(delete(&state, m, member)), Ok(true), "{how}");
            }
            let out = match how {
                "ban" => signed_event(&state, owner, Utc::now() + chrono::Duration::seconds(1), ServerEvent::MemberBan { server: sid, user: member }),
                _ => signed_event(&state, member, Utc::now() + chrono::Duration::seconds(1), ServerEvent::MemberLeave { server: sid, user: member }),
            };
            assert_eq!(state.apply_signed(&out), Ok(true), "{how}");
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
        let last = all.last().unwrap();
        assert!(state.tombstone(last.id).is_some());
        assert_eq!(create(&mut state, &all[all.len() - 2]), Ok(false));
        assert!(!state.has_message(all[all.len() - 2].id));
    }

    fn filling(msg: &Message, size: usize) -> String {
        let probe = Message { content: "x".repeat(200), ..msg.clone() };
        "x".repeat(200 + size - probe.authored_size())
    }

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
        let mut at_limit = sized(&state, channel, owner, MAX_MESSAGE_BYTES);
        at_limit.reactions.push(Reaction { emoji: ":party:".into(), users: (0..50).map(|_| UserId::new()).collect() });
        assert_eq!(create(&mut state, &at_limit), Ok(true));

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
        let mut restored = reload(&state);
        assert_eq!(restored.prune_expired(Utc::now()), 0);
        let late = post(&state, channel, owner, "late");
        assert_eq!(create(&mut restored, &late), Ok(true));
        assert!(restored.has_message(msg.id));
        assert_eq!(crate::catchup::digests(&restored, Utc::now()).len(), 1);
    }

    fn filler_member() -> Member {
        Member { user: UserId::new(), nickname: None, roles: vec![], joined_at: Utc::now(), timeout: None }
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
                mentionable: false,
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
            mentionable: false,
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

    #[test]
    fn every_announced_device_of_a_member_may_carry_their_media() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let voice = state.server.channels[1].id;
        let member = join(&mut state, "friend");
        let (desktop, tablet) = ("ab".repeat(32), "cd".repeat(32));
        state.profiles.get_mut(&member).unwrap().node = Some(desktop.clone());
        let announce = |node: &str, voice| ServerEvent::DeviceAnnounce { server: sid, user: member, node: node.into(), voice, since: None };
        assert_eq!(state.apply(announce(&tablet.to_uppercase(), None)), Ok(true));
        assert_eq!(state.apply(announce(&tablet, None)), Ok(false), "the same device, said twice");
        assert!(state.media_peer_ok(member, &tablet, voice, false));
        assert!(state.media_peer_ok(member, &desktop, voice, false), "the announced profile node still counts");
        assert!(!state.media_peer_ok(member, &"ef".repeat(32), voice, false));

        let in_voice = ServerEvent::VoiceState { user: member, channel: Some(voice), muted: false, deafened: false, since: None };
        state.apply(in_voice).unwrap();
        assert_eq!(state.voice_node(member), Some(desktop.clone()), "no device said it is in voice");
        state.apply(announce(&tablet, Some(voice))).unwrap();
        assert_eq!(state.voice_node(member), Some(tablet.clone()));
        assert!(state.media_peer_ok(member, &tablet, voice, true));

        let text = state.server.channels[0].id;
        assert_eq!(state.apply(announce(&tablet, Some(text))), Ok(false), "voice happens in voice channels");
        assert_eq!(state.apply(announce("not a node", None)), Ok(false));
        let stranger = UserId::new();
        let theirs = ServerEvent::DeviceAnnounce { server: sid, user: stranger, node: "12".repeat(32), voice: None, since: None };
        assert_eq!(state.apply(theirs), Ok(false), "strangers have no devices here");

        state.apply(ServerEvent::MemberLeave { server: sid, user: member }).unwrap();
        assert!(state.devices_of(member).is_empty());
    }

    fn two_rooms(state: &mut ServerState) -> (ChannelId, ChannelId) {
        let sid = state.server.id;
        let lounge = crate::server::Channel::new("Lounge", ChannelKind::Voice, 9);
        let lounge_id = lounge.id;
        state.apply(ServerEvent::ChannelCreate { server: sid, channel: lounge }).unwrap();
        (state.server.channels[1].id, lounge_id)
    }

    fn device(state: &ServerState, user: UserId, node: &str, voice: Option<ChannelId>, since_ms: i64) -> ServerEvent {
        ServerEvent::DeviceAnnounce { server: state.server.id, user, node: node.into(), voice, since: voice.map(|_| since_ms) }
    }

    fn sits(user: UserId, channel: Option<ChannelId>) -> ServerEvent {
        ServerEvent::VoiceState { user, channel, muted: false, deafened: false, since: None }
    }

    #[test]
    fn only_the_device_in_voice_can_end_the_seat() {
        let (mut state, _, _) = fixture();
        let (general, _) = two_rooms(&mut state);
        let member = join(&mut state, "friend");
        let (desktop, browser) = ("ab".repeat(32), "cd".repeat(32));
        state.apply(device(&state, member, &browser, None, 0)).unwrap();
        state.apply(device(&state, member, &desktop, Some(general), 1_000)).unwrap();
        assert_eq!(state.voice_states.get(&member), Some(&general), "the device in voice seats the member");
        assert_eq!(state.voice_device(member), Some(desktop.as_str()));

        assert_eq!(state.apply(sits(member, None)), Ok(false), "a leave the device in voice did not say is ignored");
        assert_eq!(state.voice_states.get(&member), Some(&general));
        state.apply(device(&state, member, &browser, None, 0)).unwrap();
        assert_eq!(state.voice_states.get(&member), Some(&general), "the other device was never in voice");

        assert_eq!(state.apply(device(&state, member, &desktop, None, 0)), Ok(true));
        assert_eq!(state.voice_states.get(&member), None, "the device in voice left, so the member left");
    }

    #[test]
    fn the_device_in_voice_decides_the_room_whatever_its_clock_says() {
        let (mut state, _, _) = fixture();
        let (general, lounge) = two_rooms(&mut state);
        let member = join(&mut state, "friend");
        let key = identity_for(member);
        let sid = state.server.id;
        let (desktop, phone) = ("ab".repeat(32), "cd".repeat(32));
        let now = Utc::now().timestamp_millis();
        let signed = |at: i64, event: ServerEvent| crate::SignedEvent::sign(sid, member, at, event, &key);

        let ahead = now + 20_000;
        state.apply_signed(&signed(ahead, device(&state, member, &desktop, Some(general), 1_000))).unwrap();
        state.apply_signed(&signed(ahead, sits(member, Some(general)))).unwrap();
        assert_eq!(state.voice_states.get(&member), Some(&general));

        state.apply_signed(&signed(now, device(&state, member, &phone, Some(lounge), 2_000))).unwrap();
        assert_eq!(state.voice_states.get(&member), Some(&lounge), "the later join wins although its clock is behind");
        let muted = ServerEvent::VoiceState { user: member, channel: Some(lounge), muted: true, deafened: false, since: None };
        assert_eq!(state.apply_signed(&signed(now + 1, muted)), Ok(true), "the new device is heard although its clock is behind");
        assert_eq!(state.voice_flags(member), (true, false));

        assert_eq!(state.apply_signed(&signed(ahead + 1, sits(member, Some(general)))), Ok(false), "a stale join from the old device");
        assert_eq!(state.apply_signed(&signed(ahead + 2, sits(member, None))), Ok(false), "a stale leave from the old device");
        assert_eq!(state.voice_states.get(&member), Some(&lounge));
        assert_eq!(state.apply_signed(&signed(ahead + 3, device(&state, member, &desktop, Some(general), 1_000))), Ok(true));
        assert_eq!(state.voice_states.get(&member), Some(&lounge), "an older join heard late does not take the seat back");
    }

    #[test]
    fn a_member_without_announced_devices_still_comes_and_goes_by_voice_state() {
        let (mut state, _, _) = fixture();
        let (general, lounge) = two_rooms(&mut state);
        let member = join(&mut state, "friend");
        assert_eq!(state.apply(sits(member, Some(general))), Ok(true));
        assert_eq!(state.apply(sits(member, Some(lounge))), Ok(true));
        assert_eq!(state.apply(sits(member, None)), Ok(true));
        assert!(state.voice_states.is_empty());
    }

    #[test]
    fn ending_voice_clears_the_seat_the_share_the_camera_and_the_device_seat() {
        let (mut state, _, _) = fixture();
        let (general, _) = two_rooms(&mut state);
        let member = join(&mut state, "friend");
        state.apply(device(&state, member, &"ab".repeat(32), Some(general), 1_000)).unwrap();
        state.apply(ServerEvent::ScreenShare { user: member, channel: general, active: true }).unwrap();
        state.apply(ServerEvent::CameraState { user: member, channel: general, active: true }).unwrap();
        assert!(state.end_voice(member));
        assert!(state.voice_states.is_empty() && state.screen_shares.is_empty() && state.cameras.is_empty());
        assert_eq!(state.voice_device(member), None);
        assert!(!state.end_voice(member));
        assert_eq!(state.apply(sits(member, Some(general))), Ok(true), "a fresh join after the seat ended is heard");
    }

    #[test]
    fn a_fresh_copy_of_the_server_keeps_who_is_in_voice_and_on_which_device() {
        let (mut old, _, owner) = fixture();
        let (general, _) = two_rooms(&mut old);
        let member = join(&mut old, "friend");
        old.apply(device(&old, member, &"ab".repeat(32), Some(general), 1_000)).unwrap();
        old.apply(ServerEvent::VoiceState { user: member, channel: Some(general), muted: true, deafened: false, since: Some(5) }).unwrap();
        old.apply(ServerEvent::ScreenShare { user: member, channel: general, active: true }).unwrap();
        old.apply(sits(owner, Some(general))).unwrap();
        let mut fresh = ServerState::new(old.server.clone());
        fresh.carry_live_voice(&old);
        assert_eq!(fresh.voice_states, old.voice_states);
        assert_eq!(fresh.voice_flags(member), (true, false));
        assert_eq!(fresh.voice_since(member), old.voice_since(member));
        assert_eq!(fresh.screen_shares.get(&member), Some(&general));
        assert_eq!(fresh.voice_device(member), Some("ab".repeat(32).as_str()));
        assert_eq!(fresh.apply(sits(member, None)), Ok(false), "the device in voice still decides");
    }

    #[test]
    fn only_the_account_key_can_announce_its_devices() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let member = join(&mut state, "friend");
        let event = ServerEvent::DeviceAnnounce { server: sid, user: member, node: "cd".repeat(32), voice: None, since: None };
        assert_eq!(state.apply_unsigned(event.clone()), Err(StoreError::Unsigned));
        let forged = crate::SignedEvent::sign(sid, owner, 1_000, event.clone(), &identity_for(owner));
        assert_eq!(state.apply_signed(&forged), Err(StoreError::NotAllowed));
        let own = crate::SignedEvent::sign(sid, member, 1_000, event, &identity_for(member));
        assert_eq!(state.apply_signed(&own), Ok(true));
        assert!(!state.holds_signed_action(&format!("device:{member}:{}", "cd".repeat(32))), "devices are live, never relayed");
    }

    fn timeout(state: &ServerState, user: UserId, by: UserId, at: DateTime<Utc>, secs: i64) -> ServerEvent {
        let until = (secs > 0).then(|| at + chrono::Duration::seconds(secs));
        ServerEvent::MemberTimeout { server: state.server.id, user, by, at, until }
    }

    #[test]
    fn a_timeout_sits_on_the_member_record_until_it_runs_out() {
        let (mut state, _, owner) = fixture();
        let user = join(&mut state, "loud");
        let now = Utc::now();
        assert_eq!(state.apply(timeout(&state, user, owner, now, 600)), Ok(true));
        let held = state.server.member(user).unwrap().timeout.unwrap();
        assert_eq!((held.since, held.until, held.by), (now, now + chrono::Duration::seconds(600), owner));
        assert!(state.server.is_timed_out(user, now));
        assert!(state.server.is_timed_out(user, now + chrono::Duration::seconds(599)));
        assert!(!state.server.is_timed_out(user, now + chrono::Duration::seconds(600)), "it expires on its own");
        assert_eq!(held.remaining_secs(now + chrono::Duration::seconds(100)), 500);
        assert_eq!(held.remaining_secs(now + chrono::Duration::seconds(700)), 0);
        assert!(state.server.timed_out_at(user, now + chrono::Duration::seconds(10)));
        assert!(!state.server.timed_out_at(user, now - chrono::Duration::seconds(10)), "before it was set");
        assert_eq!(state.server.timed_out_members(now), vec![(user, held)]);
        assert!(state.server.timed_out_members(now + chrono::Duration::hours(1)).is_empty());
        assert_eq!(state.apply(timeout(&state, user, owner, now, 600)), Ok(false), "a copy changes nothing");
        assert_eq!(state.apply(timeout(&state, user, owner, now, 0)), Ok(true), "cleared early");
        assert!(state.server.member(user).unwrap().timeout.is_none());
        assert_eq!(state.apply(timeout(&state, user, owner, now, 0)), Ok(false));
        let restored = ServerState::from_snapshot(state.snapshot());
        assert!(restored.server.member(user).is_some());
    }

    #[test]
    fn timeouts_refuse_the_owner_strangers_and_impossible_lengths() {
        let (mut state, _, owner) = fixture();
        let user = join(&mut state, "loud");
        let now = Utc::now();
        assert_eq!(state.apply(timeout(&state, owner, user, now, 60)), Ok(false));
        assert_eq!(state.apply(timeout(&state, UserId::new(), owner, now, 60)), Ok(false));
        assert_eq!(state.apply(timeout(&state, user, owner, now, -5)), Ok(false));
        let too_long = crate::server::MAX_TIMEOUT_SECS + 1;
        assert_eq!(state.apply(timeout(&state, user, owner, now, too_long)), Ok(false));
        assert!(state.server.member(user).unwrap().timeout.is_none());
        assert_eq!(state.apply(timeout(&state, user, owner, now, crate::server::MAX_TIMEOUT_SECS)), Ok(true));
        let other = ServerEvent::MemberTimeout { server: ServerId::new(), user, by: owner, at: now, until: None };
        assert_eq!(state.apply(other), Err(StoreError::WrongServer));
    }

    #[test]
    fn messages_edits_and_reactions_from_a_timed_out_member_are_dropped() {
        let (mut state, channel, owner) = fixture();
        let user = join(&mut state, "loud");
        let now = Utc::now();
        let before = post_at(&state, channel, user, "before", now - chrono::Duration::seconds(5));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: before.clone() }), Ok(true));
        state.apply(timeout(&state, user, owner, now, 300)).unwrap();
        let during = post_at(&state, channel, user, "during", now + chrono::Duration::seconds(5));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: during }), Err(StoreError::TimedOut));
        let edit_at = now + chrono::Duration::seconds(6);
        assert_eq!(state.apply(edit(&state, &before, "changed", edit_at)), Err(StoreError::TimedOut));
        assert_eq!(state.message(before.id).unwrap().content, "before");
        let react = ServerEvent::ReactionSet { channel, message: before.id, emoji: "x".into(), user, active: true };
        assert!(!crate::action::authorized(&state.server, user, &react));
        assert!(!crate::action::authorized(&state.server, user, &ServerEvent::TypingStart { channel, user }));
        let play = ServerEvent::SoundboardPlay { server: state.server.id, channel, user, name: "boop".into() };
        assert!(!crate::action::authorized(&state.server, user, &play));
        let listen = ServerEvent::VoiceState { user, channel: Some(state.server.channels[1].id), muted: false, deafened: false, since: None };
        assert!(crate::action::authorized(&state.server, user, &listen), "they can still join voice to listen");
        let owners = post_at(&state, channel, owner, "fine", now + chrono::Duration::seconds(5));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: owners }), Ok(true));
        let after = post_at(&state, channel, user, "after", now + chrono::Duration::seconds(301));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: after }), Ok(true), "the timeout ran out");
        let late_copy = post_at(&state, channel, user, "late", now + chrono::Duration::seconds(10));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: late_copy }), Err(StoreError::TimedOut), "sent during the timeout, delivered after");
        assert!(!crate::action::authorized(&state.server, user, &react), "still timed out by the clock");
        state.apply(timeout(&state, user, owner, now, 0)).unwrap();
        assert!(crate::action::authorized(&state.server, user, &react), "reacting works again once lifted");
    }

    #[test]
    fn only_members_allowed_to_time_out_may_and_never_on_themselves() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let mod_ = join(&mut state, "mod");
        let user = join(&mut state, "user");
        let role = Role {
            id: RoleId::new(),
            name: "mods".into(),
            color: None,
            permissions: Permissions::TIME_OUT_MEMBERS,
            position: 1,
            hoist: false,
            self_assignable: false,
            tag: None,
            banner: None,
            mentionable: false,
        };
        let rid = role.id;
        state.apply(ServerEvent::RoleCreate { server: sid, role }).unwrap();
        state.apply(ServerEvent::RoleAssign { server: sid, user: mod_, role: rid }).unwrap();
        let now = Utc::now();
        let by = |who: UserId, target: UserId| ServerEvent::MemberTimeout { server: sid, user: target, by: who, at: now, until: Some(now + chrono::Duration::minutes(5)) };
        assert!(crate::action::authorized(&state.server, owner, &by(owner, user)));
        assert!(crate::action::authorized(&state.server, mod_, &by(mod_, user)));
        assert!(!crate::action::authorized(&state.server, user, &by(user, mod_)), "no permission");
        assert!(!crate::action::authorized(&state.server, mod_, &by(owner, user)), "signed by someone else");
        assert!(!crate::action::authorized(&state.server, mod_, &by(mod_, mod_)), "not on oneself");
        let signed = SignedEvent::sign(sid, mod_, 1_000, by(mod_, user), &identity_for(mod_));
        assert_eq!(state.apply_signed(&signed), Ok(true));
        assert!(state.server.is_timed_out(user, now));
        let forged = SignedEvent::sign(sid, user, 2_000, timeout(&state, user, user, now, 0), &identity_for(user));
        assert_eq!(state.apply_signed(&forged), Err(StoreError::NotAllowed));
        let lifted = SignedEvent::sign(sid, owner, 3_000, timeout(&state, user, owner, now, 0), &identity_for(owner));
        assert_eq!(state.apply_signed(&lifted), Ok(true));
        assert_eq!(state.apply_signed(&signed), Ok(false), "the older timeout loses");
        assert!(!state.server.is_timed_out(user, now));
        let kinds: Vec<AuditKind> = state.audit_log().iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec![AuditKind::MemberTimeout, AuditKind::MemberTimeoutClear]);
        assert_eq!(state.audit_log()[0].detail.as_deref(), Some("5 minutes"));
        assert!(crate::action::relayable(&by(mod_, user)));
        assert!(crate::action::snapshot_omits("timeout:x"), "joiners learn timeouts from the relayed signed action");
    }
}

#[cfg(test)]
mod poll_tests {
    use super::*;
    use crate::permissions::Permissions;
    use crate::poll::{Poll, PollTally};
    use crate::test_support::{delete, fixture, join, join_moderator, post, signed};

    fn poll(multi: bool, closes_at: Option<DateTime<Utc>>) -> Poll {
        Poll {
            question: "Raid night?".into(),
            options: vec!["Friday".into(), "Saturday".into(), "Sunday".into()],
            multi,
            closes_at,
        }
    }

    fn post_poll(state: &mut ServerState, channel: ChannelId, author: UserId, poll: Poll) -> Message {
        let mut message = Message::new(channel, author, "");
        message.attachments.push(poll.into_attachment());
        let message = signed(state, message);
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: message.clone() }), Ok(true));
        message
    }

    fn vote(state: &ServerState, message: &Message, user: UserId, choices: Vec<u8>) -> ServerEvent {
        ServerEvent::PollVote { server: state.server.id, channel: message.channel, message: message.id, user, choices }
    }

    fn close(state: &ServerState, message: &Message, by: UserId) -> ServerEvent {
        ServerEvent::PollClose { server: state.server.id, channel: message.channel, message: message.id, by }
    }

    fn counts(state: &ServerState, message: &Message) -> Vec<usize> {
        state.poll_tally(message.id, Utc::now()).unwrap().counts
    }

    #[test]
    fn members_without_the_files_permission_can_still_post_a_poll() {
        let (mut state, channel, owner) = fixture();
        let member = join(&mut state, "member");
        for role in state.server.roles.iter_mut() {
            role.permissions.remove(Permissions::ATTACH_FILES);
        }
        assert!(state.server.permissions_for(owner).allows(Permissions::ATTACH_FILES), "the owner keeps everything");
        assert!(!state.server.permissions_for(member).allows(Permissions::ATTACH_FILES));
        let message = post_poll(&mut state, channel, member, poll(false, None));
        assert_eq!(state.message(message.id).unwrap().poll(), Some(poll(false, None)));
    }

    #[test]
    fn polls_need_the_send_polls_permission() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let member = join(&mut state, "member");
        let first = post_poll(&mut state, channel, member, poll(false, None));
        let mut everyone = state.server.roles[0].clone();
        everyone.permissions.insert(Permissions::NO_SEND_POLLS);
        state.apply(ServerEvent::RoleUpdate { server: sid, role: everyone }).unwrap();
        let mut blocked = Message::new(channel, member, "");
        blocked.attachments.push(poll(true, None).into_attachment());
        let blocked = signed(&state, blocked);
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: blocked }), Err(StoreError::NoPolls));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: first.clone() }), Ok(false), "held polls stay");
        assert_eq!(state.apply(vote(&state, &first, member, vec![0])), Ok(true), "and can still be voted on");
        let words = post(&state, channel, member, "no poll here");
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: words }), Ok(true));
        post_poll(&mut state, channel, owner, poll(false, None));
        let pollsters = crate::server::Role {
            id: crate::ids::RoleId::new(),
            name: "pollsters".into(),
            color: None,
            permissions: Permissions::NONE,
            position: 1,
            hoist: false,
            self_assignable: false,
            mentionable: false,
            tag: None,
            banner: None,
        };
        let rid = pollsters.id;
        state.apply(ServerEvent::RoleCreate { server: sid, role: pollsters }).unwrap();
        state.apply(ServerEvent::RoleAssign { server: sid, user: member, role: rid }).unwrap();
        post_poll(&mut state, channel, member, poll(true, None));
    }

    #[test]
    fn latest_vote_per_member_wins_and_repeats_change_nothing() {
        let (mut state, channel, owner) = fixture();
        let member = join(&mut state, "member");
        let message = post_poll(&mut state, channel, owner, poll(false, None));
        assert_eq!(state.apply(vote(&state, &message, owner, vec![0])), Ok(true));
        assert_eq!(state.apply(vote(&state, &message, owner, vec![0])), Ok(false));
        assert_eq!(state.apply(vote(&state, &message, member, vec![2])), Ok(true));
        assert_eq!(counts(&state, &message), vec![1, 0, 1]);
        assert_eq!(state.apply(vote(&state, &message, owner, vec![1])), Ok(true));
        assert_eq!(counts(&state, &message), vec![0, 1, 1]);
        assert_eq!(state.poll_choices(message.id, owner), &[1]);
        assert_eq!(state.poll_tally(message.id, Utc::now()).unwrap().voters, 2);
    }

    #[test]
    fn empty_choices_retract_a_vote() {
        let (mut state, channel, owner) = fixture();
        let message = post_poll(&mut state, channel, owner, poll(false, None));
        assert_eq!(state.apply(vote(&state, &message, owner, vec![])), Ok(false), "nothing to retract");
        state.apply(vote(&state, &message, owner, vec![2])).unwrap();
        assert_eq!(state.apply(vote(&state, &message, owner, vec![])), Ok(true));
        assert_eq!(state.poll_choices(message.id, owner), &[] as &[u8]);
        assert_eq!(state.poll_tally(message.id, Utc::now()).unwrap(), PollTally { counts: vec![0, 0, 0], voters: 0, closed: false });
        assert_eq!(state.ballots().count(), 0);
    }

    #[test]
    fn single_choice_polls_take_one_answer_and_multi_polls_several() {
        let (mut state, channel, owner) = fixture();
        let single = post_poll(&mut state, channel, owner, poll(false, None));
        assert_eq!(state.apply(vote(&state, &single, owner, vec![0, 1])), Ok(false));
        assert_eq!(state.apply(vote(&state, &single, owner, vec![7])), Ok(false), "out of range");
        assert_eq!(counts(&state, &single), vec![0, 0, 0]);
        let multi = post_poll(&mut state, channel, owner, poll(true, None));
        assert_eq!(state.apply(vote(&state, &multi, owner, vec![2, 0, 2])), Ok(true));
        assert_eq!(counts(&state, &multi), vec![1, 0, 1]);
        assert_eq!(state.poll_choices(multi.id, owner), &[0, 2]);
        assert_eq!(state.apply(vote(&state, &multi, owner, vec![0, 2])), Ok(false), "same choices in another order");
    }

    #[test]
    fn votes_need_a_poll_and_a_known_message() {
        let (mut state, channel, owner) = fixture();
        let plain = post(&state, channel, owner, "no poll here");
        state.apply(ServerEvent::MessageCreate { message: plain.clone() }).unwrap();
        assert_eq!(state.apply(vote(&state, &plain, owner, vec![0])), Ok(false));
        assert_eq!(state.poll_tally(plain.id, Utc::now()), None);
        let missing = Message::new(channel, owner, "");
        assert_eq!(state.apply(vote(&state, &missing, owner, vec![0])), Err(StoreError::UnknownMessage));
        let elsewhere = Message { channel: ChannelId::new(), ..plain };
        assert_eq!(state.apply(vote(&state, &elsewhere, owner, vec![0])), Err(StoreError::UnknownChannel));
        let other_server = ServerEvent::PollVote { server: ServerId::new(), channel, message: missing.id, user: owner, choices: vec![] };
        assert_eq!(state.apply(other_server), Err(StoreError::WrongServer));
    }

    #[test]
    fn votes_stop_at_the_closing_time() {
        let (mut state, channel, owner) = fixture();
        let past = Utc::now() - chrono::Duration::minutes(1);
        let ended = post_poll(&mut state, channel, owner, poll(false, Some(past)));
        assert_eq!(state.apply(vote(&state, &ended, owner, vec![0])), Ok(false));
        assert!(state.poll_closed(ended.id, Utc::now()));
        let later = Utc::now() + chrono::Duration::hours(1);
        let open = post_poll(&mut state, channel, owner, poll(false, Some(later)));
        assert_eq!(state.apply(vote(&state, &open, owner, vec![0])), Ok(true));
        assert!(!state.poll_closed(open.id, Utc::now()));
        assert!(state.poll_closed(open.id, later));
        assert!(state.poll_tally(open.id, later).unwrap().closed);
    }

    #[test]
    fn the_author_or_a_moderator_closes_a_poll_early() {
        let (mut state, channel, owner) = fixture();
        let member = join(&mut state, "member");
        let other = join(&mut state, "other");
        let moderator = join_moderator(&mut state, "mod");
        let message = post_poll(&mut state, channel, member, poll(true, None));
        state.apply(vote(&state, &message, other, vec![1])).unwrap();
        assert_eq!(state.apply(close(&state, &message, other)), Err(StoreError::NotAllowed));
        assert!(!state.poll_closed(message.id, Utc::now()));
        assert_eq!(state.apply(close(&state, &message, member)), Ok(true));
        assert_eq!(state.apply(close(&state, &message, member)), Ok(false));
        assert!(state.poll_closed(message.id, Utc::now()));
        assert_eq!(state.apply(vote(&state, &message, owner, vec![0])), Ok(false), "closed polls take no votes");
        assert_eq!(counts(&state, &message), vec![0, 1, 0]);
        let second = post_poll(&mut state, channel, member, poll(true, None));
        assert_eq!(state.apply(close(&state, &second, moderator)), Ok(true));
        let third = post_poll(&mut state, channel, member, poll(true, None));
        assert_eq!(state.apply(close(&state, &third, owner)), Ok(true), "the owner moderates");
        let plain = post(&state, channel, member, "text");
        state.apply(ServerEvent::MessageCreate { message: plain.clone() }).unwrap();
        assert_eq!(state.apply(close(&state, &plain, member)), Ok(false));
    }

    #[test]
    fn closing_by_a_moderator_lands_in_the_audit_log_but_not_by_the_author() {
        let (mut state, channel, _owner) = fixture();
        let member = join(&mut state, "member");
        let moderator = join_moderator(&mut state, "mod");
        let sid = state.server.id;
        let first = post_poll(&mut state, channel, member, poll(false, None));
        let second = post_poll(&mut state, channel, member, poll(false, None));
        let sign = |by: UserId, event: ServerEvent, at: i64| {
            SignedEvent::sign(sid, by, at, event, &crate::test_support::identity_for(by))
        };
        let own = sign(member, close(&state, &first, member), 1_000);
        assert_eq!(state.apply_signed(&own), Ok(true));
        assert!(state.audit_log().is_empty());
        let moderated = sign(moderator, close(&state, &second, moderator), 2_000);
        assert_eq!(state.apply_signed(&moderated), Ok(true));
        let entry = state.audit_log().last().unwrap();
        assert_eq!(entry.kind, AuditKind::PollClose);
        assert_eq!(entry.target, Some(second.id.to_string()));
        assert_eq!(entry.by, moderator);
    }

    #[test]
    fn signed_votes_are_ordered_by_time_and_relayed_to_late_peers() {
        let (mut state, channel, owner) = fixture();
        let member = join(&mut state, "member");
        let sid = state.server.id;
        let message = post_poll(&mut state, channel, owner, poll(false, None));
        let identity = crate::test_support::identity_for(member);
        let at = |state: &ServerState, choices: Vec<u8>, at: i64| {
            SignedEvent::sign(sid, member, at, vote(state, &message, member, choices), &identity)
        };
        let second = at(&state, vec![1], 2_000);
        let first = at(&state, vec![0], 1_000);
        assert_eq!(state.apply_signed(&second), Ok(true));
        assert_eq!(state.apply_signed(&first), Ok(false), "an older vote never replaces a newer one");
        assert_eq!(state.poll_choices(message.id, member), &[1]);
        assert_eq!(state.apply_signed(&second), Ok(false));
        let relayed = state.signed_actions();
        assert!(relayed.iter().any(|s| s.event == second.event), "late peers hear the vote");
        let stranger = UserId::new();
        let unsigned = vote(&state, &message, stranger, vec![0]);
        assert_eq!(state.apply_unsigned(unsigned), Err(StoreError::Unsigned));
    }

    #[test]
    fn ballots_survive_snapshots_and_move_to_a_rebuilt_state() {
        let (mut state, channel, owner) = fixture();
        let member = join(&mut state, "member");
        let message = post_poll(&mut state, channel, owner, poll(true, None));
        let gone = post_poll(&mut state, channel, owner, poll(true, None));
        state.apply(vote(&state, &message, owner, vec![0, 2])).unwrap();
        state.apply(vote(&state, &message, member, vec![2])).unwrap();
        state.apply(vote(&state, &gone, member, vec![1])).unwrap();
        state.apply(close(&state, &gone, owner)).unwrap();
        state.apply(delete(&state, &gone, owner)).unwrap();
        let snapshot = state.snapshot();
        assert_eq!(snapshot.ballots.len(), 2, "deleted polls leave no ballots behind");
        assert!(snapshot.closed_polls.is_empty());
        let json = serde_json::to_string(&snapshot).unwrap();
        let restored = ServerState::from_snapshot(serde_json::from_str(&json).unwrap());
        assert_eq!(restored.poll_tally(message.id, Utc::now()).unwrap().counts, vec![1, 0, 2]);
        assert_eq!(restored.poll_choices(message.id, owner), &[0, 2]);
        let mut rebuilt = ServerState::new(state.server.clone());
        rebuilt.profiles = state.profiles.clone();
        rebuilt.apply(ServerEvent::MessageCreate { message: message.clone() }).unwrap();
        rebuilt.apply(vote(&rebuilt, &message, owner, vec![1])).unwrap();
        rebuilt.carry_polls(&state);
        assert_eq!(rebuilt.poll_choices(message.id, owner), &[1], "fresher local votes stay");
        assert_eq!(rebuilt.poll_choices(message.id, member), &[2]);
        state.apply(close(&state, &message, owner)).unwrap();
        rebuilt.carry_polls(&state);
        assert!(rebuilt.poll_closed(message.id, Utc::now()));
    }

    #[test]
    fn stage_speakers_are_listed_members_and_hosts_and_everyone_else_is_audience() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let stage = crate::server::Channel::new("Town Hall", ChannelKind::Stage, 9);
        let stage_id = stage.id;
        state.apply(ServerEvent::ChannelCreate { server: sid, channel: stage }).unwrap();
        let guest = join(&mut state, "guest");
        let host = crate::test_support::join_moderator(&mut state, "host");
        let set = |user, speaker| ServerEvent::StageSet { server: sid, channel: stage_id, user, speaker, by: owner };
        assert!(state.server.is_stage_speaker(stage_id, owner), "the owner hosts every stage");
        assert!(state.server.in_audience(stage_id, guest));
        assert!(state.server.in_audience(stage_id, host), "managing messages is not hosting a stage");
        assert_eq!(state.apply(set(guest, true)), Ok(true));
        assert_eq!(state.apply(set(guest, true)), Ok(false));
        assert!(state.server.is_stage_speaker(stage_id, guest));
        assert_eq!(state.apply(set(guest, false)), Ok(true));
        assert_eq!(state.apply(set(guest, false)), Ok(false));
        assert!(state.server.in_audience(stage_id, guest));
        assert_eq!(state.apply(set(UserId::new(), true)), Ok(false), "strangers cannot be speakers");
        let voice = state.server.channels[1].id;
        let plain = ServerEvent::StageSet { server: sid, channel: voice, user: guest, speaker: true, by: owner };
        assert_eq!(state.apply(plain), Ok(false), "only a stage keeps a speakers list");
        assert!(!state.server.in_audience(voice, guest), "a voice channel has no audience");
        let gone = ServerEvent::StageSet { server: sid, channel: ChannelId::new(), user: guest, speaker: true, by: owner };
        assert_eq!(state.apply(gone), Err(StoreError::UnknownChannel));
    }

    #[test]
    fn stage_speakers_survive_a_snapshot_and_stop_at_the_cap() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let stage = crate::server::Channel::new("Town Hall", ChannelKind::Stage, 9);
        let stage_id = stage.id;
        state.apply(ServerEvent::ChannelCreate { server: sid, channel: stage }).unwrap();
        for i in 0..crate::server::MAX_STAGE_SPEAKERS {
            let user = join(&mut state, &format!("s{i}"));
            assert_eq!(state.apply(ServerEvent::StageSet { server: sid, channel: stage_id, user, speaker: true, by: owner }), Ok(true));
        }
        let extra = join(&mut state, "extra");
        let event = ServerEvent::StageSet { server: sid, channel: stage_id, user: extra, speaker: true, by: owner };
        assert_eq!(state.apply(event), Err(StoreError::Limit));
        let back = ServerState::from_snapshot(state.snapshot());
        assert_eq!(back.server.channel(stage_id).unwrap().speakers.len(), crate::server::MAX_STAGE_SPEAKERS);
    }

    #[test]
    fn hands_go_up_only_from_the_audience_and_come_down_on_leaving_or_promotion() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let stage = crate::server::Channel::new("Town Hall", ChannelKind::Stage, 9);
        let stage_id = stage.id;
        state.apply(ServerEvent::ChannelCreate { server: sid, channel: stage }).unwrap();
        let guest = join(&mut state, "guest");
        let hand = |user, raised| ServerEvent::HandRaise { server: sid, channel: stage_id, user, raised };
        let sit = |user, channel| ServerEvent::VoiceState { user, channel, muted: false, deafened: false, since: None };
        assert_eq!(state.apply(hand(guest, true)), Ok(false), "not in the room yet");
        state.apply(sit(guest, Some(stage_id))).unwrap();
        assert_eq!(state.apply(hand(guest, true)), Ok(true));
        assert_eq!(state.apply(hand(guest, true)), Ok(false));
        assert_eq!(state.hands_in(stage_id), vec![guest]);
        assert_eq!(state.hand_of(guest), Some(stage_id));
        assert_eq!(state.apply(hand(guest, false)), Ok(true));
        assert_eq!(state.apply(hand(guest, false)), Ok(false));
        assert!(state.hands_in(stage_id).is_empty());

        state.apply(hand(guest, true)).unwrap();
        state.apply(ServerEvent::StageSet { server: sid, channel: stage_id, user: guest, speaker: true, by: owner }).unwrap();
        assert!(state.hands_in(stage_id).is_empty(), "a new speaker no longer asks");
        assert_eq!(state.apply(hand(guest, true)), Ok(false), "speakers do not raise hands");
        state.apply(ServerEvent::StageSet { server: sid, channel: stage_id, user: guest, speaker: false, by: owner }).unwrap();

        state.apply(hand(guest, true)).unwrap();
        state.apply(sit(guest, None)).unwrap();
        assert!(state.hands_in(stage_id).is_empty(), "leaving lowers the hand");

        state.apply(sit(guest, Some(stage_id))).unwrap();
        state.apply(hand(guest, true)).unwrap();
        let voice = state.server.channels[1].id;
        state.apply(sit(guest, Some(voice))).unwrap();
        assert!(state.hands_in(stage_id).is_empty(), "moving rooms lowers the hand");

        state.apply(sit(owner, Some(stage_id))).unwrap();
        assert_eq!(state.apply(hand(owner, true)), Ok(false), "hosts already speak");
        assert!(state.snapshot().server.channel(stage_id).unwrap().speakers.is_empty());
        let unknown = ServerEvent::HandRaise { server: sid, channel: ChannelId::new(), user: guest, raised: true };
        assert_eq!(state.apply(unknown), Err(StoreError::UnknownChannel));
    }

    #[test]
    fn stage_channels_take_a_user_limit_and_voice_moves_like_voice_channels() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let stage = crate::server::Channel::new("Town Hall", ChannelKind::Stage, 9);
        let stage_id = stage.id;
        state.apply(ServerEvent::ChannelCreate { server: sid, channel: stage }).unwrap();
        assert_eq!(state.apply(ServerEvent::ChannelLimit { server: sid, channel: stage_id, user_limit: 5 }), Ok(true));
        let guest = join(&mut state, "guest");
        let voice = state.server.channels[1].id;
        state.apply(ServerEvent::VoiceState { user: guest, channel: Some(voice), muted: false, deafened: false, since: None }).unwrap();
        assert_eq!(state.apply(ServerEvent::VoiceMove { server: sid, user: guest, channel: Some(stage_id) }), Ok(true));
        assert_eq!(state.apply(ServerEvent::AfkChannelSet { server: sid, channel: Some(stage_id) }), Ok(false), "a stage is no AFK room");
    }
}
