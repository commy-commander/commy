mod account;
mod account_folder;
mod account_log;
mod app_icon;
mod browser;
mod activity;
mod afk;
mod call_service;
mod calls;
mod entry_list;
mod catchup;
mod channel_lock;
mod community_sync;
mod deleted;
mod devices;
mod dm_sync;
mod directory;
mod doorbell;
mod expressions;
mod media_engine;
mod mirror;
mod igdb;
mod instance;
mod invite_link;
mod key_handoff;
mod link_preview;
mod local_key;
mod liveness;
mod outbox;
mod overlay;
mod persist;
mod picker;
mod pictures;
mod pins;
mod profile_images;
mod push_wake;
mod read_state;
mod relaunch;
mod safety;
mod saved;
use commy_view::catalog;
use commy_view::search;
mod server_wake;
mod settle;
mod state;
mod storage;
mod stream_deck;
mod sync_link;
mod templates;
mod toast;
mod tray;
mod updater;
use commy_view as view;
mod voice_keys;
mod voice_message;
mod voice_invite;
mod wake;

use media_engine::{should_dial, CameraSession, MicTest, ScreenShareSession, ScreenWatch, StreamAudioSink, VoiceSession};
use state::{App, VoiceSettings};
use tauri::{Emitter, Manager};
use view::{
    dm_message_view, dm_thread_views, DmMessageView, DmThreadView, MessageView, ServerSummary,
    ServerView,
};

use base64::Engine as _;
use commy_core::SignedEvent;
use commy_core::{
    Activity, ActivityInput, Attachment, Channel, ChannelId, ChannelKind, DmAction, DmMessage, Identity, Message, MessageId, Permissions,
    Presence, RoleId, ServerEvent, ServerId, ServerState, StoreError, UserId,
    UserProfile,
};
use commy_core::channel_key::{ChannelSecret, KeyId};
use commy_core::server::Role;
use commy_core::template::{text_channel_name, MAX_RETENTION_DAYS};
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
fn is_mobile_app() -> bool {
    cfg!(mobile)
}

#[tauri::command]
fn ui_ready() {
    tracing::info!("webview connected over the tauri bridge");
}

#[tauri::command]
fn init_user(app: tauri::State<App>, handle: tauri::AppHandle, username: String) -> CmdResult<UserProfile> {
    let username = username.trim();
    if username.is_empty() || username.len() > 32 {
        return Err("username must be 1 to 32 characters".into());
    }
    let mut profile = UserProfile::new(username);
    profile.node = app.node_hex.lock().unwrap().clone();
    profile.identity = app.identity().map(|i| i.public());
    *app.profile.lock().unwrap() = Some(profile.clone());
    app.persist_profile();
    entry_list::schedule(&handle);
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

fn clean_message(content: &str) -> CmdResult<String> {
    let content = content.trim();
    if content.is_empty() || content.len() > 4000 {
        return Err("message must be 1 to 4000 characters".into());
    }
    Ok(content.to_string())
}

fn clean_server_name(name: &str) -> CmdResult<String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err("server name must be 1 to 100 characters".into());
    }
    Ok(name.to_string())
}

fn check_channel_name(name: String) -> CmdResult<String> {
    if name.is_empty() || name.len() > 50 {
        return Err("channel name must be 1 to 50 characters".into());
    }
    Ok(name)
}

const KEY_SETTLE_MAX: std::time::Duration = std::time::Duration::from_secs(120);

#[tauri::command]
async fn create_server(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    name: String,
    template: Option<String>,
) -> CmdResult<ServerView> {
    let name = clean_server_name(&name)?;
    let template = templates::resolve(template.as_deref())?;
    current_profile(&app)?;
    account::require_account(&app)?;

    let server_id = ServerId::new();
    let hosting = async {
        let mut net = app.net.lock().await;
        if let Some(node) = net.as_mut() {
            match node.host_server(server_id, &name, app.own_key_for_tickets()).await {
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
    let (state, _, signing) =
        own_server_state_from(&app, server_id, hosting, |owner| template.build(&name, owner)).await?;
    let view = ServerView::for_viewer(&state, Some(state.server.owner), &channel_lock::holder_of(&app));
    app.servers.lock().unwrap().insert(server_id, state);
    drop(signing);
    app.persist_server(server_id);
    app.persist_invites();
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
    if app.deleted.lock().unwrap().contains(ticket.server_id) {
        return Err("This server was deleted by its owner".into());
    }

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
        app.persist_invites();
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
    doorbell::record_join(app, ticket.server_id);
    handle.emit("servers-changed", serde_json::Value::Null);
    Ok(summary)
}

async fn own_server_state<'a>(
    app: &'a App,
    id: ServerId,
    name: &str,
    connect: impl std::future::Future<Output = CmdResult<()>>,
) -> CmdResult<(ServerState, UserProfile, tokio::sync::RwLockReadGuard<'a, ()>)> {
    own_server_state_from(app, id, connect, |owner| commy_core::Server::new(name, owner)).await
}

async fn own_server_state_from<'a>(
    app: &'a App,
    id: ServerId,
    connect: impl std::future::Future<Output = CmdResult<()>>,
    make: impl FnOnce(UserId) -> commy_core::Server,
) -> CmdResult<(ServerState, UserProfile, tokio::sync::RwLockReadGuard<'a, ()>)> {
    connect.await?;
    let signing = app.signing.read().await;
    let profile = current_profile(app)?;
    let mut server = make(profile.id);
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
    let keys = channel_lock::holder_of(&app);
    let friends = app.safety.lock().unwrap().friends.clone();
    let contacts = app.dms.lock().unwrap().contacts.clone();
    let servers = app.servers.lock().unwrap();
    let state = servers.get(&id).ok_or("unknown server")?;
    Ok(ServerView::for_viewer(state, me, &keys).with_friend_details(&friends, &contacts))
}

#[tauri::command]
fn get_invite(app: tauri::State<App>, server_id: String) -> CmdResult<String> {
    let id = server_id.parse().map_err(err_to_string)?;
    require_invite_permission(&app, id)?;
    let ticket = app.vouched_ticket(id).ok_or("no invite available; you may be offline")?;
    if ticket.owner.is_none() {
        return Err("your account key is still being confirmed; try again in a moment".into());
    }
    Ok(ticket.encode())
}

#[tauri::command]
fn get_messages(
    app: tauri::State<App>,
    server_id: String,
    channel_id: String,
    around: Option<String>,
) -> CmdResult<Vec<MessageView>> {
    let sid = server_id.parse().map_err(err_to_string)?;
    let cid = channel_id.parse().map_err(err_to_string)?;
    let around: Option<MessageId> = around.as_deref().map(str::parse).transpose().map_err(err_to_string)?;
    let pending = app.outbox.lock().unwrap().pending_ids(sid);
    let keys = channel_lock::holder_of(&app);
    let me = current_profile(&app)?.id;
    let servers = app.servers.lock().unwrap();
    let state = servers.get(&sid).ok_or("unknown server")?;
    if !state.server.can_view(cid, me) {
        return Ok(Vec::new());
    }
    let floor = {
        let opens = app.channel_opens.lock().unwrap();
        let none = view::ChannelOpens::new();
        view::history_floor(state, Some(me), opens.get(&sid).unwrap_or(&none), cid, chrono::Utc::now())
    };
    if let Some(id) = around {
        return view::message_views_around(state, cid, id, &pending, Some(me), &keys, floor).ok_or_else(|| {
            if floor.is_some() && state.message(id).is_some_and(|m| m.channel == cid) {
                HISTORY_HIDDEN.to_string()
            } else {
                "that message is no longer on this device".to_string()
            }
        });
    }
    Ok(view::message_views(state, cid, 100, &pending, Some(me), &keys, floor))
}

const HISTORY_HIDDEN: &str = "you can see only new messages in this channel";

#[tauri::command]
fn open_channel(app: tauri::State<App>, server_id: String, channel_id: String) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    app.channel_opens.lock().unwrap().entry(sid).or_default().insert(cid, chrono::Utc::now());
    Ok(())
}

#[tauri::command]
fn search_messages(
    app: tauri::State<App>,
    server_id: String,
    terms: Vec<String>,
    from: Option<String>,
    mentions: Option<String>,
    channel_id: Option<String>,
    offset: Option<usize>,
) -> CmdResult<search::SearchResults> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let from: Option<UserId> = from.as_deref().map(str::parse).transpose().map_err(err_to_string)?;
    let mentions: Option<UserId> = mentions.as_deref().map(str::parse).transpose().map_err(err_to_string)?;
    let channel: Option<ChannelId> = channel_id.as_deref().map(str::parse).transpose().map_err(err_to_string)?;
    let query = search::SearchQuery::new(&terms, from, mentions, channel);
    let keys = channel_lock::holder_of(&app);
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let opens = app.channel_opens.lock().unwrap().get(&sid).cloned().unwrap_or_default();
    let servers = app.servers.lock().unwrap();
    let state = servers.get(&sid).ok_or("unknown server")?;
    Ok(search::search(state, &query, me, offset.map_or(search::Page::newest(), search::Page::at), &keys, &opens, Some(chrono::Utc::now())))
}

#[tauri::command]
fn search_dms(
    app: tauri::State<App>,
    terms: Vec<String>,
    from: Option<String>,
    mentions: Option<String>,
    user_id: Option<String>,
    offset: Option<usize>,
) -> CmdResult<search::DmSearchResults> {
    let from: Option<UserId> = from.as_deref().map(str::parse).transpose().map_err(err_to_string)?;
    let mentions: Option<UserId> = mentions.as_deref().map(str::parse).transpose().map_err(err_to_string)?;
    let partner: Option<UserId> = user_id.as_deref().map(str::parse).transpose().map_err(err_to_string)?;
    let query = search::SearchQuery::new(&terms, from, mentions, None);
    let keys = channel_lock::holder_of(&app);
    let me = current_profile(&app)?.id;
    let blocked = app.safety.lock().unwrap().blocked.clone();
    let servers = app.servers.lock().unwrap();
    let dms = app.dms.lock().unwrap();
    Ok(search::search_dms(&dms, &servers, &query, me, partner, &blocked, offset.map_or(search::Page::newest(), search::Page::at), &keys))
}

fn slow_mode_words(seconds: i64) -> String {
    match seconds {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m {}s", s / 60, s % 60),
        s => format!("{}h {}m", s / 3600, (s % 3600) / 60),
    }
}

async fn commit_message(
    app: &App,
    handle: &tauri::AppHandle,
    sid: ServerId,
    message: Message,
) -> CmdResult<MessageView> {
    let event = ServerEvent::MessageCreate {
        message: message.clone(),
    };
    let keys = channel_lock::holder_of(app);
    let view = {
        let mut servers = app.servers.lock().unwrap();
        let state = servers.get_mut(&sid).ok_or("unknown server")?;
        require_rules_accepted(state, message.author)?;
        require_can_post(app, state, message.author, message.channel)?;
        if let Some(wait) = state.slow_mode_wait(message.channel, message.author, message.created_at) {
            return Err(format!("slow mode is on here: you can send again in {}", slow_mode_words(wait)));
        }
        refuse_held(keys.hold(state, &message))?;
        state.apply(event.clone()).map_err(err_to_string)?;
        MessageView::build(&message, state, &keys)
    };
    let delivered = outbox::broadcast_local(app, sid, event).await;
    app.persist_server(sid);
    doorbell::ring(handle, sid, message.id, !delivered);
    wake::after_send(handle, sid, message.id);
    let _ = handle.emit("server-event", &sid.to_string());
    Ok(view.with_pending(!delivered))
}

fn command_marker(app: &App, sid: ServerId, used: commy_core::CommandUse) -> CmdResult<commy_core::Attachment> {
    let used = used.tidy().ok_or("that command's options do not fit")?;
    let servers = app.servers.lock().unwrap();
    let state = servers.get(&sid).ok_or("unknown server")?;
    if !state.is_bot(used.bot) || !state.server.is_current_member(used.bot) {
        return Err("that bot is not in this server".into());
    }
    let command = state.command_of(&used).ok_or("that bot has no such command, or an option does not fit")?;
    for (name, value) in &used.options {
        let Some(option) = command.option(name) else { continue };
        let known = match option.kind {
            commy_core::OptionKind::User => value.parse::<UserId>().is_ok_and(|u| state.server.is_current_member(u)),
            commy_core::OptionKind::Channel => value.parse::<ChannelId>().is_ok_and(|c| state.server.channel(c).is_some()),
            _ => true,
        };
        if !known {
            return Err(format!("{name} needs a member or channel of this server"));
        }
    }
    Ok(used.into_attachment())
}

fn seal_and_sign(app: &App, sid: ServerId, message: &mut Message) -> CmdResult<()> {
    let keys = channel_lock::holder_of(app);
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_can_post(app, state, message.author, message.channel)?;
        keys.seal_outgoing(state, message)?;
    }
    message.sign(sid, &signing_key(app)?);
    Ok(())
}

#[tauri::command]
async fn send_typing(app: tauri::State<'_, App>, server_id: String, channel_id: String) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let channel: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?.id;
    let allowed = app.servers.lock().unwrap().get(&sid).is_some_and(|s| s.server.channel(channel).is_some());
    if !allowed {
        return Err("unknown channel".into());
    }
    let silenced = app.servers.lock().unwrap().get(&sid).is_some_and(|s| s.server.is_timed_out(me, chrono::Utc::now()));
    if silenced {
        return Ok(());
    }
    broadcast_event(&app, sid, ServerEvent::TypingStart { channel, user: me }).await;
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
    preview_url: Option<String>,
    tts: Option<bool>,
    command: Option<commy_core::CommandUse>,
) -> CmdResult<MessageView> {
    let content = clean_message(&content)?;
    let sid = server_id.parse().map_err(err_to_string)?;
    let cid = channel_id.parse().map_err(err_to_string)?;
    let profile = current_profile(&app)?;
    let content = expressions_for_server(&app, sid, profile.id, content).await?;
    let mut message = Message::new(cid, profile.id, content);
    if tts.unwrap_or(false) {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, profile.id, Permissions::SEND_TTS_MESSAGES, "send text-to-speech messages here")?;
        message.mark_tts();
    }
    if let Some(used) = command {
        message.attachments.push(command_marker(&app, sid, used)?);
    }
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
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_inside_post(state, cid, message.reply_to)?;
    }
    let card = preview_card(&app, sid, &message, preview_url.as_deref()).await;
    let carries_card = card.is_some();
    let draft = message.clone();
    message.attachments.extend(card);
    let _signing = app.signing.read().await;
    seal_and_sign(&app, sid, &mut message)?;
    if carries_card && !signed_fits_one_gossip_frame(&message) {
        message = draft;
        seal_and_sign(&app, sid, &mut message)?;
    }
    if !signed_fits_one_gossip_frame(&message) {
        return Err("that message is too long to send; try splitting it up".into());
    }
    commit_message(&app, &handle, sid, message).await
}

#[tauri::command]
async fn send_poll(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    question: String,
    options: Vec<String>,
    multi: bool,
    hours: Option<u32>,
) -> CmdResult<MessageView> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid = channel_id.parse().map_err(err_to_string)?;
    let profile = current_profile(&app)?;
    let poll = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        view::act::poll(our_permissions(&app, state, profile.id), question, options, multi, hours, chrono::Utc::now())?
    };
    let mut message = Message::new(cid, profile.id, "");
    message.attachments.push(poll.into_attachment());
    let _signing = app.signing.read().await;
    seal_and_sign(&app, sid, &mut message)?;
    if !signed_fits_one_gossip_frame(&message) {
        return Err("that poll is too long to send; shorten the question or the answers".into());
    }
    commit_message(&app, &handle, sid, message).await
}


#[tauri::command]
async fn vote_poll(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    message_id: String,
    choices: Vec<u8>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let mid: MessageId = message_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let event = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        view::act::poll_vote(state, &acting(&app, state, me.id), mid, &choices)?
    };
    apply_and_broadcast(&app, &handle, sid, event).await
}

#[tauri::command]
async fn close_poll(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    message_id: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let mid: MessageId = message_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let event = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        view::act::poll_close(state, &acting(&app, state, me.id), mid)?
    };
    apply_and_broadcast(&app, &handle, sid, event).await
}

async fn preview_card(app: &App, sid: ServerId, message: &Message, url: Option<&str>) -> Option<Attachment> {
    let url = url.filter(|u| message.content.contains(u))?;
    let may_share_pictures = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid)?;
        let perms = our_permissions(app, state, message.author);
        if !perms.can_embed_links() {
            return None;
        }
        perms.allows(Permissions::ATTACH_FILES)
    };
    let prepared = link_preview::prepared(url)?;
    let picture = match prepared.picture {
        Some(bytes) if may_share_pictures => {
            let size_bytes = bytes.len() as u64;
            let net = app.net.lock().await;
            let node = net.as_ref()?;
            let picture = node.add_blob(bytes.clone()).await.ok().map(|blob| commy_core::PreviewPicture {
                blob,
                size_bytes,
                provider: Some(node.node_id_hex()),
            });
            if let Some(picture) = &picture {
                pictures::offer(&app.http, &picture.blob, &bytes);
            }
            picture
        }
        _ => None,
    };
    Some(prepared.preview.into_attachment(picture))
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
    let me = current_profile(&app)?;
    let event = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        view::act::reaction(state, &acting(&app, state, me.id), cid, mid, &emoji, active)?
    };
    apply_and_broadcast(&app, &handle, sid, event).await
}

fn server_images(state: &ServerState) -> std::collections::HashSet<String> {
    let images = state.server.emojis.iter().map(|e| &e.blob).chain(state.server.stickers.iter().map(|s| &s.blob));
    images.filter_map(|r| commy_core::server::ImageRef::parse(r)).map(|r| r.blob).collect()
}

async fn expressions_for_server(app: &App, sid: ServerId, me: UserId, content: String) -> CmdResult<String> {
    let (local, may_use_others) = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        (server_images(state), our_permissions(app, state, me).allows(Permissions::USE_EXTERNAL_EMOJIS))
    };
    let foreign = commy_core::server::image_tokens(&content).iter().any(|(_, _, image)| !local.contains(&image.blob));
    if foreign && !may_use_others {
        return Err("you can't use emojis or stickers from other servers here".into());
    }
    Ok(carry_expressions(app, content, &local).await)
}

async fn carry_expressions(app: &App, content: String, local: &std::collections::HashSet<String>) -> String {
    let tokens = commy_core::server::image_tokens(&content);
    if tokens.iter().all(|(_, _, image)| local.contains(&image.blob)) {
        return content;
    }
    let ours = app.net.lock().await.as_ref().map(|n| n.node_id_hex());
    let mut out = String::with_capacity(content.len());
    let mut last = 0;
    for (start, end, image) in tokens {
        out.push_str(&content[last..start]);
        let token = &content[start..end];
        let carried = if local.contains(&image.blob) || ours.as_deref() == Some(image.provider.as_str()) {
            None
        } else {
            match expression_bytes(app, &image.encode()).await {
                Ok(bytes) => store_shared_blob(app, bytes).await.ok(),
                Err(e) => {
                    tracing::debug!("kept the original source for an image we could not load: {e}");
                    None
                }
            }
        };
        match carried {
            Some(ours) => out.push_str(&token.replacen(&image.encode(), &ours, 1)),
            None => out.push_str(token),
        }
        last = end;
    }
    out.push_str(&content[last..]);
    out
}

async fn carry_profile_emojis(app: &App, text: String) -> String {
    carry_expressions(app, text, &std::collections::HashSet::new()).await
}

fn signed_fits_one_gossip_frame(signed: &Message) -> bool {
    let wire = WirePayload::Event(ServerEvent::MessageCreate { message: signed.clone() });
    commy_net::protocol::gossip_size(&wire) <= commy_net::protocol::MAX_GOSSIP_BYTES
}

fn acting(app: &App, state: &ServerState, me: UserId) -> view::act::Me {
    view::act::Me { user: me, perms: our_permissions(app, state, me), now: chrono::Utc::now() }
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
    let content = clean_message(&content)?;
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let mid: MessageId = message_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let content = expressions_for_server(&app, sid, me.id, content).await?;
    let keys = channel_lock::holder_of(&app);
    let _signing = app.signing.read().await;
    let identity = signing_key(&app)?;
    let edit = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        view::act::edit(state, &acting(&app, state, me.id), &identity, &keys, cid, mid, content)?
    };
    apply_and_broadcast(&app, &handle, sid, edit).await
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
    let event = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        view::act::delete(state, &acting(&app, state, me.id), &identity, cid, mid)?
    };
    apply_and_broadcast(&app, &handle, sid, event).await
}

const MAX_ATTACHMENT_BYTES: u64 = 512 * 1024 * 1024;
const ATTACHMENT_TOO_LARGE: &str = "files larger than 512 MB are not supported yet";

async fn send_attachment(
    app: &tauri::State<'_, App>,
    handle: &tauri::AppHandle,
    sid: ServerId,
    cid: ChannelId,
    filename: String,
    bytes: Vec<u8>,
) -> CmdResult<MessageView> {
    if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
        return Err(ATTACHMENT_TOO_LARGE.into());
    }
    let profile = current_profile(app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(app, state, profile.id, Permissions::ATTACH_FILES, "send files or images here")?;
    }
    let size_bytes = bytes.len() as u64;
    let (blob, provider) = store_blob(app, bytes).await?;

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
    seal_and_sign(app, sid, &mut message)?;
    commit_message(app, handle, sid, message).await
}

async fn store_blob(app: &App, bytes: Vec<u8>) -> CmdResult<(String, Option<String>)> {
    let net = app.net.lock().await;
    let node = net.as_ref().ok_or("network is not ready yet")?;
    let hash = node.add_blob(bytes).await.map_err(err_to_string)?;
    Ok((hash, Some(node.node_id_hex())))
}

#[tauri::command]
async fn start_voice_recording(app: tauri::State<'_, App>) -> CmdResult<()> {
    let stale = app.voice_recorder.lock().unwrap().take();
    if let Some(stale) = stale {
        stale.discard().await;
    }
    let voice = app.voice.lock().unwrap().clone();
    let recorder = voice_message::VoiceRecorder::start(&voice).map_err(|e| format!("could not start recording: {e}"))?;
    *app.voice_recorder.lock().unwrap() = Some(recorder);
    Ok(())
}

#[tauri::command]
async fn cancel_voice_recording(app: tauri::State<'_, App>) -> CmdResult<()> {
    let recorder = app.voice_recorder.lock().unwrap().take();
    if let Some(recorder) = recorder {
        recorder.discard().await;
    }
    Ok(())
}

async fn take_recording(app: &App) -> CmdResult<voice_message::Recording> {
    let recorder = app.voice_recorder.lock().unwrap().take().ok_or("no voice message is being recorded")?;
    let clip = recorder.stop().await.map_err(|e| format!("recording failed: {e}"))?;
    if clip.duration_ms < voice_message::MIN_VOICE_MESSAGE_MS {
        return Err("that voice message is too short to send".into());
    }
    if clip.bytes.len() as u64 > voice_message::MAX_VOICE_MESSAGE_BYTES {
        return Err("that voice message is too long to send".into());
    }
    Ok(clip)
}

#[tauri::command]
async fn send_voice_message(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
) -> CmdResult<MessageView> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let clip = take_recording(&app).await?;
    let profile = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, profile.id, Permissions::SEND_VOICE_MESSAGES, "send voice messages here")?;
    }
    let size_bytes = clip.bytes.len() as u64;
    let (blob, provider) = store_blob(&app, clip.bytes).await?;
    let mut message = Message::new(cid, profile.id, String::new());
    message.attachments.push(Attachment::voice_message(blob, size_bytes, provider));
    let _signing = app.signing.read().await;
    seal_and_sign(&app, sid, &mut message)?;
    commit_message(&app, &handle, sid, message).await
}

#[tauri::command]
async fn send_dm_voice_message(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    user_id: String,
) -> CmdResult<DmMessageView> {
    let (me, partner) = dm_recipient(&app, &user_id)?;
    let clip = take_recording(&app).await?;
    let size_bytes = clip.bytes.len() as u64;
    let (blob, provider) = store_blob(&app, clip.bytes).await?;
    let mut message = DmMessage::new(me.id, partner, String::new());
    message.attachments.push(Attachment::voice_message(blob, size_bytes, provider));
    let view = post_dm(&app, &handle, &me, partner, message).await?;
    view.ok_or_else(|| "voice message not sent".to_string())
}

#[tauri::command]
async fn get_voice_message(
    app: tauri::State<'_, App>,
    blob: String,
    provider: Option<String>,
    size_bytes: Option<u64>,
) -> CmdResult<tauri::ipc::Response> {
    let max = voice_message::MAX_VOICE_MESSAGE_BYTES;
    if size_bytes.is_some_and(|size| size > max) {
        return Err("voice message too large to play here".into());
    }
    let bytes = fetch_blob(&app, &blob, provider.as_deref(), max).await?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
async fn attach_file(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    kind: Option<String>,
) -> CmdResult<Option<MessageView>> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;

    let (title, filter) = match kind.as_deref() {
        Some("video") => ("Post a video", Some(picker::Filter { label: "Videos", extensions: INLINE_VIDEO_EXTENSIONS })),
        _ => ("Share a file", None),
    };
    let Some(picked) = picker::pick_file(&handle, title, filter, MAX_ATTACHMENT_BYTES, ATTACHMENT_TOO_LARGE).await? else {
        return Ok(None);
    };
    send_attachment(&app, &handle, sid, cid, picked.name, picked.bytes)
        .await
        .map(Some)
}

const INLINE_VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "webm"];
const MAX_INLINE_VIDEO_BYTES: usize = 100 * 1024 * 1024;

#[tauri::command]
async fn get_attachment_video(
    app: tauri::State<'_, App>,
    blob: String,
    provider: Option<String>,
    size_bytes: Option<u64>,
) -> CmdResult<tauri::ipc::Response> {
    let max = MAX_INLINE_VIDEO_BYTES as u64;
    if size_bytes.is_some_and(|size| size > max) {
        return Err("video too large to play here".into());
    }
    let bytes = fetch_blob(&app, &blob, provider.as_deref(), max).await?;
    if bytes.len() > MAX_INLINE_VIDEO_BYTES {
        return Err("video too large to play here".into());
    }
    Ok(tauri::ipc::Response::new(bytes))
}

