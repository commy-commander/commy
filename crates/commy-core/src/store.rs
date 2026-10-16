use crate::event::ServerEvent;
use crate::identity::{IdentityKey, Signature};
use crate::ids::{ChannelId, MessageId, RoleId, ServerId, UserId};
use crate::message::{
    Message, Reaction, TombstoneProof, MAX_MESSAGE_BYTES, MAX_RELAYED_REACTIONS, MAX_RELAYED_REACTORS,
};
use crate::permissions::Permissions;
use crate::server::{ChannelKind, Member, Role, Server};
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
    #[error("not allowed")]
    NotAllowed,
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
}

/// Days a delete proof is kept, well past the catch-up window.
pub const TOMBSTONE_KEEP_DAYS: i64 = 60;

/// Most delete proofs kept for one message from signers who are neither
/// a moderator nor its author, the earliest first. Those two are never
/// pushed out.
pub const MAX_OTHER_PROOFS: usize = 8;

/// Most delete proofs kept from one signer that are not applied here:
/// for messages never seen here, or held here but not theirs to delete
/// (yet). Every replica keeps the same best ranked ones, the earliest
/// first: a better one pushes out the signer's worst, a worse one is
/// refused. Even a moderator clearing out a raid this replica missed
/// stays below it within the days proofs are kept.
pub const MAX_UNAPPLIED_PROOFS: usize = 2000;

/// How far ahead of our clock a signed time may be, in ms.
pub const MAX_AHEAD_MS: i64 = 86_400_000;

/// Days a role's removal from a member, and a member's leave, are
/// remembered, well past the 30 days members re-announce such changes
/// for.
pub const ROLE_MARK_KEEP_DAYS: i64 = 60;

/// The latest grant (`held`) or removal of a role for a member, kept so
/// that an older change arriving late cannot undo it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleMark {
    pub user: UserId,
    pub role: RoleId,
    pub held: bool,
    /// When it was made, in ms since the epoch.
    pub at: i64,
    /// For a removal: when the grant it ended was made, if known
    /// ([`i64::MIN`] for a role held from before marks were kept), so a
    /// delete is judged by the roles its signer held when signing it.
    #[serde(default)]
    pub since: Option<i64>,
}

impl RoleMark {
    /// Whether this change replaces `other`: it is later, or as late and
    /// a removal, so every replica keeps the same one.
    fn beats(&self, other: &RoleMark) -> bool {
        (self.at, !self.held) > (other.at, !other.held)
    }

    /// The event that makes this change.
    pub fn event(&self, server: ServerId) -> ServerEvent {
        let RoleMark { user, role, at, since, .. } = *self;
        if self.held {
            ServerEvent::RoleAssign { server, user, role, at }
        } else {
            ServerEvent::RoleUnassign { server, user, role, at, since }
        }
    }
}

/// Where a user stands in a server after a [`MemberMark`]. Of two
/// changes made at once, the one further out wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Membership {
    Joined,
    Left,
    Banned,
}

/// The latest join, leave (or kick) or ban of a user, kept so that an
/// older change arriving late cannot undo it: a stale join cannot bring
/// back someone kicked since, and a replica that missed a join learns
/// it when it is announced again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberMark {
    pub user: UserId,
    pub state: Membership,
    /// When it was made, in ms since the epoch.
    pub at: i64,
    /// For a leave or ban: when the membership it ended began, if known
    /// ([`i64::MIN`] for one from before marks were kept).
    #[serde(default)]
    pub since: Option<i64>,
}

impl MemberMark {
    /// Whether this change replaces `other`: it is later, or as late and
    /// further out, so every replica keeps the same one.
    fn beats(&self, other: &MemberMark) -> bool {
        (self.at, self.state) > (other.at, other.state)
    }
}

/// Where an item of [`ServerState::changes_since`] stands in its order,
/// for paging through it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChangeKey {
    rank: u8,
    /// The time, reversed so the newest come first.
    newest: i64,
    user: Option<UserId>,
    role: Option<RoleId>,
}

/// What is known here of who held which role, and who was a member,
/// when: enough to judge a delete by its signer's standing at the time
/// it was signed. A role's permissions are taken as they are now; only
/// who held it when is kept.
#[derive(Clone, Copy)]
struct History<'a> {
    server: &'a Server,
    roles: &'a HashMap<(UserId, RoleId), RoleMark>,
    members: &'a HashMap<UserId, MemberMark>,
}

impl History<'_> {
    /// Whether `user` was a member at `at` (ms). A member without a mark
    /// counts as one since ever.
    fn member_at(&self, user: UserId, at: i64) -> bool {
        match self.members.get(&user) {
            Some(m) if m.state == Membership::Joined => m.at <= at,
            Some(m) => within(m.since, m.at, at),
            None => self.server.is_member(user),
        }
    }

    /// Whether `user` held `role`, not the everyone role, at `at`. A role
    /// held without a mark is held since ever.
    fn held_at(&self, user: UserId, role: RoleId, at: i64) -> bool {
        match self.roles.get(&(user, role)) {
            Some(m) if m.held => m.at <= at,
            Some(m) => within(m.since, m.at, at),
            None => {
                self.server.member(user).is_some_and(|m| m.roles.contains(&role)) && self.member_at(user, at)
            }
        }
    }

    /// Whether `user` held Manage Messages at `at`: the owner always,
    /// anyone else through the roles they held then.
    fn moderator_at(&self, user: UserId, at: DateTime<Utc>) -> bool {
        if user == self.server.owner {
            return true;
        }
        let at = at.timestamp_millis();
        let held = |r: &&Role| {
            if r.position == 0 {
                self.member_at(user, at)
            } else {
                self.held_at(user, r.id, at)
            }
        };
        let perms = self.server.roles.iter().filter(held).fold(Permissions::NONE, |acc, r| acc | r.permissions);
        perms.allows(Permissions::MANAGE_MESSAGES)
    }

    /// Whether `proof` deletes a message by `author` made at `created_at`:
    /// signed by its author or by a moderator at the time, and not dated
    /// before the message.
    fn may_delete(&self, proof: &TombstoneProof, author: UserId, created_at: DateTime<Utc>) -> bool {
        proof.at.timestamp_millis() >= created_at.timestamp_millis()
            && (proof.by == author || self.moderator_at(proof.by, proof.at))
    }

    /// Whether `proof` may be kept for a message not held here: the id's
    /// author signed it, or someone who held Manage Messages then.
    fn may_delete_unseen(&self, proof: &TombstoneProof) -> bool {
        proof.message.is_bound_to(proof.by) || self.moderator_at(proof.by, proof.at)
    }
}

/// Whether `at` falls in a stretch that began at `since` and ended at
/// `until`.
fn within(since: Option<i64>, until: i64, at: i64) -> bool {
    since.is_some_and(|s| s <= at) && at < until
}

/// A delete proof as held here. `accepted_for` is the author of the
/// message this peer checked the proof against when it applied it. Such
/// a proof keeps that message out even if the deleter later loses the
/// right to delete it. A proof without it is either for a message not
/// held here, or for one held here that its signer may not delete (yet),
/// see [`ServerState::settle_proofs`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredProof {
    proof: TombstoneProof,
    accepted_for: Option<UserId>,
}

/// Where a proof not applied here ranks among its signer's, best first:
/// the earliest, then the lowest signature. Message and channel find it.
type UnappliedKey = (DateTime<Utc>, [u8; 64], MessageId, ChannelId);

