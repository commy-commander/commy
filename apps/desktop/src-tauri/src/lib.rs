mod account;
mod activity;
mod afk;
mod entry_list;
mod catchup;
mod doorbell;
#[cfg(all(test, feature = "e2e"))]
mod e2e;
mod expressions;
mod media_engine;
mod igdb;
mod instance;
mod invite_link;
mod outbox;
mod overlay;
mod persist;
mod safety;
mod settle;
mod state;
mod toast;
mod tray;
mod updater;
mod view;
mod wake;

use media_engine::{should_dial, CameraSession, MicTest, ScreenShareSession, ScreenWatch, VoiceSession};
use state::{App, VoiceSettings};
use tauri::{Emitter, Manager};
use view::{
    dm_message_view, dm_thread_views, DmMessageView, DmThreadView, MessageView, ServerSummary,
    ServerView,
};

use base64::Engine as _;
use commy_core::{
    Attachment, Channel, ChannelId, ChannelKind, DmMessage, Identity, Message, MessageId, Permissions,
    Presence, RoleId, ServerEvent, ServerId, ServerState, Signature, StoreError, TombstoneProof, UserId,
    UserProfile, MAX_MESSAGE_BYTES,
};
use commy_core::server::Role;
use commy_media::screen::{SourceKind, StreamQuality};
use view::permissions_from_names;
use commy_media::StreamGuards;
use commy_net::media::MediaHello;
use commy_net::{CommyNode, DmEnvelope, MediaChannel, NetEvent, ServerTicket, WirePayload};

type CmdResult<T> = Result<T, String>;

fn err_to_string(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command]
fn ui_ready() {
    tracing::info!("webview connected over the tauri bridge");
}

#[tauri::command]
fn init_user(app: tauri::State<App>, username: String) -> CmdResult<UserProfile> {
    let username = username.trim();
    if username.is_empty() || username.len() > 32 {
        return Err("username must be 1 to 32 characters".into());
    }
    let mut profile = UserProfile::new(username);
    profile.node = app.node_hex.lock().unwrap().clone();
    profile.identity = app.identity().map(|i| i.public());
    *app.profile.lock().unwrap() = Some(profile.clone());
    app.persist_profile();
    Ok(profile)
}

#[tauri::command]
fn get_user(app: tauri::State<App>) -> Option<UserProfile> {
    app.profile.lock().unwrap().clone()
}

pub(crate) fn current_profile(app: &App) -> CmdResult<UserProfile> {
    let mut profile = app
        .profile
        .lock()
        .unwrap()
        .clone()
        .ok_or("set a username first")?;
    if profile.node.is_none() {
        profile.node = app.node_hex.lock().unwrap().clone();
    }
    if profile.identity.is_none() {
        profile.identity = app.identity().map(|i| i.public());
    }
    Ok(profile)
}

fn signing_key(app: &App) -> CmdResult<Identity> {
    app.identity().ok_or_else(|| "your identity key is not ready yet".to_string())
}

fn sign_dm(app: &App, message: &mut DmMessage) -> CmdResult<()> {
    let node = app.node_hex.lock().unwrap().clone().unwrap_or_default();
    message.sign(&node, &signing_key(app)?);
    Ok(())
}

fn refuse_while_key_may_be_replaced(app: &App) -> CmdResult<()> {
    if app.key_settled() {
        Ok(())
    } else {
        Err("your identity is still being set up; try again in a minute".into())
    }
}

const KEY_SETTLE_MAX: std::time::Duration = std::time::Duration::from_secs(120);

#[tauri::command]
async fn create_server(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    name: String,
) -> CmdResult<ServerView> {
    let name = name.trim().to_string();
    if name.is_empty() || name.len() > 100 {
        return Err("server name must be 1 to 100 characters".into());
    }
    current_profile(&app)?;
    account::require_account(&app)?;

    let server_id = ServerId::new();
    let hosting = async {
        let mut net = app.net.lock().await;
        if let Some(node) = net.as_mut() {
            match node.host_server(server_id, &name).await {
                Ok(ticket) => {
                    app.invites
                        .lock()
                        .unwrap()
                        .insert(server_id, ticket.encode());
                }
                Err(e) => tracing::warn!("hosting server offline: {e}"),
            }
        }
        Ok(())
    };
    let (state, _, signing) = own_server_state(&app, server_id, &name, hosting).await?;
    let view = ServerView::for_viewer(&state, Some(state.server.owner));
    app.servers.lock().unwrap().insert(server_id, state);
    drop(signing);
    app.persist_server(server_id);
    if let Some(dir) = app.data_dir() {
        let _ = persist::save_invites(dir, &app.invites.lock().unwrap().clone());
    }
    entry_list::record_join(&app, server_id);
    entry_list::schedule(&handle);
    let _ = handle.emit("servers-changed", ());
    Ok(view)
}

#[tauri::command]
async fn join_server(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    ticket: String,
) -> CmdResult<ServerSummary> {
    account::require_account(&app)?;
    let ticket = ServerTicket::decode(&ticket).map_err(err_to_string)?;
    let id = ticket.server_id;
    let summary = join_ticket(&app, &handle, ticket).await?;
    entry_list::record_join(&app, id);
    entry_list::schedule(&handle);
    Ok(summary)
}

pub(crate) async fn join_ticket(app: &App, handle: &impl catchup::AppAccess, ticket: ServerTicket) -> CmdResult<ServerSummary> {
    current_profile(app)?;

    let joining = async {
        let reached_peer = {
            let mut net = app.net.lock().await;
            let node = net.as_mut().ok_or("network is not ready yet")?;
            node.join_server(&ticket).await.map_err(err_to_string)?
        };
        app.invites
            .lock()
            .unwrap()
            .insert(ticket.server_id, ticket.encode());
        if let Some(dir) = app.data_dir() {
            let _ = persist::save_invites(dir, &app.invites.lock().unwrap().clone());
        }
        if !reached_peer {
            tracing::warn!("joined topic but no members are online yet");
        }
        Ok(())
    };

    let (mut state, profile, signing) = own_server_state(app, ticket.server_id, &ticket.server_name, joining).await?;
    let _ = state.apply(ServerEvent::MemberJoin { server: ticket.server_id, profile: profile.clone() });
    let summary = ServerSummary::from_state(&state, Some(profile.id));
    app.servers.lock().unwrap().insert(ticket.server_id, state);
    for payload in catchup::await_snapshot(app, ticket.server_id, profile) {
        broadcast(app, ticket.server_id, payload).await;
    }
    drop(signing);

    app.persist_server(ticket.server_id);
    handle.emit("servers-changed", serde_json::Value::Null);
    Ok(summary)
}

async fn own_server_state<'a>(
    app: &'a App,
    id: ServerId,
    name: &str,
    connect: impl std::future::Future<Output = CmdResult<()>>,
) -> CmdResult<(ServerState, UserProfile, tokio::sync::RwLockReadGuard<'a, ()>)> {
    connect.await?;
    let signing = app.signing.read().await;
    let profile = current_profile(app)?;
    let mut server = commy_core::Server::new(name, profile.id);
    server.id = id;
    let mut state = ServerState::new(server);
    state.profiles.insert(profile.id, profile.clone());
    Ok((state, profile, signing))
}

#[tauri::command]
fn list_servers(app: tauri::State<App>) -> Vec<ServerSummary> {
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let servers = app.servers.lock().unwrap();
    let mut list: Vec<ServerSummary> = servers.values().map(|s| ServerSummary::from_state(s, me)).collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

#[tauri::command]
fn get_server(app: tauri::State<App>, server_id: String) -> CmdResult<ServerView> {
    let id = server_id.parse().map_err(err_to_string)?;
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let servers = app.servers.lock().unwrap();
    let state = servers.get(&id).ok_or("unknown server")?;
    Ok(ServerView::for_viewer(state, me))
}

#[tauri::command]
fn get_invite(app: tauri::State<App>, server_id: String) -> CmdResult<String> {
    let id = server_id.parse().map_err(err_to_string)?;
    require_invite_permission(&app, id)?;
    app.invites
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or_else(|| "no invite available; you may be offline".into())
}

#[tauri::command]
fn get_messages(
    app: tauri::State<App>,
    server_id: String,
    channel_id: String,
) -> CmdResult<Vec<MessageView>> {
    let sid = server_id.parse().map_err(err_to_string)?;
    let cid = channel_id.parse().map_err(err_to_string)?;
    let pending = app.outbox.lock().unwrap().pending_ids(sid);
    let servers = app.servers.lock().unwrap();
    let state = servers.get(&sid).ok_or("unknown server")?;
    Ok(view::message_views(state, cid, 100, &pending))
}

async fn commit_message(
    app: &tauri::State<'_, App>,
    handle: &tauri::AppHandle,
    sid: ServerId,
    message: Message,
) -> CmdResult<MessageView> {
    let event = ServerEvent::MessageCreate {
        message: message.clone(),
    };
    let view = {
        let mut servers = app.servers.lock().unwrap();
        let state = servers.get_mut(&sid).ok_or("unknown server")?;
        require_rules_accepted(state, message.author)?;
        require_can_post(&app, state, message.author, message.channel)?;
        state.apply(event.clone()).map_err(err_to_string)?;
        MessageView::build(&message, state)
    };
    let delivered = outbox::broadcast_local(app, sid, event).await;
    app.persist_server(sid);
    doorbell::ring(handle, sid, message.id, !delivered);
    wake::after_send(handle, sid, message.id);
    let _ = handle.emit("server-event", &sid.to_string());
    Ok(view.with_pending(!delivered))
}

fn sign_message(app: &App, sid: ServerId, message: &mut Message) -> CmdResult<()> {
    message.sign(sid, &signing_key(app)?);
    Ok(())
}

#[tauri::command]
async fn send_message(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    content: String,
    reply_to: Option<String>,
) -> CmdResult<MessageView> {
    let content = content.trim().to_string();
    if content.is_empty() || content.len() > 4000 {
        return Err("message must be 1 to 4000 characters".into());
    }
    let sid = server_id.parse().map_err(err_to_string)?;
    let cid = channel_id.parse().map_err(err_to_string)?;
    let profile = current_profile(&app)?;
    let mut message = Message::new(cid, profile.id, content);
    if let Some(raw) = reply_to {
        let target: MessageId = raw.parse().map_err(err_to_string)?;
        let exists = {
            let servers = app.servers.lock().unwrap();
            servers
                .get(&sid)
                .is_some_and(|s| s.messages(cid).iter().any(|m| m.id == target))
        };
        if !exists {
            return Err("the message you replied to no longer exists".into());
        }
        message.reply_to = Some(target);
    }
    let _signing = app.signing.read().await;
    sign_message(&app, sid, &mut message)?;
    if !signed_fits_one_gossip_frame(&message) {
        return Err("that message is too long to send; try splitting it up".into());
    }
    commit_message(&app, &handle, sid, message).await
}

#[tauri::command]
async fn set_reaction(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    message_id: String,
    emoji: String,
    active: bool,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let mid: MessageId = message_id.parse().map_err(err_to_string)?;
    let emoji = emoji.trim().to_string();
    if !commy_core::server::valid_reaction(&emoji) {
        return Err("reactions must be a single emoji".into());
    }
    let me = current_profile(&app)?;
    apply_and_broadcast(
        &app,
        &handle,
        sid,
        ServerEvent::ReactionSet { channel: cid, message: mid, emoji, user: me.id, active },
    )
    .await?;
    Ok(())
}

fn signed_fits_one_gossip_frame(signed: &Message) -> bool {
    let wire = WirePayload::Event(ServerEvent::MessageCreate { message: signed.clone() });
    commy_net::protocol::gossip_size(&wire) <= commy_net::protocol::MAX_GOSSIP_BYTES
}

fn message_author(state: &ServerState, cid: ChannelId, mid: MessageId) -> CmdResult<UserId> {
    state
        .messages(cid)
        .iter()
        .find(|m| m.id == mid)
        .map(|m| m.author)
        .ok_or_else(|| "that message no longer exists".to_string())
}

fn edit_time_after_last_edit(original: &Message, now: chrono::DateTime<chrono::Utc>) -> chrono::DateTime<chrono::Utc> {
    match original.edited_at {
        Some(last) if now.timestamp_millis() <= last.timestamp_millis() => last + chrono::Duration::milliseconds(1),
        _ => now,
    }
}

fn sign_edit(
    sid: ServerId,
    original: &Message,
    content: String,
    now: chrono::DateTime<chrono::Utc>,
    identity: &Identity,
) -> CmdResult<ServerEvent> {
    let edited_at = edit_time_after_last_edit(original, now);
    let placeholder_signature_of_real_size = Some(Signature([0; 64]));
    let edited = Message { content, edited_at: Some(edited_at), sig: placeholder_signature_of_real_size, ..original.clone() };
    if edited.authored_size() > MAX_MESSAGE_BYTES {
        return Err("that edit is too long; try shortening it".into());
    }
    Ok(original.edit_event(sid, edited.content, edited_at, identity))
}

#[tauri::command]
async fn edit_message(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    message_id: String,
    content: String,
) -> CmdResult<()> {
    let content = content.trim().to_string();
    if content.is_empty() || content.len() > 4000 {
        return Err("message must be 1 to 4000 characters".into());
    }
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let mid: MessageId = message_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let _signing = app.signing.read().await;
    let identity = signing_key(&app)?;
    let edit = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        if message_author(state, cid, mid)? != me.id {
            return Err("you can only edit your own messages".into());
        }
        let original = state.messages(cid).iter().find(|m| m.id == mid).ok_or("that message no longer exists")?;
        sign_edit(sid, original, content, chrono::Utc::now(), &identity)?
    };
    apply_and_broadcast(&app, &handle, sid, edit)
        .await?;
    Ok(())
}

#[tauri::command]
async fn delete_message(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    message_id: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let mid: MessageId = message_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let _signing = app.signing.read().await;
    let identity = signing_key(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        may_delete(&app, state, cid, mid, me.id)?;
    }
    let proof = TombstoneProof::sign(sid, cid, mid, me.id, chrono::Utc::now(), &identity);
    apply_and_broadcast(&app, &handle, sid, proof.into_event()).await?;
    Ok(())
}

fn may_delete(app: &App, state: &ServerState, channel: ChannelId, message: MessageId, me: UserId) -> CmdResult<()> {
    if message_author(state, channel, message)? != me {
        require_permission(app, state, me, Permissions::MANAGE_MESSAGES, "delete other people's messages")?;
    }
    Ok(())
}

async fn send_attachment(
    app: &tauri::State<'_, App>,
    handle: &tauri::AppHandle,
    sid: ServerId,
    cid: ChannelId,
    filename: String,
    bytes: Vec<u8>,
) -> CmdResult<MessageView> {
    if bytes.len() > 512 * 1024 * 1024 {
        return Err("files larger than 512 MB are not supported yet".into());
    }
    let profile = current_profile(app)?;
    let size_bytes = bytes.len() as u64;

    let (blob, provider) = {
        let net = app.net.lock().await;
        let node = net.as_ref().ok_or("network is not ready yet")?;
        let hash = node.add_blob(bytes).await.map_err(err_to_string)?;
        (hash, Some(node.node_id_hex()))
    };

    let mime_type = mime_guess::from_path(&filename)
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_string();

    let mut message = Message::new(cid, profile.id, String::new());
    message.attachments.push(Attachment {
        blob,
        filename,
        size_bytes,
        mime_type,
        provider,
    });
    let _signing = app.signing.read().await;
    sign_message(app, sid, &mut message)?;
    commit_message(app, handle, sid, message).await
}

#[tauri::command]
async fn attach_file(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
) -> CmdResult<Option<MessageView>> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;

    let Some(picked) = rfd::AsyncFileDialog::new()
        .set_title("Share a file")
        .pick_file()
        .await
    else {
        return Ok(None);
    };
    let path = picked.path().to_path_buf();
    let filename = picked.file_name();
    let bytes = tokio::fs::read(&path).await.map_err(err_to_string)?;
    send_attachment(&app, &handle, sid, cid, filename, bytes)
        .await
        .map(Some)
}

const MAX_PICKED_IMAGE_BYTES: u64 = 25 * 1024 * 1024;

#[tauri::command]
async fn pick_image(title: Option<String>) -> CmdResult<Option<String>> {
    let Some(picked) = rfd::AsyncFileDialog::new()
        .set_title(title.as_deref().unwrap_or("Choose an image"))
        .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp"])
        .pick_file()
        .await
    else {
        return Ok(None);
    };
    let path = picked.path().to_path_buf();
    let size = tokio::fs::metadata(&path).await.map_err(err_to_string)?.len();
    if size > MAX_PICKED_IMAGE_BYTES {
        return Err("that image is larger than 25 MB".into());
    }
    let bytes = tokio::fs::read(&path).await.map_err(err_to_string)?;
    let mime = match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "image/jpeg",
    };
    Ok(Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    )))
}

#[tauri::command]
async fn attach_bytes(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    filename: String,
    data: String,
) -> CmdResult<MessageView> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let filename = filename.trim().to_string();
    if filename.is_empty() || filename.len() > 255 {
        return Err("bad filename".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| "bad file data".to_string())?;
    if bytes.is_empty() {
        return Err("empty file".into());
    }
    send_attachment(&app, &handle, sid, cid, filename, bytes).await
}

#[tauri::command]
async fn note_user_activity(app: tauri::State<'_, App>, handle: tauri::AppHandle) -> CmdResult<()> {
    app.last_user_activity.store(
        chrono::Utc::now().timestamp(),
        std::sync::atomic::Ordering::Relaxed,
    );
    if app.auto_idle.load(std::sync::atomic::Ordering::Relaxed) {
        set_auto_presence(&app, &handle, Presence::Online, false).await;
    }
    Ok(())
}

#[tauri::command]
fn open_url(url: String) -> CmdResult<()> {
    let parsed = url.trim();
    if !(parsed.starts_with("https://") || parsed.starts_with("http://")) {
        return Err("only http and https links can be opened".into());
    }
    open::that_detached(parsed).map_err(err_to_string)
}

#[tauri::command]
async fn download_attachment(
    app: tauri::State<'_, App>,
    blob: String,
    filename: String,
    provider: Option<String>,
) -> CmdResult<Option<String>> {
    let bytes = fetch_blob(&app, &blob, provider.as_deref()).await?;
    let Some(target) = rfd::AsyncFileDialog::new()
        .set_file_name(&filename)
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let path = target.path().to_path_buf();
    tokio::fs::write(&path, bytes).await.map_err(err_to_string)?;
    Ok(Some(path.display().to_string()))
}

#[tauri::command]
async fn set_voice_channel(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: Option<String>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: Option<ChannelId> = match &channel_id {
        Some(c) => Some(c.parse().map_err(err_to_string)?),
        None => None,
    };
    let profile = current_profile(&app)?;
    let voice = app.voice.lock().unwrap().clone();

    match cid {
        Some(channel) => {
            let full = app.servers.lock().unwrap().get(&sid).is_some_and(|s| voice_channel_full(&app, s, channel, profile.id));
            if full {
                return Err("that voice channel is full".into());
            }
            let previous = app.voice_session.lock().unwrap().as_ref().map(|s| s.server).filter(|s| *s != sid);
            if let Some(previous) = previous {
                leave_voice(&app, &handle, previous, &profile, &voice).await;
            }
            start_voice_session(&app, &handle, sid, channel, &voice, profile.id)?;
            apply_personal_volumes(&app);
            refresh_forced_voice(&app, &handle);
            dial_voice_members(&app, &handle, sid, channel, &profile);
            set_own_voice_state(&app, &handle, sid, &profile, &voice, Some(channel)).await
        }
        None => {
            leave_voice(&app, &handle, sid, &profile, &voice).await;
            Ok(())
        }
    }
}

async fn leave_voice(
    app: &tauri::State<'_, App>,
    handle: &tauri::AppHandle,
    sid: ServerId,
    profile: &UserProfile,
    voice: &VoiceSettings,
) {
    *app.voice_session.lock().unwrap() = None;
    *app.camera.lock().unwrap() = None;
    app.camera_watches.lock().unwrap().clear();
    app.own_preview.lock().unwrap().take();
    let share = app.screen_share.lock().unwrap().take();
    if let Some(share) = share {
        broadcast(
            app,
            sid,
            WirePayload::Event(ServerEvent::ScreenShare {
                user: profile.id,
                channel: share.channel,
                active: false,
            }),
        )
        .await;
    }
    if let Err(e) = set_own_voice_state(app, handle, sid, profile, voice, None).await {
        tracing::warn!("leaving voice: {e}");
    }
}

