use commy_core::{ChannelId, ChannelKind, DmMessage, Message, ServerId, ServerState, UserId, VoiceInvite};
use serde::Serialize;

use crate::act::{can_view, Act};
use crate::{display_name, server_name};

#[derive(Debug, Serialize)]
pub struct VoicePresenceUser {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct VoicePresenceView {
    pub server_name: String,
    pub channel_name: String,
    pub user_limit: u32,
    pub can_view: bool,
    pub users: Vec<VoicePresenceUser>,
}

pub fn invite_from(state: &ServerState, sid: ServerId, cid: ChannelId, me: UserId) -> Act<VoiceInvite> {
    let channel = state.server.channel(cid).ok_or("unknown channel")?;
    if channel.kind != ChannelKind::Voice {
        return Err("that is not a voice channel".into());
    }
    can_view(state, me, cid)?;
    Ok(VoiceInvite {
        server: sid,
        channel: cid,
        invite_link: None,
        server_name: server_name(state, Some(me)),
        channel_name: channel.name.clone(),
    })
}

pub fn invite_target(state: &ServerState, me: UserId, cid: ChannelId) -> Act<()> {
    let target = state.server.channel(cid).ok_or("unknown channel")?;
    if !matches!(target.kind, ChannelKind::Text | ChannelKind::Announcement) {
        return Err("invites go in a text channel".into());
    }
    can_view(state, me, cid)
}

pub fn presence_of(state: &ServerState, cid: ChannelId, me: Option<UserId>) -> Act<VoicePresenceView> {
    let channel = state.server.channel(cid).ok_or("that voice channel is gone")?;
    let can_view = me.is_some_and(|me| state.server.can_view(cid, me));
    let users = if can_view {
        state
            .voice_members(cid)
            .into_iter()
            .map(|u| VoicePresenceUser { id: u.to_string(), name: display_name(state, u) })
            .collect()
    } else {
        Vec::new()
    };
    Ok(VoicePresenceView {
        server_name: server_name(state, me),
        channel_name: channel.name.clone(),
        user_limit: channel.user_limit,
        can_view,
        users,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct VoiceInviteView {
    pub server: String,
    pub channel: String,
    pub invite_link: Option<String>,
    pub server_name: String,
    pub channel_name: String,
}

impl VoiceInviteView {
    pub fn of(invite: VoiceInvite) -> Self {
        Self {
            server: invite.server.to_string(),
            channel: invite.channel.to_string(),
            invite_link: invite.invite_link,
            server_name: invite.server_name,
            channel_name: invite.channel_name,
        }
    }
}

pub fn excerpt(invite: &VoiceInvite) -> String {
    if invite.channel_name.is_empty() {
        "Invite to a voice call".to_string()
    } else {
        format!("Invite to a call in {}", invite.channel_name)
    }
}

pub fn dm_view_of(message: &DmMessage) -> Option<VoiceInviteView> {
    message.voice_invite().map(VoiceInviteView::of)
}

pub fn view_of(message: &Message) -> Option<VoiceInviteView> {
    message.voice_invite().map(VoiceInviteView::of)
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Server, ServerEvent, UserProfile};

    fn state_with_voice() -> (ServerState, UserId, ChannelId) {
        let owner = UserProfile::new("owner");
        let server = Server::new("Rust Hangout", owner.id);
        let voice = server.channels.iter().find(|c| c.kind == ChannelKind::Voice && c.name == "General").unwrap().id;
        let mut state = ServerState::new(server);
        state.profiles.insert(owner.id, owner.clone());
        (state, owner.id, voice)
    }

    #[test]
    fn an_invite_names_the_server_and_voice_channel_but_carries_nobody() {
        let (state, me, voice) = state_with_voice();
        let invite = invite_from(&state, state.server.id, voice, me).unwrap();
        assert_eq!(invite.server, state.server.id);
        assert_eq!(invite.channel, voice);
        assert_eq!(invite.server_name, "Rust Hangout");
        assert_eq!(invite.channel_name, "General");
        assert_eq!(invite.invite_link, None);
        let text = state.server.channels.iter().find(|c| c.kind == ChannelKind::Text).unwrap().id;
        assert_eq!(invite_from(&state, state.server.id, text, me).unwrap_err(), "that is not a voice channel");
        assert!(invite_from(&state, state.server.id, ChannelId::new(), me).is_err());
    }

    #[test]
    fn invites_are_posted_only_in_text_channels_the_sender_can_see() {
        let (state, me, voice) = state_with_voice();
        let text = state.server.channels.iter().find(|c| c.kind == ChannelKind::Text).unwrap().id;
        assert_eq!(invite_target(&state, me, text), Ok(()));
        assert_eq!(invite_target(&state, me, voice).unwrap_err(), "invites go in a text channel");
        assert_eq!(invite_target(&state, me, ChannelId::new()).unwrap_err(), "unknown channel");
    }

    #[test]
    fn presence_is_read_live_from_the_viewer_s_copy_of_the_server() {
        let (mut state, me, voice) = state_with_voice();
        let empty = presence_of(&state, voice, Some(me)).unwrap();
        assert_eq!(empty.channel_name, "General");
        assert_eq!(empty.server_name, "Rust Hangout");
        assert!(empty.can_view);
        assert!(empty.users.is_empty());
        state.apply(ServerEvent::VoiceState { user: me, channel: Some(voice), since: None, muted: false, deafened: false }).unwrap();
        let joined = presence_of(&state, voice, Some(me)).unwrap();
        assert_eq!(joined.users.len(), 1);
        assert_eq!(joined.users[0].id, me.to_string());
        assert_eq!(joined.users[0].name, "owner");
        assert_eq!(presence_of(&state, ChannelId::new(), Some(me)).unwrap_err(), "that voice channel is gone");
        let json = serde_json::to_value(&joined).unwrap();
        assert_eq!(json["users"][0]["name"], "owner");
        assert_eq!(json["user_limit"], 0);
    }

    #[test]
    fn the_message_views_expose_the_card_and_an_excerpt() {
        let (state, me, voice) = state_with_voice();
        let invite = invite_from(&state, state.server.id, voice, me).unwrap();
        assert_eq!(excerpt(&invite), "Invite to a call in General");
        assert_eq!(excerpt(&VoiceInvite::new(state.server.id, voice)), "Invite to a voice call");
        let mut message = Message::new(voice, me, "");
        assert!(view_of(&message).is_none());
        message.attachments.push(invite.clone().into_attachment());
        let view = view_of(&message).unwrap();
        assert_eq!((view.server.as_str(), view.channel.as_str()), (state.server.id.to_string().as_str(), voice.to_string().as_str()));
        assert_eq!(view.channel_name, "General");
        let mut dm = DmMessage::new(me, UserId::new(), "");
        assert!(dm_view_of(&dm).is_none());
        dm.attachments.push(invite.into_attachment());
        assert_eq!(dm_view_of(&dm).unwrap().server_name, "Rust Hangout");
    }
}
