use std::sync::Mutex;

use chrono::Utc;
use commy_core::{ChannelId, MessageId, ServerId, UserId};
use commy_view::nudge::{self, Heard, NudgeLog, HEARING, SENDING};
use tauri::{AppHandle, Emitter};

use crate::state::App;

static SENT: Mutex<NudgeLog> = Mutex::new(NudgeLog::new());
static HEARD: Mutex<NudgeLog> = Mutex::new(NudgeLog::new());

const OFFLINE: &str = "they could not be reached right now; try again in a moment";

struct Target {
    message: MessageId,
    text: String,
}

fn parse<T: std::str::FromStr>(raw: Option<String>) -> Result<Option<T>, String> {
    raw.map(|raw| raw.parse().map_err(|_| "that conversation is not known".to_string())).transpose()
}

fn latest_message(app: &App, me: UserId, person: UserId, server: Option<ServerId>, channel: Option<ChannelId>) -> Option<Target> {
    let now = Utc::now();
    match (server, channel) {
        (Some(server), Some(channel)) => {
            let servers = app.servers.lock().unwrap();
            let state = servers.get(&server)?;
            if !nudge::server_nudge_welcome(state, me, person, Some(channel)) {
                return None;
            }
            nudge::latest_mention(state, me, person, channel, now).map(|m| Target { message: m.id, text: m.content.clone() })
        }
        (None, None) => nudge::latest_dm(&app.dms.lock().unwrap(), me, person, now).map(|m| Target { message: m.id, text: m.content.clone() }),
        _ => None,
    }
}

#[tauri::command]
pub async fn notify_anyway(
    app: tauri::State<'_, App>,
    user_id: String,
    server_id: Option<String>,
    channel_id: Option<String>,
) -> Result<(), String> {
    let person: UserId = user_id.parse().map_err(|_| "that person is not known".to_string())?;
    let server: Option<ServerId> = parse(server_id)?;
    let channel: Option<ChannelId> = parse(channel_id)?;
    let _signing = app.signing.read().await;
    let me = crate::current_profile(&app)?;
    if person == me.id || app.safety.lock().unwrap().is_blocked(person) {
        return Err("you cannot notify that person".into());
    }
    let target = latest_message(&app, me.id, person, server, channel).ok_or("there is no recent message of yours for them to see")?;
    SENT.lock().unwrap().take(person, Utc::now().timestamp(), SENDING).map_err(nudge::wait_line)?;
    let message = nudge::nudge(me.id, person, target.message, server, channel, &target.text);
    let hex = crate::node_hex_for_user(&app, person);
    crate::send_direct_dm(&app, &me, hex.as_deref(), message, OFFLINE, None).await
}

fn names(app: &App, heard: &Heard) -> (Option<String>, Option<String>) {
    let Some(server) = heard.server else { return (None, None) };
    let servers = app.servers.lock().unwrap();
    let Some(state) = servers.get(&server) else { return (None, None) };
    let channel = heard.channel.and_then(|c| state.server.channel(c)).map(|c| c.name.clone());
    (Some(commy_core::server::shown_server_name(&state.server.name)), channel)
}

pub fn heard(app: &App, handle: &AppHandle, heard: Heard, name: String) {
    if HEARD.lock().unwrap().take(heard.from, Utc::now().timestamp(), HEARING).is_err() {
        tracing::info!("let a notify-anyway from {} pass quietly; they sent several just now", heard.from);
        return;
    }
    let (server_name, channel_name) = names(app, &heard);
    let _ = handle.emit(
        "notify-anyway",
        serde_json::json!({
            "from": heard.from.to_string(),
            "author_name": name,
            "server": heard.server.map(|s| s.to_string()),
            "channel": heard.channel.map(|c| c.to_string()),
            "message": heard.target.to_string(),
            "server_name": server_name,
            "channel_name": channel_name,
            "content": heard.text,
        }),
    );
}