fn data_url(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

const MAX_PICKED_AVATAR_BYTES: u64 = 64 * 1024 * 1024;

#[tauri::command]
async fn pick_avatar_model(handle: tauri::AppHandle) -> CmdResult<tauri::ipc::Response> {
    let filter = picker::Filter { label: "VRM avatars", extensions: &["vrm"] };
    let picked = picker::pick_file(&handle, "Choose an avatar", Some(filter), MAX_PICKED_AVATAR_BYTES, "that avatar is larger than 64 MB").await?;
    Ok(tauri::ipc::Response::new(picked.map(|p| p.bytes).unwrap_or_default()))
}

const MAX_PICKED_IMAGE_BYTES: u64 = 25 * 1024 * 1024;
const IMAGE_TOO_LARGE: &str = "that image is larger than 25 MB";

#[tauri::command]
async fn pick_image(handle: tauri::AppHandle, title: Option<String>) -> CmdResult<Option<String>> {
    let filter = picker::Filter { label: "Images", extensions: &["png", "jpg", "jpeg", "gif", "webp"] };
    let title = title.as_deref().unwrap_or("Choose an image");
    let Some(picked) = picker::pick_file(&handle, title, Some(filter), MAX_PICKED_IMAGE_BYTES, IMAGE_TOO_LARGE).await? else {
        return Ok(None);
    };
    let bytes = picked.bytes;
    let mime = match std::path::Path::new(&picked.name).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "image/jpeg",
    };
    Ok(Some(data_url(mime, &bytes)))
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
fn open_url(handle: tauri::AppHandle, url: String) -> CmdResult<()> {
    let target = web_url(&url).ok_or("only http and https links can be opened")?;
    browser::open(&handle, &target)
}

fn web_url(raw: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(raw.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return None;
    }
    Some(parsed.to_string())
}

const MAX_FILE_NAME_CHARS: usize = 120;

fn safe_file_name(name: &str) -> String {
    let last_segment = name.rsplit(['/', '\\']).find(|part| !part.trim().is_empty()).unwrap_or("");
    let mut cleaned: String = last_segment
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| if matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    cleaned = cleaned.trim().trim_start_matches(['.', ' ']).trim_end_matches(['.', ' ']).to_string();
    if cleaned.chars().count() > MAX_FILE_NAME_CHARS {
        let (stem, ext) = match cleaned.rsplit_once('.') {
            Some((stem, ext)) if ext.len() <= 10 && !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
            _ => (cleaned.clone(), String::new()),
        };
        let keep = MAX_FILE_NAME_CHARS.saturating_sub(ext.chars().count()).max(1);
        cleaned = format!("{}{ext}", stem.chars().take(keep).collect::<String>());
    }
    let stem = cleaned.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.ends_with(|c: char| c.is_ascii_digit()));
    if reserved {
        cleaned = format!("_{cleaned}");
    }
    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned
    }
}

#[tauri::command]
async fn download_attachment(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    blob: String,
    filename: String,
    provider: Option<String>,
    size_bytes: Option<u64>,
) -> CmdResult<Option<String>> {
    if let Some(size) = size_bytes {
        commy_net::node::check_blob_size(size, MAX_ATTACHMENT_BYTES).map_err(err_to_string)?;
    }
    let bytes = fetch_blob(&app, &blob, provider.as_deref(), MAX_ATTACHMENT_BYTES).await?;
    picker::save_file(&handle, &safe_file_name(&filename), bytes).await
}

#[tauri::command]
async fn set_voice_channel(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: Option<String>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: Option<ChannelId> = channel_id.as_deref().map(str::parse).transpose().map_err(err_to_string)?;
    let profile = current_profile(&app)?;
    let voice = app.voice.lock().unwrap().clone();

    match cid {
        Some(channel) => {
            {
                let servers = app.servers.lock().unwrap();
                let state = servers.get(&sid).ok_or("unknown server")?;
                view::act::join_voice(state, our_permissions(&app, state, profile.id), profile.id, channel)?;
            }
            calls::end(&app, &handle, true, "hung_up").await;
            let previous = app.voice_session.lock().unwrap().as_ref().map(|s| s.server).filter(|s| *s != sid);
            if let Some(previous) = previous {
                leave_voice(&app, &handle, previous, &profile, &voice).await;
            }
            start_voice_session(&app, &handle, sid, channel, &voice, profile.id);
            apply_personal_volumes(&app);
            refresh_forced_voice(&app, &handle);
            refresh_priority_speakers(&app);
            devices::announce(&app, sid).await;
            dial_voice_members(&app, &handle, sid, channel, &profile);
            set_own_voice_state(&app, &handle, sid, &profile, &voice, Some(channel)).await?;
            devices::claim_profile_node(&app, sid).await;
            Ok(())
        }
        None => {
            leave_voice(&app, &handle, sid, &profile, &voice).await;
            Ok(())
        }
    }
}

async fn leave_voice(
    app: &App,
    handle: &tauri::AppHandle,
    sid: ServerId,
    profile: &UserProfile,
    voice: &VoiceSettings,
) {
    stop_voice_here(app, sid, profile).await;
    if let Err(e) = set_own_voice_state(app, handle, sid, profile, voice, None).await {
        tracing::warn!("leaving voice: {e}");
    }
    devices::announce(app, sid).await;
}

async fn stop_voice_here(app: &App, sid: ServerId, profile: &UserProfile) {
    let testing = pause_mic_test(app);
    *app.voice_session.lock().unwrap() = None;
    resume_mic_test(app, testing);
    *app.camera.lock().unwrap() = None;
    app.camera_watches.lock().unwrap().clear();
    app.own_preview.lock().unwrap().take();
    let share = app.screen_share.lock().unwrap().take();
    if let Some(share) = share {
        broadcast_event(
            app,
            sid,
            ServerEvent::ScreenShare {
                user: profile.id,
                channel: share.channel,
                active: false,
            },
        )
        .await;
    }
}

async fn set_own_voice_state(
    app: &App,
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
    broadcast_event(app, sid, event).await;
    let _ = handle.emit("server-event", &sid.to_string());
    Ok(())
}

fn active_voice_channel(app: &App) -> Option<ChannelId> {
    app.voice_session.lock().unwrap().as_ref().map(|s| s.channel)
}

fn start_voice_session(
    app: &tauri::State<'_, App>,
    handle: &tauri::AppHandle,
    sid: ServerId,
    channel: ChannelId,
    voice: &VoiceSettings,
    me: UserId,
) {
    let testing = pause_mic_test(app);
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
    let mut session = VoiceSession::start(
        sid,
        channel,
        StreamGuards { mic, speaker: None },
        mic_rx,
        mic_tx,
        voice.deafened,
        me,
        on_speaking,
    );
    let far_end = app.processor.lock().unwrap().far_end_sink();
    let speaker = match commy_media::start_playback(session.mixer.clone(), voice.output_device.clone(), Some(far_end)) {
        Ok(handle) => Some(handle),
        Err(e) => {
            tracing::warn!("speaker unavailable: {e}");
            None
        }
    };
    let mut slot = app.voice_session.lock().unwrap();
    session.set_speaker(speaker);
    session.mixer.lock().unwrap().set_master(voice.output_volume);
    *slot = Some(session);
    drop(slot);
    resume_mic_test(app, testing);
}

fn open_mic_test(app: &App) -> anyhow::Result<MicTest> {
    if let Some(session) = app.voice_session.lock().unwrap().as_ref() {
        return Ok(MicTest::within_call(session));
    }
    let voice = app.voice.lock().unwrap().clone();
    MicTest::start(app.processor.clone(), voice.input_device, voice.output_device)
}

fn pause_mic_test(app: &App) -> bool {
    app.mic_test.lock().unwrap().take().is_some()
}

fn resume_mic_test(app: &App, was_running: bool) {
    if !was_running {
        return;
    }
    match open_mic_test(app) {
        Ok(test) => *app.mic_test.lock().unwrap() = Some(test),
        Err(e) => tracing::warn!("mic test stopped: {e}"),
    }
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
            .filter_map(|(user, _)| state.voice_node(*user).map(|hex| (*user, hex)))
            .collect()
    };
    for (user, their_hex) in targets {
        if should_dial(&our_hex, &their_hex) && claim_voice_dial(app, channel, user) {
            dial_voice_peer(handle, user, their_hex, channel, me.id);
        }
    }
}

fn claim_voice_dial(app: &App, channel: ChannelId, user: UserId) -> bool {
    app.voice_session
        .lock()
        .unwrap()
        .as_mut()
        .is_some_and(|session| session.channel == channel && session.claim_dial(user))
}

const VOICE_REDIAL_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

async fn run_voice_redialer(handle: tauri::AppHandle) {
    loop {
        tokio::time::sleep(VOICE_REDIAL_EVERY).await;
        let app = handle.state::<App>();
        let call = app.voice_session.lock().unwrap().as_ref().map(|s| (s.server, s.channel));
        let Some((sid, channel)) = call else { continue };
        let Some(me) = app.profile.lock().unwrap().clone() else { continue };
        if sid == commy_core::ids::CALL_SERVER {
            calls::dial_partner(&app, &handle, &me);
        } else {
            dial_voice_members(&app, &handle, sid, channel, &me);
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
        let dialed = dial_media(&app, &their_hex, hello).await;
        let mut session = app.voice_session.lock().unwrap();
        let Some(session) = session.as_mut().filter(|s| s.channel == channel) else { return };
        match dialed {
            Ok(media) => session.attach_peer(user, media),
            Err(e) => {
                tracing::warn!("voice dial failed, retrying shortly: {e}");
                session.dial_failed(user);
            }
        }
    });
}

fn servers_using_image(app: &App, image: &str) -> Vec<ServerId> {
    let servers = app.servers.lock().unwrap();
    servers
        .values()
        .filter(|state| {
            let server = &state.server;
            server.emojis.iter().any(|e| e.blob == image)
                || server.stickers.iter().any(|s| s.blob == image)
                || server.soundboard.iter().any(|s| s.blob == image)
                || server.icon.as_deref() == Some(image)
                || server.banner.as_deref() == Some(image)
                || state.invite_background() == Some(image)
        })
        .map(|state| state.server.id)
        .collect()
}

fn image_sources(provider: &str, neighbors: Vec<[u8; 32]>) -> Vec<String> {
    let mut sources: Vec<String> = Vec::new();
    for node in neighbors.iter().map(|n| data_encoding::HEXLOWER.encode(n)) {
        if !sources.contains(&node) {
            sources.push(node);
        }
    }
    match sources.iter().position(|n| n == provider) {
        Some(at) => sources.swap(0, at),
        None => sources.push(provider.to_string()),
    }
    sources
}

async fn fetch_shared_image(app: &App, image: &commy_core::server::ImageRef, max: u64) -> CmdResult<Vec<u8>> {
    if let Some(bytes) = pictures::fetch(&app.http, &image.blob, max).await {
        return Ok(bytes);
    }
    let (fetcher, sources) = {
        let net = app.net.lock().await;
        let node = net.as_ref().ok_or("network is not ready yet")?;
        let neighbors = servers_using_image(app, &image.encode()).into_iter().flat_map(|s| node.neighbors(s)).collect();
        (node.blob_fetcher(), image_sources(&image.provider, neighbors))
    };
    let mut failure = String::from("nobody who has it is online");
    for source in sources {
        match fetcher.fetch(&image.blob, Some(&source), max).await {
            Ok(bytes) => {
                pictures::offer(&app.http, &image.blob, &bytes);
                return Ok(bytes);
            }
            Err(e) => failure = err_to_string(e),
        }
    }
    Err(failure)
}

fn cache_shared_image(app: &App, blob: &str, bytes: &[u8]) {
    if let Some(path) = server_image_path(app, blob) {
        let _ = write_blob_cache(&path, bytes);
    }
}

async fn expression_bytes(app: &App, blob: &str) -> CmdResult<Vec<u8>> {
    let image = commy_core::server::ImageRef::parse(blob).ok_or("bad reference")?;
    if let Some(bytes) = server_image_path(app, &image.blob).and_then(|p| commy_core::at_rest::read(&p).ok()) {
        return Ok(bytes);
    }
    let bytes = fetch_shared_image(app, &image, expressions::MAX_SOUND_BYTES as u64).await?;
    if expressions::expression_mime(&bytes).is_none() {
        return Err("not a valid emoji or sound".into());
    }
    cache_shared_image(app, &image.blob, &bytes);
    Ok(bytes)
}

fn prefetch_emoji<A: catchup::AppAccess + Clone>(access: &A, event: &ServerEvent) {
    let blob = match event {
        ServerEvent::EmojiAdd { emoji, .. } => emoji.blob.clone(),
        ServerEvent::StickerAdd { sticker, .. } => sticker.blob.clone(),
        _ => return,
    };
    let access = access.clone();
    doorbell::spawn(async move {
        if let Err(e) = expression_bytes(access.app(), &blob).await {
            tracing::debug!("emoji image not fetched yet: {e}");
        }
    });
}

fn neighbor_announcements(app: &App, sid: ServerId, me: UserId) -> Vec<WirePayload> {
    let relayed: Vec<SignedEvent> = {
        let servers = app.servers.lock().unwrap();
        servers.get(&sid).map(|s| s.signed_actions().into_iter().cloned().collect()).unwrap_or_default()
    };
    let held: std::collections::HashSet<String> = relayed.iter().filter_map(|s| commy_core::action::subject(&s.event)).collect();
    let ours = structure_announcements(app, sid, me)
        .into_iter()
        .filter(|event| commy_core::action::subject(event).is_none_or(|subject| !held.contains(&subject)))
        .map(|event| app.wire_for(sid, event));
    relayed.into_iter().map(WirePayload::Signed).chain(ours).collect()
}

async fn fetch_blob(app: &App, blob: &str, provider: Option<&str>, max: u64) -> CmdResult<Vec<u8>> {
    let fetcher = {
        let net = app.net.lock().await;
        net.as_ref().ok_or("network is not ready yet")?.blob_fetcher()
    };
    fetcher.fetch(blob, provider, max).await.map_err(err_to_string)
}