async fn set_own_voice_state(
    app: &tauri::State<'_, App>,
    handle: &tauri::AppHandle,
    sid: ServerId,
    profile: &UserProfile,
    voice: &VoiceSettings,
    channel: Option<ChannelId>,
) -> CmdResult<()> {
    let event = ServerEvent::VoiceState {
        user: profile.id,
        channel,
        muted: voice.muted,
        deafened: voice.deafened,
        since: channel.map(|_| chrono::Utc::now().timestamp()),
    };
    {
        let mut servers = app.servers.lock().unwrap();
        let state = servers.get_mut(&sid).ok_or("unknown server")?;
        state.apply(event.clone()).map_err(err_to_string)?;
    }
    broadcast(app, sid, WirePayload::Event(event)).await;
    let _ = handle.emit("server-event", &sid.to_string());
    Ok(())
}

fn active_voice_channel(app: &tauri::State<'_, App>, _sid: ServerId) -> Option<ChannelId> {
    app.voice_session.lock().unwrap().as_ref().map(|s| s.channel)
}

fn start_voice_session(
    app: &tauri::State<'_, App>,
    handle: &tauri::AppHandle,
    sid: ServerId,
    channel: ChannelId,
    voice: &VoiceSettings,
    me: UserId,
) -> CmdResult<()> {
    let emitter = handle.clone();
    let on_speaking: media_engine::SpeakingSink = std::sync::Arc::new(move |user: UserId, speaking: bool| {
        let _ = emitter.emit(
            "voice-speaking",
            serde_json::json!({ "user": user.to_string(), "speaking": speaking }),
        );
    });
    let (mic_tx, mic_rx) = tokio::sync::mpsc::unbounded_channel();
    let mic = match commy_media::start_microphone(app.processor.clone(), mic_tx.clone(), voice.input_device.clone()) {
        Ok(handle) => Some(handle),
        Err(e) => {
            tracing::warn!("microphone unavailable, joining listen-only: {e}");
            None
        }
    };
    let session = VoiceSession::start(
        sid,
        channel,
        StreamGuards { mic, speaker: None },
        mic_rx,
        mic_tx,
        voice.deafened,
        me,
        on_speaking,
    );
    let speaker = match commy_media::start_playback(session.mixer.clone(), voice.output_device.clone()) {
        Ok(handle) => Some(handle),
        Err(e) => {
            tracing::warn!("speaker unavailable: {e}");
            None
        }
    };
    let mut slot = app.voice_session.lock().unwrap();
    let mut session = session;
    session.set_speaker(speaker);
    session.mixer.lock().unwrap().set_master(voice.output_volume);
    *slot = Some(session);
    Ok(())
}

async fn run_afk_watchdog(handle: tauri::AppHandle) {
    let env_override: Option<i64> = std::env::var("COMMY_AFK_SECS")
        .ok()
        .and_then(|v| v.parse().ok());
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let app = handle.state::<App>();
        let timeout_secs = env_override.unwrap_or_else(|| {
            app.voice.lock().unwrap().afk_timeout_minutes as i64 * 60
        });
        let voice = app.voice_session.lock().unwrap().as_ref().map(|s| {
            let last = s.last_activity.load(std::sync::atomic::Ordering::Relaxed);
            (s.server, s.channel, last)
        });
        let last_ui = app
            .last_user_activity
            .load(std::sync::atomic::Ordering::Relaxed);
        let last_voice = voice.map_or(0, |(_, _, last)| last);
        let sharing = app.screen_share.lock().unwrap().is_some();
        let idle_for = if sharing {
            0
        } else {
            chrono::Utc::now().timestamp() - last_voice.max(last_ui)
        };
        let Some(presence) = app.profile.lock().unwrap().as_ref().map(|p| p.presence) else {
            continue;
        };
        let auto_idle = app.auto_idle.load(std::sync::atomic::Ordering::Relaxed);
        match afk::decide(idle_for, timeout_secs, presence, auto_idle) {
            afk::AfkAction::GoIdle => {
                tracing::info!("idle for {idle_for}s, marking AFK");
                set_auto_presence(&app, &handle, Presence::Idle, true).await;
            }
            afk::AfkAction::Restore => {
                set_auto_presence(&app, &handle, Presence::Online, false).await;
            }
            afk::AfkAction::None => {}
        }

        let Some((sid, current, _)) = voice else { continue };
        let (afk, server_minutes) = {
            let servers = app.servers.lock().unwrap();
            servers
                .get(&sid)
                .map_or((None, None), |s| (s.server.afk_channel(), s.server.afk_timeout_minutes))
        };
        let voice_timeout = if env_override.is_some() {
            timeout_secs
        } else {
            afk::voice_timeout_secs(timeout_secs, server_minutes)
        };
        if voice_timeout <= 0 || idle_for < voice_timeout {
            continue;
        }
        let Some(afk) = afk else {
            if server_minutes.is_some() {
                tracing::info!("idle for {idle_for}s past the server limit, leaving voice");
                if set_voice_channel(handle.state::<App>(), handle.clone(), sid.to_string(), None).await.is_ok() {
                    let _ = handle.emit("afk-disconnected", ());
                }
            }
            continue;
        };
        if afk == current {
            continue;
        }
        tracing::info!("idle for {idle_for}s, moving to AFK channel");
        if set_voice_channel(
            handle.state::<App>(),
            handle.clone(),
            sid.to_string(),
            Some(afk.to_string()),
        )
        .await
        .is_ok()
        {
            let _ = handle.emit("afk-moved", ());
        }
    }
}

fn dial_voice_members(
    app: &tauri::State<'_, App>,
    handle: &tauri::AppHandle,
    sid: ServerId,
    channel: ChannelId,
    me: &UserProfile,
) {
    let Some(our_hex) = me.node.clone() else {
        tracing::warn!("no node identity yet; voice runs local only");
        return;
    };
    let targets: Vec<(UserId, String)> = {
        let servers = app.servers.lock().unwrap();
        let Some(state) = servers.get(&sid) else { return };
        state
            .voice_states
            .iter()
            .filter(|(user, ch)| **ch == channel && **user != me.id)
            .filter_map(|(user, _)| {
                state
                    .profiles
                    .get(user)
                    .and_then(|p| p.node.clone())
                    .map(|hex| (*user, hex))
            })
            .collect()
    };
    for (user, their_hex) in targets {
        if should_dial(&our_hex, &their_hex) {
            dial_voice_peer(handle, user, their_hex, channel, me.id);
        }
    }
}

fn dial_voice_peer(
    handle: &tauri::AppHandle,
    user: UserId,
    their_hex: String,
    channel: ChannelId,
    our_user: UserId,
) {
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        let app = handle.state::<App>();
        let hello = MediaHello::Voice {
            user: our_user,
            channel,
        };
        match dial_media(&app, &their_hex, hello).await {
            Ok(media) => {
                let mut session = app.voice_session.lock().unwrap();
                if let Some(session) = session.as_mut() {
                    if session.channel == channel {
                        session.attach_peer(user, media);
                    }
                }
            }
            Err(e) => tracing::warn!("voice dial failed: {e}"),
        }
    });
}

async fn fetch_blob(app: &App, blob: &str, provider: Option<&str>) -> CmdResult<Vec<u8>> {
    let fetcher = {
        let net = app.net.lock().await;
        net.as_ref().ok_or("network is not ready yet")?.blob_fetcher()
    };
    fetcher.fetch(blob, provider).await.map_err(err_to_string)
}

async fn dial_media(app: &App, their_hex: &str, hello: MediaHello) -> CmdResult<MediaChannel> {
    let dialer = {
        let net = app.net.lock().await;
        net.as_ref().ok_or("network is not ready yet")?.media_dialer()
    };
    dialer.connect(their_hex, hello).await.map_err(err_to_string)
}

#[derive(serde::Serialize)]
struct ShareSourceView {
    kind: String,
    id: u32,
    name: String,
    is_primary: bool,
    game: Option<String>,
}

#[tauri::command]
async fn list_share_sources(app: tauri::State<'_, App>) -> CmdResult<Vec<ShareSourceView>> {
    let sources = tokio::task::spawn_blocking(commy_media::screen::list_sources)
        .await
        .map_err(err_to_string)?
        .map_err(err_to_string)?;
    let game = app.last_game.lock().unwrap().clone();
    let mut views: Vec<ShareSourceView> = sources
        .into_iter()
        .map(|s| ShareSourceView {
            kind: match s.kind {
                SourceKind::Monitor => "monitor".to_string(),
                SourceKind::Window => "window".to_string(),
            },
            id: s.id,
            game: game.as_ref().filter(|(pid, _)| s.pid == Some(*pid)).map(|(_, name)| name.clone()),
            name: s.name,
            is_primary: s.is_primary,
        })
        .collect();
    if views.iter().any(|v| v.game.is_some()) {
        views.sort_by_key(|v| v.game.is_none());
    } else if game.is_some() {
        *app.last_game.lock().unwrap() = None;
    }
    Ok(views)
}

fn parse_source(kind: Option<&str>, id: Option<u32>) -> Option<(SourceKind, u32)> {
    match (kind, id) {
        (Some("monitor"), Some(id)) => Some((SourceKind::Monitor, id)),
        (Some("window"), Some(id)) => Some((SourceKind::Window, id)),
        _ => None,
    }
}

#[tauri::command]
async fn share_source_preview(source_kind: String, source_id: u32) -> CmdResult<String> {
    let (kind, id) =
        parse_source(Some(source_kind.as_str()), Some(source_id)).ok_or("unknown source kind")?;
    let frame = tokio::task::spawn_blocking(move || {
        commy_media::screen::capture_source(kind, id, 360, 55)
    })
    .await
    .map_err(err_to_string)?
    .map_err(err_to_string)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&frame.jpeg))
}

#[derive(serde::Serialize)]
struct OwnShareView {
    width: u32,
    height: u32,
    jpeg: Option<String>,
    viewers: usize,
    frames: u64,
}

#[tauri::command]
fn own_share_frame(app: tauri::State<App>) -> CmdResult<OwnShareView> {
    let share = app.screen_share.lock().unwrap();
    let session = share.as_ref().ok_or("not sharing")?;
    let frame = session.latest_frame();
    Ok(OwnShareView {
        width: frame.as_ref().map_or(0, |f| f.width),
        height: frame.as_ref().map_or(0, |f| f.height),
        jpeg: frame.map(|f| base64::engine::general_purpose::STANDARD.encode(&f.jpeg)),
        viewers: session.viewer_count(),
        frames: session.frames_encoded(),
    })
}

#[tauri::command]
async fn start_screen_share(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    source_kind: Option<String>,
    source_id: Option<u32>,
    quality: Option<String>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let profile = current_profile(&app)?;
    let channel = active_voice_channel(&app, sid).ok_or("join a voice channel first")?;

    let source = parse_source(source_kind.as_deref(), source_id);
    let quality = StreamQuality::from_name(quality.as_deref().unwrap_or(""));
    let session = ScreenShareSession::start(sid, channel, quality.fps(), quality.max_width(), source);
    let (preview_tx, preview_rx) = tokio::sync::mpsc::unbounded_channel();
    session.add_subscriber(preview_tx);
    let emitter = handle.clone();
    let preview = ScreenWatch::from_frames(profile.id, preview_rx, None, move |user, seq, width, height, jpeg| {
        let _ = emitter.emit("video-frame", video_frame_payload(user, Some(seq), width, height, &jpeg));
    });
    *app.own_preview.lock().unwrap() = Some(preview);
    *app.screen_share.lock().unwrap() = Some(session);

    let event = ServerEvent::ScreenShare {
        user: profile.id,
        channel,
        active: true,
    };
    {
        let mut servers = app.servers.lock().unwrap();
        if let Some(state) = servers.get_mut(&sid) {
            state.apply(event.clone()).map_err(err_to_string)?;
        }
    }
    broadcast(&app, sid, WirePayload::Event(event)).await;
    let _ = handle.emit("server-event", &sid.to_string());
    Ok(())
}

#[tauri::command]
async fn stop_screen_share(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let profile = current_profile(&app)?;
    app.own_preview.lock().unwrap().take();
    let Some(session) = app.screen_share.lock().unwrap().take() else {
        return Ok(());
    };
    let event = ServerEvent::ScreenShare {
        user: profile.id,
        channel: session.channel,
        active: false,
    };
    {
        let mut servers = app.servers.lock().unwrap();
        if let Some(state) = servers.get_mut(&sid) {
            let _ = state.apply(event.clone());
        }
    }
    broadcast(&app, sid, WirePayload::Event(event)).await;
    let _ = handle.emit("server-event", &sid.to_string());
    Ok(())
}

#[tauri::command]
async fn watch_screen(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    user_id: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let user: UserId = user_id.parse().map_err(err_to_string)?;
    let profile = current_profile(&app)?;

    let (their_hex, channel) = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        let channel = *state
            .screen_shares
            .get(&user)
            .ok_or("that user is not sharing")?;
        let hex = state
            .profiles
            .get(&user)
            .and_then(|p| p.node.clone())
            .ok_or("sharer has no reachable identity")?;
        (hex, channel)
    };

    let media = dial_media(
        &app,
        &their_hex,
        MediaHello::ScreenSubscribe {
            user: profile.id,
            channel,
        },
    )
    .await?;

    let emitter = handle.clone();
    let watch = ScreenWatch::start(user, media, move |user, seq, width, height, jpeg| {
        let _ = emitter.emit("video-frame", video_frame_payload(user, Some(seq), width, height, &jpeg));
    });
    app.watches.lock().unwrap().insert(user, watch);
    Ok(())
}

fn video_frame_payload(user: UserId, seq: Option<u64>, width: u32, height: u32, jpeg: &[u8]) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "user": user.to_string(),
        "width": width,
        "height": height,
        "jpeg": base64::engine::general_purpose::STANDARD.encode(jpeg),
    });
    if let Some(seq) = seq {
        payload["seq"] = serde_json::json!(seq);
    }
    payload
}

const MAX_CAMERA_FRAME_BYTES: usize = 400 * 1024;

#[tauri::command]
async fn start_camera(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let profile = current_profile(&app)?;
    let channel = active_voice_channel(&app, sid).ok_or("join a voice channel first")?;
    *app.camera.lock().unwrap() = Some(CameraSession::new(sid, channel));
    apply_and_broadcast(
        &app,
        &handle,
        sid,
        ServerEvent::CameraState { user: profile.id, channel, active: true },
    )
    .await?;
    Ok(())
}

#[tauri::command]
async fn stop_camera(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let profile = current_profile(&app)?;
    let Some(session) = app.camera.lock().unwrap().take() else {
        return Ok(());
    };
    apply_and_broadcast(
        &app,
        &handle,
        sid,
        ServerEvent::CameraState { user: profile.id, channel: session.channel, active: false },
    )
    .await?;
    Ok(())
}

#[tauri::command]
fn push_camera_frame(app: tauri::State<App>, jpeg: String, width: u32, height: u32) -> CmdResult<usize> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(jpeg)
        .map_err(|_| "camera frame is not valid base64".to_string())?;
    if bytes.len() > MAX_CAMERA_FRAME_BYTES || !bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Err("camera frame must be a JPEG under 400 KB".into());
    }
    let mut camera = app.camera.lock().unwrap();
    let session = camera.as_mut().ok_or("camera is off")?;
    Ok(session.push(width.min(4096), height.min(4096), bytes))
}

#[tauri::command]
async fn watch_camera(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    user_id: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let user: UserId = user_id.parse().map_err(err_to_string)?;
    let profile = current_profile(&app)?;
    if app.camera_watches.lock().unwrap().contains_key(&user) {
        return Ok(());
    }
    let (their_hex, channel) = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        let channel = *state.cameras.get(&user).ok_or("that camera is off")?;
        let hex = state
            .profiles
            .get(&user)
            .and_then(|p| p.node.clone())
            .ok_or("that member has no reachable identity")?;
        (hex, channel)
    };
    let media = dial_media(&app, &their_hex, MediaHello::CameraSubscribe { user: profile.id, channel }).await?;
    let emitter = handle.clone();
    let watch = ScreenWatch::start(user, media, move |user, _seq, width, height, jpeg| {
        let _ = emitter.emit("camera-frame", video_frame_payload(user, None, width, height, &jpeg));
    });
    app.camera_watches.lock().unwrap().insert(user, watch);
    Ok(())
}

#[tauri::command]
fn stop_watching_camera(app: tauri::State<App>, user_id: String) -> CmdResult<()> {
    let user: UserId = user_id.parse().map_err(err_to_string)?;
    app.camera_watches.lock().unwrap().remove(&user);
    Ok(())
}

#[tauri::command]
fn stop_watching(app: tauri::State<App>, user_id: String) -> CmdResult<()> {
    let user: UserId = user_id.parse().map_err(err_to_string)?;
    app.watches.lock().unwrap().remove(&user);
    Ok(())
}

pub(crate) fn node_hex_for_user(app: &App, user: UserId) -> Option<String> {
    if let Some(hex) = app
        .dms
        .lock()
        .unwrap()
        .contacts
        .get(&user)
        .and_then(|p| p.node.clone())
    {
        return Some(hex);
    }
    let servers = app.servers.lock().unwrap();
    servers
        .values()
        .find_map(|s| s.profiles.get(&user).and_then(|p| p.node.clone()))
}

#[tauri::command]
fn list_dm_threads(app: tauri::State<App>) -> Vec<DmThreadView> {
    let me = match app.profile.lock().unwrap().as_ref() {
        Some(p) => p.id,
        None => return Vec::new(),
    };
    let dms = app.dms.lock().unwrap();
    let safety = app.safety.lock().unwrap();
    let mut views = dm_thread_views(&dms, me);
    views.retain_mut(|view| {
        let Ok(partner) = view.user.parse::<UserId>() else { return true };
        if safety.is_blocked(partner) {
            return false;
        }
        if !safety.accepted.contains(&partner) && !dms.has_sent_to(me, partner) {
            view.kind = if safety.spam.contains(&partner) { "spam" } else { "request" }.to_string();
        }
        true
    });
    views
}

async fn send_friend_signal(app: &tauri::State<'_, App>, to: UserId, signal: safety::FriendSignal) -> CmdResult<()> {
    let _signing = app.signing.read().await;
    let me = current_profile(app)?;
    refuse_while_key_may_be_replaced(app)?;
    let hex = node_hex_for_user(app, to).ok_or("that person's device is unknown")?;
    let mut message = DmMessage::new(me.id, to, signal.encode());
    sign_dm(app, &mut message)?;
    let envelope = DmEnvelope { message, sender: me.as_seen_by_others() };
    let send = {
        let net = app.net.lock().await;
        net.as_ref().ok_or("network is not ready yet")?.send_dm(&hex, &envelope)
    };
    send.await.map_err(|_| "they are offline right now; try again later".to_string())
}

#[derive(serde::Serialize)]
struct FriendsView {
    friends: Vec<BlockedUserView>,
    incoming: Vec<BlockedUserView>,
    outgoing: Vec<BlockedUserView>,
}

#[tauri::command]
fn list_friends(app: tauri::State<'_, App>) -> FriendsView {
    let (friends, incoming, outgoing) = {
        let s = app.safety.lock().unwrap();
        (s.friends.clone(), s.friend_in.clone(), s.friend_out.clone())
    };
    let view = |set: std::collections::HashSet<UserId>| {
        let mut list: Vec<BlockedUserView> =
            set.into_iter().map(|id| BlockedUserView { id: id.to_string(), name: known_name(&app, id) }).collect();
        list.sort_by_key(|v| v.name.to_lowercase());
        list
    };
    FriendsView { friends: view(friends), incoming: view(incoming), outgoing: view(outgoing) }
}

