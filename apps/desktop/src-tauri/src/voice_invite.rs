use crate::invite_link::{self, Expiry};
use crate::view::{display_name, server_name, DmMessageView, MessageView};
use crate::{
    commit_message, current_profile, dm_partner, err_to_string, post_dm, require_can_view, seal_and_sign,
    signed_fits_one_gossip_frame, App, CmdResult,
};
use commy_core::{ChannelId, ChannelKind, DmMessage, Message, ServerId, ServerState, UserId, VoiceInvite};
use serde::Serialize;


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

fn invite_from(state: &ServerState, sid: ServerId, cid: ChannelId, me: UserId) -> CmdResult<VoiceInvite> {
    let channel = state.server.channel(cid).ok_or("unknown channel")?;
    if channel.kind != ChannelKind::Voice {
        return Err("that is not a voice channel".into());
    }
    require_can_view(state, me, cid)?;
    Ok(VoiceInvite {
        server: sid,
        channel: cid,
        invite_link: None,
        server_name: server_name(state, Some(me)),
        channel_name: channel.name.clone(),
    })
}

fn build_invite(app: &App, server_id: &str, channel_id: &str, me: UserId) -> CmdResult<VoiceInvite> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let servers = app.servers.lock().unwrap();
    let state = servers.get(&sid).ok_or("unknown server")?;
    invite_from(state, sid, cid, me)
}

async fn link_for_outsiders(app: &App, handle: &tauri::AppHandle, server_id: &str, sid: ServerId) -> Option<String> {
    let (group, public) = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid)?;
        (state.server.group, state.server.public)
    };
    if group {
        return None;
    }
    let expiry = if public { Expiry::Never } else { Expiry::Week };
    invite_link::create(app, handle, server_id, expiry, None).await.ok().map(|created| created.url)
}

#[tauri::command]
pub async fn send_voice_invite(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    to_server_id: String,
    to_channel_id: String,
) -> CmdResult<MessageView> {
    let profile = current_profile(&app)?;
    let invite = build_invite(&app, &server_id, &channel_id, profile.id)?;
    let to_sid: ServerId = to_server_id.parse().map_err(err_to_string)?;
    let to_cid: ChannelId = to_channel_id.parse().map_err(err_to_string)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&to_sid).ok_or("unknown server")?;
        let target = state.server.channel(to_cid).ok_or("unknown channel")?;
        if !matches!(target.kind, ChannelKind::Text | ChannelKind::Announcement) {
            return Err("invites go in a text channel".into());
        }
        require_can_view(state, profile.id, to_cid)?;
    }
    let mut message = Message::new(to_cid, profile.id, "");
    message.attachments.push(invite.into_attachment());
    let _signing = app.signing.read().await;
    seal_and_sign(&app, to_sid, &mut message)?;
    if !signed_fits_one_gossip_frame(&message) {
        return Err("that invite is too long to send".into());
    }
    commit_message(&app, &handle, to_sid, message).await
}

#[tauri::command]
pub async fn send_dm_voice_invite(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    user_id: String,
) -> CmdResult<DmMessageView> {
    let (me, partner) = dm_partner(&app, &user_id)?;
    let mut invite = build_invite(&app, &server_id, &channel_id, me.id)?;
    let already_in = app.servers.lock().unwrap().get(&invite.server).is_some_and(|s| s.server.member(partner).is_some());
    if !already_in {
        invite.invite_link = link_for_outsiders(&app, &handle, &server_id, invite.server).await;
    }
    let mut message = DmMessage::new(me.id, partner, "");
    message.attachments.push(invite.into_attachment());
    let view = post_dm(&app, &handle, &me, partner, message).await?;
    view.ok_or_else(|| "invite not sent".to_string())
}

#[tauri::command]
pub fn voice_channel_presence(
    app: tauri::State<App>,
    server_id: String,
    channel_id: String,
) -> CmdResult<Option<VoicePresenceView>> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let servers = app.servers.lock().unwrap();
    let Some(state) = servers.get(&sid) else {
        return Ok(None);
    };
    Ok(Some(presence_of(state, cid, me)?))
}

pub fn presence_of(state: &ServerState, cid: ChannelId, me: Option<UserId>) -> CmdResult<VoicePresenceView> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use commy_view::voice_invite::{dm_view_of, excerpt, view_of};
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
