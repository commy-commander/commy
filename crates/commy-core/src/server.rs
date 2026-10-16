use crate::ids::{ChannelId, RoleId, ServerId, UserId};
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
    /// Category this channel is nested under, if any.
    pub parent: Option<ChannelId>,
    pub position: u32,
    /// Days messages are kept before every peer deletes them; None keeps
    /// them forever.
    #[serde(default)]
    pub retention_days: Option<u32>,
    /// Voice channels: most people allowed in at once; 0 is unlimited.
    #[serde(default)]
    pub user_limit: u32,
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    pub id: RoleId,
    pub name: String,
    /// RGB hex without leading hash, e.g. "5865f2".
    pub color: Option<String>,
    pub permissions: Permissions,
    pub position: u32,
    /// Shown as its own group in the member list.
    pub hoist: bool,
    /// Members can pick this role for themselves.
    #[serde(default)]
    pub self_assignable: bool,
    /// Short text or emoji shown next to holders' names.
    #[serde(default)]
    pub tag: Option<String>,
    /// Banner image ref shown on holders' profile cards.
    #[serde(default)]
    pub banner: Option<String>,
}

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
    /// "blob@provider" of the audio clip.
    pub blob: String,
    pub volume: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomEmoji {
    pub name: String,
    /// "blob@provider" of the emoji image.
    pub blob: String,
}

/// A Commy server (a community). Lives entirely on its members'
/// machines; there is no hosted backend for server content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Server {
    pub id: ServerId,
    pub name: String,
    /// Server avatar as an image ref, see [`ImageRef`].
    pub icon: Option<String>,
    pub owner: UserId,
    pub channels: Vec<Channel>,
    pub roles: Vec<Role>,
    pub members: Vec<Member>,
    pub emojis: Vec<CustomEmoji>,
    pub soundboard: Vec<SoundboardSound>,
    pub created_at: DateTime<Utc>,
    /// Ids of deleted channels, so a late re-announce of a channel
    /// cannot bring it back.
    #[serde(default)]
    pub deleted_channels: Vec<ChannelId>,
    /// Explicit AFK voice channel; falls back to one named "AFK".
    #[serde(default)]
    pub afk_channel_id: Option<ChannelId>,
    /// Set when the server chose to have no AFK channel at all.
    #[serde(default)]
    pub afk_disabled: bool,
    /// Banner shown above the channel list, as an image ref.
    #[serde(default)]
    pub banner: Option<String>,
    /// Members may use this server's emojis in other servers and DMs.
    #[serde(default)]
    pub emojis_shareable: bool,
    /// Everyone (not just Manage Expressions) may upload sounds.
    #[serde(default)]
    pub soundboard_open: bool,
    /// Games the server is about, for a future discovery feature.
    #[serde(default)]
    pub game_tags: Vec<String>,
    /// IGDB id and cover for tags picked from search, keyed by tag name.
    #[serde(default)]
    pub game_tag_meta: Vec<GameTagMeta>,
    /// Rules new members must accept before posting (markdown).
    #[serde(default)]
    pub rules: Option<String>,
    #[serde(default)]
    pub rules_accepted: Vec<UserId>,
    #[serde(default)]
    pub events: Vec<ScheduledEvent>,
    /// Ids of deleted events, so a late re-announce cannot restore one.
    #[serde(default)]
    pub deleted_events: Vec<String>,
    /// Members a moderator muted or deafened in voice for everyone.
    #[serde(default)]
    pub server_muted: Vec<UserId>,
    #[serde(default)]
    pub server_deafened: Vec<UserId>,
    /// Banned users; their joins are ignored.
    #[serde(default)]
    pub banned: Vec<UserId>,
    /// Server-wide voice idle limit in minutes; None leaves it to each member.
    #[serde(default)]
    pub afk_timeout_minutes: Option<u32>,
    /// Marked public by someone who manages it: anyone may join, so
    /// invite links to it may be made that never expire.
    #[serde(default)]
    pub public: bool,
}

impl Server {
    /// Creates a server with the usual defaults: a "general" text
    /// channel, a "General" voice channel, and an everyone role.
    pub fn new(name: impl Into<String>, owner: UserId) -> Self {
        let everyone = Role {
            id: RoleId::new(),
            name: "everyone".to_string(),
            color: None,
            permissions: Permissions::default_member(),
            position: 0,
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
        }
    }

    pub fn channel(&self, id: ChannelId) -> Option<&Channel> {
        self.channels.iter().find(|c| c.id == id)
    }

    pub fn member(&self, user: UserId) -> Option<&Member> {
        self.members.iter().find(|m| m.user == user)
    }

    pub fn is_member(&self, user: UserId) -> bool {
        self.member(user).is_some()
    }