#[tauri::command]
async fn friend_action(app: tauri::State<'_, App>, handle: tauri::AppHandle, user_id: String, action: String) -> CmdResult<()> {
    let user: UserId = user_id.parse().map_err(err_to_string)?;
    if Some(user) == app.profile.lock().unwrap().as_ref().map(|p| p.id) {
        return Err("you cannot friend yourself".into());
    }
    let signal = match action.as_str() {
        "request" => safety::FriendSignal::Request,
        "accept" => safety::FriendSignal::Accept,
        "decline" => safety::FriendSignal::Decline,
        "remove" => safety::FriendSignal::Remove,
        _ => return Err("unknown friend action".into()),
    };
    if signal == safety::FriendSignal::Request && app.safety.lock().unwrap().is_blocked(user) {
        return Err("unblock them first".into());
    }
    send_friend_signal(&app, user, signal).await?;
    {
        let mut s = app.safety.lock().unwrap();
        match signal {
            safety::FriendSignal::Request => {
                if s.friend_in.remove(&user) {
                    s.friends.insert(user);
                } else {
                    s.friend_out.insert(user);
                }
            }
            safety::FriendSignal::Accept => {
                if s.friend_in.remove(&user) {
                    s.friends.insert(user);
                }
            }
            safety::FriendSignal::Decline => {
                s.friend_in.remove(&user);
            }
            safety::FriendSignal::Remove => {
                s.friends.remove(&user);
                s.friend_out.remove(&user);
            }
        }
    }
    app.persist_safety();
    let _ = handle.emit("friends-changed", serde_json::json!({ "user": user.to_string() }));
    Ok(())
}

#[derive(serde::Serialize)]
struct BlockedUserView {
    id: String,
    name: String,
}

#[derive(serde::Serialize)]
struct SafetyView {
    blocked: Vec<BlockedUserView>,
    dm_policy: safety::DmPolicy,
    spam_filter: bool,
}

fn known_name(app: &tauri::State<'_, App>, user: UserId) -> String {
    if let Some(p) = app.dms.lock().unwrap().contacts.get(&user) {
        return p.visible_name().to_string();
    }
    app.servers
        .lock()
        .unwrap()
        .values()
        .find_map(|s| s.profiles.get(&user).map(|p| p.visible_name().to_string()))
        .unwrap_or_else(|| "Unknown User".to_string())
}

#[tauri::command]
fn get_safety(app: tauri::State<'_, App>) -> SafetyView {
    let (blocked, dm_policy, spam_filter) = {
        let s = app.safety.lock().unwrap();
        (s.blocked.clone(), s.dm_policy, s.spam_filter)
    };
    SafetyView {
        blocked: blocked
            .into_iter()
            .map(|id| BlockedUserView { id: id.to_string(), name: known_name(&app, id) })
            .collect(),
        dm_policy,
        spam_filter,
    }
}

#[tauri::command]
fn set_blocked(app: tauri::State<'_, App>, handle: tauri::AppHandle, user_id: String, blocked: bool) -> CmdResult<()> {
    let user: UserId = user_id.parse().map_err(err_to_string)?;
    if Some(user) == app.profile.lock().unwrap().as_ref().map(|p| p.id) {
        return Err("you cannot block yourself".into());
    }
    if app.safety.lock().unwrap().set_blocked(user, blocked) {
        app.persist_safety();
        let _ = handle.emit("safety-changed", ());
    }
    Ok(())
}

#[tauri::command]
fn set_dm_settings(app: tauri::State<'_, App>, policy: String, spam_filter: bool) -> CmdResult<()> {
    let policy = safety::DmPolicy::parse(&policy).ok_or("unknown DM setting")?;
    {
        let mut s = app.safety.lock().unwrap();
        s.dm_policy = policy;
        s.spam_filter = spam_filter;
    }
    app.persist_safety();
    Ok(())
}

#[tauri::command]
fn accept_dm_request(app: tauri::State<'_, App>, handle: tauri::AppHandle, user_id: String) -> CmdResult<()> {
    let user: UserId = user_id.parse().map_err(err_to_string)?;
    {
        let mut s = app.safety.lock().unwrap();
        s.accepted.insert(user);
        s.spam.remove(&user);
    }
    app.persist_safety();
    let _ = handle.emit("dm-changed", user.to_string());
    Ok(())
}

#[tauri::command]
fn decline_dm_request(app: tauri::State<'_, App>, handle: tauri::AppHandle, user_id: String) -> CmdResult<()> {
    let user: UserId = user_id.parse().map_err(err_to_string)?;
    app.dms.lock().unwrap().remove_conversation(user);
    app.safety.lock().unwrap().spam.remove(&user);
    app.persist_dms();
    app.persist_safety();
    let _ = handle.emit("dm-changed", user.to_string());
    Ok(())
}

#[tauri::command]
async fn report_user(
    app: tauri::State<'_, App>,
    server_id: String,
    user_id: String,
    channel_id: Option<String>,
    message_id: Option<String>,
    reason: String,
    note: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let reported: UserId = user_id.parse().map_err(err_to_string)?;
    if !safety::REPORT_REASONS.contains(&reason.as_str()) {
        return Err("pick a reason for the report".into());
    }
    let _signing = app.signing.read().await;
    let me = current_profile(&app)?;
    if reported == me.id {
        return Err("you cannot report yourself".into());
    }
    refuse_while_key_may_be_replaced(&app)?;
    let (owner, excerpt) = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        let excerpt = match (&channel_id, &message_id) {
            (Some(c), Some(m)) => {
                let cid: ChannelId = c.parse().map_err(err_to_string)?;
                let mid: MessageId = m.parse().map_err(err_to_string)?;
                state
                    .messages(cid)
                    .iter()
                    .find(|msg| msg.id == mid && msg.author == reported)
                    .map(|msg| msg.content.clone())
                    .ok_or("that message is no longer available")?
            }
            _ => String::new(),
        };
        (state.server.owner, excerpt)
    };
    if owner == me.id {
        return Err("you own this server, so reports would come to you".into());
    }
    if reported == owner {
        return Err("reports go to the server owner, so the owner cannot be reported here".into());
    }
    let report = safety::Report {
        id: MessageId::new().to_string(),
        server: sid.to_string(),
        channel: channel_id,
        message: message_id,
        reporter: me.id.to_string(),
        reporter_name: me.visible_name().to_string(),
        reported: reported.to_string(),
        reported_name: known_name(&app, reported),
        reason,
        note: safety::truncate(note.trim(), 500),
        excerpt: safety::truncate(&excerpt, 300),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let owner_hex = node_hex_for_user(&app, owner)
        .ok_or("the server owner's device is unknown; try again once they have been online")?;
    let mut message = DmMessage::new(me.id, owner, report.encode());
    sign_dm(&app, &mut message)?;
    let envelope = DmEnvelope { message, sender: me.as_seen_by_others() };
    let send = {
        let net = app.net.lock().await;
        net.as_ref().ok_or("network is not ready yet")?.send_dm(&owner_hex, &envelope)
    };
    send.await
        .map_err(|_| "the server owner is offline right now; try again later".to_string())?;
    Ok(())
}

fn receive_report(app: &tauri::State<'_, App>, handle: &tauri::AppHandle, sender: UserId, content: &str) {
    let Some(report) = safety::Report::decode(content, sender) else { return };
    let Some(me) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { return };
    let valid = report.server.parse::<ServerId>().ok().is_some_and(|sid| {
        app.servers.lock().unwrap().get(&sid).is_some_and(|s| {
            s.server.owner == me && s.server.members.iter().any(|m| m.user == sender)
        })
    });
    if !valid || app.safety.lock().unwrap().is_blocked(sender) {
        return;
    }
    let notice = serde_json::json!({
        "server": report.server,
        "reported_name": report.reported_name,
        "reporter_name": report.reporter_name,
        "reason": report.reason,
    });
    if app.safety.lock().unwrap().add_report(report) {
        app.persist_safety();
        let _ = handle.emit("report-received", notice);
    }
}

#[tauri::command]
fn list_reports(app: tauri::State<'_, App>, server_id: String) -> CmdResult<Vec<safety::Report>> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let owner = app.servers.lock().unwrap().get(&sid).map(|s| s.server.owner).ok_or("unknown server")?;
    if owner != me.id {
        return Err("only the server owner receives reports".into());
    }
    Ok(app
        .safety
        .lock()
        .unwrap()
        .reports
        .iter()
        .filter(|r| r.server == server_id)
        .cloned()
        .collect())
}

#[tauri::command]
fn dismiss_report(app: tauri::State<'_, App>, report_id: String) {
    app.safety.lock().unwrap().reports.retain(|r| r.id != report_id);
    app.persist_safety();
}

#[tauri::command]
fn get_dm_messages(app: tauri::State<App>, user_id: String) -> CmdResult<Vec<DmMessageView>> {
    let partner: UserId = user_id.parse().map_err(err_to_string)?;
    let me = app
        .profile
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.id)
        .ok_or("set a username first")?;
    let pending = app.outbox.lock().unwrap().pending_dms(partner);
    let dms = app.dms.lock().unwrap();
    Ok(dms
        .conversation(partner)
        .iter()
        .map(|m| dm_message_view(m, me, &dms, pending.contains(&m.id)))
        .collect())
}

#[tauri::command]
fn open_dm(app: tauri::State<App>, user_id: String) -> CmdResult<DmThreadView> {
    let partner: UserId = user_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    if partner == me.id {
        return Err("you cannot message yourself".into());
    }
    let profile = outbox::member_profile(&app.servers.lock().unwrap(), partner);
    let mut dms = app.dms.lock().unwrap();
    if let Some(profile) = profile {
        dms.remember_contact(profile);
    }
    let name = dms
        .contacts
        .get(&partner)
        .map(|p| p.visible_name().to_string())
        .ok_or("unknown user")?;
    drop(dms);
    app.persist_dms();
    Ok(DmThreadView {
        user: partner.to_string(),
        name,
        last_message: String::new(),
        last_at: String::new(),
        kind: "inbox".to_string(),
    })
}

#[tauri::command]
async fn send_dm(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    user_id: String,
    content: String,
) -> CmdResult<DmMessageView> {
    let content = content.trim().to_string();
    if content.is_empty() || content.len() > 4000 {
        return Err("message must be 1 to 4000 characters".into());
    }
    let partner: UserId = user_id.parse().map_err(err_to_string)?;
    let _signing = app.signing.read().await;
    let me = current_profile(&app)?;
    if partner == me.id {
        return Err("you cannot message yourself".into());
    }
    if app.safety.lock().unwrap().is_blocked(partner) {
        return Err("you blocked this person; unblock them to send a message".into());
    }

    let mut message = DmMessage::new(me.id, partner, content);
    sign_dm(&app, &mut message)?;
    let envelope = DmEnvelope {
        message: message.clone(),
        sender: me.as_seen_by_others(),
    };
    let waiting = app.outbox.lock().unwrap().has_dms(partner);
    let delivered = !waiting && outbox::try_send_dm(&app, partner, &envelope).await;

    let view = {
        let mut dms = app.dms.lock().unwrap();
        dms.insert(partner, message.clone());
        dm_message_view(&message, me.id, &dms, !delivered)
    };
    if !delivered {
        outbox::queue_dm(&app, partner, message.id);
        if waiting {
            outbox::wake_dm(&app, partner);
        }
    }
    if app.safety.lock().unwrap().accepted.insert(partner) {
        app.persist_safety();
    }
    app.persist_dms();
    let _ = handle.emit("dm-changed", partner.to_string());
    Ok(view)
}

fn our_permissions(app: &App, state: &ServerState, me: UserId) -> Permissions {
    let perms = state.server.permissions_for(me);
    if app.awaits_snapshot(state.server.id) {
        perms & Permissions::default_member()
    } else {
        perms
    }
}

pub(crate) fn require_invite_permission(app: &App, sid: ServerId) -> CmdResult<()> {
    let me = current_profile(app)?;
    let servers = app.servers.lock().unwrap();
    let state = servers.get(&sid).ok_or("unknown server")?;
    require_permission(app, state, me.id, Permissions::CREATE_INVITES, "invite people")
}

fn require_permission(
    app: &App,
    state: &ServerState,
    me: UserId,
    needed: Permissions,
    action: &str,
) -> CmdResult<()> {
    if our_permissions(app, state, me).allows(needed) {
        Ok(())
    } else {
        Err(format!("you do not have permission to {action}"))
    }
}

async fn apply_and_broadcast(
    app: &tauri::State<'_, App>,
    handle: &tauri::AppHandle,
    sid: ServerId,
    event: ServerEvent,
) -> CmdResult<()> {
    {
        let mut servers = app.servers.lock().unwrap();
        let state = servers.get_mut(&sid).ok_or("unknown server")?;
        state.apply(event.clone()).map_err(err_to_string)?;
    }
    outbox::broadcast_local(app, sid, event).await;
    app.persist_server(sid);
    let _ = handle.emit("server-event", &sid.to_string());
    Ok(())
}

#[tauri::command]
async fn rename_server(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    name: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let name = name.trim().to_string();
    if name.is_empty() || name.len() > 100 {
        return Err("server name must be 1 to 100 characters".into());
    }
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_SERVER, "rename the server")?;
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::ServerRename { server: sid, name })
        .await?;
    let _ = handle.emit("servers-changed", ());
    Ok(())
}

fn structure_announcements(app: &App, sid: ServerId, me: UserId) -> Vec<ServerEvent> {
    let servers = app.servers.lock().unwrap();
    let Some(state) = servers.get(&sid) else { return Vec::new() };
    let mut events = Vec::new();
    if our_permissions(app, state, me).allows(Permissions::MANAGE_CHANNELS) {
        for channel in &state.server.channels {
            events.push(ServerEvent::ChannelCreate { server: sid, channel: channel.clone() });
            events.push(ServerEvent::ChannelUpdate {
                server: sid,
                channel: channel.id,
                name: channel.name.clone(),
                retention_days: channel.retention_days,
            });
        }
        for &channel in &state.server.deleted_channels {
            events.push(ServerEvent::ChannelDelete { server: sid, channel });
        }
    }
    if (state.server.icon.is_some() || state.server.banner.is_some())
        && our_permissions(app, state, me).allows(Permissions::MANAGE_SERVER)
    {
        events.push(ServerEvent::ServerImages {
            server: sid,
            icon: state.server.icon.clone(),
            banner: state.server.banner.clone(),
        });
    }
    for emoji in &state.server.emojis {
        events.push(ServerEvent::EmojiAdd { server: sid, emoji: emoji.clone() });
    }
    for sound in &state.server.soundboard {
        events.push(ServerEvent::SoundAdd { server: sid, sound: sound.clone() });
    }
    let perms = our_permissions(app, state, me);
    if perms.allows(Permissions::MANAGE_SERVER) {
        if !state.server.game_tags.is_empty() {
            events.push(ServerEvent::GameTagsSet {
                server: sid,
                tags: state.server.game_tags.clone(),
                meta: state.server.game_tag_meta.clone(),
            });
        }
        if state.server.rules.is_some() {
            events.push(ServerEvent::RulesSet { server: sid, rules: state.server.rules.clone() });
        }
        if state.server.afk_timeout_minutes.is_some() {
            events.push(ServerEvent::AfkTimeoutSet { server: sid, minutes: state.server.afk_timeout_minutes });
        }
    }
    if state.server.rules_accepted.contains(&me) {
        events.push(ServerEvent::RulesAccept { server: sid, user: me });
    }
    for event in &state.server.events {
        events.push(ServerEvent::EventUpsert { server: sid, event: event.clone() });
        for (user, rsvp) in &event.rsvps {
            events.push(ServerEvent::EventRsvp { server: sid, event: event.id.clone(), user: *user, rsvp: Some(*rsvp) });
        }
    }
    for id in &state.server.deleted_events {
        events.push(ServerEvent::EventDelete { server: sid, event: id.clone() });
    }
    if perms.allows(Permissions::MANAGE_CHANNELS) {
        for channel in &state.server.channels {
            events.push(ServerEvent::ChannelLayout {
                server: sid,
                channel: channel.id,
                parent: channel.parent,
                position: channel.position,
            });
            if channel.user_limit > 0 {
                events.push(ServerEvent::ChannelLimit { server: sid, channel: channel.id, user_limit: channel.user_limit });
            }
            if channel.banner.is_some() {
                events.push(ServerEvent::ChannelBanner { server: sid, channel: channel.id, banner: channel.banner.clone() });
            }
            if channel.color.is_some() {
                events.push(ServerEvent::ChannelColor { server: sid, channel: channel.id, color: channel.color.clone() });
            }
            if channel.kind == ChannelKind::Announcement {
                events.push(ServerEvent::ChannelKindSet { server: sid, channel: channel.id, kind: channel.kind });
            }
        }
    }
    if perms.can_pin() {
        for pin in &state.server.pins {
            events.push(ServerEvent::MessagePin {
                server: sid,
                channel: pin.channel,
                message: pin.message,
                by: pin.by,
                at: pin.at,
                pinned: true,
            });
        }
    }
    if perms.allows(Permissions::MUTE_MEMBERS) {
        let moderated = state.server.server_muted.iter().chain(&state.server.server_deafened);
        let mut seen = Vec::new();
        for user in moderated {
            if seen.contains(user) {
                continue;
            }
            seen.push(*user);
            events.push(ServerEvent::VoiceModerate {
                server: sid,
                user: *user,
                muted: state.server.server_muted.contains(user),
                deafened: state.server.server_deafened.contains(user),
            });
        }
    }
    if perms.allows(Permissions::BAN_MEMBERS) {
        for user in &state.server.banned {
            events.push(ServerEvent::MemberBan { server: sid, user: *user });
        }
    }
    if perms.allows(Permissions::MANAGE_ROLES) {
        for role in state.server.roles.iter().filter(|r| r.self_assignable || r.tag.is_some() || r.banner.is_some()) {
            events.push(ServerEvent::RoleUpdate { server: sid, role: role.clone() });
        }
    }
    let custom = state.server.emojis_shareable || state.server.soundboard_open;
    if custom && our_permissions(app, state, me).allows(Permissions::MANAGE_SERVER) {
        events.push(ServerEvent::ExpressionSettings {
            server: sid,
            emojis_shareable: state.server.emojis_shareable,
            soundboard_open: state.server.soundboard_open,
        });
    }
    if state.server.public && our_permissions(app, state, me).allows(Permissions::MANAGE_SERVER) {
        events.push(ServerEvent::PublicSet { server: sid, public: true });
    }
    let explicit = state.server.afk_disabled || state.server.afk_channel_id.is_some();
    if explicit && our_permissions(app, state, me).allows(Permissions::MANAGE_SERVER) {
        events.push(ServerEvent::AfkChannelSet {
            server: sid,
            channel: state.server.afk_channel(),
        });
    }
    events
}

const MAX_RETENTION_DAYS: u32 = 365;

#[tauri::command]
async fn update_channel(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    name: String,
    retention_days: Option<u32>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let kind = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_CHANNELS, "edit channels")?;
        state.server.channel(cid).ok_or("unknown channel")?.kind
    };
    let name = match kind {
        ChannelKind::Text | ChannelKind::Announcement => name.trim().to_lowercase().replace(' ', "-"),
        _ => name.trim().to_string(),
    };
    if name.is_empty() || name.len() > 50 {
        return Err("channel name must be 1 to 50 characters".into());
    }
    let retention_days = retention_days.filter(|d| *d > 0).map(|d| d.min(MAX_RETENTION_DAYS));
    apply_and_broadcast(
        &app,
        &handle,
        sid,
        ServerEvent::ChannelUpdate { server: sid, channel: cid, name, retention_days },
    )
    .await?;
    Ok(())
}

const MESH_HEAL_EVERY: std::time::Duration = std::time::Duration::from_secs(15);

async fn run_mesh_healer(handle: tauri::AppHandle) {
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    loop {
        let app = handle.state::<App>();
        if wake::is_resting(&app) {
            tokio::time::sleep(MESH_HEAL_EVERY).await;
            continue;
        }
        let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
        let targets: Vec<(ServerId, Vec<String>)> = {
            let servers = app.servers.lock().unwrap();
            servers
                .values()
                .map(|s| {
                    let peers = s
                        .profiles
                        .values()
                        .filter(|p| Some(p.id) != me)
                        .filter_map(|p| p.node.clone())
                        .collect();
                    (s.server.id, peers)
                })
                .collect()
        };
        {
            let net = app.net.lock().await;
            if let Some(node) = net.as_ref() {
                for (sid, peers) in targets {
                    if peers.is_empty() || node.neighbor_count(sid) > 0 {
                        continue;
                    }
                    let dial = node.join_peers(sid, &peers);
                    match tokio::time::timeout(std::time::Duration::from_secs(2), dial).await {
                        Ok(Ok(n)) if n > 0 => tracing::info!("server {sid} has no neighbors, re-dialing {n} members"),
                        Ok(Ok(_)) => {}
                        Ok(Err(e)) => tracing::debug!("re-dial for {sid} failed: {e}"),
                        Err(_) => tracing::debug!("re-dial for {sid} is still in progress"),
                    }
                }
            }
        }
        tokio::time::sleep(MESH_HEAL_EVERY).await;
    }
}