fn media_gate(handle: &tauri::AppHandle) -> commy_net::node::MediaGate {
    let handle = handle.clone();
    std::sync::Arc::new(move |peer, hello| {
        let app = handle.state::<App>();
        let node_hex = data_encoding::HEXLOWER.encode(&peer);
        let (user, channel, voice, server) = match *hello {
            MediaHello::Voice { user, channel } => {
                let session = app.voice_session.lock().unwrap();
                (user, channel, true, session.as_ref().filter(|s| s.channel == channel).map(|s| s.server))
            }
            MediaHello::ScreenSubscribe { user, channel } => {
                let share = app.screen_share.lock().unwrap();
                (user, channel, false, share.as_ref().filter(|s| s.channel == channel).map(|s| s.server))
            }
            MediaHello::CameraSubscribe { user, channel } => {
                let camera = app.camera.lock().unwrap();
                (user, channel, false, camera.as_ref().filter(|c| c.channel == channel).map(|c| c.server))
            }
        };
        let Some(server) = server else { return false };
        if server == commy_core::ids::CALL_SERVER {
            return voice && calls::peer_ok(&app, user, &node_hex, channel);
        }
        let servers = app.servers.lock().unwrap();
        servers.get(&server).is_some_and(|state| state.media_peer_ok(user, &node_hex, channel, voice))
    })
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
    audio: bool,
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
        audio: session.shares_audio(),
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
    audio: Option<bool>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let profile = current_profile(&app)?;
    let channel = active_voice_channel(&app).ok_or("join a voice channel first")?;

    let source = parse_source(source_kind.as_deref(), source_id);
    let quality = StreamQuality::from_name(quality.as_deref().unwrap_or(""));
    let share_audio = audio.unwrap_or(true) && commy_media::system_audio_supported();
    let session = ScreenShareSession::start(sid, channel, quality.fps(), quality.max_width(), source, share_audio);
    let (preview_tx, preview_rx) = tokio::sync::mpsc::unbounded_channel();
    session.add_subscriber(preview_tx);
    let emitter = handle.clone();
    let preview = ScreenWatch::from_frames(profile.id, preview_rx, None, None, move |user, seq, width, height, jpeg| {
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
    broadcast_event(&app, sid, event).await;
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
    broadcast_event(&app, sid, event).await;
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
        let hex = state.voice_node(user).ok_or("sharer has no reachable identity")?;
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
    let audio = stream_audio_sink(&app, user);
    let watch = ScreenWatch::start(user, media, audio, move |user, seq, width, height, jpeg| {
        let _ = emitter.emit("video-frame", video_frame_payload(user, Some(seq), width, height, &jpeg));
    });
    app.watches.lock().unwrap().insert(user, watch);
    Ok(())
}

fn stream_audio_sink(app: &App, sharer: UserId) -> Option<StreamAudioSink> {
    let voice = app.voice.lock().unwrap().clone();
    let gain = voice.stream_gain(app.forced_deaf.load(std::sync::atomic::Ordering::Relaxed));
    if let Some(session) = app.voice_session.lock().unwrap().as_ref() {
        return Some(StreamAudioSink::in_call(session.mixer.clone(), sharer, gain));
    }
    match StreamAudioSink::standalone(sharer, gain, voice.output_volume, voice.output_device.clone()) {
        Ok(sink) => Some(sink),
        Err(e) => {
            tracing::warn!("watching without stream audio: {e}");
            None
        }
    }
}

fn apply_stream_volume(app: &App) {
    let voice = app.voice.lock().unwrap().clone();
    let gain = voice.stream_gain(app.forced_deaf.load(std::sync::atomic::Ordering::Relaxed));
    for watch in app.watches.lock().unwrap().values() {
        watch.set_stream_volume(gain);
        watch.set_master_volume(voice.output_volume);
    }
}

#[tauri::command]
fn share_audio_supported() -> bool {
    commy_media::system_audio_supported()
}

#[tauri::command]
fn set_stream_volume(app: tauri::State<App>, volume: f32) -> VoiceSettings {
    let snapshot = app.update_voice(|voice| voice.stream_volume = volume.clamp(0.0, 2.0));
    apply_stream_volume(&app);
    snapshot
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
    let channel = active_voice_channel(&app).ok_or("join a voice channel first")?;
    *app.camera.lock().unwrap() = Some(CameraSession::new(sid, channel));
    apply_and_broadcast(
        &app,
        &handle,
        sid,
        ServerEvent::CameraState { user: profile.id, channel, active: true },
    ).await
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
    ).await
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
        let hex = state.voice_node(user).ok_or("that member has no reachable identity")?;
        (hex, channel)
    };
    let media = dial_media(&app, &their_hex, MediaHello::CameraSubscribe { user: profile.id, channel }).await?;
    let emitter = handle.clone();
    let watch = ScreenWatch::start(user, media, None, move |user, _seq, width, height, jpeg| {
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
    known_profile_field(app, user, |p| p.node.clone())
}

fn known_profile_field<T>(app: &App, user: UserId, field: impl Fn(&UserProfile) -> Option<T>) -> Option<T> {
    if let Some(found) = app.dms.lock().unwrap().contacts.get(&user).and_then(&field) {
        return Some(found);
    }
    app.servers.lock().unwrap().values().find_map(|s| s.profiles.get(&user).and_then(&field))
}

async fn join_group(app: &App, handle: &tauri::AppHandle, from: UserId, ticket: &str, key: Option<(KeyId, ChannelSecret)>) {
    let Ok(ticket) = ServerTicket::decode(ticket) else {
        tracing::debug!("ignored a group invite from {from} with a bad ticket");
        return;
    };
    if app.servers.lock().unwrap().contains_key(&ticket.server_id) {
        return;
    }
    if let Some((id, secret)) = key {
        community_sync::remember_invite_key(ticket.server_id, id, secret);
    }
    match join_ticket(app, handle, ticket.clone()).await {
        Ok(_) => {
            entry_list::record_join(app, ticket.server_id);
            entry_list::schedule(handle);
            community_sync::refresh_soon(handle);
        }
        Err(e) => tracing::warn!("could not join a group from {from}: {e}"),
    }
}

async fn invite_to_group(app: &App, handle: &tauri::AppHandle, me: &UserProfile, sid: ServerId, user_ids: Vec<String>) -> CmdResult<()> {
    let ticket = app.vouched_ticket(sid).ok_or("no invite available; you may be offline")?.encode();
    let key = community_sync::invite_key(app, sid);
    for raw in user_ids {
        let (_, user) = dm_partner(app, &raw)?;
        let already = app.servers.lock().unwrap().get(&sid).is_some_and(|s| s.server.is_current_member(user));
        if already {
            continue;
        }
        let invite = commy_view::dms::group_invite(me.id, user, ticket.clone(), key.as_ref().map(|(id, secret)| (*id, secret)));
        post_dm(app, handle, me, user, invite).await?;
    }
    Ok(())
}

#[tauri::command]
async fn create_group_dm(app: tauri::State<'_, App>, handle: tauri::AppHandle, user_ids: Vec<String>) -> CmdResult<ServerSummary> {
    account::require_account(&app)?;
    let me = current_profile(&app)?;
    if user_ids.is_empty() {
        return Err("pick at least one person".into());
    }
    let server_id = ServerId::new();
    let hosting = async {
        let mut net = app.net.lock().await;
        let node = net.as_mut().ok_or("network is not ready yet")?;
        let ticket = node.host_server(server_id, "", app.own_key_for_tickets()).await.map_err(err_to_string)?;
        app.invites.lock().unwrap().insert(server_id, ticket.encode());
        Ok(())
    };
    let (state, _, signing) = own_server_state_from(&app, server_id, hosting, commy_core::Server::group).await?;
    let summary = ServerSummary::from_state(&state, Some(me.id));
    app.servers.lock().unwrap().insert(server_id, state);
    drop(signing);
    app.persist_server(server_id);
    app.persist_invites();
    entry_list::record_join(&app, server_id);
    entry_list::schedule(&handle);
    let _ = handle.emit("servers-changed", ());
    invite_to_group(&app, &handle, &me, server_id, user_ids).await?;
    Ok(summary)
}

#[tauri::command]
async fn add_group_members(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String, user_ids: Vec<String>) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let group = app.servers.lock().unwrap().get(&sid).is_some_and(|s| s.server.group && s.server.is_current_member(me.id));
    if !group {
        return Err("that is not a group you are in".into());
    }
    invite_to_group(&app, &handle, &me, sid, user_ids).await
}

fn group_thread_views(app: &App, me: UserId) -> Vec<DmThreadView> {
    commy_view::dms::group_threads(app.servers.lock().unwrap().values(), me)
}

#[tauri::command]
fn list_dm_threads(app: tauri::State<App>) -> Vec<DmThreadView> {
    let me = match app.profile.lock().unwrap().as_ref() {
        Some(p) => p.id,
        None => return Vec::new(),
    };
    let groups = group_thread_views(&app, me);
    let dms = app.dms.lock().unwrap();
    let safety = app.safety.lock().unwrap();
    let mut views = dm_thread_views(&dms, me);
    views.extend(groups);
    commy_view::dms::sort_threads(&mut views, &dms, &safety, me);
    views
}

async fn send_friend_signal(app: &App, to: UserId, signal: safety::FriendSignal, note: Option<String>) -> CmdResult<()> {
    let _signing = app.signing.read().await;
    let me = current_profile(app)?;
    refuse_while_key_may_be_replaced(app)?;
    let hex = node_hex_for_user(app, to);
    let message = DmMessage::new(me.id, to, signal.encode());
    let me = match signal {
        safety::FriendSignal::Remove | safety::FriendSignal::Decline => me.without_details(),
        safety::FriendSignal::Request | safety::FriendSignal::Accept => me,
    };
    send_direct_dm(app, &me, hex.as_deref(), message, "they are offline right now; try again later", note).await
}

async fn send_signed_dm(
    app: &App,
    me: &UserProfile,
    to: UserId,
    hex: Option<&str>,
    content: String,
    offline: &str,
) -> CmdResult<()> {
    send_direct_dm(app, me, hex, DmMessage::new(me.id, to, content), offline, None).await
}

async fn send_direct_dm(
    app: &App,
    me: &UserProfile,
    hex: Option<&str>,
    mut message: DmMessage,
    offline: &str,
    note: Option<String>,
) -> CmdResult<()> {
    sign_dm(app, &mut message)?;
    let sender = app.profile_for(me, message.to);
    let to = message.to;
    let envelope = DmEnvelope::new(message, sender).with_note(note);
    let inboxed = dm_sync::post_inbox(app, to, &envelope).await;
    let Some(hex) = hex else {
        return if inboxed { Ok(()) } else { Err("that person's device is unknown".into()) };
    };
    let send = {
        let net = app.net.lock().await;
        match net.as_ref() {
            Some(node) => node.send_dm(hex, &envelope),
            None if inboxed => return Ok(()),
            None => return Err("network is not ready yet".into()),
        }
    };
    match send.await {
        Ok(()) => Ok(()),
        Err(_) if inboxed => Ok(()),
        Err(_) => Err(offline.to_string()),
    }
}

#[derive(serde::Serialize)]
struct FriendRequestView {
    id: String,
    name: String,
    note: Option<String>,
}

#[derive(serde::Serialize)]
struct FriendsView {
    friends: Vec<BlockedUserView>,
    incoming: Vec<FriendRequestView>,
    outgoing: Vec<BlockedUserView>,
}

#[tauri::command]
fn list_friends(app: tauri::State<'_, App>) -> FriendsView {
    let (friends, incoming, outgoing, notes) = {
        let s = app.safety.lock().unwrap();
        (s.friends.clone(), s.friend_in.clone(), s.friend_out.clone(), s.friend_notes.clone())
    };
    let view = |set: std::collections::HashSet<UserId>| {
        let mut list: Vec<BlockedUserView> =
            set.into_iter().map(|id| BlockedUserView { id: id.to_string(), name: known_name(&app, id) }).collect();
        list.sort_by_key(|v| v.name.to_lowercase());
        list
    };
    let incoming = view(incoming)
        .into_iter()
        .map(|v| FriendRequestView { note: v.id.parse().ok().and_then(|id| notes.get(&id).cloned()), id: v.id, name: v.name })
        .collect();
    FriendsView { friends: view(friends), incoming, outgoing: view(outgoing) }
}

#[tauri::command]
async fn friend_action(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    user_id: String,
    action: String,
    note: Option<String>,
) -> CmdResult<()> {
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
    let note = match (signal, note.as_deref()) {
        (safety::FriendSignal::Request, Some(raw)) => {
            if raw.trim().is_empty() {
                None
            } else {
                Some(safety::clean_friend_note(raw).ok_or_else(|| {
                    format!("a note can be up to {} characters", safety::MAX_FRIEND_NOTE_CHARS)
                })?)
            }
        }
        _ => None,
    };
    send_friend_signal(&app, user, signal, note).await?;
    {
        let mut s = app.safety.lock().unwrap();
        s.friend_notes.remove(&user);
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
    profile_policy: safety::ProfilePolicy,
}

fn known_name(app: &App, user: UserId) -> String {
    known_profile_field(app, user, |p| Some(p.visible_name().to_string()))
        .unwrap_or_else(|| view::UNKNOWN_USER.to_string())
}

#[tauri::command]
fn get_safety(app: tauri::State<'_, App>) -> SafetyView {
    let (blocked, dm_policy, spam_filter, profile_policy) = {
        let s = app.safety.lock().unwrap();
        (s.blocked.clone(), s.dm_policy, s.spam_filter, s.profile_policy)
    };
    SafetyView {
        blocked: blocked
            .into_iter()
            .map(|id| BlockedUserView { id: id.to_string(), name: known_name(&app, id) })
            .collect(),
        dm_policy,
        spam_filter,
        profile_policy,
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
async fn set_profile_privacy(app: tauri::State<'_, App>, handle: tauri::AppHandle, policy: String) -> CmdResult<()> {
    let policy = safety::ProfilePolicy::parse(&policy).ok_or("unknown profile setting")?;
    let changed = {
        let mut s = app.safety.lock().unwrap();
        let changed = s.profile_policy != policy;
        s.profile_policy = policy;
        changed
    };
    if !changed {
        return Ok(());
    }
    app.persist_safety();
    let me = current_profile(&app)?;
    announce_profile(&app, &handle, &me).await;
    let pronouns = app.voice.lock().unwrap().pronouns.clone().filter(|_| policy.shows_details_to_servers());
    let server_ids: Vec<ServerId> = app.servers.lock().unwrap().keys().copied().collect();
    for sid in server_ids {
        let event = ServerEvent::PronounsSet { server: sid, user: me.id, pronouns: pronouns.clone() };
        let _ = apply_and_broadcast(&app, &handle, sid, event).await;
    }
    let _ = handle.emit("servers-changed", ());
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
    let keys = channel_lock::holder_of(&app);
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
                    .and_then(|msg| keys.open(state, msg))
                    .map(|msg| msg.content)
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
    let owner_hex = node_hex_for_user(&app, owner);
    let offline = "the server owner is offline right now; try again later";
    send_signed_dm(&app, &me, owner, owner_hex.as_deref(), report.encode(), offline).await
}

fn receive_report(app: &App, handle: &tauri::AppHandle, sender: UserId, content: &str) {
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
    let pending = dm_sync::waiting_dms(&app, partner);
    let dms = app.dms.lock().unwrap();
    Ok(dms
        .posts(partner)
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

fn dm_partner(app: &App, user_id: &str) -> CmdResult<(UserProfile, UserId)> {
    let partner: UserId = user_id.parse().map_err(err_to_string)?;
    let me = current_profile(app)?;
    if partner == me.id {
        return Err("you cannot message yourself".into());
    }
    if app.safety.lock().unwrap().is_blocked(partner) {
        return Err("you blocked this person; unblock them to send a message".into());
    }
    Ok((me, partner))
}

fn known_user(app: &App, user: UserId) -> bool {
    known_profile_field(app, user, |_| Some(())).is_some()
        || app.servers.lock().unwrap().values().any(|s| s.server.is_current_member(user))
}

fn dm_recipient(app: &App, user_id: &str) -> CmdResult<(UserProfile, UserId)> {
    let (me, partner) = dm_partner(app, user_id)?;
    if !known_user(app, partner) {
        return Err("you can only message people you share a server with or who have messaged you".into());
    }
    Ok((me, partner))
}

async fn post_dm(
    app: &App,
    handle: &tauri::AppHandle,
    me: &UserProfile,
    partner: UserId,
    message: DmMessage,
) -> CmdResult<Option<DmMessageView>> {
    post_dm_with(handle, me, partner, message, |envelope| async move {
        let inboxed = dm_sync::post_inbox(app, partner, &envelope).await;
        outbox::try_send_dm(app, partner, &envelope).await || inboxed
    })
    .await
}

async fn post_dm_with<A, F, Fut>(
    access: &A,
    me: &UserProfile,
    partner: UserId,
    mut message: DmMessage,
    send: F,
) -> CmdResult<Option<DmMessageView>>
where
    A: catchup::AppAccess,
    F: FnOnce(DmEnvelope) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let app = access.app();
    let _signing = app.signing.read().await;
    sign_dm(app, &mut message)?;
    if !app.dms.lock().unwrap().apply(partner, message.clone()) {
        return Err(if message.is_post() { "message not sent" } else { "nothing to change" }.into());
    }
    let envelope = DmEnvelope::new(message.clone(), app.profile_for(me, partner));
    let waiting = app.outbox.lock().unwrap().has_dms(partner);
    outbox::queue_dm(app, partner, message.id);
    app.persist_dms();
    access.emit("dm-sending", partner.to_string().into());
    let stored = dm_sync::publish(app, partner, &envelope).await;
    if stored {
        outbox::dm_held(app, partner, message.id);
    }
    let held = stored && dm_sync::partner_follows(app, partner);
    let jumps = !commy_view::dms::syncable(&message);
    let delivered = !held && (!waiting || jumps) && send(envelope).await;
    if delivered {
        outbox::dm_delivered(app, partner, message.id);
    } else if waiting || held {
        outbox::wake_dm(app, partner);
    }

    let view = {
        let dms = app.dms.lock().unwrap();
        message.is_post().then(|| dm_message_view(&message, me.id, &dms, !delivered && !held))
    };
    if app.safety.lock().unwrap().accepted.insert(partner) {
        app.persist_safety();
    }
    app.persist_dms();
    access.emit("dm-changed", partner.to_string().into());
    Ok(view)
}

#[tauri::command]
async fn send_dm(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    user_id: String,
    content: String,
    reply_to: Option<String>,
) -> CmdResult<DmMessageView> {
    let content = clean_message(&content)?;
    let (me, partner) = dm_recipient(&app, &user_id)?;
    let content = carry_expressions(&app, content, &std::collections::HashSet::new()).await;
    let mut message = DmMessage::new(me.id, partner, content);
    if let Some(raw) = reply_to {
        let target: MessageId = raw.parse().map_err(err_to_string)?;
        if app.dms.lock().unwrap().post(partner, target).is_none() {
            return Err("the message you replied to no longer exists".into());
        }
        message.reply_to = Some(target);
    }
    let view = post_dm(&app, &handle, &me, partner, message).await?;
    view.ok_or_else(|| "message not sent".to_string())
}

#[tauri::command]
async fn edit_dm(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    user_id: String,
    message_id: String,
    content: String,
) -> CmdResult<()> {
    let content = clean_message(&content)?;
    let (me, partner) = dm_partner(&app, &user_id)?;
    let target: MessageId = message_id.parse().map_err(err_to_string)?;
    {
        let dms = app.dms.lock().unwrap();
        let original = dms.post(partner, target).ok_or("that message no longer exists")?;
        if original.from != me.id {
            return Err("you can only edit your own messages".into());
        }
    }
    let content = carry_expressions(&app, content, &std::collections::HashSet::new()).await;
    let message = DmMessage::acting(me.id, partner, DmAction::Edit { target }, content);
    post_dm(&app, &handle, &me, partner, message).await.map(|_| ())
}

#[tauri::command]
async fn delete_dm(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    user_id: String,
    message_id: String,
) -> CmdResult<()> {
    let (me, partner) = dm_partner(&app, &user_id)?;
    let target: MessageId = message_id.parse().map_err(err_to_string)?;
    {
        let dms = app.dms.lock().unwrap();
        let original = dms.post(partner, target).ok_or("that message no longer exists")?;
        if original.from != me.id {
            return Err("you can only delete your own messages".into());
        }
    }
    let message = DmMessage::acting(me.id, partner, DmAction::Delete { target }, "");
    post_dm(&app, &handle, &me, partner, message).await.map(|_| ())
}

#[tauri::command]
async fn set_dm_reaction(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    user_id: String,
    message_id: String,
    emoji: String,
    active: bool,
) -> CmdResult<()> {
    let emoji = emoji.trim().to_string();
    if !commy_core::server::valid_reaction(&emoji) {
        return Err("reactions must be a single emoji".into());
    }
    let (me, partner) = dm_partner(&app, &user_id)?;
    let target: MessageId = message_id.parse().map_err(err_to_string)?;
    let message = DmMessage::acting(me.id, partner, DmAction::React { target, emoji, active }, "");
    post_dm(&app, &handle, &me, partner, message).await.map(|_| ())
}

async fn send_dm_attachment(
    app: &App,
    handle: &tauri::AppHandle,
    user_id: &str,
    filename: String,
    bytes: Vec<u8>,
) -> CmdResult<DmMessageView> {
    if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
        return Err(ATTACHMENT_TOO_LARGE.into());
    }
    let (me, partner) = dm_partner(app, user_id)?;
    let size_bytes = bytes.len() as u64;
    let (blob, provider) = store_blob(app, bytes).await?;
    let mime_type = mime_guess::from_path(&filename)
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_string();
    let mut message = DmMessage::new(me.id, partner, String::new());
    message.attachments.push(Attachment { blob, filename, size_bytes, mime_type, provider });
    let view = post_dm(app, handle, &me, partner, message).await?;
    view.ok_or_else(|| "file not sent".to_string())
}

#[tauri::command]
async fn attach_dm_file(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    user_id: String,
    kind: Option<String>,
) -> CmdResult<Option<DmMessageView>> {
    dm_partner(&app, &user_id)?;
    let (title, filter) = match kind.as_deref() {
        Some("video") => ("Send a video", Some(picker::Filter { label: "Videos", extensions: INLINE_VIDEO_EXTENSIONS })),
        _ => ("Send a file", None),
    };
    let Some(picked) = picker::pick_file(&handle, title, filter, MAX_ATTACHMENT_BYTES, ATTACHMENT_TOO_LARGE).await? else {
        return Ok(None);
    };
    send_dm_attachment(&app, &handle, &user_id, picked.name, picked.bytes).await.map(Some)
}

#[tauri::command]
async fn attach_dm_bytes(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    user_id: String,
    filename: String,
    data: String,
) -> CmdResult<DmMessageView> {
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
    send_dm_attachment(&app, &handle, &user_id, filename, bytes).await
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
    manage_check(app, sid, me.id, Permissions::CREATE_INVITES, "invite people")
}

pub(crate) fn require_manage_server(app: &App, sid: ServerId, action: &str) -> CmdResult<()> {
    let me = current_profile(app)?;
    manage_check(app, sid, me.id, Permissions::MANAGE_SERVER, action)
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
    app: &App,
    handle: &tauri::AppHandle,
    sid: ServerId,
    event: ServerEvent,
) -> CmdResult<()> {
    {
        let mut servers = app.servers.lock().unwrap();
        let state = servers.get_mut(&sid).ok_or("unknown server")?;
        match app.sign_event(sid, event.clone()) {
            Some(signed) => state.apply_own(&signed).map_err(err_to_string)?,
            None => state.apply(event.clone()).map_err(err_to_string)?,
        };
    }
    outbox::broadcast_local(app, sid, event).await;
    app.persist_server(sid);
    directory::server_changed(app, sid);
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
    let name = clean_server_name(&name)?;
    let me = current_profile(&app)?;
    manage_check(&app, sid, me.id, Permissions::MANAGE_SERVER, "rename the server")?;
    apply_and_broadcast(&app, &handle, sid, ServerEvent::ServerRename { server: sid, name })
        .await?;
    let _ = handle.emit("servers-changed", ());
    Ok(())
}

fn structure_announcements(app: &App, sid: ServerId, me: UserId) -> Vec<ServerEvent> {
    let shows_pronouns = app.safety.lock().unwrap().profile_policy.shows_details_to_servers();
    let pronouns = app.voice.lock().unwrap().pronouns.clone().filter(|_| shows_pronouns);
    let servers = app.servers.lock().unwrap();
    let Some(state) = servers.get(&sid) else { return Vec::new() };
    let perms = our_permissions(app, state, me);
    let mut events = Vec::new();
    if perms.allows(Permissions::MANAGE_CHANNELS) {
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
    if (state.server.icon.is_some() || state.server.banner.is_some()) && perms.allows(Permissions::MANAGE_SERVER) {
        events.push(ServerEvent::ServerImages {
            server: sid,
            icon: state.server.icon.clone(),
            banner: state.server.banner.clone(),
        });
    }
    for emoji in &state.server.emojis {
        events.push(ServerEvent::EmojiAdd { server: sid, emoji: emoji.clone() });
    }
    for sticker in &state.server.stickers {
        events.push(ServerEvent::StickerAdd { server: sid, sticker: sticker.clone() });
    }
    for emoticon in &state.server.emoticons {
        events.push(ServerEvent::EmoticonAdd { server: sid, emoticon: emoticon.clone() });
    }
    for sound in &state.server.soundboard {
        events.push(ServerEvent::SoundAdd { server: sid, sound: sound.clone() });
    }
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
        if state.server.onboarding.is_some() {
            events.push(ServerEvent::OnboardingSet { server: sid, onboarding: state.server.onboarding.clone() });
        }
        if state.server.afk_timeout_minutes.is_some() {
            events.push(ServerEvent::AfkTimeoutSet { server: sid, minutes: state.server.afk_timeout_minutes });
        }
    }
    if state.server.rules_accepted.contains(&me) {
        events.push(ServerEvent::RulesAccept { server: sid, user: me });
    }
    if let Some(pronouns) = pronouns {
        events.push(ServerEvent::PronounsSet { server: sid, user: me, pronouns: Some(pronouns) });
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
            if channel.sensitive {
                events.push(ServerEvent::ChannelSensitiveSet { server: sid, channel: channel.id, sensitive: true });
            }
            if channel.is_restricted() {
                events.push(ServerEvent::ChannelAccessSet {
                    server: sid,
                    channel: channel.id,
                    view: channel.view_roles.clone(),
                    send: channel.send_roles.clone(),
                });
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
        let mut seen = std::collections::HashSet::new();
        for user in moderated {
            if !seen.insert(*user) {
                continue;
            }
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
    if custom && perms.allows(Permissions::MANAGE_SERVER) {
        events.push(ServerEvent::ExpressionSettings {
            server: sid,
            emojis_shareable: state.server.emojis_shareable,
            soundboard_open: state.server.soundboard_open,
        });
    }
    if state.server.sounds_shareable && perms.allows(Permissions::MANAGE_SERVER) {
        events.push(ServerEvent::SoundSharingSet { server: sid, shareable: true });
    }
    if state.server.activities_enabled && perms.allows(Permissions::MANAGE_SERVER) {
        events.push(ServerEvent::ActivitiesSet { server: sid, enabled: true });
    }
    let sharing = state.server.stickers_shareable || state.server.emoticons_shareable;
    if sharing && perms.allows(Permissions::MANAGE_SERVER) {
        events.push(ServerEvent::ExpressionSharingSet {
            server: sid,
            stickers: state.server.stickers_shareable,
            emoticons: state.server.emoticons_shareable,
        });
    }
    if state.server.public && perms.allows(Permissions::MANAGE_SERVER) {
        events.push(ServerEvent::PublicSet { server: sid, public: true });
    }
    let explicit = state.server.afk_disabled || state.server.afk_channel_id.is_some();
    if explicit && perms.allows(Permissions::MANAGE_SERVER) {
        events.push(ServerEvent::AfkChannelSet {
            server: sid,
            channel: state.server.afk_channel(),
        });
    }
    events.retain(|event| commy_core::action::authorized(&state.server, me, event));
    events
}


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
    let name = check_channel_name(match kind {
        kind if kind.holds_messages() => text_channel_name(&name),
        _ => name.trim().to_string(),
    })?;
    let retention_days = retention_days.filter(|d| *d > 0).map(|d| d.min(MAX_RETENTION_DAYS));
    apply_and_broadcast(
        &app,
        &handle,
        sid,
        ServerEvent::ChannelUpdate { server: sid, channel: cid, name, retention_days },
    ).await
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
        trim_history(&handle);
        tokio::time::sleep(RETENTION_PRUNE_EVERY).await;
    }
}

fn trim_history(handle: &tauri::AppHandle) {
    let app = handle.state::<App>();
    let storage = app.storage.lock().unwrap().clone();
    let pruned: Vec<ServerId> = {
        let mut servers = app.servers.lock().unwrap();
        let now = chrono::Utc::now();
        servers
            .values_mut()
            .filter_map(|s| {
                let floor = storage.floor(s.server.id, now);
                let removed = s.prune_expired(now) + s.roll_off(floor);
                (removed > 0).then_some(s.server.id)
            })
            .collect()
    };
    for sid in pruned {
        app.persist_server(sid);
        let _ = handle.emit("server-event", &sid.to_string());
    }
}

#[tauri::command]
fn get_storage_settings(app: tauri::State<'_, App>) -> storage::StorageSettings {
    app.storage.lock().unwrap().clone()
}

#[tauri::command]
fn set_history_window(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: Option<String>,
    keep: Option<String>,
) -> CmdResult<storage::StorageSettings> {
    let server: Option<ServerId> = server_id.as_deref().map(str::parse).transpose().map_err(err_to_string)?;
    let window = keep
        .as_deref()
        .map(|raw| storage::KeepWindow::parse(raw).ok_or("unknown history window"))
        .transpose()?;
    let settings = {
        let mut storage = app.storage.lock().unwrap();
        storage.set(server, window)?;
        storage.clone()
    };
    app.persist_storage();
    trim_history(&handle);
    Ok(settings)
}

fn forget_history_window(app: &App, sid: ServerId) {
    if app.storage.lock().unwrap().forget(sid) {
        app.persist_storage();
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
    let channel: Option<ChannelId> =
        channel_id.as_deref().filter(|c| !c.is_empty()).map(str::parse).transpose().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    manage_check(&app, sid, me.id, Permissions::MANAGE_SERVER, "change the AFK channel")?;
    apply_and_broadcast(&app, &handle, sid, ServerEvent::AfkChannelSet { server: sid, channel }).await
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
    let name = check_channel_name(text_channel_name(&name))?;
    let kind = match kind.as_str() {
        "text" => ChannelKind::Text,
        "voice" => ChannelKind::Voice,
        "announcement" => ChannelKind::Announcement,
        "forum" => ChannelKind::Forum,
        "stage" => ChannelKind::Stage,
        _ => return Err("channel kind must be text, voice, stage, announcement or forum".into()),
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

    broadcast_event(&app, sid, ServerEvent::MemberLeave { server: sid, user: me.id })
    .await;
    leave_server_topic(&app, sid).await;
    forget_server_locally(&app, sid);
    entry_list::record_leave(&app, sid);
    entry_list::schedule(&handle);
    let _ = handle.emit("servers-changed", ());
    Ok(())
}

pub(crate) async fn leave_from_another_device(app: &App, sid: ServerId) {
    end_voice_in(app, sid);
    leave_server_topic(app, sid).await;
    forget_server_locally(app, sid);
}

async fn leave_server_topic(app: &App, sid: ServerId) {
    let mut net = app.net.lock().await;
    if let Some(node) = net.as_mut() {
        node.leave_server(sid);
    }
}

fn forget_server_locally(app: &App, sid: ServerId) -> Option<ServerState> {
    let removed = app.servers.lock().unwrap().remove(&sid);
    app.invites.lock().unwrap().remove(&sid);
    forget_server_outbox(app, sid);
    forget_history_window(app, sid);
    if let Some(dir) = app.data_dir() {
        let _ = persist::delete_server(dir, sid);
    }
    removed
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
        Some(e) if commy_core::user::valid_status_emoji(&e) => Some(Some(carry_profile_emojis(&app, e).await)),
        Some(_) => return Err("status emoji must be a single emoji".into()),
    };
    let clean = |s: Option<String>| {
        s.map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && commy_core::user::rich_text_fits(s, commy_core::user::MAX_STATUS_CHARS))
    };
    let custom_status = match clean(custom_status) {
        Some(status) => Some(carry_profile_emojis(&app, status).await),
        None => None,
    };
    let display_name = clean(display_name).filter(|name| name.len() <= 64);
    let profile = edit_profile(&app, |profile| {
        profile.display_name = display_name;
        profile.custom_status = custom_status;
        if presence != profile.presence {
            profile.presence_until = None;
        }
        profile.presence = presence;
        if let Some(emoji) = status_emoji {
            profile.status_emoji = emoji;
        }
    })?;
    app.auto_idle.store(false, std::sync::atomic::Ordering::Relaxed);
    publish_profile(&app, &handle, &profile).await;
    entry_list::record_profile_edit(&app);
    entry_list::schedule(&handle);
    Ok(profile)
}

#[tauri::command]
async fn set_pronouns(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    pronouns: Option<String>,
) -> CmdResult<VoiceSettings> {
    let me = current_profile(&app)?;
    let cleaned = match pronouns.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        None => None,
        Some(text) => Some(commy_core::user::clean_pronouns(text).ok_or_else(|| {
            format!("pronouns can be up to {} characters", commy_core::user::MAX_PRONOUNS_CHARS)
        })?),
    };
    let settings = app.update_voice(|voice| voice.pronouns = cleaned.clone());
    if !app.safety.lock().unwrap().profile_policy.shows_details_to_servers() {
        return Ok(settings);
    }
    let server_ids: Vec<ServerId> = app.servers.lock().unwrap().keys().copied().collect();
    for sid in server_ids {
        let event = ServerEvent::PronounsSet { server: sid, user: me.id, pronouns: cleaned.clone() };
        let _ = apply_and_broadcast(&app, &handle, sid, event).await;
    }
    let _ = handle.emit("servers-changed", ());
    Ok(settings)
}

#[tauri::command]
fn set_icons(
    app: tauri::State<App>,
    handle: tauri::AppHandle,
    app_icon: String,
    tray_icon: String,
) -> CmdResult<VoiceSettings> {
    if !app_icon::valid_choice("app", &app_icon) || !app_icon::valid_choice("tray", &tray_icon) {
        return Err("unknown icon".into());
    }
    let settings = app.update_voice(|voice| {
        voice.app_icon = app_icon.clone();
        voice.tray_icon = tray_icon.clone();
    });
    app_icon::apply(&handle, app.data_dir().map(|d| d.as_path()), &app_icon, &tray_icon);
    Ok(settings)
}

#[tauri::command]
async fn choose_custom_icon(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    target: String,
) -> CmdResult<Option<VoiceSettings>> {
    if !app_icon::TARGETS.contains(&target.as_str()) {
        return Err("unknown icon".into());
    }
    let filter = picker::Filter { label: "PNG image", extensions: &["png"] };
    let Some(picked) = picker::pick_file(&handle, "Choose an icon", Some(filter), MAX_PICKED_IMAGE_BYTES, IMAGE_TOO_LARGE).await? else {
        return Ok(None);
    };
    let bytes = picked.bytes;
    let dir = app.data_dir().cloned().ok_or("storage is not ready yet")?;
    app_icon::save_custom(&dir, &target, &bytes)?;
    let settings = app.update_voice(|voice| {
        if target == "app" {
            voice.app_icon = app_icon::CUSTOM.to_string();
        } else {
            voice.tray_icon = app_icon::CUSTOM.to_string();
        }
    });
    app_icon::apply(&handle, Some(dir.as_path()), &settings.app_icon, &settings.tray_icon);
    Ok(Some(settings))
}

#[tauri::command]
fn get_custom_icon(app: tauri::State<App>, target: String) -> Option<String> {
    let dir = app.data_dir()?;
    if !app_icon::TARGETS.contains(&target.as_str()) {
        return None;
    }
    let bytes = std::fs::read(app_icon::custom_path(dir, &target)).ok()?;
    Some(data_url("image/png", &bytes))
}

#[tauri::command]
fn set_window_title(handle: tauri::AppHandle, server: Option<String>) {
    let label = std::env::var("COMMY_INSTANCE_LABEL").ok();
    let base = instance::window_title(instance::TEST_BUILD, label.as_deref()).unwrap_or_else(|| "Commy".to_string());
    if let Some(window) = handle.get_webview_window("main") {
        let _ = window.set_title(&instance::server_window_title(&base, server.as_deref()));
    }
}

#[tauri::command]
fn has_custom_titlebar() -> bool {
    cfg!(windows)
}

#[cfg(desktop)]
#[tauri::command]
fn window_action(window: tauri::WebviewWindow, action: String) {
    match action.as_str() {
        "minimize" => {
            let _ = window.minimize();
        }
        "toggle-maximize" => {
            if window.is_maximized().unwrap_or(false) {
                let _ = window.unmaximize();
            } else {
                let _ = window.maximize();
            }
        }
        "close" => {
            let _ = window.close();
        }
        "drag" => {
            let _ = window.start_dragging();
        }
        _ => {}
    }
}

#[cfg(mobile)]
#[tauri::command]
fn window_action(_action: String) {}

#[tauri::command]
async fn set_status_expiry(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    presence_until: Option<i64>,
    status_until: Option<i64>,
) -> CmdResult<VoiceSettings> {
    let now = chrono::Utc::now().timestamp();
    let future = |t: Option<i64>| t.filter(|t| *t > now);
    let settings = app.update_voice(|voice| {
        voice.presence_until = future(presence_until);
        voice.status_until = future(status_until);
    });
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
        publish_profile(&app, &handle, &profile).await;
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
        app.update_voice(|voice| {
            if presence {
                voice.presence_until = None;
            }
            if status {
                voice.status_until = None;
            }
        });
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
        publish_profile(&app, &handle, &profile).await;
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;
    use view::act::{edit_time_after_last_edit, sign_edit};

    #[test]
    fn only_known_people_can_be_messaged() {
        let app = App::default();
        let stranger = UserProfile::new("stranger");
        assert!(!known_user(&app, stranger.id), "an id nobody has seen is refused");
        app.dms.lock().unwrap().contacts.insert(stranger.id, stranger.clone());
        assert!(known_user(&app, stranger.id), "someone who has messaged you is known");
    }

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
mod dm_send_tests {
    use super::*;
    use crate::catchup::TestHandle;
    use std::sync::Arc;
    use std::time::Instant;

    fn signed_in() -> (Arc<App>, UserProfile) {
        let app = Arc::new(App::default());
        let identity = Identity::generate();
        let mut me = UserProfile::new("me");
        me.identity = Some(identity.public());
        *app.profile.lock().unwrap() = Some(me.clone());
        *app.identity.lock().unwrap() = Some(identity);
        (app, me)
    }

    fn event_names(handle: &TestHandle) -> Vec<String> {
        handle.take_events().into_iter().map(|(name, _)| name).collect()
    }

    #[tokio::test]
    async fn a_dm_shows_as_waiting_while_the_friend_is_still_being_reached() {
        let (app, me) = signed_in();
        let handle = TestHandle::new(app.clone());
        let partner = UserId::new();
        let message = DmMessage::new(me.id, partner, "are you there?");
        let id = message.id;

        let view = post_dm_with(&handle, &me, partner, message, |envelope: DmEnvelope| {
            assert_eq!(envelope.message.id, id);
            assert!(app.outbox.lock().unwrap().pending_dms(partner).contains(&id));
            assert!(app.dms.lock().unwrap().post(partner, id).is_some());
            assert_eq!(event_names(&handle), vec!["dm-sending"]);
            async { true }
        })
        .await
        .unwrap()
        .unwrap();

        assert!(!view.pending);
        assert!(!app.outbox.lock().unwrap().has_dms(partner));
        assert!(app.dm_backoff.lock().unwrap().due(partner, Instant::now()));
        assert_eq!(event_names(&handle), vec!["dm-changed"]);
    }

    #[tokio::test]
    async fn a_dm_to_an_offline_friend_stays_waiting_in_the_outbox() {
        let (app, me) = signed_in();
        let handle = TestHandle::new(app.clone());
        let partner = UserId::new();
        let message = DmMessage::new(me.id, partner, "see you later");
        let id = message.id;

        let view = post_dm_with(&handle, &me, partner, message, |_| async { false }).await.unwrap().unwrap();

        assert!(view.pending);
        assert_eq!(app.outbox.lock().unwrap().dm_queue(partner), vec![id]);
        assert!(!app.dm_backoff.lock().unwrap().due(partner, Instant::now()));
        assert_eq!(event_names(&handle), vec!["dm-sending", "dm-changed"]);
    }

    #[tokio::test]
    async fn a_dm_behind_waiting_ones_queues_without_jumping_ahead() {
        let (app, me) = signed_in();
        let handle = TestHandle::new(app.clone());
        let partner = UserId::new();
        let first = DmMessage::new(me.id, partner, "first");
        let first_id = first.id;
        post_dm_with(&handle, &me, partner, first, |_| async { false }).await.unwrap();
        handle.take_events();

        let second = DmMessage::new(me.id, partner, "second");
        let second_id = second.id;
        let view = post_dm_with(&handle, &me, partner, second, |_| async { panic!("sent ahead of the queue") })
            .await
            .unwrap()
            .unwrap();

        assert!(view.pending);
        assert_eq!(app.outbox.lock().unwrap().dm_queue(partner), vec![first_id, second_id]);
        assert_eq!(event_names(&handle), vec!["dm-sending", "dm-changed"]);
    }

    #[tokio::test]
    async fn a_group_invite_goes_out_at_once_even_behind_waiting_dms() {
        let (app, me) = signed_in();
        let handle = TestHandle::new(app.clone());
        let partner = UserId::new();
        let first = DmMessage::new(me.id, partner, "first");
        let first_id = first.id;
        post_dm_with(&handle, &me, partner, first, |_| async { false }).await.unwrap();

        let invite = commy_view::dms::group_invite(me.id, partner, "ticket".into(), None);
        let sent = std::sync::Mutex::new(Vec::new());
        post_dm_with(&handle, &me, partner, invite.clone(), |envelope: DmEnvelope| {
            sent.lock().unwrap().push(envelope.message.id);
            async { true }
        })
        .await
        .unwrap();

        assert_eq!(*sent.lock().unwrap(), vec![invite.id]);
        assert_eq!(app.outbox.lock().unwrap().dm_queue(partner), vec![first_id]);
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

        assert_eq!(view::act::may_delete(&state, our_permissions(&app, &state, me.id), me.id, channel, ours.id), Ok(()));
        let refused = Err("you do not have permission to delete other people's messages".to_string());
        assert_eq!(view::act::may_delete(&state, our_permissions(&app, &state, me.id), me.id, channel, theirs.id), refused);
        let moderation = [
            Permissions::MANAGE_SERVER,
            Permissions::MANAGE_CHANNELS,
            Permissions::KICK_MEMBERS,
            Permissions::BAN_MEMBERS,
            Permissions::MUTE_MEMBERS,
            Permissions::MOVE_MEMBERS,
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
        assert_eq!(view::act::may_delete(&state, our_permissions(&app, &state, me.id), me.id, channel, theirs.id), Ok(()));
        for needed in moderation {
            assert!(require_permission(&app, &state, me.id, needed, "moderate").is_ok(), "{needed:?}");
        }
        assert!(structural(structure_announcements(&app, sid, me.id)));
    }

    #[test]
    fn a_plain_member_re_announces_only_what_it_may_change() {
        let owner = UserProfile::new("owner");
        let mut server = commy_core::Server::new("s", owner.id);
        server.emojis.push(commy_core::server::CustomEmoji { name: "wave".into(), blob: format!("{}@{}", "a1".repeat(32), "b2".repeat(32)) });
        server.events.push(commy_core::server::ScheduledEvent {
            id: "night".into(),
            title: "Game night".into(),
            description: String::new(),
            start: chrono::Utc::now(),
            end: None,
            channel: None,
            creator: owner.id,
            rsvps: vec![(owner.id, commy_core::server::Rsvp::Going)],
        });
        let sid = server.id;
        let mut state = ServerState::new(server);
        state.profiles.insert(owner.id, owner.clone());
        let me = UserProfile::new("me");
        state.apply(ServerEvent::MemberJoin { server: sid, profile: me.clone() }).unwrap();
        state.apply(ServerEvent::EventRsvp { server: sid, event: "night".into(), user: me.id, rsvp: Some(commy_core::server::Rsvp::Interested) }).unwrap();
        let app = App::default();
        app.servers.lock().unwrap().insert(sid, state.clone());
        app.set_awaiting_snapshot(sid, false);

        let mine = structure_announcements(&app, sid, me.id);
        assert!(mine.iter().all(|e| commy_core::action::authorized(&state.server, me.id, e)), "{mine:?}");
        assert!(mine.iter().any(|e| matches!(e, ServerEvent::EventRsvp { user, .. } if *user == me.id)));
        assert!(!mine.iter().any(|e| matches!(e, ServerEvent::EmojiAdd { .. } | ServerEvent::EventUpsert { .. })));
        assert!(!mine.iter().any(|e| matches!(e, ServerEvent::EventRsvp { user, .. } if *user == owner.id)));

        let theirs = structure_announcements(&app, sid, owner.id);
        assert!(theirs.iter().all(|e| commy_core::action::authorized(&state.server, owner.id, e)), "{theirs:?}");
        assert!(theirs.iter().any(|e| matches!(e, ServerEvent::EmojiAdd { .. })));
        assert!(theirs.iter().any(|e| matches!(e, ServerEvent::EventUpsert { .. })));
    }
}

#[cfg(test)]
mod shared_image_tests {
    use super::*;

    #[test]
    fn images_come_from_the_uploader_first_then_from_members_online() {
        let (a, b) = ([1u8; 32], [2u8; 32]);
        let hex = |n: [u8; 32]| data_encoding::HEXLOWER.encode(&n);
        assert_eq!(image_sources(&hex(b), vec![a, b, a]), vec![hex(b), hex(a)]);
        assert_eq!(image_sources("away", vec![a]), vec![hex(a), "away".to_string()], "an absent uploader is tried last");
        assert_eq!(image_sources("away", vec![]), vec!["away".to_string()]);
    }

    #[test]
    fn members_pass_on_signed_changes_as_their_authors_signed_them() {
        let owner_key = Identity::generate();
        let mut owner = UserProfile::new("owner");
        owner.identity = Some(owner_key.public());
        let member_key = Identity::generate();
        let mut member = UserProfile::new("member");
        member.identity = Some(member_key.public());
        let server = commy_core::Server::new("s", owner.id);
        let sid = server.id;
        let mut state = ServerState::new(server);
        state.profiles.insert(owner.id, owner.clone());
        state.apply(ServerEvent::MemberJoin { server: sid, profile: member.clone() }).unwrap();
        let emoji = commy_core::server::CustomEmoji { name: "wave".into(), blob: format!("{}@{}", "b".repeat(52), "0".repeat(64)) };
        let add = SignedEvent::sign(sid, owner.id, 1_000, ServerEvent::EmojiAdd { server: sid, emoji }, &owner_key);
        state.apply_signed(&add).unwrap();

        let app = App::default();
        app.servers.lock().unwrap().insert(sid, state);
        *app.profile.lock().unwrap() = Some(member.clone());
        *app.identity.lock().unwrap() = Some(member_key);
        let relayed: Vec<SignedEvent> = neighbor_announcements(&app, sid, member.id)
            .into_iter()
            .filter_map(|p| match p {
                WirePayload::Signed(signed) if matches!(signed.event, ServerEvent::EmojiAdd { .. }) => Some(signed),
                _ => None,
            })
            .collect();
        assert_eq!(relayed, vec![add.clone()], "the owner's own signature, once");

        *app.profile.lock().unwrap() = Some(owner.clone());
        *app.identity.lock().unwrap() = Some(owner_key);
        let emojis = neighbor_announcements(&app, sid, owner.id)
            .into_iter()
            .filter(|p| matches!(p, WirePayload::Signed(s) if matches!(s.event, ServerEvent::EmojiAdd { .. })))
            .count();
        assert_eq!(emojis, 1, "what we hold signed is not signed again");
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

#[cfg(mobile)]
fn register_hotkeys(_handle: &tauri::AppHandle, _voice: &VoiceSettings) {}

#[cfg(desktop)]
fn register_hotkeys(handle: &tauri::AppHandle, voice: &VoiceSettings) {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let shortcuts = handle.global_shortcut();
    let _ = shortcuts.unregister_all();
    let mut taken = Vec::new();
    let overlay = &voice.overlay;
    let mut wanted: Vec<&str> = Vec::new();
    if overlay.enabled {
        wanted.extend([overlay.toggle_hotkey.as_str(), overlay.chat_hotkey.as_str()]);
    }
    let keys = voice.voice_keys();
    wanted.extend(keys.registered());
    for key in wanted {
        if let Err(e) = shortcuts.register(key) {
            tracing::warn!("could not register hotkey {key}: {e}");
            taken.push(key.to_string());
        }
    }
    *handle.state::<App>().overlay_hotkeys_taken.lock().unwrap() = taken;
}

fn taken_hotkeys(app: &App, keys: &[&str]) -> Vec<String> {
    let taken = app.overlay_hotkeys_taken.lock().unwrap();
    keys.iter().filter(|k| taken.iter().any(|t| t.eq_ignore_ascii_case(k))).map(|k| k.to_string()).collect()
}

#[tauri::command]
fn overlay_hotkey_problems(app: tauri::State<'_, App>) -> Vec<String> {
    let overlay = app.voice.lock().unwrap().overlay.clone();
    taken_hotkeys(&app, &[&overlay.toggle_hotkey, &overlay.chat_hotkey])
}

#[tauri::command]
fn voice_hotkey_problems(app: tauri::State<'_, App>) -> Vec<String> {
    let keys = app.voice.lock().unwrap().voice_keys();
    taken_hotkeys(&app, &keys.registered())
}

#[cfg(desktop)]
fn hotkey_id(key: &str) -> Option<u32> {
    key.parse::<tauri_plugin_global_shortcut::Shortcut>().ok().map(|s| s.id())
}

#[cfg(desktop)]
fn on_hotkey(handle: &tauri::AppHandle, id: u32, pressed: bool) {
    let app = handle.state::<App>();
    let voice = app.voice.lock().unwrap().clone();
    let keys = voice.voice_keys();
    if keys.talk.as_deref().and_then(hotkey_id) == Some(id) {
        set_talk_key(handle, pressed, voice.talk_release_ms);
    } else if !pressed {
    } else if keys.mute.as_deref().and_then(hotkey_id) == Some(id) {
        let handle = handle.clone();
        tauri::async_runtime::spawn(async move {
            let app = handle.state::<App>();
            let muted = !app.voice.lock().unwrap().muted;
            match set_muted(app, handle.clone(), muted).await {
                Ok(snapshot) => emit_voice_settings(&handle, &snapshot),
                Err(e) => tracing::warn!("mute hotkey: {e}"),
            }
        });
    } else if keys.deafen.as_deref().and_then(hotkey_id) == Some(id) {
        let handle = handle.clone();
        tauri::async_runtime::spawn(async move {
            let app = handle.state::<App>();
            let deafened = !app.voice.lock().unwrap().deafened;
            match set_deafened(app, handle.clone(), deafened).await {
                Ok(snapshot) => emit_voice_settings(&handle, &snapshot),
                Err(e) => tracing::warn!("deafen hotkey: {e}"),
            }
        });
    } else {
        on_overlay_hotkey(handle, id);
    }
}

#[cfg(desktop)]
fn emit_voice_settings(handle: &tauri::AppHandle, voice: &VoiceSettings) {
    let _ = handle.emit("voice-settings-changed", voice);
}

#[cfg(desktop)]
fn set_talk_key(handle: &tauri::AppHandle, pressed: bool, release_ms: u32) {
    let app = handle.state::<App>();
    let press = app.talk_press.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    if pressed {
        app.processor.lock().unwrap().set_talk_key_down(true);
        let _ = handle.emit("talk-key", true);
        return;
    }
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(release_ms as u64)).await;
        let app = handle.state::<App>();
        if app.talk_press.load(std::sync::atomic::Ordering::SeqCst) != press {
            return;
        }
        app.processor.lock().unwrap().set_talk_key_down(false);
        let _ = handle.emit("talk-key", false);
    });
}

#[tauri::command]
fn set_input_mode(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    mode: String,
    talk_key: String,
    release_ms: u32,
) -> CmdResult<VoiceSettings> {
    let mode = voice_keys::InputMode::parse(&mode).ok_or("unknown input mode")?;
    let talk_key = talk_key.trim().to_string();
    if !overlay::valid_hotkey(&talk_key) {
        return Err(format!("{talk_key} is not a key Commy can use for push to talk"));
    }
    let snapshot = app.update_voice(|voice| {
        voice.input_mode = mode;
        voice.talk_key = talk_key;
        voice.talk_release_ms = voice_keys::clamp_release_ms(release_ms);
    });
    if let Some(clash) = voice_keys::clashing(&all_hotkeys(&snapshot)) {
        return Err(format!("{clash} is already used by another Commy hotkey"));
    }
    let _ = app.talk_press.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    app.processor.lock().unwrap().set_push_to_talk(mode == voice_keys::InputMode::PushToTalk);
    register_hotkeys(&handle, &snapshot);
    Ok(snapshot)
}

fn all_hotkeys(voice: &VoiceSettings) -> Vec<String> {
    let mut keys: Vec<String> = voice.voice_keys().registered().into_iter().map(str::to_string).collect();
    if voice.overlay.enabled {
        keys.extend([voice.overlay.toggle_hotkey.clone(), voice.overlay.chat_hotkey.clone()]);
    }
    keys
}

#[tauri::command]
fn set_voice_hotkeys(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    mute: String,
    deafen: String,
) -> CmdResult<VoiceSettings> {
    let mute = voice_keys::validate_optional_hotkey(&mute, "mute")?;
    let deafen = voice_keys::validate_optional_hotkey(&deafen, "deafen")?;
    let snapshot = app.update_voice(|voice| {
        voice.mute_hotkey = mute;
        voice.deafen_hotkey = deafen;
    });
    if let Some(clash) = voice_keys::clashing(&all_hotkeys(&snapshot)) {
        return Err(format!("{clash} is already used by another Commy hotkey"));
    }
    register_hotkeys(&handle, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
fn set_sensitivity(app: tauri::State<'_, App>, auto: bool, threshold_db: f32) -> VoiceSettings {
    let snapshot = app.update_voice(|voice| {
        voice.auto_sensitivity = auto;
        voice.sensitivity_db = voice_keys::clamp_sensitivity_db(threshold_db);
    });
    app.processor.lock().unwrap().set_sensitivity(snapshot.sensitivity());
    snapshot
}

#[tauri::command]
fn mic_level(app: tauri::State<'_, App>) -> f32 {
    let listening = app.mic_test.lock().unwrap().is_some() || app.voice_session.lock().unwrap().is_some();
    if listening {
        app.processor.lock().unwrap().last_level_db()
    } else {
        commy_audio::SILENCE_DB
    }
}

#[cfg(desktop)]
fn on_overlay_hotkey(handle: &tauri::AppHandle, id: u32) {
    let app = handle.state::<App>();
    let settings = app.voice.lock().unwrap().overlay.clone();
    if hotkey_id(&settings.toggle_hotkey) == Some(id) {
        let visible = !app.overlay_visible.load(std::sync::atomic::Ordering::Relaxed);
        tracing::info!("overlay hotkey: {}", if visible { "show" } else { "hide" });
        app.overlay_visible.store(visible, std::sync::atomic::Ordering::Relaxed);
        overlay::refresh(handle);
    } else if hotkey_id(&settings.chat_hotkey) == Some(id) {
        let settings = app.update_voice(|voice| voice.overlay.show_chat = !voice.overlay.show_chat).overlay;
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
    let voice = app.update_voice(|voice| voice.overlay = settings.clone());
    if let Some(clash) = voice_keys::clashing(&all_hotkeys(&voice)) {
        return Err(format!("{clash} is already used by another Commy hotkey"));
    }
    if settings.enabled {
        app.overlay_visible.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    register_hotkeys(&handle, &voice);
    overlay::refresh(&handle);
    Ok(settings)
}

#[tauri::command]
fn preview_overlay(app: tauri::State<'_, App>, handle: tauri::AppHandle) {
    use std::sync::atomic::Ordering::Relaxed;
    if !app.voice.lock().unwrap().overlay.enabled {
        return;
    }
    app.overlay_preview.store(true, Relaxed);
    overlay::refresh(&handle);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(8)).await;
        handle.state::<App>().overlay_preview.store(false, Relaxed);
        overlay::refresh(&handle);
    });
}

#[tauri::command]
async fn set_about(app: tauri::State<'_, App>, handle: tauri::AppHandle, about: String) -> CmdResult<UserProfile> {
    let about = about.trim().to_string();
    if !commy_core::user::rich_text_fits(&about, commy_core::user::MAX_ABOUT_CHARS) {
        return Err(format!(
            "About Me can be up to {} characters and {} custom emojis",
            commy_core::user::MAX_ABOUT_CHARS,
            commy_core::user::MAX_PROFILE_EMOJIS
        ));
    }
    let about = carry_profile_emojis(&app, about).await;
    let profile = edit_profile(&app, |profile| profile.about = (!about.is_empty()).then_some(about))?;
    publish_profile(&app, &handle, &profile).await;
    Ok(profile)
}

const SERVER_IMAGE_DIR: &str = "server-images";
const MAX_SERVER_IMAGE_BYTES: usize = view::pictures::MAX_SERVER_IMAGE_BYTES;

fn server_image_path(app: &App, blob: &str) -> Option<std::path::PathBuf> {
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
    let bytes = data
        .map(|b64| decode_image(&b64, MAX_SERVER_IMAGE_BYTES, "server images must be a JPEG or PNG under 1 MB"))
        .transpose()?;
    if which == "invite" && bytes.as_ref().is_some_and(|b| b.len() > invite_link::MAX_INVITE_BACKGROUND_BYTES) {
        return Err("invite backgrounds must be under 400 KB".into());
    }
    let image = match bytes {
        None => None,
        Some(bytes) => Some(store_shared_blob(&app, bytes).await?),
    };
    let (icon, banner) = match which.as_str() {
        "icon" => (image, banner),
        "banner" => (icon, image),
        "invite" => {
            apply_and_broadcast(&app, &handle, sid, ServerEvent::InviteBackgroundSet { server: sid, background: image }).await?;
            let _ = handle.emit("servers-changed", ());
            return Ok(());
        }
        _ => return Err("unknown server image".into()),
    };
    apply_and_broadcast(&app, &handle, sid, ServerEvent::ServerImages { server: sid, icon, banner }).await?;
    let _ = handle.emit("servers-changed", ());
    Ok(())
}

#[tauri::command]
async fn fetch_server_image(app: tauri::State<'_, App>, image: String) -> CmdResult<String> {
    let bytes = server_image_bytes(&app, &image).await?;
    Ok(picture_data_url(&bytes))
}

pub(crate) fn picture_data_url(bytes: &[u8]) -> String {
    data_url(view::pictures::picture_mime(bytes), bytes)
}

pub(crate) async fn server_image_bytes(app: &App, image: &str) -> CmdResult<Vec<u8>> {
    let image = commy_core::server::ImageRef::parse(image).ok_or("bad image reference")?;
    let path = server_image_path(app, &image.blob);
    if let Some(bytes) = path.as_ref().and_then(|p| commy_core::at_rest::read(p).ok()) {
        return Ok(bytes);
    }
    let bytes = fetch_shared_image(app, &image, MAX_SERVER_IMAGE_BYTES as u64).await?;
    if bytes.len() > MAX_SERVER_IMAGE_BYTES || !is_jpeg_or_png(&bytes) {
        return Err("server image is not a valid picture".into());
    }
    if let Some(path) = &path {
        let _ = write_blob_cache(path, &bytes);
    }
    Ok(bytes)
}

fn manage_check(app: &App, sid: ServerId, user: UserId, needed: Permissions, action: &str) -> CmdResult<()> {
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
async fn set_onboarding(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    onboarding: Option<commy_core::Onboarding>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    manage_check(&app, sid, me.id, Permissions::MANAGE_SERVER, "change the onboarding questions")?;
    let roles = {
        let servers = app.servers.lock().unwrap();
        servers.get(&sid).ok_or("unknown server")?.server.roles.clone()
    };
    let onboarding = onboarding
        .map(|o| o.tidied(|r| commy_core::onboarding::grantable(&roles, r)))
        .filter(|o| !o.questions.is_empty());
    if onboarding.as_ref().is_some_and(|o| !o.fits()) {
        return Err(onboarding_limits());
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::OnboardingSet { server: sid, onboarding }).await
}

fn onboarding_limits() -> String {
    use commy_core::onboarding::{MAX_ROLE_REFS, MAX_TEXT_BYTES};
    format!("keep the questions and answers under {MAX_TEXT_BYTES} characters in all, with at most {MAX_ROLE_REFS} roles across every answer")
}

#[tauri::command]
async fn answer_onboarding(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    grant: Vec<String>,
    revoke: Vec<String>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let parse = |ids: Vec<String>| ids.iter().map(|id| id.parse::<RoleId>().map_err(err_to_string)).collect::<CmdResult<Vec<RoleId>>>();
    let (grant, revoke) = (parse(grant)?, parse(revoke)?);
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        let allowed = grant.iter().chain(&revoke).all(|r| commy_core::onboarding::may_self_assign(&state.server, me.id, *r));
        if !allowed {
            return Err("this server's questions do not hand out that role".into());
        }
    }
    for role in revoke {
        apply_and_broadcast(&app, &handle, sid, ServerEvent::RoleUnassign { server: sid, user: me.id, role }).await?;
    }
    for role in grant {
        apply_and_broadcast(&app, &handle, sid, ServerEvent::RoleAssign { server: sid, user: me.id, role }).await?;
    }
    Ok(())
}

#[tauri::command]
async fn accept_rules(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    apply_and_broadcast(&app, &handle, sid, ServerEvent::RulesAccept { server: sid, user: me.id }).await
}

fn require_can_view(state: &ServerState, user: UserId, channel: ChannelId) -> CmdResult<()> {
    view::act::can_view(state, user, channel)
}

fn require_not_timed_out(state: &ServerState, user: UserId) -> CmdResult<()> {
    view::act::not_timed_out(state, user, chrono::Utc::now())
}

fn require_can_post(app: &App, state: &ServerState, user: UserId, channel: ChannelId) -> CmdResult<()> {
    view::act::can_post(state, our_permissions(app, state, user), user, channel, chrono::Utc::now())
}

fn refuse_held(hold: Option<commy_core::Hold>) -> CmdResult<()> {
    hold.map_or(Ok(()), |hold| Err(hold.refusal().into()))
}

#[tauri::command]
async fn set_automod(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    rules: commy_core::AutoMod,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    manage_check(&app, sid, me.id, Permissions::MANAGE_SERVER, "change auto moderation")?;
    let rules = rules.tidied();
    if !rules.valid() {
        return Err(automod_limits());
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::AutoModSet { server: sid, rules }).await
}

fn automod_limits() -> String {
    use commy_core::automod::{MAX_BLOCKED_TERMS, MAX_TERMS_BYTES, MAX_TERM_BYTES};
    format!(
        "keep it to {MAX_BLOCKED_TERMS} blocked words of up to {MAX_TERM_BYTES} characters each, {MAX_TERMS_BYTES} characters in all"
    )
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
    let channel: Option<ChannelId> =
        channel_id.as_deref().filter(|c| !c.is_empty()).map(str::parse).transpose().map_err(err_to_string)?;
    let creator = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        let perms = our_permissions(&app, state, me.id);
        let can_manage = perms.allows(Permissions::MANAGE_EVENTS);
        let existing = event_id.as_ref().and_then(|id| state.server.events.iter().find(|e| &e.id == id));
        match existing {
            Some(e) if e.creator == me.id || can_manage => e.creator,
            Some(_) => return Err("you can only edit events you created".into()),
            None if perms.can_create_events() => me.id,
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
        let parent: Option<ChannelId> = m.parent.as_deref().map(str::parse).transpose().map_err(err_to_string)?;
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

#[tauri::command]
async fn set_channel_slow_mode(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String, channel_id: String, seconds: u32) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    manage_check(&app, sid, me.id, Permissions::MANAGE_CHANNELS, "change slow mode")?;
    if seconds > commy_core::server::MAX_SLOW_MODE {
        return Err("slow mode can be at most six hours".into());
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::ChannelSlowMode { server: sid, channel: cid, seconds }).await
}

#[tauri::command]
async fn set_voice_status(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String, channel_id: String, text: String) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    if text.trim().chars().count() > commy_core::server::MAX_VOICE_STATUS_CHARS {
        return Err(format!("keep the status to {} characters", commy_core::server::MAX_VOICE_STATUS_CHARS));
    }
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        let channel = state.server.channel(cid).ok_or("unknown channel")?;
        if channel.kind != ChannelKind::Voice {
            return Err("only voice channels have a status".into());
        }
        let in_channel = state.voice_states.get(&me.id) == Some(&cid);
        if !our_permissions(&app, state, me.id).can_set_voice_status(in_channel) {
            return Err("join the channel to set its status".into());
        }
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::VoiceStatusSet { server: sid, channel: cid, text, by: me.id }).await
}

fn refresh_priority_speakers(app: &App) {
    let Some((sid, channel, me)) = app.voice_session.lock().unwrap().as_ref().map(|s| (s.server, s.channel, s.me())) else {
        return;
    };
    if sid == commy_core::ids::CALL_SERVER {
        return;
    }
    let flags: Vec<(UserId, bool)> = {
        let servers = app.servers.lock().unwrap();
        let Some(state) = servers.get(&sid) else { return };
        let holders: std::collections::HashSet<UserId> = state.priority_speakers(channel).into_iter().collect();
        state
            .voice_members(channel)
            .into_iter()
            .chain(std::iter::once(me))
            .map(|user| (user, holders.contains(&user) || (user == me && our_permissions(app, state, me).is_priority_speaker())))
            .collect()
    };
    if let Some(session) = app.voice_session.lock().unwrap().as_ref() {
        let mut mixer = session.mixer.lock().unwrap();
        for (user, priority) in flags {
            mixer.set_priority(media_engine::mixer_key(user), priority);
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct ForumTagInput {
    pub id: Option<String>,
    pub name: String,
    pub emoji: Option<String>,
}

fn forum_channel(state: &ServerState, channel: ChannelId) -> CmdResult<&Channel> {
    let found = state.server.channel(channel).ok_or("unknown channel")?;
    if !found.is_forum() {
        return Err("that is not a forum channel".into());
    }
    Ok(found)
}

fn parse_tag_picks(raw: Vec<String>) -> CmdResult<Vec<commy_core::TagId>> {
    raw.iter().map(|t| t.parse::<commy_core::TagId>().map_err(|_| "that tag no longer exists".to_string())).collect()
}

fn require_known_tags(forum: &Channel, picks: &[commy_core::TagId]) -> CmdResult<()> {
    if picks.iter().all(|pick| forum.tags.iter().any(|t| t.id == *pick)) {
        Ok(())
    } else {
        Err("that tag is not on this forum".into())
    }
}

fn forum_post_root(state: &ServerState, channel: ChannelId, from: MessageId) -> Option<MessageId> {
    let messages = state.messages(channel);
    let mut current = messages.iter().find(|m| m.id == from)?;
    let mut seen = std::collections::HashSet::new();
    while let Some(parent) = current.reply_to.and_then(|p| messages.iter().find(|m| m.id == p)) {
        if !seen.insert(current.id) {
            break;
        }
        current = parent;
    }
    Some(current.id)
}

fn require_inside_post(state: &ServerState, channel: ChannelId, reply_to: Option<MessageId>) -> CmdResult<()> {
    let Some(forum) = state.server.channel(channel).filter(|c| c.is_forum()) else { return Ok(()) };
    let Some(target) = reply_to else {
        return Err("start a post to write in a forum channel".into());
    };
    match forum_post_root(state, channel, target) {
        Some(root) if forum.posts.contains_key(&root) => Ok(()),
        _ => Err("replies in a forum go inside a post".into()),
    }
}

#[tauri::command]
async fn set_forum_tags(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String, channel_id: String, tags: Vec<ForumTagInput>) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    manage_check(&app, sid, me.id, Permissions::MANAGE_CHANNELS, "edit channels")?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        forum_channel(state, cid)?;
    }
    let tags: Vec<commy_core::ForumTag> = tags
        .into_iter()
        .map(|t| commy_core::ForumTag {
            id: t.id.and_then(|id| id.parse().ok()).unwrap_or_else(commy_core::TagId::new),
            name: t.name,
            emoji: t.emoji,
        })
        .collect();
    let limits = format!(
        "a forum can have up to {} tags of {} characters each",
        commy_core::server::MAX_FORUM_TAGS,
        commy_core::server::MAX_FORUM_TAG_CHARS
    );
    let tags = commy_core::server::clean_forum_tags(&tags).ok_or(limits)?;
    apply_and_broadcast(&app, &handle, sid, ServerEvent::ForumTagsSet { server: sid, channel: cid, tags, by: me.id }).await
}

#[tauri::command]
async fn post_forum(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    title: String,
    content: String,
    tags: Vec<String>,
) -> CmdResult<MessageView> {
    let title = commy_core::server::clean_post_title(&title)
        .ok_or(format!("a post title is 1 to {} characters", commy_core::server::MAX_POST_TITLE_CHARS))?;
    let content = clean_message(&content)?;
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let tags = parse_tag_picks(tags)?;
    let profile = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_known_tags(forum_channel(state, cid)?, &tags)?;
        require_can_post(&app, state, profile.id, cid)?;
    }
    let content = expressions_for_server(&app, sid, profile.id, content).await?;
    let mut message = Message::new(cid, profile.id, content);
    let _signing = app.signing.read().await;
    seal_and_sign(&app, sid, &mut message)?;
    if !signed_fits_one_gossip_frame(&message) {
        return Err("that post is too long to send; try splitting it up".into());
    }
    let root = message.id;
    let view = commit_message(&app, &handle, sid, message).await?;
    let post = ServerEvent::ForumPost { server: sid, channel: cid, message: root, title, tags, by: profile.id };
    apply_and_broadcast(&app, &handle, sid, post).await?;
    Ok(view)
}

#[tauri::command]
async fn edit_forum_post(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    message_id: String,
    title: String,
    tags: Vec<String>,
) -> CmdResult<()> {
    let title = commy_core::server::clean_post_title(&title)
        .ok_or(format!("a post title is 1 to {} characters", commy_core::server::MAX_POST_TITLE_CHARS))?;
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let mid: MessageId = message_id.parse().map_err(err_to_string)?;
    let tags = parse_tag_picks(tags)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        let forum = forum_channel(state, cid)?;
        require_known_tags(forum, &tags)?;
        let post = forum.posts.get(&mid).ok_or("that post no longer exists")?;
        if post.author != me.id && !our_permissions(&app, state, me.id).allows(Permissions::MANAGE_MESSAGES) {
            return Err("you can only change your own posts".into());
        }
    }
    let post = ServerEvent::ForumPost { server: sid, channel: cid, message: mid, title, tags, by: me.id };
    apply_and_broadcast(&app, &handle, sid, post).await
}

fn voice_channel_full(app: &App, state: &ServerState, channel: ChannelId, user: UserId) -> bool {
    view::act::voice_full(state, our_permissions(app, state, user), user, channel)
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
    hoist: Option<bool>,
    tag: Option<String>,
    permissions: Option<Vec<String>>,
    mentionable: Option<bool>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let rid: RoleId = role_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let tag = tag.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    if tag.as_deref().is_some_and(|t| !commy_core::server::valid_role_tag(t)) {
        return Err(format!("tags can be up to {} characters", commy_core::server::MAX_ROLE_TAG_CHARS));
    }
    let color = color.map(|c| c.trim_start_matches('#').to_lowercase()).filter(|c| !c.is_empty());
    if color.as_deref().is_some_and(|c| !commy_core::server::valid_hex_color(c)) {
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
        commy_core::server::Role {
            name,
            color,
            self_assignable,
            hoist: hoist.unwrap_or(existing.hoist),
            tag,
            permissions,
            mentionable: mentionable.unwrap_or(existing.mentionable),
            ..existing.clone()
        }
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
            let bytes = decode_image(&b64, MAX_SERVER_IMAGE_BYTES, "role banners must be a JPEG or PNG under 1 MB")?;
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
            let bytes = decode_image(&b64, MAX_SERVER_IMAGE_BYTES, "channel banners must be a JPEG or PNG under 1 MB")?;
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
    if color.as_deref().is_some_and(|c| !commy_core::server::valid_hex_color(c)) {
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
async fn set_channel_sensitive(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    sensitive: bool,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_CHANNELS, "edit channels")?;
        let channel = state.server.channel(cid).ok_or("unknown channel")?;
        if !channel.kind.holds_messages() {
            return Err("only text and forum channels can be age-restricted".into());
        }
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::ChannelSensitiveSet { server: sid, channel: cid, sensitive }).await
}

#[tauri::command]
async fn set_channel_access(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    view_roles: Vec<String>,
    send_roles: Vec<String>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let parse = |raw: Vec<String>| raw.iter().map(|r| r.parse::<RoleId>()).collect::<Result<Vec<_>, _>>();
    let view = parse(view_roles).map_err(err_to_string)?;
    let send = parse(send_roles).map_err(err_to_string)?;
    if view.len() > commy_core::server::MAX_ACCESS_ROLES || send.len() > commy_core::server::MAX_ACCESS_ROLES {
        return Err(format!("pick at most {} roles", commy_core::server::MAX_ACCESS_ROLES));
    }
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_CHANNELS, "edit channels")?;
        let channel = state.server.channel(cid).ok_or("unknown channel")?;
        if channel.kind == ChannelKind::Category {
            return Err("categories do not hold messages to lock".into());
        }
        if [&view, &send].into_iter().flatten().any(|r| !state.server.roles.iter().any(|known| known.id == *r && !known.is_everyone())) {
            return Err("that role no longer exists".into());
        }
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::ChannelAccessSet { server: sid, channel: cid, view, send }).await?;
    leave_unseen_voice(&app, &handle, sid).await;
    channel_lock::tend_channel(&app, &handle, sid, cid).await;
    Ok(())
}

async fn leave_unseen_voice(app: &App, handle: &tauri::AppHandle, sid: ServerId) {
    let Some(channel) = app.voice_session.lock().unwrap().as_ref().filter(|s| s.server == sid).map(|s| s.channel) else {
        return;
    };
    let Ok(profile) = current_profile(app) else { return };
    let sees = app.servers.lock().unwrap().get(&sid).is_some_and(|s| s.server.can_view(channel, profile.id));
    if !sees {
        let voice = app.voice.lock().unwrap().clone();
        leave_voice(app, handle, sid, &profile, &voice).await;
        let _ = handle.emit("voice-left", &sid.to_string());
    }
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
    let event = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        view::act::pin(state, &acting(&app, state, me.id), cid, mid, pinned)?
    };
    apply_and_broadcast(&app, &handle, sid, event).await
}

#[tauri::command]
async fn set_thread(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    root_id: String,
    name: Option<String>,
    archived: bool,
    locked: bool,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let root: MessageId = root_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let event = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        view::act::thread(state, &acting(&app, state, me.id), cid, root, name.as_deref(), archived, locked)?
    };
    apply_and_broadcast(&app, &handle, sid, event).await
}

#[tauri::command]
fn get_pins(app: tauri::State<App>, server_id: String, channel_id: Option<String>) -> CmdResult<Vec<view::PinView>> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let cid: Option<ChannelId> = channel_id.as_deref().map(str::parse).transpose().map_err(err_to_string)?;
    let keys = channel_lock::holder_of(&app);
    let me = current_profile(&app)?.id;
    let servers = app.servers.lock().unwrap();
    let state = servers.get(&sid).ok_or("unknown server")?;
    Ok(view::pin_views(state, cid, me, &keys))
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
    let server_names: Vec<Option<String>> = {
        let servers = app.servers.lock().unwrap();
        stats.iter().map(|s| servers.get(&sid).map(|st| view::display_name(st, s.user))).collect()
    };
    let names: Vec<String> = stats
        .iter()
        .zip(server_names)
        .map(|(s, name)| name.unwrap_or_else(|| known_name(&app, s.user)))
        .collect();
    let links = stats
        .into_iter()
        .zip(names)
        .map(|(s, name)| {
            LinkView {
                user: s.user.to_string(),
                name,
                rtt_ms: s.rtt_ms,
                loss_pct: s.loss_pct,
                jitter_ms: s.jitter_ms,
                path: s.path.kind.into(),
                relay: s.path.relay,
                addr: s.path.addr,
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
async fn set_stage_speaker(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    user_id: String,
    speaker: bool,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let channel: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let user: UserId = user_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        if !our_permissions(&app, state, me.id).can_manage_stage() {
            return Err("you need to manage channels or voice to choose stage speakers".into());
        }
        require_can_view(state, me.id, channel)?;
        if !state.server.channel(channel).is_some_and(|c| c.is_stage()) {
            return Err("that is not a stage channel".into());
        }
        if !state.server.is_member(user) {
            return Err("that person is not a member of this server".into());
        }
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::StageSet { server: sid, channel, user, speaker, by: me.id }).await?;
    refresh_forced_voice(&app, &handle);
    Ok(())
}

#[tauri::command]
async fn raise_hand(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String, raised: bool) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let channel = app
        .voice_session
        .lock()
        .unwrap()
        .as_ref()
        .filter(|s| s.server == sid)
        .map(|s| s.channel)
        .ok_or("you are not in a stage channel here")?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        if !state.server.channel(channel).is_some_and(|c| c.is_stage()) {
            return Err("hands go up in stage channels".into());
        }
        if raised && !state.server.in_audience(channel, me.id) {
            return Err("you already have the floor".into());
        }
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::HandRaise { server: sid, channel, user: me.id, raised }).await
}

#[tauri::command]
async fn set_nickname(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    user_id: String,
    nickname: Option<String>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let user: UserId = user_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let cleaned = match nickname.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        None => None,
        Some(text) => Some(commy_core::server::clean_nickname(text).ok_or_else(|| {
            format!("nicknames can be up to {} characters", commy_core::server::MAX_NICKNAME_CHARS)
        })?),
    };
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        if !state.server.is_member(user) {
            return Err("that person is not a member of this server".into());
        }
        let own = user == me.id && our_permissions(&app, state, me.id).allows(Permissions::CHANGE_NICKNAME);
        if !own {
            require_permission(&app, state, me.id, Permissions::MANAGE_NICKNAMES, "change nicknames")?;
            if user == state.server.owner {
                return Err("only the server owner can change their own nickname".into());
            }
        }
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::MemberNickname { server: sid, user, nickname: cleaned }).await
}

