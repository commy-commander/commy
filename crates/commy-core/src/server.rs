use crate::ids::{ChannelId, MessageId, RoleId, ServerId, UserId};
use crate::permissions::Permissions;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Text,
    Voice,
    Announcement,
    Category,
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
}

pub const MAX_ACCESS_ROLES: usize = 16;

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
        }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub user: UserId,
    pub nickname: Option<String>,
    pub roles: Vec<RoleId>,
    pub joined_at: DateTime<Utc>,
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
    pub pins: Vec<Pin>,
}

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
            pins: vec![],
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

    pub fn permissions_for(&self, user: UserId) -> Permissions {
        if user == self.owner {
            return Permissions::all();
        }
        let Some(member) = self.member(user) else {
            return Permissions::NONE;
        };
        let mut perms = self
            .roles
            .iter()
            .filter(|r| r.is_everyone() || member.roles.contains(&r.id))
            .fold(Permissions::NONE, |acc, r| acc | permissions_granted_by(r));
        if perms.contains(Permissions::ADMINISTRATOR) {
            perms = Permissions::all();
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

    pub fn sorted_channels(&self) -> Vec<&Channel> {
        let mut channels: Vec<&Channel> = self.channels.iter().collect();
        channels.sort_by_key(|c| (c.parent, c.position));
        channels
    }
}

fn permissions_granted_by(role: &Role) -> Permissions {
    let mut granted = role.permissions;
    if role.is_everyone() {
        granted.remove(Permissions::CREATE_INVITES);
    }
    granted
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
            tag: None,
            banner: None,
        };
        let rid = role.id;
        server.roles.push(role);
        for (user, roles) in [(plain, vec![]), (announcer, vec![rid])] {
            server.members.push(Member { user, nickname: None, roles, joined_at: Utc::now() });
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
        });
        let perms = server.permissions_for(user);
        assert!(perms.allows(Permissions::SEND_MESSAGES));
        assert!(!perms.allows(Permissions::KICK_MEMBERS));
        assert!(!perms.allows(Permissions::CREATE_INVITES));
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
            tag: None,
            banner: None,
        };
        let rid = invite_role.id;
        server.roles.push(invite_role);
        for (user, roles) in [(plain, vec![]), (inviter, vec![rid])] {
            server.members.push(Member { user, nickname: None, roles, joined_at: Utc::now() });
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
            tag: None,
            banner: None,
        };
        server.members.push(Member {
            user,
            nickname: None,
            roles: vec![mod_role.id],
            joined_at: Utc::now(),
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
            tag: None,
            banner: None,
        };
        server.members.push(Member {
            user,
            nickname: None,
            roles: vec![admin_role.id],
            joined_at: Utc::now(),
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
    fn role_tags_are_short() {
        assert!(valid_role_tag("PRO"));
        assert!(valid_role_tag("\u{1F3AE}"));
        assert!(!valid_role_tag(""));
        assert!(!valid_role_tag("way too long tag"));
    }
}