const RETENTION_PRUNE_EVERY: std::time::Duration = std::time::Duration::from_secs(600);

async fn run_retention_pruner(handle: tauri::AppHandle) {
    loop {
        let app = handle.state::<App>();
        let pruned: Vec<ServerId> = {
            let mut servers = app.servers.lock().unwrap();
            let now = chrono::Utc::now();
            servers
                .values_mut()
                .filter_map(|s| (s.prune_expired(now) > 0).then_some(s.server.id))
                .collect()
        };
        for sid in pruned {
            app.persist_server(sid);
            let _ = handle.emit("server-event", &sid.to_string());
        }
        tokio::time::sleep(RETENTION_PRUNE_EVERY).await;
    }
}

#[tauri::command]
async fn set_afk_channel(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: Option<String>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let channel: Option<ChannelId> = match channel_id.as_deref() {
        Some(c) if !c.is_empty() => Some(c.parse().map_err(err_to_string)?),
        _ => None,
    };
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_SERVER, "change the AFK channel")?;
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::AfkChannelSet { server: sid, channel })
        .await?;
    Ok(())
}

#[tauri::command]
async fn create_channel(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    name: String,
    kind: String,
    parent_id: Option<String>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let name = name.trim().to_lowercase().replace(' ', "-");
    if name.is_empty() || name.len() > 50 {
        return Err("channel name must be 1 to 50 characters".into());
    }
    let kind = match kind.as_str() {
        "text" => ChannelKind::Text,
        "voice" => ChannelKind::Voice,
        "announcement" => ChannelKind::Announcement,
        _ => return Err("channel kind must be text, voice or announcement".into()),
    };
    let me = current_profile(&app)?;
    let channel = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_CHANNELS, "create channels")?;
        if state.server.channels.iter().any(|c| c.name == name && c.kind == kind) {
            return Err("a channel with that name already exists".into());
        }
        let position = state.server.channels.len() as u32;
        let mut channel = Channel::new(name, kind, position);
        channel.parent = parent_id
            .and_then(|p| p.parse::<ChannelId>().ok())
            .filter(|p| state.server.channel(*p).is_some_and(|c| c.kind == ChannelKind::Category));
        channel
    };
    apply_and_broadcast(
        &app,
        &handle,
        sid,
        ServerEvent::ChannelCreate { server: sid, channel },
    )
    .await
}

#[tauri::command]
async fn delete_channel(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_CHANNELS, "delete channels")?;
        let remaining_text = state
            .server
            .channels
            .iter()
            .filter(|c| c.kind == ChannelKind::Text && c.id != cid)
            .count();
        if remaining_text == 0 {
            return Err("a server needs at least one text channel".into());
        }
    }
    apply_and_broadcast(
        &app,
        &handle,
        sid,
        ServerEvent::ChannelDelete { server: sid, channel: cid },
    )
    .await
}

#[tauri::command]
async fn kick_member(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    user_id: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let target: UserId = user_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::KICK_MEMBERS, "kick members")?;
        if target == state.server.owner {
            return Err("the owner cannot be kicked".into());
        }
    }
    apply_and_broadcast(
        &app,
        &handle,
        sid,
        ServerEvent::MemberLeave { server: sid, user: target },
    )
    .await
}

#[tauri::command]
async fn leave_server(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;

    broadcast(
        &app,
        sid,
        WirePayload::Event(ServerEvent::MemberLeave { server: sid, user: me.id }),
    )
    .await;
    {
        let mut net = app.net.lock().await;
        if let Some(node) = net.as_mut() {
            node.leave_server(sid);
        }
    }
    app.servers.lock().unwrap().remove(&sid);
    app.invites.lock().unwrap().remove(&sid);
    forget_server_outbox(&app, sid);
    if let Some(dir) = app.data_dir() {
        let _ = persist::delete_server(dir, sid);
    }
    entry_list::record_leave(&app, sid);
    entry_list::schedule(&handle);
    let _ = handle.emit("servers-changed", ());
    Ok(())
}

fn forget_server_outbox(app: &App, sid: ServerId) {
    app.catchup.lock().unwrap().forget_server(sid);
    app.set_awaiting_snapshot(sid, false);
    if app.outbox.lock().unwrap().forget_server(sid) {
        app.persist_outbox();
    }
}

#[tauri::command]
async fn update_profile(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    display_name: Option<String>,
    custom_status: Option<String>,
    presence: String,
    status_emoji: Option<String>,
) -> CmdResult<UserProfile> {
    let presence: Presence = serde_json::from_value(serde_json::json!(presence))
        .map_err(|_| "invalid presence".to_string())?;
    let status_emoji = match status_emoji.map(|e| e.trim().to_string()) {
        None => None,
        Some(e) if e.is_empty() => Some(None),
        Some(e) if commy_core::user::is_emoji_like(&e) => Some(Some(e)),
        Some(_) => return Err("status emoji must be a single emoji".into()),
    };
    let clean = |s: Option<String>| {
        s.map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s.len() <= 64)
    };
    let profile = {
        let mut guard = app.profile.lock().unwrap();
        let profile = guard.as_mut().ok_or("set a username first")?;
        profile.display_name = clean(display_name);
        profile.custom_status = clean(custom_status);
        if presence != profile.presence {
            profile.presence_until = None;
        }
        profile.presence = presence;
        if let Some(emoji) = status_emoji {
            profile.status_emoji = emoji;
        }
        profile.clone()
    };
    app.auto_idle.store(false, std::sync::atomic::Ordering::Relaxed);
    app.persist_profile();
    announce_profile(&app, &handle, &profile).await;
    let _ = handle.emit("profile-changed", &profile);
    entry_list::record_profile_edit(&app);
    entry_list::schedule(&handle);
    Ok(profile)
}

#[tauri::command]
async fn set_status_expiry(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    presence_until: Option<i64>,
    status_until: Option<i64>,
) -> CmdResult<VoiceSettings> {
    let now = chrono::Utc::now().timestamp();
    let future = |t: Option<i64>| t.filter(|t| *t > now);
    let settings = {
        let mut voice = app.voice.lock().unwrap();
        voice.presence_until = future(presence_until);
        voice.status_until = future(status_until);
        voice.clone()
    };
    app.persist_voice();
    let profile = {
        let mut guard = app.profile.lock().unwrap();
        match guard.as_mut() {
            Some(profile) if profile.presence_until != settings.presence_until => {
                profile.presence_until = settings.presence_until;
                Some(profile.clone())
            }
            _ => None,
        }
    };
    if let Some(profile) = profile {
        app.persist_profile();
        announce_profile(&app, &handle, &profile).await;
        let _ = handle.emit("profile-changed", &profile);
    }
    Ok(settings)
}

fn expired_status(voice: &VoiceSettings, now: i64) -> (bool, bool) {
    (
        voice.presence_until.is_some_and(|t| t <= now),
        voice.status_until.is_some_and(|t| t <= now),
    )
}

async fn run_status_expiry(handle: tauri::AppHandle) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        let app = handle.state::<App>();
        let now = chrono::Utc::now().timestamp();
        let (presence, status) = expired_status(&app.voice.lock().unwrap(), now);
        if !presence && !status {
            continue;
        }
        {
            let mut voice = app.voice.lock().unwrap();
            if presence {
                voice.presence_until = None;
            }
            if status {
                voice.status_until = None;
            }
        }
        app.persist_voice();
        let profile = {
            let mut guard = app.profile.lock().unwrap();
            let Some(profile) = guard.as_mut() else { continue };
            if presence {
                profile.presence = Presence::Online;
                profile.presence_until = None;
            }
            if status {
                profile.custom_status = None;
                profile.status_emoji = None;
            }
            profile.clone()
        };
        app.persist_profile();
        announce_profile(&app, &handle, &profile).await;
        let _ = handle.emit("profile-changed", &profile);
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;

    #[test]
    fn an_edit_is_dated_after_the_last_one_so_peers_take_it() {
        let identity = Identity::generate();
        let mut me = UserProfile::new("me");
        me.identity = Some(identity.public());
        let server = commy_core::Server::new("s", me.id);
        let (sid, channel) = (server.id, server.channels[0].id);
        let mut state = ServerState::new(server);
        state.profiles.insert(me.id, me.clone());
        let mut message = Message::new(channel, me.id, "first");
        message.sign(sid, &identity);
        state.apply(ServerEvent::MessageCreate { message: message.clone() }).unwrap();
        let now = chrono::Utc::now();
        assert_eq!(edit_time_after_last_edit(&message, now), now, "never edited");

        let ahead = now + chrono::Duration::seconds(30);
        state.apply(message.edit_event(sid, "from the other device", ahead, &identity)).unwrap();
        let stored = state.message(message.id).unwrap().clone();
        let at = edit_time_after_last_edit(&stored, now);
        assert!(at > ahead);
        assert_eq!(state.apply(stored.edit_event(sid, "from here", at, &identity)), Ok(true));
        assert_eq!(state.message(message.id).unwrap().content, "from here");

        let last = state.message(message.id).unwrap().clone();
        let same = last.edited_at.unwrap();
        let again = last.edit_event(sid, "again", edit_time_after_last_edit(&last, same), &identity);
        assert_eq!(state.apply(again), Ok(true));
        assert_eq!(state.message(message.id).unwrap().content, "again");

        let later = same + chrono::Duration::seconds(1);
        assert_eq!(edit_time_after_last_edit(&last, later), later);
    }

    #[test]
    fn an_edit_is_refused_by_us_exactly_where_the_store_refuses_it() {
        let identity = Identity::generate();
        let mut me = UserProfile::new("me");
        me.identity = Some(identity.public());
        let server = commy_core::Server::new("s", me.id);
        let (sid, channel) = (server.id, server.channels[0].id);
        let mut state = ServerState::new(server);
        state.profiles.insert(me.id, me.clone());
        let mut message = Message::new(channel, me.id, "short");
        message.sign(sid, &identity);
        state.apply(ServerEvent::MessageCreate { message: message.clone() }).unwrap();

        let now = chrono::Utc::now();
        let edit = |len: usize| sign_edit(sid, &message, "x".repeat(len), now, &identity);
        let longest = (0..4000).collect::<Vec<_>>().partition_point(|&len| edit(len).is_ok()) - 1;
        let fits = edit(longest).unwrap();
        let gossip = |event: &ServerEvent| commy_net::protocol::gossip_size(&WirePayload::Event(event.clone()));
        assert!(gossip(&fits) <= commy_net::protocol::MAX_GOSSIP_BYTES);
        assert_eq!(state.clone().apply(fits), Ok(true));

        assert_eq!(edit(longest + 1), Err("that edit is too long; try shortening it".to_string()));
        let over = message.edit_event(sid, "x".repeat(longest + 1), now, &identity);
        assert!(gossip(&over) <= commy_net::protocol::MAX_GOSSIP_BYTES);
        assert_eq!(state.apply(over), Err(StoreError::TooLarge));
    }
}

#[cfg(test)]
mod own_server_tests {
    use super::*;

    #[tokio::test]
    async fn our_entry_and_announcement_take_a_key_settled_while_connecting() {
        let nanos = std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos();
        let dir = std::env::temp_dir().join("commy-own-server-tests").join(format!("{nanos:x}"));
        std::fs::create_dir_all(&dir).unwrap();
        let write_key = |identity: &Identity| {
            let hex = data_encoding::HEXLOWER.encode(&identity.to_bytes());
            std::fs::write(dir.join(commy_core::identity::IDENTITY_FILE), hex).unwrap();
        };
        let held = Identity::generate();
        write_key(&held);
        let app = App::default();
        app.data_dir.set(dir.clone()).unwrap();
        *app.profile.lock().unwrap() = Some(UserProfile::new("me"));
        app.reload_identity().unwrap();
        app.mark_key_unsettled();
        app.hold_key(std::time::Instant::now() + KEY_SETTLE_MAX);
        assert_eq!(app.announced(current_profile(&app).unwrap()).identity, None);

        let account = Identity::generate();
        let joining = async {
            write_key(&account);
            let swap = app.signing.write().await;
            settle::settle_loaded_key(&app, &swap).unwrap();
            Ok(())
        };
        let sid = ServerId::new();
        let (state, profile, signing) = own_server_state(&app, sid, "s", joining).await.unwrap();
        assert_eq!(state.server.id, sid);
        assert_eq!(state.identity_of(profile.id), Some(account.public()));
        assert_eq!(app.announced(profile).identity, Some(account.public()));
        assert!(app.signing.try_write().is_err());
        drop(signing);
        assert!(app.signing.try_write().is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod placeholder_tests {
    use super::*;

    #[test]
    fn a_placeholder_naming_us_owner_grants_nothing_over_others() {
        let member = |name: &str| {
            let identity = Identity::generate();
            let mut profile = UserProfile::new(name);
            profile.identity = Some(identity.public());
            (profile, identity)
        };
        let ((me, my_key), (other, other_key)) = (member("me"), member("other"));
        let server = commy_core::Server::new("s", me.id);
        let (sid, channel) = (server.id, server.channels[0].id);
        let mut state = ServerState::new(server);
        state.profiles.insert(me.id, me.clone());
        state.apply(ServerEvent::MemberJoin { server: sid, profile: other.clone() }).unwrap();
        let post = |author: &UserProfile, key: &Identity| {
            let mut message = Message::new(channel, author.id, "hi");
            message.sign(sid, key);
            message
        };
        let (ours, theirs) = (post(&me, &my_key), post(&other, &other_key));
        for message in [&ours, &theirs] {
            state.apply(ServerEvent::MessageCreate { message: message.clone() }).unwrap();
        }
        let app = App::default();
        app.servers.lock().unwrap().insert(sid, state.clone());
        app.set_awaiting_snapshot(sid, true);

        assert_eq!(may_delete(&app, &state, channel, ours.id, me.id), Ok(()));
        let refused = Err("you do not have permission to delete other people's messages".to_string());
        assert_eq!(may_delete(&app, &state, channel, theirs.id, me.id), refused);
        let moderation = [
            Permissions::MANAGE_SERVER,
            Permissions::MANAGE_CHANNELS,
            Permissions::KICK_MEMBERS,
            Permissions::BAN_MEMBERS,
            Permissions::MUTE_MEMBERS,
            Permissions::MANAGE_ROLES,
            Permissions::MANAGE_EXPRESSIONS,
            Permissions::MANAGE_EVENTS,
        ];
        for needed in moderation {
            assert!(require_permission(&app, &state, me.id, needed, "moderate").is_err(), "{needed:?}");
        }
        assert!(require_permission(&app, &state, me.id, Permissions::USE_SOUNDBOARD, "play").is_ok());
        assert!(!can_upload_sounds(&app, &state, me.id));
        let structural = |events: Vec<ServerEvent>| {
            events.iter().any(|e| matches!(e, ServerEvent::ChannelCreate { .. } | ServerEvent::AfkChannelSet { .. }))
        };
        assert!(!structural(structure_announcements(&app, sid, me.id)));

        app.set_awaiting_snapshot(sid, false);
        assert_eq!(may_delete(&app, &state, channel, theirs.id, me.id), Ok(()));
        for needed in moderation {
            assert!(require_permission(&app, &state, me.id, needed, "moderate").is_ok(), "{needed:?}");
        }
        assert!(structural(structure_announcements(&app, sid, me.id)));
    }
}

#[cfg(test)]
mod status_expiry_tests {
    use super::*;

    #[test]
    fn only_past_deadlines_expire() {
        let mut voice = VoiceSettings::default();
        assert_eq!(expired_status(&voice, 100), (false, false));
        voice.presence_until = Some(100);
        voice.status_until = Some(200);
        assert_eq!(expired_status(&voice, 150), (true, false));
        assert_eq!(expired_status(&voice, 200), (true, true));
    }
}

fn register_overlay_hotkeys(handle: &tauri::AppHandle, settings: &overlay::OverlaySettings) -> Vec<String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let shortcuts = handle.global_shortcut();
    let _ = shortcuts.unregister_all();
    let mut taken = Vec::new();
    if settings.enabled {
        for key in [&settings.toggle_hotkey, &settings.chat_hotkey] {
            if let Err(e) = shortcuts.register(key.as_str()) {
                tracing::warn!("could not register hotkey {key}: {e}");
                taken.push(key.clone());
            }
        }
    }
    *handle.state::<App>().overlay_hotkeys_taken.lock().unwrap() = taken.clone();
    taken
}

#[tauri::command]
fn overlay_hotkey_problems(app: tauri::State<'_, App>) -> Vec<String> {
    app.overlay_hotkeys_taken.lock().unwrap().clone()
}

fn hotkey_id(key: &str) -> Option<u32> {
    key.parse::<tauri_plugin_global_shortcut::Shortcut>().ok().map(|s| s.id())
}

fn on_overlay_hotkey(handle: &tauri::AppHandle, id: u32) {
    let app = handle.state::<App>();
    let settings = app.voice.lock().unwrap().overlay.clone();
    if hotkey_id(&settings.toggle_hotkey) == Some(id) {
        let visible = !app.overlay_visible.load(std::sync::atomic::Ordering::Relaxed);
        tracing::info!("overlay hotkey: {}", if visible { "show" } else { "hide" });
        app.overlay_visible.store(visible, std::sync::atomic::Ordering::Relaxed);
        overlay::apply(handle, &settings, visible);
    } else if hotkey_id(&settings.chat_hotkey) == Some(id) {
        let settings = {
            let mut voice = app.voice.lock().unwrap();
            voice.overlay.show_chat = !voice.overlay.show_chat;
            voice.overlay.clone()
        };
        app.persist_voice();
        let _ = handle.emit_to(overlay::LABEL, "overlay-config", &settings);
        let _ = handle.emit("overlay-settings-changed", &settings);
    }
}

#[tauri::command]
fn get_overlay_settings(app: tauri::State<'_, App>) -> overlay::OverlaySettings {
    app.voice.lock().unwrap().overlay.clone()
}

#[tauri::command]
async fn set_overlay_settings(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    settings: overlay::OverlaySettings,
) -> CmdResult<overlay::OverlaySettings> {
    let settings = settings.sanitized();
    for key in [&settings.toggle_hotkey, &settings.chat_hotkey] {
        if !overlay::valid_hotkey(key) {
            return Err(format!("{key} is not a hotkey Commy can use"));
        }
    }
    if settings.toggle_hotkey.eq_ignore_ascii_case(&settings.chat_hotkey) {
        return Err("the two overlay hotkeys must be different".into());
    }
    app.voice.lock().unwrap().overlay = settings.clone();
    app.persist_voice();
    if settings.enabled {
        app.overlay_visible.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    register_overlay_hotkeys(&handle, &settings);
    overlay::apply(&handle, &settings, app.overlay_visible.load(std::sync::atomic::Ordering::Relaxed));
    Ok(settings)
}

#[tauri::command]
async fn set_about(app: tauri::State<'_, App>, handle: tauri::AppHandle, about: String) -> CmdResult<UserProfile> {
    let about = about.trim().to_string();
    if about.chars().count() > commy_core::user::MAX_ABOUT_CHARS {
        return Err(format!("About Me can be up to {} characters", commy_core::user::MAX_ABOUT_CHARS));
    }
    let profile = {
        let mut guard = app.profile.lock().unwrap();
        let profile = guard.as_mut().ok_or("set a username first")?;
        profile.about = (!about.is_empty()).then_some(about);
        profile.clone()
    };
    app.persist_profile();
    announce_profile(&app, &handle, &profile).await;
    let _ = handle.emit("profile-changed", &profile);
    Ok(profile)
}

const SERVER_IMAGE_DIR: &str = "server-images";
const MAX_SERVER_IMAGE_BYTES: usize = 1024 * 1024;

fn server_image_path(app: &tauri::State<'_, App>, blob: &str) -> Option<std::path::PathBuf> {
    app.data_dir().map(|d| d.join(SERVER_IMAGE_DIR).join(format!("{blob}.img")))
}

#[tauri::command]
async fn set_server_image(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    which: String,
    data: Option<String>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let (icon, banner) = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_SERVER, "change the server's images")?;
        (state.server.icon.clone(), state.server.banner.clone())
    };
    let image = match data {
        None => None,
        Some(b64) => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|_| "image data is not valid base64".to_string())?;
            if bytes.len() > MAX_SERVER_IMAGE_BYTES || !is_jpeg_or_png(&bytes) {
                return Err("server images must be a JPEG or PNG under 1 MB".into());
            }
            let (blob, provider) = {
                let net = app.net.lock().await;
                let node = net.as_ref().ok_or("network is not ready yet")?;
                (node.add_blob(bytes.clone()).await.map_err(err_to_string)?, node.node_id_hex())
            };
            if let Some(path) = server_image_path(&app, &blob) {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(&path, &bytes).map_err(err_to_string)?;
            }
            Some(commy_core::server::ImageRef { blob, provider }.encode())
        }
    };
    let (icon, banner) = match which.as_str() {
        "icon" => (image, banner),
        "banner" => (icon, image),
        _ => return Err("unknown server image".into()),
    };
    apply_and_broadcast(&app, &handle, sid, ServerEvent::ServerImages { server: sid, icon, banner }).await?;
    let _ = handle.emit("servers-changed", ());
    Ok(())
}