#[tauri::command]
async fn move_voice_member(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    user_id: String,
    channel_id: Option<String>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let user: UserId = user_id.parse().map_err(err_to_string)?;
    let channel: Option<ChannelId> = channel_id.as_deref().map(str::parse).transpose().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MOVE_MEMBERS, "move members between voice channels")?;
        if user == state.server.owner || user == me.id {
            return Err("you cannot move that member".into());
        }
        let Some(current) = state.voice_states.get(&user).copied() else {
            return Err("that member is not in voice here".into());
        };
        if let Some(target) = channel {
            match state.server.channel(target) {
                Some(c) if c.kind.is_voice() => {}
                _ => return Err("that is not a voice channel".into()),
            }
            require_can_view(state, me.id, target)?;
            if target == current {
                return Err("they are already in that channel".into());
            }
            if voice_channel_full(&app, state, target, user) {
                return Err("that voice channel is full".into());
            }
        }
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::VoiceMove { server: sid, user, channel }).await
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
    apply_and_broadcast(&app, &handle, sid, ServerEvent::MemberBan { server: sid, user }).await?;
    channel_lock::tend_server(&app, &handle, sid).await;
    Ok(())
}

fn timeout_event(sid: ServerId, user: UserId, by: UserId, seconds: Option<u32>) -> CmdResult<ServerEvent> {
    let at = chrono::Utc::now();
    let until = match seconds {
        Some(secs) if i64::from(secs) > commy_core::server::MAX_TIMEOUT_SECS => {
            return Err("a timeout can last four weeks at most".into());
        }
        Some(0) => return Err("choose how long the timeout lasts".into()),
        Some(secs) => Some(at + chrono::Duration::seconds(i64::from(secs))),
        None => None,
    };
    Ok(ServerEvent::MemberTimeout { server: sid, user, by, at, until })
}