    /// Effective permissions for a member: union of the everyone role
    /// and every role they hold. The owner always has administrator.
    pub fn permissions_for(&self, user: UserId) -> Permissions {
        if user == self.owner {
            return Permissions::all();
        }
        let Some(member) = self.member(user) else {
            return Permissions::NONE;
        };
        // Inviting is only ever granted by a role someone was given, so
        // the everyone role cannot hand it out, even on servers created
        // when the default member permissions still included it.
        let mut perms = self
            .roles
            .iter()
            .filter(|r| r.position == 0 || member.roles.contains(&r.id))
            .fold(Permissions::NONE, |acc, r| {
                let mut p = r.permissions;
                if r.position == 0 {
                    p.remove(Permissions::CREATE_INVITES);
                }
                acc | p
            });
        if perms.contains(Permissions::ADMINISTRATOR) {
            perms = Permissions::all();
        }
        perms
    }

    /// The voice channel inactive users get parked in: the one chosen in
    /// server settings, else by convention the voice channel named "AFK".
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

    /// Display color for a member: the highest positioned colored role
    /// they hold, if any.
    pub fn role_color_for(&self, user: UserId) -> Option<String> {
        let member = self.member(user)?;
        self.roles
            .iter()
            .filter(|r| member.roles.contains(&r.id) && r.color.is_some())
            .max_by_key(|r| r.position)
            .and_then(|r| r.color.clone())
    }

    /// Text channels and categories ordered for the sidebar.
    pub fn sorted_channels(&self) -> Vec<&Channel> {
        let mut channels: Vec<&Channel> = self.channels.iter().collect();
        channels.sort_by_key(|c| (c.parent, c.position));
        channels
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
        // A server made before the default changed still has it on everyone.
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

/// A server image (avatar or banner): an iroh blob plus the node that
/// uploaded it, stored as "<blob hash>@<node hex>".
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

/// Most custom emojis a server can hold.
pub const MAX_EMOJIS: usize = 50;
/// Most soundboard clips a server can hold.
pub const MAX_SOUNDS: usize = 24;

/// Emoji and sound names: 2 to 32 of a-z, 0-9 and underscore, used as
/// ":name:" in messages and reactions.
pub fn valid_expression_name(name: &str) -> bool {
    (2..=32).contains(&name.len())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Custom emoji as written into messages and reactions: "<:name:blob@provider>".
/// Carries its image so it renders outside the server it came from.
pub fn parse_emoji_token(token: &str) -> Option<(String, ImageRef)> {
    let inner = token.strip_prefix("<:")?.strip_suffix('>')?;
    let (name, image) = inner.split_once(':')?;
    let image = ImageRef::parse(image)?;
    valid_expression_name(name).then(|| (name.to_string(), image))
}

pub fn emoji_token(name: &str, image: &str) -> String {
    format!("<:{name}:{image}>")
}

/// A reaction is a unicode emoji, a custom ":name:" or a custom emoji token.
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

/// Where a game tag came from on IGDB, for covers and discovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameTagMeta {
    pub name: String,
    pub igdb_id: u64,
    /// IGDB image id of the cover.
    pub cover: Option<String>,
}

/// Keeps metadata only for tags that exist, with safe cover ids.
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

/// RSVP to a scheduled event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rsvp {
    Going,
    Interested,
}

/// A server event on the calendar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledEvent {
    /// Random id chosen by the creator.
    pub id: String,
    pub title: String,
    pub description: String,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    /// Voice channel the event happens in, if any.
    pub channel: Option<ChannelId>,
    pub creator: UserId,
    #[serde(default)]
    pub rsvps: Vec<(UserId, Rsvp)>,
}

/// Choices for the server AFK timeout, in minutes.
pub const SERVER_AFK_MINUTES: &[u32] = &[1, 5, 15, 30, 60];

pub const MAX_GAME_TAGS: usize = 5;
pub const MAX_GAME_TAG_CHARS: usize = 32;
/// Rules travel in one gossip frame, so they are capped in bytes.
pub const MAX_RULES_BYTES: usize = 3000;
pub const MAX_EVENTS: usize = 50;
pub const MAX_EVENT_TITLE_CHARS: usize = 100;
pub const MAX_EVENT_DESCRIPTION_BYTES: usize = 1500;
pub const MAX_ROLE_TAG_CHARS: usize = 12;
pub const MAX_USER_LIMIT: u32 = 99;

/// Trims, drops empties and duplicates (case-insensitive), caps count
/// and length. Returns None if a tag is too long.
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

pub fn valid_role_tag(tag: &str) -> bool {
    if parse_emoji_token(tag).is_some() {
        return true;
    }
    let n = tag.chars().count();
    n >= 1 && n <= MAX_ROLE_TAG_CHARS && !tag.chars().any(char::is_control)
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