#[tauri::command]
async fn fetch_server_image(app: tauri::State<'_, App>, image: String) -> CmdResult<String> {
    let image = commy_core::server::ImageRef::parse(&image).ok_or("bad image reference")?;
    let cached = server_image_path(&app, &image.blob).and_then(|p| std::fs::read(p).ok());
    let bytes = match cached {
        Some(bytes) => bytes,
        None => {
            let bytes = fetch_blob(&app, &image.blob, Some(&image.provider)).await?;
            if bytes.len() > MAX_SERVER_IMAGE_BYTES || !is_jpeg_or_png(&bytes) {
                return Err("server image is not a valid picture".into());
            }
            if let Some(path) = server_image_path(&app, &image.blob) {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&path, &bytes);
            }
            bytes
        }
    };
    let mime = if bytes.starts_with(&[0x89, b'P']) { "image/png" } else { "image/jpeg" };
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes)))
}

fn manage_check(app: &tauri::State<'_, App>, sid: ServerId, user: UserId, needed: Permissions, action: &str) -> CmdResult<()> {
    let servers = app.servers.lock().unwrap();
    let state = servers.get(&sid).ok_or("unknown server")?;
    require_permission(app, state, user, needed, action)
}

#[tauri::command]
async fn set_server_afk_timeout(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    minutes: Option<u32>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    manage_check(&app, sid, me.id, Permissions::MANAGE_SERVER, "change the AFK timeout")?;
    let minutes = minutes.filter(|m| *m > 0);
    if minutes.is_some_and(|m| !commy_core::server::SERVER_AFK_MINUTES.contains(&m)) {
        return Err("pick 1, 5, 15, 30 or 60 minutes".into());
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::AfkTimeoutSet { server: sid, minutes }).await
}

#[tauri::command]
async fn set_game_tags(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    tags: Vec<String>,
    meta: Option<Vec<commy_core::server::GameTagMeta>>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    manage_check(&app, sid, me.id, Permissions::MANAGE_SERVER, "change game tags")?;
    let tags = commy_core::server::clean_game_tags(&tags).ok_or(format!(
        "up to {} games, each at most {} characters",
        commy_core::server::MAX_GAME_TAGS,
        commy_core::server::MAX_GAME_TAG_CHARS
    ))?;
    let meta = commy_core::server::clean_game_tag_meta(&tags, &meta.unwrap_or_default());
    apply_and_broadcast(&app, &handle, sid, ServerEvent::GameTagsSet { server: sid, tags, meta }).await
}

#[tauri::command]
async fn set_rules(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String, rules: Option<String>) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    manage_check(&app, sid, me.id, Permissions::MANAGE_SERVER, "change the rules")?;
    let rules = rules.map(|r| r.trim().to_string()).filter(|r| !r.is_empty());
    if rules.as_deref().is_some_and(|r| !commy_core::server::valid_rules(r)) {
        return Err("rules are too long; keep them under about 3000 characters".into());
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::RulesSet { server: sid, rules }).await
}

#[tauri::command]
async fn accept_rules(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    apply_and_broadcast(&app, &handle, sid, ServerEvent::RulesAccept { server: sid, user: me.id }).await
}

fn require_can_post(app: &App, state: &ServerState, user: UserId, channel: ChannelId) -> CmdResult<()> {
    let announcement = state.server.channel(channel).is_some_and(|c| c.kind == ChannelKind::Announcement);
    if announcement && !our_permissions(app, state, user).can_announce() {
        return Err("only members allowed to post announcements can write in this channel".into());
    }
    Ok(())
}

fn require_rules_accepted(state: &ServerState, user: UserId) -> CmdResult<()> {
    let exempt = user == state.server.owner || state.server.permissions_for(user).allows(Permissions::MANAGE_SERVER);
    if state.server.rules.is_some() && !exempt && !state.server.rules_accepted.contains(&user) {
        return Err("read and accept the server rules before posting".into());
    }
    Ok(())
}

fn parse_time(raw: &str) -> CmdResult<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|t| t.with_timezone(&chrono::Utc))
        .map_err(|_| "invalid date or time".to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn save_event(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    event_id: Option<String>,
    title: String,
    description: String,
    start: String,
    end: Option<String>,
    channel_id: Option<String>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let channel: Option<ChannelId> = match channel_id.filter(|c| !c.is_empty()) {
        Some(c) => Some(c.parse().map_err(err_to_string)?),
        None => None,
    };
    let creator = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        let can_manage = our_permissions(&app, state, me.id).allows(Permissions::MANAGE_EVENTS);
        let existing = event_id.as_ref().and_then(|id| state.server.events.iter().find(|e| &e.id == id));
        match existing {
            Some(e) if e.creator == me.id || can_manage => e.creator,
            Some(_) => return Err("you can only edit events you created".into()),
            None if can_manage => me.id,
            None => return Err("you do not have permission to create events".into()),
        }
    };
    let event = commy_core::server::ScheduledEvent {
        id: event_id.unwrap_or_else(|| MessageId::new().to_string()),
        title: title.trim().to_string(),
        description: description.trim().to_string(),
        start: parse_time(&start)?,
        end: end.filter(|e| !e.is_empty()).map(|e| parse_time(&e)).transpose()?,
        channel,
        creator,
        rsvps: Vec::new(),
    };
    if !commy_core::server::valid_event(&event) {
        return Err("events need a title (up to 100 characters), and the end must be after the start".into());
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::EventUpsert { server: sid, event }).await
}

#[tauri::command]
async fn delete_event(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String, event_id: String) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        let mine = state.server.events.iter().any(|e| e.id == event_id && e.creator == me.id);
        if !mine {
            require_permission(&app, state, me.id, Permissions::MANAGE_EVENTS, "delete events")?;
        }
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::EventDelete { server: sid, event: event_id }).await
}

#[tauri::command]
async fn set_rsvp(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String, event_id: String, rsvp: Option<String>) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let rsvp = match rsvp.as_deref() {
        None | Some("") => None,
        Some("going") => Some(commy_core::server::Rsvp::Going),
        Some("interested") => Some(commy_core::server::Rsvp::Interested),
        Some(_) => return Err("unknown RSVP".into()),
    };
    apply_and_broadcast(&app, &handle, sid, ServerEvent::EventRsvp { server: sid, event: event_id, user: me.id, rsvp }).await
}

#[derive(serde::Deserialize)]
struct ChannelMove {
    channel: String,
    parent: Option<String>,
    position: u32,
}

#[tauri::command]
async fn move_channels(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String, moves: Vec<ChannelMove>) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    manage_check(&app, sid, me.id, Permissions::MANAGE_CHANNELS, "move channels")?;
    for m in moves.into_iter().take(200) {
        let channel: ChannelId = m.channel.parse().map_err(err_to_string)?;
        let parent: Option<ChannelId> = match m.parent {
            Some(p) => Some(p.parse().map_err(err_to_string)?),
            None => None,
        };
        let event = ServerEvent::ChannelLayout { server: sid, channel, parent, position: m.position };
        let _ = apply_and_broadcast(&app, &handle, sid, event).await;
    }
    Ok(())
}

#[tauri::command]
async fn create_category(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String, name: String) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 50 {
        return Err("category names must be 1 to 50 characters".into());
    }
    let me = current_profile(&app)?;
    let channel = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_CHANNELS, "create categories")?;
        let position = state.server.channels.iter().map(|c| c.position).max().unwrap_or(0) + 1;
        Channel::new(name, ChannelKind::Category, position)
    };
    apply_and_broadcast(&app, &handle, sid, ServerEvent::ChannelCreate { server: sid, channel }).await
}

#[tauri::command]
async fn set_channel_limit(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String, channel_id: String, limit: u32) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    manage_check(&app, sid, me.id, Permissions::MANAGE_CHANNELS, "change user limits")?;
    if limit > commy_core::server::MAX_USER_LIMIT {
        return Err(format!("the limit can be at most {}", commy_core::server::MAX_USER_LIMIT));
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::ChannelLimit { server: sid, channel: cid, user_limit: limit }).await
}

fn voice_channel_full(app: &App, state: &ServerState, channel: ChannelId, user: UserId) -> bool {
    let Some(c) = state.server.channel(channel) else { return false };
    if c.user_limit == 0 || our_permissions(app, state, user).allows(Permissions::MANAGE_CHANNELS) {
        return false;
    }
    let inside = state.voice_states.iter().filter(|(u, ch)| **ch == channel && **u != user).count();
    inside >= c.user_limit as usize
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn update_role(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    role_id: String,
    name: String,
    color: Option<String>,
    self_assignable: bool,
    tag: Option<String>,
    permissions: Option<Vec<String>>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let rid: RoleId = role_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let tag = tag.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    if tag.as_deref().is_some_and(|t| !commy_core::server::valid_role_tag(t)) {
        return Err(format!("tags can be up to {} characters", commy_core::server::MAX_ROLE_TAG_CHARS));
    }
    let color = color.map(|c| c.trim_start_matches('#').to_lowercase()).filter(|c| !c.is_empty());
    if color.as_deref().is_some_and(|c| c.len() != 6 || !c.chars().all(|ch| ch.is_ascii_hexdigit())) {
        return Err("colors are six hex digits".into());
    }
    let role = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_ROLES, "edit roles")?;
        let existing = state.server.roles.iter().find(|r| r.id == rid).ok_or("unknown role")?;
        let name = name.trim().to_string();
        if name.is_empty() || name.chars().count() > 50 {
            return Err("role names must be 1 to 50 characters".into());
        }
        let permissions = permissions
            .map(|names| permissions_from_names(&names))
            .unwrap_or(existing.permissions);
        commy_core::server::Role { name, color, self_assignable, tag, permissions, ..existing.clone() }
    };
    apply_and_broadcast(&app, &handle, sid, ServerEvent::RoleUpdate { server: sid, role }).await
}

#[tauri::command]
async fn set_role_banner(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    role_id: String,
    data: Option<String>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let rid: RoleId = role_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let existing = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_ROLES, "edit roles")?;
        state.server.roles.iter().find(|r| r.id == rid).cloned().ok_or("unknown role")?
    };
    let banner = match data {
        None => None,
        Some(b64) => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|_| "image data is not valid base64".to_string())?;
            if bytes.len() > MAX_SERVER_IMAGE_BYTES || !is_jpeg_or_png(&bytes) {
                return Err("role banners must be a JPEG or PNG under 1 MB".into());
            }
            Some(store_shared_blob(&app, bytes).await?)
        }
    };
    let role = commy_core::server::Role { banner, ..existing };
    apply_and_broadcast(&app, &handle, sid, ServerEvent::RoleUpdate { server: sid, role }).await
}

#[tauri::command]
async fn set_channel_banner(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    data: Option<String>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_CHANNELS, "edit channels")?;
        state.server.channel(cid).ok_or("unknown channel")?;
    }
    let banner = match data {
        None => None,
        Some(b64) => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|_| "image data is not valid base64".to_string())?;
            if bytes.len() > MAX_SERVER_IMAGE_BYTES || !is_jpeg_or_png(&bytes) {
                return Err("channel banners must be a JPEG or PNG under 1 MB".into());
            }
            Some(store_shared_blob(&app, bytes).await?)
        }
    };
    apply_and_broadcast(&app, &handle, sid, ServerEvent::ChannelBanner { server: sid, channel: cid, banner }).await
}

#[tauri::command]
async fn set_channel_color(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    color: Option<String>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_CHANNELS, "edit channels")?;
        state.server.channel(cid).ok_or("unknown channel")?;
    }
    let color = color.map(|c| c.trim().trim_start_matches('#').to_ascii_lowercase());
    if color.as_deref().is_some_and(|c| c.len() != 6 || !c.chars().all(|ch| ch.is_ascii_hexdigit())) {
        return Err("channel colour must be six hex digits".into());
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::ChannelColor { server: sid, channel: cid, color }).await
}

#[tauri::command]
async fn set_channel_kind(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    kind: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let kind = match kind.as_str() {
        "text" => ChannelKind::Text,
        "announcement" => ChannelKind::Announcement,
        _ => return Err("channel kind must be text or announcement".into()),
    };
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_CHANNELS, "edit channels")?;
        let channel = state.server.channel(cid).ok_or("unknown channel")?;
        if !matches!(channel.kind, ChannelKind::Text | ChannelKind::Announcement) {
            return Err("only text channels can become announcement channels".into());
        }
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::ChannelKindSet { server: sid, channel: cid, kind }).await
}

#[tauri::command]
async fn pin_message(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    message_id: String,
    pinned: bool,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let mid: MessageId = message_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        if !our_permissions(&app, state, me.id).can_pin() {
            return Err("you do not have permission to pin messages".into());
        }
        message_author(state, cid, mid)?;
        if pinned && state.server.pins_in(cid).len() >= commy_core::server::MAX_PINS_PER_CHANNEL {
            return Err(format!(
                "a channel can hold {} pinned messages; unpin one first",
                commy_core::server::MAX_PINS_PER_CHANNEL
            ));
        }
    }
    let event = ServerEvent::MessagePin { server: sid, channel: cid, message: mid, by: me.id, at: chrono::Utc::now(), pinned };
    apply_and_broadcast(&app, &handle, sid, event).await
}

#[tauri::command]
fn get_pins(app: tauri::State<App>, server_id: String, channel_id: Option<String>) -> CmdResult<Vec<view::PinView>> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid = match channel_id {
        Some(raw) => Some(raw.parse::<ChannelId>().map_err(err_to_string)?),
        None => None,
    };
    let servers = app.servers.lock().unwrap();
    let state = servers.get(&sid).ok_or("unknown server")?;
    Ok(view::pin_views(state, cid))
}

#[derive(serde::Serialize)]
struct LinkView {
    user: String,
    name: String,
    rtt_ms: f32,
    loss_pct: f32,
    jitter_ms: f32,
    path: String,
    relay: Option<String>,
    addr: Option<String>,
    tx_bytes: u64,
    rx_bytes: u64,
}

#[derive(serde::Serialize)]
struct VoiceStatsView {
    links: Vec<LinkView>,
    codec: String,
    bitrate_kbps: u32,
}

#[tauri::command]
async fn voice_stats(app: tauri::State<'_, App>) -> CmdResult<VoiceStatsView> {
    let (stats, sid) = {
        let session = app.voice_session.lock().unwrap();
        let session = session.as_ref().ok_or("not in voice")?;
        (session.link_stats(), session.server)
    };
    let names: Vec<(String, Option<String>)> = {
        let servers = app.servers.lock().unwrap();
        stats
            .iter()
            .map(|s| {
                let name = servers.get(&sid).map(|st| view::display_name(st, s.user)).unwrap_or_else(|| "Someone".into());
                (name, node_hex_for_user(&app, s.user))
            })
            .collect()
    };
    let net = app.net.lock().await;
    let links = stats
        .into_iter()
        .zip(names)
        .map(|(s, (name, hex))| {
            let (path, relay, addr) = hex
                .and_then(|h| net.as_ref().and_then(|n| n.peer_path(&h)))
                .unwrap_or_else(|| ("none".into(), None, None));
            LinkView {
                user: s.user.to_string(),
                name,
                rtt_ms: s.rtt_ms,
                loss_pct: s.loss_pct,
                jitter_ms: s.jitter_ms,
                path,
                relay,
                addr,
                tx_bytes: s.tx_bytes,
                rx_bytes: s.rx_bytes,
            }
        })
        .collect();
    Ok(VoiceStatsView {
        links,
        codec: "Opus, 48 kHz mono, 10 ms frames".into(),
        bitrate_kbps: (commy_audio::codec::VOICE_BITRATE / 1000) as u32,
    })
}

#[tauri::command]
async fn moderate_voice(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    user_id: String,
    muted: bool,
    deafened: bool,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let user: UserId = user_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MUTE_MEMBERS, "server mute or deafen")?;
        if user == state.server.owner {
            return Err("the server owner cannot be muted or deafened".into());
        }
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::VoiceModerate { server: sid, user, muted, deafened }).await?;
    refresh_forced_voice(&app, &handle);
    Ok(())
}

#[tauri::command]
async fn ban_member(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String, user_id: String) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let user: UserId = user_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::BAN_MEMBERS, "ban members")?;
        if user == state.server.owner || user == me.id {
            return Err("you cannot ban that member".into());
        }
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::MemberBan { server: sid, user }).await
}

#[tauri::command]
async fn unban_member(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String, user_id: String) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let user: UserId = user_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    manage_check(&app, sid, me.id, Permissions::BAN_MEMBERS, "unban members")?;
    apply_and_broadcast(&app, &handle, sid, ServerEvent::MemberUnban { server: sid, user }).await
}

async fn on_banned(app: &tauri::State<'_, App>, handle: &tauri::AppHandle, sid: ServerId) {
    let name = app.servers.lock().unwrap().remove(&sid).map(|s| s.server.name);
    let in_voice_there = app.voice_session.lock().unwrap().as_ref().is_some_and(|s| s.server == sid);
    if in_voice_there {
        *app.voice_session.lock().unwrap() = None;
        app.own_preview.lock().unwrap().take();
        *app.screen_share.lock().unwrap() = None;
    }
    app.invites.lock().unwrap().remove(&sid);
    forget_server_outbox(app, sid);
    if let Some(dir) = app.data_dir() {
        let _ = persist::delete_server(dir, sid);
    }
    {
        let mut net = app.net.lock().await;
        if let Some(node) = net.as_mut() {
            node.leave_server(sid);
        }
    }
    let _ = handle.emit("banned", serde_json::json!({ "server": sid.to_string(), "name": name }));
    let _ = handle.emit("servers-changed", ());
}

fn effective_volume(voice: &VoiceSettings, user: &str) -> f32 {
    if voice.muted_users.iter().any(|u| u == user) {
        0.0
    } else {
        voice.user_volumes.get(user).copied().unwrap_or(1.0)
    }
}

fn apply_personal_volumes(app: &App) {
    let voice = app.voice.lock().unwrap().clone();
    if let Some(session) = app.voice_session.lock().unwrap().as_ref() {
        let mut mixer = session.mixer.lock().unwrap();
        let users = voice.user_volumes.keys().chain(voice.muted_users.iter());
        for user in users {
            if let Ok(id) = user.parse::<UserId>() {
                mixer.set_peer_gain(media_engine::mixer_key(id), effective_volume(&voice, user));
            }
        }
    }
}

#[tauri::command]
fn set_user_voice(app: tauri::State<'_, App>, user_id: String, volume: Option<f32>, muted: Option<bool>) -> CmdResult<VoiceSettings> {
    let user: UserId = user_id.parse().map_err(err_to_string)?;
    let key = user.to_string();
    let settings = {
        let mut voice = app.voice.lock().unwrap();
        if let Some(volume) = volume {
            let volume = volume.clamp(0.0, 2.0);
            if (volume - 1.0).abs() < f32::EPSILON {
                voice.user_volumes.remove(&key);
            } else {
                voice.user_volumes.insert(key.clone(), volume);
            }
        }
        if let Some(muted) = muted {
            voice.muted_users.retain(|u| u != &key);
            if muted {
                voice.muted_users.push(key.clone());
            }
        }
        voice.clone()
    };
    app.persist_voice();
    if let Some(session) = app.voice_session.lock().unwrap().as_ref() {
        session.mixer.lock().unwrap().set_peer_gain(media_engine::mixer_key(user), effective_volume(&settings, &key));
    }
    Ok(settings)
}