async fn set_member_timeout(app: &App, handle: &tauri::AppHandle, server_id: String, user_id: String, seconds: Option<u32>) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let user: UserId = user_id.parse().map_err(err_to_string)?;
    let me = current_profile(app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(app, state, me.id, Permissions::TIME_OUT_MEMBERS, "time out members")?;
        if user == state.server.owner || user == me.id {
            return Err("you cannot time out that member".into());
        }
        if !state.server.is_member(user) {
            return Err("that member is no longer in the server".into());
        }
    }
    apply_and_broadcast(app, handle, sid, timeout_event(sid, user, me.id, seconds)?).await?;
    refresh_forced_voice(app, handle);
    Ok(())
}

#[tauri::command]
async fn time_out_member(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String, user_id: String, seconds: u32) -> CmdResult<()> {
    set_member_timeout(&app, &handle, server_id, user_id, Some(seconds)).await
}

#[tauri::command]
async fn clear_timeout(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String, user_id: String) -> CmdResult<()> {
    set_member_timeout(&app, &handle, server_id, user_id, None).await
}

#[tauri::command]
async fn unban_member(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String, user_id: String) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let user: UserId = user_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    manage_check(&app, sid, me.id, Permissions::BAN_MEMBERS, "unban members")?;
    apply_and_broadcast(&app, &handle, sid, ServerEvent::MemberUnban { server: sid, user }).await
}

fn end_voice_in(app: &App, sid: ServerId) {
    let in_voice_there = app.voice_session.lock().unwrap().as_ref().is_some_and(|s| s.server == sid);
    if in_voice_there {
        let testing = pause_mic_test(app);
        *app.voice_session.lock().unwrap() = None;
        resume_mic_test(app, testing);
        app.own_preview.lock().unwrap().take();
        *app.screen_share.lock().unwrap() = None;
    }
}

async fn on_banned(app: &App, handle: &tauri::AppHandle, sid: ServerId) {
    let name = forget_server_locally(app, sid).map(|s| s.server.name);
    end_voice_in(app, sid);
    leave_server_topic(app, sid).await;
    let _ = handle.emit("banned", serde_json::json!({ "server": sid.to_string(), "name": name }));
    let _ = handle.emit("servers-changed", ());
}

fn retire_deleted_server(app: &App, handle: &tauri::AppHandle, sid: ServerId, notice: SignedEvent, owned: bool) -> String {
    let ticket = (!owned).then(|| app.invites.lock().unwrap().get(&sid).cloned()).flatten();
    let name = forget_server_locally(app, sid).map(|s| s.server.name).unwrap_or_default();
    end_voice_in(app, sid);
    let kept = deleted::DeletedServer { name: name.clone(), notice, ticket };
    app.deleted.lock().unwrap().insert(sid, kept);
    app.persist_deleted();
    entry_list::record_leave(app, sid);
    entry_list::schedule(handle);
    let _ = handle.emit("servers-changed", ());
    name
}

fn on_server_deleted(app: &App, handle: &tauri::AppHandle, sid: ServerId, notice: SignedEvent) {
    let name = retire_deleted_server(app, handle, sid, notice, false);
    let _ = handle.emit("server-deleted", serde_json::json!({ "server": sid.to_string(), "name": name }));
}

#[tauri::command]
async fn transfer_ownership(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    user_id: String,
    confirm_name: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let user: UserId = user_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        if state.server.owner != me.id {
            return Err("only the owner can hand over this server".into());
        }
        if user == me.id || !state.server.is_member(user) {
            return Err("pick another member of this server".into());
        }
        if confirm_name.trim() != state.server.name.trim() {
            return Err("type the server name exactly as it is shown to hand it over".into());
        }
    }
    apply_and_broadcast(&app, &handle, sid, ServerEvent::OwnerTransfer { server: sid, user }).await
}

#[tauri::command]
async fn delete_server(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    confirm_name: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let notice = {
        let mut servers = app.servers.lock().unwrap();
        let state = servers.get_mut(&sid).ok_or("unknown server")?;
        if state.server.owner != me.id {
            return Err("only the owner can delete this server".into());
        }
        if confirm_name.trim() != state.server.name.trim() {
            return Err("type the server name exactly as it is shown to delete it".into());
        }
        let notice = app
            .sign_event(sid, ServerEvent::ServerDelete { server: sid })
            .ok_or("your account key is still being confirmed; try again in a moment")?;
        state.apply_own(&notice).map_err(err_to_string)?;
        notice
    };
    let payload = WirePayload::Signed(notice.clone());
    if !community_sync::publish_now(sid, &payload).await {
        tracing::warn!("the sync service did not take the deletion notice; members hear it from each other");
    }
    broadcast(&app, sid, payload).await;
    retire_deleted_server(&app, &handle, sid, notice, true);
    Ok(())
}

#[derive(serde::Serialize)]
struct AuditEntryView {
    at: i64,
    by: String,
    by_name: String,
    kind: commy_core::AuditKind,
    target: Option<String>,
    label: Option<String>,
    detail: Option<String>,
}

#[tauri::command]
fn audit_log(app: tauri::State<App>, server_id: String) -> CmdResult<Vec<AuditEntryView>> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let servers = app.servers.lock().unwrap();
    let state = servers.get(&sid).ok_or("unknown server")?;
    if !our_permissions(&app, state, me.id).can_view_audit_log() {
        return Err("you do not have permission to view the audit log".into());
    }
    let entries = state.audit_log().iter().rev().map(|entry| AuditEntryView {
        at: entry.at,
        by: entry.by.to_string(),
        by_name: commy_core::audit::member_name(state, entry.by).unwrap_or_else(|| "Someone".into()),
        kind: entry.kind,
        target: entry.target.clone(),
        label: entry.label.clone(),
        detail: entry.detail.clone(),
    });
    Ok(entries.collect())
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
    let settings = app.update_voice(|voice| {
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
    });
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
            broadcast_event(&app, sid, ServerEvent::ScreenShare { user: me, channel, active: true }).await;
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

async fn store_shared_blob(app: &App, bytes: Vec<u8>) -> CmdResult<String> {
    let (blob, provider) = {
        let net = app.net.lock().await;
        let node = net.as_ref().ok_or("network is not ready yet")?;
        (node.add_blob(bytes.clone()).await.map_err(err_to_string)?, node.node_id_hex())
    };
    pictures::offer(&app.http, &blob, &bytes);
    if let Some(path) = server_image_path(app, &blob) {
        write_blob_cache(&path, &bytes).map_err(err_to_string)?;
    }
    Ok(commy_core::server::ImageRef { blob, provider }.encode())
}

fn write_blob_cache(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    commy_core::at_rest::write(path, bytes)
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

fn check_can_add(app: &App, state: &ServerState, me: UserId, kind: Permissions, taken: bool, noun: &str) -> CmdResult<()> {
    let perms = our_permissions(app, state, me);
    if !state.server.is_member(me) || !perms.can_add(kind) {
        return Err(format!("you can't add {noun}s in this server"));
    }
    if taken && !perms.allows(Permissions::MANAGE_EXPRESSIONS) {
        return Err(format!("a {noun} with that name already exists"));
    }
    Ok(())
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
        let replacing = state.server.emojis.iter().any(|e| e.name == name);
        check_can_add(&app, state, me.id, Permissions::ADD_EMOJIS, replacing, "emoji")?;
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
    manage_check(&app, sid, me.id, Permissions::MANAGE_EXPRESSIONS, "manage emojis")?;
    apply_and_broadcast(&app, &handle, sid, ServerEvent::EmojiRemove { server: sid, name }).await
}

#[tauri::command]
async fn add_sticker(
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
        let replacing = state.server.stickers.iter().any(|s| s.name == name);
        check_can_add(&app, state, me.id, Permissions::ADD_STICKERS, replacing, "sticker")?;
        if !replacing && state.server.stickers.len() >= commy_core::server::MAX_STICKERS {
            return Err(format!("servers can have up to {} stickers", commy_core::server::MAX_STICKERS));
        }
    }
    let bytes = decode_upload(&data)?;
    if bytes.len() > expressions::MAX_STICKER_BYTES || expressions::image_mime(&bytes).is_none() {
        return Err("stickers must be a PNG, GIF, JPEG or WebP under 512 KB".into());
    }
    let blob = store_shared_blob(&app, bytes).await?;
    let sticker = commy_core::server::Sticker { name, blob };
    apply_and_broadcast(&app, &handle, sid, ServerEvent::StickerAdd { server: sid, sticker }).await
}

#[tauri::command]
async fn remove_sticker(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    name: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    manage_check(&app, sid, me.id, Permissions::MANAGE_EXPRESSIONS, "remove stickers")?;
    apply_and_broadcast(&app, &handle, sid, ServerEvent::StickerRemove { server: sid, name }).await
}

#[tauri::command]
async fn add_emoticon(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    name: String,
    text: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let name = check_expression_name(&name)?;
    let text = commy_core::server::clean_emoticon(&text).ok_or(format!(
        "emoticons are one line of up to {} characters",
        commy_core::server::MAX_EMOTICON_CHARS
    ))?;
    let me = current_profile(&app)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        let replacing = state.server.emoticons.iter().any(|e| e.name == name);
        check_can_add(&app, state, me.id, Permissions::ADD_EMOTICONS, replacing, "emoticon")?;
        if !replacing && state.server.emoticons.len() >= commy_core::server::MAX_EMOTICONS {
            return Err(format!("servers can have up to {} emoticons", commy_core::server::MAX_EMOTICONS));
        }
    }
    let emoticon = commy_core::server::Emoticon { name, text };
    apply_and_broadcast(&app, &handle, sid, ServerEvent::EmoticonAdd { server: sid, emoticon }).await
}

