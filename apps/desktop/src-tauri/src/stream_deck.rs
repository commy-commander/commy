mod controls;
mod protocol;
mod server;

use std::sync::Arc;

use commy_core::{Permissions, ServerId, ServerState};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

use crate::expressions::{DEFAULT_SOUNDS, DEFAULT_SOUND_PREFIX};
use crate::state::App;
use crate::CmdResult;
use protocol::{Control, DeckStatus, Directory, Settings, SoundChoice, SoundboardList, SoundboardServer};
use server::{Backend, PairingAsk, SoundUpload};

pub const RELEASE_PORT: u16 = 47615;
pub const TEST_BUILD_PORT: u16 = 47616;
const SETTINGS_FILE: &str = "stream-deck.json";

pub fn port() -> u16 {
    if crate::instance::TEST_BUILD {
        TEST_BUILD_PORT
    } else {
        RELEASE_PORT
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    Off,
    Starting,
    Listening,
    Busy,
}

#[derive(Default)]
pub struct StreamDeck {
    settings: Settings,
    status: Status,
    listener: Option<tauri::async_runtime::JoinHandle<()>>,
    pending: Option<PendingPairing>,
    next_request: u64,
    holding_mute: bool,
}

struct PendingPairing {
    request: u64,
    answer: oneshot::Sender<bool>,
}

#[derive(Clone, Serialize)]
pub struct StreamDeckView {
    enabled: bool,
    status: Status,
    decks: Vec<PairedDeckView>,
}

#[derive(Clone, Serialize)]
struct PairedDeckView {
    id: String,
    name: String,
    paired_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Serialize)]
struct PairingPrompt {
    request: u64,
    name: String,
}

impl StreamDeck {
    fn view(&self) -> StreamDeckView {
        StreamDeckView {
            enabled: self.settings.enabled,
            status: self.status,
            decks: self
                .settings
                .decks
                .iter()
                .map(|d| PairedDeckView { id: d.id.clone(), name: d.name.clone(), paired_at: d.paired_at })
                .collect(),
        }
    }
}

pub fn load(app: &App, handle: &AppHandle) {
    let Some(dir) = app.data_dir() else { return };
    let Some(settings) = crate::persist::read_json::<Settings>(&dir.join(SETTINGS_FILE)) else { return };
    let enabled = settings.enabled;
    app.stream_deck.lock().unwrap().settings = settings;
    if enabled {
        start(handle);
    }
}

fn save(app: &App, settings: &Settings) {
    let Some(dir) = app.data_dir() else { return };
    if let Err(e) = crate::persist::write_secret_json(&dir.join(SETTINGS_FILE), settings) {
        tracing::warn!("could not save stream deck settings: {e}");
    }
}

fn notify_changed(handle: &AppHandle) {
    let view = handle.state::<App>().stream_deck.lock().unwrap().view();
    let _ = handle.emit("stream-deck-changed", view);
}

fn start(handle: &AppHandle) {
    {
        let app = handle.state::<App>();
        let mut deck = app.stream_deck.lock().unwrap();
        if deck.listener.is_some() || deck.status == Status::Starting {
            return;
        }
        deck.status = Status::Starting;
    }
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        let bound = server::bind(port()).await;
        let app = handle.state::<App>();
        {
            let mut deck = app.stream_deck.lock().unwrap();
            if !deck.settings.enabled {
                deck.status = Status::Off;
                return;
            }
            match bound {
                _ if deck.listener.is_some() => deck.status = Status::Listening,
                Ok(listener) => {
                    let backend = Arc::new(AppBackend { handle: handle.clone() });
                    deck.listener = Some(tauri::async_runtime::spawn(server::listen(listener, backend)));
                    deck.status = Status::Listening;
                }
                Err(e) => {
                    tracing::warn!("stream deck control unavailable: {e}");
                    deck.status = Status::Busy;
                }
            }
        }
        notify_changed(&handle);
    });
}

fn stop(deck: &mut StreamDeck) -> Option<u64> {
    if let Some(listener) = deck.listener.take() {
        listener.abort();
    }
    deck.status = Status::Off;
    deck.pending.take().map(|p| p.request)
}

struct AppBackend {
    handle: AppHandle,
}

impl Backend for AppBackend {
    fn is_paired(&self, token: &str) -> bool {
        self.handle.state::<App>().stream_deck.lock().unwrap().settings.deck_for(token).is_some()
    }

    fn ask_to_pair(&self, name: &str) -> Option<PairingAsk> {
        let request = {
            let app = self.handle.state::<App>();
            let mut deck = app.stream_deck.lock().unwrap();
            if deck.pending.is_some() {
                return None;
            }
            deck.next_request += 1;
            let request = deck.next_request;
            let (answer, rx) = oneshot::channel();
            deck.pending = Some(PendingPairing { request, answer });
            PairingAsk { request, answer: rx }
        };
        crate::tray::show_main_window(&self.handle);
        let _ = self.handle.emit("stream-deck-pair", PairingPrompt { request: request.request, name: name.to_string() });
        Some(request)
    }