async fn run_share_heartbeat(handle: tauri::AppHandle) {
    const BEAT: std::time::Duration = std::time::Duration::from_secs(8);
    const EXPIRE: std::time::Duration = std::time::Duration::from_secs(25);
    loop {
        tokio::time::sleep(BEAT).await;
        let app = handle.state::<App>();
        let Some(me) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { continue };
        let ours = app.screen_share.lock().unwrap().as_ref().map(|s| (s.server, s.channel));
        if let Some((sid, channel)) = ours {
            broadcast(&app, sid, WirePayload::Event(ServerEvent::ScreenShare { user: me, channel, active: true })).await;
        }
        let now = std::time::Instant::now();
        let mut expired = Vec::new();
        {
            let servers = app.servers.lock().unwrap();
            let mut seen = app.share_seen.lock().unwrap();
            for (sid, state) in servers.iter() {
                for user in state.screen_shares.keys().filter(|u| **u != me) {
                    let last = *seen.entry((*sid, *user)).or_insert(now);
                    if now.duration_since(last) > EXPIRE {
                        expired.push((*sid, *user));
                    }
                }
            }
        }
        for (sid, user) in expired {
            let changed = {
                let mut servers = app.servers.lock().unwrap();
                servers.get_mut(&sid).map(|state| {
                    let channel = state.screen_shares.get(&user).copied();
                    channel.is_some_and(|channel| {
                        state.apply(ServerEvent::ScreenShare { user, channel, active: false }) == Ok(true)
                    })
                })
            };
            app.share_seen.lock().unwrap().remove(&(sid, user));
            if changed == Some(true) {
                tracing::info!("expired a screen share nobody has announced recently");
                let _ = handle.emit("server-event", &sid.to_string());
            }
        }
    }
}

#[tauri::command]
async fn set_own_role(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String, role_id: String, held: bool) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let rid: RoleId = role_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        let pickable = state.server.roles.iter().any(|r| r.id == rid && r.self_assignable);
        if !pickable {
            return Err("that role cannot be picked".into());
        }
    }
    let event = if held {
        ServerEvent::RoleAssign { server: sid, user: me.id, role: rid }
    } else {
        ServerEvent::RoleUnassign { server: sid, user: me.id, role: rid }
    };
    apply_and_broadcast(&app, &handle, sid, event).await
}

async fn run_event_reminders(handle: tauri::AppHandle) {
    const LEAD_MINUTES: i64 = 15;
    let mut reminded: std::collections::HashSet<String> = std::collections::HashSet::new();
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let app = handle.state::<App>();
        let Some(profile) = app.profile.lock().unwrap().clone() else { continue };
        if profile.presence == Presence::DoNotDisturb {
            continue;
        }
        let now = chrono::Utc::now();
        let due: Vec<(ServerId, String, String, String, i64)> = app
            .servers
            .lock()
            .unwrap()
            .values()
            .flat_map(|s| {
                s.server.events.iter().filter_map(|e| {
                    let mins = (e.start - now).num_minutes();
                    let answered = e.rsvps.iter().any(|(u, _)| *u == profile.id);
                    (answered && (0..=LEAD_MINUTES).contains(&mins)).then(|| {
                        (s.server.id, s.server.name.clone(), e.id.clone(), e.title.clone(), mins)
                    })
                })
            })
            .collect();
        for (sid, server_name, id, title, mins) in due {
            if !reminded.insert(format!("{sid}:{id}")) {
                continue;
            }
            let when = if mins <= 0 { "now".to_string() } else { format!("in {mins} min") };
            let _ = toast::show(&handle, &format!("{title} starts {when}"), &server_name, None);
            let _ = handle.emit("event-reminder", serde_json::json!({ "server": sid.to_string(), "event": id }));
        }
    }
}

async fn store_shared_blob(app: &tauri::State<'_, App>, bytes: Vec<u8>) -> CmdResult<String> {
    let (blob, provider) = {
        let net = app.net.lock().await;
        let node = net.as_ref().ok_or("network is not ready yet")?;
        (node.add_blob(bytes.clone()).await.map_err(err_to_string)?, node.node_id_hex())
    };
    if let Some(path) = server_image_path(app, &blob) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&path, &bytes).map_err(err_to_string)?;
    }
    Ok(commy_core::server::ImageRef { blob, provider }.encode())
}

fn decode_upload(data: &str) -> CmdResult<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| "file data is not valid base64".to_string())
}

fn check_expression_name(name: &str) -> CmdResult<String> {
    let name = name.trim().to_ascii_lowercase();
    if commy_core::server::valid_expression_name(&name) {
        Ok(name)
    } else {
        Err("names must be 2 to 32 letters, numbers or underscores".into())
    }
}

#[tauri::command]
async fn add_emoji(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    name: String,
    data: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let name = check_expression_name(&name)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_EXPRESSIONS, "manage emojis")?;
        let replacing = state.server.emojis.iter().any(|e| e.name == name);
        if !replacing && state.server.emojis.len() >= commy_core::server::MAX_EMOJIS {
            return Err(format!("servers can have up to {} emojis", commy_core::server::MAX_EMOJIS));
        }
    }
    let bytes = decode_upload(&data)?;
    if bytes.len() > expressions::MAX_EMOJI_BYTES || expressions::image_mime(&bytes).is_none() {
        return Err("emojis must be a PNG, GIF, JPEG or WebP under 256 KB".into());
    }
    let blob = store_shared_blob(&app, bytes).await?;
    let emoji = commy_core::server::CustomEmoji { name, blob };
    apply_and_broadcast(&app, &handle, sid, ServerEvent::EmojiAdd { server: sid, emoji }).await
}

#[tauri::command]
async fn remove_emoji(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    name: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_EXPRESSIONS, "manage emojis")?;
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::EmojiRemove { server: sid, name }).await
}

fn can_upload_sounds(app: &App, state: &ServerState, user: UserId) -> bool {
    our_permissions(app, state, user).allows(Permissions::MANAGE_EXPRESSIONS)
        || (state.server.soundboard_open && state.server.members.iter().any(|m| m.user == user))
}

#[tauri::command]
async fn add_sound(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    name: String,
    data: String,
    volume: Option<f32>,
    emoji: Option<String>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let name = check_expression_name(&name)?;
    let emoji = check_sound_emoji(emoji)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        if !can_upload_sounds(&app, state, me.id) {
            return Err("you do not have permission to add sounds".into());
        }
        let existing = state.server.soundboard.iter().any(|s| s.name == name);
        if existing && !our_permissions(&app, state, me.id).allows(Permissions::MANAGE_EXPRESSIONS) {
            return Err("a sound with that name already exists".into());
        }
        if !existing && state.server.soundboard.len() >= commy_core::server::MAX_SOUNDS {
            return Err(format!("servers can have up to {} sounds", commy_core::server::MAX_SOUNDS));
        }
    }
    let bytes = decode_upload(&data)?;
    if bytes.len() > expressions::MAX_SOUND_BYTES || expressions::audio_mime(&bytes).is_none() {
        return Err("sounds must be an MP3, OGG or WAV under 1 MB".into());
    }
    let blob = store_shared_blob(&app, bytes).await?;
    let sound = commy_core::server::SoundboardSound {
        name,
        blob,
        volume: volume.unwrap_or(1.0).clamp(0.0, 2.0),
        emoji,
    };
    apply_and_broadcast(&app, &handle, sid, ServerEvent::SoundAdd { server: sid, sound }).await
}

fn check_sound_emoji(emoji: Option<String>) -> CmdResult<Option<String>> {
    match emoji.map(|e| e.trim().to_string()).filter(|e| !e.is_empty()) {
        None => Ok(None),
        Some(e) if commy_core::server::valid_sound_emoji(&e) => Ok(Some(e)),
        Some(_) => Err("pick a single emoji for the sound".into()),
    }
}

#[tauri::command]
async fn set_sound_emoji(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    name: String,
    emoji: Option<String>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let emoji = check_sound_emoji(emoji)?;
    let me = current_profile(&app)?;
    let sound = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_EXPRESSIONS, "change sounds")?;
        let mut sound = state.server.soundboard.iter().find(|s| s.name == name).cloned().ok_or("unknown sound")?;
        sound.emoji = emoji;
        sound
    };
    apply_and_broadcast(&app, &handle, sid, ServerEvent::SoundAdd { server: sid, sound }).await
}

#[tauri::command]
async fn remove_sound(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    name: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_EXPRESSIONS, "remove sounds")?;
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::SoundRemove { server: sid, name }).await
}

#[tauri::command]
async fn set_expression_settings(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    emojis_shareable: bool,
    soundboard_open: bool,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_SERVER, "change expression settings")?;
    }
    let event = ServerEvent::ExpressionSettings { server: sid, emojis_shareable, soundboard_open };
    apply_and_broadcast(&app, &handle, sid, event).await?;
    let _ = handle.emit("servers-changed", ());
    Ok(())
}

#[tauri::command]
async fn set_server_public(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    public: bool,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_SERVER, "make this server public")?;
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::PublicSet { server: sid, public }).await?;
    let _ = handle.emit("servers-changed", ());
    Ok(())
}

#[derive(Clone, serde::Serialize)]
struct SoundboardPlayView {
    server: String,
    channel: String,
    user: String,
    name: String,
    blob: String,
    volume: f32,
}

fn soundboard_play_view(state: &ServerState, channel: ChannelId, user: UserId, name: &str) -> Option<SoundboardPlayView> {
    if expressions::is_default_sound(name) {
        return Some(SoundboardPlayView {
            server: state.server.id.to_string(),
            channel: channel.to_string(),
            user: user.to_string(),
            name: name.to_string(),
            blob: name.to_string(),
            volume: 1.0,
        });
    }
    let sound = state.server.soundboard.iter().find(|s| s.name == name)?;
    Some(SoundboardPlayView {
        server: state.server.id.to_string(),
        channel: channel.to_string(),
        user: user.to_string(),
        name: sound.name.clone(),
        blob: sound.blob.clone(),
        volume: sound.volume,
    })
}

#[tauri::command]
async fn play_soundboard(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    name: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let (channel, in_server) = {
        let session = app.voice_session.lock().unwrap();
        let s = session.as_ref().ok_or("join a voice channel first")?;
        (s.channel, s.server == sid)
    };
    if !in_server {
        return Err("join a voice channel in this server first".into());
    }
    let view = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::USE_SOUNDBOARD, "use the soundboard")?;
        soundboard_play_view(state, channel, me.id, &name).ok_or("that sound no longer exists")?
    };
    broadcast(
        &app,
        sid,
        WirePayload::Event(ServerEvent::SoundboardPlay { server: sid, channel, user: me.id, name }),
    )
    .await;
    let _ = handle.emit("soundboard-play", view);
    Ok(())
}

fn on_remote_soundboard(app: &tauri::State<'_, App>, handle: &tauri::AppHandle, sid: ServerId, channel: ChannelId, user: UserId, name: &str) {
    let mine = app.voice_session.lock().unwrap().as_ref().map(|s| (s.server, s.channel));
    if mine != Some((sid, channel)) {
        return;
    }
    let view = {
        let servers = app.servers.lock().unwrap();
        servers.get(&sid).and_then(|state| {
            let allowed = state.voice_states.get(&user) == Some(&channel)
                && state.server.permissions_for(user).allows(Permissions::USE_SOUNDBOARD);
            allowed.then(|| soundboard_play_view(state, channel, user, name)).flatten()
        })
    };
    if let Some(view) = view {
        let _ = handle.emit("soundboard-play", view);
    }
}

#[derive(Clone, serde::Serialize)]
struct SharedEmojiView {
    server: String,
    server_name: String,
    name: String,
    image: String,
}

#[tauri::command]
fn list_shareable_emojis(app: tauri::State<'_, App>) -> Vec<SharedEmojiView> {
    let servers = app.servers.lock().unwrap();
    let mut out: Vec<SharedEmojiView> = servers
        .values()
        .filter(|s| s.server.emojis_shareable)
        .flat_map(|s| {
            s.server.emojis.iter().map(|e| SharedEmojiView {
                server: s.server.id.to_string(),
                server_name: s.server.name.clone(),
                name: e.name.clone(),
                image: e.blob.clone(),
            })
        })
        .collect();
    out.sort_by(|a, b| a.server_name.cmp(&b.server_name).then(a.name.cmp(&b.name)));
    out
}

#[tauri::command]
async fn fetch_expression(app: tauri::State<'_, App>, blob: String) -> CmdResult<String> {
    let image = commy_core::server::ImageRef::parse(&blob).ok_or("bad reference")?;
    let cached = server_image_path(&app, &image.blob).and_then(|p| std::fs::read(p).ok());
    let bytes = match cached {
        Some(bytes) => bytes,
        None => {
            let bytes = fetch_blob(&app, &image.blob, Some(&image.provider)).await?;
            if expressions::expression_mime(&bytes).is_none() {
                return Err("not a valid emoji or sound".into());
            }
            if let Some(path) = server_image_path(&app, &image.blob) {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&path, &bytes);
            }
            bytes
        }
    };
    let mime = expressions::expression_mime(&bytes).ok_or("not a valid emoji or sound")?;
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes)))
}

#[tauri::command]
async fn pick_sound() -> CmdResult<Option<(String, String)>> {
    let Some(picked) = rfd::AsyncFileDialog::new()
        .set_title("Choose a sound")
        .add_filter("Sounds", &["mp3", "ogg", "wav"])
        .pick_file()
        .await
    else {
        return Ok(None);
    };
    let bytes = tokio::fs::read(picked.path()).await.map_err(err_to_string)?;
    if bytes.len() > expressions::MAX_SOUND_BYTES {
        return Err("sounds must be under 1 MB".into());
    }
    let mime = expressions::audio_mime(&bytes).ok_or("sounds must be an MP3, OGG or WAV file")?;
    Ok(Some((
        picked.file_name().rsplit_once('.').map(|(stem, _)| stem.to_string()).unwrap_or_default(),
        format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes)),
    )))
}

const IROH_STARTUP_BEFORE_BLOB_RESTORE: std::time::Duration = std::time::Duration::from_secs(8);

async fn run_blob_restore(handle: tauri::AppHandle) {
    tokio::time::sleep(IROH_STARTUP_BEFORE_BLOB_RESTORE).await;
    let app = handle.state::<App>();
    let net = app.net.lock().await;
    if let Some(node) = net.as_ref() {
        restore_avatar_blob(&app, node).await;
        restore_server_images(&app, node).await;
    }
}

async fn restore_server_images(app: &tauri::State<'_, App>, node: &CommyNode) {
    let Some(dir) = app.data_dir().map(|d| d.join(SERVER_IMAGE_DIR)) else { return };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if let Ok(bytes) = std::fs::read(entry.path()) {
            let _ = node.add_blob(bytes).await;
        }
    }
}

const MAX_AVATAR_BYTES: usize = 512 * 1024;

#[tauri::command]
async fn set_avatar(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    kind: String,
    data: Option<String>,
) -> CmdResult<UserProfile> {
    let avatar = match kind.as_str() {
        "none" => None,
        "emoji" => {
            let emoji = data.unwrap_or_default().trim().to_string();
            if !commy_core::user::is_emoji_like(&emoji) {
                return Err("pick a single emoji".into());
            }
            Some(commy_core::Avatar::Emoji(emoji))
        }
        "image" => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data.unwrap_or_default())
                .map_err(|_| "image data is not valid base64".to_string())?;
            if bytes.len() > MAX_AVATAR_BYTES {
                return Err("avatar image is too large".into());
            }
            if !is_jpeg_or_png(&bytes) {
                return Err("avatar must be a JPEG or PNG image".into());
            }
            if let Some(dir) = app.data_dir() {
                std::fs::write(dir.join(AVATAR_FILE), &bytes).map_err(err_to_string)?;
            }
            let blob = {
                let net = app.net.lock().await;
                let node = net.as_ref().ok_or("network is not ready yet")?;
                node.add_blob(bytes).await.map_err(err_to_string)?
            };
            Some(commy_core::Avatar::Image { blob })
        }
        _ => return Err("unknown avatar kind".into()),
    };
    let profile = {
        let mut guard = app.profile.lock().unwrap();
        let profile = guard.as_mut().ok_or("set a username first")?;
        profile.avatar = avatar.map(|a| a.encode());
        profile.clone()
    };
    app.persist_profile();
    announce_profile(&app, &handle, &profile).await;
    let _ = handle.emit("profile-changed", &profile);
    entry_list::record_profile_edit(&app);
    entry_list::schedule(&handle);
    Ok(profile)
}

const AVATAR_FILE: &str = "avatar.img";

fn is_jpeg_or_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xff, 0xd8, 0xff]) || bytes.starts_with(&[0x89, b'P', b'N', b'G'])
}

async fn restore_avatar_blob(app: &tauri::State<'_, App>, node: &CommyNode) {
    let wants_image = app
        .profile
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|p| p.avatar.as_deref().and_then(commy_core::Avatar::parse))
        .is_some_and(|a| matches!(a, commy_core::Avatar::Image { .. }));
    if !wants_image {
        return;
    }
    let Some(path) = app.data_dir().map(|d| d.join(AVATAR_FILE)) else { return };
    match std::fs::read(&path) {
        Ok(bytes) => {
            if let Err(e) = node.add_blob(bytes).await {
                tracing::warn!("could not restore avatar blob: {e}");
            }
        }
        Err(e) => tracing::warn!("avatar image missing: {e}"),
    }
}

async fn announce_profile(app: &tauri::State<'_, App>, handle: &tauri::AppHandle, profile: &UserProfile) {
    let server_ids: Vec<ServerId> = {
        let mut servers = app.servers.lock().unwrap();
        for state in servers.values_mut() {
            let _ = state.apply(ServerEvent::ProfileUpdate { profile: profile.clone() });
        }
        servers.keys().copied().collect()
    };
    let announced = app.announced(profile.clone());
    for sid in server_ids {
        outbox::broadcast_local(app, sid, ServerEvent::ProfileUpdate { profile: announced.clone() }).await;
        app.persist_server(sid);
        let _ = handle.emit("server-event", &sid.to_string());
    }
}

async fn set_auto_presence(app: &tauri::State<'_, App>, handle: &tauri::AppHandle, presence: Presence, auto_idle: bool) {
    let profile = {
        let mut guard = app.profile.lock().unwrap();
        let Some(profile) = guard.as_mut() else { return };
        profile.presence = presence;
        profile.clone()
    };
    app.auto_idle.store(auto_idle, std::sync::atomic::Ordering::Relaxed);
    announce_profile(app, handle, &profile).await;
    let _ = handle.emit("profile-changed", &profile);
}

const ACTIVITY_POLL: std::time::Duration = std::time::Duration::from_secs(2);

async fn run_activity_watcher(handle: tauri::AppHandle) {
    let mut seen_last_poll: Option<String> = None;
    loop {
        tokio::time::sleep(ACTIVITY_POLL).await;
        let app = handle.state::<App>();
        let enabled = app.voice.lock().unwrap().share_activity;
        let (seen, pid) = tokio::task::spawn_blocking(|| {
            let pid = activity::foreground_pid();
            let seen = activity::foreground_app().and_then(|(path, desc)| activity::describe(&path, desc.as_deref()));
            (seen, pid)
        })
        .await
        .unwrap_or((None, None));
        if let (Some(text), Some(pid)) = (&seen, pid) {
            if let Some(game) = text.strip_prefix("Playing ") {
                *app.last_game.lock().unwrap() = Some((pid, game.to_string()));
            }
        }
        let seen = if enabled { seen } else { None };
        let stable = if !enabled || seen_last_poll == seen {
            seen.clone()
        } else {
            seen_last_poll = seen;
            continue;
        };
        seen_last_poll = stable.clone();
        let (stable, cover) = enrich_activity(&app, stable).await;
        let current = app.profile.lock().unwrap().as_ref().map(|p| (p.activity.clone(), p.activity_cover.clone()));
        let Some(current) = current else { continue };
        if current == (stable.clone(), cover.clone()) {
            continue;
        }
        let profile = {
            let mut guard = app.profile.lock().unwrap();
            let Some(profile) = guard.as_mut() else { continue };
            if profile.activity != stable {
                profile.activity_since = stable.as_ref().map(|_| chrono::Utc::now().timestamp());
            }
            profile.activity = stable;
            profile.activity_cover = cover;
            profile.clone()
        };
        if profile.presence != Presence::Invisible {
            announce_profile(&app, &handle, &profile).await;
        }
        let _ = handle.emit("profile-changed", &profile);
    }
}

