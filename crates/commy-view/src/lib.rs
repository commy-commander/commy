pub mod act;
pub mod activities;
pub mod afk;
pub mod calls;
pub mod catalog;
pub mod dms;
pub mod events;
pub mod expressions;
pub mod forum;
pub mod keys;
pub mod links;
pub mod liveness;
pub mod manage;
pub mod moderate;
pub mod pictures;
pub mod pins;
pub mod profile;
pub mod reads;
pub mod roles;
pub mod safety;
pub mod saved;
pub mod search;
pub mod server_tags;
pub mod templates;
pub mod voice_invite;

use crate::keys::KeyHolder;
use commy_core::server::Rsvp;
use commy_core::{
    AutoMod, ChannelId, ChannelKind, DmMessage, DmStore, Hold, Message, MessageId, Permissions, Presence, ServerState,
    UserId, UserProfile,
};
use std::collections::{HashMap, HashSet};
use serde::Serialize;

const EVERYONE_ROLE_POSITION: u32 = 0;

pub fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

pub fn valid_image_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

#[derive(Debug, Serialize)]
pub struct ServerSummary {
    pub id: String,
    pub name: String,
    pub member_count: usize,
    pub icon: Option<String>,
    pub can_invite: bool,
    pub group: bool,
    pub tag: Option<server_tags::ServerTagView>,
}

pub fn server_name(state: &ServerState, viewer: Option<UserId>) -> String {
    if !state.server.group || !state.server.name.trim().is_empty() {
        return state.server.name.clone();
    }
    let mut names: Vec<String> = state
        .server
        .members
        .iter()
        .filter(|m| Some(m.user) != viewer)
        .map(|m| display_name(state, m.user))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    match names.len() {
        0 => "Just you".to_string(),
        1..=3 => names.join(", "),
        n => format!("{}, {} and {} others", names[0], names[1], n - 2),
    }
}

