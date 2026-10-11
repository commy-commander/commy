use crate::media_engine::{CameraSession, MicTest, ScreenShareSession, ScreenWatch, VoiceSession};
use commy_audio::VoiceProcessor;
use commy_core::{DmStore, ServerId, ServerState, UserId, UserProfile};
use commy_net::CommyNode;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

fn default_gain() -> f32 {
    1.0
}

fn default_afk_minutes() -> u32 {
    crate::afk::DEFAULT_TIMEOUT_MINUTES
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceSettings {
    pub noise_cancellation: bool,
    pub muted: bool,
    pub deafened: bool,
    /// Master speaker volume, 0.0 to 2.0.
    #[serde(default = "default_gain")]
    pub output_volume: f32,
    /// Microphone gain, 0.0 to 2.0.
    #[serde(default = "default_gain")]
    pub input_gain: f32,
    /// Minutes of inactivity before going AFK; 0 disables it.
    #[serde(default = "default_afk_minutes")]
    pub afk_timeout_minutes: u32,
    /// Share the foreground game or app with peers.
    #[serde(default = "default_true")]
    pub share_activity: bool,
    /// Chosen microphone and speaker by device name; None is the
    /// system default.
    #[serde(default)]
    pub input_device: Option<String>,
    #[serde(default)]
    pub output_device: Option<String>,
    /// Closing the window keeps Commy running in the tray.
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
    /// Unix seconds when a manually picked presence reverts to Online.
    #[serde(default)]
    pub presence_until: Option<i64>,
    /// Unix seconds when the custom status text and emoji clear.
    #[serde(default)]
    pub status_until: Option<i64>,
    #[serde(default)]
    pub overlay: crate::overlay::OverlaySettings,
    /// Personal volume per user id (1.0 is normal), and users we muted for ourselves.
    #[serde(default)]
    pub user_volumes: std::collections::HashMap<String, f32>,
    #[serde(default)]
    pub muted_users: Vec<String>,
}

fn default_true() -> bool {
    true
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            noise_cancellation: true,
            muted: false,
            deafened: false,
            output_volume: 1.0,
            input_gain: 1.0,
            afk_timeout_minutes: default_afk_minutes(),
            share_activity: true,
            input_device: None,
            output_device: None,
            close_to_tray: true,
            presence_until: None,
            status_until: None,
            overlay: crate::overlay::OverlaySettings::default(),
            user_volumes: std::collections::HashMap::new(),
            muted_users: Vec::new(),
        }
    }
}

/// Global app state managed by Tauri. std Mutexes guard sync-only data
/// and are never held across an await; the network node uses a tokio
/// Mutex because broadcasting is async.
pub struct App {
    pub profile: Mutex<Option<UserProfile>>,
    pub servers: Mutex<HashMap<ServerId, ServerState>>,
    pub invites: Mutex<HashMap<ServerId, String>>,
    pub voice: Mutex<VoiceSettings>,
    pub processor: std::sync::Arc<Mutex<VoiceProcessor>>,
    pub net: tokio::sync::Mutex<Option<CommyNode>>,
    /// Our iroh node id (hex), set once the P2P stack is up.
    pub node_hex: Mutex<Option<String>>,
    /// Where state files live; set during Tauri setup.
    pub data_dir: OnceLock<PathBuf>,
    pub voice_session: Mutex<Option<VoiceSession>>,
    pub screen_share: Mutex<Option<ScreenShareSession>>,
    pub watches: Mutex<HashMap<UserId, ScreenWatch>>,
    pub camera: Mutex<Option<CameraSession>>,
    pub camera_watches: Mutex<HashMap<UserId, ScreenWatch>>,
    pub dms: Mutex<DmStore>,
    pub safety: Mutex<crate::safety::SafetySettings>,
    pub dm_filter: Mutex<crate::safety::DmFilter>,
    pub mic_test: Mutex<Option<MicTest>>,
    /// Unix seconds of the last UI interaction, reported by the
    /// frontend. Keeps users who type or click from being moved to AFK.
    pub last_user_activity: std::sync::atomic::AtomicI64,
    /// True while our Idle presence was set automatically by AFK.
    pub auto_idle: std::sync::atomic::AtomicBool,
    /// Overlay shown (when enabled); flipped by its hotkey.
    pub overlay_visible: std::sync::atomic::AtomicBool,
    pub overlay_hotkeys_taken: Mutex<Vec<String>>,
    /// A moderator server-muted or server-deafened us in our voice server.
    pub forced_mute: std::sync::atomic::AtomicBool,
    pub forced_deaf: std::sync::atomic::AtomicBool,
    /// Last time a peer's screen share was announced, to expire dead ones.
    pub share_seen: Mutex<HashMap<(ServerId, UserId), std::time::Instant>>,
    /// The last game seen in the foreground: (process id, title). Kept
    /// after alt-tabbing so Share Screen can offer it first.
    pub last_game: Mutex<Option<(u32, String)>>,
    /// Commy account sign-in, and a device-code sign-in in progress.
    pub account: Mutex<crate::account::AccountState>,
    pub sign_in: Mutex<Option<crate::account::PendingSignIn>>,
    pub igdb_creds: Mutex<crate::igdb::IgdbCredentials>,
    pub igdb: tokio::sync::Mutex<crate::igdb::IgdbClient>,
    pub http: reqwest::Client,
}