    fn withdraw_pairing(&self, request: u64) {
        let app = self.handle.state::<App>();
        let mut deck = app.stream_deck.lock().unwrap();
        if deck.pending.as_ref().is_some_and(|p| p.request == request) {
            deck.pending = None;
        }
        drop(deck);
        let _ = self.handle.emit("stream-deck-pair-closed", request);
    }

    fn finish_pairing(&self, name: &str) -> String {
        let app = self.handle.state::<App>();
        let (token, settings) = {
            let mut deck = app.stream_deck.lock().unwrap();
            let token = deck.settings.pair(name, chrono::Utc::now()).token;
            (token, deck.settings.clone())
        };
        save(&app, &settings);
        notify_changed(&self.handle);
        token
    }

    fn soundboards(&self) -> SoundboardList {
        soundboard_list(&self.handle.state::<App>())
    }

    async fn play(&self, server: Option<String>, sound: String) -> Result<(), String> {
        let app = self.handle.state::<App>();
        let sid = match server.filter(|s| !s.is_empty()) {
            Some(id) => id.parse::<ServerId>().map_err(|_| "unknown server".to_string())?,
            None => voice_server(&app).ok_or("join a voice channel first")?,
        };
        crate::play_sound_in_voice(&app, &self.handle, sid, sound).await
    }

    async fn add_sound(&self, upload: SoundUpload) -> Result<SoundChoice, String> {
        let app = self.handle.state::<App>();
        let sid = upload.server.parse::<ServerId>().map_err(|_| "unknown server".to_string())?;
        let name = commy_view::expressions::name(&upload.name)?;
        if has_sound(&app, sid, &name) {
            return Err(format!("{name} is taken, pick another name"));
        }
        crate::upload_sound(&app, &self.handle, sid, &name, &upload.data, None, upload.emoji.as_deref()).await?;
        Ok(SoundChoice { emoji: key_emoji(upload.emoji.as_deref()), label: name.clone(), name })
    }

    fn status(&self) -> DeckStatus {
        controls::deck_status(&self.handle.state::<App>())
    }

    async fn directory(&self) -> Directory {
        let devices = tokio::task::spawn_blocking(|| (commy_media::list_device_names(true), commy_media::list_device_names(false)));
        let (inputs, outputs) = devices.await.unwrap_or_default();
        Directory { inputs, outputs, ..controls::directory(&self.handle.state::<App>()) }
    }

    async fn control(&self, control: Control) -> Result<(), String> {
        controls::run(&self.handle, control).await
    }
}

fn has_sound(app: &App, sid: ServerId, name: &str) -> bool {
    app.servers.lock().unwrap().get(&sid).is_some_and(|state| state.server.soundboard.iter().any(|s| s.name == name))
}

fn voice_server(app: &App) -> Option<ServerId> {
    app.voice_session.lock().unwrap().as_ref().map(|s| s.server)
}

fn default_choices() -> Vec<SoundChoice> {
    DEFAULT_SOUNDS
        .iter()
        .map(|s| SoundChoice { name: format!("{DEFAULT_SOUND_PREFIX}{}", s.id), label: s.label.to_string(), emoji: None })
        .collect()
}

fn server_choices(state: &ServerState, can_play: bool, may_add: bool) -> SoundboardServer {
    SoundboardServer {
        id: state.server.id.to_string(),
        name: state.server.name.clone(),
        can_play,
        can_add: may_add && state.server.soundboard.len() < commy_core::server::MAX_SOUNDS,
        sounds: state
            .server
            .soundboard
            .iter()
            .map(|s| SoundChoice { name: s.name.clone(), label: s.name.clone(), emoji: key_emoji(s.emoji.as_deref()) })
            .collect(),
    }
}

fn key_emoji(emoji: Option<&str>) -> Option<String> {
    emoji.filter(|e| commy_core::server::parse_emoji_token(e).is_none()).map(String::from)
}

fn soundboard_list(app: &App) -> SoundboardList {
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let mut servers: Vec<SoundboardServer> = {
        let servers = app.servers.lock().unwrap();
        servers
            .values()
            .map(|state| {
                let perms = me.map(|me| (me, crate::our_permissions(app, state, me)));
                let can_play = perms.is_some_and(|(_, p)| p.allows(Permissions::USE_SOUNDBOARD));
                let may_add = perms.is_some_and(|(me, p)| commy_view::expressions::may_add_sound(state, p, me));
                server_choices(state, can_play, may_add)
            })
            .collect()
    };
    servers.sort_by_key(|s| s.name.to_lowercase());
    SoundboardList { voice_server: voice_server(app).map(|s| s.to_string()), defaults: default_choices(), servers }
}

#[tauri::command]
pub fn get_stream_deck(app: tauri::State<'_, App>) -> StreamDeckView {
    app.stream_deck.lock().unwrap().view()
}

