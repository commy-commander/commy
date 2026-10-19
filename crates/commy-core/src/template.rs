use crate::ids::UserId;
use crate::permissions::Permissions;
use crate::server::{
    parse_emoji_token, valid_hex_color, valid_role_tag, valid_rules, Channel, ChannelKind, Role, Server,
    MAX_USER_LIMIT, SERVER_AFK_MINUTES,
};
use crate::store::{MAX_CHANNELS, MAX_ROLES};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};

pub const TEMPLATE_VERSION: u32 = 1;
pub const TEMPLATE_CODE_PREFIX: &str = "commy-template:";
pub const MAX_TEMPLATE_CODE_BYTES: usize = 256 * 1024;
pub const MAX_TEMPLATE_NAME_CHARS: usize = 100;
pub const MAX_RETENTION_DAYS: u32 = 365;
const MAX_CHANNEL_NAME_CHARS: usize = 50;
const MAX_ROLE_NAME_CHARS: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateChannel {
    pub name: String,
    pub kind: ChannelKind,
    #[serde(default)]
    pub category: Option<usize>,
    #[serde(default)]
    pub retention_days: Option<u32>,
    #[serde(default)]
    pub user_limit: u32,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateRole {
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    pub permissions: Permissions,
    #[serde(default)]
    pub hoist: bool,
    #[serde(default)]
    pub self_assignable: bool,
    #[serde(default)]
    pub tag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerTemplate {
    pub version: u32,
    pub name: String,
    pub channels: Vec<TemplateChannel>,
    #[serde(default)]
    pub roles: Vec<TemplateRole>,
    pub everyone: Permissions,
    #[serde(default)]
    pub rules: Option<String>,
    #[serde(default)]
    pub afk_channel: Option<usize>,
    #[serde(default)]
    pub afk_disabled: bool,
    #[serde(default)]
    pub afk_timeout_minutes: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    #[error("that is not a server template code")]
    NotATemplate,
    #[error("this template code is too long")]
    TooLong,
    #[error("this template was made with a newer version of Commy; update to use it")]
    NewerVersion,
    #[error("this template has too many channels")]
    TooManyChannels,
    #[error("this template has too many roles")]
    TooManyRoles,
    #[error("a channel in this template has an invalid name")]
    BadChannelName,
    #[error("a role in this template has an invalid name")]
    BadRoleName,
}

fn known_permissions() -> Permissions {
    [
        Permissions::VIEW_CHANNELS,
        Permissions::SEND_MESSAGES,
        Permissions::MANAGE_MESSAGES,
        Permissions::MANAGE_CHANNELS,
        Permissions::MANAGE_SERVER,
        Permissions::KICK_MEMBERS,
        Permissions::BAN_MEMBERS,
        Permissions::CONNECT_VOICE,
        Permissions::SPEAK,
        Permissions::MUTE_MEMBERS,
        Permissions::DEAFEN_MEMBERS,
        Permissions::STREAM,
        Permissions::ATTACH_FILES,
        Permissions::ADD_REACTIONS,
        Permissions::USE_SOUNDBOARD,
        Permissions::USE_EXTERNAL_EMOJIS,
        Permissions::CREATE_INVITES,
        Permissions::MANAGE_ROLES,
        Permissions::ADMINISTRATOR,
        Permissions::MANAGE_EXPRESSIONS,
        Permissions::MANAGE_EVENTS,
        Permissions::PIN_MESSAGES,
        Permissions::POST_ANNOUNCEMENTS,
    ]
    .into_iter()
    .fold(Permissions::NONE, |acc, p| acc | p)
}

fn everyone_may_hold() -> Permissions {
    known_permissions() & !(Permissions::ADMINISTRATOR | Permissions::MANAGE_ROLES)
}

fn is_text_like(kind: ChannelKind) -> bool {
    matches!(kind, ChannelKind::Text | ChannelKind::Announcement)
}

pub fn text_channel_name(name: &str) -> String {
    name.trim().to_lowercase().replace(' ', "-")
}

fn clean_channel_name(name: &str, kind: ChannelKind) -> Result<String, TemplateError> {
    let name = if is_text_like(kind) { text_channel_name(name) } else { name.trim().to_string() };
    let chars = name.chars().count();
    if chars == 0 || chars > MAX_CHANNEL_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(TemplateError::BadChannelName);
    }
    Ok(name)
}

fn clean_color(color: Option<&str>) -> Option<String> {
    color
        .map(|c| c.trim().trim_start_matches('#').to_ascii_lowercase())
        .filter(|c| valid_hex_color(c))
}

fn portable_tag(tag: Option<&str>) -> Option<String> {
    tag.map(str::trim)
        .filter(|t| parse_emoji_token(t).is_none() && valid_role_tag(t))
        .map(String::from)
}

fn clean_template_name(name: &str) -> String {
    name.trim().chars().filter(|c| !c.is_control()).take(MAX_TEMPLATE_NAME_CHARS).collect()
}

impl ServerTemplate {
    pub fn from_server(server: &Server) -> Result<Self, TemplateError> {
        let mut ordered: Vec<&Channel> = server.channels.iter().collect();
        ordered.sort_by_key(|c| c.position);
        let index_of = |id| ordered.iter().position(|c| c.id == id);
        let channels = ordered
            .iter()
            .map(|c| TemplateChannel {
                name: c.name.clone(),
                kind: c.kind,
                category: c.parent.and_then(index_of),
                retention_days: c.retention_days,
                user_limit: c.user_limit,
                color: c.color.clone(),
            })
            .collect();
        let mut roles: Vec<&Role> = server.roles.iter().filter(|r| !r.is_everyone()).collect();
        roles.sort_by_key(|r| r.position);
        let everyone = server
            .roles
            .iter()
            .find(|r| r.is_everyone())
            .map_or_else(Permissions::default_member, |r| r.permissions);
        Self {
            version: TEMPLATE_VERSION,
            name: server.name.clone(),
            channels,
            roles: roles
                .into_iter()
                .map(|r| TemplateRole {
                    name: r.name.clone(),
                    color: r.color.clone(),
                    permissions: r.permissions,
                    hoist: r.hoist,
                    self_assignable: r.self_assignable,
                    tag: portable_tag(r.tag.as_deref()),
                })
                .collect(),
            everyone,
            rules: server.rules.clone(),
            afk_channel: server.afk_channel().and_then(index_of),
            afk_disabled: server.afk_disabled,
            afk_timeout_minutes: server.afk_timeout_minutes,
        }
        .cleaned()
    }

    pub fn cleaned(self) -> Result<Self, TemplateError> {
        if self.version > TEMPLATE_VERSION {
            return Err(TemplateError::NewerVersion);
        }
        if self.channels.len() > MAX_CHANNELS {
            return Err(TemplateError::TooManyChannels);
        }
        if self.roles.len() >= MAX_ROLES {
            return Err(TemplateError::TooManyRoles);
        }
        let kinds: Vec<ChannelKind> = self.channels.iter().map(|c| c.kind).collect();
        let is_category = |i: usize| kinds.get(i) == Some(&ChannelKind::Category);
        let mut channels = Vec::with_capacity(self.channels.len());
        for c in &self.channels {
            channels.push(TemplateChannel {
                name: clean_channel_name(&c.name, c.kind)?,
                kind: c.kind,
                category: c.category.filter(|i| c.kind != ChannelKind::Category && is_category(*i)),
                retention_days: c
                    .retention_days
                    .filter(|d| *d > 0 && is_text_like(c.kind))
                    .map(|d| d.min(MAX_RETENTION_DAYS)),
                user_limit: if c.kind == ChannelKind::Voice { c.user_limit.min(MAX_USER_LIMIT) } else { 0 },
                color: clean_color(c.color.as_deref()),
            });
        }
        if !channels.iter().any(|c| c.kind != ChannelKind::Category) {
            channels.push(TemplateChannel {
                name: "general".into(),
                kind: ChannelKind::Text,
                category: None,
                retention_days: None,
                user_limit: 0,
                color: None,
            });
        }
        let mut roles = Vec::with_capacity(self.roles.len());
        for r in &self.roles {
            let name = r.name.trim().to_string();
            let chars = name.chars().count();
            if chars == 0 || chars > MAX_ROLE_NAME_CHARS || name.chars().any(char::is_control) {
                return Err(TemplateError::BadRoleName);
            }
            roles.push(TemplateRole {
                name,
                color: clean_color(r.color.as_deref()),
                permissions: r.permissions & known_permissions(),
                hoist: r.hoist,
                self_assignable: r.self_assignable,
                tag: portable_tag(r.tag.as_deref()),
            });
        }
        let afk_channel = self
            .afk_channel
            .filter(|i| channels.get(*i).is_some_and(|c| c.kind == ChannelKind::Voice));
        Ok(Self {
            version: TEMPLATE_VERSION,
            name: clean_template_name(&self.name),
            channels,
            roles,
            everyone: self.everyone & everyone_may_hold(),
            rules: self.rules.map(|r| r.trim().to_string()).filter(|r| valid_rules(r)),
            afk_channel,
            afk_disabled: self.afk_disabled,
            afk_timeout_minutes: self.afk_timeout_minutes.filter(|m| SERVER_AFK_MINUTES.contains(m)),
        })
    }

    pub fn build(&self, name: impl Into<String>, owner: UserId) -> Server {
        let mut server = Server::new(name, owner);
        let channels: Vec<Channel> = self
            .channels
            .iter()
            .enumerate()
            .map(|(i, c)| Channel {
                retention_days: c.retention_days,
                user_limit: c.user_limit,
                color: c.color.clone(),
                ..Channel::new(c.name.clone(), c.kind, i as u32)
            })
            .collect();
        let ids: Vec<_> = channels.iter().map(|c| c.id).collect();
        server.channels = channels
            .into_iter()
            .zip(&self.channels)
            .map(|(channel, t)| Channel { parent: t.category.and_then(|i| ids.get(i).copied()), ..channel })
            .collect();
        if let Some(everyone) = server.roles.iter_mut().find(|r| r.is_everyone()) {
            everyone.permissions = self.everyone;
        }
        server.roles.extend(self.roles.iter().enumerate().map(|(i, r)| Role {
            id: crate::ids::RoleId::new(),
            name: r.name.clone(),
            color: r.color.clone(),
            permissions: r.permissions,
            position: i as u32 + 1,
            hoist: r.hoist,
            self_assignable: r.self_assignable,
            tag: r.tag.clone(),
            banner: None,
        }));
        server.rules = self.rules.clone();
        server.afk_channel_id = self.afk_channel.and_then(|i| ids.get(i).copied());
        server.afk_disabled = self.afk_disabled;
        server.afk_timeout_minutes = self.afk_timeout_minutes;
        server
    }

    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("templates serialize");
        format!("{TEMPLATE_CODE_PREFIX}{}", URL_SAFE_NO_PAD.encode(json))
    }

    pub fn decode(code: &str) -> Result<Self, TemplateError> {
        let code = code.trim();
        if code.len() > MAX_TEMPLATE_CODE_BYTES {
            return Err(TemplateError::TooLong);
        }
        let body = code.strip_prefix(TEMPLATE_CODE_PREFIX).ok_or(TemplateError::NotATemplate)?;
        let json = URL_SAFE_NO_PAD.decode(body.trim()).map_err(|_| TemplateError::NotATemplate)?;
        let template: Self = serde_json::from_slice(&json).map_err(|_| TemplateError::NotATemplate)?;
        template.cleaned()
    }

    pub fn looks_like_code(text: &str) -> bool {
        text.trim_start().starts_with(TEMPLATE_CODE_PREFIX)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinTemplate {
    Fresh,
    Friends,
    Gaming,
    Community,
    Study,
}

impl BuiltinTemplate {
    pub const ALL: [BuiltinTemplate; 5] = [Self::Fresh, Self::Friends, Self::Gaming, Self::Community, Self::Study];

    pub fn id(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Friends => "friends",
            Self::Gaming => "gaming",
            Self::Community => "community",
            Self::Study => "study",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Fresh => "Start fresh",
            Self::Friends => "Friends",
            Self::Gaming => "Gaming group",
            Self::Community => "Community",
            Self::Study => "Study group",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Self::Fresh => "A chat channel and a voice room",
            Self::Friends => "Photos, memes and plans",
            Self::Gaming => "Squads, clips and a lobby",
            Self::Community => "Welcome area and moderators",
            Self::Study => "Focus rooms and resources",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == id)
    }
}

struct Draft {
    channels: Vec<TemplateChannel>,
    roles: Vec<TemplateRole>,
    rules: Option<String>,
}

impl Draft {
    fn new() -> Self {
        Self { channels: vec![], roles: vec![], rules: None }
    }

    fn add(&mut self, name: &str, kind: ChannelKind, category: Option<usize>) -> usize {
        self.channels.push(TemplateChannel {
            name: name.into(),
            kind,
            category,
            retention_days: None,
            user_limit: 0,
            color: None,
        });
        self.channels.len() - 1
    }

    fn category(&mut self, name: &str) -> usize {
        self.add(name, ChannelKind::Category, None)
    }

    fn text(&mut self, category: Option<usize>, names: &[&str]) {
        for name in names {
            self.add(name, ChannelKind::Text, category);
        }
    }

    fn announcement(&mut self, category: Option<usize>, name: &str) {
        self.add(name, ChannelKind::Announcement, category);
    }

    fn voice(&mut self, category: Option<usize>, names: &[&str]) {
        for name in names {
            self.add(name, ChannelKind::Voice, category);
        }
    }

    fn voice_limited(&mut self, category: Option<usize>, name: &str, limit: u32) {
        let at = self.add(name, ChannelKind::Voice, category);
        self.channels[at].user_limit = limit;
    }

    fn role(&mut self, name: &str, color: &str, hoist: bool, permissions: Permissions) {
        self.roles.push(TemplateRole {
            name: name.into(),
            color: Some(color.into()),
            permissions,
            hoist,
            self_assignable: false,
            tag: None,
        });
    }

    fn self_role(&mut self, name: &str, color: &str) {
        self.role(name, color, false, Permissions::NONE);
        if let Some(role) = self.roles.last_mut() {
            role.self_assignable = true;
        }
    }

    fn finish(self, title: &str) -> ServerTemplate {
        let afk_channel = self
            .channels
            .iter()
            .rposition(|c| c.kind == ChannelKind::Voice && c.name.eq_ignore_ascii_case("afk"));
        ServerTemplate {
            version: TEMPLATE_VERSION,
            name: title.into(),
            channels: self.channels,
            roles: self.roles,
            everyone: Permissions::default_member(),
            rules: self.rules,
            afk_channel,
            afk_disabled: false,
            afk_timeout_minutes: None,
        }
    }
}

fn moderator_permissions() -> Permissions {
    Permissions::MANAGE_MESSAGES
        | Permissions::PIN_MESSAGES
        | Permissions::KICK_MEMBERS
        | Permissions::MUTE_MEMBERS
        | Permissions::DEAFEN_MEMBERS
        | Permissions::CREATE_INVITES
}

const COMMUNITY_RULES: &str = "1. Be kind and respectful to everyone.\n\
2. Keep conversations in the channel they belong to.\n\
3. No spam, scams or unrequested self-promotion.\n\
4. Respect privacy: never share someone's personal details.\n\
5. Moderators have the final say.";

impl ServerTemplate {
    pub fn builtin(which: BuiltinTemplate) -> Self {
        let mut d = Draft::new();
        match which {
            BuiltinTemplate::Fresh => {
                d.text(None, &["general"]);
                d.voice(None, &["General", "AFK"]);
            }
            BuiltinTemplate::Friends => {
                d.text(None, &["general", "photos", "memes", "plans"]);
                d.voice(None, &["Hangout", "Gaming", "Movie Night", "AFK"]);
            }
            BuiltinTemplate::Gaming => {
                d.text(None, &["general", "looking-for-group", "clips", "strategy"]);
                d.voice(None, &["Lobby"]);
                d.voice_limited(None, "Squad 1", 5);
                d.voice_limited(None, "Squad 2", 5);
                d.voice(None, &["Streaming", "AFK"]);
                let news = d.category("News");
                d.announcement(Some(news), "announcements");
                d.text(Some(news), &["patch-notes"]);
                d.role("Squad Lead", "5b8cff", true, Permissions::MUTE_MEMBERS | Permissions::PIN_MESSAGES | Permissions::MANAGE_EVENTS);
                d.role("Streamer", "a970ff", true, Permissions::POST_ANNOUNCEMENTS);
            }
            BuiltinTemplate::Community => {
                d.text(None, &["welcome"]);
                d.announcement(None, "announcements");
                d.text(None, &["introductions", "general", "off-topic", "media", "help"]);
                d.voice(None, &["Lounge", "Events", "AFK"]);
                let staff = d.category("Staff");
                d.text(Some(staff), &["mod-chat"]);
                d.role("Moderator", "3ddc97", true, moderator_permissions());
                d.role("Event Host", "ff9f43", true, Permissions::MANAGE_EVENTS | Permissions::POST_ANNOUNCEMENTS);
                d.self_role("Events Ping", "5b8cff");
                d.rules = Some(COMMUNITY_RULES.into());
            }
            BuiltinTemplate::Study => {
                d.text(None, &["general", "resources", "homework-help", "focus-log"]);
                d.voice(None, &["Study Room", "Quiet Room", "Break Room", "AFK"]);
                d.role("Tutor", "3ddc97", true, Permissions::PIN_MESSAGES | Permissions::MUTE_MEMBERS);
            }
        }
        d.finish(which.title())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner() -> UserId {
        UserId::new()
    }

    #[test]
    fn every_builtin_is_clean_and_builds() {
        for which in BuiltinTemplate::ALL {
            let template = ServerTemplate::builtin(which);
            assert_eq!(template.clone().cleaned().unwrap(), template, "{which:?}");
            let server = template.build("Test", owner());
            assert_eq!(server.channels.len(), template.channels.len());
            assert!(server.afk_channel().is_some(), "{which:?} has an AFK room");
            assert!(server.roles.iter().filter(|r| r.is_everyone()).count() == 1);
        }
    }

    #[test]
    fn fresh_matches_a_plain_new_server() {
        let built = ServerTemplate::builtin(BuiltinTemplate::Fresh).build("Plain", owner());
        let plain = Server::new("Plain", owner());
        let shape = |s: &Server| s.channels.iter().map(|c| (c.name.clone(), c.kind)).collect::<Vec<_>>();
        assert_eq!(shape(&built), shape(&plain));
        assert_eq!(built.roles[0].permissions, plain.roles[0].permissions);
    }

    #[test]
    fn categories_and_afk_survive_a_round_trip() {
        let original = ServerTemplate::builtin(BuiltinTemplate::Gaming).build("Raid Night", owner());
        let template = ServerTemplate::from_server(&original).unwrap();
        let rebuilt = template.build("Copy", owner());
        let layout = |s: &Server| {
            let mut channels: Vec<&Channel> = s.channels.iter().collect();
            channels.sort_by_key(|c| c.position);
            channels
                .iter()
                .map(|c| (c.name.clone(), c.kind, c.parent.map(|p| s.channel(p).unwrap().name.clone()), c.user_limit))
                .collect::<Vec<_>>()
        };
        assert_eq!(layout(&rebuilt), layout(&original));
        assert_eq!(rebuilt.channel(rebuilt.afk_channel().unwrap()).unwrap().name, "AFK");
        assert!(rebuilt.channels.iter().all(|c| original.channel(c.id).is_none()));
        let names = |s: &Server| s.roles.iter().map(|r| (r.name.clone(), r.position, r.permissions)).collect::<Vec<_>>();
        assert_eq!(names(&rebuilt), names(&original));
        assert_eq!(template.name, "Raid Night");
    }

    #[test]
    fn exports_leave_out_people_messages_and_images() {
        let mut server = ServerTemplate::builtin(BuiltinTemplate::Community).build("Hub", owner());
        let guest = UserId::new();
        server.members.push(crate::server::Member { user: guest, nickname: None, roles: vec![], joined_at: chrono::Utc::now() });
        server.banned.push(guest);
        server.icon = Some("icon@node".into());
        server.channels[0].banner = Some("banner@node".into());
        server.roles[1].banner = Some("role@node".into());
        server.roles[1].tag = Some("<:star:blob@node>".into());
        server.roles[2].tag = Some("HOST".into());
        let code = ServerTemplate::from_server(&server).unwrap().encode();
        let rebuilt = ServerTemplate::decode(&code).unwrap().build("Hub 2", owner());
        assert_eq!(rebuilt.members.len(), 1);
        assert!(rebuilt.banned.is_empty() && rebuilt.icon.is_none());
        assert!(rebuilt.channels.iter().all(|c| c.banner.is_none()));
        assert!(rebuilt.roles.iter().all(|r| r.banner.is_none()));
        assert_eq!(rebuilt.roles[1].tag, None);
        assert_eq!(rebuilt.roles[2].tag.as_deref(), Some("HOST"));
        assert!(!code.contains(&guest.to_string()));
        assert_eq!(rebuilt.rules.as_deref(), Some(COMMUNITY_RULES));
    }

    #[test]
    fn codes_round_trip() {
        let template = ServerTemplate::builtin(BuiltinTemplate::Study);
        let code = template.encode();
        assert!(ServerTemplate::looks_like_code(&format!("  {code}")));
        assert_eq!(ServerTemplate::decode(&format!("\n{code} ")).unwrap(), template);
    }

    #[test]
    fn junk_codes_are_refused() {
        assert_eq!(ServerTemplate::decode("hello"), Err(TemplateError::NotATemplate));
        assert_eq!(ServerTemplate::decode("commy-template:!!!"), Err(TemplateError::NotATemplate));
        let not_json = format!("{TEMPLATE_CODE_PREFIX}{}", URL_SAFE_NO_PAD.encode(b"{\"version\":1}"));
        assert_eq!(ServerTemplate::decode(&not_json), Err(TemplateError::NotATemplate));
        let huge = format!("{TEMPLATE_CODE_PREFIX}{}", "A".repeat(MAX_TEMPLATE_CODE_BYTES));
        assert_eq!(ServerTemplate::decode(&huge), Err(TemplateError::TooLong));
    }

    #[test]
    fn newer_templates_ask_for_an_update() {
        let mut template = ServerTemplate::builtin(BuiltinTemplate::Fresh);
        template.version = TEMPLATE_VERSION + 1;
        assert_eq!(ServerTemplate::decode(&template.encode()), Err(TemplateError::NewerVersion));
    }

    #[test]
    fn everyone_never_gets_administrator_from_a_code() {
        let mut template = ServerTemplate::builtin(BuiltinTemplate::Fresh);
        template.everyone = Permissions::all();
        template.roles.push(TemplateRole {
            name: "Admin".into(),
            color: None,
            permissions: Permissions::all(),
            hoist: true,
            self_assignable: false,
            tag: None,
        });
        let cleaned = ServerTemplate::decode(&template.encode()).unwrap();
        assert!(!cleaned.everyone.contains(Permissions::ADMINISTRATOR));
        assert!(!cleaned.everyone.contains(Permissions::MANAGE_ROLES));
        assert!(cleaned.everyone.contains(Permissions::SEND_MESSAGES));
        assert_eq!(cleaned.roles[0].permissions, known_permissions());
        let server = cleaned.build("Safe", owner());
        let member = UserId::new();
        let mut server = server;
        server.members.push(crate::server::Member { user: member, nickname: None, roles: vec![], joined_at: chrono::Utc::now() });
        assert!(!server.permissions_for(member).contains(Permissions::ADMINISTRATOR));
    }

    #[test]
    fn bad_channel_settings_are_cleaned() {
        let mut template = ServerTemplate::builtin(BuiltinTemplate::Fresh);
        template.channels[0].name = "Big News Here".into();
        template.channels[0].category = Some(1);
        template.channels[0].retention_days = Some(9000);
        template.channels[0].user_limit = 12;
        template.channels[1].user_limit = 500;
        template.channels[1].color = Some("#FF00AA".into());
        template.channels[2].color = Some("red".into());
        template.afk_channel = Some(0);
        template.afk_timeout_minutes = Some(7);
        let cleaned = template.cleaned().unwrap();
        assert_eq!(cleaned.channels[0].name, "big-news-here");
        assert_eq!(cleaned.channels[0].category, None);
        assert_eq!(cleaned.channels[0].retention_days, Some(MAX_RETENTION_DAYS));
        assert_eq!(cleaned.channels[0].user_limit, 0);
        assert_eq!(cleaned.channels[1].user_limit, MAX_USER_LIMIT);
        assert_eq!(cleaned.channels[1].color.as_deref(), Some("ff00aa"));
        assert_eq!(cleaned.channels[2].color, None);
        assert_eq!(cleaned.afk_channel, None);
        assert_eq!(cleaned.afk_timeout_minutes, None);
    }

    #[test]
    fn invalid_names_and_sizes_are_refused() {
        let mut template = ServerTemplate::builtin(BuiltinTemplate::Fresh);
        template.channels[0].name = "   ".into();
        assert_eq!(template.cleaned(), Err(TemplateError::BadChannelName));
        let mut template = ServerTemplate::builtin(BuiltinTemplate::Gaming);
        template.roles[0].name = "x".repeat(51);
        assert_eq!(template.cleaned(), Err(TemplateError::BadRoleName));
        let mut template = ServerTemplate::builtin(BuiltinTemplate::Fresh);
        template.channels = vec![template.channels[0].clone(); MAX_CHANNELS + 1];
        assert_eq!(template.cleaned(), Err(TemplateError::TooManyChannels));
    }

    #[test]
    fn a_template_without_channels_still_gets_one() {
        let mut template = ServerTemplate::builtin(BuiltinTemplate::Fresh);
        template.channels = vec![TemplateChannel {
            name: "Only".into(),
            kind: ChannelKind::Category,
            category: None,
            retention_days: None,
            user_limit: 0,
            color: None,
        }];
        template.afk_channel = None;
        let cleaned = template.cleaned().unwrap();
        assert_eq!(cleaned.channels.len(), 2);
        assert_eq!(cleaned.channels[1].name, "general");
    }

    #[test]
    fn builtin_ids_are_stable() {
        for which in BuiltinTemplate::ALL {
            assert_eq!(BuiltinTemplate::from_id(which.id()), Some(which));
        }
        assert_eq!(BuiltinTemplate::from_id("nope"), None);
    }
}
