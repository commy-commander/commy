use commy_core::server::ChannelKind;
use commy_core::{ChannelId, ServerId, ServerState, UserId};
use tauri::{AppHandle, Emitter, Manager};

use super::protocol::{
    ChannelChoice, Control, DeckStatus, Directory, DirectoryServer, Person, ServerStats, UserVolume, VoiceSpot,
    VolumeTarget,
};
use crate::state::{App, VoiceSettings};
use crate::voice_keys::InputMode;
use crate::CmdResult;

pub fn deck_status(app: &App) -> DeckStatus {
    let voice = app.voice.lock().unwrap().clone();
    let spot = app
        .voice_session
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| VoiceSpot { server: s.server.to_string(), channel: s.channel.to_string() });
    let mut servers: Vec<ServerStats> = app.servers.lock().unwrap().values().map(server_stats).collect();
    servers.sort_by(|a, b| a.id.cmp(&b.id));
    let mut user_volumes: Vec<UserVolume> =
        voice.user_volumes.iter().map(|(user, volume)| UserVolume { user: user.clone(), volume: *volume }).collect();
    user_volumes.sort_by(|a, b| a.user.cmp(&b.user));
    DeckStatus {
        muted: voice.muted,
        deafened: voice.deafened,
        push_to_talk: voice.input_mode == InputMode::PushToTalk,
        voice: spot,
        camera: app.camera.lock().unwrap().is_some(),
        sharing: app.screen_share.lock().unwrap().is_some(),
        output_volume: voice.output_volume,
        input_gain: voice.input_gain,
        input_device: voice.input_device,
        output_device: voice.output_device,
        user_volumes,
        unread: crate::tray::unread(),
        servers,
    }
}

pub fn server_stats(state: &ServerState) -> ServerStats {
    let members = state.member_list();
    ServerStats {
        id: state.server.id.to_string(),
        members: members.len(),
        online: members.iter().filter(|p| p.presence.is_visible_online()).count(),
        in_voice: state.voice_states.len(),
    }
}

pub fn directory(app: &App) -> Directory {
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let wanted = people_to_list(app, me);
    let servers = app.servers.lock().unwrap();
    let mut listed: Vec<DirectoryServer> = servers.values().map(|state| directory_server(state, me)).collect();
    listed.sort_by_key(|s| s.name.to_lowercase());
    let mut people: Vec<Person> = wanted
        .into_iter()
        .filter_map(|user| {
            let state = servers.values().find(|s| s.profiles.contains_key(&user))?;
            Some(Person { id: user.to_string(), name: commy_view::display_name(state, user) })
        })
        .collect();
    people.sort_by_key(|p| p.name.to_lowercase());
    Directory { servers: listed, people, inputs: Vec::new(), outputs: Vec::new() }
}

fn directory_server(state: &ServerState, me: Option<UserId>) -> DirectoryServer {
    let visible = |id: ChannelId| me.is_some_and(|me| state.server.can_view(id, me));
    let channels = |kinds: &[ChannelKind]| {
        state
            .server
            .channels
            .iter()
            .filter(|c| kinds.contains(&c.kind) && visible(c.id))
            .map(|c| ChannelChoice { id: c.id.to_string(), name: c.name.clone() })
            .collect()
    };
    DirectoryServer {
        id: state.server.id.to_string(),
        name: state.server.name.clone(),
        text: channels(&[ChannelKind::Text, ChannelKind::Announcement, ChannelKind::Forum]),
        voice: channels(&[ChannelKind::Voice, ChannelKind::Stage]),
    }
}

fn people_to_list(app: &App, me: Option<UserId>) -> Vec<UserId> {
    let mut people: Vec<UserId> =
        app.voice.lock().unwrap().user_volumes.keys().filter_map(|u| u.parse().ok()).collect();
    let spot = app.voice_session.lock().unwrap().as_ref().map(|s| (s.server, s.channel));
    if let Some((sid, channel)) = spot {
        if let Some(state) = app.servers.lock().unwrap().get(&sid) {
            people.extend(state.voice_states.iter().filter(|(_, c)| **c == channel).map(|(u, _)| *u));
        }
    }
    people.retain(|u| Some(*u) != me);
    people.sort();
    people.dedup();
    people
}