#[tauri::command]
pub fn set_stream_deck_enabled(app: tauri::State<'_, App>, handle: AppHandle, enabled: bool) -> StreamDeckView {
    let (settings, withdrawn) = {
        let mut deck = app.stream_deck.lock().unwrap();
        deck.settings.enabled = enabled;
        let withdrawn = if enabled { None } else { stop(&mut deck) };
        (deck.settings.clone(), withdrawn)
    };
    save(&app, &settings);
    if let Some(request) = withdrawn {
        let _ = handle.emit("stream-deck-pair-closed", request);
    }
    if enabled {
        start(&handle);
    }
    app.stream_deck.lock().unwrap().view()
}

#[tauri::command]
pub fn forget_stream_deck(app: tauri::State<'_, App>, id: String) -> CmdResult<StreamDeckView> {
    let settings = {
        let mut deck = app.stream_deck.lock().unwrap();
        if !deck.settings.forget(&id) {
            return Err("that Stream Deck is not connected".into());
        }
        deck.settings.clone()
    };
    save(&app, &settings);
    Ok(app.stream_deck.lock().unwrap().view())
}

#[tauri::command]
pub fn answer_stream_deck_pairing(app: tauri::State<'_, App>, request: u64, allow: bool) {
    let mut deck = app.stream_deck.lock().unwrap();
    if deck.pending.as_ref().is_some_and(|p| p.request == request) {
        if let Some(pending) = deck.pending.take() {
            let _ = pending.answer.send(allow);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::server::SoundboardSound;
    use commy_core::{Server, UserProfile};

    #[test]
    fn release_and_test_builds_use_their_own_ports() {
        assert_ne!(RELEASE_PORT, TEST_BUILD_PORT);
        assert!([RELEASE_PORT, TEST_BUILD_PORT].contains(&port()));
    }

    #[test]
    fn defaults_carry_the_prefixed_name_and_label() {
        let defaults = default_choices();
        assert_eq!(defaults.len(), DEFAULT_SOUNDS.len());
        assert_eq!(defaults[0].name, "default:airhorn");
        assert_eq!(defaults[0].label, "Airhorn");
        assert!(defaults.iter().all(|d| commy_view::expressions::is_default_sound(&d.name)));
    }

    #[test]
    fn server_sounds_keep_their_emoji() {
        let owner = UserProfile::new("owner");
        let mut state = ServerState::new(Server::new("Friends", owner.id));
        state.server.soundboard.push(SoundboardSound { name: "boom".into(), blob: "b".into(), volume: 1.0, emoji: Some("💥".into()) });
        let choices = server_choices(&state, true, true);
        assert_eq!(choices.name, "Friends");
        assert!(choices.can_play);
        assert!(choices.can_add);
        assert_eq!(choices.sounds, vec![SoundChoice { name: "boom".into(), label: "boom".into(), emoji: Some("💥".into()) }]);
    }

    #[test]
    fn server_emoji_pictures_stay_off_keys_that_only_show_text() {
        let owner = UserProfile::new("owner");
        let mut state = ServerState::new(Server::new("Friends", owner.id));
        let token = commy_core::server::emoji_token("party", &format!("{}@{}", "a1".repeat(32), "b2".repeat(32)));
        state.server.soundboard.push(SoundboardSound { name: "cheer".into(), blob: "b".into(), volume: 1.0, emoji: Some(token) });
        assert_eq!(server_choices(&state, true, true).sounds[0].emoji, None);
    }

    #[test]
    fn a_full_soundboard_takes_no_uploads() {
        let owner = UserProfile::new("owner");
        let mut state = ServerState::new(Server::new("Friends", owner.id));
        assert!(!server_choices(&state, true, false).can_add);
        for i in 0..commy_core::server::MAX_SOUNDS {
            state.server.soundboard.push(SoundboardSound { name: format!("s{i}"), blob: "b".into(), volume: 1.0, emoji: None });
        }
        assert!(!server_choices(&state, true, true).can_add);
    }

    #[test]
    fn a_fresh_app_lists_only_the_defaults() {
        let app = App::default();
        let list = soundboard_list(&app);
        assert!(list.servers.is_empty());
        assert_eq!(list.voice_server, None);
        assert_eq!(list.defaults.len(), DEFAULT_SOUNDS.len());
    }

    #[test]
    fn turning_off_drops_a_waiting_prompt() {
        let mut deck = StreamDeck::default();
        let (answer, _rx) = oneshot::channel();
        deck.pending = Some(PendingPairing { request: 4, answer });
        deck.status = Status::Listening;
        assert_eq!(stop(&mut deck), Some(4));
        assert_eq!(deck.status, Status::Off);
        assert!(deck.pending.is_none());
    }

    #[test]
    fn the_view_never_shows_tokens() {
        let mut deck = StreamDeck::default();
        let paired = deck.settings.pair("Desk", chrono::Utc::now());
        let json = serde_json::to_string(&deck.view()).unwrap();
        assert!(json.contains("Desk"));
        assert!(!json.contains(&paired.token));
    }
}