impl Default for App {
    fn default() -> Self {
        Self {
            profile: Mutex::new(None),
            servers: Mutex::new(HashMap::new()),
            invites: Mutex::new(HashMap::new()),
            voice: Mutex::new(VoiceSettings::default()),
            processor: std::sync::Arc::new(Mutex::new(VoiceProcessor::new())),
            net: tokio::sync::Mutex::new(None),
            node_hex: Mutex::new(None),
            data_dir: OnceLock::new(),
            voice_session: Mutex::new(None),
            screen_share: Mutex::new(None),
            watches: Mutex::new(HashMap::new()),
            camera: Mutex::new(None),
            camera_watches: Mutex::new(HashMap::new()),
            dms: Mutex::new(DmStore::new()),
            safety: Mutex::new(crate::safety::SafetySettings::default()),
            dm_filter: Mutex::new(crate::safety::DmFilter::default()),
            mic_test: Mutex::new(None),
            last_user_activity: std::sync::atomic::AtomicI64::new(
                chrono::Utc::now().timestamp(),
            ),
            auto_idle: std::sync::atomic::AtomicBool::new(false),
            overlay_visible: std::sync::atomic::AtomicBool::new(true),
            overlay_hotkeys_taken: Mutex::new(Vec::new()),
            forced_mute: std::sync::atomic::AtomicBool::new(false),
            forced_deaf: std::sync::atomic::AtomicBool::new(false),
            share_seen: Mutex::new(HashMap::new()),
            last_game: Mutex::new(None),
            account: Mutex::new(crate::account::AccountState::default()),
            sign_in: Mutex::new(None),
            igdb_creds: Mutex::new(crate::igdb::IgdbCredentials::default()),
            igdb: tokio::sync::Mutex::new(crate::igdb::IgdbClient::default()),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .user_agent("Commy")
                .build()
                .unwrap_or_default(),
        }
    }
}

impl App {
    pub fn data_dir(&self) -> Option<&PathBuf> {
        self.data_dir.get()
    }

    /// Persists one server if a data dir is configured.
    pub fn persist_server(&self, id: ServerId) {
        let Some(dir) = self.data_dir() else { return };
        let servers = self.servers.lock().unwrap();
        if let Some(state) = servers.get(&id) {
            if let Err(e) = crate::persist::save_server(dir, state) {
                tracing::warn!("failed to persist server: {e}");
            }
        }
    }

    pub fn persist_profile(&self) {
        let Some(dir) = self.data_dir() else { return };
        if let Some(profile) = self.profile.lock().unwrap().as_ref() {
            if let Err(e) = crate::persist::save_profile(dir, profile) {
                tracing::warn!("failed to persist profile: {e}");
            }
        }
    }

    pub fn persist_voice(&self) {
        let Some(dir) = self.data_dir() else { return };
        let voice = self.voice.lock().unwrap().clone();
        if let Err(e) = crate::persist::save_voice(dir, &voice) {
            tracing::warn!("failed to persist voice settings: {e}");
        }
    }

    pub fn persist_safety(&self) {
        let Some(dir) = self.data_dir() else { return };
        let safety = self.safety.lock().unwrap().clone();
        if let Err(e) = crate::persist::save_safety(dir, &safety) {
            tracing::warn!("failed to persist safety settings: {e}");
        }
    }

    pub fn persist_dms(&self) {
        let Some(dir) = self.data_dir() else { return };
        let dms = self.dms.lock().unwrap().clone();
        if let Err(e) = crate::persist::save_dms(dir, &dms) {
            tracing::warn!("failed to persist dms: {e}");
        }
    }
}
