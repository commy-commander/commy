use crate::ids::{ChannelId, MessageId, RoleId, ServerId, TagId, UserId};
use crate::permissions::Permissions;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Text,
    Voice,
    Announcement,
    Category,
    Forum,
    Stage,
}

impl ChannelKind {
    pub fn holds_messages(self) -> bool {
        matches!(self, ChannelKind::Text | ChannelKind::Announcement | ChannelKind::Forum)
    }

    pub fn is_voice(self) -> bool {
        matches!(self, ChannelKind::Voice | ChannelKind::Stage)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForumTag {
    pub id: TagId,
    pub name: String,
    #[serde(default)]
    pub emoji: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForumPost {
    pub title: String,
    pub tags: Vec<TagId>,
    pub author: UserId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    pub id: ChannelId,
    pub name: String,
    pub kind: ChannelKind,
    pub topic: Option<String>,
    pub parent: Option<ChannelId>,
    pub position: u32,
    #[serde(default)]
    pub retention_days: Option<u32>,
    #[serde(default)]
    pub user_limit: u32,
    #[serde(default)]
    pub banner: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub view_roles: Vec<RoleId>,
    #[serde(default)]
    pub send_roles: Vec<RoleId>,
    #[serde(default)]
    pub slow_mode: u32,
    #[serde(default)]
    pub sensitive: bool,
    #[serde(default)]
    pub threads: BTreeMap<MessageId, Thread>,
    pub voice_status: Option<String>,
    #[serde(default)]
    pub tags: Vec<ForumTag>,
    #[serde(default)]
    pub posts: std::collections::BTreeMap<MessageId, ForumPost>,
    #[serde(default)]
    pub speakers: Vec<UserId>,
}

pub const MAX_ACCESS_ROLES: usize = 16;
pub const MAX_STAGE_SPEAKERS: usize = 50;
pub const MAX_SLOW_MODE: u32 = 21_600;
pub const MAX_THREAD_NAME_CHARS: usize = 100;
pub const MAX_THREADS_PER_CHANNEL: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    pub name: Option<String>,
    pub archived: bool,
    pub locked: bool,
    pub by: UserId,
}

impl Thread {
    pub fn is_plain(&self) -> bool {
        self.name.is_none() && !self.archived && !self.locked
    }

    pub fn same_settings(&self, other: &Thread) -> bool {
        self.name == other.name && self.archived == other.archived && self.locked == other.locked
    }
}

pub fn clean_thread_name(name: &str) -> Option<String> {
    let name = name.chars().filter(|c| !c.is_control()).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
    (!name.is_empty()).then_some(name)
}

pub fn valid_thread_name(name: &str) -> bool {
    clean_thread_name(name).as_deref() == Some(name) && name.chars().count() <= MAX_THREAD_NAME_CHARS
}

pub const MAX_VOICE_STATUS_CHARS: usize = 80;

pub fn clean_voice_status(text: &str) -> Option<String> {
    let cleaned: String = text
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let cleaned: String = cleaned.chars().take(MAX_VOICE_STATUS_CHARS).collect();
    let cleaned = cleaned.trim_end().to_string();
    (!cleaned.is_empty()).then_some(cleaned)
}

pub const MAX_FORUM_TAGS: usize = 20;
pub const MAX_FORUM_TAG_CHARS: usize = 20;
pub const MAX_POST_TAGS: usize = 5;
pub const MAX_POST_TITLE_CHARS: usize = 100;
pub const MAX_FORUM_POSTS: usize = 1000;

pub fn clean_forum_tags(tags: &[ForumTag]) -> Option<Vec<ForumTag>> {
    let mut out: Vec<ForumTag> = Vec::new();
    for tag in tags {
        let name = tag.name.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() {
            continue;
        }
        if name.chars().count() > MAX_FORUM_TAG_CHARS {
            return None;
        }
        if out.iter().any(|t| t.id == tag.id || t.name.eq_ignore_ascii_case(&name)) {
            continue;
        }
        let emoji = tag.emoji.as_deref().map(str::trim).filter(|e| valid_sound_emoji(e)).map(String::from);
        out.push(ForumTag { id: tag.id, name, emoji });
    }
    (out.len() <= MAX_FORUM_TAGS).then_some(out)
}

pub fn clean_post_title(title: &str) -> Option<String> {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let fits = (1..=MAX_POST_TITLE_CHARS).contains(&title.chars().count()) && !title.chars().any(char::is_control);
    fits.then_some(title)
}

pub fn clean_post_tags(picks: &[TagId], known: &[ForumTag]) -> Vec<TagId> {
    let mut out: Vec<TagId> = Vec::new();
    for pick in picks {
        if known.iter().any(|t| t.id == *pick) && !out.contains(pick) {
            out.push(*pick);
        }
    }
    out.truncate(MAX_POST_TAGS);
    out
}

impl Channel {
    pub fn new(name: impl Into<String>, kind: ChannelKind, position: u32) -> Self {
        Self {
            id: ChannelId::new(),
            name: name.into(),
            kind,
            topic: None,
            parent: None,
            position,
            retention_days: None,
            user_limit: 0,
            banner: None,
            color: None,
            view_roles: Vec::new(),
            send_roles: Vec::new(),
            slow_mode: 0,
            sensitive: false,
            threads: BTreeMap::new(),
            voice_status: None,
            tags: Vec::new(),
            posts: std::collections::BTreeMap::new(),
            speakers: Vec::new(),
        }
    }

    pub fn is_forum(&self) -> bool {
        self.kind == ChannelKind::Forum
    }

    pub fn is_stage(&self) -> bool {
        self.kind == ChannelKind::Stage
    }

    pub fn is_private(&self) -> bool {
        !self.view_roles.is_empty()
    }

    pub fn is_restricted(&self) -> bool {
        self.is_private() || !self.send_roles.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    pub id: RoleId,
    pub name: String,
    pub color: Option<String>,
    pub permissions: Permissions,
    pub position: u32,
    pub hoist: bool,
    #[serde(default)]
    pub self_assignable: bool,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub banner: Option<String>,
    #[serde(default)]
    pub mentionable: bool,
}

pub const EVERYONE_ROLE_POSITION: u32 = 0;

impl Role {
    pub fn is_everyone(&self) -> bool {
        self.position == EVERYONE_ROLE_POSITION
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pin {
    pub channel: ChannelId,
    pub message: MessageId,
    pub by: UserId,
    pub at: DateTime<Utc>,
}

pub const MAX_PINS_PER_CHANNEL: usize = 50;

pub const MAX_TIMEOUT_SECS: i64 = 28 * 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timeout {
    pub since: DateTime<Utc>,
    pub until: DateTime<Utc>,
    pub by: UserId,
}

impl Timeout {
    pub fn new(since: DateTime<Utc>, until: DateTime<Utc>, by: UserId) -> Option<Self> {
        let length = until.signed_duration_since(since).num_seconds();
        (length > 0 && length <= MAX_TIMEOUT_SECS).then_some(Self { since, until, by })
    }

    pub fn active_at(&self, now: DateTime<Utc>) -> bool {
        now < self.until
    }

    pub fn covers(&self, at: DateTime<Utc>) -> bool {
        self.since <= at && at < self.until
    }

    pub fn remaining_secs(&self, now: DateTime<Utc>) -> i64 {
        self.until.signed_duration_since(now).num_seconds().max(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub user: UserId,
    pub nickname: Option<String>,
    pub roles: Vec<RoleId>,
    pub joined_at: DateTime<Utc>,
    #[serde(default)]
    pub timeout: Option<Timeout>,
}

pub const MAX_NICKNAME_CHARS: usize = 32;

pub fn clean_nickname(raw: &str) -> Option<String> {
    let text = raw.trim();
    let length = text.chars().count();
    let fits = (1..=MAX_NICKNAME_CHARS).contains(&length) && !text.chars().any(char::is_control);
    fits.then(|| text.to_string())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoundboardSound {
    pub name: String,
    pub blob: String,
    pub volume: f32,
    #[serde(default)]
    pub emoji: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomEmoji {
    pub name: String,
    pub blob: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sticker {
    pub name: String,
    pub blob: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Emoticon {
    pub name: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Server {
    pub id: ServerId,
    pub name: String,
    pub icon: Option<String>,
    pub owner: UserId,
    pub channels: Vec<Channel>,
    pub roles: Vec<Role>,
    pub members: Vec<Member>,
    pub emojis: Vec<CustomEmoji>,
    pub soundboard: Vec<SoundboardSound>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub deleted_channels: Vec<ChannelId>,
    #[serde(default)]
    pub afk_channel_id: Option<ChannelId>,
    #[serde(default)]
    pub afk_disabled: bool,
    #[serde(default)]
    pub banner: Option<String>,
    #[serde(default)]
    pub emojis_shareable: bool,
    #[serde(default)]
    pub soundboard_open: bool,
    #[serde(default)]
    pub sounds_shareable: bool,
    #[serde(default)]
    pub pronouns: std::collections::BTreeMap<UserId, String>,
    #[serde(default)]
    pub game_tags: Vec<String>,
    #[serde(default)]
    pub game_tag_meta: Vec<GameTagMeta>,
    #[serde(default)]
    pub rules: Option<String>,
    #[serde(default)]
    pub rules_accepted: Vec<UserId>,
    #[serde(default)]
    pub events: Vec<ScheduledEvent>,
    #[serde(default)]
    pub deleted_events: Vec<String>,
    #[serde(default)]
    pub server_muted: Vec<UserId>,
    #[serde(default)]
    pub server_deafened: Vec<UserId>,
    #[serde(default)]
    pub banned: Vec<UserId>,
    #[serde(default)]
    pub afk_timeout_minutes: Option<u32>,
    #[serde(default)]
    pub public: bool,
    #[serde(default)]
    pub group: bool,
    #[serde(default)]
    pub pins: Vec<Pin>,
    #[serde(default)]
    pub stickers: Vec<Sticker>,
    #[serde(default)]
    pub emoticons: Vec<Emoticon>,
    #[serde(default)]
    pub stickers_shareable: bool,
    #[serde(default)]
    pub emoticons_shareable: bool,
    #[serde(default)]
    pub onboarding: Option<crate::onboarding::Onboarding>,
    #[serde(default)]
    pub defaults_version: u32,
}

pub const MEMBER_DEFAULTS_VERSION: u32 = 1;

const LATE_MEMBER_DEFAULTS: Permissions = Permissions(
    Permissions::ADD_EMOJIS.0
        | Permissions::ADD_STICKERS.0
        | Permissions::ADD_EMOTICONS.0
        | Permissions::CHANGE_NICKNAME.0
        | Permissions::SEND_VOICE_MESSAGES.0
        | Permissions::SEND_TTS_MESSAGES.0,
);

impl Server {
    pub fn new(name: impl Into<String>, owner: UserId) -> Self {
        let everyone = Role {
            id: RoleId::new(),
            name: "everyone".to_string(),
            color: None,
            permissions: Permissions::default_member(),
            position: EVERYONE_ROLE_POSITION,
            hoist: false,
            self_assignable: false,
            mentionable: false,
            tag: None,
            banner: None,
        };
        let afk = Channel::new("AFK", ChannelKind::Voice, 2);
        let afk_id = afk.id;
        Self {
            id: ServerId::new(),
            name: name.into(),
            icon: None,
            owner,
            channels: vec![
                Channel::new("general", ChannelKind::Text, 0),
                Channel::new("General", ChannelKind::Voice, 1),
                afk,
            ],
            roles: vec![everyone],
            members: vec![Member {
                user: owner,
                nickname: None,
                roles: vec![],
                joined_at: Utc::now(),
                timeout: None,
            }],
            emojis: vec![],
            soundboard: vec![],
            created_at: Utc::now(),
            deleted_channels: vec![],
            afk_channel_id: Some(afk_id),
            afk_disabled: false,
            banner: None,
            emojis_shareable: false,
            soundboard_open: false,
            sounds_shareable: false,
            pronouns: std::collections::BTreeMap::new(),
            game_tags: vec![],
            game_tag_meta: vec![],
            rules: None,
            rules_accepted: vec![],
            events: vec![],
            deleted_events: vec![],
            server_muted: vec![],
            server_deafened: vec![],
            banned: vec![],
            afk_timeout_minutes: None,
            public: false,
            group: false,
            pins: vec![],
            stickers: vec![],
            emoticons: vec![],
            stickers_shareable: false,
            emoticons_shareable: false,
            onboarding: None,
            defaults_version: MEMBER_DEFAULTS_VERSION,
        }
    }

    pub fn channel(&self, id: ChannelId) -> Option<&Channel> {
        self.channels.iter().find(|c| c.id == id)
    }

    pub fn channel_mut(&mut self, id: ChannelId) -> Option<&mut Channel> {
        self.channels.iter_mut().find(|c| c.id == id)
    }

    pub fn member(&self, user: UserId) -> Option<&Member> {
        self.members.iter().find(|m| m.user == user)
    }

    pub fn member_mut(&mut self, user: UserId) -> Option<&mut Member> {
        self.members.iter_mut().find(|m| m.user == user)
    }

    pub fn is_member(&self, user: UserId) -> bool {
        self.member(user).is_some()
    }

    pub fn is_current_member(&self, user: UserId) -> bool {
        self.is_member(user) && !self.banned.contains(&user)
    }

    pub fn timeout_of(&self, user: UserId, now: DateTime<Utc>) -> Option<Timeout> {
        self.member(user)?.timeout.filter(|t| t.active_at(now))
    }

    pub fn is_timed_out(&self, user: UserId, now: DateTime<Utc>) -> bool {
        self.timeout_of(user, now).is_some()
    }

    pub fn timed_out_at(&self, user: UserId, at: DateTime<Utc>) -> bool {
        self.member(user).and_then(|m| m.timeout).is_some_and(|t| t.covers(at))
    }

    pub fn timed_out_members(&self, now: DateTime<Utc>) -> Vec<(UserId, Timeout)> {
        self.members
            .iter()
            .filter_map(|m| m.timeout.filter(|t| t.active_at(now)).map(|t| (m.user, t)))
            .collect()
    }

    pub fn permissions_for(&self, user: UserId) -> Permissions {
        if user == self.owner {
            return Permissions::all();
        }
        let Some(member) = self.member(user) else {
            return Permissions::NONE;
        };
        let applicable: Vec<&Role> = self.roles.iter().filter(|r| r.is_everyone() || member.roles.contains(&r.id)).collect();
        let mut perms = applicable.iter().fold(Permissions::NONE, |acc, r| acc | self.granted_by(r)) & !Permissions::RESTRICTIONS;
        if perms.contains(Permissions::ADMINISTRATOR) {
            return Permissions::all();
        }
        if !applicable.is_empty() {
            perms |= applicable.iter().fold(Permissions::RESTRICTIONS, |acc, r| acc & r.permissions);
        }
        perms
    }

    pub fn can_pin(&self, user: UserId) -> bool {
        self.permissions_for(user).can_pin()
    }

    pub fn can_post_in(&self, channel: ChannelId, user: UserId) -> bool {
        let perms = self.permissions_for(user);
        let Some(found) = self.channel(channel) else {
            return perms.allows(Permissions::SEND_MESSAGES);
        };
        let allowed = match found.kind {
            ChannelKind::Announcement => perms.can_announce(),
            _ => perms.allows(Permissions::SEND_MESSAGES),
        };
        allowed && self.admits(user, &found.view_roles) && self.admits(user, &found.send_roles)
    }

    pub fn can_view(&self, channel: ChannelId, user: UserId) -> bool {
        self.channel(channel).is_some_and(|c| self.admits(user, &c.view_roles))
    }

    pub fn is_stage_speaker(&self, channel: ChannelId, user: UserId) -> bool {
        self.channel(channel).is_some_and(|c| {
            c.is_stage() && (c.speakers.contains(&user) || self.permissions_for(user).can_manage_stage())
        })
    }

    pub fn in_audience(&self, channel: ChannelId, user: UserId) -> bool {
        self.channel(channel).is_some_and(|c| c.is_stage()) && !self.is_stage_speaker(channel, user)
    }

    fn admits(&self, user: UserId, roles: &[RoleId]) -> bool {
        if roles.is_empty() {
            return true;
        }
        if self.permissions_for(user).contains(Permissions::ADMINISTRATOR) {
            return true;
        }
        self.member(user).is_some_and(|m| m.roles.iter().any(|r| roles.contains(r)))
    }

    pub fn readers_of(&self, channel: ChannelId) -> Vec<UserId> {
        self.members
            .iter()
            .map(|m| m.user)
            .filter(|u| !self.banned.contains(u) && self.can_view(channel, *u))
            .collect()
    }

    pub fn is_pinned(&self, message: MessageId) -> bool {
        self.pins.iter().any(|p| p.message == message)
    }

    pub fn pins_in(&self, channel: ChannelId) -> Vec<&Pin> {
        let mut pins: Vec<&Pin> = self.pins.iter().filter(|p| p.channel == channel).collect();
        pins.sort_by(|a, b| b.at.cmp(&a.at));
        pins
    }

    pub fn afk_channel(&self) -> Option<ChannelId> {
        if self.afk_disabled {
            return None;
        }
        let is_voice = |id: ChannelId| {
            self.channel(id).is_some_and(|c| c.kind == ChannelKind::Voice)
        };
        if let Some(id) = self.afk_channel_id.filter(|id| is_voice(*id)) {
            return Some(id);
        }
        self.channels
            .iter()
            .find(|c| c.kind == ChannelKind::Voice && c.name.eq_ignore_ascii_case("afk"))
            .map(|c| c.id)
    }

    pub fn role_color_for(&self, user: UserId) -> Option<String> {
        self.top_role_with(user, |r| r.color.is_some()).and_then(|r| r.color.clone())
    }

    pub fn top_role_with(&self, user: UserId, wanted: impl Fn(&Role) -> bool) -> Option<&Role> {
        let member = self.member(user)?;
        self.roles
            .iter()
            .filter(|r| member.roles.contains(&r.id) && wanted(r))
            .max_by_key(|r| r.position)
    }

    pub fn group(owner: UserId) -> Self {
        let mut server = Self::new("", owner);
        server.group = true;
        server.channels = vec![Channel::new("chat", ChannelKind::Text, 0), Channel::new("call", ChannelKind::Voice, 1)];
        server.afk_channel_id = None;
        server
    }

    pub fn sorted_channels(&self) -> Vec<&Channel> {
        let mut channels: Vec<&Channel> = self.channels.iter().collect();
        channels.sort_by_key(|c| (c.parent, c.position));
        channels
    }
}

impl Server {
    pub fn granted_by(&self, role: &Role) -> Permissions {
        let mut granted = role.permissions;
        if role.is_everyone() {
            granted.remove(Permissions::CREATE_INVITES);
            granted |= self.late_member_defaults();
        }
        granted
    }

    pub fn late_member_defaults(&self) -> Permissions {
        if self.defaults_version < MEMBER_DEFAULTS_VERSION {
            LATE_MEMBER_DEFAULTS
        } else {
            Permissions::NONE
        }
    }

    pub fn everyone_permissions(&self) -> Permissions {
        self.roles
            .iter()
            .find(|r| r.is_everyone())
            .map(|r| r.permissions | self.late_member_defaults())
            .unwrap_or(Permissions::NONE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_server_has_defaults() {
        let owner = UserId::new();
        let server = Server::new("Rust Hangout", owner);
        assert_eq!(server.channels.len(), 3);
        assert_eq!(server.channels[0].kind, ChannelKind::Text);
        assert_eq!(server.channels[1].kind, ChannelKind::Voice);
        assert!(server.is_member(owner));
        assert_eq!(server.afk_channel(), Some(server.channels[2].id));
    }

    #[test]
    fn role_color_uses_highest_colored_role() {
        let owner = UserId::new();
        let mut server = Server::new("s", owner);
        let low = Role {
            id: RoleId::new(),
            name: "low".into(),
            color: Some("aaaaaa".into()),
            permissions: Permissions::NONE,
            position: 1,
            hoist: false,
            self_assignable: false,
            mentionable: false,
            tag: None,
            banner: None,
        };
        let high_colorless = Role {
            id: RoleId::new(),
            name: "high".into(),
            color: None,
            permissions: Permissions::NONE,
            position: 2,
            hoist: false,
            self_assignable: false,
            mentionable: false,
            tag: None,
            banner: None,
        };
        server.members[0].roles = vec![low.id, high_colorless.id];
        server.roles.push(low);
        server.roles.push(high_colorless);
        assert_eq!(server.role_color_for(owner).as_deref(), Some("aaaaaa"));
        assert_eq!(server.role_color_for(UserId::new()), None);
    }

    #[test]
    fn announcement_channels_take_posts_from_announcers_only() {
        let owner = UserId::new();
        let mut server = Server::new("s", owner);
        let news = Channel::new("news", ChannelKind::Announcement, 3);
        let news_id = news.id;
        let general = server.channels[0].id;
        server.channels.push(news);
        let plain = UserId::new();
        let announcer = UserId::new();
        let role = Role {
            id: RoleId::new(),
            name: "news team".to_string(),
            color: None,
            permissions: Permissions::POST_ANNOUNCEMENTS,
            position: 1,
            hoist: false,
            self_assignable: false,
            mentionable: false,
            tag: None,
            banner: None,
        };
        let rid = role.id;
        server.roles.push(role);
        for (user, roles) in [(plain, vec![]), (announcer, vec![rid])] {
            server.members.push(Member { user, nickname: None, roles, joined_at: Utc::now(), timeout: None });
        }
        assert!(server.can_post_in(general, plain));
        assert!(!server.can_post_in(news_id, plain));
        assert!(server.can_post_in(news_id, announcer));
        assert!(server.can_post_in(news_id, owner));
        assert!(!server.can_post_in(news_id, UserId::new()));
        assert!(!server.can_pin(plain));
        assert!(server.can_pin(owner));
    }

    #[test]
    fn pins_list_newest_first_per_channel() {
        let owner = UserId::new();
        let mut server = Server::new("s", owner);
        let a = server.channels[0].id;
        let b = ChannelId::new();
        let t0 = Utc::now();
        let (m1, m2, m3) = (MessageId::new(), MessageId::new(), MessageId::new());
        server.pins.push(Pin { channel: a, message: m1, by: owner, at: t0 });
        server.pins.push(Pin { channel: b, message: m2, by: owner, at: t0 + chrono::Duration::seconds(1) });
        server.pins.push(Pin { channel: a, message: m3, by: owner, at: t0 + chrono::Duration::seconds(2) });
        let ids: Vec<MessageId> = server.pins_in(a).iter().map(|p| p.message).collect();
        assert_eq!(ids, vec![m3, m1]);
        assert!(server.is_pinned(m2));
        assert!(!server.is_pinned(MessageId::new()));
    }

    #[test]
    fn owner_has_all_permissions() {
        let owner = UserId::new();
        let server = Server::new("s", owner);
        assert!(server.permissions_for(owner).allows(Permissions::BAN_MEMBERS));
    }

    #[test]
    fn non_member_has_no_permissions() {
        let server = Server::new("s", UserId::new());
        assert_eq!(server.permissions_for(UserId::new()), Permissions::NONE);
    }

    fn role_with(name: &str, permissions: Permissions, position: u32) -> Role {
        Role {
            id: RoleId::new(),
            name: name.into(),
            color: None,
            permissions,
            position,
            hoist: false,
            self_assignable: false,
            mentionable: false,
            tag: None,
            banner: None,
        }
    }

    #[test]
    fn restrictions_hold_only_when_every_applicable_role_carries_them() {
        let owner = UserId::new();
        let mut server = Server::new("s", owner);
        let member = UserId::new();
        server.members.push(Member { user: member, nickname: None, roles: vec![], joined_at: Utc::now(), timeout: None });
        assert!(server.permissions_for(member).can_send_polls(), "no role mentions the bit, so it is allowed");
        assert!(!server.permissions_for(member).contains(Permissions::RESTRICTIONS));
        server.roles[0].permissions.insert(Permissions::NO_SEND_POLLS | Permissions::NO_EMBED_LINKS);
        let restricted = server.permissions_for(member);
        assert!(!restricted.can_send_polls() && !restricted.can_embed_links(), "everyone restricted and no other role: restricted");
        assert!(restricted.can_read_history(), "only the bits every role carries hold");
        let trusted = role_with("trusted", Permissions::NONE, 1);
        let trusted_id = trusted.id;
        server.roles.push(trusted);
        server.member_mut(member).unwrap().roles.push(trusted_id);
        let lifted = server.permissions_for(member);
        assert!(lifted.can_send_polls() && lifted.can_embed_links(), "a role without the restriction lets its members through");
        assert!(lifted.allows(Permissions::SEND_MESSAGES), "grants stay a union");
        let other = UserId::new();
        server.members.push(Member { user: other, nickname: None, roles: vec![], joined_at: Utc::now(), timeout: None });
        assert!(!server.permissions_for(other).can_send_polls(), "members without the role stay restricted");
        server.roles[0].permissions.remove(Permissions::RESTRICTIONS);
        let lone = role_with("quiet", Permissions::NO_SEND_POLLS, 2);
        let lone_id = lone.id;
        server.roles.push(lone);
        server.member_mut(other).unwrap().roles.push(lone_id);
        assert!(server.permissions_for(other).can_send_polls(), "a lone restricted role on top of a permissive everyone does nothing");
        server.roles[0].permissions.insert(Permissions::RESTRICTIONS);
        let admins = role_with("admins", Permissions::ADMINISTRATOR | Permissions::RESTRICTIONS, 3);
        let admins_id = admins.id;
        server.roles.push(admins);
        server.member_mut(other).unwrap().roles.push(admins_id);
        assert_eq!(server.permissions_for(other), Permissions::all(), "administrator clears everything");
        assert!(server.permissions_for(other).can_read_history());
        assert_eq!(server.permissions_for(owner), Permissions::all());
    }

    #[test]
    fn member_gets_everyone_role_permissions() {
        let owner = UserId::new();
        let mut server = Server::new("s", owner);
        let user = UserId::new();
        server.members.push(Member {
            user,
            nickname: None,
            roles: vec![],
            joined_at: Utc::now(),
            timeout: None,
        });
        let perms = server.permissions_for(user);
        assert!(perms.allows(Permissions::SEND_MESSAGES));
        assert!(!perms.allows(Permissions::KICK_MEMBERS));
        assert!(!perms.allows(Permissions::CREATE_INVITES));
    }

    #[test]
    fn servers_from_before_the_newer_member_defaults_still_grant_them() {
        let owner = UserId::new();
        let mut server = Server::new("s", owner);
        let plain = UserId::new();
        server.members.push(Member { user: plain, nickname: None, roles: vec![], joined_at: Utc::now(), timeout: None });
        server.roles[0].permissions.remove(Permissions::CHANGE_NICKNAME | Permissions::SEND_VOICE_MESSAGES);
        assert!(!server.permissions_for(plain).can_change_own_nickname(), "a current server honours the stored bits");
        server.defaults_version = 0;
        let perms = server.permissions_for(plain);
        assert!(perms.can_change_own_nickname() && perms.allows(Permissions::SEND_VOICE_MESSAGES) && perms.can_send_tts());
        assert!(!perms.allows(Permissions::CREATE_INVITES));
        assert!(server.everyone_permissions().allows(Permissions::ADD_EMOJIS));
        assert_eq!(Server::new("t", owner).defaults_version, MEMBER_DEFAULTS_VERSION);
    }

    #[test]
    fn everyone_role_never_grants_invites() {
        let owner = UserId::new();
        let mut server = Server::new("s", owner);
        server.roles[0].permissions.insert(Permissions::CREATE_INVITES);
        let plain = UserId::new();
        let inviter = UserId::new();
        let invite_role = Role {
            id: RoleId::new(),
            name: "greeters".to_string(),
            color: None,
            permissions: Permissions::CREATE_INVITES,
            position: 1,
            hoist: false,
            self_assignable: false,
            mentionable: false,
            tag: None,
            banner: None,
        };
        let rid = invite_role.id;
        server.roles.push(invite_role);
        for (user, roles) in [(plain, vec![]), (inviter, vec![rid])] {
            server.members.push(Member { user, nickname: None, roles, joined_at: Utc::now(), timeout: None });
        }
        assert!(!server.permissions_for(plain).allows(Permissions::CREATE_INVITES));
        assert!(server.permissions_for(inviter).allows(Permissions::CREATE_INVITES));
        assert!(server.permissions_for(owner).allows(Permissions::CREATE_INVITES));
    }

    #[test]
    fn role_grants_stack() {
        let owner = UserId::new();
        let mut server = Server::new("s", owner);
        let user = UserId::new();
        let mod_role = Role {
            id: RoleId::new(),
            name: "mods".to_string(),
            color: Some("5865f2".to_string()),
            permissions: Permissions::KICK_MEMBERS | Permissions::MANAGE_MESSAGES,
            position: 1,
            hoist: true,
            self_assignable: false,
            mentionable: false,
            tag: None,
            banner: None,
        };
        server.members.push(Member {
            user,
            nickname: None,
            roles: vec![mod_role.id],
            joined_at: Utc::now(),
            timeout: None,
        });
        server.roles.push(mod_role);
        let perms = server.permissions_for(user);
        assert!(perms.allows(Permissions::KICK_MEMBERS));
        assert!(perms.allows(Permissions::SEND_MESSAGES));
        assert!(!perms.allows(Permissions::BAN_MEMBERS));
    }

    #[test]
    fn administrator_role_expands_to_all() {
        let owner = UserId::new();
        let mut server = Server::new("s", owner);
        let user = UserId::new();
        let admin_role = Role {
            id: RoleId::new(),
            name: "admin".to_string(),
            color: None,
            permissions: Permissions::ADMINISTRATOR,
            position: 2,
            hoist: true,
            self_assignable: false,
            mentionable: false,
            tag: None,
            banner: None,
        };
        server.members.push(Member {
            user,
            nickname: None,
            roles: vec![admin_role.id],
            joined_at: Utc::now(),
            timeout: None,
        });
        server.roles.push(admin_role);
        assert!(server.permissions_for(user).allows(Permissions::BAN_MEMBERS));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    pub blob: String,
    pub provider: String,
}

impl ImageRef {
    pub fn parse(raw: &str) -> Option<Self> {
        let (blob, provider) = raw.split_once('@')?;
        let ok = |s: &str| !s.is_empty() && s.len() <= 128 && s.chars().all(|c| c.is_ascii_alphanumeric());
        (ok(blob) && ok(provider)).then(|| Self { blob: blob.to_string(), provider: provider.to_string() })
    }

    pub fn encode(&self) -> String {
        format!("{}@{}", self.blob, self.provider)
    }
}

#[cfg(test)]
mod image_ref_tests {
    use super::*;

    #[test]
    fn roundtrips() {
        let r = ImageRef { blob: "abc123".into(), provider: "f00d".into() };
        assert_eq!(ImageRef::parse(&r.encode()), Some(r));
    }

    #[test]
    fn rejects_malformed() {
        for bad in ["", "abc", "@node", "blob@", "a/b@c", "a@b@c", "x@y z"] {
            assert_eq!(ImageRef::parse(bad), None, "{bad}");
        }
    }
}

pub const MAX_EMOJIS: usize = 50;
pub const MAX_SOUNDS: usize = 24;
pub const MAX_STICKERS: usize = 30;
pub const MAX_EMOTICONS: usize = 50;
pub const MAX_EMOTICON_CHARS: usize = 64;
const MAX_EMOTICON_BYTES: usize = 256;

pub fn valid_expression_name(name: &str) -> bool {
    (2..=32).contains(&name.len())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

pub fn parse_emoji_token(token: &str) -> Option<(String, ImageRef)> {
    let inner = token.strip_prefix("<:")?.strip_suffix('>')?;
    let (name, image) = inner.split_once(':')?;
    let image = ImageRef::parse(image)?;
    valid_expression_name(name).then(|| (name.to_string(), image))
}

pub fn emoji_token(name: &str, image: &str) -> String {
    format!("<:{name}:{image}>")
}

pub fn parse_sticker_token(token: &str) -> Option<(String, ImageRef)> {
    let inner = token.strip_prefix("<sticker:")?.strip_suffix('>')?;
    let (name, image) = inner.split_once(':')?;
    let image = ImageRef::parse(image)?;
    valid_expression_name(name).then(|| (name.to_string(), image))
}

pub fn sticker_token(name: &str, image: &str) -> String {
    format!("<sticker:{name}:{image}>")
}

pub fn image_tokens(content: &str) -> Vec<(usize, usize, ImageRef)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(start) = content[from..].find('<').map(|i| from + i) {
        let Some(end) = content[start..].find('>').map(|i| start + i + 1) else { break };
        let token = &content[start..end];
        if let Some((_, image)) = parse_emoji_token(token).or_else(|| parse_sticker_token(token)) {
            found.push((start, end, image));
            from = end;
        } else {
            from = start + 1;
        }
    }
    found
}

pub fn clean_emoticon(text: &str) -> Option<String> {
    let text = text.trim();
    let chars = text.chars().count();
    let fits = (1..=MAX_EMOTICON_CHARS).contains(&chars) && text.len() <= MAX_EMOTICON_BYTES;
    (fits && !text.chars().any(char::is_control)).then(|| text.to_string())
}

pub fn valid_reaction(emoji: &str) -> bool {
    if parse_emoji_token(emoji).is_some() {
        return true;
    }
    if let Some(name) = emoji.strip_prefix(':').and_then(|s| s.strip_suffix(':')) {
        return valid_expression_name(name);
    }
    crate::user::is_emoji_like(emoji)
}

#[cfg(test)]
mod expression_tests {
    use super::*;

    #[test]
    fn emoji_tokens_roundtrip_and_validate() {
        let token = emoji_token("party", "ab12@cd34");
        let (name, image) = parse_emoji_token(&token).unwrap();
        assert_eq!(name, "party");
        assert_eq!(image.encode(), "ab12@cd34");
        assert!(valid_reaction(&token));
        for bad in ["<:party:ab12>", "<:Party:a@b>", ":party:a@b>", "<:party:a@b", "<::a@b>"] {
            assert!(parse_emoji_token(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn sticker_tokens_roundtrip_and_stay_apart_from_emojis() {
        let token = sticker_token("wave", "ab12@cd34");
        let (name, image) = parse_sticker_token(&token).unwrap();
        assert_eq!(name, "wave");
        assert_eq!(image.encode(), "ab12@cd34");
        assert!(parse_emoji_token(&token).is_none());
        assert!(parse_sticker_token(&emoji_token("wave", "ab12@cd34")).is_none());
        for bad in ["<sticker:wave:ab12>", "<sticker:Wave:a@b>", "<sticker::a@b>", "sticker:wave:a@b>"] {
            assert!(parse_sticker_token(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn image_tokens_finds_emojis_and_stickers_in_text() {
        let text = format!("a <b {} c {} <:x:", emoji_token("gg", "aa@bb"), sticker_token("hi", "cc@dd"));
        let found: Vec<String> = image_tokens(&text).into_iter().map(|(s, e, i)| format!("{}={}", &text[s..e], i.blob)).collect();
        assert_eq!(found, vec!["<:gg:aa@bb>=aa", "<sticker:hi:cc@dd>=cc"]);
    }

    #[test]
    fn emoticons_are_short_single_line_text() {
        assert_eq!(clean_emoticon("  ¯\\_(ツ)_/¯ ").as_deref(), Some("¯\\_(ツ)_/¯"));
        assert_eq!(clean_emoticon("(╯°□°)╯︵ ┻━┻").as_deref(), Some("(╯°□°)╯︵ ┻━┻"));
        assert!(clean_emoticon("   ").is_none());
        assert!(clean_emoticon("two\nlines").is_none());
        assert!(clean_emoticon(&"x".repeat(MAX_EMOTICON_CHARS + 1)).is_none());
    }

    #[test]
    fn names_are_short_lowercase_words() {
        assert!(valid_expression_name("party_parrot"));
        assert!(valid_expression_name("gg"));
        assert!(!valid_expression_name("a"));
        assert!(!valid_expression_name("Party"));
        assert!(!valid_expression_name("has space"));
        assert!(!valid_expression_name(&"x".repeat(33)));
    }

    #[test]
    fn reactions_accept_unicode_and_custom() {
        assert!(valid_reaction("🔥"));
        assert!(valid_reaction(":party_parrot:"));
        assert!(!valid_reaction(":Bad Name:"));
        assert!(!valid_reaction("plain text"));
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameTagMeta {
    pub name: String,
    pub igdb_id: u64,
    pub cover: Option<String>,
}

pub fn clean_game_tag_meta(tags: &[String], meta: &[GameTagMeta]) -> Vec<GameTagMeta> {
    let safe = |id: &str| !id.is_empty() && id.len() <= 32 && id.chars().all(|c| c.is_ascii_alphanumeric());
    let mut out: Vec<GameTagMeta> = Vec::new();
    for m in meta {
        if tags.iter().any(|t| t == &m.name) && !out.iter().any(|o| o.name == m.name) {
            out.push(GameTagMeta { cover: m.cover.clone().filter(|c| safe(c)), ..m.clone() });
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rsvp {
    Going,
    Interested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledEvent {
    pub id: String,
    pub title: String,
    pub description: String,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    pub channel: Option<ChannelId>,
    pub creator: UserId,
    #[serde(default)]
    pub rsvps: Vec<(UserId, Rsvp)>,
}

pub const SERVER_AFK_MINUTES: &[u32] = &[1, 5, 15, 30, 60];

pub const MAX_GAME_TAGS: usize = 5;
pub const MAX_GAME_TAG_CHARS: usize = 32;
pub const MAX_RULES_BYTES: usize = 3000;
pub const MAX_EVENTS: usize = 50;
pub const MAX_EVENT_TITLE_CHARS: usize = 100;
pub const MAX_EVENT_DESCRIPTION_BYTES: usize = 1500;
pub const MAX_ROLE_TAG_CHARS: usize = 12;
pub const MAX_SOUND_EMOJI_CHARS: usize = 12;
pub const MAX_USER_LIMIT: u32 = 99;

pub fn clean_game_tags(tags: &[String]) -> Option<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        let tag = tag.split_whitespace().collect::<Vec<_>>().join(" ");
        if tag.is_empty() {
            continue;
        }
        if tag.chars().count() > MAX_GAME_TAG_CHARS {
            return None;
        }
        if !out.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
            out.push(tag);
        }
    }
    (out.len() <= MAX_GAME_TAGS).then_some(out)
}

pub fn valid_rules(rules: &str) -> bool {
    !rules.trim().is_empty() && rules.len() <= MAX_RULES_BYTES
}

pub fn valid_event(event: &ScheduledEvent) -> bool {
    let title = event.title.trim();
    !title.is_empty()
        && title.chars().count() <= MAX_EVENT_TITLE_CHARS
        && event.description.len() <= MAX_EVENT_DESCRIPTION_BYTES
        && event.id.len() <= 64
        && !event.id.is_empty()
        && event.end.map_or(true, |end| end > event.start)
}

pub fn valid_sound_emoji(emoji: &str) -> bool {
    if parse_emoji_token(emoji).is_some() {
        return true;
    }
    let n = emoji.chars().count();
    (1..=MAX_SOUND_EMOJI_CHARS).contains(&n)
        && !emoji.chars().any(|c| c.is_ascii_alphanumeric() || c.is_control() || c.is_whitespace())
}

pub fn valid_role_tag(tag: &str) -> bool {
    if parse_emoji_token(tag).is_some() {
        return true;
    }
    let n = tag.chars().count();
    n >= 1 && n <= MAX_ROLE_TAG_CHARS && !tag.chars().any(char::is_control)
}

pub fn valid_hex_color(color: &str) -> bool {
    color.len() == 6 && color.chars().all(|ch| ch.is_ascii_hexdigit())
}

#[cfg(test)]
mod community_tests {
    use super::*;

    #[test]
    fn game_tags_are_cleaned_and_capped() {
        let tags: Vec<String> = ["  Valorant ", "valorant", "", "Rocket  League"].map(String::from).to_vec();
        assert_eq!(clean_game_tags(&tags), Some(vec!["Valorant".to_string(), "Rocket League".to_string()]));
        let many: Vec<String> = (0..6).map(|i| format!("game {i}")).collect();
        assert_eq!(clean_game_tags(&many), None);
        assert_eq!(clean_game_tags(&["x".repeat(33)]), None);
    }

    #[test]
    fn rules_must_fit() {
        assert!(valid_rules("Be kind."));
        assert!(!valid_rules("   "));
        assert!(!valid_rules(&"a".repeat(MAX_RULES_BYTES + 1)));
    }

    fn event() -> ScheduledEvent {
        ScheduledEvent {
            id: "e1".into(),
            title: "Game night".into(),
            description: String::new(),
            start: Utc::now(),
            end: None,
            channel: None,
            creator: UserId::new(),
            rsvps: vec![],
        }
    }

    #[test]
    fn events_need_a_title_and_a_sane_end() {
        assert!(valid_event(&event()));
        assert!(!valid_event(&ScheduledEvent { title: " ".into(), ..event() }));
        let e = event();
        assert!(!valid_event(&ScheduledEvent { end: Some(e.start), ..e.clone() }));
        assert!(valid_event(&ScheduledEvent { end: Some(e.start + chrono::Duration::hours(2)), ..e }));
    }

    #[test]
    fn voice_statuses_are_one_tidy_line_of_at_most_eighty_characters() {
        assert_eq!(clean_voice_status("Ranked grind"), Some("Ranked grind".into()));
        assert_eq!(clean_voice_status("  Ranked \n\t grind  "), Some("Ranked grind".into()));
        assert_eq!(clean_voice_status("Ran\u{0}ked"), Some("Ranked".into()));
        assert_eq!(clean_voice_status(""), None);
        assert_eq!(clean_voice_status(" \n "), None);
        let long = "a".repeat(100);
        assert_eq!(clean_voice_status(&long).map(|s| s.chars().count()), Some(MAX_VOICE_STATUS_CHARS));
        let cut = format!("{} tail", "b".repeat(79));
        assert_eq!(clean_voice_status(&cut), Some("b".repeat(79)), "no dangling space after the cut");
    }

    #[test]
    fn role_tags_are_short() {
        assert!(valid_role_tag("PRO"));
        assert!(valid_role_tag("\u{1F3AE}"));
        assert!(!valid_role_tag(""));
        assert!(!valid_role_tag("way too long tag"));
    }
}