pub async fn run(handle: &AppHandle, control: Control) -> CmdResult<()> {
    let app = handle.state::<App>();
    match control {
        Control::ToggleMute => {
            let muted = !app.voice.lock().unwrap().muted;
            let voice = crate::set_muted(app, handle.clone(), muted).await?;
            voice_changed(handle, &voice);
        }
        Control::ToggleDeafen => {
            let deafened = !app.voice.lock().unwrap().deafened;
            let voice = crate::set_deafened(app, handle.clone(), deafened).await?;
            voice_changed(handle, &voice);
        }
        Control::Talk { down } => {
            if down && app.voice.lock().unwrap().input_mode != InputMode::PushToTalk {
                return Err("turn on push to talk in Commy first".into());
            }
            hold_talk(handle, down);
        }
        Control::HoldMute { down } => hold_mute(handle, down).await?,
        Control::ToggleInputMode => {
            let (mode, talk_key, release_ms) = {
                let voice = app.voice.lock().unwrap();
                let next = if voice.input_mode == InputMode::PushToTalk { "voice_activity" } else { "push_to_talk" };
                (next.to_string(), voice.talk_key.clone(), voice.talk_release_ms)
            };
            let voice = crate::set_input_mode(app, handle.clone(), mode, talk_key, release_ms)?;
            voice_synced(handle, &voice);
        }
        Control::Volume { target, change } => {
            let (output, input) = {
                let voice = app.voice.lock().unwrap();
                (voice.output_volume, voice.input_gain)
            };
            let (output, input) = match target {
                VolumeTarget::Output => (change.apply(output), input),
                VolumeTarget::Input => (output, change.apply(input)),
            };
            let voice = crate::set_volumes(app, output, input);
            voice_synced(handle, &voice);
        }
        Control::UserVolume { user, change } => {
            let current = app.voice.lock().unwrap().user_volumes.get(&user).copied().unwrap_or(1.0);
            let voice = crate::set_user_voice(app, user, Some(change.apply(current)), None)?;
            voice_synced(handle, &voice);
        }
        Control::Device { target, device } => {
            let (mut input, mut output) = {
                let voice = app.voice.lock().unwrap();
                (voice.input_device.clone(), voice.output_device.clone())
            };
            let chosen = Some(device).filter(|d| !d.trim().is_empty());
            match target {
                VolumeTarget::Input => input = chosen,
                VolumeTarget::Output => output = chosen,
            }
            let voice = crate::set_audio_devices(app, input, output).await?;
            voice_synced(handle, &voice);
        }
        Control::ToggleCamera => {
            in_voice(&app)?;
            ask_app(handle, serde_json::json!({ "action": "toggle_camera" }));
        }
        Control::ToggleStream => {
            in_voice(&app)?;
            if app.screen_share.lock().unwrap().is_none() {
                crate::tray::show_main_window(handle);
            }
            ask_app(handle, serde_json::json!({ "action": "toggle_stream" }));
        }
        Control::JoinVoice { server, channel } => {
            find_channel(&app, &server, &channel, &[ChannelKind::Voice, ChannelKind::Stage], "voice")?;
            ask_app(handle, serde_json::json!({ "action": "join_voice", "server": server, "channel": channel }));
        }
        Control::OpenChannel { server, channel } => {
            find_channel(&app, &server, &channel, &[ChannelKind::Text, ChannelKind::Announcement, ChannelKind::Forum], "text")?;
            crate::tray::show_main_window(handle);
            ask_app(handle, serde_json::json!({ "action": "open_channel", "server": server, "channel": channel }));
        }
        Control::OpenApp => crate::tray::show_main_window(handle),
    }
    Ok(())
}

fn voice_changed(handle: &AppHandle, voice: &VoiceSettings) {
    let _ = handle.emit("voice-settings-changed", voice);
}

fn voice_synced(handle: &AppHandle, voice: &VoiceSettings) {
    let _ = handle.emit("voice-settings-synced", voice);
}

fn ask_app(handle: &AppHandle, command: serde_json::Value) {
    let _ = handle.emit("stream-deck-command", command);
}

fn in_voice(app: &App) -> CmdResult<()> {
    app.voice_session.lock().unwrap().as_ref().map(|_| ()).ok_or_else(|| "join a voice channel first".to_string())
}

fn find_channel(app: &App, server: &str, channel: &str, kinds: &[ChannelKind], noun: &str) -> CmdResult<()> {
    let sid: ServerId = server.parse().map_err(|_| "unknown server".to_string())?;
    let cid: ChannelId = channel.parse().map_err(|_| "unknown channel".to_string())?;
    let me = crate::current_profile(app)?.id;
    let servers = app.servers.lock().unwrap();
    let state = servers.get(&sid).ok_or("unknown server")?;
    channel_fits(state, cid, me, kinds, noun)
}

