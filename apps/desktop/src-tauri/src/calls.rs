use crate::state::App;
use crate::{current_profile, node_hex_for_user, CmdResult};
use commy_core::ids::CALL_SERVER;
use commy_core::{CallSignal, ChannelId, DmAction, DmMessage, UserId, UserProfile};
use crate::media_engine::should_dial;
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

pub const RING_FOR: Duration = Duration::from_secs(45);

#[derive(Debug, Clone)]
pub struct DmCall {
    pub partner: UserId,
    pub channel: ChannelId,
    pub outgoing: bool,
    pub active: bool,
    pub since: Instant,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DmCallView {
    pub state: String,
    pub user: String,
    pub name: String,
    pub outgoing: bool,
    pub seconds: u64,
    pub reason: Option<String>,
}

fn view(app: &App, call: &DmCall, state: &str, reason: Option<&str>) -> DmCallView {
    DmCallView {
        state: state.to_string(),
        user: call.partner.to_string(),
        name: crate::known_name(app, call.partner),
        outgoing: call.outgoing,
        seconds: call.since.elapsed().as_secs(),
        reason: reason.map(str::to_string),
    }
}

fn announce(app: &App, handle: &tauri::AppHandle, call: &DmCall, state: &str, reason: Option<&str>) -> DmCallView {
    let view = view(app, call, state, reason);
    let _ = handle.emit("dm-call", &view);
    view
}

pub fn current(app: &App) -> Option<DmCallView> {
    let call = app.dm_call.lock().unwrap().clone()?;
    Some(view(app, &call, if call.active { "active" } else { "ringing" }, None))
}

fn busy(app: &App) -> bool {
    app.voice_session.lock().unwrap().is_some() || app.dm_call.lock().unwrap().is_some()
}

fn matching(app: &App, partner: UserId, channel: ChannelId) -> Option<DmCall> {
    app.dm_call.lock().unwrap().clone().filter(|c| c.partner == partner && c.channel == channel)
}

async fn send_signal(app: &App, to: UserId, call: ChannelId, signal: CallSignal) -> CmdResult<()> {
    let _signing = app.signing.read().await;
    let me = current_profile(app)?;
    let hex = node_hex_for_user(app, to).ok_or("that person's device is unknown")?;
    let message = DmMessage::acting(me.id, to, DmAction::Call { call, signal }, "");
    crate::send_direct_dm(app, &me, &hex, message, "they are offline right now", None).await
}

pub fn peer_ok(app: &App, user: UserId, node_hex: &str, channel: ChannelId) -> bool {
    let active = app.dm_call.lock().unwrap().as_ref().is_some_and(|c| c.active && c.partner == user && c.channel == channel);
    active && node_hex_for_user(app, user).is_some_and(|hex| hex.eq_ignore_ascii_case(node_hex))
}

fn stop_session(app: &App, channel: ChannelId) {
    let mine = app.voice_session.lock().unwrap().as_ref().is_some_and(|s| s.server == CALL_SERVER && s.channel == channel);
    if mine {
        let testing = crate::pause_mic_test(app);
        *app.voice_session.lock().unwrap() = None;
        crate::resume_mic_test(app, testing);
    }
}

pub async fn end(app: &App, handle: &tauri::AppHandle, notify: bool, reason: &str) {
    let Some(call) = app.dm_call.lock().unwrap().take() else { return };
    stop_session(app, call.channel);
    if notify {
        if let Err(e) = send_signal(app, call.partner, call.channel, CallSignal::HangUp).await {
            tracing::debug!("could not tell {} the call ended: {e}", call.partner);
        }
    }
    announce(app, handle, &call, "ended", Some(reason));
}

pub fn dial_partner(app: &App, handle: &tauri::AppHandle, me: &UserProfile) {
    let Some(call) = app.dm_call.lock().unwrap().clone().filter(|c| c.active) else { return };
    let (Some(ours), Some(theirs)) = (me.node.clone(), node_hex_for_user(app, call.partner)) else { return };
    if should_dial(&ours, &theirs) && crate::claim_voice_dial(app, call.channel, call.partner) {
        crate::dial_voice_peer(handle, call.partner, theirs, call.channel, me.id);
    }
}

async fn connect(app: &tauri::State<'_, App>, handle: &tauri::AppHandle, call: DmCall) -> CmdResult<()> {
    let me = current_profile(app)?;
    let voice = app.voice.lock().unwrap().clone();
    crate::start_voice_session(app, handle, CALL_SERVER, call.channel, &voice, me.id);
    crate::apply_personal_volumes(app);
    let still_on = {
        let mut slot = app.dm_call.lock().unwrap();
        match slot.as_mut() {
            Some(c) if c.channel == call.channel => {
                c.active = true;
                c.since = Instant::now();
                Some(c.clone())
            }
            _ => None,
        }
    };
    let Some(call) = still_on else {
        stop_session(app, call.channel);
        return Err("the call ended".into());
    };
    dial_partner(app, handle, &me);
    announce(app, handle, &call, "active", None);
    Ok(())
}

fn ring_timeout(handle: tauri::AppHandle, channel: ChannelId) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(RING_FOR).await;
        let app = handle.state::<App>();
        let stale = app.dm_call.lock().unwrap().clone().filter(|c| c.channel == channel && !c.active);
        if let Some(call) = stale {
            let reason = if call.outgoing { "unanswered" } else { "missed" };
            end(&app, &handle, call.outgoing, reason).await;
        }
    });
}