#[tauri::command]
async fn remove_emoticon(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    name: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    manage_check(&app, sid, me.id, Permissions::MANAGE_EXPRESSIONS, "remove emoticons")?;
    apply_and_broadcast(&app, &handle, sid, ServerEvent::EmoticonRemove { server: sid, name }).await
}

#[tauri::command]
async fn set_expression_sharing(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    stickers: bool,
    emoticons: bool,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    manage_check(&app, sid, me.id, Permissions::MANAGE_SERVER, "change sharing")?;
    apply_and_broadcast(&app, &handle, sid, ServerEvent::ExpressionSharingSet { server: sid, stickers, emoticons }).await?;
    let _ = handle.emit("servers-changed", ());
    Ok(())
}

const MAX_CATALOG_IMAGE_BYTES: usize = 4 * 1024 * 1024;

#[tauri::command]
async fn search_emoji_catalog(
    app: tauri::State<'_, App>,
    query: String,
    category: Option<u32>,
    page: Option<u32>,
) -> CmdResult<catalog::EmojiPage> {
    let params = catalog::emoji_params(&query, category, page);
    let params: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let found: Option<catalog::EmojiPage> = account::server_search_with(&app, "emojis", &params).await?;
    Ok(catalog::safe_emojis(found.ok_or("Sign in to an account server that offers emoji search")?))
}

#[tauri::command]
async fn fetch_catalog_image(app: tauri::State<'_, App>, url: String) -> CmdResult<String> {
    let source = catalog::source(&url).ok_or("only emoji.gg and KLIPY images can be added")?;
    let (_, bytes) = account::fetch_media(&app, &url)
        .await
        .map_err(|status| format!("{source} could not be reached right now ({status})"))?;
    if bytes.len() > MAX_CATALOG_IMAGE_BYTES {
        return Err("that image is too large".into());
    }
    let mime = expressions::image_mime(&bytes).ok_or_else(|| format!("{source} sent something that is not an image"))?;
    Ok(data_url(mime, &bytes))
}

#[tauri::command]
async fn sticker_search(
    app: tauri::State<'_, App>,
    query: String,
    from: Option<String>,
    page: Option<u32>,
) -> CmdResult<Option<catalog::StickerPage>> {
    let params = catalog::sticker_params(&query, from.as_deref(), page);
    let params: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let found: Option<catalog::StickerPage> = account::server_search_with(&app, "stickers", &params).await?;
    Ok(found.map(catalog::safe_stickers))
}

#[tauri::command]
async fn store_catalog_emoji(app: tauri::State<'_, App>, name: String, data: String) -> CmdResult<String> {
    let name = check_expression_name(&name)?;
    let bytes = decode_upload(&data)?;
    if bytes.len() > expressions::MAX_EMOJI_BYTES || expressions::image_mime(&bytes).is_none() {
        return Err("emojis must be a PNG, GIF, JPEG or WebP under 256 KB".into());
    }
    let image = store_shared_blob(&app, bytes).await?;
    Ok(commy_core::server::emoji_token(&name, &image))
}

#[tauri::command]
async fn store_catalog_sticker(app: tauri::State<'_, App>, name: String, data: String) -> CmdResult<String> {
    let name = check_expression_name(&name)?;
    let bytes = decode_upload(&data)?;
    if bytes.len() > expressions::MAX_STICKER_BYTES || expressions::image_mime(&bytes).is_none() {
        return Err("stickers must be a PNG, GIF, JPEG or WebP under 512 KB".into());
    }
    let image = store_shared_blob(&app, bytes).await?;
    Ok(commy_core::server::sticker_token(&name, &image))
}

fn can_upload_sounds(app: &App, state: &ServerState, user: UserId) -> bool {
    our_permissions(app, state, user).allows(Permissions::MANAGE_EXPRESSIONS)
        || (state.server.soundboard_open && state.server.is_member(user))
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
    manage_check(&app, sid, me.id, Permissions::MANAGE_EXPRESSIONS, "remove sounds")?;
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
    manage_check(&app, sid, me.id, Permissions::MANAGE_SERVER, "change expression settings")?;
    let event = ServerEvent::ExpressionSettings { server: sid, emojis_shareable, soundboard_open };
    apply_and_broadcast(&app, &handle, sid, event).await?;
    let _ = handle.emit("servers-changed", ());
    Ok(())
}

#[tauri::command]
async fn set_sound_sharing(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    shareable: bool,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    manage_check(&app, sid, me.id, Permissions::MANAGE_SERVER, "change sound sharing")?;
    apply_and_broadcast(&app, &handle, sid, ServerEvent::SoundSharingSet { server: sid, shareable }).await?;
    let _ = handle.emit("servers-changed", ());
    Ok(())
}

#[tauri::command]
async fn set_activities_enabled(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    enabled: bool,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    manage_check(&app, sid, me.id, Permissions::MANAGE_SERVER, "turn Activities on or off")?;
    apply_and_broadcast(&app, &handle, sid, ServerEvent::ActivitiesSet { server: sid, enabled }).await?;
    let _ = handle.emit("servers-changed", ());
    Ok(())
}

fn role_access_permission(access: &str) -> CmdResult<(Permissions, &'static str)> {
    match access {
        "files" => Ok((Permissions::ATTACH_FILES, "change who can send files")),
        "emojis" => Ok((Permissions::ADD_EMOJIS, "change who can add emojis")),
        "stickers" => Ok((Permissions::ADD_STICKERS, "change who can add stickers")),
        "emoticons" => Ok((Permissions::ADD_EMOTICONS, "change who can add emoticons")),
        "embeds" => Ok((Permissions::NO_EMBED_LINKS, "change who can embed links")),
        "history" => Ok((Permissions::NO_READ_MESSAGE_HISTORY, "change who can read message history")),
        "threads" => Ok((Permissions::NO_CREATE_THREADS, "change who can start threads")),
        "polls" => Ok((Permissions::NO_SEND_POLLS, "change who can create polls")),
        "events" => Ok((Permissions::NO_CREATE_EVENTS, "change who can create events")),
        _ => Err("unknown setting".into()),
    }
}

fn role_access_updates(roles: &[Role], permission: Permissions, restricted: bool, allowed: &[RoleId]) -> Vec<Role> {
    let inverted = permission.is_restriction();
    roles
        .iter()
        .filter_map(|r| {
            let mut role = r.clone();
            if inverted {
                role.permissions.remove(permission);
                if restricted && !allowed.contains(&r.id) {
                    role.permissions.insert(permission);
                }
            } else if r.is_everyone() {
                role.permissions.remove(permission);
                if !restricted {
                    role.permissions.insert(permission);
                }
            } else if restricted {
                if allowed.contains(&r.id) {
                    role.permissions.insert(permission);
                } else {
                    role.permissions.remove(permission);
                }
            }
            (role.permissions != r.permissions).then_some(role)
        })
        .collect()
}

#[cfg(test)]
mod role_access_tests {
    use super::*;
    use commy_core::server::EVERYONE_ROLE_POSITION;

    fn role(permissions: Permissions, position: u32) -> Role {
        Role {
            id: RoleId::new(),
            name: "r".into(),
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

    fn after(roles: &[Role], updates: Vec<Role>) -> Vec<Permissions> {
        roles
            .iter()
            .map(|r| updates.iter().find(|u| u.id == r.id).map_or(r.permissions, |u| u.permissions))
            .collect()
    }

    #[test]
    fn restricting_everyone_sets_the_no_bit_on_every_role_except_the_allowed_ones() {
        let roles = [
            role(Permissions::default_member(), EVERYONE_ROLE_POSITION),
            role(Permissions::MANAGE_MESSAGES, 1),
            role(Permissions::NONE, 2),
        ];
        let bit = Permissions::NO_SEND_POLLS;
        let updates = role_access_updates(&roles, bit, true, &[roles[2].id]);
        assert_eq!(updates.len(), 2);
        let perms = after(&roles, updates);
        assert!(perms[0].contains(bit));
        assert!(perms[1].contains(bit));
        assert!(!perms[2].contains(bit));
        assert!(perms[1].allows(Permissions::MANAGE_MESSAGES));
    }

    #[test]
    fn lifting_a_restriction_clears_the_no_bit_everywhere_and_touches_nothing_else() {
        let bit = Permissions::NO_EMBED_LINKS;
        let roles = [
            role(Permissions::default_member() | bit, EVERYONE_ROLE_POSITION),
            role(Permissions::MANAGE_MESSAGES | bit, 1),
            role(Permissions::NONE, 2),
        ];
        let updates = role_access_updates(&roles, bit, false, &[]);
        assert_eq!(updates.len(), 2);
        let perms = after(&roles, updates);
        assert_eq!(perms[0], Permissions::default_member());
        assert_eq!(perms[1], Permissions::MANAGE_MESSAGES);
        assert_eq!(perms[2], Permissions::NONE);
    }

    #[test]
    fn positive_permissions_keep_the_grant_model() {
        let roles = [
            role(Permissions::default_member(), EVERYONE_ROLE_POSITION),
            role(Permissions::NONE, 1),
        ];
        let updates = role_access_updates(&roles, Permissions::ATTACH_FILES, true, &[roles[1].id]);
        let perms = after(&roles, updates);
        assert!(!perms[0].contains(Permissions::ATTACH_FILES));
        assert!(perms[1].contains(Permissions::ATTACH_FILES));
    }
}

#[tauri::command]
async fn set_role_access(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    access: String,
    restricted: bool,
    role_ids: Vec<String>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let (permission, action) = role_access_permission(&access)?;
    let me = current_profile(&app)?;
    let allowed = role_ids
        .iter()
        .map(|r| r.parse::<RoleId>())
        .collect::<Result<Vec<RoleId>, _>>()
        .map_err(err_to_string)?;
    let updates = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::MANAGE_ROLES, action)?;
        role_access_updates(&state.server.roles, permission, restricted, &allowed)
    };
    for role in updates {
        apply_and_broadcast(&app, &handle, sid, ServerEvent::RoleUpdate { server: sid, role }).await?;
    }
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
    manage_check(&app, sid, me.id, Permissions::MANAGE_SERVER, "make this server public")?;
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
    play_sound_in_voice(&app, &handle, sid, name).await
}

async fn play_sound_in_voice(app: &App, handle: &tauri::AppHandle, sid: ServerId, name: String) -> CmdResult<()> {
    let me = current_profile(app)?;
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
        require_permission(app, state, me.id, Permissions::USE_SOUNDBOARD, "use the soundboard")?;
        require_not_timed_out(state, me.id)?;
        soundboard_play_view(state, channel, me.id, &name).ok_or("that sound no longer exists")?
    };
    broadcast_event(app, sid, ServerEvent::SoundboardPlay { server: sid, channel, user: me.id, name })
    .await;
    let _ = handle.emit("soundboard-play", view);
    Ok(())
}

#[derive(Clone, serde::Serialize)]
struct SharedSoundView {
    server: String,
    server_name: String,
    name: String,
    blob: String,
    volume: f32,
    emoji: Option<String>,
}

#[tauri::command]
fn list_shareable_sounds(app: tauri::State<'_, App>) -> Vec<SharedSoundView> {
    let servers = app.servers.lock().unwrap();
    let mut out: Vec<SharedSoundView> = servers
        .values()
        .filter(|s| s.server.sounds_shareable)
        .flat_map(|s| {
            s.server.soundboard.iter().map(|sound| SharedSoundView {
                server: s.server.id.to_string(),
                server_name: s.server.name.clone(),
                name: sound.name.clone(),
                blob: sound.blob.clone(),
                volume: sound.volume,
                emoji: sound.emoji.clone(),
            })
        })
        .collect();
    out.sort_by(|a, b| a.server_name.cmp(&b.server_name).then(a.name.cmp(&b.name)));
    out
}

fn shared_sound_view(sid: ServerId, channel: ChannelId, user: UserId, name: &str, blob: &str, volume: f32) -> Option<SoundboardPlayView> {
    commy_core::server::ImageRef::parse(blob)?;
    Some(SoundboardPlayView {
        server: sid.to_string(),
        channel: channel.to_string(),
        user: user.to_string(),
        name: name.to_string(),
        blob: blob.to_string(),
        volume: volume.clamp(0.0, 2.0),
    })
}

#[tauri::command]
async fn play_shared_sound(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    source_server_id: String,
    name: String,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let source: ServerId = source_server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let (channel, in_server) = {
        let session = app.voice_session.lock().unwrap();
        let s = session.as_ref().ok_or("join a voice channel first")?;
        (s.channel, s.server == sid)
    };
    if !in_server {
        return Err("join a voice channel in this server first".into());
    }
    let (blob, volume) = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_permission(&app, state, me.id, Permissions::USE_SOUNDBOARD, "use the soundboard")?;
        let from = servers.get(&source).ok_or("unknown server")?;
        if !from.server.sounds_shareable {
            return Err("that server does not share its sounds".into());
        }
        let sound = from.server.soundboard.iter().find(|s| s.name == name).ok_or("that sound no longer exists")?;
        (sound.blob.clone(), sound.volume)
    };
    let view = shared_sound_view(sid, channel, me.id, &name, &blob, volume).ok_or("that sound cannot be played")?;
    broadcast_event(&app, sid, ServerEvent::SharedSoundPlay { server: sid, channel, user: me.id, name, blob, volume }).await;
    let _ = handle.emit("soundboard-play", view);
    Ok(())
}

fn on_remote_shared_sound(app: &tauri::State<'_, App>, handle: &tauri::AppHandle, sid: ServerId, channel: ChannelId, user: UserId, name: &str, blob: &str, volume: f32) {
    let mine = app.voice_session.lock().unwrap().as_ref().map(|s| (s.server, s.channel));
    if mine != Some((sid, channel)) {
        return;
    }
    let allowed = {
        let servers = app.servers.lock().unwrap();
        servers.get(&sid).is_some_and(|state| {
            state.voice_states.get(&user) == Some(&channel) && state.server.permissions_for(user).allows(Permissions::USE_SOUNDBOARD)
        })
    };
    if !allowed {
        return;
    }
    if let Some(view) = shared_sound_view(sid, channel, user, name, blob, volume) {
        let _ = handle.emit("soundboard-play", view);
    }
}

#[derive(Clone, serde::Serialize)]
struct ActivitySetView {
    server: String,
    channel: String,
    activity: Option<Activity>,
    by: String,
}

#[derive(Clone, serde::Serialize)]
struct ActivityInputView {
    server: String,
    channel: String,
    seq: u64,
    input: ActivityInput,
    by: String,
}

fn activity_set_view(sid: ServerId, channel: ChannelId, activity: &Option<Activity>, by: UserId) -> ActivitySetView {
    ActivitySetView { server: sid.to_string(), channel: channel.to_string(), activity: activity.clone(), by: by.to_string() }
}

fn my_voice_channel(app: &App, sid: ServerId) -> CmdResult<ChannelId> {
    let session = app.voice_session.lock().unwrap();
    let s = session.as_ref().ok_or("join a voice channel first")?;
    if s.server != sid {
        return Err("join a voice channel in this server first".into());
    }
    Ok(s.channel)
}

#[tauri::command]
async fn set_activity(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    activity: Option<Activity>,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let channel = my_voice_channel(&app, sid)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_not_timed_out(state, me.id)?;
        if activity.is_some() && !state.server.activities_enabled {
            return Err("Activities are turned off in this server's settings".into());
        }
        if activity.as_ref().is_some_and(|a| !a.valid()) {
            return Err("paste a direct https link to an .mp4, .webm or .ogg file".into());
        }
        if !commy_core::activity::admits_set(state, me.id, channel, activity.as_ref()) {
            return Err(match (&activity, state.activities.get(&channel)) {
                (Some(_), Some(_)) => "another activity is already running in this channel",
                (None, Some(_)) => "only the person who started it or a moderator can end it",
                (None, None) => "nothing is running in this channel",
                (Some(_), None) => "you are not in this voice channel",
            }
            .into());
        }
    }
    let event = ServerEvent::ActivitySet { server: sid, channel, activity: activity.clone(), by: me.id };
    apply_and_broadcast(&app, &handle, sid, event).await?;
    let _ = handle.emit("activity-set", activity_set_view(sid, channel, &activity, me.id));
    Ok(())
}

#[tauri::command]
async fn send_activity_input(
    app: tauri::State<'_, App>,
    server_id: String,
    seq: u64,
    input: ActivityInput,
) -> CmdResult<()> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let channel = my_voice_channel(&app, sid)?;
    {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        require_not_timed_out(state, me.id)?;
        if !commy_core::activity::admits_input(state, me.id, channel, &input) {
            return Err("that is not possible in the running activity".into());
        }
    }
    broadcast_event(&app, sid, ServerEvent::ActivityInput { server: sid, channel, seq, input, by: me.id }).await;
    Ok(())
}

