use crate::invite_link::{self, Expiry};
use crate::view::voice_invite::{invite_from, invite_target, presence_of, VoicePresenceView};
use crate::view::{DmMessageView, MessageView};
use crate::{
    commit_message, current_profile, dm_partner, err_to_string, post_dm, seal_and_sign, signed_fits_one_gossip_frame, App,
    CmdResult,
};
use commy_core::{ChannelId, DmMessage, Message, ServerId, UserId, VoiceInvite};

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
        invite_target(state, profile.id, to_cid)?;
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
