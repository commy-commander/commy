use crate::event::ServerEvent;
use crate::ids::{ChannelId, MessageId, UserId};
use crate::message::Message;
use crate::server::{ChannelKind, Member, Server};
use crate::user::{Presence, UserProfile};
use chrono::Utc;
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
    /// Ids of deleted messages, so a late copy cannot bring one back.
    deleted_messages: std::collections::HashSet<MessageId>,
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
        }
    }

    pub fn from_snapshot(snapshot: Snapshot) -> Self {
        let mut state = Self::new(snapshot.server);
        for profile in snapshot.profiles {
            let mut profile = profile;
            // Nobody is online at load time until presence arrives.
            profile.presence = Presence::Offline;
            state.profiles.insert(profile.id, profile);
        }
        state.deleted_messages = snapshot.deleted_messages.into_iter().collect();
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
    pub fn prune_expired(&mut self, now: chrono::DateTime<Utc>) -> usize {
        let mut removed = 0;
        for channel in &self.server.channels {
            let Some(days) = channel.retention_days.filter(|d| *d > 0) else { continue };
            let cutoff = now - chrono::Duration::days(days as i64);
            if let Some(list) = self.messages.get_mut(&channel.id) {
                let before = list.len();
                list.retain(|m| m.created_at >= cutoff);
                removed += before - list.len();
            }
        }
        removed
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
            ServerEvent::MessageCreate { message } => {
                let list = self
                    .messages
                    .get_mut(&message.channel)
                    .ok_or(StoreError::UnknownChannel)?;
                if self.deleted_messages.contains(&message.id) {
                    return Ok(false);
                }
                if list.iter().any(|m| m.id == message.id) {
                    return Ok(false);
                }
                // A late copy of an already-expired message stays gone.
                let expired = self
                    .server
                    .channel(message.channel)
                    .and_then(|c| c.retention_days)
                    .filter(|d| *d > 0)
                    .is_some_and(|d| message.created_at < Utc::now() - chrono::Duration::days(d as i64));
                if expired {
                    return Ok(false);
                }
                // Keep messages sorted by creation time for display.
                let pos = list.partition_point(|m| m.created_at <= message.created_at);
                list.insert(pos, message);
                Ok(true)
            }
            ServerEvent::MessageEdit {
                channel,
                message,
                content,
            } => {
                let msg = self.find_message_mut(channel, message)?;
                if msg.content == content {
                    return Ok(false);
                }
                msg.content = content;
                msg.edited_at = Some(Utc::now());
                Ok(true)
            }
            ServerEvent::MessageDelete { channel, message } => {
                let list = self
                    .messages
                    .get_mut(&channel)
                    .ok_or(StoreError::UnknownChannel)?;
                let before = list.len();
                list.retain(|m| m.id != message);
                let removed = list.len() != before;
                Ok(self.deleted_messages.insert(message) || removed)
            }
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
            ServerEvent::MemberJoin { server, profile } => {
                if server != self.server.id {
                    return Err(StoreError::WrongServer);
                }
                if self.server.banned.contains(&profile.id) {
                    return Ok(false);
                }
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
                let parent_ok = parent.map_or(true, |p| {
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
            ServerEvent::ProfileUpdate { profile } => {
                if !self.server.is_member(profile.id) {
                    return Ok(false);
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ServerId;
    use crate::server::Server;

    fn fixture() -> (ServerState, ChannelId, UserId) {
        let owner = UserId::new();
        let server = Server::new("test", owner);
        let channel = server.channels[0].id;
        (ServerState::new(server), channel, owner)
    }

    #[test]
    fn message_create_and_read_back() {
        let (mut state, channel, owner) = fixture();
        let msg = Message::new(channel, owner, "hello world");
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: msg.clone() }), Ok(true));
        assert_eq!(state.messages(channel), &[msg]);
    }

    #[test]
    fn duplicate_message_is_ignored() {
        let (mut state, channel, owner) = fixture();
        let msg = Message::new(channel, owner, "hello");
        state.apply(ServerEvent::MessageCreate { message: msg.clone() }).unwrap();
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: msg }), Ok(false));
        assert_eq!(state.messages(channel).len(), 1);
    }

    #[test]
    fn message_to_unknown_channel_fails() {
        let (mut state, _, owner) = fixture();
        let msg = Message::new(ChannelId::new(), owner, "lost");
        assert_eq!(
            state.apply(ServerEvent::MessageCreate { message: msg }),
            Err(StoreError::UnknownChannel)
        );
    }

    #[test]
    fn edit_updates_content_and_timestamp() {
        let (mut state, channel, owner) = fixture();
        let msg = Message::new(channel, owner, "typo");
        let id = msg.id;
        state.apply(ServerEvent::MessageCreate { message: msg }).unwrap();
        state
            .apply(ServerEvent::MessageEdit {
                channel,
                message: id,
                content: "fixed".to_string(),
            })
            .unwrap();
        let stored = &state.messages(channel)[0];
        assert_eq!(stored.content, "fixed");
        assert!(stored.edited_at.is_some());
    }

    #[test]
    fn delete_removes_message() {
        let (mut state, channel, owner) = fixture();
        let msg = Message::new(channel, owner, "bye");
        let id = msg.id;
        state.apply(ServerEvent::MessageCreate { message: msg }).unwrap();
        assert_eq!(state.apply(ServerEvent::MessageDelete { channel, message: id }), Ok(true));
        assert!(state.messages(channel).is_empty());
        // Redelivery of the same delete is a no-op.
        assert_eq!(state.apply(ServerEvent::MessageDelete { channel, message: id }), Ok(false));
    }

    #[test]
    fn pagination_returns_latest_window() {
        let (mut state, channel, owner) = fixture();
        for i in 0..10 {
            let msg = Message::new(channel, owner, format!("m{i}"));
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
                    message: Message::new(channel, owner, format!("m{i}")),
                })
                .unwrap();
        }
        let json = serde_json::to_string(&state.snapshot()).unwrap();
        let restored = ServerState::from_snapshot(serde_json::from_str(&json).unwrap());
        assert_eq!(restored.server.id, sid);
        assert_eq!(restored.messages(channel).len(), 5);
        assert_eq!(restored.messages(channel)[4].content, "m4");
        assert_eq!(restored.profiles.len(), 1);
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
                message: Message::new(channel, owner, "hi"),
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
        let list = state.member_list();
        assert_eq!(list[0].username, "zzz_online");
        assert_eq!(list[1].username, "aaa_offline");
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
        let msg = Message::new(channel, owner, "oops");
        state.apply(ServerEvent::MessageCreate { message: msg.clone() }).unwrap();
        assert_eq!(state.apply(ServerEvent::MessageDelete { channel, message: msg.id }), Ok(true));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: msg.clone() }), Ok(false));
        assert!(state.messages(channel).is_empty());
        // Repeat deletes are no-ops; tombstones survive a save and load.
        assert_eq!(state.apply(ServerEvent::MessageDelete { channel, message: msg.id }), Ok(false));
        let json = serde_json::to_string(&state.snapshot()).unwrap();
        let mut restored = ServerState::from_snapshot(serde_json::from_str(&json).unwrap());
        assert_eq!(restored.apply(ServerEvent::MessageCreate { message: msg }), Ok(false));
    }

    #[test]
    fn delete_that_arrives_first_still_wins() {
        let (mut state, channel, owner) = fixture();
        let msg = Message::new(channel, owner, "racy");
        assert_eq!(state.apply(ServerEvent::MessageDelete { channel, message: msg.id }), Ok(true));
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
        let msg = Message::new(channel, owner, "nice");
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
        let msg = Message::new(channel, owner, "nice");
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
}