fn on_remote_activity_input(app: &tauri::State<'_, App>, handle: &tauri::AppHandle, sid: ServerId, channel: ChannelId, seq: u64, input: &ActivityInput, by: UserId) {
    let mine = app.voice_session.lock().unwrap().as_ref().map(|s| (s.server, s.channel));
    if mine != Some((sid, channel)) {
        return;
    }
    let allowed = {
        let servers = app.servers.lock().unwrap();
        servers.get(&sid).is_some_and(|state| commy_core::activity::admits_input(state, by, channel, input))
    };
    if allowed {
        let view = ActivityInputView { server: sid.to_string(), channel: channel.to_string(), seq, input: input.clone(), by: by.to_string() };
        let _ = handle.emit("activity-input", view);
    }
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

#[derive(Clone, serde::Serialize)]
struct SharedEmoticonView {
    server: String,
    server_name: String,
    name: String,
    text: String,
}

#[derive(serde::Serialize)]
struct SharedExpressions {
    emojis: Vec<SharedEmojiView>,
    stickers: Vec<SharedEmojiView>,
    emoticons: Vec<SharedEmoticonView>,
}

fn shared_images<'a>(
    servers: impl Iterator<Item = &'a ServerState>,
    pick: impl Fn(&'a ServerState) -> Vec<(&'a str, &'a str)>,
) -> Vec<SharedEmojiView> {
    let mut out: Vec<SharedEmojiView> = servers
        .flat_map(|s| {
            pick(s).into_iter().map(move |(name, image)| SharedEmojiView {
                server: s.server.id.to_string(),
                server_name: s.server.name.clone(),
                name: name.to_string(),
                image: image.to_string(),
            })
        })
        .collect();
    out.sort_by(|a, b| a.server_name.cmp(&b.server_name).then(a.name.cmp(&b.name)));
    out
}

#[tauri::command]
fn list_shareable_expressions(app: tauri::State<'_, App>) -> SharedExpressions {
    let servers = app.servers.lock().unwrap();
    let emojis = shared_images(servers.values().filter(|s| s.server.emojis_shareable), |s| {
        s.server.emojis.iter().map(|e| (e.name.as_str(), e.blob.as_str())).collect()
    });
    let stickers = shared_images(servers.values().filter(|s| s.server.stickers_shareable), |s| {
        s.server.stickers.iter().map(|e| (e.name.as_str(), e.blob.as_str())).collect()
    });
    let mut emoticons: Vec<SharedEmoticonView> = servers
        .values()
        .filter(|s| s.server.emoticons_shareable)
        .flat_map(|s| {
            s.server.emoticons.iter().map(|e| SharedEmoticonView {
                server: s.server.id.to_string(),
                server_name: s.server.name.clone(),
                name: e.name.clone(),
                text: e.text.clone(),
            })
        })
        .collect();
    emoticons.sort_by(|a, b| a.server_name.cmp(&b.server_name).then(a.name.cmp(&b.name)));
    SharedExpressions { emojis, stickers, emoticons }
}

#[tauri::command]
async fn fetch_expression(app: tauri::State<'_, App>, blob: String) -> CmdResult<String> {
    let bytes = expression_bytes(&app, &blob).await?;
    let mime = expressions::expression_mime(&bytes).ok_or("not a valid emoji or sound")?;
    Ok(data_url(mime, &bytes))
}

#[tauri::command]
async fn pick_sound(handle: tauri::AppHandle) -> CmdResult<Option<(String, String)>> {
    let filter = picker::Filter { label: "Sounds", extensions: &["mp3", "ogg", "wav"] };
    let max = expressions::MAX_SOUND_BYTES as u64;
    let Some(picked) = picker::pick_file(&handle, "Choose a sound", Some(filter), max, "sounds must be under 1 MB").await? else {
        return Ok(None);
    };
    let bytes = picked.bytes;
    let mime = expressions::audio_mime(&bytes).ok_or("sounds must be an MP3, OGG or WAV file")?;
    Ok(Some((
        picked.name.rsplit_once('.').map(|(stem, _)| stem.to_string()).unwrap_or_default(),
        data_url(mime, &bytes),
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
    drop(net);
    profile_images::prune(&app);
}

async fn restore_server_images(app: &tauri::State<'_, App>, node: &CommyNode) {
    let Some(dir) = app.data_dir().map(|d| d.join(SERVER_IMAGE_DIR)) else { return };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if let Ok(bytes) = commy_core::at_rest::read(&entry.path()) {
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
            let data = data.unwrap_or_default();
            let (too_large, not_image) = ("avatar image is too large", "avatar must be a JPEG or PNG image");
            let blob = store_profile_image(&app, &data, MAX_AVATAR_BYTES, too_large, not_image, AVATAR_FILE).await?;
            Some(commy_core::Avatar::Image { blob })
        }
        _ => return Err("unknown avatar kind".into()),
    };
    let profile = edit_profile(&app, |profile| profile.avatar = avatar.map(|a| a.encode()))?;
    publish_profile(&app, &handle, &profile).await;
    entry_list::record_profile_edit(&app);
    entry_list::schedule(&handle);
    Ok(profile)
}

const AVATAR_FILE: &str = "avatar.img";
const BANNER_FILE: &str = "profile-banner.img";
const MAX_BANNER_BYTES: usize = 512 * 1024;

#[tauri::command]
async fn set_profile_banner(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    data: Option<String>,
) -> CmdResult<UserProfile> {
    let banner = match data {
        None => None,
        Some(data) => {
            let (too_large, not_image) = ("banner image is too large", "banner must be a JPEG or PNG image");
            let blob = store_profile_image(&app, &data, MAX_BANNER_BYTES, too_large, not_image, BANNER_FILE).await?;
            Some(commy_core::Avatar::Image { blob }.encode())
        }
    };
    if banner.is_none() {
        if let Some(dir) = app.data_dir() {
            let _ = std::fs::remove_file(dir.join(BANNER_FILE));
        }
    }
    let profile = edit_profile(&app, |profile| profile.banner = banner)?;
    publish_profile(&app, &handle, &profile).await;
    Ok(profile)
}

pub(crate) fn is_jpeg_or_png(bytes: &[u8]) -> bool {
    view::pictures::is_jpeg_or_png(bytes)
}

fn decode_base64_image(data: &str) -> CmdResult<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| "image data is not valid base64".to_string())
}

fn decode_image(data: &str, max: usize, invalid: &str) -> CmdResult<Vec<u8>> {
    let bytes = decode_base64_image(data)?;
    if bytes.len() > max || !is_jpeg_or_png(&bytes) {
        return Err(invalid.into());
    }
    Ok(bytes)
}

async fn store_profile_image(
    app: &App,
    data: &str,
    max: usize,
    too_large: &str,
    not_image: &str,
    file: &str,
) -> CmdResult<String> {
    let bytes = decode_base64_image(data)?;
    if bytes.len() > max {
        return Err(too_large.into());
    }
    if !is_jpeg_or_png(&bytes) {
        return Err(not_image.into());
    }
    if let Some(dir) = app.data_dir() {
        commy_core::at_rest::write(&dir.join(file), &bytes).map_err(err_to_string)?;
    }
    let blob = {
        let net = app.net.lock().await;
        let node = net.as_ref().ok_or("network is not ready yet")?;
        node.add_blob(bytes.clone()).await.map_err(err_to_string)?
    };
    pictures::offer(&app.http, &blob, &bytes);
    Ok(blob)
}

async fn restore_avatar_blob(app: &tauri::State<'_, App>, node: &CommyNode) {
    let (wants_avatar, wants_banner) = {
        let guard = app.profile.lock().unwrap();
        let profile = guard.as_ref();
        let avatar = profile
            .and_then(|p| p.avatar.as_deref().and_then(commy_core::Avatar::parse))
            .is_some_and(|a| matches!(a, commy_core::Avatar::Image { .. }));
        let banner = profile.is_some_and(|p| p.banner.as_deref().is_some_and(commy_core::user::valid_banner));
        (avatar, banner)
    };
    for (wanted, file, what) in [(wants_avatar, AVATAR_FILE, "avatar"), (wants_banner, BANNER_FILE, "banner")] {
        if !wanted {
            continue;
        }
        let Some(path) = app.data_dir().map(|d| d.join(file)) else { continue };
        match commy_core::at_rest::read(&path) {
            Ok(bytes) => {
                if let Err(e) = node.add_blob(bytes).await {
                    tracing::warn!("could not restore {what} blob: {e}");
                }
            }
            Err(e) => tracing::warn!("{what} image missing: {e}"),
        }
    }
}

async fn announce_profile(app: &App, handle: &tauri::AppHandle, profile: &UserProfile) {
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

fn edit_profile(app: &App, edit: impl FnOnce(&mut UserProfile)) -> CmdResult<UserProfile> {
    let mut guard = app.profile.lock().unwrap();
    let profile = guard.as_mut().ok_or("set a username first")?;
    edit(profile);
    Ok(profile.clone())
}

async fn publish_profile(app: &App, handle: &tauri::AppHandle, profile: &UserProfile) {
    app.persist_profile();
    announce_profile(app, handle, profile).await;
    let _ = handle.emit("profile-changed", profile);
}

async fn set_auto_presence(app: &App, handle: &tauri::AppHandle, presence: Presence, auto_idle: bool) {
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
        let in_game = seen.as_deref().is_some_and(|text| text.starts_with("Playing "));
        if app.game_in_front.swap(in_game, std::sync::atomic::Ordering::Relaxed) != in_game {
            overlay::refresh(&handle);
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
    match resolve_game(app, name).await {
        Some(game) => (Some(format!("Playing {}", game.name)), game.cover),
        None => (Some(text), None),
    }
}

async fn resolve_game(app: &tauri::State<'_, App>, name: &str) -> Option<igdb::GameHit> {
    let creds = app.igdb_creds.lock().unwrap().clone();
    if creds.configured() {
        app.igdb.lock().await.resolve(&app.http, &creds, name).await
    } else {
        resolve_game_via_account(app, name).await
    }
}

const MAX_GAME_NAME_CHARS: usize = 128;

#[tauri::command]
async fn game_details(app: tauri::State<'_, App>, name: String) -> CmdResult<Option<igdb::GameHit>> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_GAME_NAME_CHARS {
        return Ok(None);
    }
    Ok(resolve_game(&app, name).await)
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

#[tauri::command]
async fn gif_search(app: tauri::State<'_, App>, query: String) -> CmdResult<Option<Vec<catalog::GifHit>>> {
    let hits = account::server_search::<catalog::GifHit>(&app, "gifs", &catalog::gif_query(&query)).await?;
    Ok(hits.map(catalog::safe_gifs))
}

#[tauri::command]
async fn set_share_activity(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    enabled: bool,
) -> CmdResult<VoiceSettings> {
    let settings = app.update_voice(|voice| voice.share_activity = enabled);
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
    let settings = app.update_voice(|voice| {
        voice.input_device = clean(input);
        voice.output_device = clean(output);
    });
    let testing = pause_mic_test(&app);
    if let Some(session) = app.voice_session.lock().unwrap().as_mut() {
        session.switch_devices(
            app.processor.clone(),
            settings.input_device.clone(),
            settings.output_device.clone(),
        );
    }
    if testing {
        let test = open_mic_test(&app).map_err(err_to_string)?;
        *app.mic_test.lock().unwrap() = Some(test);
    }
    Ok(settings)
}

#[tauri::command]
fn set_afk_timeout(app: tauri::State<App>, minutes: u32) -> VoiceSettings {
    app.update_voice(|voice| voice.afk_timeout_minutes = afk::clamp_timeout_minutes(minutes))
}

#[tauri::command]
fn set_volumes(app: tauri::State<App>, output_volume: f32, input_gain: f32) -> VoiceSettings {
    let snapshot = app.update_voice(|voice| {
        voice.output_volume = output_volume.clamp(0.0, 2.0);
        voice.input_gain = input_gain.clamp(0.0, 2.0);
    });
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
    apply_stream_volume(&app);
    snapshot
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn create_role(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    name: String,
    color: Option<String>,
    permissions: Vec<String>,
    tag: Option<String>,
    self_assignable: Option<bool>,
    hoist: Option<bool>,
    mentionable: Option<bool>,
) -> CmdResult<String> {
    let sid: ServerId = server_id.parse().map_err(err_to_string)?;
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 50 {
        return Err("role name must be 1 to 50 characters".into());
    }
    let color = color
        .map(|c| c.trim().trim_start_matches('#').to_lowercase())
        .filter(|c| commy_core::server::valid_hex_color(c));
    let tag = tag.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    if tag.as_deref().is_some_and(|t| !commy_core::server::valid_role_tag(t)) {
        return Err(format!("tags can be up to {} characters", commy_core::server::MAX_ROLE_TAG_CHARS));
    }
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
            hoist: hoist.unwrap_or(false),
            self_assignable: self_assignable.unwrap_or(false),
            tag,
            banner: None,
            mentionable: mentionable.unwrap_or(false),
        }
    };
    let id = role.id.to_string();
    apply_and_broadcast(&app, &handle, sid, ServerEvent::RoleCreate { server: sid, role }).await?;
    Ok(id)
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
    manage_check(&app, sid, me.id, Permissions::MANAGE_ROLES, "manage roles")?;
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
    manage_check(&app, sid, me.id, Permissions::MANAGE_ROLES, "manage roles")?;
    let event = if grant {
        ServerEvent::RoleAssign { server: sid, user: uid, role: rid }
    } else {
        ServerEvent::RoleUnassign { server: sid, user: uid, role: rid }
    };
    apply_and_broadcast(&app, &handle, sid, event).await?;
    channel_lock::tend_server(&app, &handle, sid).await;
    Ok(())
}

#[tauri::command]
async fn start_mic_test(app: tauri::State<'_, App>) -> CmdResult<()> {
    if app.mic_test.lock().unwrap().is_some() {
        return Ok(());
    }
    let test = open_mic_test(&app).map_err(err_to_string)?;
    *app.mic_test.lock().unwrap() = Some(test);
    Ok(())
}

#[tauri::command]
async fn stop_mic_test(app: tauri::State<'_, App>) -> CmdResult<()> {
    *app.mic_test.lock().unwrap() = None;
    Ok(())
}

fn is_data_url_safe_image_mime(peer_supplied: &str) -> bool {
    let subtype = peer_supplied.strip_prefix("image/").unwrap_or("");
    !subtype.is_empty() && subtype.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'))
}

const AUTO_PREVIEW_BYTES: u64 = 5 * 1024 * 1024;

fn preview_limit(clicked: bool) -> u64 {
    if clicked {
        commy_net::node::MAX_BLOB_BYTES
    } else {
        AUTO_PREVIEW_BYTES
    }
}

#[tauri::command]
async fn get_attachment_preview(
    app: tauri::State<'_, App>,
    blob: String,
    provider: Option<String>,
    mime: String,
    size_bytes: Option<u64>,
    clicked: Option<bool>,
    shared: Option<bool>,
) -> CmdResult<String> {
    if !is_data_url_safe_image_mime(&mime) {
        return Err("not an image".into());
    }
    let max = preview_limit(clicked.unwrap_or(false));
    if size_bytes.is_some_and(|size| size > max) {
        return Err("image too large to preview".into());
    }
    let shared = shared.unwrap_or(false);
    let held = match shared {
        true => pictures::fetch(&app.http, &blob, max).await,
        false => None,
    };
    let bytes = match held {
        Some(bytes) => bytes,
        None => {
            let bytes = fetch_blob(&app, &blob, provider.as_deref(), max).await?;
            if shared {
                pictures::offer(&app.http, &blob, &bytes);
            }
            bytes
        }
    };
    if bytes.len() as u64 > max {
        return Err("image too large to preview".into());
    }
    Ok(data_url(&mime, &bytes))
}

fn servers_showing_profile_image(app: &App, blob: &str) -> Vec<ServerId> {
    let servers = app.servers.lock().unwrap();
    servers
        .values()
        .filter(|state| state.profiles.values().any(|p| profile_images::image_blobs(p).iter().any(|b| b == blob)))
        .map(|state| state.server.id)
        .collect()
}

pub(crate) async fn profile_image_bytes(app: &App, blob: &str, provider: Option<&str>) -> CmdResult<Vec<u8>> {
    if let Some(bytes) = profile_images::cached(app, blob) {
        return Ok(bytes);
    }
    let max = profile_images::MAX_PROFILE_IMAGE_BYTES as u64;
    if let Some(bytes) = pictures::fetch(&app.http, blob, max).await.filter(|bytes| is_jpeg_or_png(bytes)) {
        profile_images::keep_if_wanted(app, blob, &bytes);
        return Ok(bytes);
    }
    let (fetcher, sources) = {
        let net = app.net.lock().await;
        let node = net.as_ref().ok_or("network is not ready yet")?;
        let neighbors = servers_showing_profile_image(app, blob).into_iter().flat_map(|s| node.neighbors(s)).collect();
        (node.blob_fetcher(), image_sources(provider.unwrap_or_default(), neighbors))
    };
    let mut attempts: Vec<Option<&str>> = sources.iter().filter(|s| !s.is_empty()).map(|s| Some(s.as_str())).collect();
    if attempts.is_empty() {
        attempts.push(None);
    }
    let mut failure = String::from("nobody who has it is online");
    for source in attempts {
        match fetcher.fetch(blob, source, max).await {
            Ok(bytes) if is_jpeg_or_png(&bytes) => {
                profile_images::keep_if_wanted(app, blob, &bytes);
                pictures::offer(&app.http, blob, &bytes);
                return Ok(bytes);
            }
            Ok(_) => failure = "not a profile picture".to_string(),
            Err(e) => failure = err_to_string(e),
        }
    }
    Err(failure)
}

#[tauri::command]
async fn get_profile_image(app: tauri::State<'_, App>, blob: String, provider: Option<String>) -> CmdResult<String> {
    let bytes = profile_image_bytes(&app, &blob, provider.as_deref()).await?;
    Ok(picture_data_url(&bytes))
}

#[tauri::command]
fn get_voice_settings(app: tauri::State<App>) -> VoiceSettings {
    app.voice.lock().unwrap().clone()
}

#[tauri::command]
fn set_noise_cancellation(app: tauri::State<App>, enabled: bool) -> VoiceSettings {
    app.update_voice(|voice| {
        voice.noise_cancellation = enabled;
        app.processor
            .lock()
            .unwrap()
            .set_noise_cancellation(enabled);
    })
}

#[tauri::command]
fn set_echo_cancellation(app: tauri::State<App>, enabled: bool) -> VoiceSettings {
    app.update_voice(|voice| {
        voice.echo_cancellation = enabled;
        app.processor.lock().unwrap().set_echo_cancellation(enabled);
    })
}

#[tauri::command]
async fn set_muted(app: tauri::State<'_, App>, handle: tauri::AppHandle, muted: bool) -> CmdResult<VoiceSettings> {
    let snapshot = app.update_voice(|voice| voice.muted = muted);
    apply_voice_gates(&app);
    announce_voice_state(&app, &handle).await;
    Ok(snapshot)
}

async fn announce_voice_state(app: &tauri::State<'_, App>, handle: &tauri::AppHandle) {
    let Some((sid, channel)) = app.voice_session.lock().unwrap().as_ref().map(|s| (s.server, s.channel)) else { return };
    let Ok(profile) = current_profile(app) else { return };
    let (muted, deafened) = {
        let voice = app.voice.lock().unwrap();
        (voice.muted, voice.deafened)
    };
    let (event, hand) = {
        let mut servers = app.servers.lock().unwrap();
        let Some(state) = servers.get_mut(&sid) else { return };
        let since = state.voice_since(profile.id).unwrap_or_else(|| chrono::Utc::now().timestamp());
        let event = ServerEvent::VoiceState {
            user: profile.id,
            channel: Some(channel),
            muted,
            deafened,
            since: Some(since),
        };
        let _ = state.apply(event.clone());
        let hand = (state.hand_of(profile.id) == Some(channel))
            .then(|| ServerEvent::HandRaise { server: sid, channel, user: profile.id, raised: true });
        (event, hand)
    };
    broadcast_event(app, sid, event).await;
    if let Some(hand) = hand {
        broadcast_event(app, sid, hand).await;
    }
    let _ = handle.emit("server-event", &sid.to_string());
}

fn apply_voice_gates(app: &App) {
    let (muted, deafened) = {
        let voice = app.voice.lock().unwrap();
        (voice.muted, voice.deafened)
    };
    let forced_mute = app.forced_mute.load(std::sync::atomic::Ordering::Relaxed);
    let forced_deaf = app.forced_deaf.load(std::sync::atomic::Ordering::Relaxed);
    let audience = app.audience.load(std::sync::atomic::Ordering::Relaxed);
    let deaf = deafened || forced_deaf;
    app.processor.lock().unwrap().set_muted(muted || deaf || forced_mute || audience);
    if let Some(session) = app.voice_session.lock().unwrap().as_ref() {
        session.deafened.store(deaf, std::sync::atomic::Ordering::Relaxed);
    }
    apply_stream_volume(app);
}

fn announce_new_owner(app: &App, handle: &tauri::AppHandle, sid: ServerId, user: UserId) {
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let (name, who) = {
        let servers = app.servers.lock().unwrap();
        let Some(state) = servers.get(&sid) else { return };
        (state.server.name.clone(), view::display_name(state, user))
    };
    let _ = handle.emit(
        "owner-changed",
        serde_json::json!({ "server": sid.to_string(), "name": name, "user": user.to_string(), "who": who, "mine": me == Some(user) }),
    );
}

async fn follow_voice_move(handle: &tauri::AppHandle, sid: ServerId, channel: Option<ChannelId>) {
    let app = handle.state::<App>();
    let in_voice_here = app.voice_session.lock().unwrap().as_ref().is_some_and(|s| s.server == sid);
    if !in_voice_here {
        return;
    }
    let name = channel.and_then(|c| {
        let servers = app.servers.lock().unwrap();
        servers.get(&sid).and_then(|s| s.server.channel(c)).map(|c| c.name.clone())
    });
    if channel.is_some() && name.is_none() {
        return;
    }
    tracing::info!("a moderator moved us to {name:?}");
    let moved = set_voice_channel(handle.state::<App>(), handle.clone(), sid.to_string(), channel.map(|c| c.to_string())).await;
    if moved.is_ok() {
        let _ = handle.emit(
            "voice-moved",
            serde_json::json!({ "server": sid.to_string(), "channel": channel.map(|c| c.to_string()), "name": name }),
        );
    }
}

fn refresh_forced_voice(app: &App, handle: &tauri::AppHandle) {
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let session = app.voice_session.lock().unwrap().as_ref().map(|s| (s.server, s.channel));
    let server = session.map(|(sid, _)| sid);
    let (muted, deafened, audience) = match (me, session) {
        (Some(me), Some((sid, channel))) => app.servers.lock().unwrap().get(&sid).map_or((false, false, false), |s| {
            (s.server.server_muted.contains(&me), s.server.server_deafened.contains(&me), s.server.in_audience(channel, me))
        }),
        _ => (false, false, false),
    };
    if let Some(sid) = server {
        let timeouts = {
            let servers = app.servers.lock().unwrap();
            servers.get(&sid).map(|s| {
                s.server
                    .timed_out_members(chrono::Utc::now())
                    .into_iter()
                    .map(|(user, timeout)| (user, timeout.until.timestamp_millis()))
                    .collect::<std::collections::HashMap<_, _>>()
            })
        };
        if let (Some(session), Some(timeouts)) = (app.voice_session.lock().unwrap().as_mut(), timeouts) {
            session.set_timeouts(timeouts);
        }
    }
    let was = (
        app.forced_mute.swap(muted, std::sync::atomic::Ordering::Relaxed),
        app.forced_deaf.swap(deafened, std::sync::atomic::Ordering::Relaxed),
    );
    let was_audience = app.audience.swap(audience, std::sync::atomic::Ordering::Relaxed);
    apply_voice_gates(app);
    if was != (muted, deafened) {
        let _ = handle.emit("voice-moderated", serde_json::json!({ "muted": muted, "deafened": deafened }));
    }
    if was_audience != audience {
        let _ = handle.emit("stage-role", serde_json::json!({ "audience": audience }));
    }
}

#[tauri::command]
async fn set_deafened(app: tauri::State<'_, App>, handle: tauri::AppHandle, deafened: bool) -> CmdResult<VoiceSettings> {
    let snapshot = app.update_voice(|voice| {
        voice.deafened = deafened;
        if deafened {
            voice.muted = true;
        }
    });
    apply_voice_gates(&app);
    announce_voice_state(&app, &handle).await;
    Ok(snapshot)
}

async fn broadcast_event(app: &App, server: ServerId, event: ServerEvent) {
    let payload = app.wire_for(server, event);
    match &payload {
        WirePayload::Event(ServerEvent::MemberJoin { profile, .. }) => community_sync::announce_profile(server, profile, &payload),
        WirePayload::Signed(signed) => match &signed.event {
            ServerEvent::MemberJoin { profile, .. } => community_sync::announce_profile(server, profile, &payload),
            _ => community_sync::publish(server, &payload),
        },
        _ => community_sync::publish(server, &payload),
    }
    broadcast(app, server, payload).await;
}

fn remote_action_ok(app: &App, server: ServerId, event: &ServerEvent, action: Option<commy_core::Action>) -> bool {
    let servers = app.servers.lock().unwrap();
    let Some(state) = servers.get(&server) else { return false };
    match action {
        Some(action) => state.admits_signed(&SignedEvent { action, event: event.clone() }).is_ok(),
        None => state.admits_unsigned(event).is_ok(),
    }
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
            NetEvent::Event { server, sender, event, action } => {
                if let ServerEvent::TypingStart { channel, user } = &event {
                    if remote_action_ok(&app, server, &event, action) {
                        liveness::heard(&app, server, *user, std::time::Instant::now());
                        let _ = handle.emit(
                            "typing",
                            serde_json::json!({ "server": server.to_string(), "channel": channel.to_string(), "user": user.to_string() }),
                        );
                    }
                    continue;
                }
                if let ServerEvent::SoundboardPlay { channel, user, name, .. } = &event {
                    if remote_action_ok(&app, server, &event, action) {
                        on_remote_soundboard(&app, &handle, server, *channel, *user, name);
                    }
                    continue;
                }
                if let ServerEvent::SharedSoundPlay { channel, user, name, blob, volume, .. } = &event {
                    if remote_action_ok(&app, server, &event, action) {
                        on_remote_shared_sound(&app, &handle, server, *channel, *user, name, blob, *volume);
                    }
                    continue;
                }
                if let ServerEvent::ActivityInput { channel, seq, input, by, .. } = &event {
                    if remote_action_ok(&app, server, &event, action) {
                        on_remote_activity_input(&app, &handle, server, *channel, *seq, input, *by);
                    }
                    continue;
                }
                let activity_set = match &event {
                    ServerEvent::ActivitySet { channel, activity, by, .. } => Some(activity_set_view(server, *channel, activity, *by)),
                    _ => None,
                };
                if matches!(event, ServerEvent::DoorbellSet { .. }) {
                    doorbell::on_announce(&app, server, &event);
                    continue;
                }
                if let ServerEvent::ScreenShare { user, active: true, .. } = &event {
                    app.share_seen.lock().unwrap().insert((server, *user), std::time::Instant::now());
                }
                let moderation = matches!(
                    event,
                    ServerEvent::VoiceModerate { .. }
                        | ServerEvent::MemberTimeout { .. }
                        | ServerEvent::StageSet { .. }
                        | ServerEvent::RoleAssign { .. }
                        | ServerEvent::RoleUnassign { .. }
                        | ServerEvent::RoleUpdate { .. }
                        | ServerEvent::RoleDelete { .. }
                );
                let priority = matches!(
                    event,
                    ServerEvent::VoiceState { .. }
                        | ServerEvent::VoiceMove { .. }
                        | ServerEvent::RoleAssign { .. }
                        | ServerEvent::RoleUnassign { .. }
                        | ServerEvent::RoleUpdate { .. }
                        | ServerEvent::RoleDelete { .. }
                        | ServerEvent::OwnerTransfer { .. }
                );
                let my_timeout = match &event {
                    ServerEvent::MemberTimeout { user, until, .. }
                        if app.profile.lock().unwrap().as_ref().is_some_and(|p| p.id == *user) =>
                    {
                        Some(until.map(|t| t.timestamp_millis()))
                    }
                    _ => None,
                };
                let access = matches!(
                    event,
                    ServerEvent::ChannelAccessSet { .. }
                        | ServerEvent::RoleUnassign { .. }
                        | ServerEvent::RoleDelete { .. }
                        | ServerEvent::RoleUpdate { .. }
                );
                let banned_me = match &event {
                    ServerEvent::MemberBan { user, .. } => app.profile.lock().unwrap().as_ref().is_some_and(|p| p.id == *user),
                    _ => false,
                };
                let new_owner = match &event {
                    ServerEvent::OwnerTransfer { user, .. } => Some(*user),
                    _ => None,
                };
                let moved_me = match &event {
                    ServerEvent::VoiceMove { user, channel, .. }
                        if app.profile.lock().unwrap().as_ref().is_some_and(|p| p.id == *user) =>
                    {
                        Some(*channel)
                    }
                    _ => None,
                };
                let voice_update = match &event {
                    ServerEvent::VoiceState { user, channel, .. } => Some((*user, *channel)),
                    _ => None,
                };
                let deletion = deleted::is_notice(&event)
                    .then(|| action.map(|action| SignedEvent { action, event: event.clone() }))
                    .flatten();
                let holdable = catchup::holdable(&event);
                let about = commy_core::action::subject(&event);
                let early = app.awaits_snapshot(server);
                let kept = (holdable || action.is_some()).then(|| event.clone());
                let new_emoji = matches!(event, ServerEvent::EmojiAdd { .. }).then(|| event.clone());
                let profile_changed = match &event {
                    ServerEvent::ProfileUpdate { profile } | ServerEvent::MemberJoin { profile, .. } => Some(profile.id),
                    _ => None,
                };
                let replayed = sender == commy_net::community_streams::SERVICE_SENDER;
                let presence = outbox::online_user(&event).filter(|_| !replayed);
                let device_presence = match &event {
                    ServerEvent::PresenceUpdate { user, presence } => Some((*user, *presence)),
                    _ => None,
                };
                let device_news = matches!(event, ServerEvent::DeviceAnnounce { .. }).then(|| event.clone());
                let speaker = liveness::speaker(&event, action.as_ref()).filter(|_| !replayed);
                let (applied, was_online, new_message, pinned) = {
                    let mut servers = app.servers.lock().unwrap();
                    match servers.get_mut(&server) {
                        Some(state) => {
                            let was_online = presence.map(|(user, _)| {
                                state.profiles.get(&user).is_some_and(|p| p.presence.is_visible_online())
                            });
                            let shown = profile_changed
                                .filter(|_| replayed)
                                .map(|user| state.profiles.get(&user).map_or(Presence::Offline, |p| p.presence));
                            let (mut applied, new_message, pinned) = catchup::apply_live(state, event, action);
                            if let (Some(user), Some(shown)) = (profile_changed, shown) {
                                if let Some(profile) = state.profiles.get_mut(&user) {
                                    profile.presence = shown;
                                }
                            }
                            if let (Ok(_), Some((user, said))) = (&applied, device_presence) {
                                let now = std::time::Instant::now();
                                if liveness::settle_device_presence(&app, state, user, &sender, said, now) {
                                    applied = Ok(true);
                                }
                            }
                            (applied, was_online, new_message, pinned)
                        }
                        None => continue,
                    }
                };
                if let (Ok(_), Some(news)) = (&applied, &device_news) {
                    devices::on_announce(&handle, server, news).await;
                }
                if applied.is_ok() {
                    if let Some(user) = speaker {
                        liveness::heard(&app, server, user, std::time::Instant::now());
                    }
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
                        if let Some(notice) = deletion {
                            on_server_deleted(&app, &handle, server, notice);
                            continue;
                        }
                        app.persist_server(server);
                        directory::server_changed(&app, server);
                        if let Some(user) = new_owner {
                            announce_new_owner(&app, &handle, server, user);
                        }
                        if let Some(event) = &new_emoji {
                            prefetch_emoji(&handle, event);
                        }
                        if let Some(user) = profile_changed {
                            let stored = app.servers.lock().unwrap().get(&server).and_then(|s| s.profiles.get(&user).cloned());
                            if let Some(profile) = stored {
                                profile_images::prefetch(&handle, &profile);
                            }
                        }
                        if moderation {
                            refresh_forced_voice(&app, &handle);
                        }
                        if priority {
                            refresh_priority_speakers(&app);
                        }
                        if let Some(until) = my_timeout {
                            let _ = handle.emit("timed-out", serde_json::json!({ "server": server.to_string(), "until": until }));
                        }
                        if let Some(channel) = moved_me {
                            follow_voice_move(&handle, server, channel).await;
                        }
                        if let Some(view) = activity_set {
                            let _ = handle.emit("activity-set", view);
                        }
                        if access {
                            leave_unseen_voice(&app, &handle, server).await;
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
                    Err(e) => match (e, kept, action) {
                        (_, Some(event), Some(action)) if early => {
                            app.catchup.lock().unwrap().hold_early(server, SignedEvent { action, event }, std::time::Instant::now())
                        }
                        (StoreError::UnknownAuthorKey(signer), Some(event), _) if holdable => {
                            catchup::hold(&app, server, signer, event)
                        }
                        (StoreError::UnknownAuthorKey(_), Some(event), Some(action)) => {
                            let signed = SignedEvent { action, event };
                            app.catchup.lock().unwrap().hold_action(server, signed, std::time::Instant::now())
                        }
                        (e, _, _) => tracing::warn!("rejected remote event about {}: {e}", about.as_deref().unwrap_or("something")),
                    },
                }
            }
            NetEvent::SyncRequested { server, .. } => {
                for payload in catchup::sync_replies(&app, server) {
                    broadcast(&app, server, payload).await;
                }
            }
            NetEvent::SyncReceived { server, payload } => {
                if catchup::apply_snapshot(&handle, server, payload) {
                    community_sync::refresh_soon(&handle);
                }
            }
            NetEvent::VoiceDial { .. } => {}
            NetEvent::NeighborUp { server, peer } => {
                let notice = app.deleted.lock().unwrap().notice(server);
                if let Some(notice) = notice {
                    broadcast(&app, server, WirePayload::Signed(notice)).await;
                    continue;
                }
                let me = app.profile.lock().unwrap().clone();
                let Some(me) = me else { continue };
                let my_id = me.id;
                let needs_sync = app.awaits_snapshot(server);
                let mut profile = app.announced(me);
                if let Some(state) = app.servers.lock().unwrap().get(&server) {
                    devices::point_at_voice(state, &mut profile);
                }
                broadcast_event(&app, server, ServerEvent::MemberJoin { server, profile }).await;
                if needs_sync {
                    broadcast(&app, server, WirePayload::SyncRequest).await;
                }
                for payload in neighbor_announcements(&app, server, my_id) {
                    broadcast(&app, server, payload).await;
                }
                devices::announce(&app, server).await;
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
    if app.profile.lock().unwrap().as_ref().is_some_and(|p| p.id == user) {
        return;
    }

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
            let their_hex = app.servers.lock().unwrap().get(&server).and_then(|s| s.voice_node(user));
            let Some(their_hex) = their_hex else { return };
            if should_dial(&our_hex, &their_hex) && claim_voice_dial(app, our_channel, user) {
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
    let keys = channel_lock::holder_of(app);
    let Some(me) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { return };
    let payload = {
        let servers = app.servers.lock().unwrap();
        let Some(state) = servers.get(&sid) else { return };
        if !state.server.can_view(message.channel, me) {
            return;
        }
        if keys.hold(state, message).is_some() {
            return;
        }
        let Some(message) = keys.open(state, message) else { return };
        let mut names = app.profile.lock().unwrap().as_ref().map(catchup::mention_names).unwrap_or_default();
        names.extend(view::nickname_of(state, me));
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let mentions = state.server.mentions_of(message.author, &message.content, me, &names);
        serde_json::json!({
            "server": sid.to_string(),
            "server_name": state.server.name,
            "channel": message.channel.to_string(),
            "channel_name": state.server.channel(message.channel).map(|c| c.name.clone()).unwrap_or_default(),
            "announcement": state.server.channel(message.channel).is_some_and(|c| c.kind == ChannelKind::Announcement),
            "sensitive": state.server.channel(message.channel).is_some_and(|c| c.sensitive),
            "author": message.author.to_string(),
            "author_name": view::known_name_of(state, message.author).unwrap_or_else(|| "Someone".into()),
            "mentioned": mentions.any(),
            "mentions": mentions.kinds(),
            "content": message.content,
            "has_attachments": message.files().next().is_some(),
            "voice_message": commy_core::message::only_a_voice_message(&message.content, message.attachments.iter()),
            "tts": message.speaks_for(state.server.permissions_for(message.author)),
        })
    };
    let _ = handle.emit("message-created", payload);
}

#[tauri::command]
fn notify(handle: tauri::AppHandle, title: String, body: String, target: Option<String>) -> CmdResult<()> {
    let target = target.filter(|t| toast::valid_target(t));
    toast::show(&handle, &safety::truncate(&title, 80), &safety::truncate(&body, 240), target)
}

#[tauri::command]
fn set_unread(handle: tauri::AppHandle, count: u32) {
    tray::set_unread(&handle, count);
}

#[tauri::command]
fn set_close_to_tray(app: tauri::State<App>, enabled: bool) -> VoiceSettings {
    app.update_voice(|voice| voice.close_to_tray = enabled)
}

#[tauri::command]
fn tray_prompt_due(app: tauri::State<App>) -> bool {
    cfg!(desktop) && !app.voice.lock().unwrap().tray_prompt_seen
}

#[tauri::command]
fn autostart_allowed() -> bool {
    cfg!(desktop) && instance::autostart_allowed_here()
}

#[tauri::command]
fn finish_tray_prompt(
    app: tauri::State<App>,
    handle: tauri::AppHandle,
    autostart: bool,
    close_to_tray: bool,
) -> VoiceSettings {
    #[cfg(desktop)]
    if instance::autostart_allowed_here() {
        let _ = set_autostart(handle, autostart);
    }
    #[cfg(mobile)]
    let _ = (&handle, autostart);
    app.update_voice(|voice| {
        voice.tray_prompt_seen = true;
        voice.close_to_tray = close_to_tray;
    })
}

#[cfg(mobile)]
#[tauri::command]
fn get_autostart() -> bool {
    false
}

#[cfg(mobile)]
#[tauri::command]
fn set_autostart(_enabled: bool) -> CmdResult<bool> {
    Ok(false)
}

#[cfg(desktop)]
#[tauri::command]
fn get_autostart(handle: tauri::AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    handle.autolaunch().is_enabled().unwrap_or(false)
}

#[cfg(desktop)]
#[tauri::command]
fn set_autostart(handle: tauri::AppHandle, enabled: bool) -> CmdResult<bool> {
    use tauri_plugin_autostart::ManagerExt;
    let launcher = handle.autolaunch();
    if enabled && !instance::autostart_allowed_here() {
        return Ok(false);
    }
    if enabled {
        launcher.enable().map_err(|e| e.to_string())?;
    } else {
        launcher.disable().map_err(|e| e.to_string())?;
    }
    Ok(launcher.is_enabled().unwrap_or(enabled))
}

#[tauri::command]
fn set_auto_update(app: tauri::State<App>, handle: tauri::AppHandle, enabled: bool) -> VoiceSettings {
    let settings = app.update_voice(|voice| voice.auto_update = enabled);
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
        take_dm(&handle, dm, true).await;
    }
}

pub(crate) async fn take_dm(handle: &tauri::AppHandle, dm: commy_net::node::IncomingDm, ring: bool) {
    let app = handle.state::<App>();
    let sender = dm.envelope.message.from;
    let received = if ring { outbox::receive_dm(handle, dm) } else { outbox::receive_dm(&dm_sync::Quiet(handle.clone()), dm) };
    match received {
        outbox::Received::Report { from, content } => receive_report(&app, handle, from, &content),
        outbox::Received::Reply { to, signal } => {
            let _ = send_friend_signal(&app, to, signal, None).await;
        }
        outbox::Received::Call { from, call, signal } => calls::on_signal(&app, handle, from, call, signal).await,
        outbox::Received::GroupInvite { from, ticket, key } => join_group(&app, handle, from, &ticket, key).await,
        outbox::Received::Taken => {
            dm_sync::noticed(handle, sender);
            let contact = app.dms.lock().unwrap().contacts.get(&sender).cloned();
            if let Some(profile) = contact {
                profile_images::prefetch(handle, &profile);
            }
        }
        outbox::Received::Dropped => {}
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
                        let MediaChannel { incoming, outgoing, .. } = media;
                        s.add_viewer(outgoing, incoming);
                        tracing::info!("viewer {} subscribed to our screen", user);
                    }
                    _ => tracing::debug!("screen subscribe while not sharing"),
                }
            }
        }
    }
}