/// Replicated state of one server, held in memory by every member.
/// Events are idempotent where possible so that gossip redelivery
/// does not corrupt state.
#[derive(Debug, Clone)]
pub struct ServerState {
    pub server: Server,
    pub profiles: HashMap<UserId, UserProfile>,
    messages: HashMap<ChannelId, Vec<Message>>,
    /// user -> voice channel they are connected to.
    pub voice_states: HashMap<UserId, ChannelId>,
    /// user -> voice channel they are sharing their screen in.
    pub screen_shares: HashMap<UserId, ChannelId>,
    /// user -> voice channel they have their camera on in.
    pub cameras: HashMap<UserId, ChannelId>,
    /// Ids of messages that stay deleted for good, so a late copy cannot
    /// bring one back: deleted before deletes were signed, or whose
    /// accepted delete proof was pruned.
    deleted_messages: HashSet<MessageId>,
    /// Signed deletes by message id, at most one per deleter and channel.
    /// They keep late copies out; catch-up relays those valid here, see
    /// [`Self::tombstones_since`].
    tombstones: HashMap<MessageId, Vec<StoredProof>>,
    /// Per signer, their proofs not applied here, see
    /// [`MAX_UNAPPLIED_PROOFS`]. Built again when None, which every change
    /// to the proofs other than [`Self::store_proof`] sets.
    unapplied: Option<HashMap<UserId, BTreeSet<UnappliedKey>>>,
    /// The latest change to each role a member holds or held. Roles held
    /// from before these were recorded have none.
    role_marks: HashMap<(UserId, RoleId), RoleMark>,
    /// The latest join, leave or ban of each user. Members from before
    /// these were recorded have none.
    member_marks: HashMap<UserId, MemberMark>,
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
    pub role_marks: Vec<RoleMark>,
    #[serde(default)]
    pub member_marks: Vec<MemberMark>,
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
            voice_states: HashMap::new(),
            screen_shares: HashMap::new(),
            cameras: HashMap::new(),
            deleted_messages: Default::default(),
            tombstones: HashMap::new(),
            unapplied: None,
            role_marks: HashMap::new(),
            member_marks: HashMap::new(),
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
            role_marks: self.role_marks.values().copied().collect(),
            member_marks: self.member_marks.values().copied().collect(),
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
        state.role_marks = snapshot.role_marks.into_iter().map(|m| ((m.user, m.role), m)).collect();
        state.member_marks = snapshot.member_marks.into_iter().map(|m| (m.user, m)).collect();
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
        for (channel, list) in snapshot.messages {
            // Only keep channels that still exist on the server.
            if state.messages.contains_key(&channel) {
                state.messages.insert(channel, list);
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
    /// stays deleted. Removals of roles and leaves older than
    /// [`ROLE_MARK_KEEP_DAYS`] are forgotten.
    pub fn prune_expired(&mut self, now: DateTime<Utc>) -> usize {
        let marks_from = (now - chrono::Duration::days(ROLE_MARK_KEEP_DAYS)).timestamp_millis();
        self.role_marks.retain(|_, m| m.held || m.at >= marks_from);
        self.member_marks.retain(|_, m| m.state != Membership::Left || m.at >= marks_from);
        let mut gone = Vec::new();
        for channel in &self.server.channels {
            let Some(cutoff) = retention_cutoff(channel, now) else { continue };
            if let Some(list) = self.messages.get_mut(&channel.id) {
                list.retain(|m| {
                    let keep = m.created_at >= cutoff;
                    if !keep {
                        gone.push(m.id);
                    }
                    keep
                });
            }
        }
        let removed = gone.len();
        self.drop_unapplied(gone);
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

    /// The latest change to whether `user` holds `role`, if one is known.
    pub fn role_mark(&self, user: UserId, role: RoleId) -> Option<&RoleMark> {
        self.role_marks.get(&(user, role))
    }

    /// Every such change known here, in no particular order.
    pub fn role_marks(&self) -> impl Iterator<Item = &RoleMark> {
        self.role_marks.values()
    }

    /// The event that gives (`held`) or takes `role` from `user` at `now`,
    /// dated after the change it replaces and not before the user joined.
    pub fn role_change(&self, user: UserId, role: RoleId, held: bool, now: DateTime<Utc>) -> ServerEvent {
        let at = crate::server::change_time(self.role_mark(user, role).map_or(0, |m| m.at), now);
        let at = at.max(self.joined_at(user).unwrap_or(i64::MIN));
        let since = if held { None } else { self.removed_since(user, role) };
        RoleMark { user, role, held, at, since }.event(self.server.id)
    }

    /// When the grant of `role` that a removal now would end was made:
    /// [`i64::MIN`] for one held from before marks were kept, None when
    /// `user` does not hold it.
    fn removed_since(&self, user: UserId, role: RoleId) -> Option<i64> {
        match self.role_mark(user, role) {
            Some(m) => m.held.then_some(m.at),
            None => self.server.member(user).is_some_and(|m| m.roles.contains(&role)).then_some(i64::MIN),
        }
    }

    /// The latest join, leave or ban of `user` known here.
    pub fn member_mark(&self, user: UserId) -> Option<&MemberMark> {
        self.member_marks.get(&user)
    }

    /// Every such change known here, in no particular order.
    pub fn member_marks(&self) -> impl Iterator<Item = &MemberMark> {
        self.member_marks.values()
    }

    /// When `user` joined, if that is the latest change known to their
    /// membership.
    pub fn joined_at(&self, user: UserId) -> Option<i64> {
        self.member_mark(user).filter(|m| m.state == Membership::Joined).map(|m| m.at)
    }

    /// The time for a join, leave, ban or unban of `user` made at `now`:
    /// after the latest one known, so it wins even when our clock is
    /// behind.
    pub fn member_change_time(&self, user: UserId, now: DateTime<Utc>) -> i64 {
        crate::server::change_time(self.member_mark(user).map_or(0, |m| m.at), now)
    }

    /// The role definitions, role marks and membership marks made at or
    /// after `since` (ms since the epoch), as events for members to
    /// announce again, since gossip has no backfill. Each comes with its
    /// key, in key order, which is the order of priority: role
    /// definitions first (a grant of a role not known yet is ignored),
    /// then marks of roles that carry permissions, then joins, leaves and
    /// bans (a join with the member's profile; one whose profile is not
    /// known here is left out), then marks of the other roles; within
    /// each the newest first. The order is stable, so a caller can page
    /// through it: pass the key of the last item sent as `after` to get
    /// the ones that follow. Who may announce what is the caller's to
    /// decide. Not included: role deletes, which carry no time. An unban
    /// counts as a leave and goes out as one, which lifts no ban where it
    /// arrives. A join replaces the profile of someone already a member
    /// where it arrives, as their own join does. A grant to someone the
    /// receiver first learns of from the same list comes before their
    /// join there and applies when announced again.
    pub fn changes_since(&self, since: i64, after: Option<ChangeKey>) -> Vec<(ChangeKey, ServerEvent)> {
        let server = self.server.id;
        let key = |rank, at: i64, user, role| ChangeKey { rank, newest: !at, user, role };
        let mut out = Vec::new();
        for role in self.server.roles.iter().filter(|r| r.updated_at >= since) {
            let event = ServerEvent::RoleCreate { server, role: role.clone() };
            out.push((key(0, role.updated_at, None, Some(role.id)), event));
        }
        for mark in self.role_marks.values().filter(|m| m.at >= since) {
            let carries = self.server.roles.iter().any(|r| r.id == mark.role && r.permissions != Permissions::NONE);
            let rank = if carries { 1 } else { 3 };
            out.push((key(rank, mark.at, Some(mark.user), Some(mark.role)), mark.event(server)));
        }
        for mark in self.member_marks.values().filter(|m| m.at >= since) {
            let MemberMark { user, at, .. } = *mark;
            let event = match mark.state {
                Membership::Joined => match self.profiles.get(&user) {
                    Some(profile) => ServerEvent::MemberJoin { server, profile: profile.clone(), at },
                    None => continue,
                },
                Membership::Left => ServerEvent::MemberLeave { server, user, at },
                Membership::Banned => ServerEvent::MemberBan { server, user, at },
            };
            out.push((key(2, at, Some(user), None), event));
        }
        out.retain(|(k, _)| after.is_none_or(|after| *k > after));
        out.sort_by_key(|(k, _)| *k);
        out
    }

    fn history(&self) -> History<'_> {
        History { server: &self.server, roles: &self.role_marks, members: &self.member_marks }
    }

    /// Messages in `channel` created at or after `since`, oldest first.
    pub fn messages_since(&self, channel: ChannelId, since: DateTime<Utc>) -> impl Iterator<Item = &Message> {
        self.messages(channel).iter().filter(move |m| m.created_at >= since)
    }

    /// Looks a message up by id in any channel.
    pub fn message(&self, id: MessageId) -> Option<&Message> {
        self.messages.values().flat_map(|list| list.iter()).find(|m| m.id == id)
    }

    pub fn has_message(&self, id: MessageId) -> bool {
        self.message(id).is_some()
    }

    /// Best delete proof stored for a message id, if any. See
    /// [`Self::tombstones_for`] for the order.
    pub fn tombstone(&self, id: MessageId) -> Option<&TombstoneProof> {
        let list = self.tombstones.get(&id)?;
        let history = self.history();
        list.iter().min_by_key(|s| proof_rank(&history, s)).map(|s| &s.proof)
    }

    /// Every delete proof stored for a message id, one per deleter and
    /// channel, best first: accepted here, then a moderator's (at the
    /// time), then the earliest, then the lowest signature. Some may not
    /// be valid here, see [`Self::valid_tombstones_for`].
    pub fn tombstones_for(&self, id: MessageId) -> Vec<&TombstoneProof> {
        let history = self.history();
        let mut list: Vec<&StoredProof> = self.tombstones.get(&id).into_iter().flatten().collect();
        list.sort_by_key(|s| proof_rank(&history, s));
        list.into_iter().map(|s| &s.proof).collect()
    }

    /// Delete proofs valid here made at or after `since`, in no particular
    /// order: each removed a message here or is for a message not held
    /// here, and a peer lacking the message would take it, as it is
    /// signed by the author its id is bound to or by someone who held
    /// Manage Messages when signing it. So deletes of messages saved by
    /// older versions, whose ids are bound to nobody, stay out unless a
    /// moderator made them. Catch-up lists and relays only these; the
    /// others stay here until they apply, see [`Self::settle_proofs`]. A
    /// message deleted by several people has one proof for each.
    pub fn tombstones_since(&self, since: DateTime<Utc>) -> impl Iterator<Item = &TombstoneProof> {
        let held = self.held_with_proofs();
        self.stored_proofs()
            .filter(move |s| s.proof.at >= since && self.valid(s, &held))
            .map(|s| &s.proof)
    }

    /// The proofs valid here (see [`Self::tombstones_since`]) for `ids`,
    /// by id in the order given, each id once and its proofs best first.
    pub fn valid_tombstones_for(&self, ids: &[MessageId]) -> Vec<&TombstoneProof> {
        let held = self.held_with_proofs();
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for id in ids.iter().filter(|id| seen.insert(**id)) {
            let mut list: Vec<&StoredProof> =
                self.tombstones.get(id).into_iter().flatten().filter(|s| self.valid(s, &held)).collect();
            list.sort_by_key(|s| proof_rank(&self.history(), s));
            out.extend(list.into_iter().map(|s| &s.proof));
        }
        out
    }

    fn stored_proofs(&self) -> impl Iterator<Item = &StoredProof> {
        self.tombstones.values().flatten()
    }

    /// Ids of messages held here that proofs are stored for.
    fn held_with_proofs(&self) -> HashSet<MessageId> {
        if self.tombstones.is_empty() {
            return HashSet::new();
        }
        self.messages.values().flatten().map(|m| m.id).filter(|id| self.tombstones.contains_key(id)).collect()
    }

    /// Whether a stored proof is valid here, see [`Self::tombstones_since`].
    fn valid(&self, stored: &StoredProof, held: &HashSet<MessageId>) -> bool {
        (stored.accepted_for.is_some() || !held.contains(&stored.proof.message))
            && self.history().may_delete_unseen(&stored.proof)
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

    /// Applies an event to local state. Returns Ok(false) when the event
    /// was a duplicate or no-op (safe to ignore on gossip redelivery).
    pub fn apply(&mut self, event: ServerEvent) -> Result<bool, StoreError> {
        // Events that change who held which role or was a member, when,
        // or what a role allows: kept deletes may apply now.
        let history = matches!(
            event,
            ServerEvent::RoleCreate { .. }
                | ServerEvent::RoleUpdate { .. }
                | ServerEvent::RoleAssign { .. }
                | ServerEvent::RoleUnassign { .. }
                | ServerEvent::MemberJoin { .. }
                | ServerEvent::MemberLeave { .. }
                | ServerEvent::MemberBan { .. }
                | ServerEvent::MemberUnban { .. }
        );
        let changed = self.apply_event(event)?;
        if changed && history {
            self.settle_proofs();
        }
        Ok(changed)
    }

    fn apply_event(&mut self, event: ServerEvent) -> Result<bool, StoreError> {
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
                msg.toggle_reaction(&emoji, user);
                Ok(true)
            }
            ServerEvent::MemberJoin { server, profile, at } => self.join(server, profile, at),
            ServerEvent::MemberLeave { server, user, at } => {
                self.check_member_change(server, at)?;
                // Only an unban lifts a ban.
                if self.server.banned.contains(&user) {
                    return Ok(false);
                }
                Ok(self.remove_member(user, Membership::Left, at))
            }
            ServerEvent::PresenceUpdate { user, presence } => {
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
                self.messages.remove(&channel);
                if retain_proofs(&mut self.tombstones, |s| s.proof.channel != channel) {
                    self.unapplied = None;
                }
                let new_tombstone = !self.server.deleted_channels.contains(&channel);
                if new_tombstone {
                    self.server.deleted_channels.push(channel);
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
            ServerEvent::VoiceState { user, channel, .. } => match channel {
                Some(ch) => {
                    if self.server.channel(ch).is_none() {
                        return Err(StoreError::UnknownChannel);
                    }
                    Ok(self.voice_states.insert(user, ch) != Some(ch))
                }
                None => {
                    // Leaving voice also ends any screen share or camera.
                    self.screen_shares.remove(&user);
                    self.cameras.remove(&user);
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
            ServerEvent::MemberBan { server, user, at } => {
                self.check_member_change(server, at)?;
                if user == self.server.owner || !self.remove_member(user, Membership::Banned, at) {
                    return Ok(false);
                }
                if !self.server.banned.contains(&user) {
                    self.server.banned.push(user);
                }
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
            ServerEvent::MemberUnban { server, user, at } => {
                self.check_member_change(server, at)?;
                // A ban made later stands. The user counts as left.
                if !self.remove_member(user, Membership::Left, at) {
                    return Ok(false);
                }
                self.server.banned.retain(|u| *u != user);
                Ok(true)
            }
            ServerEvent::RoleUpdate { server, role } => self.put_role(server, role, false),
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
                match self.profiles.get(&profile.id) {
                    Some(existing) if *existing == profile => Ok(false),
                    _ => {
                        self.profiles.insert(profile.id, profile);
                        Ok(true)
                    }
                }
            }
            ServerEvent::RoleCreate { server, role } => self.put_role(server, role, true),
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
                // Known here or not, since a delete may beat its create.
                let new_tombstone = !self.server.deleted_roles.contains(&role);
                if new_tombstone {
                    self.server.deleted_roles.push(role);
                }
                let before = self.server.roles.len();
                self.server.roles.retain(|r| r.id != role);
                for member in &mut self.server.members {
                    member.roles.retain(|r| *r != role);
                }
                self.role_marks.retain(|(_, r), _| *r != role);
                Ok(self.server.roles.len() != before || new_tombstone)
            }
            ServerEvent::RoleAssign { server, user, role, at } => {
                self.mark_role(server, RoleMark { user, role, held: true, at, since: None })
            }
            ServerEvent::RoleUnassign { server, user, role, at, since } => {
                self.mark_role(server, RoleMark { user, role, held: false, at, since })
            }
            // Typing indicators are transient UI state, not stored.
            ServerEvent::TypingStart { .. } => Ok(true),
        }
    }

    /// A role definition; the newest wins (see [`Role::updated_at`]), ties
    /// going to the larger encoding so every replica keeps the same one. A
    /// create also adds a role not known here, unless it was deleted.
    fn put_role(&mut self, server: ServerId, role: Role, create: bool) -> Result<bool, StoreError> {
        if server != self.server.id {
            return Err(StoreError::WrongServer);
        }
        if too_far_ahead_ms(role.updated_at, Utc::now()) {
            return Err(StoreError::TooFarAhead);
        }
        if role.banner.as_deref().is_some_and(|b| crate::server::ImageRef::parse(b).is_none())
            || role.tag.as_deref().is_some_and(|t| !crate::server::valid_role_tag(t))
            || role.name.trim().is_empty()
            || self.server.deleted_roles.contains(&role.id)
        {
            return Ok(false);
        }
        let Some(existing) = self.server.roles.iter_mut().find(|r| r.id == role.id) else {
            // Position 0 is reserved for the implicit everyone role.
            if !create || role.position == 0 {
                return Ok(false);
            }
            self.server.roles.push(role);
            return Ok(true);
        };
        // The everyone role keeps its name and place and cannot be
        // self-assigned, and no other role takes its place.
        let everyone = existing.position == 0;
        if everyone != (role.position == 0) || (everyone && (role.name != existing.name || role.self_assignable)) {
            return Ok(false);
        }
        let encoded = |r: &Role| postcard::to_stdvec(r).unwrap_or_default();
        let newer = match role.updated_at.cmp(&existing.updated_at) {
            std::cmp::Ordering::Equal => encoded(&role) > encoded(existing),
            order => order.is_gt(),
        };
        if !newer {
            return Ok(false);
        }
        *existing = role;
        Ok(true)
    }

    fn check_member_change(&self, server: ServerId, at: i64) -> Result<(), StoreError> {
        if server != self.server.id {
            return Err(StoreError::WrongServer);
        }
        if too_far_ahead_ms(at, Utc::now()) {
            return Err(StoreError::TooFarAhead);
        }
        Ok(())
    }

    /// A join, unless the user is banned or a later change to their
    /// membership is known (see [`MemberMark::beats`]). Someone already a
    /// member takes the profile either way, as their own announcement.
    fn join(&mut self, server: ServerId, mut profile: UserProfile, at: i64) -> Result<bool, StoreError> {
        self.check_member_change(server, at)?;
        let user = profile.id;
        if self.server.banned.contains(&user) {
            return Ok(false);
        }
        self.pin_identity(&mut profile)?;
        let member = self.server.is_member(user);
        let mark = MemberMark { user, state: Membership::Joined, at, since: None };
        if self.member_marks.get(&user).is_some_and(|known| !mark.beats(known)) {
            if member {
                self.profiles.insert(user, profile);
            }
            return Ok(false);
        }
        self.member_marks.insert(user, mark);
        if !member {
            self.server.members.push(Member { user, nickname: None, roles: vec![], joined_at: Utc::now() });
        }
        self.profiles.insert(user, profile);
        Ok(true)
    }

    /// Takes `user` out at `at` as `state`, left or banned, unless a later
    /// change to their membership is known. The roles they held become
    /// removals at that time, see [`Self::end_roles`]. Returns whether
    /// anything changed.
    fn remove_member(&mut self, user: UserId, state: Membership, at: i64) -> bool {
        let mut mark = MemberMark { user, state, at, since: None };
        if self.member_marks.get(&user).is_some_and(|known| !mark.beats(known)) {
            return false;
        }
        let held = self.server.member(user).map(|m| m.roles.clone());
        if let Some(roles) = held {
            mark.since = Some(self.joined_at(user).unwrap_or(i64::MIN));
            self.server.members.retain(|m| m.user != user);
            self.voice_states.remove(&user);
            self.end_roles(user, at, &roles);
        }
        self.member_marks.insert(user, mark);
        true
    }

    /// Turns the roles `user` held, granted by a mark or from before marks
    /// were kept, into removals at `at`, each remembering when its grant
    /// was made. So a grant from before cannot come back with a later
    /// join, while a later real grant still wins. A grant marked later
    /// than `at` goes with the membership; removals stay as they are.
    fn end_roles(&mut self, user: UserId, at: i64, held: &[RoleId]) {
        let marked: Vec<RoleId> = self.role_marks.keys().filter(|(u, _)| *u == user).map(|(_, r)| *r).collect();
        for role in held.iter().copied().chain(marked) {
            let since = match self.role_marks.get(&(user, role)) {
                Some(m) if !m.held => continue,
                Some(m) if m.at > at => {
                    self.role_marks.remove(&(user, role));
                    continue;
                }
                Some(m) => m.at,
                None => i64::MIN,
            };
            self.role_marks.insert((user, role), RoleMark { user, role, held: false, at, since: Some(since) });
        }
    }

    /// Gives or takes a role from a member unless a change at least as
    /// late is known (see [`RoleMark::beats`]). A removal is kept even for
    /// a role not held here, so an older grant arriving later stays out,
    /// and for someone who left, as it tells which roles they held when.
    /// A grant dated before the member's latest join is refused: it was
    /// made in an earlier membership, which ended with every role.
    fn mark_role(&mut self, server: ServerId, mut mark: RoleMark) -> Result<bool, StoreError> {
        if server != self.server.id {
            return Err(StoreError::WrongServer);
        }
        if too_far_ahead_ms(mark.at, Utc::now()) {
            return Err(StoreError::TooFarAhead);
        }
        let RoleMark { user, role, held, .. } = mark;
        if !self.server.roles.iter().any(|r| r.id == role) {
            return Ok(false);
        }
        let member = self.server.is_member(user);
        let former = self.member_marks.get(&user).is_some_and(|m| m.state != Membership::Joined);
        if !(member || (former && !held)) || (held && self.joined_at(user).is_some_and(|joined| mark.at < joined)) {
            return Ok(false);
        }
        // What we know of the grant it ends goes before what it says.
        mark.since = if held { None } else { self.removed_since(user, role).or(mark.since) };
        if let Some(known) = self.role_marks.get_mut(&(user, role)) {
            if !mark.beats(known) {
                // The same removal again may tell when the grant it ended was made.
                let learned = known.since.is_none() && *known == RoleMark { since: None, ..mark } && mark.since.is_some();
                if learned {
                    known.since = mark.since;
                }
                return Ok(learned);
            }
        }
        self.role_marks.insert((user, role), mark);
        if let Some(member) = self.server.members.iter_mut().find(|m| m.user == user) {
            let has = member.roles.contains(&role);
            if held && !has {
                member.roles.push(role);
            } else if !held && has {
                member.roles.retain(|r| *r != role);
            }
        }
        Ok(true)
    }

    /// Signed create with an id bound to its author, no larger than
    /// [`MAX_MESSAGE_BYTES`]. A copy of a message we already hold is taken
    /// only as a newer signed edit of it. A late copy kept out by a delete
    /// returns Ok(true) when checking it changed the stored proofs, so
    /// callers save them.
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
        // Proofs for a message held here are ones not applied yet.
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
            if millis(message.edited_at) <= millis(existing.edited_at) {
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
        // None of its proofs kept it out: each names another channel or
        // author, or a signer who may not delete it. They count no more.
        if self.tombstones.remove(&message.id).is_some() {
            self.unapplied = None;
        }
        let list = self
            .messages
            .get_mut(&message.channel)
            .ok_or(StoreError::UnknownChannel)?;
        message.reactions = relayed_reactions(std::mem::take(&mut message.reactions));
        // Keep messages sorted by creation time for display.
        let pos = list.partition_point(|m| m.created_at <= message.created_at);
        list.insert(pos, message);
        Ok(true)
    }

    /// Checks the proofs stored for a message not held here. One accepted
    /// here for its author keeps it out, as does one for its channel that
    /// may delete it (see [`History::may_delete`]), which so becomes
    /// accepted; the rest, those dated before the message among them,
    /// then go. Returns whether the stored proofs changed when one kept
    /// it out, or None when none did.
    fn bind_proofs(&mut self, message: &Message) -> Option<bool> {
        let mut list = self.tombstones.remove(&message.id)?;
        let history = History { server: &self.server, roles: &self.role_marks, members: &self.member_marks };
        let author = message.author;
        let before = list.len();
        let (mut blocked, mut bound) = (false, false);
        for s in &mut list {
            if s.proof.channel == message.channel
                && (s.accepted_for == Some(author) || history.may_delete(&s.proof, author, message.created_at))
            {
                bound |= s.accepted_for.is_none();
                s.accepted_for.get_or_insert(author);
                blocked = true;
            }
        }
        if blocked {
            list.retain(|s| s.accepted_for.is_some());
        }
        let changed = bound || list.len() != before;
        if changed {
            self.unapplied = None;
        }
        self.tombstones.insert(message.id, list);
        blocked.then_some(changed)
    }

    /// Signed edit by the author; older edits than the stored one lose.
    /// The edited message must stay within [`MAX_MESSAGE_BYTES`].
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
        if Some(edited_at.timestamp_millis()) <= millis(msg.edited_at) {
            return Ok(false);
        }
        msg.content = edited.content;
        msg.edited_at = edited.edited_at;
        msg.sig = edited.sig;
        Ok(true)
    }

    /// Signed delete, judged by its signer's standing when it was made
    /// (see [`History::may_delete`]). A delete of a message held here that
    /// its signer may not make is kept, not applied: the role allowing it
    /// may not have reached us yet, see [`Self::settle_proofs`]. One
    /// naming another channel than the message's, or dated before the
    /// message, is refused. Without the message at hand only its author
    /// or a moderator may delete; the proof is kept and checked against
    /// the message if it turns up later. Each signer has at most
    /// [`MAX_UNAPPLIED_PROOFS`] kept, see [`Self::store_unapplied`].
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
        let held = self.messages.iter().find_map(|(channel, list)| {
            let pos = list.iter().position(|m| m.id == proof.message)?;
            Some((*channel, pos, list[pos].author, list[pos].created_at))
        });
        if let Some((channel, pos, author, created_at)) = held {
            // Peers lacking the message could not tell it apart from the
            // real one, and those holding it would never take it.
            if channel != proof.channel {
                return Err(StoreError::WrongChannel);
            }
            // Nobody could delete it before it was written.
            if proof.at.timestamp_millis() < created_at.timestamp_millis() {
                return Err(StoreError::NotAllowed);
            }
            if !self.history().may_delete(&proof, author, created_at) {
                return self.store_unapplied(proof);
            }
            if let Some(list) = self.messages.get_mut(&channel) {
                list.remove(pos);
            }
            let id = proof.message;
            self.store_proof(proof, Some(author));
            self.drop_unapplied([id]);
            return Ok(true);
        }
        if !self.history().may_delete_unseen(&proof) {
            return Err(StoreError::NotAllowed);
        }
        self.store_unapplied(proof)
    }

    /// Keeps a proof not applied here. Every replica keeps the same ones
    /// of each signer: up to [`MAX_UNAPPLIED_PROOFS`], the best ranked
    /// (see [`UnappliedKey`]). A better one than the signer's worst
    /// pushes that out; a worse one is refused.
    fn store_unapplied(&mut self, proof: TombstoneProof) -> Result<bool, StoreError> {
        let (by, key) = (proof.by, unapplied_key(&proof));
        // Another copy of one kept already takes no new place.
        let kept = self
            .tombstones
            .get(&proof.message)
            .is_some_and(|list| list.iter().any(|s| s.proof.by == by && s.proof.channel == proof.channel));
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

    /// Applies kept deletes of messages held here that their signer may
    /// make (see [`History::may_delete`]), after a change to who held
    /// which role or was a member when, or to what a role allows: a
    /// moderator's delete that arrived before the role allowing it. The
    /// message goes and the proof counts as accepted. Returns whether
    /// anything changed.
    fn settle_proofs(&mut self) -> bool {
        let mut removed = Vec::new();
        {
            let Self { server, messages, tombstones, role_marks, member_marks, .. } = self;
            let history = History { server, roles: role_marks, members: member_marks };
            let waiting: HashSet<MessageId> = tombstones
                .values()
                .flatten()
                .filter(|s| s.accepted_for.is_none())
                .map(|s| s.proof.message)
                .collect();
            if waiting.is_empty() {
                return false;
            }
            for (channel, list) in messages.iter_mut() {
                list.retain(|m| {
                    if !waiting.contains(&m.id) {
                        return true;
                    }
                    let Some(proofs) = tombstones.get_mut(&m.id) else { return true };
                    let mut applied = false;
                    for s in proofs.iter_mut() {
                        if s.accepted_for.is_none()
                            && s.proof.channel == *channel
                            && history.may_delete(&s.proof, m.author, m.created_at)
                        {
                            s.accepted_for = Some(m.author);
                            applied = true;
                        }
                    }
                    if applied {
                        removed.push(m.id);
                    }
                    !applied
                });
            }
        }
        let changed = !removed.is_empty();
        if changed {
            self.unapplied = None;
        }
        self.drop_unapplied(removed);
        changed
    }

    /// For messages no longer held, drops the proofs kept only while they
    /// were: not accepted, and not ones we take for a message not held.
    /// Such a proof would be refused by every peer lacking the message.
    fn drop_unapplied(&mut self, ids: impl IntoIterator<Item = MessageId>) {
        let history = History { server: &self.server, roles: &self.role_marks, members: &self.member_marks };
        for id in ids {
            let Some(list) = self.tombstones.get_mut(&id) else { continue };
            let before = list.len();
            list.retain(|s| s.accepted_for.is_some() || history.may_delete_unseen(&s.proof));
            if list.len() != before {
                self.unapplied = None;
            }
            if list.is_empty() {
                self.tombstones.remove(&id);
            }
        }
    }

    /// Keeps a proof next to the others for its message, one per deleter
    /// and channel. Proofs accepted here, or signed by a moderator at the
    /// time or by the message's author when known, are always kept; of
    /// the others at most [`MAX_OTHER_PROOFS`], the best ranked. Returns
    /// whether anything changed.
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
        let history = History { server: &self.server, roles: &self.role_marks, members: &self.member_marks };
        let rank = |s: &StoredProof| proof_rank(&history, s);
        let mut new = StoredProof { proof, accepted_for };
        let (by, channel) = (new.proof.by, new.proof.channel);
        let list = self.tombstones.entry(new.proof.message).or_default();
        if let Some(old) = list.iter_mut().find(|s| s.proof.by == by && s.proof.channel == channel) {
            // The same delete made twice (two devices, a retry) keeps the
            // acceptance either one had.
            new.accepted_for = new.accepted_for.or(old.accepted_for);
            let changed = old.accepted_for != new.accepted_for;
            old.accepted_for = new.accepted_for;
            if rank(&new) < rank(old) {
                *old = new;
                return true;
            }
            return changed;
        }
        list.push(new);
        // Only an accepted proof tells who wrote the message.
        let author = list.iter().find_map(|s| s.accepted_for);
        let kept = |s: &StoredProof| {
            s.accepted_for.is_some() || Some(s.proof.by) == author || history.moderator_at(s.proof.by, s.proof.at)
        };
        // Dropping only the new one changes nothing.
        let (mut dropped_new, mut dropped_other) = (false, false);
        while list.iter().filter(|s| !kept(s)).count() > MAX_OTHER_PROOFS {
            let Some(worst) = list
                .iter()
                .enumerate()
                .filter(|(_, s)| !kept(s))
                .max_by_key(|(_, s)| rank(s))
                .map(|(i, _)| i)
            else {
                break;
            };
            let gone = list.swap_remove(worst).proof;
            if gone.by == by && gone.channel == channel {
                dropped_new = true;
            } else {
                dropped_other = true;
            }
        }
        !dropped_new || dropped_other
    }

    /// Pins the first identity key seen for a user. A profile without a
    /// key keeps the pinned one; a different key is refused.
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
        self.messages.values_mut().flat_map(|list| list.iter_mut()).find(|m| m.id == id)
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
    Some(now - chrono::Duration::days(days as i64))
}

/// Preference between proofs for one message, lowest first: accepted
/// here, then a moderator's at the time, then the earliest, then the
/// lowest signature, so peers holding the same proofs keep the same ones.
fn proof_rank(history: &History, stored: &StoredProof) -> (bool, bool, DateTime<Utc>, [u8; 64]) {
    let moderator = history.moderator_at(stored.proof.by, stored.proof.at);
    (stored.accepted_for.is_none(), !moderator, stored.proof.at, stored.proof.sig.0)
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

/// Whether a signed time lies more than [`MAX_AHEAD_MS`] past `now`.
fn too_far_ahead(t: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    too_far_ahead_ms(t.timestamp_millis(), now)
}

fn too_far_ahead_ms(ms: i64, now: DateTime<Utc>) -> bool {
    ms.saturating_sub(now.timestamp_millis()) > MAX_AHEAD_MS
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ServerId;
    use crate::test_support::{delete, edit, fixture, identity_for, join, join_moderator, post, post_at, proof, signed};

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
            state.apply(ServerEvent::MemberJoin { server: sid, profile: profile.clone(), at: 0 }),
            Ok(true)
        );
        assert_eq!(
            state.apply(ServerEvent::MemberJoin { server: sid, profile, at: 0 }),
            Ok(false)
        );
    }

    #[test]
    fn wrong_server_event_rejected() {
        let (mut state, _, _) = fixture();
        let profile = UserProfile::new("spoof");
        assert_eq!(
            state.apply(ServerEvent::MemberJoin { server: ServerId::new(), profile, at: 0 }),
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
                deafened: false
            }),
            Ok(true)
        );
        assert_eq!(state.voice_members(voice), vec![owner]);
        assert_eq!(
            state.apply(ServerEvent::VoiceState {
                user: owner,
                channel: None,
                muted: false,
                deafened: false
            }),
            Ok(true)
        );
        assert!(state.voice_members(voice).is_empty());
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
                deafened: false,
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
                deafened: false,
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
                at: 0,
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
                at: 0,
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
            updated_at: 0,
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
        let assign = ServerEvent::RoleAssign { server: sid, user: owner, role: role.id, at: 1 };
        assert_eq!(state.apply(assign.clone()), Ok(true));
        assert!(state
            .server
            .permissions_for(owner)
            .allows(crate::Permissions::KICK_MEMBERS));
        assert_eq!(state.apply(assign), Ok(false));
        assert_eq!(
            state.apply(ServerEvent::RoleUnassign { server: sid, user: owner, role: role.id, at: 2, since: None }),
            Ok(true)
        );
        // Re-assign then delete removes it from the member too.
        state
            .apply(ServerEvent::RoleAssign { server: sid, user: owner, role: role.id, at: 3 })
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
            updated_at: 0,
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
            updated_at: 0,
        };
        let rid = role.id;
        state.apply(ServerEvent::RoleCreate { server: sid, role }).unwrap();
        assert_eq!(
            state.apply(ServerEvent::RoleAssign { server: sid, user: UserId::new(), role: rid, at: 1 }),
            Ok(false)
        );
        // Assigning a role that does not exist is also a no-op.
        assert_eq!(
            state.apply(ServerEvent::RoleAssign {
                server: sid,
                user: state.server.owner,
                role: crate::ids::RoleId::new(),
                at: 1,
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
        state.apply(ServerEvent::MemberJoin { server: sid, profile: offline, at: 0 }).unwrap();
        state.apply(ServerEvent::MemberJoin { server: sid, profile: online, at: 0 }).unwrap();
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
        let join = |ch| ServerEvent::VoiceState { user: owner, channel: ch, muted: false, deafened: false };
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

        let sound = SoundboardSound { name: "airhorn".into(), blob: "dd@bb".into(), volume: 1.0 };
        assert_eq!(state.apply(ServerEvent::SoundAdd { server: sid, sound: sound.clone() }), Ok(true));
        let loud = SoundboardSound { volume: 9.0, ..sound };
        assert_eq!(state.apply(ServerEvent::SoundAdd { server: sid, sound: loud }), Ok(false));
        assert_eq!(state.apply(ServerEvent::SoundRemove { server: sid, name: "airhorn".into() }), Ok(true));
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
        let pickable = crate::server::Role { self_assignable: true, updated_at: 1, ..everyone.clone() };
        assert_eq!(state.apply(ServerEvent::RoleUpdate { server: sid, role: pickable }), Ok(false), "everyone is not pickable");
        let tagged = crate::server::Role { tag: Some("GG".into()), updated_at: 1, ..everyone.clone() };
        assert_eq!(state.apply(ServerEvent::RoleUpdate { server: sid, role: tagged }), Ok(true));
        let custom = crate::server::Role { tag: Some("<:gg:ab12@cd34>".into()), banner: Some("ab@cd".into()), updated_at: 2, ..everyone.clone() };
        assert_eq!(state.apply(ServerEvent::RoleUpdate { server: sid, role: custom }), Ok(true), "custom emoji tag and banner");
        let bad_banner = crate::server::Role { banner: Some("../x".into()), updated_at: 3, ..everyone.clone() };
        assert_eq!(state.apply(ServerEvent::RoleUpdate { server: sid, role: bad_banner }), Ok(false));
        let long = crate::server::Role { tag: Some("much too long tag".into()), updated_at: 3, ..everyone };
        assert_eq!(state.apply(ServerEvent::RoleUpdate { server: sid, role: long }), Ok(false));
    }

    #[test]
    fn server_mute_and_deafen() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let other = UserProfile::new("other");
        let uid = other.id;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: other, at: 0 }).unwrap();
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
        let join = |at| ServerEvent::MemberJoin { server: sid, profile: other.clone(), at };
        state.apply(join(1)).unwrap();
        assert_eq!(state.apply(ServerEvent::MemberBan { server: sid, user: uid, at: 10 }), Ok(true));
        assert!(!state.server.is_member(uid));
        assert_eq!(state.apply(join(20)), Ok(false));
        assert!(!state.server.is_member(uid), "banned users cannot rejoin, even later");
        assert_eq!(state.apply(ServerEvent::MemberLeave { server: sid, user: uid, at: 20 }), Ok(false));
        assert_eq!(state.apply(ServerEvent::MemberBan { server: sid, user: owner, at: 10 }), Ok(false));
        // An unban lifts only a ban made before it.
        for at in [5, 10] {
            assert_eq!(state.apply(ServerEvent::MemberUnban { server: sid, user: uid, at }), Ok(false), "{at}");
        }
        assert_eq!(state.apply(ServerEvent::MemberUnban { server: sid, user: uid, at: 30 }), Ok(true));
        assert!(state.server.banned.is_empty());
        // The ban announced again changes nothing, nor a join from before.
        assert_eq!(state.apply(ServerEvent::MemberBan { server: sid, user: uid, at: 10 }), Ok(false));
        assert_eq!(state.apply(join(20)), Ok(false));
        assert_eq!(state.apply(join(40)), Ok(true));
        assert!(state.server.is_member(uid));
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
        state.apply(ServerEvent::MemberJoin { server: sid, profile: profile.clone(), at: 0 }).unwrap();
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
            state.apply(ServerEvent::MemberJoin { server: sid, profile: imposter.clone(), at: 0 }),
            Err(StoreError::IdentityMismatch)
        );
        assert_eq!(state.identity_of(uid), Some(key));
        assert_eq!(state.profiles[&uid].display_name, None);

        // The pin outlives leaving the server.
        state.apply(ServerEvent::MemberLeave { server: sid, user: uid, at: 1 }).unwrap();
        assert_eq!(
            state.apply(ServerEvent::MemberJoin { server: sid, profile: imposter, at: 2 }),
            Err(StoreError::IdentityMismatch)
        );
        assert!(!state.server.is_member(uid));
        assert_eq!(state.apply(ServerEvent::MemberJoin { server: sid, profile, at: 2 }), Ok(true));
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
        state.apply(ServerEvent::MemberJoin { server: sid, profile: keyless, at: 0 }).unwrap();
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
    fn deletes_need_the_author_or_manage_messages() {
        let (mut state, channel, _) = fixture();
        let sid = state.server.id;
        let author = join(&mut state, "author");
        let moderator = join_moderator(&mut state, "moderator");
        let stranger = join(&mut state, "stranger");
        let (a, b, c) = (
            post(&state, channel, author, "a"),
            post(&state, channel, author, "b"),
            post(&state, channel, author, "c"),
        );
        for m in [&a, &b, &c] {
            create(&mut state, m).unwrap();
        }

        // Kept but not applied, see `a_delete_waits_for_the_role_that_allows_it`.
        assert_eq!(state.apply(delete(&state, &a, stranger)), Ok(true));
        assert!(state.has_message(a.id));
        assert_eq!(state.apply(delete(&state, &a, author)), Ok(true));
        let by: Vec<UserId> = state.tombstones_for(a.id).iter().map(|p| p.by).collect();
        assert_eq!(by, vec![author], "the stranger's goes with the message");
        assert_eq!(state.apply(delete(&state, &b, moderator)), Ok(true));
        assert_eq!(state.tombstone(b.id).map(|p| p.by), Some(moderator));
        assert!(!state.has_message(a.id) && !state.has_message(b.id));

        let keyless = UserProfile::new("keyless");
        let kid = keyless.id;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: keyless, at: 0 }).unwrap();
        assert_eq!(state.apply(delete(&state, &c, kid)), Err(StoreError::UnknownAuthorKey(kid)));

        let forged = TombstoneProof::sign(sid, channel, c.id, author, Utc::now(), &identity_for(stranger));
        assert_eq!(state.apply(forged.into_event()), Err(StoreError::BadSignature));
        let unknown_channel = TombstoneProof::sign(sid, ChannelId::new(), c.id, author, Utc::now(), &identity_for(author));
        assert_eq!(state.apply(unknown_channel.into_event()), Err(StoreError::UnknownChannel));
        assert!(state.has_message(c.id));
    }

    #[test]
    fn early_tombstone_is_checked_against_the_late_message() {
        let (mut state, channel, _) = fixture();
        let sid = state.server.id;
        let author = join(&mut state, "author");
        let moderator = join_moderator(&mut state, "moderator");
        let stranger = join(&mut state, "stranger");

        // A stranger may not delete a message not held here.
        let one = post(&state, channel, author, "one");
        assert_eq!(state.apply(delete(&state, &one, stranger)), Err(StoreError::NotAllowed));
        assert!(state.tombstone(one.id).is_none());
        assert_eq!(create(&mut state, &one), Ok(true));
        assert!(state.has_message(one.id));

        // A moderator's proof keeps it out.
        let two = post(&state, channel, author, "two");
        assert_eq!(state.apply(delete(&state, &two, moderator)), Ok(true));
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

        // Each deleter's proof is kept, a moderator's ranking first.
        let four = post(&state, channel, author, "four");
        assert_eq!(state.apply(delete(&state, &four, moderator)), Ok(true));
        assert_eq!(state.apply(delete(&state, &four, author)), Ok(true));
        assert_eq!(state.tombstone(four.id).map(|p| p.by), Some(moderator));
        assert_eq!(create(&mut state, &four), Ok(true));
        assert!(!state.has_message(four.id));
        let left: Vec<UserId> = state.tombstones_for(four.id).iter().map(|p| p.by).collect();
        assert_eq!(left, vec![moderator, author]);
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
            ("leave", |server, user, _| ServerEvent::MemberLeave { server, user, at: Utc::now().timestamp_millis() }),
            ("unassign", |server, user, role| ServerEvent::RoleUnassign { server, user, role, at: Utc::now().timestamp_millis(), since: None }),
            ("ban", |server, user, _| ServerEvent::MemberBan { server, user, at: Utc::now().timestamp_millis() }),
        ];
        for (how, demote) in demotions {
            let moderator = join_moderator(&mut state, "moderator");
            let msg = post(&state, channel, author, how);
            create(&mut state, &msg).unwrap();
            assert_eq!(state.apply(delete(&state, &msg, moderator)), Ok(true));
            assert_eq!(state.apply(demote(sid, moderator, mods_role(&state, moderator))), Ok(true));
            assert!(!state.server.permissions_for(moderator).allows(Permissions::MANAGE_MESSAGES), "{how}");
            assert_eq!(create(&mut state, &msg), Ok(false), "{how}");
            assert_eq!(state.tombstone(msg.id).map(|p| p.by), Some(moderator), "{how}");
            assert_eq!(create(&mut reload(&state), &msg), Ok(false), "{how}, after a restart");
        }
    }

    #[test]
    fn early_proof_binds_once_checked_against_the_message() {
        let (mut state, channel, _) = fixture();
        let author = join(&mut state, "author");
        let moderator = join_moderator(&mut state, "moderator");
        let role = mods_role(&state, moderator);
        let checked = post(&state, channel, author, "checked");
        let unchecked = post(&state, channel, author, "unchecked");
        state.apply(delete(&state, &checked, moderator)).unwrap();
        state.apply(delete(&state, &unchecked, moderator)).unwrap();
        // The message turns up while its deleter is still a moderator.
        assert_eq!(create(&mut state, &checked), Ok(true));
        assert!(!state.has_message(checked.id));
        let demoted = Utc::now() + chrono::Duration::seconds(1);
        state.apply(state.role_change(moderator, role, false, demoted)).unwrap();
        assert_eq!(create(&mut state, &checked), Ok(false));
        // A proof never checked is judged by the roles held when it was made.
        assert_eq!(create(&mut state, &unchecked), Ok(true));
        assert!(!state.has_message(unchecked.id));
        assert_eq!(state.tombstone(unchecked.id).map(|p| p.by), Some(moderator));
        let later = post_at(&state, channel, author, "later", demoted);
        create(&mut state, &later).unwrap();
        assert_eq!(state.apply(proof(&state, &later, moderator, demoted).into_event()), Ok(true));
        assert!(state.has_message(later.id), "kept, not applied");
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
        assert_eq!(create(&mut restored, &msg), Ok(true), "still a moderator, bound again");
        assert!(!restored.has_message(msg.id));
    }

    #[test]
    fn a_strangers_proof_does_not_shut_out_the_authors() {
        let (mut state, channel, _) = fixture();
        let author = join(&mut state, "author");
        let stranger = join(&mut state, "stranger");
        let msg = post(&state, channel, author, "mine");
        create(&mut state, &msg).unwrap();
        assert_eq!(state.apply(delete(&state, &msg, stranger)), Ok(true));
        assert!(state.has_message(msg.id), "kept, not applied");
        let own = proof(&state, &msg, author, Utc::now());
        assert_eq!(state.apply(own.clone().into_event()), Ok(true));
        assert_eq!(state.apply(own.clone().into_event()), Ok(false), "redelivery");
        assert!(!state.has_message(msg.id));
        assert_eq!(state.tombstones_for(msg.id), vec![&own], "the stranger's goes with the message");
        // Nobody holding the message would take it now.
        assert_eq!(state.apply(delete(&state, &msg, stranger)), Err(StoreError::NotAllowed));
    }

    #[test]
    fn proofs_from_others_are_capped_keeping_the_earliest() {
        let (mut state, channel, _) = fixture();
        let author = join(&mut state, "author");
        let strangers: Vec<UserId> =
            (0..=MAX_OTHER_PROOFS).map(|i| join(&mut state, &format!("s{i}"))).collect();
        let msg = post_at(&state, channel, author, "busy", Utc::now() - chrono::Duration::days(2));
        create(&mut state, &msg).unwrap();
        let now = Utc::now();
        for (i, s) in strangers.iter().enumerate() {
            let p = proof(&state, &msg, *s, now + chrono::Duration::seconds(i as i64));
            assert_eq!(state.apply(p.into_event()), Ok(i < MAX_OTHER_PROOFS), "proof {i}");
        }
        assert!(state.has_message(msg.id));
        // An earlier one displaces the latest.
        let early = join(&mut state, "early");
        let p = proof(&state, &msg, early, now - chrono::Duration::days(1));
        assert_eq!(state.apply(p.into_event()), Ok(true));
        let kept: Vec<UserId> = state.tombstones_for(msg.id).iter().map(|p| p.by).collect();
        assert_eq!(kept, [&[early], &strangers[..MAX_OTHER_PROOFS - 1]].concat());
    }

    #[test]
    fn backdated_proofs_never_push_out_the_authors_or_a_moderators() {
        let (mut state, channel, _) = fixture();
        let author = join(&mut state, "author");
        let moderator = join_moderator(&mut state, "moderator");
        let strangers: Vec<UserId> =
            (0..MAX_OTHER_PROOFS + 3).map(|i| join(&mut state, &format!("s{i}"))).collect();
        let mut lacking = state.clone();
        let msg = post_at(&state, channel, author, "contested", Utc::now() - chrono::Duration::days(31));
        create(&mut state, &msg).unwrap();
        let now = Utc::now();
        let backdated: Vec<TombstoneProof> = strangers
            .iter()
            .enumerate()
            .map(|(i, s)| proof(&state, &msg, *s, now - chrono::Duration::days(30) - chrono::Duration::seconds(i as i64)))
            .collect();
        // Strangers' deletes wait where the message is held, the earliest
        // few of them, and are refused where it is not.
        for p in &backdated {
            state.apply(p.clone().into_event()).unwrap();
            assert_eq!(lacking.apply(p.clone().into_event()), Err(StoreError::NotAllowed));
        }
        let earliest: Vec<UserId> = strangers.iter().rev().take(MAX_OTHER_PROOFS).copied().collect();
        let kept: Vec<UserId> = state.tombstones_for(msg.id).iter().map(|p| p.by).collect();
        assert_eq!(kept, earliest);
        // The moderator's delete removes the message; the author's own
        // comes later, unchecked. A replica that never had the message
        // takes both.
        let (by_moderator, by_author) = (proof(&state, &msg, moderator, now), proof(&state, &msg, author, now));
        for replica in [&mut state, &mut lacking] {
            assert_eq!(replica.apply(by_moderator.clone().into_event()), Ok(true));
            assert_eq!(replica.apply(by_author.clone().into_event()), Ok(true));
            for p in &backdated {
                assert_eq!(replica.apply(p.clone().into_event()), Err(StoreError::NotAllowed));
            }
            let kept: Vec<UserId> = replica.tombstones_for(msg.id).iter().map(|p| p.by).collect();
            assert_eq!(kept, vec![moderator, author]);
            assert_eq!(create(replica, &msg), Ok(true));
            assert!(!replica.has_message(msg.id));
        }
    }

    #[test]
    fn proofs_from_moderators_are_never_capped() {
        let (mut state, channel, _) = fixture();
        let author = join(&mut state, "author");
        let moderators: Vec<UserId> =
            (0..MAX_OTHER_PROOFS + 2).map(|i| join_moderator(&mut state, &format!("m{i}"))).collect();
        let former = join_moderator(&mut state, "former");
        let role = mods_role(&state, former);
        let msg = post(&state, channel, author, "reported");
        let now = Utc::now();
        // One made while its signer was a moderator still counts as a moderator's.
        let early = proof(&state, &msg, former, now - chrono::Duration::days(1));
        assert_eq!(state.apply(early.into_event()), Ok(true));
        state.apply(state.role_change(former, role, false, Utc::now())).unwrap();
        for (i, m) in moderators.iter().enumerate() {
            let p = proof(&state, &msg, *m, now + chrono::Duration::seconds(i as i64));
            assert_eq!(state.apply(p.into_event()), Ok(true), "moderator {i}");
        }
        let kept: Vec<UserId> = state.tombstones_for(msg.id).iter().map(|p| p.by).collect();
        assert_eq!(kept.len(), moderators.len() + 1);
        assert!(moderators.iter().all(|m| kept.contains(m)));
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
    fn equal_proofs_at_the_cap_keep_the_lowest_signatures() {
        let (mut a, channel, _) = fixture();
        let author = join(&mut a, "author");
        let strangers: Vec<UserId> =
            (0..MAX_OTHER_PROOFS + 2).map(|i| join(&mut a, &format!("s{i}"))).collect();
        let mut b = a.clone();
        let msg = post(&a, channel, author, "contested");
        create(&mut a, &msg).unwrap();
        create(&mut b, &msg).unwrap();
        let at = Utc::now();
        let proofs: Vec<TombstoneProof> = strangers.iter().map(|s| proof(&a, &msg, *s, at)).collect();
        for p in &proofs {
            a.apply(p.clone().into_event()).unwrap();
        }
        for p in proofs.iter().rev() {
            b.apply(p.clone().into_event()).unwrap();
        }
        let mut expected: Vec<&TombstoneProof> = proofs.iter().collect();
        expected.sort_by_key(|p| p.sig.0);
        expected.truncate(MAX_OTHER_PROOFS);
        assert_eq!(a.tombstones_for(msg.id), expected);
        assert_eq!(b.tombstones_for(msg.id), expected);
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
        let author = join(&mut state, "author");
        let stranger = join(&mut state, "stranger");
        let moderator = join_moderator(&mut state, "moderator");
        let role = mods_role(&state, moderator);
        let [held, applied, unseen, orphaned] =
            ["held", "applied", "unseen", "orphaned"].map(|text| post(&state, channel, author, text));
        create(&mut state, &held).unwrap();
        create(&mut state, &applied).unwrap();
        let now = Utc::now();
        let waiting = proof(&state, &held, stranger, now);
        let accepted = proof(&state, &applied, moderator, now);
        let own = proof(&state, &unseen, author, now);
        let former = proof(&state, &orphaned, moderator, now);
        for p in [&waiting, &accepted, &own, &former] {
            assert_eq!(state.apply(p.clone().into_event()), Ok(true));
        }
        state.apply(state.role_change(moderator, role, false, now + chrono::Duration::seconds(1))).unwrap();
        let ids = [held.id, applied.id, unseen.id, orphaned.id];
        let relayed = |state: &ServerState| {
            let mut list: Vec<TombstoneProof> = state.tombstones_since(now - chrono::Duration::days(1)).cloned().collect();
            list.sort_by_key(|p| ids.iter().position(|id| *id == p.message));
            list
        };
        // Judged by the roles held when they were made, so the demotion
        // since changes nothing.
        assert_eq!(relayed(&state), vec![accepted.clone(), own.clone(), former.clone()]);
        assert_eq!(state.valid_tombstones_for(&ids), vec![&accepted, &own, &former]);
        // What a role allows counts as it is now.
        let sid = state.server.id;
        let mods = state.server.roles.iter().find(|r| r.id == role).unwrap().clone();
        let update = |permissions, updated_at| ServerEvent::RoleUpdate {
            server: sid,
            role: Role { permissions, updated_at, ..mods.clone() },
        };
        state.apply(update(Permissions::NONE, 1)).unwrap();
        assert_eq!(relayed(&state), vec![own.clone()]);
        // The others stay here, and count again once valid.
        assert!(state.tombstone(held.id).is_some() && state.tombstone(orphaned.id).is_some());
        state.apply(update(Permissions::MANAGE_MESSAGES, 2)).unwrap();
        assert_eq!(relayed(&state), vec![accepted, own, former]);
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
    fn deletes_of_messages_not_held_need_their_author_or_a_moderator() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let author = join(&mut state, "author");
        let member = join(&mut state, "member");
        let banned = join(&mut state, "banned");
        let moderator = join_moderator(&mut state, "moderator");
        state.apply(ServerEvent::MemberBan { server: sid, user: banned, at: 1 }).unwrap();
        let theirs = post(&state, channel, author, "theirs");
        for by in [member, banned] {
            assert_eq!(state.apply(delete(&state, &theirs, by)), Err(StoreError::NotAllowed));
        }
        assert!(state.tombstone(theirs.id).is_none());
        assert_eq!(state.apply(delete(&state, &theirs, moderator)), Ok(true));
        assert_eq!(state.apply(delete(&state, &theirs, owner)), Ok(true), "the owner moderates");
        // A banned member may still take back their own.
        let own = post(&state, channel, banned, "own");
        assert_eq!(state.apply(delete(&state, &own, banned)), Ok(true));
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
        let stranger = join(&mut a, "stranger");
        let mut b = a.clone();
        let base = Utc::now() - chrono::Duration::hours(1);
        let held = post_at(&a, channel, stranger, "held", base - chrono::Duration::hours(1));
        create(&mut a, &held).unwrap();
        create(&mut b, &held).unwrap();
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
        // A delete of someone else's held message waits, and counts too.
        let waiting = proof(&a, &held, author, base - chrono::Duration::seconds(1));
        for state in [&mut a, &mut b] {
            assert_eq!(state.apply(waiting.clone().into_event()), Ok(true));
            assert!(state.has_message(held.id));
        }
        let kept = |state: &ServerState| {
            let mut list: Vec<TombstoneProof> = state.snapshot().tombstone_proofs;
            list.sort_by_key(|p| p.message);
            list
        };
        assert_eq!(kept(&a), kept(&b));
        let mut expected: Vec<TombstoneProof> = proofs[..MAX_UNAPPLIED_PROOFS - 1].to_vec();
        expected.push(waiting);
        expected.sort_by_key(|p| p.message);
        assert_eq!(kept(&a), expected);
    }

    #[test]
    fn leaving_or_a_ban_keeps_the_deletes_made_before() {
        let (mut state, channel, _) = fixture();
        let sid = state.server.id;
        let author = join(&mut state, "author");
        type Out = fn(ServerId, UserId, i64) -> ServerEvent;
        let ways: [(&str, Out); 2] = [
            ("leave", |server, user, at| ServerEvent::MemberLeave { server, user, at }),
            ("ban", |server, user, at| ServerEvent::MemberBan { server, user, at }),
        ];
        for (how, out) in ways {
            let member = join(&mut state, how);
            let theirs = post(&state, channel, author, "theirs");
            let own = post(&state, channel, member, "own");
            let gone = post(&state, channel, member, "gone");
            for m in [&theirs, &gone] {
                create(&mut state, m).unwrap();
            }
            for m in [&theirs, &own, &gone] {
                assert_eq!(state.apply(delete(&state, m, member)), Ok(true), "{how}");
            }
            assert_eq!(state.apply(out(sid, member, Utc::now().timestamp_millis() + 1)), Ok(true), "{how}");
            // They were the member's to make: the own one is still relayed,
            // and the one not theirs to make still waits.
            let relayed: Vec<MessageId> = state.tombstones_since(DateTime::<Utc>::MIN_UTC).map(|p| p.message).collect();
            assert!(relayed.contains(&own.id) && relayed.contains(&gone.id) && !relayed.contains(&theirs.id), "{how}");
            assert!(state.tombstone(theirs.id).is_some() && state.has_message(theirs.id), "{how}");
        }
    }

    #[test]
    fn a_delete_waits_for_the_role_that_allows_it() {
        let (mut state, channel, _) = fixture();
        let sid = state.server.id;
        let author = join(&mut state, "author");
        let moderator = join(&mut state, "moderator");
        let msg = post(&state, channel, author, "rude");
        create(&mut state, &msg).unwrap();
        // The role that lets the moderator delete it has not arrived yet.
        let by_moderator = proof(&state, &msg, moderator, Utc::now());
        assert_eq!(state.apply(by_moderator.clone().into_event()), Ok(true));
        assert_eq!(state.apply(by_moderator.clone().into_event()), Ok(false), "redelivery");
        assert!(state.has_message(msg.id));
        assert_eq!(state.tombstones_for(msg.id), vec![&by_moderator]);
        assert!(state.snapshot().accepted.is_empty());
        // Another copy of the message does not drop it, nor does a restart.
        assert_eq!(create(&mut state, &msg), Ok(false));
        let mut state = reload(&state);
        assert_eq!(state.tombstones_for(msg.id), vec![&by_moderator]);

        let role = crate::server::Role {
            id: crate::ids::RoleId::new(),
            name: "mods".into(),
            color: None,
            permissions: Permissions::MANAGE_MESSAGES,
            position: 1,
            hoist: false,
            self_assignable: false,
            tag: None,
            banner: None,
            updated_at: 0,
        };
        let rid = role.id;
        let later = crate::server::Role { id: crate::ids::RoleId::new(), ..role.clone() };
        for role in [role, later.clone()] {
            assert_eq!(state.apply(ServerEvent::RoleCreate { server: sid, role }), Ok(true));
        }
        assert!(state.has_message(msg.id));
        // A grant made after the delete does not cover it; one made before does.
        let made = by_moderator.at.timestamp_millis();
        assert_eq!(state.apply(ServerEvent::RoleAssign { server: sid, user: moderator, role: later.id, at: made + 1 }), Ok(true));
        assert!(state.has_message(msg.id));
        assert_eq!(state.apply(ServerEvent::RoleAssign { server: sid, user: moderator, role: rid, at: made - 1 }), Ok(true));
        assert!(!state.has_message(msg.id));
        assert_eq!(state.snapshot().accepted, vec![(msg.id, moderator, channel, author)]);
        assert_eq!(create(&mut state, &msg), Ok(false));
    }

    #[test]
    fn a_role_update_that_grants_manage_messages_applies_waiting_deletes() {
        let (mut state, channel, _) = fixture();
        let sid = state.server.id;
        let author = join(&mut state, "author");
        let member = join(&mut state, "member");
        let msg = post(&state, channel, author, "x");
        create(&mut state, &msg).unwrap();
        state.apply(delete(&state, &msg, member)).unwrap();
        assert!(state.has_message(msg.id));
        let everyone = crate::server::Role {
            permissions: Permissions::MANAGE_MESSAGES,
            updated_at: 1,
            ..state.server.roles[0].clone()
        };
        assert_eq!(state.apply(ServerEvent::RoleUpdate { server: sid, role: everyone }), Ok(true));
        assert!(!state.has_message(msg.id));
        assert_eq!(state.tombstone(msg.id).map(|p| p.by), Some(member));
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
}

#[cfg(test)]
mod role_tests {
    use super::*;
    use crate::test_support::{fixture, join, keyed_profile};

    fn role(name: &str, permissions: Permissions, updated_at: i64) -> Role {
        Role {
            id: RoleId::new(),
            name: name.into(),
            color: None,
            permissions,
            position: 1,
            hoist: false,
            self_assignable: false,
            tag: None,
            banner: None,
            updated_at,
        }
    }

    fn assign(state: &ServerState, user: UserId, role: RoleId, at: i64) -> ServerEvent {
        ServerEvent::RoleAssign { server: state.server.id, user, role, at }
    }

    fn unassign(state: &ServerState, user: UserId, role: RoleId, at: i64) -> ServerEvent {
        ServerEvent::RoleUnassign { server: state.server.id, user, role, at, since: None }
    }

    fn holds(state: &ServerState, user: UserId, role: RoleId) -> bool {
        state.server.member(user).is_some_and(|m| m.roles.contains(&role))
    }

    #[test]
    fn an_older_assign_does_not_undo_a_newer_unassign() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let member = join(&mut state, "member");
        let mods = role("mods", Permissions::MANAGE_MESSAGES, 1);
        state.apply(ServerEvent::RoleCreate { server: sid, role: mods.clone() }).unwrap();
        assert_eq!(state.apply(assign(&state, member, mods.id, 10)), Ok(true));
        assert_eq!(state.apply(unassign(&state, member, mods.id, 5)), Ok(false), "older");
        assert!(holds(&state, member, mods.id));
        assert_eq!(state.apply(unassign(&state, member, mods.id, 20)), Ok(true));
        assert_eq!(state.apply(assign(&state, member, mods.id, 10)), Ok(false), "stale grant");
        assert_eq!(state.apply(assign(&state, member, mods.id, 20)), Ok(false), "a removal wins a tie");
        assert!(!holds(&state, member, mods.id));
        assert!(!state.server.permissions_for(member).allows(Permissions::MANAGE_MESSAGES));
        assert_eq!(state.apply(assign(&state, member, mods.id, 21)), Ok(true));
        assert!(holds(&state, member, mods.id));

        // A removal that arrives before the grant it undoes is kept.
        let other = join(&mut state, "other");
        assert_eq!(state.apply(unassign(&state, other, mods.id, 30)), Ok(true));
        assert_eq!(state.apply(assign(&state, other, mods.id, 25)), Ok(false));
        assert!(!holds(&state, other, mods.id));

        // Our own change is dated after the last one, even one made by a
        // clock ahead of ours.
        let ahead = (Utc::now() + chrono::Duration::hours(1)).timestamp_millis();
        assert_eq!(state.apply(unassign(&state, other, mods.id, ahead)), Ok(true));
        let back = state.role_change(other, mods.id, true, Utc::now());
        assert_eq!(back, assign(&state, other, mods.id, ahead + 1));
        assert_eq!(state.apply(back), Ok(true));
    }

    #[test]
    fn the_newest_role_definition_wins() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let first = role("mods", Permissions::NONE, 10);
        state.apply(ServerEvent::RoleCreate { server: sid, role: first.clone() }).unwrap();
        let granted = Role { permissions: Permissions::MANAGE_MESSAGES, updated_at: 20, ..first.clone() };
        // A create of a known role is an update, so announcing it is enough.
        assert_eq!(state.apply(ServerEvent::RoleCreate { server: sid, role: granted.clone() }), Ok(true));
        let renamed = Role { name: "old name".into(), updated_at: 15, ..first.clone() };
        assert_eq!(state.apply(ServerEvent::RoleUpdate { server: sid, role: renamed }), Ok(false));
        assert_eq!(state.apply(ServerEvent::RoleCreate { server: sid, role: first.clone() }), Ok(false));
        assert_eq!(state.server.roles[1], granted);

        // Two changes made at the same time end the same on every replica.
        let a = Role { name: "a".into(), updated_at: 30, ..first.clone() };
        let b = Role { name: "b".into(), updated_at: 30, ..first.clone() };
        let mut other = state.clone();
        for (state, order) in [(&mut state, [&a, &b]), (&mut other, [&b, &a])] {
            for role in order {
                let _ = state.apply(ServerEvent::RoleUpdate { server: sid, role: role.clone() });
            }
        }
        assert_eq!(state.server.roles, other.server.roles);

        // Neither takes the everyone role's place, nor it theirs.
        let everyone = state.server.roles[0].clone();
        let moved = Role { position: 3, updated_at: 40, ..everyone };
        assert_eq!(state.apply(ServerEvent::RoleUpdate { server: sid, role: moved }), Ok(false));
        let shadow = Role { position: 0, updated_at: 40, ..first };
        assert_eq!(state.apply(ServerEvent::RoleCreate { server: sid, role: shadow }), Ok(false));
        let ahead = Role { updated_at: (Utc::now() + chrono::Duration::days(2)).timestamp_millis(), ..a };
        assert_eq!(state.apply(ServerEvent::RoleUpdate { server: sid, role: ahead }), Err(StoreError::TooFarAhead));
    }

    #[test]
    fn a_deleted_role_stays_deleted() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let gone = role("gone", Permissions::MANAGE_MESSAGES, 10);
        // The delete arrives before the create.
        assert_eq!(state.apply(ServerEvent::RoleDelete { server: sid, role: gone.id }), Ok(true));
        assert_eq!(state.apply(ServerEvent::RoleDelete { server: sid, role: gone.id }), Ok(false));
        assert_eq!(state.apply(ServerEvent::RoleCreate { server: sid, role: gone.clone() }), Ok(false));
        assert_eq!(state.apply(assign(&state, owner, gone.id, 20)), Ok(false));
        let restored = ServerState::from_snapshot(state.snapshot());
        assert_eq!(restored.server.deleted_roles, vec![gone.id]);
        let everyone = state.server.roles[0].id;
        assert_eq!(state.apply(ServerEvent::RoleDelete { server: sid, role: everyone }), Ok(false));
        assert_eq!(state.server.deleted_roles, vec![gone.id]);
    }

    #[test]
    fn role_and_member_marks_are_saved_and_pruned() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let [kept, removed, leaving, gone] = ["kept", "removed", "leaving", "gone"].map(|name| join(&mut state, name));
        let mods = role("mods", Permissions::MANAGE_MESSAGES, 10);
        state.apply(ServerEvent::RoleCreate { server: sid, role: mods.clone() }).unwrap();
        let now = Utc::now();
        let ms = now.timestamp_millis();
        let long_ago = (now - chrono::Duration::days(ROLE_MARK_KEEP_DAYS + 1)).timestamp_millis();
        state.apply(assign(&state, kept, mods.id, long_ago)).unwrap();
        state.apply(unassign(&state, removed, mods.id, long_ago)).unwrap();
        state.apply(assign(&state, leaving, mods.id, ms)).unwrap();
        state.apply(ServerEvent::MemberLeave { server: sid, user: leaving, at: ms + 1 }).unwrap();
        state.apply(ServerEvent::MemberLeave { server: sid, user: gone, at: long_ago }).unwrap();
        // Leaving turns the grant into a removal that remembers it.
        let ended = RoleMark { user: leaving, role: mods.id, held: false, at: ms + 1, since: Some(ms) };
        assert_eq!(state.role_mark(leaving, mods.id), Some(&ended));
        assert_eq!(state.member_mark(leaving).map(|m| (m.state, m.since)), Some((Membership::Left, Some(0))));

        let restored = ServerState::from_snapshot(serde_json::from_str(&serde_json::to_string(&state.snapshot()).unwrap()).unwrap());
        assert_eq!(restored.role_mark(removed, mods.id), state.role_mark(removed, mods.id));
        assert_eq!(restored.role_mark(leaving, mods.id), Some(&ended));
        assert_eq!(restored.role_marks().count(), 3);
        assert_eq!(restored.member_mark(gone), state.member_mark(gone));

        state.prune_expired(now);
        assert!(state.role_mark(kept, mods.id).is_some(), "a held role keeps its mark");
        assert!(state.role_mark(removed, mods.id).is_none());
        assert_eq!(state.role_mark(leaving, mods.id), Some(&ended));
        assert!(state.member_mark(gone).is_none() && state.member_mark(leaving).is_some());
        let ahead = (now + chrono::Duration::days(2)).timestamp_millis();
        assert_eq!(state.apply(assign(&state, kept, mods.id, ahead)), Err(StoreError::TooFarAhead));
        assert_eq!(state.apply(ServerEvent::MemberLeave { server: sid, user: kept, at: ahead }), Err(StoreError::TooFarAhead));
    }

    #[test]
    fn a_snapshot_from_before_role_marks_still_loads() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let mods = role("mods", Permissions::MANAGE_MESSAGES, 10);
        state.apply(ServerEvent::RoleCreate { server: sid, role: mods.clone() }).unwrap();
        state.apply(assign(&state, owner, mods.id, 10)).unwrap();
        let mut json = serde_json::to_value(state.snapshot()).unwrap();
        // Saved by this branch before marks remembered grants or joins.
        let mut earlier = json.clone();
        earlier.as_object_mut().unwrap().remove("member_marks");
        for mark in earlier["role_marks"].as_array_mut().unwrap() {
            mark.as_object_mut().unwrap().remove("since");
        }
        let earlier = ServerState::from_snapshot(serde_json::from_value(earlier).unwrap());
        assert_eq!(earlier.role_mark(owner, mods.id), state.role_mark(owner, mods.id));
        assert_eq!(earlier.member_marks().count(), 0);
        json.as_object_mut().unwrap().remove("role_marks");
        json.as_object_mut().unwrap().remove("member_marks");
        json["server"].as_object_mut().unwrap().remove("deleted_roles");
        for role in json["server"]["roles"].as_array_mut().unwrap() {
            role.as_object_mut().unwrap().remove("updated_at");
        }
        let mut restored = ServerState::from_snapshot(serde_json::from_value(json).unwrap());
        assert!(holds(&restored, owner, mods.id));
        assert_eq!(restored.role_marks().count(), 0);
        assert!(restored.server.roles.iter().all(|r| r.updated_at == 0));
        // Any dated change replaces what was held before marks were kept.
        assert_eq!(restored.apply(unassign(&restored, owner, mods.id, 1)), Ok(true));
        let renamed = Role { name: "renamed".into(), updated_at: 1, ..mods };
        assert_eq!(restored.apply(ServerEvent::RoleUpdate { server: sid, role: renamed }), Ok(true));
    }

    fn leave(state: &ServerState, user: UserId, at: i64) -> ServerEvent {
        ServerEvent::MemberLeave { server: state.server.id, user, at }
    }

    #[test]
    fn a_kicked_member_who_rejoins_gets_no_old_role_back() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let member = join(&mut state, "member");
        let profile = state.profiles[&member].clone();
        let mods = role("mods", Permissions::MANAGE_MESSAGES, 1);
        state.apply(ServerEvent::RoleCreate { server: sid, role: mods.clone() }).unwrap();
        // One replica missed the grant; a manager that missed the kick
        // announces it again after the rejoin.
        let mut missed = state.clone();
        let grant = assign(&state, member, mods.id, 10);
        state.apply(grant.clone()).unwrap();
        for replica in [&mut state, &mut missed] {
            assert_eq!(replica.apply(leave(replica, member, 20)), Ok(true));
            let rejoin = ServerEvent::MemberJoin { server: sid, profile: profile.clone(), at: 30 };
            assert_eq!(replica.apply(rejoin), Ok(true));
            assert_eq!(replica.apply(grant.clone()), Ok(false));
            assert!(!holds(replica, member, mods.id));
            assert!(!replica.server.permissions_for(member).allows(Permissions::MANAGE_MESSAGES));
        }
        let ended = RoleMark { user: member, role: mods.id, held: false, at: 20, since: Some(10) };
        assert_eq!(state.role_mark(member, mods.id), Some(&ended), "the kick ended the grant");
        // A real grant after the rejoin still wins, dated after the join
        // even by a clock behind it.
        for replica in [&mut state, &mut missed] {
            let regrant = replica.role_change(member, mods.id, true, DateTime::from_timestamp_millis(25).unwrap());
            assert_eq!(regrant, assign(replica, member, mods.id, 30));
            assert_eq!(replica.apply(regrant), Ok(true));
            assert!(holds(replica, member, mods.id));
        }
    }

    #[test]
    fn a_stale_join_does_not_undo_a_kick() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let profile = keyed_profile("member");
        let user = profile.id;
        let join_at = |at| ServerEvent::MemberJoin { server: sid, profile: profile.clone(), at };
        // One replica never saw the join and gets the kick first.
        let mut missed = state.clone();
        assert_eq!(state.apply(join_at(10)), Ok(true));
        for replica in [&mut state, &mut missed] {
            assert_eq!(replica.apply(leave(replica, user, 20)), Ok(true));
            assert_eq!(replica.apply(join_at(10)), Ok(false));
            assert_eq!(replica.apply(join_at(20)), Ok(false), "a removal wins a tie");
            assert!(!replica.server.is_member(user));
            assert_eq!(replica.member_mark(user).map(|m| (m.state, m.at)), Some((Membership::Left, 20)));
            // A real rejoin after the kick brings them back, and the kick
            // announced again changes nothing.
            assert_eq!(replica.apply(join_at(30)), Ok(true));
            assert_eq!(replica.apply(leave(replica, user, 20)), Ok(false));
            assert!(replica.server.is_member(user));
            assert_eq!(replica.joined_at(user), Some(30));
        }
        // A ban is not undone by a stale join either.
        let ban = ServerEvent::MemberBan { server: sid, user, at: 40 };
        assert_eq!(state.apply(ban.clone()), Ok(true));
        assert_eq!(state.apply(join_at(30)), Ok(false));
        assert!(!state.server.is_member(user) && state.server.banned.contains(&user));
        // Of a join and a leave made at once the leave wins, in either order.
        let other = keyed_profile("other");
        let join_other = ServerEvent::MemberJoin { server: sid, profile: other.clone(), at: 50 };
        let mut leave_first = state.clone();
        assert_eq!(state.apply(join_other.clone()), Ok(true));
        assert_eq!(state.apply(leave(&state, other.id, 50)), Ok(true));
        assert_eq!(leave_first.apply(leave(&leave_first, other.id, 50)), Ok(true));
        assert_eq!(leave_first.apply(join_other), Ok(false));
        assert!(!state.server.is_member(other.id) && !leave_first.server.is_member(other.id));
        // Our own change is dated after the last one known.
        assert_eq!(state.member_change_time(user, DateTime::from_timestamp_millis(5).unwrap()), 41);
    }

    #[test]
    fn changes_since_come_by_priority_and_page() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let mods = role("mods", Permissions::MANAGE_MESSAGES, 100);
        let badge = role("badge", Permissions::NONE, 50);
        let old = role("old", Permissions::NONE, 1);
        for role in [&mods, &badge, &old] {
            state.apply(ServerEvent::RoleCreate { server: sid, role: role.clone() }).unwrap();
        }
        let [a, b, c] = ["a", "b", "c"].map(keyed_profile);
        // A replica that misses all of it.
        let mut missed = state.clone();
        let events = [
            ServerEvent::MemberJoin { server: sid, profile: a.clone(), at: 200 },
            ServerEvent::MemberJoin { server: sid, profile: b.clone(), at: 300 },
            ServerEvent::MemberBan { server: sid, user: c.id, at: 350 },
            leave(&state, b.id, 400),
            assign(&state, a.id, mods.id, 250),
            assign(&state, a.id, badge.id, 260),
        ];
        for event in events {
            assert_eq!(state.apply(event), Ok(true));
        }
        let all: Vec<ServerEvent> = state.changes_since(10, None).into_iter().map(|(_, e)| e).collect();
        let expected = vec![
            ServerEvent::RoleCreate { server: sid, role: mods.clone() },
            ServerEvent::RoleCreate { server: sid, role: badge.clone() },
            assign(&state, a.id, mods.id, 250),
            leave(&state, b.id, 400),
            ServerEvent::MemberBan { server: sid, user: c.id, at: 350 },
            ServerEvent::MemberJoin { server: sid, profile: state.profiles[&a.id].clone(), at: 200 },
            assign(&state, a.id, badge.id, 260),
        ];
        assert_eq!(all, expected);
        // Paging goes on after the last key sent.
        let first = state.changes_since(10, None);
        let rest = state.changes_since(10, Some(first[2].0));
        let paged: Vec<ServerEvent> = first[..3].iter().chain(&rest).map(|(_, e)| e.clone()).collect();
        assert_eq!(paged, expected);
        assert!(state.changes_since(10, first.last().map(|(k, _)| *k)).is_empty());
        assert_eq!(state.changes_since(300, None).len(), 2, "only what changed since");

        // The replica that missed it all learns the members from their
        // joins; a grant that came before the join takes the next round.
        for round in 0..2 {
            for (_, event) in state.changes_since(0, None) {
                let _ = missed.apply(event);
            }
            assert_eq!(missed.identity_of(a.id), a.identity);
            assert_eq!(holds(&missed, a.id, mods.id), round == 1);
            assert!(holds(&missed, a.id, badge.id));
        }
        let users = |s: &ServerState| s.server.members.iter().map(|m| m.user).collect::<Vec<_>>();
        assert_eq!(users(&missed), users(&state));
        assert!(!missed.server.is_member(b.id) && missed.server.banned == vec![c.id]);
    }
}