fn channel_fits(state: &ServerState, cid: ChannelId, me: UserId, kinds: &[ChannelKind], noun: &str) -> CmdResult<()> {
    let found = state.server.channels.iter().find(|c| c.id == cid && state.server.can_view(cid, me));
    match found {
        None => Err("that channel is gone".into()),
        Some(c) if !kinds.contains(&c.kind) => Err(format!("that is not a {noun} channel")),
        Some(_) => Ok(()),
    }
}

async fn hold_mute(handle: &AppHandle, down: bool) -> CmdResult<()> {
    let app = handle.state::<App>();
    let unmute_after = {
        let mut deck = app.stream_deck.lock().unwrap();
        if down {
            deck.holding_mute = !app.voice.lock().unwrap().muted;
            if !deck.holding_mute {
                return Ok(());
            }
            false
        } else {
            std::mem::take(&mut deck.holding_mute)
        }
    };
    if !down && !unmute_after {
        return Ok(());
    }
    let voice = crate::set_muted(app, handle.clone(), down).await?;
    voice_changed(handle, &voice);
    Ok(())
}

#[cfg(desktop)]
fn hold_talk(handle: &AppHandle, down: bool) {
    let release_ms = handle.state::<App>().voice.lock().unwrap().talk_release_ms;
    crate::set_talk_key(handle, down, release_ms);
}

#[cfg(mobile)]
fn hold_talk(_handle: &AppHandle, _down: bool) {}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::server::{Channel, Member};
    use commy_core::user::Presence;
    use commy_core::{Server, UserProfile};

    fn friends() -> (ServerState, UserProfile, UserProfile) {
        let mut owner = UserProfile::new("owner");
        owner.presence = Presence::Online;
        let mut friend = UserProfile::new("friend");
        friend.presence = Presence::Offline;
        let mut state = ServerState::new(Server::new("Friends", owner.id));
        state.server.members.push(Member { user: friend.id, nickname: None, roles: vec![], joined_at: chrono::Utc::now(), timeout: None });
        state.profiles.insert(owner.id, owner.clone());
        state.profiles.insert(friend.id, friend.clone());
        (state, owner, friend)
    }

    #[test]
    fn stats_count_members_online_and_in_voice() {
        let (mut state, owner, _) = friends();
        let voice = state.server.channels.iter().find(|c| c.kind == ChannelKind::Voice).unwrap().id;
        state.voice_states.insert(owner.id, voice);
        let stats = server_stats(&state);
        assert_eq!((stats.members, stats.online, stats.in_voice), (2, 1, 1));
        assert_eq!(stats.id, state.server.id.to_string());
    }

    #[test]
    fn the_directory_splits_text_from_voice() {
        let (mut state, owner, _) = friends();
        state.server.channels.push(Channel::new("news", ChannelKind::Announcement, 5));
        state.server.channels.push(Channel::new("Stage", ChannelKind::Stage, 6));
        state.server.channels.push(Channel::new("Talk", ChannelKind::Category, 7));
        let listed = directory_server(&state, Some(owner.id));
        let names = |list: &[ChannelChoice]| list.iter().map(|c| c.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&listed.text), ["general", "news"]);
        assert_eq!(names(&listed.voice), ["General", "AFK", "Stage"]);
        assert!(directory_server(&state, None).text.is_empty());
    }

    #[test]
    fn a_channel_of_the_wrong_kind_says_so() {
        let (state, owner, _) = friends();
        let text = state.server.channels.iter().find(|c| c.kind == ChannelKind::Text).unwrap().id;
        let voice = [ChannelKind::Voice, ChannelKind::Stage];
        assert_eq!(channel_fits(&state, text, owner.id, &voice, "voice").unwrap_err(), "that is not a voice channel");
        assert_eq!(channel_fits(&state, ChannelId::new(), owner.id, &voice, "voice").unwrap_err(), "that channel is gone");
        assert!(channel_fits(&state, text, owner.id, &[ChannelKind::Text], "text").is_ok());
    }

    #[test]
    fn a_fresh_app_reports_a_quiet_status() {
        let app = App::default();
        let status = deck_status(&app);
        assert!(!status.muted && !status.camera && !status.sharing);
        assert_eq!(status.voice, None);
        assert!(status.servers.is_empty());
        assert!(directory(&app).people.is_empty());
    }
}