pub(crate) fn open_account(handle: &tauri::AppHandle, dir: std::path::PathBuf) {
    let app = handle.state::<App>();
    if app.data_dir.set(dir.clone()).is_err() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!("could not create the account folder: {e}");
    }
    if let Some(mut profile) = persist::load_profile(&dir) {
        profile.activity = None;
        profile.activity_cover = None;
        profile.activity_since = None;
        *app.profile.lock().unwrap() = Some(profile);
    }
    if let Some(dms) = persist::load_dms(&dir) {
        *app.dms.lock().unwrap() = dms;
    }
    if let Some(creds) = persist::load_integrations(&dir) {
        *app.igdb_creds.lock().unwrap() = creds;
    }
    stream_deck::load(&app, &handle);
    if let Some(storage) = persist::load_storage(&dir) {
        *app.storage.lock().unwrap() = storage;
    }
    *app.listings.lock().unwrap() = directory::load(&dir);
    match persist::load_safety(&dir) {
        Some(safety) => *app.safety.lock().unwrap() = safety,
        None => {
            let partners = app.dms.lock().unwrap().partners();
            app.safety.lock().unwrap().accepted.extend(partners);
        }
    }
    if let Some(voice) = persist::load_voice(&dir) {
        voice.apply_to(&mut app.processor.lock().unwrap());
        let (app_choice, tray_choice) = (voice.app_icon.clone(), voice.tray_icon.clone());
        *app.voice.lock().unwrap() = voice;
        app_icon::apply(&handle, Some(dir.as_path()), &app_choice, &tray_choice);
    }
    {
        let mut servers = app.servers.lock().unwrap();
        for state in persist::load_servers(&dir) {
            servers.insert(state.server.id, state);
        }
    }
    *app.deleted.lock().unwrap() = deleted::DeletedServers::load(&dir, chrono::Utc::now().timestamp_millis());
    let mut awaiting = persist::load_awaiting(&dir);
    {
        let servers = app.servers.lock().unwrap();
        awaiting.retain(|id| servers.contains_key(id));
    }
    *app.awaiting_snapshot.lock().unwrap() = awaiting;
    let mut queue = outbox::Outbox::load(&dir);
    let now = chrono::Utc::now();
    let pruned = app.servers.lock().unwrap().values().fold(false, |acc, s| queue.prune(s, now) | acc);
    *app.outbox.lock().unwrap() = queue;
    if pruned {
        app.persist_outbox();
    }
    let mut bell = doorbell::Doorbell::load(&dir);
    {
        let servers = app.servers.lock().unwrap();
        bell.retain_servers(|id| servers.contains_key(id));
    }
    *app.doorbell.lock().unwrap() = bell;
    {
        let handle = handle.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let voice = handle.state::<App>().voice.lock().unwrap().clone();
            register_hotkeys(&handle, &voice);
            overlay::refresh(&handle);
        });
    }
    let handle = handle.clone();
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
    let node_port = app.data_dir().and_then(|dir| persist::load_node_port(dir));

    tauri::async_runtime::spawn(doorbell::run(handle.clone()));
    tauri::async_runtime::spawn(directory::keep_listings_current(handle.clone()));
    tauri::async_runtime::spawn(async move {
        match CommyNode::spawn_on_port(node_key, node_port).await {
            Ok((mut node, events, media, dms)) => {
                let app = handle.state::<App>();
                if let (Some(dir), Some(port)) = (app.data_dir(), node.bound_port()) {
                    persist::save_node_port(dir, port);
                }
                let hex = node.node_id_hex();
                if let Some(asks) = node.take_entry_key_requests() {
                    tauri::async_runtime::spawn(entry_list::run_entry_key_requests(handle.clone(), asks));
                }
                if let Some(asks) = node.take_mirror_requests() {
                    tauri::async_runtime::spawn(mirror::run_mirror_requests(handle.clone(), asks));
                }

                let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
                let stored_invites = app
                    .data_dir()
                    .map(|d| persist::load_invites(d))
                    .unwrap_or_default();
                catchup::mark_joined_placeholders(&app, &stored_invites, node.node_id_bytes());
                catchup::release_own_servers(&app, &stored_invites);
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
                        match node.host_server(id, &name, app.own_key_for_tickets()).await {
                            Ok(ticket) => {
                                app.invites.lock().unwrap().insert(id, ticket.encode());
                            }
                            Err(e) => tracing::warn!("re-host failed for {name}: {e}"),
                        }
                    } else if let Some(raw) = stored_invites.get(&id) {
                        match ServerTicket::decode(raw) {
                            Ok(ticket) => {
                                if let Err(e) = node.rejoin_server(&ticket).await {
                                    tracing::warn!("rejoin failed for {name}: {e}");
                                }
                                app.invites.lock().unwrap().insert(id, raw.clone());
                            }
                            Err(e) => tracing::warn!("stored invite corrupt for {name}: {e}"),
                        }
                    }
                }
                app.persist_invites();
                let retired = app.deleted.lock().unwrap().to_rejoin();
                for (id, gone) in retired {
                    let rejoined = match gone.ticket.as_deref().map(ServerTicket::decode) {
                        Some(Ok(ticket)) => node.rejoin_server(&ticket).await,
                        Some(Err(e)) => Err(e.into()),
                        None => node.host_server(id, &gone.name, app.own_key_for_tickets()).await.map(|_| ()),
                    };
                    if let Err(e) = rejoined {
                        tracing::debug!("could not stay on a deleted server to pass on its deletion: {e}");
                    }
                }

                let afk_channels: Vec<(ServerId, ServerEvent)> = {
                    let mut servers = app.servers.lock().unwrap();
                    servers
                        .values_mut()
                        .filter(|s| Some(s.server.owner) == me && s.server.afk_channel().is_none())
                        .map(|state| {
                            let sid = state.server.id;
                            let position = state.server.channels.len() as u32;
                            let channel = Channel::new("AFK", ChannelKind::Voice, position);
                            let event = ServerEvent::ChannelCreate { server: sid, channel };
                            let _ = state.apply(event.clone());
                            (sid, event)
                        })
                        .collect()
                };
                for (sid, event) in afk_channels {
                    let payload = app.wire_for(sid, event);
                    community_sync::publish(sid, &payload);
                    if let Err(e) = node.broadcast(sid, payload).await {
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
                node.set_media_gate(media_gate(&handle));
                *app.net.lock().await = Some(node);
                tracing::info!("iroh node online");

                tauri::async_runtime::spawn(run_media_loop(handle.clone(), media));
                tauri::async_runtime::spawn(run_dm_loop(handle.clone(), dms));
                tauri::async_runtime::spawn(run_afk_watchdog(handle.clone()));
                tauri::async_runtime::spawn(run_voice_redialer(handle.clone()));
                tauri::async_runtime::spawn(run_status_expiry(handle.clone()));
                tauri::async_runtime::spawn(run_event_reminders(handle.clone()));
                {
                    let handle = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        account::verify_saved_session(handle.clone()).await;
                        entry_list::schedule(&handle);
                    });
                }
                tauri::async_runtime::spawn(read_state::run(handle.clone()));
                tauri::async_runtime::spawn(run_share_heartbeat(handle.clone()));
                tauri::async_runtime::spawn(liveness::run(handle.clone()));
                tauri::async_runtime::spawn(run_activity_watcher(handle.clone()));
                tauri::async_runtime::spawn(run_retention_pruner(handle.clone()));
                tauri::async_runtime::spawn(run_mesh_healer(handle.clone()));
                tauri::async_runtime::spawn(channel_lock::run(handle.clone()));
                tauri::async_runtime::spawn(community_sync::run(handle.clone()));
                tauri::async_runtime::spawn(wake::run_watchdog(handle.clone()));
                tauri::async_runtime::spawn(run_blob_restore(handle.clone()));
                tauri::async_runtime::spawn(catchup::run_periodic(handle.clone()));
                tauri::async_runtime::spawn(outbox::run_dm_outbox(handle.clone()));
                run_net_loop(handle.clone(), events).await;
            }
            Err(e) => tracing::error!("P2P stack failed to start: {e}"),
        }
    });
}

fn on_resumed(handle: &tauri::AppHandle) {
    entry_list::on_resumed(handle);
    account_log::nudge();
    dm_sync::nudge();
    community_sync::nudge();
    push_wake::on_resumed(handle);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,iroh=warn,iroh_net=warn".into()),
        )
        .try_init();

    instance::apply_test_build_data_dir();

    let launch_link = invite_link::link_from_args();
    if let (Some(link), Some(dir)) = (&launch_link, invite_link::default_data_dir()) {
        if invite_link::forward_to_running_instance(&dir, link) {
            return;
        }
    }

    instance::apply_webview_data_dir_before_any_window();

    let builder = tauri::Builder::default()
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
        .plugin(tauri_plugin_notification::init());
    #[cfg(mobile)]
    let builder = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init());
    #[cfg(target_os = "android")]
    let builder = builder.plugin(call_service::init()).plugin(relaunch::init()).plugin(push_wake::init());
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|handle, shortcut, event| {
                    let pressed = event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed;
                    on_hotkey(handle, shortcut.id(), pressed);
                })
                .build(),
        );
    builder
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() != "main" {
                    return;
                }
                let to_tray = window.state::<App>().voice.lock().unwrap().close_to_tray;
                if to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                    let first = {
                        let app = window.state::<App>();
                        let mut seen = false;
                        app.update_voice(|voice| {
                            seen = voice.tray_hint_seen;
                            voice.tray_hint_seen = true;
                        });
                        !seen
                    };
                    if first {
                        let _ = toast::show(
                            window.app_handle(),
                            "Commy is still running",
                            "It lives in the system tray so messages and calls keep arriving. Quit from the tray icon.",
                            None,
                        );
                    }
                }
            }
        })
        .setup(|tauri_app| {
            let handle = tauri_app.handle().clone();
            let label = std::env::var("COMMY_INSTANCE_LABEL").ok();
            if let Some(title) = instance::window_title(instance::TEST_BUILD, label.as_deref()) {
                if let Some(window) = tauri_app.get_webview_window("main") {
                    let _ = window.set_title(&title);
                }
            }
            #[cfg(windows)]
            if let Some(window) = tauri_app.get_webview_window("main") {
                let _ = window.set_decorations(false);
            }
            updater::init(&handle);
            updater::start(&handle);
            let data_dir = std::env::var("COMMY_DATA_DIR")
                .ok()
                .map(std::path::PathBuf::from)
                .or_else(|| tauri_app.path().app_data_dir().ok());
            let show_window = data_dir.as_deref().is_some_and(account_folder::take_show_window);
            match tray::build(&handle) {
                Ok(()) => {
                    if !show_window && std::env::args().any(|arg| arg == "--minimized") {
                        if let Some(window) = tauri_app.get_webview_window("main") {
                            let _ = window.hide();
                        }
                    }
                }
                Err(e) => tracing::warn!("tray icon unavailable: {e}"),
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

            if let Some(dir) = data_dir {
                if let Err(why) = local_key::unlock(&dir) {
                    tracing::error!("local files are encrypted and their key is unavailable: {why}");
                    #[cfg(desktop)]
                    rfd::MessageDialog::new()
                        .set_level(rfd::MessageLevel::Error)
                        .set_title("Commy cannot open your data")
                        .set_description(format!(
                            "Your messages and keys on this computer are encrypted, and Commy could not get their key from your system keychain ({why}). Allow Commy to use it, then start Commy again."
                        ))
                        .show();
                    std::process::exit(1);
                }
                let _ = app.base_dir.set(dir.clone());
                invite_link::register_scheme();
                invite_link::start(&handle, &dir, launch_link);
                *app.account.lock().unwrap() = account::load(&dir);
                tauri::async_runtime::spawn(account::resume_sign_in(handle.clone()));
                account_folder::set_aside_unowned_data(&dir);
                if let Some(folder) = account::ready_folder(&app) {
                    open_account(&handle, folder);
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            doorbell::get_awaited,
            link_preview::prepare_link_preview,
            account::account_status,
            account_log::sync_prefs_load,
            account_log::sync_prefs_set,
            account::set_account_server,
            account::account_begin_sign_in,
            account::account_cancel_sign_in,
            account::account_sign_out,
            account::account_switch,
            account::account_sign_out_other,
            account::account_claim_profile,
            account::account_open_folder,
            voice_stats,
            set_server_afk_timeout,
            list_friends,
            friend_action,
            set_about,
            set_window_title,
            has_custom_titlebar,
            window_action,
            set_icons,
            choose_custom_icon,
            get_custom_icon,
            set_pronouns,
            moderate_voice,
            set_stage_speaker,
            raise_hand,
            set_nickname,
            move_voice_member,
            ban_member,
            unban_member,
            time_out_member,
            clear_timeout,
            set_user_voice,
            set_role_banner,
            get_integrations,
            set_igdb_credentials,
            search_games,
            game_details,
            gif_search,
            sticker_search,
            get_overlay_settings,
            overlay_hotkey_problems,
            voice_hotkey_problems,
            set_input_mode,
            set_voice_hotkeys,
            set_sensitivity,
            mic_level,
            send_typing,
            set_overlay_settings,
            preview_overlay,
            set_game_tags,
            set_rules,
            set_automod,
            accept_rules,
            set_onboarding,
            answer_onboarding,
            save_event,
            delete_event,
            set_rsvp,
            move_channels,
            create_category,
            set_channel_limit,
            set_channel_slow_mode,
            set_thread,
            set_forum_tags,
            post_forum,
            edit_forum_post,
            update_role,
            set_own_role,
            get_safety,
            set_status_expiry,
            set_blocked,
            set_dm_settings,
            set_profile_privacy,
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
            set_sound_sharing,
            set_activities_enabled,
            set_role_access,
            add_sticker,
            remove_sticker,
            add_emoticon,
            remove_emoticon,
            set_expression_sharing,
            search_emoji_catalog,
            fetch_catalog_image,
            store_catalog_emoji,
            store_catalog_sticker,
            list_shareable_sounds,
            play_shared_sound,
            set_server_public,
            play_soundboard,
            set_activity,
            send_activity_input,
            stream_deck::get_stream_deck,
            stream_deck::set_stream_deck_enabled,
            stream_deck::forget_stream_deck,
            stream_deck::answer_stream_deck_pairing,
            fetch_expression,
            pick_sound,
            list_shareable_expressions,
            ui_ready,
            is_mobile_app,
            init_user,
            get_user,
            create_server,
            templates::list_server_templates,
            templates::preview_server_template,
            templates::export_server_template,
            join_server,
            list_servers,
            get_server,
            get_invite,
            invite_link::resolve_invite_link,
            invite_link::create_invite_link,
            invite_link::list_invites,
            invite_link::revoke_invite,
            invite_link::account_server_info,
            invite_link::take_pending_invite_link,
            directory::publish_listing,
            directory::unlist_server,
            directory::server_listing,
            directory::browse_directory,
            entry_list::entry_list_status,
            entry_list::entry_list_sync,
            entry_list::entry_list_recovery_code,
            entry_list::entry_list_use_recovery_code,
            entry_list::entry_list_start_over,
            entry_list::answer_entry_key_request,
            read_state::read_state_load,
            read_state::read_state_set,
            read_state::read_state_mark_read,
            read_state::read_state_sync,
            pins::pins_load,
            pins::pin_dm,
            pins::pin_channel,
            saved::saved_list,
            saved::saved_set,
            saved::reminder_set,
            saved::reminder_cancel,
            get_messages,
            open_channel,
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
            set_profile_banner,
            set_share_activity,
            update_channel,
            set_channel_banner,
            set_channel_color,
            set_channel_kind,
            set_channel_sensitive,
            set_voice_status,
            set_channel_access,
            pin_message,
            get_pins,
            send_poll,
            vote_poll,
            close_poll,
            voice_invite::send_voice_invite,
            voice_invite::send_dm_voice_invite,
            voice_invite::voice_channel_presence,
            search_messages,
            search_dms,
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
            tray_prompt_due,
            autostart_allowed,
            finish_tray_prompt,
            get_autostart,
            set_autostart,
            set_auto_update,
            get_update_status,
            check_for_updates,
            restart_to_update,
            set_server_image,
            fetch_server_image,
            set_reaction,
            pick_image,
            pick_avatar_model,
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
            edit_dm,
            delete_dm,
            set_dm_reaction,
            attach_dm_file,
            attach_dm_bytes,
            create_group_dm,
            add_group_members,
            calls::start_dm_call,
            calls::answer_dm_call,
            calls::end_dm_call,
            calls::get_dm_call,
            rename_server,
            create_channel,
            delete_channel,
            kick_member,
            leave_server,
            delete_server,
            transfer_ownership,
            audit_log,
            update_profile,
            start_mic_test,
            stop_mic_test,
            get_attachment_preview,
            get_profile_image,
            get_storage_settings,
            set_history_window,
            get_attachment_video,
            start_voice_recording,
            cancel_voice_recording,
            send_voice_message,
            send_dm_voice_message,
            get_voice_message,
            start_screen_share,
            stop_screen_share,
            share_audio_supported,
            set_stream_volume,
            watch_screen,
            stop_watching,
            get_voice_settings,
            set_noise_cancellation,
            set_echo_cancellation,
            set_muted,
            set_deafened,
        ])
        .build(tauri::generate_context!())
        .expect("failed to run Commy")
        .run(|handle, event| {
            if let tauri::RunEvent::Exit = event {
                liveness::say_goodbye(&handle.state::<App>());
                catchup::save_unsaved(&handle.state::<App>());
            }
            if let tauri::RunEvent::Resumed = event {
                on_resumed(handle);
            }
            #[cfg(mobile)]
            if let tauri::RunEvent::WindowEvent { event: tauri::WindowEvent::Resumed, .. } = &event {
                on_resumed(handle);
            }
            invite_link::on_run_event(handle, &event)
        });
}

#[cfg(test)]
mod hygiene_tests {
    use super::*;

    #[test]
    fn only_web_links_open_and_they_open_as_parsed() {
        assert_eq!(web_url(" https://commy.chat/docs ").as_deref(), Some("https://commy.chat/docs"));
        assert_eq!(web_url("http://localhost:3100/i/abc#key").as_deref(), Some("http://localhost:3100/i/abc#key"));
        assert_eq!(web_url("HTTPS://Example.ORG").as_deref(), Some("https://example.org/"));
        assert_eq!(web_url("file:///etc/passwd"), None);
        assert_eq!(web_url("javascript:alert(1)"), None);
        assert_eq!(web_url("commy://commy.chat/i/abc"), None);
        assert_eq!(web_url("https://"), None);
        assert_eq!(web_url("not a link"), None);
        assert_eq!(web_url(""), None);
    }

    #[test]
    fn images_load_on_their_own_only_up_to_five_megabytes() {
        assert_eq!(preview_limit(false), 5 * 1024 * 1024);
        assert_eq!(preview_limit(true), commy_net::node::MAX_BLOB_BYTES);
        assert!(preview_limit(false) < preview_limit(true));
    }

    #[test]
    fn peer_file_names_are_cleaned_before_the_save_dialog() {
        assert_eq!(safe_file_name("report.pdf"), "report.pdf");
        assert_eq!(safe_file_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_file_name("..\\windows\\system32\\x.dll"), "x.dll");
        assert_eq!(safe_file_name("photos/"), "photos");
        assert_eq!(safe_file_name("/"), "file");
        assert_eq!(safe_file_name(".bashrc"), "bashrc");
        assert_eq!(safe_file_name("  notes.txt. "), "notes.txt");
        assert_eq!(safe_file_name("a\u{0}b\r\nc.txt"), "abc.txt");
        assert_eq!(safe_file_name("CON"), "_CON");
        assert_eq!(safe_file_name("lpt1.txt"), "_lpt1.txt");
        assert_eq!(safe_file_name("com9"), "_com9");
        assert_eq!(safe_file_name("console.log"), "console.log");
        assert_eq!(safe_file_name("what?.png"), "what_.png");
        assert_eq!(safe_file_name(""), "file");
        assert_eq!(safe_file_name("..."), "file");
        let long = format!("{}.jpeg", "x".repeat(500));
        let cleaned = safe_file_name(&long);
        assert_eq!(cleaned.chars().count(), MAX_FILE_NAME_CHARS);
        assert!(cleaned.ends_with(".jpeg"));
        let no_ext = safe_file_name(&"y".repeat(300));
        assert_eq!(no_ext.chars().count(), MAX_FILE_NAME_CHARS);
    }
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
