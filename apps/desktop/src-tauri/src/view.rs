//! Serializable view models sent to the webview. Kept separate from
//! core types so the UI contract can evolve without touching the model.

use commy_core::{ChannelId, ChannelKind, DmMessage, DmStore, Message, MessageId, Permissions, ServerState, UserId};
use std::collections::HashSet;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ServerSummary {
    pub id: String,
    pub name: String,
    pub member_count: usize,
    pub icon: Option<String>,
    /// The viewer may invite people to this server.
    pub can_invite: bool,
}

impl ServerSummary {
    pub fn from_state(state: &ServerState, viewer: Option<UserId>) -> Self {
        Self {
            id: state.server.id.to_string(),
            name: state.server.name.clone(),
            member_count: state.server.members.len(),
            icon: state.server.icon.clone(),
            can_invite: viewer.is_some_and(|me| state.server.permissions_for(me).allows(Permissions::CREATE_INVITES)),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ChannelView {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub topic: Option<String>,
    /// Users currently connected, voice channels only.
    pub voice_users: Vec<String>,
    /// User ids parallel to `voice_users`, for speaking indicators.
    pub voice_user_ids: Vec<String>,
    pub retention_days: Option<u32>,
    /// Category id this channel sits under.
    pub parent: Option<String>,
    pub position: u32,
    /// Voice member cap; 0 is unlimited.
    pub user_limit: u32,
    /// Banner image ref, if the channel has one.
    pub banner: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MemberView {
    pub id: String,
    pub name: String,
    pub presence: String,
    pub custom_status: Option<String>,
    /// Hex color of their highest colored role, if any.
    pub color: Option<String>,
    /// Ids of roles this member holds.
    pub roles: Vec<String>,
    /// Raw avatar ("blob:..." or "emoji:..."), and the node serving blobs.
    pub avatar: Option<String>,
    pub avatar_provider: Option<String>,
    pub status_emoji: Option<String>,
    pub activity: Option<String>,
    pub about: Option<String>,
    /// IGDB cover image id for their game, when known.
    pub activity_cover: Option<String>,
    /// Tag of their highest tagged role, shown next to their name.
    pub tag: Option<String>,
    pub tag_color: Option<String>,
    /// Banner of their highest role that has one.
    pub role_banner: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RoleView {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
    pub position: u32,
    pub permissions: Vec<String>,
    pub self_assignable: bool,
    pub tag: Option<String>,
    pub banner: Option<String>,
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
    /// The viewer's own answer: "going", "interested" or None.
    pub my_rsvp: Option<String>,
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
    /// Ids of users with their camera on.
    pub cameras: Vec<String>,
    pub roles: Vec<RoleView>,
    pub afk_channel: Option<String>,
    pub icon: Option<String>,
    pub banner: Option<String>,
    pub can_manage_server: bool,
    pub can_manage_channels: bool,
    pub can_manage_roles: bool,
    pub can_kick: bool,
    pub can_manage_messages: bool,
    pub can_invite: bool,
    pub game_tags: Vec<String>,
    pub game_tag_meta: Vec<commy_core::server::GameTagMeta>,
    pub rules: Option<String>,
    /// The viewer must accept the rules before posting.
    pub needs_rules: bool,
    pub events: Vec<EventView>,
    pub can_manage_events: bool,
    pub server_muted: Vec<String>,
    pub server_deafened: Vec<String>,
    /// Banned users with names, for people who can unban.
    pub banned: Vec<(String, String)>,
    pub afk_timeout_minutes: Option<u32>,
    pub can_mute: bool,
    pub can_ban: bool,
    pub emojis: Vec<EmojiView>,
    pub sounds: Vec<SoundView>,
    pub emojis_shareable: bool,
    pub soundboard_open: bool,
    /// Invite links to a public server may never expire.
    pub public: bool,
    pub can_manage_expressions: bool,
    pub can_upload_sounds: bool,
    pub can_use_soundboard: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EmojiView {
    pub name: String,
    /// "blob@provider", fetched with fetch_expression.
    pub image: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SoundView {
    pub name: String,
    pub blob: String,
    pub volume: f32,
}

impl ServerView {
    pub fn from_state(state: &ServerState) -> Self {
        Self::for_viewer(state, None)
    }

    pub fn for_viewer(state: &ServerState, viewer: Option<UserId>) -> Self {
        let channels = state
            .server
            .sorted_channels()
            .into_iter()
            .map(|c| ChannelView {
                id: c.id.to_string(),
                name: c.name.clone(),
                kind: match c.kind {
                    ChannelKind::Text => "text",
                    ChannelKind::Voice => "voice",
                    ChannelKind::Announcement => "announcement",
                    ChannelKind::Category => "category",
                }
                .to_string(),
                topic: c.topic.clone(),
                voice_users: state
                    .voice_members(c.id)
                    .iter()
                    .map(|u| display_name(state, *u))
                    .collect(),
                retention_days: c.retention_days,
                parent: c.parent.map(|p| p.to_string()),
                position: c.position,
                user_limit: c.user_limit,
                banner: c.banner.clone(),
                voice_user_ids: state
                    .voice_members(c.id)
                    .iter()
                    .map(|u| u.to_string())
                    .collect(),
            })
            .collect();

        let members = state
            .member_list()
            .into_iter()
            .map(|p| MemberView {
                id: p.id.to_string(),
                name: p.visible_name().to_string(),
                presence: serde_json::to_value(p.presence)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| "offline".to_string()),
                custom_status: p.custom_status.clone(),
                color: state.server.role_color_for(p.id),
                roles: state
                    .server
                    .member(p.id)
                    .map(|m| m.roles.iter().map(|r| r.to_string()).collect())
                    .unwrap_or_default(),
                // Peer supplied: only well-formed values reach the webview.
                avatar: p
                    .avatar
                    .as_deref()
                    .and_then(commy_core::Avatar::parse)
                    .map(|a| a.encode()),
                avatar_provider: p.node.clone(),
                status_emoji: p
                    .status_emoji
                    .clone()
                    .filter(|e| commy_core::user::is_emoji_like(e)),
                activity: p
                    .activity
                    .clone()
                    .filter(|_| p.presence.is_visible_online()),
                about: p.about.clone().map(|a| a.chars().take(commy_core::user::MAX_ABOUT_CHARS).collect()),
                activity_cover: p
                    .activity_cover
                    .clone()
                    .filter(|c| p.activity.is_some() && crate::igdb::valid_image_id(c)),
                tag: member_tag(state, p.id).map(|(t, _)| t),
                tag_color: member_tag(state, p.id).and_then(|(_, c)| c),
                role_banner: state.server.member(p.id).and_then(|m| {
                    state
                        .server
                        .roles
                        .iter()
                        .filter(|r| m.roles.contains(&r.id) && r.banner.is_some())
                        .max_by_key(|r| r.position)
                        .and_then(|r| r.banner.clone())
                }),
            })
            .collect();

        let mut roles: Vec<RoleView> = state
            .server
            .roles
            .iter()
            .filter(|r| r.position > 0) // the everyone role is implicit
            .map(|r| RoleView {
                id: r.id.to_string(),
                name: r.name.clone(),
                color: r.color.clone(),
                position: r.position,
                permissions: permission_names(r.permissions),
                self_assignable: r.self_assignable,
                tag: r.tag.clone(),
                banner: r.banner.clone(),
            })
            .collect();
        roles.sort_by_key(|r| std::cmp::Reverse(r.position));

        let screen_shares = state
            .screen_shares
            .iter()
            .map(|(user, channel)| ScreenShareView {
                user: user.to_string(),
                name: display_name(state, *user),
                channel: channel.to_string(),
            })
            .collect();

        let perms = viewer
            .map(|v| state.server.permissions_for(v))
            .unwrap_or(Permissions::NONE);

        let events = state
            .server
            .events
            .iter()
            .map(|e| {
                let count = |want: commy_core::server::Rsvp| e.rsvps.iter().filter(|(_, r)| *r == want).count();
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
                    going: count(commy_core::server::Rsvp::Going),
                    interested: count(commy_core::server::Rsvp::Interested),
                    my_rsvp: viewer
                        .and_then(|v| e.rsvps.iter().find(|(u, _)| *u == v))
                        .map(|(_, r)| match r {
                            commy_core::server::Rsvp::Going => "going".to_string(),
                            commy_core::server::Rsvp::Interested => "interested".to_string(),
                        }),
                    can_edit: perms.allows(Permissions::MANAGE_EVENTS) || viewer == Some(e.creator),
                }
            })
            .collect();
        // Owners and server managers are never held at the rules gate.
        let needs_rules = viewer.is_some_and(|v| {
            state.server.rules.is_some()
                && v != state.server.owner
                && !state.server.permissions_for(v).allows(Permissions::MANAGE_SERVER)
                && !state.server.rules_accepted.contains(&v)
        });

        Self {
            id: state.server.id.to_string(),
            name: state.server.name.clone(),
            owner: state.server.owner.to_string(),
            channels,
            members,
            screen_shares,
            cameras: state.cameras.keys().map(|u| u.to_string()).collect(),
            roles,
            afk_channel: state.server.afk_channel().map(|c| c.to_string()),
            icon: state.server.icon.clone(),
            banner: state.server.banner.clone(),
            can_manage_server: perms.allows(Permissions::MANAGE_SERVER),
            can_manage_channels: perms.allows(Permissions::MANAGE_CHANNELS),
            can_manage_roles: perms.allows(Permissions::MANAGE_ROLES),
            can_kick: perms.allows(Permissions::KICK_MEMBERS),
            can_manage_messages: perms.allows(Permissions::MANAGE_MESSAGES),
            can_invite: perms.allows(Permissions::CREATE_INVITES),
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
            can_ban: perms.allows(Permissions::BAN_MEMBERS),
            emojis: state
                .server
                .emojis
                .iter()
                .map(|e| EmojiView { name: e.name.clone(), image: e.blob.clone() })
                .collect(),
            sounds: state
                .server
                .soundboard
                .iter()
                .map(|s| SoundView { name: s.name.clone(), blob: s.blob.clone(), volume: s.volume })
                .collect(),
            emojis_shareable: state.server.emojis_shareable,
            soundboard_open: state.server.soundboard_open,
            public: state.server.public,
            can_manage_expressions: perms.allows(Permissions::MANAGE_EXPRESSIONS),
            can_upload_sounds: perms.allows(Permissions::MANAGE_EXPRESSIONS)
                || (state.server.soundboard_open && viewer.is_some_and(|v| state.server.members.iter().any(|m| m.user == v))),
            can_use_soundboard: perms.allows(Permissions::USE_SOUNDBOARD),
        }
    }
}

/// The permission flags the roles UI can grant, as stable names.
pub const EDITABLE_PERMISSIONS: &[(&str, Permissions)] = &[
    ("manage_server", Permissions::MANAGE_SERVER),
    ("manage_channels", Permissions::MANAGE_CHANNELS),
    ("manage_messages", Permissions::MANAGE_MESSAGES),
    ("manage_roles", Permissions::MANAGE_ROLES),
    ("kick_members", Permissions::KICK_MEMBERS),
    ("ban_members", Permissions::BAN_MEMBERS),
    ("mute_members", Permissions::MUTE_MEMBERS),
    ("manage_expressions", Permissions::MANAGE_EXPRESSIONS),
    ("manage_events", Permissions::MANAGE_EVENTS),
    ("create_invites", Permissions::CREATE_INVITES),
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
    /// "inbox", "request" or "spam".
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
    /// Ours and not delivered yet; it goes out when they come online.
    pub pending: bool,
}

pub fn dm_message_view(msg: &DmMessage, me: UserId, dms: &DmStore, pending: bool) -> DmMessageView {
    let from_name = if msg.from == me {
        "You".to_string()
    } else {
        dms.contacts
            .get(&msg.from)
            .map(|p| p.visible_name().to_string())
            .unwrap_or_else(|| "Unknown User".to_string())
    };
    DmMessageView {
        id: msg.id.to_string(),
        from: msg.from.to_string(),
        from_name,
        content: msg.content.clone(),
        created_at: msg.created_at.to_rfc3339(),
        outgoing: msg.from == me,
        pending,
    }
}

pub fn dm_thread_views(dms: &DmStore, me: UserId) -> Vec<DmThreadView> {
    dms.partners()
        .into_iter()
        .map(|partner| {
            let convo = dms.conversation(partner);
            let last = convo.last();
            DmThreadView {
                user: partner.to_string(),
                name: dms
                    .contacts
                    .get(&partner)
                    .map(|p| p.visible_name().to_string())
                    .unwrap_or_else(|| "Unknown User".to_string()),
                last_message: last
                    .map(|m| {
                        let prefix = if m.from == me { "You: " } else { "" };
                        format!("{prefix}{}", m.content.chars().take(60).collect::<String>())
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
    pub created_at: String,
    pub edited: bool,
    pub reply_to: Option<String>,
    /// Author and opening text of the message replied to, if still present.
    pub reply_preview: Option<ReplyPreview>,
    pub reactions: Vec<ReactionView>,
    pub attachments: Vec<AttachmentView>,
    /// Ours and no member has received it yet.
    pub pending: bool,
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
    pub fn build(message: &Message, state: &ServerState) -> Self {
        Self {
            id: message.id.to_string(),
            channel: message.channel.to_string(),
            author_id: message.author.to_string(),
            author_name: display_name(state, message.author),
            content: message.content.clone(),
            created_at: message.created_at.to_rfc3339(),
            edited: message.edited_at.is_some(),
            reply_to: message.reply_to.map(|id| id.to_string()),
            reply_preview: message.reply_to.and_then(|id| {
                let original = state.messages(message.channel).iter().find(|m| m.id == id)?;
                let mut excerpt: String = original.content.chars().take(120).collect();
                if excerpt.is_empty() && !original.attachments.is_empty() {
                    excerpt = "Click to see attachment".to_string();
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
            attachments: message
                .attachments
                .iter()
                .map(|a| AttachmentView {
                    filename: a.filename.clone(),
                    size_bytes: a.size_bytes,
                    is_image: a.is_image(),
                    mime: a.mime_type.clone(),
                    blob: a.blob.clone(),
                    provider: a.provider.clone(),
                })
                .collect(),
            pending: false,
        }
    }

    pub fn with_pending(mut self, pending: bool) -> Self {
        self.pending = pending;
        self
    }
}

/// The latest messages of a channel, marking those still pending.
pub fn message_views(state: &ServerState, channel: ChannelId, limit: usize, pending: &HashSet<MessageId>) -> Vec<MessageView> {
    state
        .page(channel, None, limit)
        .iter()
        .map(|m| MessageView::build(m, state).with_pending(pending.contains(&m.id)))
        .collect()
}

pub fn display_name(state: &ServerState, user: commy_core::UserId) -> String {
    state
        .profiles
        .get(&user)
        .map(|p| p.visible_name().to_string())
        .unwrap_or_else(|| "Unknown User".to_string())
}

/// Tag and color of the member's highest role that has a tag.
pub fn member_tag(state: &ServerState, user: UserId) -> Option<(String, Option<String>)> {
    let member = state.server.member(user)?;
    state
        .server
        .roles
        .iter()
        .filter(|r| member.roles.contains(&r.id) && r.tag.is_some())
        .max_by_key(|r| r.position)
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
        let views = message_views(&state, channel, 100, &pending);
        let flags: Vec<(&str, bool)> = views.iter().map(|v| (v.content.as_str(), v.pending)).collect();
        assert_eq!(flags, vec![("sent", false), ("waiting", true)]);
        assert!(!MessageView::build(state.message(ids[1]).unwrap(), &state).pending);
        let json = serde_json::to_value(&views[1]).unwrap();
        assert_eq!(json["pending"], true);
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