pub async fn on_signal(app: &tauri::State<'_, App>, handle: &tauri::AppHandle, from: UserId, channel: ChannelId, signal: CallSignal) {
    match signal {
        CallSignal::Ring => {
            if busy(app) {
                let _ = send_signal(app, from, channel, CallSignal::Decline).await;
                return;
            }
            let call = DmCall { partner: from, channel, outgoing: false, active: false, since: Instant::now() };
            *app.dm_call.lock().unwrap() = Some(call.clone());
            announce(app, handle, &call, "ringing", None);
            ring_timeout(handle.clone(), channel);
        }
        CallSignal::Accept => {
            let Some(call) = matching(app, from, channel).filter(|c| c.outgoing && !c.active) else { return };
            if let Err(e) = connect(app, handle, call).await {
                tracing::warn!("call could not start: {e}");
                end(app, handle, true, "failed").await;
            }
        }
        CallSignal::Decline => {
            if matching(app, from, channel).is_some() {
                end(app, handle, false, "declined").await;
            }
        }
        CallSignal::HangUp => {
            if let Some(call) = matching(app, from, channel) {
                end(app, handle, false, if call.active { "hung_up" } else { "missed" }).await;
            }
        }
    }
}

#[tauri::command]
pub async fn start_dm_call(app: tauri::State<'_, App>, handle: tauri::AppHandle, user_id: String) -> CmdResult<DmCallView> {
    let (_, partner) = crate::dm_partner(&app, &user_id)?;
    if busy(&app) {
        return Err("leave your current call or voice channel first".into());
    }
    let channel = ChannelId::new();
    send_signal(&app, partner, channel, CallSignal::Ring).await?;
    let call = DmCall { partner, channel, outgoing: true, active: false, since: Instant::now() };
    *app.dm_call.lock().unwrap() = Some(call.clone());
    ring_timeout(handle.clone(), channel);
    Ok(announce(&app, &handle, &call, "ringing", None))
}

#[tauri::command]
pub async fn answer_dm_call(app: tauri::State<'_, App>, handle: tauri::AppHandle, accept: bool) -> CmdResult<()> {
    let call = app
        .dm_call
        .lock()
        .unwrap()
        .clone()
        .filter(|c| !c.outgoing && !c.active)
        .ok_or("nobody is calling")?;
    if !accept {
        app.dm_call.lock().unwrap().take();
        let _ = send_signal(&app, call.partner, call.channel, CallSignal::Decline).await;
        announce(&app, &handle, &call, "ended", Some("declined"));
        return Ok(());
    }
    if let Err(e) = send_signal(&app, call.partner, call.channel, CallSignal::Accept).await {
        end(&app, &handle, false, "failed").await;
        return Err(e);
    }
    connect(&app, &handle, call).await
}

#[tauri::command]
pub async fn end_dm_call(app: tauri::State<'_, App>, handle: tauri::AppHandle) -> CmdResult<()> {
    end(&app, &handle, true, "hung_up").await;
    Ok(())
}

#[tauri::command]
pub fn get_dm_call(app: tauri::State<App>) -> Option<DmCallView> {
    current(&app)
}