async fn enrich_activity(app: &tauri::State<'_, App>, activity: Option<String>) -> (Option<String>, Option<String>) {
    let Some(text) = activity else { return (None, None) };
    let Some(name) = text.strip_prefix("Playing ") else { return (Some(text), None) };
    let creds = app.igdb_creds.lock().unwrap().clone();
    let hit = if creds.configured() {
        app.igdb.lock().await.resolve(&app.http, &creds, name).await
    } else {
        resolve_game_via_account(app, name).await
    };
    match hit {
        Some(game) => (Some(format!("Playing {}", game.name)), game.cover),
        None => (Some(text), None),
    }
}

async fn resolve_game_via_account(app: &tauri::State<'_, App>, name: &str) -> Option<igdb::GameHit> {
    let key = igdb::normalize(name);
    if let Some(hit) = app.igdb.lock().await.resolved.get(&key) {
        return hit.clone();
    }
    let hit = match account::server_search::<igdb::GameHit>(app, "games", name).await {
        Ok(Some(hits)) => igdb::best_match(name, &igdb::safe_hits(hits)).cloned(),
        Ok(None) => None,
        Err(_) => return None,
    };
    app.igdb.lock().await.resolved.insert(key, hit.clone());
    hit
}

#[derive(serde::Serialize)]
struct IntegrationsView {
    igdb_client_id: String,
    igdb_configured: bool,
}

#[tauri::command]
fn get_integrations(app: tauri::State<'_, App>) -> IntegrationsView {
    let creds = app.igdb_creds.lock().unwrap();
    IntegrationsView { igdb_client_id: creds.client_id.clone(), igdb_configured: creds.configured() }
}

#[tauri::command]
async fn set_igdb_credentials(
    app: tauri::State<'_, App>,
    client_id: String,
    client_secret: Option<String>,
) -> CmdResult<Vec<igdb::GameHit>> {
    let mut creds = app.igdb_creds.lock().unwrap().clone();
    creds.client_id = client_id.trim().to_string();
    if let Some(secret) = client_secret.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        creds.client_secret = secret;
    }
    if creds.client_id.is_empty() {
        *app.igdb_creds.lock().unwrap() = igdb::IgdbCredentials::default();
        app.igdb.lock().await.reset();
        if let Some(dir) = app.data_dir() {
            let _ = persist::save_integrations(dir, &igdb::IgdbCredentials::default());
        }
        return Ok(Vec::new());
    }
    if !creds.configured() {
        return Err("enter both the client id and the client secret".into());
    }
    let sample = {
        let mut client = app.igdb.lock().await;
        client.reset();
        client.search(&app.http, &creds, "Portal 2").await?
    };
    *app.igdb_creds.lock().unwrap() = creds.clone();
    if let Some(dir) = app.data_dir() {
        persist::save_integrations(dir, &creds).map_err(err_to_string)?;
    }
    Ok(sample)
}

#[tauri::command]
async fn search_games(app: tauri::State<'_, App>, query: String) -> CmdResult<Vec<igdb::GameHit>> {
    let creds = app.igdb_creds.lock().unwrap().clone();
    if !creds.configured() {
        if query.trim().chars().count() < 2 {
            return Ok(Vec::new());
        }
        let hits = account::server_search::<igdb::GameHit>(&app, "games", query.trim()).await?;
        return Ok(igdb::safe_hits(hits.unwrap_or_default()));
    }
    app.igdb.lock().await.search(&app.http, &creds, &query).await
}

#[derive(serde::Serialize, serde::Deserialize)]
struct GifHit {
    preview: String,
    url: String,
    #[serde(default)]
    ratio: f64,
    #[serde(default)]
    title: String,
}

#[tauri::command]
async fn gif_search(app: tauri::State<'_, App>, query: String) -> CmdResult<Option<Vec<GifHit>>> {
    let query: String = query.trim().chars().take(100).collect();
    let hits = account::server_search::<GifHit>(&app, "gifs", &query).await?;
    Ok(hits.map(|hits| {
        hits.into_iter()
            .filter(|g| g.preview.starts_with("https://") && g.url.starts_with("https://"))
            .map(|g| GifHit { title: safety::truncate(&g.title, 200), ..g })
            .collect()
    }))
}

#[tauri::command]
async fn set_share_activity(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    enabled: bool,
) -> CmdResult<VoiceSettings> {
    let settings = {
        let mut voice = app.voice.lock().unwrap();
        voice.share_activity = enabled;
        voice.clone()
    };
    app.persist_voice();
    if !enabled {
        let profile = {
            let mut guard = app.profile.lock().unwrap();
            guard.as_mut().filter(|p| p.activity.is_some()).map(|p| {
                p.activity = None;
                p.activity_cover = None;
                p.activity_since = None;
                p.clone()
            })
        };
        if let Some(profile) = profile {
            announce_profile(&app, &handle, &profile).await;
            let _ = handle.emit("profile-changed", &profile);
        }
    }
    Ok(settings)
}

#[derive(serde::Serialize)]
struct AudioDevices {
    inputs: Vec<String>,
    outputs: Vec<String>,
}

#[tauri::command]
async fn list_audio_devices() -> CmdResult<AudioDevices> {
    tokio::task::spawn_blocking(|| AudioDevices {
        inputs: commy_media::list_device_names(true),
        outputs: commy_media::list_device_names(false),
    })
    .await
    .map_err(err_to_string)
}

#[tauri::command]
async fn set_audio_devices(
    app: tauri::State<'_, App>,
    input: Option<String>,
    output: Option<String>,
) -> CmdResult<VoiceSettings> {
    let clean = |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let settings = {
        let mut voice = app.voice.lock().unwrap();
        voice.input_device = clean(input);
        voice.output_device = clean(output);
        voice.clone()
    };
    app.persist_voice();
    if let Some(session) = app.voice_session.lock().unwrap().as_mut() {
        session.switch_devices(
            app.processor.clone(),
            settings.input_device.clone(),
            settings.output_device.clone(),
        );
    }
    let restart_test = app.mic_test.lock().unwrap().take().is_some();
    if restart_test {
        let test = MicTest::start(
            app.processor.clone(),
            settings.input_device.clone(),
            settings.output_device.clone(),
        )
        .map_err(err_to_string)?;
        *app.mic_test.lock().unwrap() = Some(test);
    }
    Ok(settings)
}

#[tauri::command]
fn set_afk_timeout(app: tauri::State<App>, minutes: u32) -> VoiceSettings {
    let settings = {
        let mut voice = app.voice.lock().unwrap();
        voice.afk_timeout_minutes = afk::clamp_timeout_minutes(minutes);
        voice.clone()
    };
    app.persist_voice();
    settings
}

#[tauri::command]
fn set_volumes(app: tauri::State<App>, output_volume: f32, input_gain: f32) -> VoiceSettings {
    let mut voice = app.voice.lock().unwrap();
    voice.output_volume = output_volume.clamp(0.0, 2.0);
    voice.input_gain = input_gain.clamp(0.0, 2.0);
    let snapshot = voice.clone();
    drop(voice);
    app.processor
        .lock()
        .unwrap()
        .set_input_gain(snapshot.input_gain);
    if let Some(session) = app.voice_session.lock().unwrap().as_ref() {
        session
            .mixer
            .lock()
            .unwrap()
            .set_master(snapshot.output_volume);
    }
    app.persist_voice();
    snapshot
}

#[tauri::command]
async fn create_role(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    name: String,
    color: Option<String>,
    permissions: Vec<String>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let name = name.trim().to_string();
    if name.is_empty() || name.len() > 50 {
        return Err("role name must be 1 to 50 characters".into());
    }
    let color = color
        .map(|c| c.trim().trim_start_matches('#').to_lowercase())
        .filter(|c| c.len() == 6 && c.chars().all(|ch| ch.is_ascii_hexdigit()));
    let me = current_profile(&app)?;
    let role = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_ROLES, "manage roles")?;
        let position = state
            .server
            .roles
            .iter()
            .map(|r| r.position)
            .max()
            .unwrap_or(0)
            + 1;
        Role {
            id: RoleId::new(),
            name,
            color,
            permissions: permissions_from_names(&permissions),
            position,
            hoist: false,
            self_assignable: false,
            tag: None,
            banner: None,
        }
    };
    apply_and_broadcast(&app, &handle, sid, ServerEvent::RoleCreate { server: sid, role }).await
}

#[tauri::command]
async fn delete_role(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    role_id: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let rid: RoleId = role_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_ROLES, "manage roles")?;
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::RoleDelete { server: sid, role: rid })
        .await
}

#[tauri::command]
async fn set_member_role(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    user_id: String,
    role_id: String,
    grant: bool,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let uid: UserId = user_id.parse().map_err(err_to_string)?;
    let rid: RoleId = role_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_ROLES, "manage roles")?;
    }
    let event = if grant {
        ServerEvent::RoleAssign { server: sid, user: uid, role: rid }
    } else {
        ServerEvent::RoleUnassign { server: sid, user: uid, role: rid }
    };
    apply_and_broadcast(&app, &handle, sid, event).await
}

#[tauri::command]
async fn start_mic_test(app: tauri::State<'_, App>) -> CmdResult<()> {
    let test = {
        if app.mic_test.lock().unwrap().is_some() {
            return Ok(());
        }
        {
            let voice = app.voice.lock().unwrap().clone();
            MicTest::start(app.processor.clone(), voice.input_device, voice.output_device)
        }.map_err(err_to_string)?
    };
    *app.mic_test.lock().unwrap() = Some(test);
    Ok(())
}

#[tauri::command]
async fn stop_mic_test(app: tauri::State<'_, App>) -> CmdResult<()> {
    *app.mic_test.lock().unwrap() = None;
    Ok(())
}

const MAX_RESPONSIVE_PREVIEW_BYTES: usize = 20 * 1024 * 1024;

fn is_data_url_safe_image_mime(peer_supplied: &str) -> bool {
    let subtype = peer_supplied.strip_prefix("image/").unwrap_or("");
    !subtype.is_empty() && subtype.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'))
}

