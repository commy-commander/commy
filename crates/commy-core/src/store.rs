use crate::event::ServerEvent;
use crate::identity::{IdentityKey, Signature};
use crate::ids::{ChannelId, MessageId, UserId};
use crate::message::{Message, TombstoneProof};
use crate::permissions::Permissions;
use crate::server::{ChannelKind, Member, Server};
use crate::user::{Presence, UserProfile};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
}

/// Days a delete proof is kept, well past the catch-up window.
pub const TOMBSTONE_KEEP_DAYS: i64 = 60;

/// Most delete proofs kept for one message, each from a different
/// deleter.
pub const MAX_PROOFS_PER_MESSAGE: usize = 4;

/// A delete proof as held here. `accepted_for` is the author of the
/// message this peer checked the proof against when it applied it. Such
/// a proof keeps that message out even if the deleter later loses the
/// right to delete it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredProof {
    proof: TombstoneProof,
    accepted_for: Option<UserId>,
}

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
    /// Ids of messages deleted before deletes were signed, so a late
    /// copy cannot bring one back.
    deleted_messages: std::collections::HashSet<MessageId>,
    /// Signed deletes by message id, at most one per deleter. They keep
    /// late copies out and are relayed by catch-up.
    tombstones: HashMap<MessageId, Vec<StoredProof>>,
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
    /// (message, deleter, author) of each proof this peer accepted.
    #[serde(default)]
    pub accepted_proofs: Vec<(MessageId, UserId, UserId)>,
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
            accepted_proofs: self
                .stored_proofs()
                .filter_map(|s| s.accepted_for.map(|author| (s.proof.message, s.proof.by, author)))
                .collect(),
        }
    }

    /// Loads a saved image as-is. Nothing is verified here: local data
    /// is trusted, including unsigned messages from older versions.
    pub fn from_snapshot(snapshot: Snapshot) -> Self {
        let mut state = Self::new(snapshot.server);
        for profile in snapshot.profiles {
            let mut profile = profile;
            // Nobody is online at load time until presence arrives.
            profile.presence = Presence::Offline;
            state.profiles.insert(profile.id, profile);
        }
        state.deleted_messages = snapshot.deleted_messages.into_iter().collect();
        let accepted: HashMap<(MessageId, UserId), UserId> = snapshot
            .accepted_proofs
            .into_iter()
            .map(|(message, by, author)| ((message, by), author))
            .collect();
        for proof in snapshot.tombstone_proofs {
            // Proofs for channels deleted since are of no use.
            if state.messages.contains_key(&proof.channel) {
                let accepted_for = accepted.get(&(proof.message, proof.by)).copied();
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
    /// proofs for channels that no longer exist.
    pub fn prune_expired(&mut self, now: DateTime<Utc>) -> usize {
        let mut removed = 0;
        for channel in &self.server.channels {
            let Some(cutoff) = retention_cutoff(channel, now) else { continue };
            if let Some(list) = self.messages.get_mut(&channel.id) {
                let before = list.len();
                list.retain(|m| m.created_at >= cutoff);
                removed += before - list.len();
            }
        }
        let keep_from = now - chrono::Duration::days(TOMBSTONE_KEEP_DAYS);
        let channels = &self.messages;
        retain_proofs(&mut self.tombstones, |p| p.at >= keep_from && channels.contains_key(&p.channel));
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
        self.messages.values().flat_map(|list| list.iter()).find(|m| m.id == id)
    }

    pub fn has_message(&self, id: MessageId) -> bool {
        self.message(id).is_some()
    }

    /// Best delete proof stored for a message id, if any. See
    /// [`Self::tombstones_for`] for the order.
    pub fn tombstone(&self, id: MessageId) -> Option<&TombstoneProof> {
        let list = self.tombstones.get(&id)?;
        list.iter().min_by_key(|s| proof_rank(&self.server, s)).map(|s| &s.proof)
    }

    /// Every delete proof stored for a message id, one per deleter, best
    /// first: accepted here, then a moderator's, then the earliest, then
    /// the lowest signature.
    pub fn tombstones_for(&self, id: MessageId) -> Vec<&TombstoneProof> {
        let mut list: Vec<&StoredProof> = self.tombstones.get(&id).into_iter().flatten().collect();
        list.sort_by_key(|s| proof_rank(&self.server, s));
        list.into_iter().map(|s| &s.proof).collect()
    }

    /// Delete proofs made at or after `since`, in no particular order. A
    /// message deleted by several people has one proof for each.
    pub fn tombstones_since(&self, since: DateTime<Utc>) -> impl Iterator<Item = &TombstoneProof> {
        self.stored_proofs().map(|s| &s.proof).filter(move |p| p.at >= since)
    }

    fn stored_proofs(&self) -> impl Iterator<Item = &StoredProof> {
        self.tombstones.values().flatten()
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
            ServerEvent::MemberJoin { server, mut profile } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                if self.server.banned.contains(&profile.id) {
                    return Ok(false);
                }
                self.pin_identity(&mut profile)?;
                if self.server.is_member(profile.id) {
                    self.profiles.insert(profile.id, profile);
                    return Ok(false);
                }
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
                Ok(self.server.members.len() != before)
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
                retain_proofs(&mut self.tombstones, |p| p.channel != channel);
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
            ServerEvent::MemberBan { server, user } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                if user == self.server.owner || self.server.banned.contains(&user) {
                    return Ok(false);
                }
                self.server.banned.push(user);
                self.server.members.retain(|m| m.user != user);
                self.voice_states.remove(&user);
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

    /// Signed create. A copy of a message we already hold is taken only
    /// as a newer signed edit of it.
    fn create_message(&mut self, message: Message) -> Result<bool, StoreError> {
        let key = self
            .identity_of(message.author)
            .ok_or(StoreError::UnknownAuthorKey(message.author))?;
        if !message.verify(self.server.id, &key) {
            return Err(StoreError::BadSignature);
        }
        if !self.messages.contains_key(&message.channel) {
            return Err(StoreError::UnknownChannel);
        }
        if let Some(mut list) = self.tombstones.remove(&message.id) {
            // A proof accepted here for this author keeps it out, as does
            // one the author or a moderator signed. Any other is void.
            let author = message.author;
            let mut blocked = false;
            for s in &mut list {
                if s.proof.channel == message.channel
                    && (s.accepted_for == Some(author) || self.may_delete(s.proof.by, author))
                {
                    s.accepted_for.get_or_insert(author);
                    blocked = true;
                }
            }
            list.retain(|s| s.accepted_for.is_some());
            if !list.is_empty() {
                self.tombstones.insert(message.id, list);
            }
            if blocked {
                return Ok(false);
            }
        }
        if self.deleted_messages.contains(&message.id) {
            return Ok(false);
        }
        // A late copy of an already-expired message stays gone.
        let now = Utc::now();
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
        let list = self
            .messages
            .get_mut(&message.channel)
            .ok_or(StoreError::UnknownChannel)?;
        // Keep messages sorted by creation time for display.
        let pos = list.partition_point(|m| m.created_at <= message.created_at);
        list.insert(pos, message);
        Ok(true)
    }

    /// Signed edit by the author; older edits than the stored one lose.
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
        if !msg.verify_edit(server, &content, edited_at, &sig, &key) {
            return Err(StoreError::BadSignature);
        }
        if Some(edited_at.timestamp_millis()) <= millis(msg.edited_at) {
            return Ok(false);
        }
        msg.content = content;
        msg.edited_at = Some(edited_at);
        msg.sig = Some(sig);
        Ok(true)
    }

    /// Signed delete. Without the message at hand the proof is kept and
    /// checked against the author if the message turns up later.
    fn delete_message(&mut self, proof: TombstoneProof) -> Result<bool, StoreError> {
        let key = self.identity_of(proof.by).ok_or(StoreError::UnknownAuthorKey(proof.by))?;
        if !proof.verify(self.server.id, &key) {
            return Err(StoreError::BadSignature);
        }
        let list = self.messages.get(&proof.channel).ok_or(StoreError::UnknownChannel)?;
        if let Some(pos) = list.iter().position(|m| m.id == proof.message) {
            let author = list[pos].author;
            if !self.may_delete(proof.by, author) {
                return Err(StoreError::NotAllowed);
            }
            if let Some(list) = self.messages.get_mut(&proof.channel) {
                list.remove(pos);
            }
            self.store_proof(proof, Some(author));
            return Ok(true);
        }
        Ok(self.store_proof(proof, None))
    }

    /// Keeps a proof next to the others for its message: one per deleter
    /// and at most [`MAX_PROOFS_PER_MESSAGE`], the best ranked winning.
    /// Returns whether anything changed.
    fn store_proof(&mut self, proof: TombstoneProof, accepted_for: Option<UserId>) -> bool {
        let server = &self.server;
        let rank = |s: &StoredProof| proof_rank(server, s);
        let mut new = StoredProof { proof, accepted_for };
        let list = self.tombstones.entry(new.proof.message).or_default();
        if let Some(old) = list.iter_mut().find(|s| s.proof.by == new.proof.by) {
            let mut changed = false;
            if old.proof.channel == new.proof.channel {
                // The same delete made twice (two devices, a retry) keeps
                // the acceptance either one had.
                new.accepted_for = new.accepted_for.or(old.accepted_for);
                changed = old.accepted_for != new.accepted_for;
                old.accepted_for = new.accepted_for;
            }
            if rank(&new) < rank(old) {
                *old = new;
                return true;
            }
            return changed;
        }
        if list.len() < MAX_PROOFS_PER_MESSAGE {
            list.push(new);
            return true;
        }
        match list.iter_mut().max_by_key(|s| rank(s)) {
            Some(worst) if rank(&new) < rank(worst) => {
                *worst = new;
                true
            }
            _ => false,
        }
    }

    /// The author may delete their own message; Manage Messages may
    /// delete anyone's.
    fn may_delete(&self, by: UserId, author: UserId) -> bool {
        by == author || self.is_moderator(by)
    }

    fn is_moderator(&self, user: UserId) -> bool {
        self.server.permissions_for(user).allows(Permissions::MANAGE_MESSAGES)
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
/// here, then a moderator's, then the earliest, then the lowest
/// signature, so peers holding the same proofs keep the same ones.
fn proof_rank(server: &Server, stored: &StoredProof) -> (bool, bool, DateTime<Utc>, [u8; 64]) {
    let moderator = server.permissions_for(stored.proof.by).allows(Permissions::MANAGE_MESSAGES);
    (stored.accepted_for.is_none(), !moderator, stored.proof.at, stored.proof.sig.0)
}

/// Keeps only the proofs `keep` accepts, dropping emptied entries.
fn retain_proofs(map: &mut HashMap<MessageId, Vec<StoredProof>>, keep: impl Fn(&TombstoneProof) -> bool) {
    map.retain(|_, list| {
        list.retain(|s| keep(&s.proof));
        !list.is_empty()
    });
}

/// Edit time in ms; None (never edited) orders first.
fn millis(t: Option<DateTime<Utc>>) -> Option<i64> {
    t.map(|t| t.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ServerId;
    use crate::test_support::{delete, edit, fixture, identity_for, join, post, proof, signed};

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

    /// Adds a member holding a role with Manage Messages.
    fn join_moderator(state: &mut ServerState) -> UserId {
        let sid = state.server.id;
        let user = join(state, "moderator");
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
        };
        let rid = role.id;
        state.apply(ServerEvent::RoleCreate { server: sid, role }).unwrap();
        state.apply(ServerEvent::RoleAssign { server: sid, user, role: rid }).unwrap();
        user
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
        let mallory = join(&mut state, "mallory");
        let original = post(&state, channel, owner, "mine");
        create(&mut state, &original).unwrap();
        let mut fake = Message::new(channel, mallory, "hijacked");
        fake.id = original.id;
        fake.edited_at = Some(Utc::now());
        let fake = signed(&state, fake);
        assert_eq!(create(&mut state, &fake), Ok(false));
        assert_eq!(state.message(original.id).unwrap().content, "mine");

        // Ids are unique across channels too.
        let voice = state.server.channels[1].id;
        let mut copy = Message::new(voice, mallory, "dup");
        copy.id = original.id;
        let copy = signed(&state, copy);
        assert_eq!(create(&mut state, &copy), Ok(false));
        assert!(state.messages(voice).is_empty());
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
        let moderator = join_moderator(&mut state);
        let stranger = join(&mut state, "stranger");
        let (a, b, c) = (
            post(&state, channel, author, "a"),
            post(&state, channel, author, "b"),
            post(&state, channel, author, "c"),
        );
        for m in [&a, &b, &c] {
            create(&mut state, m).unwrap();
        }

        assert_eq!(state.apply(delete(&state, &a, stranger)), Err(StoreError::NotAllowed));
        assert!(state.has_message(a.id) && state.tombstone(a.id).is_none());
        assert_eq!(state.apply(delete(&state, &a, author)), Ok(true));
        assert_eq!(state.apply(delete(&state, &b, moderator)), Ok(true));
        assert_eq!(state.tombstone(b.id).map(|p| p.by), Some(moderator));
        assert!(!state.has_message(a.id) && !state.has_message(b.id));

        let keyless = UserProfile::new("keyless");
        let kid = keyless.id;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: keyless }).unwrap();
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
        let moderator = join_moderator(&mut state);
        let stranger = join(&mut state, "stranger");

        // A stranger's proof is kept, then voided when the message shows up.
        let one = post(&state, channel, author, "one");
        assert_eq!(state.apply(delete(&state, &one, stranger)), Ok(true));
        assert!(state.tombstone(one.id).is_some());
        assert_eq!(create(&mut state, &one), Ok(true));
        assert!(state.has_message(one.id) && state.tombstone(one.id).is_none());

        // A moderator's proof keeps it out.
        let two = post(&state, channel, author, "two");
        assert_eq!(state.apply(delete(&state, &two, moderator)), Ok(true));
        assert_eq!(create(&mut state, &two), Ok(false));

        // A proof naming another channel does not apply.
        let voice = state.server.channels[1].id;
        let three = post(&state, channel, author, "three");
        let elsewhere = TombstoneProof::sign(sid, voice, three.id, author, Utc::now(), &identity_for(author));
        assert_eq!(state.apply(elsewhere.into_event()), Ok(true));
        assert_eq!(create(&mut state, &three), Ok(true));

        // Each deleter's proof is kept, a moderator's ranking first.
        let four = post(&state, channel, author, "four");
        assert_eq!(state.apply(delete(&state, &four, stranger)), Ok(true));
        assert_eq!(state.apply(delete(&state, &four, moderator)), Ok(true));
        assert_eq!(state.apply(delete(&state, &four, author)), Ok(true));
        assert_eq!(state.tombstone(four.id).map(|p| p.by), Some(moderator));
        assert_eq!(create(&mut state, &four), Ok(false));
        let left: Vec<UserId> = state.tombstones_for(four.id).iter().map(|p| p.by).collect();
        assert_eq!(left, vec![moderator, author], "the stranger's proof is void");
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
            let moderator = join_moderator(&mut state);
            let msg = post(&state, channel, author, how);
            create(&mut state, &msg).unwrap();
            assert_eq!(state.apply(delete(&state, &msg, moderator)), Ok(true));
            assert_eq!(state.apply(demote(sid, moderator, mods_role(&state, moderator))), Ok(true));
            assert!(!state.is_moderator(moderator), "{how}");
            assert_eq!(create(&mut state, &msg), Ok(false), "{how}");
            assert_eq!(state.tombstone(msg.id).map(|p| p.by), Some(moderator), "{how}");
            assert_eq!(create(&mut reload(&state), &msg), Ok(false), "{how}, after a restart");
        }
    }

    #[test]
    fn early_proof_binds_once_checked_against_the_message() {
        let (mut state, channel, _) = fixture();
        let sid = state.server.id;
        let author = join(&mut state, "author");
        let moderator = join_moderator(&mut state);
        let role = mods_role(&state, moderator);
        let checked = post(&state, channel, author, "checked");
        let unchecked = post(&state, channel, author, "unchecked");
        state.apply(delete(&state, &checked, moderator)).unwrap();
        state.apply(delete(&state, &unchecked, moderator)).unwrap();
        // The message turns up while its deleter is still a moderator.
        assert_eq!(create(&mut state, &checked), Ok(false));
        state.apply(ServerEvent::RoleUnassign { server: sid, user: moderator, role }).unwrap();
        assert_eq!(create(&mut state, &checked), Ok(false));
        // A proof never checked is judged by current permissions.
        assert_eq!(create(&mut state, &unchecked), Ok(true));
        assert!(state.tombstone(unchecked.id).is_none());
        let later = post(&state, channel, author, "later");
        create(&mut state, &later).unwrap();
        assert_eq!(state.apply(delete(&state, &later, moderator)), Err(StoreError::NotAllowed));
    }

    #[test]
    fn acceptance_does_not_cover_another_author_reusing_the_id() {
        let (mut state, channel, _) = fixture();
        let author = join(&mut state, "author");
        let mallory = join(&mut state, "mallory");
        let real = post(&state, channel, author, "real");
        let mut decoy = Message::new(channel, mallory, "decoy");
        decoy.id = real.id;
        let decoy = signed(&state, decoy);
        create(&mut state, &decoy).unwrap();
        assert_eq!(state.apply(delete(&state, &decoy, mallory)), Ok(true));
        assert_eq!(create(&mut state, &decoy), Ok(false));
        assert_eq!(create(&mut state, &real), Ok(true));
    }

    #[test]
    fn snapshot_without_accepted_proofs_still_loads() {
        let (mut state, channel, _) = fixture();
        let author = join(&mut state, "author");
        let moderator = join_moderator(&mut state);
        let msg = post(&state, channel, author, "rude");
        create(&mut state, &msg).unwrap();
        state.apply(delete(&state, &msg, moderator)).unwrap();
        let mut json = serde_json::to_value(state.snapshot()).unwrap();
        assert_eq!(json["accepted_proofs"].as_array().map(Vec::len), Some(1));
        json.as_object_mut().unwrap().remove("accepted_proofs");
        let mut restored = ServerState::from_snapshot(serde_json::from_value(json).unwrap());
        assert_eq!(restored.tombstone(msg.id), state.tombstone(msg.id));
        assert_eq!(create(&mut restored, &msg), Ok(false), "still a moderator");
    }

    #[test]
    fn a_strangers_proof_does_not_shut_out_the_authors() {
        let (mut state, channel, _) = fixture();
        let author = join(&mut state, "author");
        let mallory = join(&mut state, "mallory");
        let msg = post(&state, channel, author, "mine");
        assert_eq!(state.apply(delete(&state, &msg, mallory)), Ok(true));
        let own = proof(&state, &msg, author, Utc::now());
        assert_eq!(state.apply(own.clone().into_event()), Ok(true));
        assert_eq!(state.apply(own.clone().into_event()), Ok(false), "redelivery");
        assert_eq!(state.tombstones_for(msg.id).len(), 2);
        assert_eq!(create(&mut state, &msg), Ok(false));
        assert_eq!(state.tombstones_for(msg.id), vec![&own], "the stranger's proof is void");
    }

    #[test]
    fn proofs_per_message_are_capped_keeping_a_moderators() {
        let (mut state, channel, _) = fixture();
        let author = join(&mut state, "author");
        let moderator = join_moderator(&mut state);
        let strangers: Vec<UserId> =
            (0..=MAX_PROOFS_PER_MESSAGE).map(|i| join(&mut state, &format!("s{i}"))).collect();
        let msg = post(&state, channel, author, "busy");
        let now = Utc::now();
        for (i, s) in strangers.iter().enumerate() {
            let p = proof(&state, &msg, *s, now + chrono::Duration::seconds(i as i64));
            assert_eq!(state.apply(p.into_event()), Ok(i < MAX_PROOFS_PER_MESSAGE), "proof {i}");
        }
        // A moderator's proof displaces the latest stranger's.
        let by_mod = proof(&state, &msg, moderator, now + chrono::Duration::days(1));
        assert_eq!(state.apply(by_mod.into_event()), Ok(true));
        let kept: Vec<UserId> = state.tombstones_for(msg.id).iter().map(|p| p.by).collect();
        assert_eq!(kept, [&[moderator], &strangers[..MAX_PROOFS_PER_MESSAGE - 1]].concat());
        assert_eq!(create(&mut state, &msg), Ok(false));
    }

    #[test]
    fn the_same_delete_made_twice_settles_on_the_earliest() {
        let (mut a, channel, _) = fixture();
        let author = join(&mut a, "author");
        let mut b = a.clone();
        let msg = post(&a, channel, author, "twice");
        create(&mut a, &msg).unwrap();
        // Deleted from two devices either side of UTC midnight.
        let midnight = Utc::now().date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
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
            (0..MAX_PROOFS_PER_MESSAGE + 2).map(|i| join(&mut a, &format!("s{i}"))).collect();
        let mut b = a.clone();
        let msg = post(&a, channel, author, "contested");
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
        expected.truncate(MAX_PROOFS_PER_MESSAGE);
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
        let legacy = Message::new(channel, owner, "old");
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
        json.as_object_mut().unwrap().remove("accepted_proofs");
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
        state.apply(proof(&state, &new, owner, now).into_event()).unwrap();
        let recent: Vec<MessageId> = state.tombstones_since(now - chrono::Duration::days(1)).map(|p| p.message).collect();
        assert_eq!(recent, vec![new.id]);
        assert_eq!(state.tombstones_since(now - chrono::Duration::days(5)).count(), 2);
        assert_eq!(state.tombstone(old.id).map(|p| p.by), Some(owner));
        assert!(!state.has_message(old.id));
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
}