impl ServerSummary {
    pub fn from_state(state: &ServerState, viewer: Option<UserId>) -> Self {
        Self {
            id: state.server.id.to_string(),
            name: server_name(state, viewer),
            member_count: state.server.members.len(),
            icon: state.server.icon.clone(),
            can_invite: viewer.is_some_and(|me| state.server.permissions_for(me).allows(Permissions::CREATE_INVITES)),
            group: state.server.group,
            tag: state.server.tag.as_ref().map(|t| server_tags::tag_view(t, &state.server.name)),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ChannelView {
    pub id: String,
    pub name: String,
    pub kind: ChannelKind,
    pub topic: Option<String>,
    pub voice_users: Vec<String>,
    pub voice_user_ids: Vec<String>,
    pub voice_since: Vec<Option<i64>>,
    pub voice_muted: Vec<bool>,
    pub voice_deafened: Vec<bool>,
    pub retention_days: Option<u32>,
    pub parent: Option<String>,
    pub position: u32,
    pub user_limit: u32,
    pub banner: Option<String>,
    pub color: Option<String>,
    pub pins: usize,
    pub view_roles: Vec<String>,
    pub send_roles: Vec<String>,
    pub slow_mode: u32,
    pub sensitive: bool,
    pub threads: Vec<ThreadView>,
    pub voice_status: Option<String>,
    pub can_set_voice_status: bool,
    pub tags: Vec<ForumTagView>,
    pub posts: Vec<ForumPostView>,
    pub can_view: bool,
    pub can_send: bool,
    pub key_ready: bool,
    pub speakers: Vec<String>,
    pub hands: Vec<String>,
    pub activity: Option<commy_core::Activity>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadView {
    pub root: String,
    pub name: Option<String>,
    pub archived: bool,
    pub locked: bool,
    pub by: String,
}

impl ThreadView {
    fn list(channel: &commy_core::server::Channel, state: &ServerState) -> Vec<ThreadView> {
        channel
            .threads
            .iter()
            .filter(|(root, _)| state.has_message(**root))
            .map(|(root, t)| ThreadView { root: root.to_string(), name: t.name.clone(), archived: t.archived, locked: t.locked, by: t.by.to_string() })
            .collect()
    }
}

#[derive(Debug, Serialize)]
pub struct ForumTagView {
    pub id: String,
    pub name: String,
    pub emoji: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ForumPostView {
    pub message: String,
    pub title: String,
    pub tags: Vec<String>,
    pub author: String,
    pub author_name: String,
    pub replies: usize,
    pub created_at: i64,
    pub last_activity: i64,
    pub excerpt: String,
}

const POST_EXCERPT_CHARS: usize = 240;

fn forum_posts(state: &ServerState, channel: &commy_core::Channel, keys: &KeyHolder) -> Vec<ForumPostView> {
    let messages = state.messages(channel.id);
    let by_id: std::collections::HashMap<MessageId, &Message> = messages.iter().map(|m| (m.id, m)).collect();
    let root_of = |m: &Message| {
        let mut current = m;
        let mut seen = HashSet::new();
        while let Some(parent) = current.reply_to.and_then(|p| by_id.get(&p)) {
            if !seen.insert(current.id) {
                break;
            }
            current = parent;
        }
        current.id
    };
    let mut stats: std::collections::HashMap<MessageId, (usize, i64)> = std::collections::HashMap::new();
    for m in messages.iter().filter(|m| m.reply_to.is_some()) {
        let root = root_of(m);
        if root == m.id {
            continue;
        }
        let entry = stats.entry(root).or_insert((0, 0));
        entry.0 += 1;
        entry.1 = entry.1.max(m.created_at.timestamp_millis());
    }
    let mut posts: Vec<ForumPostView> = channel
        .posts
        .iter()
        .filter_map(|(id, post)| {
            let root = by_id.get(id)?;
            let created_at = root.created_at.timestamp_millis();
            let (replies, last_reply) = stats.get(id).copied().unwrap_or((0, 0));
            let excerpt = keys.open(state, root).map(|m| truncate(&m.content, POST_EXCERPT_CHARS)).unwrap_or_default();
            Some(ForumPostView {
                message: id.to_string(),
                title: post.title.clone(),
                tags: post.tags.iter().map(|t| t.to_string()).collect(),
                author: post.author.to_string(),
                author_name: display_name(state, post.author),
                replies,
                created_at,
                last_activity: created_at.max(last_reply),
                excerpt,
            })
        })
        .collect();
    posts.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
    posts
}

#[derive(Debug, Serialize)]
pub struct MemberView {
    pub id: String,
    pub name: String,
    pub username: String,
    pub nickname: Option<String>,
    pub presence: Presence,
    pub presence_until: Option<i64>,
    pub custom_status: Option<String>,
    pub color: Option<String>,
    pub roles: Vec<String>,
    pub avatar: Option<String>,
    pub avatar_provider: Option<String>,
    pub banner: Option<String>,
    pub status_emoji: Option<String>,
    pub activity: Option<String>,
    pub about: Option<String>,
    pub pronouns: Option<String>,
    pub activity_cover: Option<String>,
    pub activity_since: Option<i64>,
    pub voice_channel: Option<String>,
    pub voice_since: Option<i64>,
    pub voice_muted: bool,
    pub voice_deafened: bool,
    pub tag: Option<String>,
    pub tag_color: Option<String>,
    pub role_banner: Option<String>,
    pub bot: bool,
    pub can_mention_everyone: bool,
    pub priority_speaker: bool,
    pub timeout_until: Option<i64>,
    pub timeout_by: Option<String>,
    pub server_tag: Option<server_tags::ServerTagView>,
}

#[derive(Debug, Serialize)]
pub struct RoleView {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
    pub position: u32,
    pub permissions: Vec<String>,
    pub self_assignable: bool,
    pub hoist: bool,
    pub tag: Option<String>,
    pub banner: Option<String>,
    pub mentionable: bool,
}

#[derive(Debug, Serialize)]
pub struct EventView {
    pub id: String,
    pub title: String,
    pub description: String,
    pub start: String,
    pub end: Option<String>,
    pub channel: Option<String>,
    pub channel_name: Option<String>,
    pub creator: String,
    pub creator_name: String,
    pub going: usize,
    pub interested: usize,
    pub my_rsvp: Option<Rsvp>,
    pub can_edit: bool,
}

#[derive(Debug, Serialize)]
pub struct ScreenShareView {
    pub user: String,
    pub name: String,
    pub channel: String,
}

#[derive(Debug, Serialize)]
pub struct ServerView {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub channels: Vec<ChannelView>,
    pub members: Vec<MemberView>,
    pub screen_shares: Vec<ScreenShareView>,
    pub cameras: Vec<String>,
    pub roles: Vec<RoleView>,
    pub afk_channel: Option<String>,
    pub icon: Option<String>,
    pub banner: Option<String>,
    pub invite_background: Option<String>,
    pub can_manage_server: bool,
    pub can_manage_channels: bool,
    pub can_manage_roles: bool,
    pub can_kick: bool,
    pub can_manage_messages: bool,
    pub can_bypass_slow_mode: bool,
    pub can_invite: bool,
    pub can_pin: bool,
    pub can_post_announcements: bool,
    pub can_view_audit_log: bool,
    pub game_tags: Vec<String>,
    pub game_tag_meta: Vec<commy_core::server::GameTagMeta>,
    pub rules: Option<String>,
    pub needs_rules: bool,
    pub events: Vec<EventView>,
    pub can_manage_events: bool,
    pub server_muted: Vec<String>,
    pub server_deafened: Vec<String>,
    pub banned: Vec<(String, String)>,
    pub afk_timeout_minutes: Option<u32>,
    pub can_mute: bool,
    pub can_move: bool,
    pub can_manage_stage: bool,
    pub can_ban: bool,
    pub can_time_out: bool,
    pub my_timeout_until: Option<i64>,
    pub can_manage_nicknames: bool,
    pub can_change_nickname: bool,
    pub emojis: Vec<EmojiView>,
    pub stickers: Vec<EmojiView>,
    pub emoticons: Vec<EmoticonView>,
    pub sounds: Vec<SoundView>,
    pub emojis_shareable: bool,
    pub stickers_shareable: bool,
    pub emoticons_shareable: bool,
    pub group: bool,
    pub add_restricted: AddRestrictions,
    pub can_add_emojis: bool,
    pub can_add_stickers: bool,
    pub can_add_emoticons: bool,
    pub can_use_external: bool,
    pub soundboard_open: bool,
    pub sounds_shareable: bool,
    pub activities_enabled: bool,
    pub uploads_restricted: bool,
    pub can_attach_files: bool,
    pub can_send_voice_messages: bool,
    pub can_send_tts_messages: bool,
    pub can_embed_links: bool,
    pub can_read_history: bool,
    pub can_create_threads: bool,
    pub can_send_polls: bool,
    pub can_create_events: bool,
    pub can_use_voice_filters: bool,
    pub chat_restricted: ChatRestrictions,
    pub public: bool,
    pub can_manage_expressions: bool,
    pub can_upload_sounds: bool,
    pub can_use_soundboard: bool,
    pub join_sounds_allowed: bool,
    pub join_sounds: std::collections::BTreeMap<String, String>,
    pub my_join_sound: Option<String>,
    pub server_tag: Option<server_tags::ServerTagView>,
    pub automod: Option<AutoMod>,
    pub onboarding: Option<commy_core::Onboarding>,
    pub commands: Vec<BotCommandsView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BotCommandsView {
    pub bot: String,
    pub bot_name: String,
    pub commands: Vec<commy_core::BotCommand>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandUseView {
    pub bot: String,
    pub bot_name: String,
    pub name: String,
    pub options: std::collections::BTreeMap<String, String>,
}

impl CommandUseView {
    fn build(message: &Message, state: &ServerState) -> Option<Self> {
        let used = message.command_use()?;
        Some(Self {
            bot: used.bot.to_string(),
            bot_name: display_name(state, used.bot),
            name: used.command,
            options: used.options,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EmojiView {
    pub name: String,
    pub image: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EmoticonView {
    pub name: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct AddRestrictions {
    pub emojis: bool,
    pub stickers: bool,
    pub emoticons: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ChatRestrictions {
    pub embed_links: bool,
    pub read_history: bool,
    pub create_threads: bool,
    pub send_polls: bool,
    pub create_events: bool,
    pub voice_filters: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SoundView {
    pub name: String,
    pub blob: String,
    pub volume: f32,
    pub emoji: Option<String>,
}

impl ServerView {
    pub fn with_friend_details(mut self, friends: &HashSet<UserId>, contacts: &HashMap<UserId, UserProfile>) -> Self {
        for member in &mut self.members {
            let Some(contact) = member.id.parse::<UserId>().ok().filter(|id| friends.contains(id)).and_then(|id| contacts.get(&id)) else {
                continue;
            };
            if member.about.is_none() {
                member.about = contact.about.as_deref().map(|a| commy_core::user::truncate_rich(a, commy_core::user::MAX_ABOUT_CHARS));
            }
            if member.banner.is_none() {
                member.banner = contact.banner.clone().filter(|b| commy_core::user::valid_banner(b));
            }
        }
        self
    }

    pub fn for_viewer(state: &ServerState, viewer: Option<UserId>, keys: &KeyHolder) -> Self {
        let manages_channels = viewer.is_some_and(|v| state.server.permissions_for(v).allows(Permissions::MANAGE_CHANNELS));
        let sees = |c: ChannelId| viewer.is_some_and(|v| state.server.can_view(c, v));
        let sets_voice_status = |c: ChannelId| {
            viewer.is_some_and(|v| {
                let in_channel = state.voice_states.get(&v) == Some(&c);
                state.server.is_member(v) && state.server.permissions_for(v).can_set_voice_status(in_channel)
            })
        };
        let channels = state
            .server
            .sorted_channels()
            .into_iter()
            .filter(|c| manages_channels || sees(c.id))
            .map(|c| {
                let can_view = sees(c.id);
                let voice = if can_view { state.voice_members(c.id) } else { Vec::new() };
                ChannelView {
                    id: c.id.to_string(),
                    name: c.name.clone(),
                    kind: c.kind,
                    topic: c.topic.clone(),
                    voice_users: voice.iter().map(|u| display_name(state, *u)).collect(),
                    retention_days: c.retention_days,
                    parent: c.parent.map(|p| p.to_string()),
                    position: c.position,
                    user_limit: c.user_limit,
                    banner: c.banner.clone(),
                    color: c.color.clone(),
                    pins: state.server.pins.iter().filter(|p| p.channel == c.id && state.has_message(p.message)).count(),
                    voice_user_ids: voice.iter().map(|u| u.to_string()).collect(),
                    voice_since: voice.iter().map(|u| state.voice_since(*u)).collect(),
                    voice_muted: voice.iter().map(|u| state.voice_flags(*u).0).collect(),
                    voice_deafened: voice.iter().map(|u| state.voice_flags(*u).1).collect(),
                    view_roles: c.view_roles.iter().map(|r| r.to_string()).collect(),
                    send_roles: c.send_roles.iter().map(|r| r.to_string()).collect(),
                    slow_mode: c.slow_mode,
                    sensitive: c.sensitive,
                    threads: if can_view { ThreadView::list(c, state) } else { Vec::new() },
                    voice_status: c.voice_status.clone().filter(|_| c.kind == ChannelKind::Voice),
                    can_set_voice_status: can_view && c.kind == ChannelKind::Voice && sets_voice_status(c.id),
                    tags: c.tags.iter().map(|t| ForumTagView { id: t.id.to_string(), name: t.name.clone(), emoji: t.emoji.clone() }).collect(),
                    posts: if can_view && c.is_forum() { forum_posts(state, c, keys) } else { Vec::new() },
                    can_view,
                    can_send: can_view && viewer.is_some_and(|v| state.server.can_post_in(c.id, v)),
                    key_ready: can_view && keys.key_ready(state, c.id),
                    speakers: c.speakers.iter().map(|u| u.to_string()).collect(),
                    hands: if can_view { state.hands_in(c.id).iter().map(|u| u.to_string()).collect() } else { Vec::new() },
                    activity: if can_view { state.activities.get(&c.id).cloned() } else { None },
                }
            })
            .collect();

        let now = chrono::Utc::now();
        let members = state
            .member_list()
            .into_iter()
            .map(|p| {
                let tag = member_tag(state, p.id);
                let (voice_muted, voice_deafened) = state.voice_flags(p.id);
                MemberView {
                    id: p.id.to_string(),
                    name: display_name(state, p.id),
                    username: p.username.clone(),
                    nickname: nickname_of(state, p.id),
                    presence: p.presence,
                    presence_until: p.presence_until,
                    custom_status: p.custom_status.clone(),
                    color: state.server.role_color_for(p.id),
                    roles: state
                        .server
                        .member(p.id)
                        .map(|m| m.roles.iter().map(|r| r.to_string()).collect())
                        .unwrap_or_default(),
                    avatar: p
                        .avatar
                        .as_deref()
                        .and_then(commy_core::Avatar::parse)
                        .map(|a| a.encode()),
                    avatar_provider: p.node.clone(),
                    banner: p.banner.clone().filter(|b| commy_core::user::valid_banner(b)),
                    status_emoji: p
                        .status_emoji
                        .clone()
                        .filter(|e| commy_core::user::valid_status_emoji(e)),
                    activity: p
                        .activity
                        .clone()
                        .filter(|_| p.presence.is_visible_online()),
                    about: p.about.as_deref().map(|a| commy_core::user::truncate_rich(a, commy_core::user::MAX_ABOUT_CHARS)),
                    pronouns: state.server.pronouns.get(&p.id).cloned(),
                    activity_cover: p
                        .activity_cover
                        .clone()
                        .filter(|c| p.activity.is_some() && valid_image_id(c)),
                    activity_since: p.activity_since.filter(|_| p.activity.is_some() && p.presence.is_visible_online()),
                    voice_channel: state.voice_states.get(&p.id).map(|c| c.to_string()),
                    voice_since: state.voice_since(p.id),
                    voice_muted,
                    voice_deafened,
                    tag_color: tag.as_ref().and_then(|(_, c)| c.clone()),
                    tag: tag.map(|(t, _)| t),
                    role_banner: state.server.top_role_with(p.id, |r| r.banner.is_some()).and_then(|r| r.banner.clone()),
                    bot: state.is_bot(p.id),
                    can_mention_everyone: state.server.can_mention_everyone(p.id),
                    priority_speaker: state.server.permissions_for(p.id).is_priority_speaker(),
                    timeout_until: state.server.timeout_of(p.id, now).map(|t| t.until.timestamp_millis()),
                    timeout_by: state.server.timeout_of(p.id, now).map(|t| t.by.to_string()),
                    server_tag: server_tags::shown(p.server_tag.as_ref(), state, p.id),
                }
            })
            .collect();

        let mut roles: Vec<RoleView> = state
            .server
            .roles
            .iter()
            .filter(|r| r.position != EVERYONE_ROLE_POSITION)
            .map(|r| RoleView {
                id: r.id.to_string(),
                name: r.name.clone(),
                color: r.color.clone(),
                position: r.position,
                permissions: permission_names(r.permissions),
                self_assignable: r.self_assignable,
                hoist: r.hoist,
                tag: r.tag.clone(),
                banner: r.banner.clone(),
                mentionable: r.mentionable,
            })
            .collect();
        roles.sort_by_key(|r| std::cmp::Reverse(r.position));

        let screen_shares = state
            .screen_shares
            .iter()
            .filter(|(_, channel)| sees(**channel))
            .map(|(user, channel)| ScreenShareView {
                user: user.to_string(),
                name: display_name(state, *user),
                channel: channel.to_string(),
            })
            .collect();

        let perms = viewer
            .map(|v| state.server.permissions_for(v))
            .unwrap_or(Permissions::NONE);
        let is_member = viewer.is_some_and(|v| state.server.is_member(v));

        let events = state
            .server
            .events
            .iter()
            .map(|e| {
                let count = |want: Rsvp| e.rsvps.iter().filter(|(_, r)| *r == want).count();
                EventView {
                    id: e.id.clone(),
                    title: e.title.clone(),
                    description: e.description.clone(),
                    start: e.start.to_rfc3339(),
                    end: e.end.map(|t| t.to_rfc3339()),
                    channel: e.channel.map(|c| c.to_string()),
                    channel_name: e.channel.and_then(|c| state.server.channel(c)).map(|c| c.name.clone()),
                    creator: e.creator.to_string(),
                    creator_name: display_name(state, e.creator),
                    going: count(Rsvp::Going),
                    interested: count(Rsvp::Interested),
                    my_rsvp: viewer.and_then(|v| e.rsvps.iter().find(|(u, _)| *u == v)).map(|(_, r)| *r),
                    can_edit: perms.allows(Permissions::MANAGE_EVENTS) || viewer == Some(e.creator),
                }
            })
            .collect();
        let needs_rules = viewer.is_some_and(|v| {
            state.server.rules.is_some()
                && !perms.allows(Permissions::MANAGE_SERVER)
                && !state.server.rules_accepted.contains(&v)
        });

        Self {
            id: state.server.id.to_string(),
            name: server_name(state, viewer),
            owner: state.server.owner.to_string(),
            channels,
            members,
            screen_shares,
            cameras: state.cameras.keys().map(|u| u.to_string()).collect(),
            roles,
            afk_channel: state.server.afk_channel().map(|c| c.to_string()),
            icon: state.server.icon.clone(),
            banner: state.server.banner.clone(),
            invite_background: state.invite_background().map(str::to_string),
            can_manage_server: perms.allows(Permissions::MANAGE_SERVER),
            can_manage_channels: perms.allows(Permissions::MANAGE_CHANNELS),
            can_manage_roles: perms.allows(Permissions::MANAGE_ROLES),
            can_kick: perms.allows(Permissions::KICK_MEMBERS),
            can_manage_messages: perms.allows(Permissions::MANAGE_MESSAGES),
            can_bypass_slow_mode: perms.bypasses_slow_mode(),
            can_invite: perms.allows(Permissions::CREATE_INVITES),
            can_pin: perms.can_pin(),
            can_post_announcements: perms.can_announce(),
            can_view_audit_log: perms.can_view_audit_log(),
            game_tags: state.server.game_tags.clone(),
            game_tag_meta: state.server.game_tag_meta.clone(),
            rules: state.server.rules.clone(),
            needs_rules,
            events,
            can_manage_events: perms.allows(Permissions::MANAGE_EVENTS),
            server_muted: state.server.server_muted.iter().map(|u| u.to_string()).collect(),
            server_deafened: state.server.server_deafened.iter().map(|u| u.to_string()).collect(),
            banned: if perms.allows(Permissions::BAN_MEMBERS) {
                state.server.banned.iter().map(|u| (u.to_string(), display_name(state, *u))).collect()
            } else {
                Vec::new()
            },
            afk_timeout_minutes: state.server.afk_timeout_minutes,
            can_mute: perms.allows(Permissions::MUTE_MEMBERS),
            can_move: perms.allows(Permissions::MOVE_MEMBERS),
            can_manage_stage: perms.can_manage_stage(),
            can_ban: perms.allows(Permissions::BAN_MEMBERS),
            can_time_out: perms.allows(Permissions::TIME_OUT_MEMBERS),
            my_timeout_until: viewer.and_then(|v| state.server.timeout_of(v, now)).map(|t| t.until.timestamp_millis()),
            can_manage_nicknames: perms.allows(Permissions::MANAGE_NICKNAMES),
            can_change_nickname: is_member && perms.can_change_own_nickname(),
            emojis: state
                .server
                .emojis
                .iter()
                .map(|e| EmojiView { name: e.name.clone(), image: e.blob.clone() })
                .collect(),
            stickers: state
                .server
                .stickers
                .iter()
                .map(|s| EmojiView { name: s.name.clone(), image: s.blob.clone() })
                .collect(),
            emoticons: state
                .server
                .emoticons
                .iter()
                .map(|e| EmoticonView { name: e.name.clone(), text: e.text.clone() })
                .collect(),
            sounds: state
                .server
                .soundboard
                .iter()
                .map(|s| SoundView { name: s.name.clone(), blob: s.blob.clone(), volume: s.volume, emoji: s.emoji.clone() })
                .collect(),
            emojis_shareable: state.server.emojis_shareable,
            stickers_shareable: state.server.stickers_shareable,
            emoticons_shareable: state.server.emoticons_shareable,
            group: state.server.group,
            add_restricted: AddRestrictions {
                emojis: everyone_lacks(state, Permissions::ADD_EMOJIS),
                stickers: everyone_lacks(state, Permissions::ADD_STICKERS),
                emoticons: everyone_lacks(state, Permissions::ADD_EMOTICONS),
            },
            can_add_emojis: is_member && perms.can_add(Permissions::ADD_EMOJIS),
            can_add_stickers: is_member && perms.can_add(Permissions::ADD_STICKERS),
            can_add_emoticons: is_member && perms.can_add(Permissions::ADD_EMOTICONS),
            can_use_external: perms.allows(Permissions::USE_EXTERNAL_EMOJIS),
            soundboard_open: state.server.soundboard_open,
            sounds_shareable: state.server.sounds_shareable,
            activities_enabled: state.server.activities_enabled,
            uploads_restricted: everyone_lacks(state, Permissions::ATTACH_FILES),
            can_attach_files: perms.allows(Permissions::ATTACH_FILES),
            can_send_voice_messages: perms.allows(Permissions::SEND_VOICE_MESSAGES),
            can_send_tts_messages: perms.can_send_tts(),
            can_embed_links: perms.can_embed_links(),
            can_read_history: perms.can_read_history(),
            can_create_threads: is_member && perms.can_create_threads(),
            can_send_polls: is_member && perms.can_send_polls(),
            can_create_events: is_member && perms.can_create_events(),
            can_use_voice_filters: perms.can_use_voice_filters(),
            chat_restricted: ChatRestrictions {
                embed_links: everyone_holds(state, Permissions::NO_EMBED_LINKS),
                read_history: everyone_holds(state, Permissions::NO_READ_MESSAGE_HISTORY),
                create_threads: everyone_holds(state, Permissions::NO_CREATE_THREADS),
                send_polls: everyone_holds(state, Permissions::NO_SEND_POLLS),
                create_events: everyone_holds(state, Permissions::NO_CREATE_EVENTS),
                voice_filters: everyone_holds(state, Permissions::NO_VOICE_FILTERS),
            },
            public: state.server.public,
            can_manage_expressions: perms.allows(Permissions::MANAGE_EXPRESSIONS),
            can_upload_sounds: perms.allows(Permissions::MANAGE_EXPRESSIONS) || (state.server.soundboard_open && is_member),
            can_use_soundboard: perms.allows(Permissions::USE_SOUNDBOARD),
            join_sounds_allowed: !state.server.join_sounds_off,
            join_sounds: join_sound_clips(state),
            my_join_sound: viewer.and_then(|v| state.server.join_sounds.get(&v).cloned()),
            server_tag: state.server.tag.as_ref().map(|t| server_tags::tag_view(t, &state.server.name)),
            automod: perms.allows(Permissions::MANAGE_SERVER).then(|| state.automod().clone()),
            onboarding: is_member.then(|| state.server.onboarding.clone()).flatten().map(|o| {
                o.tidied(|r| commy_core::onboarding::grantable(&state.server.roles, r))
            }),
            commands: state
                .bot_commands()
                .filter(|(bot, _)| state.server.is_current_member(*bot))
                .map(|(bot, commands)| BotCommandsView { bot: bot.to_string(), bot_name: display_name(state, bot), commands: commands.to_vec() })
                .collect(),
        }
    }
}

fn join_sound_clips(state: &ServerState) -> std::collections::BTreeMap<String, String> {
    let clip_of = |sound: &str| match sound.strip_prefix(commy_core::server::BUILT_IN_SOUND_PREFIX) {
        Some(_) => Some(sound.to_string()),
        None => state.server.soundboard.iter().find(|s| s.name == sound).map(|s| s.blob.clone()),
    };
    state
        .server
        .join_sounds
        .keys()
        .filter_map(|user| Some((user.to_string(), clip_of(state.server.join_sound_of(*user)?)?)))
        .collect()
}

fn everyone_holds(state: &ServerState, permission: Permissions) -> bool {
    state.server.everyone_permissions().contains(permission)
}

fn everyone_lacks(state: &ServerState, permission: Permissions) -> bool {
    !state.server.everyone_permissions().contains(permission)
}

pub const EDITABLE_PERMISSIONS: &[(&str, Permissions)] = &[
    ("manage_server", Permissions::MANAGE_SERVER),
    ("manage_channels", Permissions::MANAGE_CHANNELS),
    ("manage_messages", Permissions::MANAGE_MESSAGES),
    ("pin_messages", Permissions::PIN_MESSAGES),
    ("bypass_slow_mode", Permissions::BYPASS_SLOW_MODE),
    ("attach_files", Permissions::ATTACH_FILES),
    ("send_voice_messages", Permissions::SEND_VOICE_MESSAGES),
    ("send_tts_messages", Permissions::SEND_TTS_MESSAGES),
    ("embed_links", Permissions::NO_EMBED_LINKS),
    ("read_message_history", Permissions::NO_READ_MESSAGE_HISTORY),
    ("create_threads", Permissions::NO_CREATE_THREADS),
    ("send_polls", Permissions::NO_SEND_POLLS),
    ("post_announcements", Permissions::POST_ANNOUNCEMENTS),
    ("manage_roles", Permissions::MANAGE_ROLES),
    ("kick_members", Permissions::KICK_MEMBERS),
    ("ban_members", Permissions::BAN_MEMBERS),
    ("mute_members", Permissions::MUTE_MEMBERS),
    ("time_out_members", Permissions::TIME_OUT_MEMBERS),
    ("manage_nicknames", Permissions::MANAGE_NICKNAMES),
    ("change_nickname", Permissions::CHANGE_NICKNAME),
    ("move_members", Permissions::MOVE_MEMBERS),
    ("priority_speaker", Permissions::PRIORITY_SPEAKER),
    ("use_voice_filters", Permissions::NO_VOICE_FILTERS),
    ("manage_expressions", Permissions::MANAGE_EXPRESSIONS),
    ("add_emojis", Permissions::ADD_EMOJIS),
    ("add_stickers", Permissions::ADD_STICKERS),
    ("add_emoticons", Permissions::ADD_EMOTICONS),
    ("use_external_emojis", Permissions::USE_EXTERNAL_EMOJIS),
    ("create_events", Permissions::NO_CREATE_EVENTS),
    ("manage_events", Permissions::MANAGE_EVENTS),
    ("create_invites", Permissions::CREATE_INVITES),
    ("view_audit_log", Permissions::VIEW_AUDIT_LOG),
    ("mention_everyone", Permissions::MENTION_EVERYONE),
    ("administrator", Permissions::ADMINISTRATOR),
];

pub fn permission_names(perms: Permissions) -> Vec<String> {
    EDITABLE_PERMISSIONS
        .iter()
        .filter(|(_, p)| perms.contains(*p))
        .map(|(name, _)| name.to_string())
        .collect()
}

pub fn permissions_from_names(names: &[String]) -> Permissions {
    let mut perms = Permissions::NONE;
    for name in names {
        if let Some((_, p)) = EDITABLE_PERMISSIONS.iter().find(|(n, _)| n == name) {
            perms.insert(*p);
        }
    }
    perms
}

#[derive(Debug, Serialize)]
pub struct DmThreadView {
    pub user: String,
    pub name: String,
    pub last_message: String,
    pub last_at: String,
    pub kind: String,
}

#[derive(Debug, Serialize)]
pub struct DmMessageView {
    pub id: String,
    pub from: String,
    pub from_name: String,
    pub content: String,
    pub created_at: String,
    pub outgoing: bool,
    pub pending: bool,
    pub edited: bool,
    pub reply_to: Option<String>,
    pub reply_preview: Option<ReplyPreview>,
    pub reactions: Vec<ReactionView>,
    pub attachments: Vec<AttachmentView>,
    pub voice_invite: Option<crate::voice_invite::VoiceInviteView>,
}

fn dm_author_name(msg_from: UserId, me: UserId, dms: &DmStore) -> String {
    if msg_from == me {
        "You".to_string()
    } else {
        name_or_unknown(dms.contacts.get(&msg_from))
    }
}

pub fn dm_excerpt(msg: &DmMessage) -> String {
    let excerpt = truncate(&msg.content, 120);
    if excerpt.is_empty() {
        if let Some(invite) = msg.voice_invite() {
            return crate::voice_invite::excerpt(&invite);
        }
        if let Some(file) = msg.attachments.iter().find(|a| !a.is_card()) {
            return file.filename.clone();
        }
    }
    excerpt
}

pub fn dm_message_view(msg: &DmMessage, me: UserId, dms: &DmStore, pending: bool) -> DmMessageView {
    let partner = if msg.from == me { msg.to } else { msg.from };
    DmMessageView {
        id: msg.id.to_string(),
        from: msg.from.to_string(),
        from_name: dm_author_name(msg.from, me, dms),
        content: msg.content.clone(),
        created_at: msg.created_at.to_rfc3339(),
        outgoing: msg.from == me,
        pending,
        edited: msg.edited_at.is_some(),
        reply_to: msg.reply_to.map(|id| id.to_string()),
        reply_preview: msg.reply_to.and_then(|id| {
            let original = dms.post(partner, id)?;
            Some(ReplyPreview {
                author_id: original.from.to_string(),
                author_name: dm_author_name(original.from, me, dms),
                excerpt: dm_excerpt(original),
            })
        }),
        reactions: msg
            .reactions
            .iter()
            .map(|r| ReactionView {
                emoji: r.emoji.clone(),
                count: r.users.len(),
                users: r.users.iter().map(|u| u.to_string()).collect(),
            })
            .collect(),
        attachments: msg
            .attachments
            .iter()
            .filter(|a| !a.is_card())
            .map(|a| AttachmentView {
                filename: a.filename.clone(),
                size_bytes: a.size_bytes,
                is_image: a.is_image(),
                mime: a.mime_type.clone(),
                blob: a.blob.clone(),
                provider: a.provider.clone(),
            })
            .collect(),
        voice_invite: crate::voice_invite::dm_view_of(msg),
    }
}

pub fn dm_thread_views(dms: &DmStore, me: UserId) -> Vec<DmThreadView> {
    dms.partners()
        .into_iter()
        .map(|partner| {
            let last = dms.last_post(partner);
            DmThreadView {
                user: partner.to_string(),
                name: name_or_unknown(dms.contacts.get(&partner)),
                last_message: last
                    .map(|m| {
                        let prefix = if m.from == me { "You: " } else { "" };
                        format!("{prefix}{}", truncate(&dm_excerpt(m), 60))
                    })
                    .unwrap_or_default(),
                last_at: last
                    .map(|m| m.created_at.to_rfc3339())
                    .unwrap_or_default(),
                kind: "inbox".to_string(),
            }
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub struct MessageView {
    pub id: String,
    pub channel: String,
    pub author_id: String,
    pub author_name: String,
    pub content: String,
    pub sealed: bool,
    pub created_at: String,
    pub edited: bool,
    pub reply_to: Option<String>,
    pub reply_preview: Option<ReplyPreview>,
    pub reactions: Vec<ReactionView>,
    pub attachments: Vec<AttachmentView>,
    pub previews: Vec<LinkPreviewView>,
    pub poll: Option<PollView>,
    pub tts: bool,
    pub command: Option<CommandUseView>,
    pub embeds: bool,
    pub pending: bool,
    pub pinned: bool,
    pub held: Option<String>,
    pub voice_invite: Option<crate::voice_invite::VoiceInviteView>,
}

#[derive(Debug, Serialize)]
pub struct PollView {
    pub question: String,
    pub options: Vec<PollOptionView>,
    pub multi: bool,
    pub total: usize,
    pub closed: bool,
    pub closes_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PollOptionView {
    pub text: String,
    pub votes: usize,
    pub mine: bool,
}

impl PollView {
    fn build(message: &Message, state: &ServerState, viewer: Option<UserId>) -> Option<Self> {
        let poll = message.poll()?;
        let tally = state.poll_tally(message.id, chrono::Utc::now())?;
        let mine = viewer.map(|me| state.poll_choices(message.id, me)).unwrap_or(&[]);
        let options = poll
            .options
            .iter()
            .enumerate()
            .map(|(i, text)| PollOptionView {
                text: text.clone(),
                votes: tally.counts.get(i).copied().unwrap_or(0),
                mine: mine.contains(&(i as u8)),
            })
            .collect();
        Some(Self {
            question: poll.question,
            options,
            multi: poll.multi,
            total: tally.voters,
            closed: tally.closed,
            closes_at: poll.closes_at.map(|t| t.to_rfc3339()),
        })
    }
}

#[derive(Debug, Serialize)]
pub struct LinkPreviewView {
    pub url: String,
    pub site: String,
    pub title: String,
    pub description: String,
    pub image: Option<PreviewImageView>,
}

#[derive(Debug, Serialize)]
pub struct PreviewImageView {
    pub blob: String,
    pub provider: Option<String>,
    pub mime: String,
    pub size_bytes: u64,
    pub width: u32,
    pub height: u32,
}

impl LinkPreviewView {
    fn build(preview: commy_core::LinkPreview, carrier: &commy_core::Attachment) -> Self {
        Self {
            image: preview.image.map(|image| PreviewImageView {
                blob: carrier.blob.clone(),
                provider: carrier.provider.clone(),
                mime: image.mime,
                size_bytes: carrier.size_bytes,
                width: image.width,
                height: image.height,
            }),
            url: preview.url,
            site: preview.site,
            title: preview.title,
            description: preview.description,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct PinView {
    pub channel: String,
    pub channel_name: String,
    pub by: String,
    pub by_name: String,
    pub at: String,
    pub message: MessageView,
}

pub fn pin_views(
    state: &ServerState,
    channel: Option<ChannelId>,
    viewer: UserId,
    keys: &KeyHolder,
) -> Vec<PinView> {
    let mut pins: Vec<&commy_core::server::Pin> = state
        .server
        .pins
        .iter()
        .filter(|p| channel.is_none_or(|c| p.channel == c) && state.server.can_view(p.channel, viewer))
        .collect();
    pins.sort_by(|a, b| b.at.cmp(&a.at));
    pins.into_iter()
        .filter_map(|p| {
            let message = state.message(p.message)?;
            Some(PinView {
                channel: p.channel.to_string(),
                channel_name: state.server.channel(p.channel).map(|c| c.name.clone()).unwrap_or_default(),
                by: p.by.to_string(),
                by_name: display_name(state, p.by),
                at: p.at.to_rfc3339(),
                message: MessageView::build(message, state, keys),
            })
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub struct ReplyPreview {
    pub author_id: String,
    pub author_name: String,
    pub excerpt: String,
}

#[derive(Debug, Serialize)]
pub struct ReactionView {
    pub emoji: String,
    pub count: usize,
    pub users: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AttachmentView {
    pub filename: String,
    pub size_bytes: u64,
    pub is_image: bool,
    pub mime: String,
    pub blob: String,
    pub provider: Option<String>,
}

impl MessageView {
    pub fn build(message: &Message, state: &ServerState, keys: &KeyHolder) -> Self {
        let opened = keys.open(state, message);
        let sealed = opened.is_none();
        let message = opened.as_ref().unwrap_or(message);
        let (content, attachments) = if sealed { (String::new(), &[][..]) } else { (message.content.clone(), &message.attachments[..]) };
        let author_permissions = state.server.permissions_for(message.author);
        Self {
            id: message.id.to_string(),
            channel: message.channel.to_string(),
            author_id: message.author.to_string(),
            author_name: display_name(state, message.author),
            content,
            sealed,
            created_at: message.created_at.to_rfc3339(),
            edited: message.edited_at.is_some(),
            reply_to: message.reply_to.map(|id| id.to_string()),
            reply_preview: message.reply_to.and_then(|id| {
                let stored = state.message(id).filter(|m| m.channel == message.channel)?;
                let original = keys.open(state, stored)?;
                let mut excerpt = truncate(&original.content, 120);
                if excerpt.is_empty() {
                    if let Some(poll) = original.poll() {
                        excerpt = truncate(&poll.question, 120);
                    } else if let Some(invite) = original.voice_invite() {
                        excerpt = crate::voice_invite::excerpt(&invite);
                    } else if original.files().next().is_some() {
                        excerpt = "Click to see attachment".to_string();
                    }
                }
                if keys.hold(state, stored).is_some() {
                    excerpt = HELD_EXCERPT.to_string();
                }
                Some(ReplyPreview {
                    author_id: original.author.to_string(),
                    author_name: display_name(state, original.author),
                    excerpt,
                })
            }),
            reactions: message
                .reactions
                .iter()
                .map(|r| ReactionView {
                    emoji: r.emoji.clone(),
                    count: r.users.len(),
                    users: r.users.iter().map(|u| u.to_string()).collect(),
                })
                .collect(),
            attachments: attachments
                .iter()
                .filter(|a| !a.is_card())
                .map(|a| AttachmentView {
                    filename: a.filename.clone(),
                    size_bytes: a.size_bytes,
                    is_image: a.is_image(),
                    mime: a.mime_type.clone(),
                    blob: a.blob.clone(),
                    provider: a.provider.clone(),
                })
                .collect(),
            previews: message.previews_for(author_permissions).take(1).map(|(p, a)| LinkPreviewView::build(p, a)).collect(),
            poll: (!sealed).then(|| PollView::build(message, state, keys.user())).flatten(),
            tts: !sealed && message.speaks_for(author_permissions),
            command: (!sealed).then(|| CommandUseView::build(message, state)).flatten(),
            embeds: message.embeds_for(author_permissions),
            pending: false,
            pinned: state.server.is_pinned(message.id),
            held: None,
            voice_invite: (!sealed).then(|| crate::voice_invite::view_of(message)).flatten(),
        }
    }

    pub fn with_hold(mut self, hold: Option<Hold>) -> Self {
        self.held = hold.map(|h| h.describe().to_string());
        self
    }

    pub fn with_pending(mut self, pending: bool) -> Self {
        self.pending = pending;
        self
    }
}

const HELD_EXCERPT: &str = "Held by auto moderation";

pub type ChannelOpens = HashMap<ChannelId, chrono::DateTime<chrono::Utc>>;

pub fn history_floor(
    state: &ServerState,
    viewer: Option<UserId>,
    opens: &ChannelOpens,
    channel: ChannelId,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let reads = viewer.is_none_or(|v| state.server.permissions_for(v).can_read_history());
    (!reads).then(|| opens.get(&channel).copied().unwrap_or(now))
}

pub fn within_history(floor: Option<chrono::DateTime<chrono::Utc>>, message: &Message) -> bool {
    floor.is_none_or(|from| message.created_at >= from)
}

pub fn message_views(
    state: &ServerState,
    channel: ChannelId,
    limit: usize,
    pending: &HashSet<MessageId>,
    viewer: Option<UserId>,
    keys: &KeyHolder,
    floor: Option<chrono::DateTime<chrono::Utc>>,
) -> Vec<MessageView> {
    let all = state.messages(channel);
    let start = all.iter().position(|m| within_history(floor, m)).unwrap_or(all.len());
    let recent = &all[start..];
    let page = &recent[recent.len().saturating_sub(limit)..];
    visible_views(state, page, pending, viewer, keys)
}

pub fn sees_message<'a>(state: &'a ServerState, viewer: Option<UserId>, keys: &'a KeyHolder) -> impl Fn(&Message) -> bool + 'a {
    let reviews_held = viewer.is_some_and(|v| commy_core::automod::moderates(&state.server, v));
    move |m| reviews_held || Some(m.author) == viewer || keys.hold(state, m).is_none()
}

pub fn held_view(state: &ServerState, message: &Message, keys: &KeyHolder) -> MessageView {
    MessageView::build(message, state, keys).with_hold(keys.hold(state, message))
}

fn visible_views(
    state: &ServerState,
    messages: &[Message],
    pending: &HashSet<MessageId>,
    viewer: Option<UserId>,
    keys: &KeyHolder,
) -> Vec<MessageView> {
    let sees = sees_message(state, viewer, keys);
    messages
        .iter()
        .filter(|m| sees(m))
        .map(|m| held_view(state, m, keys).with_pending(pending.contains(&m.id)))
        .collect()
}

pub const AROUND_EACH_SIDE: usize = 50;

pub fn message_views_around(
    state: &ServerState,
    channel: ChannelId,
    around: MessageId,
    pending: &HashSet<MessageId>,
    viewer: Option<UserId>,
    keys: &KeyHolder,
    floor: Option<chrono::DateTime<chrono::Utc>>,
) -> Option<Vec<MessageView>> {
    let all = state.messages(channel);
    let at = all.iter().position(|m| m.id == around)?;
    if !sees_message(state, viewer, keys)(&all[at]) || !within_history(floor, &all[at]) {
        return None;
    }
    let first = all.iter().position(|m| within_history(floor, m)).unwrap_or(all.len());
    let start = at.saturating_sub(AROUND_EACH_SIDE).max(first);
    let end = (at + AROUND_EACH_SIDE + 1).min(all.len());
    Some(visible_views(state, &all[start..end], pending, viewer, keys))
}

pub fn nickname_of(state: &ServerState, user: UserId) -> Option<String> {
    state.server.member(user).and_then(|m| m.nickname.clone())
}

pub fn known_name_of(state: &ServerState, user: UserId) -> Option<String> {
    nickname_of(state, user).or_else(|| state.profiles.get(&user).map(|p| p.visible_name().to_string()))
}

pub fn display_name(state: &ServerState, user: commy_core::UserId) -> String {
    known_name_of(state, user).unwrap_or_else(|| UNKNOWN_USER.to_string())
}

pub const UNKNOWN_USER: &str = "Unknown User";

pub fn name_or_unknown(profile: Option<&commy_core::UserProfile>) -> String {
    profile.map_or_else(|| UNKNOWN_USER.to_string(), |p| p.visible_name().to_string())
}

pub fn member_tag(state: &ServerState, user: UserId) -> Option<(String, Option<String>)> {
    state
        .server
        .top_role_with(user, |r| r.tag.is_some())
        .map(|r| (r.tag.clone().unwrap_or_default(), r.color.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Identity, Server, ServerEvent, UserProfile};

    #[test]
    fn pending_messages_are_marked_in_views() {
        let identity = Identity::generate();
        let mut owner = UserProfile::new("owner");
        owner.identity = Some(identity.public());
        let server = Server::new("s", owner.id);
        let channel = server.channels[0].id;
        let mut state = ServerState::new(server);
        state.profiles.insert(owner.id, owner.clone());
        let mut ids = Vec::new();
        for text in ["sent", "waiting"] {
            let mut message = Message::new(channel, owner.id, text);
            message.sign(state.server.id, &identity);
            ids.push(message.id);
            state.apply(ServerEvent::MessageCreate { message }).unwrap();
        }
        let pending = HashSet::from([ids[1]]);
        let keys = KeyHolder::default();
        let views = message_views(&state, channel, 100, &pending, Some(owner.id), &keys, None);
        let flags: Vec<(&str, bool)> = views.iter().map(|v| (v.content.as_str(), v.pending)).collect();
        assert_eq!(flags, vec![("sent", false), ("waiting", true)]);
        assert!(!MessageView::build(state.message(ids[1]).unwrap(), &state, &keys).pending);
        let json = serde_json::to_value(&views[1]).unwrap();
        assert_eq!(json["pending"], true);
    }

    #[test]
    fn held_messages_show_only_to_moderators_and_their_author() {
        let owner_key = Identity::generate();
        let mut owner = UserProfile::new("owner");
        owner.identity = Some(owner_key.public());
        let server = Server::new("s", owner.id);
        let (sid, channel) = (server.id, server.channels[0].id);
        let mut state = ServerState::new(server);
        state.profiles.insert(owner.id, owner.clone());
        let mut keys = Vec::new();
        for name in ["author", "reader"] {
            let identity = Identity::generate();
            let mut profile = UserProfile::new(name);
            profile.identity = Some(identity.public());
            keys.push((profile.id, identity));
            state.apply(ServerEvent::MemberJoin { server: sid, profile }).unwrap();
        }
        let [(author, author_key), (reader, _)] = <[_; 2]>::try_from(keys).ok().unwrap();
        let mut post = |text: &str, reply_to| {
            let mut message = Message::new(channel, author, text);
            message.reply_to = reply_to;
            message.sign(sid, &author_key);
            let id = message.id;
            state.apply(ServerEvent::MessageCreate { message }).unwrap();
            id
        };
        let bad = post("cheap gold here", None);
        post("fine", Some(bad));
        let rules = AutoMod { blocked_terms: vec!["gold".into()], ..AutoMod::default() };
        state.apply(ServerEvent::AutoModSet { server: sid, rules }).unwrap();
        let seen = |viewer| {
            message_views(&state, channel, 100, &HashSet::new(), Some(viewer), &KeyHolder::default(), None)
                .into_iter()
                .map(|v| (v.content, v.held))
                .collect::<Vec<_>>()
        };
        let all = vec![("cheap gold here".to_string(), Some("blocked word".to_string())), ("fine".to_string(), None)];
        assert_eq!(seen(owner.id), all);
        assert_eq!(seen(author), all);
        assert_eq!(seen(reader), vec![("fine".to_string(), None)]);
        let reply = message_views(&state, channel, 100, &HashSet::new(), Some(reader), &KeyHolder::default(), None).remove(0);
        assert_eq!(reply.reply_preview.unwrap().excerpt, HELD_EXCERPT);
        let around = |viewer| message_views_around(&state, channel, bad, &HashSet::new(), Some(viewer), &KeyHolder::default(), None).map(|v| v.len());
        assert_eq!(around(owner.id), Some(2));
        assert_eq!(around(reader), None);
    }

    #[test]
    fn a_window_around_an_old_message_holds_it_and_its_neighbours() {
        let identity = Identity::generate();
        let mut owner = UserProfile::new("owner");
        owner.identity = Some(identity.public());
        let server = Server::new("s", owner.id);
        let channel = server.channels[0].id;
        let mut state = ServerState::new(server);
        state.profiles.insert(owner.id, owner.clone());
        let mut ids = Vec::new();
        for i in 0..300 {
            let mut message = Message::new(channel, owner.id, format!("m{i}"));
            message.created_at = chrono::Utc::now() - chrono::Duration::minutes(300 - i);
            message.sign(state.server.id, &identity);
            ids.push(message.id);
            state.apply(ServerEvent::MessageCreate { message }).unwrap();
        }
        let none = HashSet::new();
        let middle = message_views_around(&state, channel, ids[120], &none, Some(owner.id), &KeyHolder::default(), None).unwrap();
        assert_eq!(middle.len(), 2 * AROUND_EACH_SIDE + 1);
        assert_eq!((middle[0].content.as_str(), middle[AROUND_EACH_SIDE].content.as_str()), ("m70", "m120"));
        let first = message_views_around(&state, channel, ids[3], &none, Some(owner.id), &KeyHolder::default(), None).unwrap();
        assert_eq!((first[0].content.as_str(), first.len()), ("m0", 3 + AROUND_EACH_SIDE + 1));
        let last = message_views_around(&state, channel, ids[299], &none, Some(owner.id), &KeyHolder::default(), None).unwrap();
        assert_eq!(last.last().unwrap().content, "m299");
        assert!(message_views_around(&state, channel, MessageId::for_author(owner.id), &none, Some(owner.id), &KeyHolder::default(), None).is_none());
    }

    fn keyed(name: &str) -> (UserProfile, Identity) {
        let identity = Identity::generate();
        let mut profile = UserProfile::new(name);
        profile.identity = Some(identity.public());
        (profile, identity)
    }

    #[test]
    fn without_read_message_history_only_what_arrives_after_opening_the_channel_shows() {
        let (owner, owner_key) = keyed("owner");
        let server = Server::new("s", owner.id);
        let (sid, channel) = (server.id, server.channels[0].id);
        let mut state = ServerState::new(server);
        state.profiles.insert(owner.id, owner.clone());
        let (reader, _) = keyed("reader");
        let reader_id = reader.id;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: reader }).unwrap();
        let now = chrono::Utc::now();
        let mut ids = Vec::new();
        for (text, seconds_ago) in [("old", 1800), ("recent", 300), ("new", 1)] {
            let mut message = Message::new(channel, owner.id, text);
            message.created_at = now - chrono::Duration::seconds(seconds_ago);
            message.sign(sid, &owner_key);
            ids.push(message.id);
            state.apply(ServerEvent::MessageCreate { message }).unwrap();
        }
        let none = HashSet::new();
        let keys = KeyHolder::default();
        let opens: ChannelOpens = HashMap::from([(channel, now - chrono::Duration::minutes(10))]);
        assert_eq!(history_floor(&state, Some(reader_id), &opens, channel, now), None, "readers see everything by default");
        let mut everyone = state.server.roles[0].clone();
        everyone.permissions.insert(Permissions::NO_READ_MESSAGE_HISTORY);
        state.apply(ServerEvent::RoleUpdate { server: sid, role: everyone }).unwrap();
        assert_eq!(history_floor(&state, Some(owner.id), &opens, channel, now), None, "the owner keeps history");
        let floor = history_floor(&state, Some(reader_id), &opens, channel, now);
        assert_eq!(floor, Some(now - chrono::Duration::minutes(10)));
        let texts = |views: Vec<MessageView>| views.into_iter().map(|v| v.content).collect::<Vec<_>>();
        assert_eq!(texts(message_views(&state, channel, 100, &none, Some(reader_id), &keys, floor)), ["recent", "new"]);
        assert_eq!(texts(message_views(&state, channel, 1, &none, Some(reader_id), &keys, floor)), ["new"]);
        assert_eq!(texts(message_views(&state, channel, 100, &none, Some(reader_id), &keys, None)), ["old", "recent", "new"]);
        let unopened = history_floor(&state, Some(reader_id), &ChannelOpens::new(), channel, now);
        assert_eq!(unopened, Some(now));
        assert!(message_views(&state, channel, 100, &none, Some(reader_id), &keys, unopened).is_empty(), "a channel never opened shows nothing yet");
        assert!(message_views_around(&state, channel, ids[0], &none, Some(reader_id), &keys, floor).is_none(), "no jumping into hidden history");
        let around = message_views_around(&state, channel, ids[2], &none, Some(reader_id), &keys, floor).unwrap();
        assert_eq!(texts(around), ["recent", "new"], "a window stops at the floor");
    }

    #[test]
    fn link_cards_and_embeds_follow_the_authors_embed_links_permission() {
        let (owner, _) = keyed("owner");
        let server = Server::new("s", owner.id);
        let (sid, channel) = (server.id, server.channels[0].id);
        let mut state = ServerState::new(server);
        state.profiles.insert(owner.id, owner.clone());
        let (author, author_key) = keyed("author");
        let author_id = author.id;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: author }).unwrap();
        let preview = commy_core::LinkPreview {
            url: "https://example.org/a".into(),
            site: String::new(),
            title: "A page".into(),
            description: String::new(),
            image: None,
        };
        let mut message = Message::new(channel, author_id, "see https://example.org/a");
        message.attachments.push(preview.into_attachment(None));
        message.sign(sid, &author_key);
        let id = message.id;
        state.apply(ServerEvent::MessageCreate { message }).unwrap();
        let keys = KeyHolder::default();
        let view = MessageView::build(state.message(id).unwrap(), &state, &keys);
        assert!(view.embeds);
        assert_eq!(view.previews.len(), 1);
        let mut everyone = state.server.roles[0].clone();
        everyone.permissions.insert(Permissions::NO_EMBED_LINKS);
        state.apply(ServerEvent::RoleUpdate { server: sid, role: everyone }).unwrap();
        let view = MessageView::build(state.message(id).unwrap(), &state, &keys);
        assert!(!view.embeds, "readers drop the card once the author may not embed");
        assert!(view.previews.is_empty());
        assert_eq!(view.content, "see https://example.org/a", "the text itself still posts");
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["embeds"], false);
        let server_view = ServerView::for_viewer(&state, Some(author_id), &keys);
        assert!(!server_view.can_embed_links && server_view.chat_restricted.embed_links);
        assert!(server_view.can_read_history && !server_view.chat_restricted.read_history);
        assert!(server_view.can_create_threads && server_view.can_send_polls && server_view.can_create_events);
        assert!(!server_view.can_manage_events);
        assert!(permission_names(Permissions::NO_CREATE_EVENTS | Permissions::NO_EMBED_LINKS).contains(&"create_events".to_string()), "a listed name means the restriction is set");
        assert_eq!(permissions_from_names(&["send_polls".to_string(), "read_message_history".to_string()]), Permissions::NO_SEND_POLLS | Permissions::NO_READ_MESSAGE_HISTORY);
        assert!(permission_names(Permissions::default_member()).iter().all(|n| !["embed_links", "send_polls", "create_events"].contains(&n.as_str())));
    }

    #[test]
    fn members_who_declare_themselves_bots_carry_the_badge() {
        let mut owner = UserProfile::new("owner");
        owner.identity = Some(Identity::generate().public());
        let server = Server::new("s", owner.id);
        let sid = server.id;
        let mut state = ServerState::new(server);
        state.profiles.insert(owner.id, owner.clone());
        let bot = UserProfile::new("helper");
        let bot_id = bot.id;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: bot }).unwrap();
        state.apply(ServerEvent::BotSet { server: sid, user: bot_id, bot: true }).unwrap();
        let view = ServerView::for_viewer(&state, Some(owner.id), &KeyHolder::default());
        let badge = |id: UserId| view.members.iter().find(|m| m.id == id.to_string()).unwrap().bot;
        assert!(badge(bot_id));
        assert!(!badge(owner.id));
    }

    #[test]
    fn registered_commands_and_their_uses_show_in_views() {
        use commy_core::command::{BotCommand, CommandUse};
        let mut owner = UserProfile::new("owner");
        owner.identity = Some(Identity::generate().public());
        let server = Server::new("s", owner.id);
        let sid = server.id;
        let channel = server.channels[0].id;
        let mut state = ServerState::new(server);
        state.profiles.insert(owner.id, owner.clone());
        let bot = UserProfile::new("Tidy");
        let bot_id = bot.id;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: bot }).unwrap();
        state.apply(ServerEvent::BotSet { server: sid, user: bot_id, bot: true }).unwrap();
        let roll = BotCommand { name: "roll".into(), description: "Roll dice".into(), options: vec![] };
        state.apply(ServerEvent::BotCommandsSet { server: sid, bot: bot_id, commands: vec![roll.clone()], by: bot_id }).unwrap();
        let view = ServerView::for_viewer(&state, Some(owner.id), &KeyHolder::default());
        assert_eq!(view.commands.len(), 1);
        assert_eq!((view.commands[0].bot_name.as_str(), &view.commands[0].commands), ("Tidy", &vec![roll]));
        let mut message = Message::new(channel, owner.id, "/roll 20");
        message.attachments.push(CommandUse::new(bot_id, "roll").with("sides", "20").into_attachment());
        let shown = MessageView::build(&message, &state, &KeyHolder::default());
        let used = shown.command.unwrap();
        assert_eq!((used.bot_name.as_str(), used.name.as_str(), used.options.get("sides").map(String::as_str)), ("Tidy", "roll", Some("20")));
        assert!(shown.attachments.is_empty(), "the marker is not shown as a file");
        state.apply(ServerEvent::MemberLeave { server: sid, user: bot_id }).unwrap();
        assert!(ServerView::for_viewer(&state, Some(owner.id), &KeyHolder::default()).commands.is_empty());
    }

    #[test]
    fn friends_fill_in_the_details_a_server_was_not_shown() {
        let owner = UserProfile::new("owner");
        let server = Server::new("s", owner.id);
        let sid = server.id;
        let mut state = ServerState::new(server);
        state.profiles.insert(owner.id, owner.clone());
        let mut quiet = UserProfile::new("quiet");
        quiet.about = Some("from a DM".into());
        quiet.banner = Some("blob:dmbanner".into());
        let mut loud = UserProfile::new("loud");
        loud.about = Some("gossiped".into());
        for p in [quiet.without_details(), loud.clone()] {
            state.apply(ServerEvent::MemberJoin { server: sid, profile: p }).unwrap();
        }
        let mut loud_contact = loud.clone();
        loud_contact.about = Some("stale DM bio".into());
        let contacts: HashMap<UserId, UserProfile> = [(quiet.id, quiet.clone()), (loud.id, loud_contact)].into_iter().collect();
        let about = |view: &ServerView, id: UserId| view.members.iter().find(|m| m.id == id.to_string()).unwrap().about.clone();
        let banner = |view: &ServerView, id: UserId| view.members.iter().find(|m| m.id == id.to_string()).unwrap().banner.clone();

        let strangers = ServerView::for_viewer(&state, Some(owner.id), &KeyHolder::default()).with_friend_details(&HashSet::new(), &contacts);
        assert_eq!(about(&strangers, quiet.id), None, "a DM contact who is not a friend adds nothing");

        let friends: HashSet<UserId> = [quiet.id, loud.id].into_iter().collect();
        let view = ServerView::for_viewer(&state, Some(owner.id), &KeyHolder::default()).with_friend_details(&friends, &contacts);
        assert_eq!(about(&view, quiet.id).as_deref(), Some("from a DM"));
        assert_eq!(banner(&view, quiet.id).as_deref(), Some("blob:dmbanner"));
        assert_eq!(about(&view, loud.id).as_deref(), Some("gossiped"), "what the server was shown wins");
    }

    #[test]
    fn voice_status_and_priority_speakers_reach_the_view_with_who_may_change_them() {
        let mut owner = UserProfile::new("owner");
        owner.identity = Some(Identity::generate().public());
        let server = Server::new("s", owner.id);
        let sid = server.id;
        let voice = server.channels[1].id;
        let mut state = ServerState::new(server);
        state.profiles.insert(owner.id, owner.clone());
        let member = UserProfile::new("member");
        let member_id = member.id;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: member }).unwrap();
        let keys = KeyHolder::default();
        fn channel<'a>(view: &'a ServerView, id: ChannelId) -> &'a ChannelView {
            view.channels.iter().find(|c| c.id == id.to_string()).unwrap()
        }
        let before = ServerView::for_viewer(&state, Some(member_id), &keys);
        assert!(!channel(&before, voice).can_set_voice_status, "not in the room");
        assert_eq!(channel(&before, voice).voice_status, None);
        let as_owner = ServerView::for_viewer(&state, Some(owner.id), &keys);
        assert!(channel(&as_owner, voice).can_set_voice_status, "managers always may");
        let join = ServerEvent::VoiceState { user: member_id, channel: Some(voice), muted: false, deafened: false, since: None };
        state.apply(join).unwrap();
        let set = ServerEvent::VoiceStatusSet { server: sid, channel: voice, text: "Ranked grind".into(), by: member_id };
        state.apply(set).unwrap();
        let after = ServerView::for_viewer(&state, Some(member_id), &keys);
        assert!(channel(&after, voice).can_set_voice_status);
        assert_eq!(channel(&after, voice).voice_status.as_deref(), Some("Ranked grind"));
        let priority = |view: &ServerView, id: UserId| view.members.iter().find(|m| m.id == id.to_string()).unwrap().priority_speaker;
        assert!(priority(&after, owner.id));
        assert!(!priority(&after, member_id));
        assert_eq!(permissions_from_names(&["priority_speaker".to_string()]), Permissions::PRIORITY_SPEAKER);
        assert_eq!(permission_names(Permissions::PRIORITY_SPEAKER), vec!["priority_speaker".to_string()]);
    }

    #[test]
    fn onboarding_questions_reach_members_without_roles_that_can_no_longer_be_handed_out() {
        use commy_core::onboarding::{Answer, Onboarding, Question};
        use commy_core::{Permissions, Role, RoleId};
        let owner = UserProfile::new("owner");
        let server = Server::new("s", owner.id);
        let sid = server.id;
        let mut state = ServerState::new(server);
        state.profiles.insert(owner.id, owner.clone());
        let role = |perms| Role { id: RoleId::new(), name: "r".into(), color: None, permissions: perms, position: 1, hoist: false, self_assignable: false, mentionable: false, tag: None, banner: None };
        let plain = role(Permissions::NONE);
        let admin = role(Permissions::ADMINISTRATOR);
        let gone = RoleId::new();
        for r in [plain.clone(), admin.clone()] {
            state.apply(ServerEvent::RoleCreate { server: sid, role: r }).unwrap();
        }
        let onboarding = Onboarding {
            questions: vec![Question {
                title: "Q".into(),
                description: String::new(),
                multiple: false,
                answers: vec![Answer { label: "A".into(), roles: vec![plain.id, admin.id, gone] }, Answer { label: "B".into(), roles: vec![] }],
            }],
        };
        state.apply(ServerEvent::OnboardingSet { server: sid, onboarding: Some(onboarding) }).unwrap();
        let keys = KeyHolder::default();
        let view = ServerView::for_viewer(&state, Some(owner.id), &keys);
        let shown = view.onboarding.as_ref().expect("members see the questions");
        assert_eq!(shown.questions[0].answers[0].roles, vec![plain.id]);
        assert!(ServerView::for_viewer(&state, Some(UserId::new()), &keys).onboarding.is_none(), "strangers do not");
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["onboarding"]["questions"][0]["answers"][1]["label"], "B");
    }

    #[test]
    fn nicknames_replace_names_inside_the_server_and_keep_the_account_name_beside_them() {
        let mut owner = UserProfile::new("owner");
        owner.identity = Some(Identity::generate().public());
        let server = Server::new("s", owner.id);
        let (sid, channel) = (server.id, server.channels[0].id);
        let mut state = ServerState::new(server);
        state.profiles.insert(owner.id, owner.clone());
        let friend_key = Identity::generate();
        let mut friend = UserProfile::new("sam");
        friend.display_name = Some("Sam".into());
        friend.identity = Some(friend_key.public());
        let friend_id = friend.id;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: friend }).unwrap();
        let mut message = Message::new(channel, friend_id, "hi");
        message.sign(sid, &friend_key);
        state.apply(ServerEvent::MessageCreate { message }).unwrap();
        state.apply(ServerEvent::MemberNickname { server: sid, user: friend_id, nickname: Some("Sammy".into()) }).unwrap();
        let keys = KeyHolder::default();
        let view = ServerView::for_viewer(&state, Some(friend_id), &keys);
        let member = view.members.iter().find(|m| m.id == friend_id.to_string()).unwrap();
        assert_eq!((member.name.as_str(), member.username.as_str(), member.nickname.as_deref()), ("Sammy", "sam", Some("Sammy")));
        let me = view.members.iter().find(|m| m.id == owner.id.to_string()).unwrap();
        assert_eq!((me.name.as_str(), me.nickname.as_deref()), ("owner", None));
        assert!(view.can_change_nickname && !view.can_manage_nicknames);
        let as_owner = ServerView::for_viewer(&state, Some(owner.id), &keys);
        assert!(as_owner.can_change_nickname && as_owner.can_manage_nicknames);
        assert!(!ServerView::for_viewer(&state, Some(UserId::new()), &keys).can_change_nickname);
        let messages = message_views(&state, channel, 10, &HashSet::new(), Some(owner.id), &keys, None);
        assert_eq!(messages[0].author_name, "Sammy");
        assert_eq!(display_name(&state, friend_id), "Sammy");
        assert_eq!(known_name_of(&state, UserId::new()), None);
        let names = permission_names(Permissions::CHANGE_NICKNAME | Permissions::MANAGE_NICKNAMES);
        assert_eq!(names, vec!["manage_nicknames".to_string(), "change_nickname".to_string()]);
        assert_eq!(permissions_from_names(&names), Permissions::CHANGE_NICKNAME | Permissions::MANAGE_NICKNAMES);
    }

    #[test]
    fn pending_dms_are_marked_in_views() {
        let (me, them) = (UserId::new(), UserId::new());
        let dms = DmStore::new();
        let message = DmMessage::new(me, them, "later");
        let waiting = dm_message_view(&message, me, &dms, true);
        assert!(waiting.pending && waiting.outgoing);
        assert_eq!(serde_json::to_value(&waiting).unwrap()["pending"], true);
        assert!(!dm_message_view(&message, me, &dms, false).pending);
    }
}