#[tauri::command]
async fn get_attachment_preview(
    app: tauri::State<'_, App>,
    blob: String,
    provider: Option<String>,
    mime: String,
) -> CmdResult<String> {
    if !is_data_url_safe_image_mime(&mime) {
        return Err("not an image".into());
    }
    let bytes = fetch_blob(&app, &blob, provider.as_deref()).await?;
    if bytes.len() > MAX_RESPONSIVE_PREVIEW_BYTES {
        return Err("image too large to preview".into());
    }
    Ok(format!(
        "data:{};base64,{}",
        mime,
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}

#[tauri::command]
fn get_voice_settings(app: tauri::State<App>) -> VoiceSettings {
    app.voice.lock().unwrap().clone()
}

#[tauri::command]
fn set_noise_cancellation(app: tauri::State<App>, enabled: bool) -> VoiceSettings {
    let mut voice = app.voice.lock().unwrap();
    voice.noise_cancellation = enabled;
    app.processor
        .lock()
        .unwrap()
        .set_noise_cancellation(enabled);
    let snapshot = voice.clone();
    drop(voice);
    app.persist_voice();
    snapshot
}

#[tauri::command]
async fn set_muted(app: tauri::State<'_, App>, handle: tauri::AppHandle, muted: bool) -> CmdResult<VoiceSettings> {
    let snapshot = {
        let mut voice = app.voice.lock().unwrap();
        voice.muted = muted;
        voice.clone()
    };
    apply_voice_gates(&app);
    app.persist_voice();
    announce_voice_state(&app, &handle).await;
    Ok(snapshot)
}

async fn announce_voice_state(app: &tauri::State<'_, App>, handle: &tauri::AppHandle) {
    let Some((sid, channel)) = app.voice_session.lock().unwrap().as_ref().map(|s| (s.server, s.channel)) else { return };
    let Ok(profile) = current_profile(app) else { return };
    let voice = app.voice.lock().unwrap().clone();
    let event = {
        let mut servers = app.servers.lock().unwrap();
        let Some(state) = servers.get_mut(&sid) else { return };
        let since = state.voice_since(profile.id).unwrap_or_else(|| chrono::Utc::now().timestamp());
        let event = ServerEvent::VoiceState {
            user: profile.id,
            channel: Some(channel),
            muted: voice.muted,
            deafened: voice.deafened,
            since: Some(since),
        };
        let _ = state.apply(event.clone());
        event
    };
    broadcast(app, sid, WirePayload::Event(event)).await;
    let _ = handle.emit("server-event", &sid.to_string());
}

fn apply_voice_gates(app: &App) {
    let voice = app.voice.lock().unwrap().clone();
    let forced_mute = app.forced_mute.load(std::sync::atomic::Ordering::Relaxed);
    let forced_deaf = app.forced_deaf.load(std::sync::atomic::Ordering::Relaxed);
    let deaf = voice.deafened || forced_deaf;
    app.processor.lock().unwrap().set_muted(voice.muted || deaf || forced_mute);
    if let Some(session) = app.voice_session.lock().unwrap().as_ref() {
        session.deafened.store(deaf, std::sync::atomic::Ordering::Relaxed);
    }
}

fn refresh_forced_voice(app: &tauri::State<'_, App>, handle: &tauri::AppHandle) {
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let server = app.voice_session.lock().unwrap().as_ref().map(|s| s.server);
    let (muted, deafened) = match (me, server) {
        (Some(me), Some(sid)) => app.servers.lock().unwrap().get(&sid).map_or((false, false), |s| {
            (s.server.server_muted.contains(&me), s.server.server_deafened.contains(&me))
        }),
        _ => (false, false),
    };
    let was = (
        app.forced_mute.swap(muted, std::sync::atomic::Ordering::Relaxed),
        app.forced_deaf.swap(deafened, std::sync::atomic::Ordering::Relaxed),
    );
    apply_voice_gates(app);
    if was != (muted, deafened) {
        let _ = handle.emit("voice-moderated", serde_json::json!({ "muted": muted, "deafened": deafened }));
    }
}

#[tauri::command]
async fn set_deafened(app: tauri::State<'_, App>, handle: tauri::AppHandle, deafened: bool) -> CmdResult<VoiceSettings> {
    let snapshot = {
        let mut voice = app.voice.lock().unwrap();
        voice.deafened = deafened;
        if deafened {
            voice.muted = true;
        }
        voice.clone()
    };
    apply_voice_gates(&app);
    app.persist_voice();
    announce_voice_state(&app, &handle).await;
    Ok(snapshot)
}

async fn broadcast(app: &App, server: ServerId, payload: WirePayload) {
    let net = app.net.lock().await;
    if let Some(node) = net.as_ref() {
        if let Err(e) = node.broadcast(server, payload).await {
            tracing::warn!("broadcast failed: {e}");
        }
    }
}

async fn run_net_loop(
    handle: tauri::AppHandle,
    mut events: tokio::sync::mpsc::UnboundedReceiver<NetEvent>,
) {
    while let Some(event) = events.recv().await {
        let app = handle.state::<App>();
        match event {
            NetEvent::Event { server, event, .. } => {
                if let ServerEvent::SoundboardPlay { channel, user, name, .. } = &event {
                    on_remote_soundboard(&app, &handle, server, *channel, *user, name);
                    continue;
                }
                if matches!(event, ServerEvent::DoorbellSet { .. }) {
                    doorbell::on_announce(&app, server, &event);
                    continue;
                }
                if let ServerEvent::ScreenShare { user, active: true, .. } = &event {
                    app.share_seen.lock().unwrap().insert((server, *user), std::time::Instant::now());
                }
                let moderation = matches!(event, ServerEvent::VoiceModerate { .. });
                let banned_me = match &event {
                    ServerEvent::MemberBan { user, .. } => app.profile.lock().unwrap().as_ref().is_some_and(|p| p.id == *user),
                    _ => false,
                };
                let voice_update = match &event {
                    ServerEvent::VoiceState { user, channel, .. } => Some((*user, *channel)),
                    _ => None,
                };
                let holdable = catchup::holdable(&event).then(|| event.clone());
                let presence = outbox::online_user(&event);
                let (applied, was_online, new_message, pinned) = {
                    let mut servers = app.servers.lock().unwrap();
                    match servers.get_mut(&server) {
                        Some(state) => {
                            let was_online = presence.map(|(user, _)| {
                                state.profiles.get(&user).is_some_and(|p| p.presence.is_visible_online())
                            });
                            let (applied, new_message, pinned) = catchup::apply_live(state, event);
                            (applied, was_online, new_message, pinned)
                        }
                        None => continue,
                    }
                };
                if applied.is_ok() {
                    if let Some(user) = pinned {
                        catchup::release_held(&handle, server, &[user]);
                    }
                    if let (Some((user, now)), Some(false)) = (presence, was_online) {
                        if now.is_visible_online() {
                            outbox::wake_dm(&app, user);
                        }
                    }
                }
                match applied {
                    Ok(true) => {
                        if banned_me {
                            on_banned(&app, &handle, server).await;
                            continue;
                        }
                        app.persist_server(server);
                        if moderation {
                            refresh_forced_voice(&app, &handle);
                        }
                        if let Some((user, channel)) = voice_update {
                            react_to_voice_change(&app, &handle, server, user, channel);
                        }
                        if let Some(message) = new_message {
                            doorbell::arrived(&handle, server, message.id);
                            notify_new_message(&app, &handle, server, &message);
                        }
                        let _ = handle.emit("server-event", &server.to_string());
                    }
                    Ok(false) => {}
                    Err(e) => match (e, holdable) {
                        (StoreError::UnknownAuthorKey(signer), Some(event)) => catchup::hold(&app, server, signer, event),
                        (e, _) => tracing::warn!("rejected remote event: {e}"),
                    },
                }
            }
            NetEvent::SyncRequested { server, .. } => {
                for payload in catchup::sync_replies(&app, server) {
                    broadcast(&app, server, payload).await;
                }
            }
            NetEvent::SyncReceived { server, payload } => {
                catchup::apply_snapshot(&handle, server, payload);
            }
            NetEvent::VoiceDial { .. } => {}
            NetEvent::NeighborUp { server, peer } => {
                let me = app.profile.lock().unwrap().clone();
                let Some(me) = me else { continue };
                let my_id = me.id;
                let needs_sync = app.awaits_snapshot(server);
                broadcast(
                    &app,
                    server,
                    WirePayload::Event(ServerEvent::MemberJoin { server, profile: app.announced(me) }),
                )
                .await;
                if needs_sync {
                    broadcast(&app, server, WirePayload::SyncRequest).await;
                }
                for event in structure_announcements(&app, server, my_id) {
                    broadcast(&app, server, WirePayload::Event(event)).await;
                }
                if app.voice_session.lock().unwrap().as_ref().is_some_and(|s| s.server == server) {
                    announce_voice_state(&app, &handle).await;
                }
                outbox::flush_queued(&app, server).await;
                outbox::wake_dms_for_node(&app, peer);
                catchup::on_neighbor_up(&handle, server, peer);
                doorbell::announce(&handle, server);
            }
            NetEvent::CaughtUp { server, peer, report } => {
                catchup::on_caught_up(&handle, server, peer, &report);
            }
            NetEvent::Wake { from } => wake::handle_wake(&handle, from),
        }
    }
}

fn react_to_voice_change(
    app: &tauri::State<'_, App>,
    handle: &tauri::AppHandle,
    server: ServerId,
    user: UserId,
    channel: Option<ChannelId>,
) {
    let session_channel = {
        let session = app.voice_session.lock().unwrap();
        match session.as_ref() {
            Some(s) if s.server == server => Some(s.channel),
            _ => None,
        }
    };
    let Some(our_channel) = session_channel else { return };

    match channel {
        Some(ch) if ch == our_channel => {
            let (me, our_hex) = {
                let profile = app.profile.lock().unwrap().clone();
                let hex = app.node_hex.lock().unwrap().clone();
                match (profile, hex) {
                    (Some(p), Some(h)) => (p, h),
                    _ => return,
                }
            };
            let their_hex = {
                let servers = app.servers.lock().unwrap();
                servers
                    .get(&server)
                    .and_then(|s| s.profiles.get(&user))
                    .and_then(|p| p.node.clone())
            };
            let Some(their_hex) = their_hex else { return };
            let already = app
                .voice_session
                .lock()
                .unwrap()
                .as_ref()
                .map(|s| s.has_peer(user))
                .unwrap_or(false);
            if !already && should_dial(&our_hex, &their_hex) {
                dial_voice_peer(handle, user, their_hex, our_channel, me.id);
            }
        }
        _ => {
            if let Some(session) = app.voice_session.lock().unwrap().as_mut() {
                session.detach_peer(user);
            }
        }
    }
}

fn notify_new_message(app: &tauri::State<'_, App>, handle: &tauri::AppHandle, sid: ServerId, message: &Message) {
    let payload = {
        let servers = app.servers.lock().unwrap();
        let Some(state) = servers.get(&sid) else { return };
        serde_json::json!({
            "server": sid.to_string(),
            "server_name": state.server.name,
            "channel": message.channel.to_string(),
            "channel_name": state.server.channel(message.channel).map(|c| c.name.clone()).unwrap_or_default(),
            "announcement": state.server.channel(message.channel).is_some_and(|c| c.kind == ChannelKind::Announcement),
            "author": message.author.to_string(),
            "author_name": state.profiles.get(&message.author).map(|p| p.visible_name().to_string()).unwrap_or_else(|| "Someone".into()),
            "content": message.content,
            "has_attachments": !message.attachments.is_empty(),
        })
    };
    let _ = handle.emit("message-created", payload);
}

#[tauri::command]
fn notify(handle: tauri::AppHandle, title: String, body: String, target: Option<String>) -> CmdResult<()> {
    let clip = |s: String, max: usize| s.chars().take(max).collect::<String>();
    let target = target.filter(|t| toast::valid_target(t));
    toast::show(&handle, &clip(title, 80), &clip(body, 240), target)
}

#[tauri::command]
fn set_unread(handle: tauri::AppHandle, count: u32) {
    tray::set_unread(&handle, count);
}

#[tauri::command]
fn set_close_to_tray(app: tauri::State<App>, enabled: bool) -> VoiceSettings {
    let settings = {
        let mut voice = app.voice.lock().unwrap();
        voice.close_to_tray = enabled;
        voice.clone()
    };
    app.persist_voice();
    settings
}

#[tauri::command]
fn set_auto_update(app: tauri::State<App>, handle: tauri::AppHandle, enabled: bool) -> VoiceSettings {
    let settings = {
        let mut voice = app.voice.lock().unwrap();
        voice.auto_update = enabled;
        voice.clone()
    };
    app.persist_voice();
    if enabled {
        tauri::async_runtime::spawn(async move {
            updater::check(&handle).await;
        });
    }
    settings
}

#[tauri::command]
fn get_update_status(handle: tauri::AppHandle) -> updater::UpdateStatus {
    updater::status(&handle)
}

#[tauri::command]
async fn check_for_updates(handle: tauri::AppHandle) -> CmdResult<updater::UpdateStatus> {
    Ok(updater::check(&handle).await)
}

#[tauri::command]
fn restart_to_update(handle: tauri::AppHandle) -> CmdResult<()> {
    updater::restart(&handle)
}

async fn run_dm_loop(
    handle: tauri::AppHandle,
    mut incoming: tokio::sync::mpsc::UnboundedReceiver<commy_net::node::IncomingDm>,
) {
    while let Some(dm) = incoming.recv().await {
        let app = handle.state::<App>();
        match outbox::receive_dm(&handle, dm) {
            outbox::Received::Report { from, content } => receive_report(&app, &handle, from, &content),
            outbox::Received::Reply { to, signal } => {
                let _ = send_friend_signal(&app, to, signal).await;
            }
            outbox::Received::Dropped | outbox::Received::Taken => {}
        }
    }
}

async fn run_media_loop(
    handle: tauri::AppHandle,
    mut channels: tokio::sync::mpsc::UnboundedReceiver<MediaChannel>,
) {
    while let Some(media) = channels.recv().await {
        let app = handle.state::<App>();
        match media.hello.clone() {
            MediaHello::Voice { user, channel } => {
                let mut session = app.voice_session.lock().unwrap();
                match session.as_mut() {
                    Some(s) if s.channel == channel => s.attach_peer(user, media),
                    _ => tracing::debug!("voice call for a channel we are not in"),
                }
            }
            MediaHello::CameraSubscribe { user, channel } => {
                let mut camera = app.camera.lock().unwrap();
                match camera.as_mut() {
                    Some(c) if c.channel == channel => {
                        c.add_subscriber(media.outgoing.clone());
                        tracing::info!("viewer {} subscribed to our camera", user);
                    }
                    _ => tracing::debug!("camera subscribe while camera is off"),
                }
            }
            MediaHello::ScreenSubscribe { user, channel } => {
                let share = app.screen_share.lock().unwrap();
                match share.as_ref() {
                    Some(s) if s.channel == channel => {
                        s.add_subscriber(media.outgoing.clone());
                        tracing::info!("viewer {} subscribed to our screen", user);
                    }
                    _ => tracing::debug!("screen subscribe while not sharing"),
                }
            }
        }
    }
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,iroh=warn,iroh_net=warn".into()),
        )
        .init();

    instance::apply_test_build_data_dir();

    let launch_link = invite_link::link_from_args();
    if let (Some(link), Some(dir)) = (&launch_link, invite_link::default_data_dir()) {
        if invite_link::forward_to_running_instance(&dir, link) {
            return;
        }
    }

    instance::apply_webview_data_dir_before_any_window();

    tauri::Builder::default()
        .manage(App::default())
        .manage(updater::Updates::default())
        .register_asynchronous_uri_scheme_protocol("commy-media", |ctx, request, responder| {
            let handle = ctx.app_handle().clone();
            let uri = request.uri().to_string();
            tauri::async_runtime::spawn(async move {
                let app = handle.state::<App>();
                responder.respond(account::media_image(&app, &uri).await);
            });
        })
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|handle, shortcut, event| {
                    if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        on_overlay_hotkey(handle, shortcut.id());
                    }
                })
                .build(),
        )
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() != "main" {
                    return;
                }
                let to_tray = window.state::<App>().voice.lock().unwrap().close_to_tray;
                if to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(|tauri_app| {
            let handle = tauri_app.handle().clone();
            let label = std::env::var("COMMY_INSTANCE_LABEL").ok();
            if let Some(title) = instance::window_title(option_env!("COMMY_TEST_BUILD").is_some(), label.as_deref()) {
                if let Some(window) = tauri_app.get_webview_window("main") {
                    let _ = window.set_title(&title);
                }
            }
            {
                let handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let settings = handle.state::<App>().voice.lock().unwrap().overlay.clone();
                    register_overlay_hotkeys(&handle, &settings);
                    overlay::apply(&handle, &settings, true);
                });
            }
            updater::init(&handle);
            updater::start(&handle);
            if let Err(e) = tray::build(&handle) {
                tracing::warn!("tray icon unavailable: {e}");
            }
            let toast_icon = tauri_app.path().app_data_dir().ok().and_then(|dir| {
                std::fs::create_dir_all(&dir).ok()?;
                let path = dir.join("notification-icon.png");
                std::fs::write(&path, tray::ICON_PNG).ok()?;
                Some(path)
            });
            toast::register(toast_icon.as_deref());
            toast::ensure_start_menu_shortcut();
            let app = tauri_app.state::<App>();

            let data_dir = std::env::var("COMMY_DATA_DIR")
                .ok()
                .map(std::path::PathBuf::from)
                .or_else(|| tauri_app.path().app_data_dir().ok());

            if let Some(dir) = data_dir {
                let _ = app.data_dir.set(dir.clone());
                invite_link::register_scheme();
                invite_link::start(&handle, &dir, launch_link);
                if let Some(mut profile) = persist::load_profile(&dir) {
                    profile.activity = None;
                    profile.activity_cover = None;
                    profile.activity_since = None;
                    *app.profile.lock().unwrap() = Some(profile);
                }
                if let Some(dms) = persist::load_dms(&dir) {
                    *app.dms.lock().unwrap() = dms;
                }
                *app.account.lock().unwrap() = account::load(&dir);
                if let Some(creds) = persist::load_integrations(&dir) {
                    *app.igdb_creds.lock().unwrap() = creds;
                }
                match persist::load_safety(&dir) {
                    Some(safety) => *app.safety.lock().unwrap() = safety,
                    None => {
                        let partners = app.dms.lock().unwrap().partners();
                        app.safety.lock().unwrap().accepted.extend(partners);
                    }
                }
                if let Some(voice) = persist::load_voice(&dir) {
                    let mut processor = app.processor.lock().unwrap();
                    processor.set_noise_cancellation(voice.noise_cancellation);
                    processor.set_muted(voice.muted);
                    processor.set_input_gain(voice.input_gain);
                    drop(processor);
                    *app.voice.lock().unwrap() = voice;
                }
                for state in persist::load_servers(&dir) {
                    app.servers.lock().unwrap().insert(state.server.id, state);
                }
                let mut awaiting = persist::load_awaiting(&dir);
                awaiting.retain(|id| app.servers.lock().unwrap().contains_key(id));
                *app.awaiting_snapshot.lock().unwrap() = awaiting;
                let mut queue = outbox::Outbox::load(&dir);
                let now = chrono::Utc::now();
                let pruned = app.servers.lock().unwrap().values().fold(false, |acc, s| queue.prune(s, now) | acc);
                *app.outbox.lock().unwrap() = queue;
                if pruned {
                    app.persist_outbox();
                }
                let mut bell = doorbell::Doorbell::load(&dir);
                bell.retain_servers(|id| app.servers.lock().unwrap().contains_key(id));
                *app.doorbell.lock().unwrap() = bell;
            }
            let signed_in = account::is_signed_in(&app.account.lock().unwrap());
            let key_file = app.data_dir().map(|dir| dir.join(commy_core::identity::IDENTITY_FILE));
            if signed_in && key_file.as_ref().is_some_and(|path| !path.exists()) {
                app.mark_key_unsettled();
            }
            if let Err(e) = app.reload_identity() {
                tracing::error!("identity key unavailable: {e}");
            }
            if app.hold_unsettled_key(signed_in, KEY_SETTLE_MAX) {
                let handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(KEY_SETTLE_MAX).await;
                    settle::settle_key(&handle).await;
                });
            }

            let node_key = app
                .data_dir()
                .and_then(|dir| persist::load_or_create_node_key(dir));

            tauri::async_runtime::spawn(async move {
                match CommyNode::spawn(node_key).await {
                    Ok((mut node, events, media, dms)) => {
                        let app = handle.state::<App>();
                        let hex = node.node_id_hex();
                        if let Some(asks) = node.take_entry_key_requests() {
                            tauri::async_runtime::spawn(entry_list::run_entry_key_requests(handle.clone(), asks));
                        }

                        let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
                        let stored_invites = app
                            .data_dir()
                            .map(|d| persist::load_invites(d))
                            .unwrap_or_default();
                        catchup::mark_joined_placeholders(&app, &stored_invites, node.node_id_bytes());
                        let awaiting = app.awaiting_snapshot.lock().unwrap().clone();
                        let to_restore: Vec<(ServerId, String, bool)> = {
                            let servers = app.servers.lock().unwrap();
                            servers
                                .values()
                                .map(|s| {
                                    let id = s.server.id;
                                    let rejoin = awaiting.contains(&id) && stored_invites.contains_key(&id);
                                    (id, s.server.name.clone(), Some(s.server.owner) == me && !rejoin)
                                })
                                .collect()
                        };
                        for (id, name, owned) in to_restore {
                            if owned {
                                match node.host_server(id, &name).await {
                                    Ok(ticket) => {
                                        app.invites.lock().unwrap().insert(id, ticket.encode());
                                    }
                                    Err(e) => tracing::warn!("re-host failed for {name}: {e}"),
                                }
                            } else if let Some(raw) = stored_invites.get(&id) {
                                match ServerTicket::decode(raw) {
                                    Ok(ticket) => {
                                        if let Err(e) = node.join_server(&ticket).await {
                                            tracing::warn!("rejoin failed for {name}: {e}");
                                        }
                                        app.invites.lock().unwrap().insert(id, raw.clone());
                                    }
                                    Err(e) => tracing::warn!("stored invite corrupt for {name}: {e}"),
                                }
                            }
                        }
                        if let Some(dir) = app.data_dir() {
                            let _ = persist::save_invites(dir, &app.invites.lock().unwrap().clone());
                        }

                        let missing_afk: Vec<ServerId> = {
                            let servers = app.servers.lock().unwrap();
                            servers
                                .values()
                                .filter(|s| {
                                    Some(s.server.owner) == me
                                        && s.server.afk_channel().is_none()
                                })
                                .map(|s| s.server.id)
                                .collect()
                        };
                        for sid in missing_afk {
                            let channel = {
                                let servers = app.servers.lock().unwrap();
                                let position = servers
                                    .get(&sid)
                                    .map(|s| s.server.channels.len() as u32)
                                    .unwrap_or(0);
                                Channel::new("AFK", ChannelKind::Voice, position)
                            };
                            let event = ServerEvent::ChannelCreate { server: sid, channel };
                            {
                                let mut servers = app.servers.lock().unwrap();
                                if let Some(state) = servers.get_mut(&sid) {
                                    let _ = state.apply(event.clone());
                                }
                            }
                            if let Err(e) = node.broadcast(sid, WirePayload::Event(event)).await {
                                tracing::debug!("afk channel announce pending: {e}");
                            }
                            app.persist_server(sid);
                        }

                        *app.node_hex.lock().unwrap() = Some(hex.clone());
                        let me = {
                            let mut profile = app.profile.lock().unwrap();
                            if let Some(p) = profile.as_mut() {
                                p.node = Some(hex);
                            }
                            profile.clone()
                        };
                        app.persist_profile();
                        if let Some(me) = me {
                            let mut servers = app.servers.lock().unwrap();
                            for state in servers.values_mut() {
                                let sid = state.server.id;
                                let _ = state.apply(ServerEvent::MemberJoin {
                                    server: sid,
                                    profile: me.clone(),
                                });
                            }
                        }
                        node.set_catchup_store(catchup::store(&handle));
                        *app.net.lock().await = Some(node);
                        tracing::info!("iroh node online");

                        tauri::async_runtime::spawn(run_media_loop(handle.clone(), media));
                        tauri::async_runtime::spawn(run_dm_loop(handle.clone(), dms));
                        tauri::async_runtime::spawn(run_afk_watchdog(handle.clone()));
                        tauri::async_runtime::spawn(run_status_expiry(handle.clone()));
                        tauri::async_runtime::spawn(run_event_reminders(handle.clone()));
                        {
                            let handle = handle.clone();
                            tauri::async_runtime::spawn(async move {
                                account::verify_saved_session(handle.clone()).await;
                                entry_list::schedule(&handle);
                            });
                        }
                        tauri::async_runtime::spawn(run_share_heartbeat(handle.clone()));
                        tauri::async_runtime::spawn(run_activity_watcher(handle.clone()));
                        tauri::async_runtime::spawn(run_retention_pruner(handle.clone()));
                        tauri::async_runtime::spawn(run_mesh_healer(handle.clone()));
                        tauri::async_runtime::spawn(wake::run_watchdog(handle.clone()));
                        tauri::async_runtime::spawn(run_blob_restore(handle.clone()));
                        tauri::async_runtime::spawn(catchup::run_periodic(handle.clone()));
                        tauri::async_runtime::spawn(outbox::run_dm_outbox(handle.clone()));
                        tauri::async_runtime::spawn(doorbell::run(handle.clone()));
                        run_net_loop(handle.clone(), events).await;
                    }
                    Err(e) => tracing::error!("P2P stack failed to start: {e}"),
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            doorbell::get_awaited,
            account::account_status,
            account::set_account_server,
            account::account_begin_sign_in,
            account::account_cancel_sign_in,
            account::account_sign_out,
            voice_stats,
            set_server_afk_timeout,
            list_friends,
            friend_action,
            set_about,
            moderate_voice,
            ban_member,
            unban_member,
            set_user_voice,
            set_role_banner,
            get_integrations,
            set_igdb_credentials,
            search_games,
            gif_search,
            get_overlay_settings,
            overlay_hotkey_problems,
            set_overlay_settings,
            set_game_tags,
            set_rules,
            accept_rules,
            save_event,
            delete_event,
            set_rsvp,
            move_channels,
            create_category,
            set_channel_limit,
            update_role,
            set_own_role,
            get_safety,
            set_status_expiry,
            set_blocked,
            set_dm_settings,
            accept_dm_request,
            decline_dm_request,
            report_user,
            list_reports,
            dismiss_report,
            add_emoji,
            remove_emoji,
            add_sound,
            remove_sound,
            set_sound_emoji,
            set_expression_settings,
            set_server_public,
            play_soundboard,
            fetch_expression,
            pick_sound,
            list_shareable_emojis,
            ui_ready,
            init_user,
            get_user,
            create_server,
            join_server,
            list_servers,
            get_server,
            get_invite,
            invite_link::resolve_invite_link,
            invite_link::create_invite_link,
            invite_link::account_server_info,
            invite_link::take_pending_invite_link,
            entry_list::entry_list_status,
            entry_list::entry_list_sync,
            entry_list::entry_list_recovery_code,
            entry_list::entry_list_use_recovery_code,
            entry_list::entry_list_start_over,
            entry_list::answer_entry_key_request,
            get_messages,
            send_message,
            attach_file,
            attach_bytes,
            open_url,
            note_user_activity,
            download_attachment,
            set_voice_channel,
            list_share_sources,
            set_afk_timeout,
            set_afk_channel,
            set_avatar,
            set_share_activity,
            update_channel,
            set_channel_banner,
            set_channel_color,
            set_channel_kind,
            pin_message,
            get_pins,
            edit_message,
            delete_message,
            list_audio_devices,
            set_audio_devices,
            start_camera,
            stop_camera,
            push_camera_frame,
            watch_camera,
            stop_watching_camera,
            notify,
            set_unread,
            set_close_to_tray,
            set_auto_update,
            get_update_status,
            check_for_updates,
            restart_to_update,
            set_server_image,
            fetch_server_image,
            set_reaction,
            pick_image,
            share_source_preview,
            own_share_frame,
            set_volumes,
            create_role,
            delete_role,
            set_member_role,
            list_dm_threads,
            get_dm_messages,
            open_dm,
            send_dm,
            rename_server,
            create_channel,
            delete_channel,
            kick_member,
            leave_server,
            update_profile,
            start_mic_test,
            stop_mic_test,
            get_attachment_preview,
            start_screen_share,
            stop_screen_share,
            watch_screen,
            stop_watching,
            get_voice_settings,
            set_noise_cancellation,
            set_muted,
            set_deafened,
        ])
        .build(tauri::generate_context!())
        .expect("failed to run Commy")
        .run(|handle, event| {
            if let tauri::RunEvent::Exit = event {
                catchup::save_unsaved(&handle.state::<App>());
            }
            invite_link::on_run_event(handle, &event)
        });
}

#[cfg(test)]
mod share_tests {
    use super::*;

    #[test]
    fn parses_monitor_and_window_sources() {
        assert_eq!(parse_source(Some("monitor"), Some(3)), Some((SourceKind::Monitor, 3)));
        assert_eq!(parse_source(Some("window"), Some(42)), Some((SourceKind::Window, 42)));
    }

    #[test]
    fn missing_or_unknown_source_means_primary_monitor() {
        assert_eq!(parse_source(None, None), None);
        assert_eq!(parse_source(Some("monitor"), None), None);
        assert_eq!(parse_source(Some("camera"), Some(1)), None);
    }

    #[test]
    fn sound_emoji_input_is_trimmed_and_checked() {
        assert_eq!(check_sound_emoji(None), Ok(None));
        assert_eq!(check_sound_emoji(Some("  ".into())), Ok(None));
        assert_eq!(check_sound_emoji(Some(" 🥁 ".into())), Ok(Some("🥁".into())));
        assert!(check_sound_emoji(Some("drum".into())).is_err());
    }
}
