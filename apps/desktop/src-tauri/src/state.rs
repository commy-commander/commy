use crate::media_engine::{CameraSession, MicTest, ScreenShareSession, ScreenWatch, VoiceSession};
use commy_audio::VoiceProcessor;
use commy_core::identity::IdentityKey;
use commy_core::{action, DmStore, Identity, ServerEvent, ServerId, ServerState, SignedEvent, UserId, UserProfile};
use commy_net::{CommyNode, ServerTicket, WirePayload};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

fn default_gain() -> f32 {
    1.0
}

fn default_app_icon() -> String {
    crate::app_icon::CLASSIC.to_string()
}

fn default_tray_icon() -> String {
    crate::app_icon::FOLLOW_APP.to_string()
}

fn default_afk_minutes() -> u32 {
    crate::afk::DEFAULT_TIMEOUT_MINUTES
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceSettings {
    pub noise_cancellation: bool,
    #[serde(default = "default_true")]
    pub echo_cancellation: bool,
    pub muted: bool,
    pub deafened: bool,
    #[serde(default = "default_gain")]
    pub output_volume: f32,
    #[serde(default = "default_gain")]
    pub input_gain: f32,
    #[serde(default = "default_gain")]
    pub stream_volume: f32,
    #[serde(default = "default_afk_minutes")]
    pub afk_timeout_minutes: u32,
    #[serde(default = "default_true")]
    pub share_activity: bool,
    #[serde(default)]
    pub input_device: Option<String>,
    #[serde(default)]
    pub output_device: Option<String>,
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
    #[serde(default = "default_app_icon")]
    pub app_icon: String,
    #[serde(default = "default_tray_icon")]
    pub tray_icon: String,
    #[serde(default)]
    pub pronouns: Option<String>,
    #[serde(default)]
    pub presence_until: Option<i64>,
    #[serde(default)]
    pub status_until: Option<i64>,
    #[serde(default)]
    pub overlay: crate::overlay::OverlaySettings,
    #[serde(default)]
    pub user_volumes: std::collections::HashMap<String, f32>,
    #[serde(default)]
    pub muted_users: Vec<String>,
    #[serde(default = "default_true")]
    pub auto_update: bool,
    #[serde(default)]
    pub input_mode: crate::voice_keys::InputMode,
    #[serde(default = "crate::voice_keys::default_talk_key")]
    pub talk_key: String,
    #[serde(default = "crate::voice_keys::default_talk_release_ms")]
    pub talk_release_ms: u32,
    #[serde(default)]
    pub mute_hotkey: String,
    #[serde(default)]
    pub deafen_hotkey: String,
    #[serde(default = "default_true")]
    pub auto_sensitivity: bool,
    #[serde(default = "crate::voice_keys::default_sensitivity_db")]
    pub sensitivity_db: f32,
}

impl VoiceSettings {
    pub fn voice_keys(&self) -> crate::voice_keys::VoiceKeys {
        crate::voice_keys::VoiceKeys::from_settings(self.input_mode, &self.talk_key, &self.mute_hotkey, &self.deafen_hotkey)
    }

    pub fn sensitivity(&self) -> commy_audio::Sensitivity {
        crate::voice_keys::sensitivity(self.auto_sensitivity, self.sensitivity_db)
    }

    pub fn stream_gain(&self, forced_deaf: bool) -> f32 {
        if self.deafened || forced_deaf {
            0.0
        } else {
            self.stream_volume
        }
    }

    pub fn apply_to(&self, processor: &mut VoiceProcessor) {
        processor.set_noise_cancellation(self.noise_cancellation);
        processor.set_echo_cancellation(self.echo_cancellation);
        processor.set_muted(self.muted);
        processor.set_input_gain(self.input_gain);
        processor.set_push_to_talk(self.input_mode == crate::voice_keys::InputMode::PushToTalk);
        processor.set_sensitivity(self.sensitivity());
    }
}

pub(crate) fn default_true() -> bool {
    true
}

pub const UNSETTLED_FILE: &str = "identity.unsettled";

fn set_own_key(
    state: &mut ServerState,
    me: UserId,
    key: commy_core::IdentityKey,
) -> Option<Option<commy_core::IdentityKey>> {
    match state.profiles.get_mut(&me) {
        Some(profile) if profile.identity != Some(key) => Some(profile.identity.replace(key)),
        _ => None,
    }
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            noise_cancellation: true,
            echo_cancellation: true,
            muted: false,
            deafened: false,
            output_volume: 1.0,
            input_gain: 1.0,
            stream_volume: 1.0,
            afk_timeout_minutes: default_afk_minutes(),
            share_activity: true,
            input_device: None,
            output_device: None,
            close_to_tray: true,
            app_icon: default_app_icon(),
            tray_icon: default_tray_icon(),
            pronouns: None,
            presence_until: None,
            status_until: None,
            overlay: crate::overlay::OverlaySettings::default(),
            user_volumes: std::collections::HashMap::new(),
            muted_users: Vec::new(),
            auto_update: true,
            input_mode: crate::voice_keys::InputMode::default(),
            talk_key: crate::voice_keys::default_talk_key(),
            talk_release_ms: crate::voice_keys::default_talk_release_ms(),
            mute_hotkey: String::new(),
            deafen_hotkey: String::new(),
            auto_sensitivity: true,
            sensitivity_db: crate::voice_keys::default_sensitivity_db(),
        }
    }
}

pub struct App {
    pub profile: Mutex<Option<UserProfile>>,
    pub servers: Mutex<HashMap<ServerId, ServerState>>,
    pub invites: Mutex<HashMap<ServerId, String>>,
    pub listings: Mutex<HashMap<ServerId, crate::directory::Listing>>,
    pub voice: Mutex<VoiceSettings>,
    pub processor: std::sync::Arc<Mutex<VoiceProcessor>>,
    pub net: tokio::sync::Mutex<Option<CommyNode>>,
    pub node_hex: Mutex<Option<String>>,
    pub data_dir: OnceLock<PathBuf>,
    pub voice_session: Mutex<Option<VoiceSession>>,
    pub dm_call: Mutex<Option<crate::calls::DmCall>>,
    pub screen_share: Mutex<Option<ScreenShareSession>>,
    pub own_preview: Mutex<Option<ScreenWatch>>,
    pub watches: Mutex<HashMap<UserId, ScreenWatch>>,
    pub camera: Mutex<Option<CameraSession>>,
    pub camera_watches: Mutex<HashMap<UserId, ScreenWatch>>,
    pub dms: Mutex<DmStore>,
    pub safety: Mutex<crate::safety::SafetySettings>,
    pub storage: Mutex<crate::storage::StorageSettings>,
    pub dm_filter: Mutex<crate::safety::DmFilter>,
    pub mic_test: Mutex<Option<MicTest>>,
    pub voice_recorder: Mutex<Option<crate::voice_message::VoiceRecorder>>,
    pub last_user_activity: std::sync::atomic::AtomicI64,
    pub auto_idle: std::sync::atomic::AtomicBool,
    pub overlay_visible: std::sync::atomic::AtomicBool,
    pub game_in_front: std::sync::atomic::AtomicBool,
    pub overlay_preview: std::sync::atomic::AtomicBool,
    pub overlay_hotkeys_taken: Mutex<Vec<String>>,
    pub talk_press: std::sync::atomic::AtomicU64,
    pub forced_mute: std::sync::atomic::AtomicBool,
    pub forced_deaf: std::sync::atomic::AtomicBool,
    pub audience: std::sync::atomic::AtomicBool,
    pub share_seen: Mutex<HashMap<(ServerId, UserId), std::time::Instant>>,
    pub last_game: Mutex<Option<(u32, String)>>,
    pub account: Mutex<crate::account::AccountState>,
    pub sign_in: Mutex<Option<crate::account::PendingSignIn>>,
    pub igdb_creds: Mutex<crate::igdb::IgdbCredentials>,
    pub stream_deck: Mutex<crate::stream_deck::StreamDeck>,
    pub igdb: tokio::sync::Mutex<crate::igdb::IgdbClient>,
    pub http: reqwest::Client,
    pub identity: Mutex<Option<Identity>>,
    pub outbox: Mutex<crate::outbox::Outbox>,
    pub channel_opens: Mutex<HashMap<ServerId, crate::view::ChannelOpens>>,
    pub dm_backoff: Mutex<crate::outbox::DmBackoff>,
    pub dm_wake: tokio::sync::Notify,
    pub catchup: Mutex<crate::catchup::Catchup>,
    pub doorbell: Mutex<crate::doorbell::Doorbell>,
    pub rest: Mutex<crate::wake::Rest>,
    pub liveness: Mutex<crate::liveness::Liveness>,
    pub awaiting_snapshot: Mutex<HashSet<ServerId>>,
    pub deleted: Mutex<crate::deleted::DeletedServers>,
    pub key_hold: Mutex<Option<Instant>>,
    pub signing: tokio::sync::RwLock<()>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            profile: Mutex::new(None),
            servers: Mutex::new(HashMap::new()),
            invites: Mutex::new(HashMap::new()),
            listings: Mutex::new(HashMap::new()),
            voice: Mutex::new(VoiceSettings::default()),
            processor: std::sync::Arc::new(Mutex::new(VoiceProcessor::new())),
            net: tokio::sync::Mutex::new(None),
            node_hex: Mutex::new(None),
            data_dir: OnceLock::new(),
            voice_session: Mutex::new(None),
            dm_call: Mutex::new(None),
            screen_share: Mutex::new(None),
            own_preview: Mutex::new(None),
            watches: Mutex::new(HashMap::new()),
            camera: Mutex::new(None),
            camera_watches: Mutex::new(HashMap::new()),
            dms: Mutex::new(DmStore::new()),
            safety: Mutex::new(crate::safety::SafetySettings::default()),
            storage: Mutex::new(crate::storage::StorageSettings::default()),
            dm_filter: Mutex::new(crate::safety::DmFilter::default()),
            mic_test: Mutex::new(None),
            voice_recorder: Mutex::new(None),
            last_user_activity: std::sync::atomic::AtomicI64::new(
                chrono::Utc::now().timestamp(),
            ),
            auto_idle: std::sync::atomic::AtomicBool::new(false),
            overlay_visible: std::sync::atomic::AtomicBool::new(true),
            game_in_front: std::sync::atomic::AtomicBool::new(false),
            overlay_preview: std::sync::atomic::AtomicBool::new(false),
            overlay_hotkeys_taken: Mutex::new(Vec::new()),
            talk_press: std::sync::atomic::AtomicU64::new(0),
            forced_mute: std::sync::atomic::AtomicBool::new(false),
            forced_deaf: std::sync::atomic::AtomicBool::new(false),
            audience: std::sync::atomic::AtomicBool::new(false),
            share_seen: Mutex::new(HashMap::new()),
            last_game: Mutex::new(None),
            account: Mutex::new(crate::account::AccountState::default()),
            sign_in: Mutex::new(None),
            igdb_creds: Mutex::new(crate::igdb::IgdbCredentials::default()),
            stream_deck: Mutex::new(crate::stream_deck::StreamDeck::default()),
            igdb: tokio::sync::Mutex::new(crate::igdb::IgdbClient::default()),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("Commy")
                .build()
                .unwrap_or_default(),
            identity: Mutex::new(None),
            outbox: Mutex::new(crate::outbox::Outbox::default()),
            channel_opens: Mutex::new(HashMap::new()),
            dm_backoff: Mutex::new(crate::outbox::DmBackoff::default()),
            dm_wake: tokio::sync::Notify::new(),
            catchup: Mutex::new(crate::catchup::Catchup::default()),
            doorbell: Mutex::new(crate::doorbell::Doorbell::default()),
            rest: Mutex::new(crate::wake::Rest::default()),
            liveness: Mutex::new(crate::liveness::Liveness::default()),
            awaiting_snapshot: Mutex::new(HashSet::new()),
            deleted: Mutex::new(crate::deleted::DeletedServers::default()),
            key_hold: Mutex::new(None),
            signing: tokio::sync::RwLock::new(()),
        }
    }
}

impl App {
    pub fn data_dir(&self) -> Option<&PathBuf> {
        self.data_dir.get()
    }

    pub fn identity(&self) -> Option<Identity> {
        self.identity.lock().unwrap().clone()
    }

    pub fn hold_key(&self, until: Instant) {
        *self.key_hold.lock().unwrap() = Some(until);
    }

    pub fn key_settled(&self) -> bool {
        self.key_hold.lock().unwrap().is_none_or(|until| Instant::now() >= until)
    }

    pub fn settle_pending(&self) -> bool {
        self.key_hold.lock().unwrap().is_some()
    }

    pub fn settle_key(&self) -> bool {
        let held = self.key_hold.lock().unwrap().take().is_some();
        if let Some(dir) = self.data_dir() {
            if let Err(e) = std::fs::remove_file(dir.join(UNSETTLED_FILE)) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!("could not record that our key settled: {e}");
                }
            }
        }
        held
    }

    pub fn mark_key_unsettled(&self) {
        let Some(dir) = self.data_dir() else { return };
        let written = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(dir.join(UNSETTLED_FILE), b""));
        if let Err(e) = written {
            tracing::warn!("could not record that our key is new: {e}");
        }
    }

    pub fn key_marked_unsettled(&self) -> bool {
        self.data_dir().is_some_and(|dir| dir.join(UNSETTLED_FILE).exists())
    }

    pub fn hold_unsettled_key(&self, signed_in: bool, max: std::time::Duration) -> bool {
        if !self.key_marked_unsettled() {
            return false;
        }
        if !signed_in {
            self.settle_key();
            return false;
        }
        self.hold_key(Instant::now() + max);
        true
    }

    fn signer(&self) -> Option<(UserId, Identity)> {
        let me = self.profile.lock().unwrap().as_ref().map(|p| p.id)?;
        Some((me, self.identity()?))
    }

    pub fn sign_event(&self, server: ServerId, event: ServerEvent) -> Option<SignedEvent> {
        let (me, identity) = self.signer()?;
        let at = chrono::Utc::now().timestamp_millis();
        Some(SignedEvent::sign(server, me, at, event, &identity))
    }

    pub fn wire_for(&self, server: ServerId, event: ServerEvent) -> WirePayload {
        if action::self_signed(&event) || (!self.key_settled() && action::may_go_unsigned(&event)) {
            return WirePayload::Event(event);
        }
        let Some((me, identity)) = self.signer() else { return WirePayload::Event(event) };
        let at = chrono::Utc::now().timestamp_millis();
        WirePayload::Signed(SignedEvent::sign(server, me, at, event, &identity))
    }

    pub fn own_key_for_tickets(&self) -> Option<(UserId, IdentityKey)> {
        if !self.key_settled() {
            return None;
        }
        let me = self.profile.lock().unwrap().as_ref().map(|p| p.id)?;
        Some((me, self.identity()?.public()))
    }

    pub fn ticket_owner(&self, server: ServerId) -> Option<(UserId, IdentityKey)> {
        let raw = self.invites.lock().unwrap().get(&server).cloned()?;
        ServerTicket::decode(&raw).ok()?.owner
    }

    pub fn vouched_ticket(&self, server: ServerId) -> Option<ServerTicket> {
        let raw = self.invites.lock().unwrap().get(&server).cloned()?;
        let mut ticket = ServerTicket::decode(&raw).ok()?;
        if ticket.owner.is_none() {
            let (owns, owner) = {
                let servers = self.servers.lock().unwrap();
                let state = servers.get(&server)?;
                let owner = state.server.owner;
                (owner, state.identity_of(owner).map(|key| (owner, key)))
            };
            let owner = owner.or_else(|| self.own_key_for_tickets().filter(|(id, _)| *id == owns));
            if let Some(owner) = owner {
                ticket.owner = Some(owner);
                self.invites.lock().unwrap().insert(server, ticket.encode());
                self.persist_invites();
            }
        }
        Some(ticket)
    }

    pub fn persist_invites(&self) {
        let Some(dir) = self.data_dir() else { return };
        let _ = crate::persist::save_invites(dir, &self.invites.lock().unwrap());
    }

    pub fn persist_listings(&self) {
        let Some(dir) = self.data_dir() else { return };
        warn_unsaved("directory listings", crate::persist::save_listings(dir, &self.listings.lock().unwrap()));
    }

    pub fn announced(&self, profile: UserProfile) -> UserProfile {
        let mut profile = profile.as_seen_by_others();
        if !self.key_settled() {
            profile.identity = None;
        }
        if !self.safety.lock().unwrap().profile_policy.shows_details_to_servers() {
            profile = profile.without_details();
        }
        profile
    }

    pub fn profile_for(&self, me: &UserProfile, partner: UserId) -> UserProfile {
        let shares_server = self.servers.lock().unwrap().values().any(|s| s.server.is_member(partner));
        let shown = self.safety.lock().unwrap().shows_details_to(partner, shares_server);
        let profile = me.as_seen_by_others();
        if shown {
            profile
        } else {
            profile.without_details()
        }
    }

    pub fn awaits_snapshot(&self, server: ServerId) -> bool {
        self.awaiting_snapshot.lock().unwrap().contains(&server)
    }

    pub fn set_awaiting_snapshot(&self, server: ServerId, awaiting: bool) {
        let changed = {
            let mut set = self.awaiting_snapshot.lock().unwrap();
            if awaiting {
                set.insert(server)
            } else {
                set.remove(&server)
            }
        };
        if changed {
            self.persist_awaiting();
        }
    }

    pub fn persist_awaiting(&self) {
        let Some(dir) = self.data_dir() else { return };
        let set = self.awaiting_snapshot.lock().unwrap();
        warn_unsaved("servers awaiting a snapshot", crate::persist::save_awaiting(dir, &set));
    }

    pub fn reload_identity(&self) -> std::io::Result<Option<UserProfile>> {
        let identity = match self.data_dir() {
            Some(dir) => commy_core::identity::load_or_create(dir)?,
            None => self.identity().unwrap_or_else(Identity::generate),
        };
        let key = identity.public();
        *self.identity.lock().unwrap() = Some(identity);
        let (profile, changed) = {
            let mut guard = self.profile.lock().unwrap();
            let Some(profile) = guard.as_mut() else { return Ok(None) };
            let changed = profile.identity != Some(key);
            profile.identity = Some(key);
            (profile.clone(), changed)
        };
        if changed {
            self.persist_profile();
        }
        let updated: Vec<ServerId> = {
            let mut servers = self.servers.lock().unwrap();
            servers
                .values_mut()
                .filter_map(|state| {
                    let before = set_own_key(state, profile.id, key)?;
                    if before.is_some() {
                        tracing::warn!(
                            "server {} had another key pinned for us; members who pinned it reject what we sign",
                            state.server.id
                        );
                    }
                    Some(state.server.id)
                })
                .collect()
        };
        for id in updated {
            self.persist_server(id);
        }
        Ok(Some(profile))
    }

    pub fn persist_server(&self, id: ServerId) {
        let Some(dir) = self.data_dir() else { return };
        let servers = self.servers.lock().unwrap();
        if let Some(state) = servers.get(&id) {
            warn_unsaved("server", crate::persist::save_server(dir, state));
        }
    }

    pub fn persist_profile(&self) {
        let Some(dir) = self.data_dir() else { return };
        if let Some(profile) = self.profile.lock().unwrap().as_ref() {
            warn_unsaved("profile", crate::persist::save_profile(dir, profile));
        }
    }

    pub fn update_voice(&self, edit: impl FnOnce(&mut VoiceSettings)) -> VoiceSettings {
        let mut voice = self.voice.lock().unwrap();
        edit(&mut voice);
        if let Some(dir) = self.data_dir() {
            warn_unsaved("voice settings", crate::persist::save_voice(dir, &voice));
        }
        voice.clone()
    }

    pub fn persist_safety(&self) {
        let Some(dir) = self.data_dir() else { return };
        let json = serde_json::to_vec_pretty(&*self.safety.lock().unwrap()).map_err(anyhow::Error::from);
        warn_unsaved("safety settings", json.and_then(|json| crate::persist::save_safety(dir, &json)));
    }

    pub fn persist_storage(&self) {
        let Some(dir) = self.data_dir() else { return };
        let storage = self.storage.lock().unwrap().clone();
        warn_unsaved("storage settings", crate::persist::save_storage(dir, &storage));
    }

    pub fn persist_deleted(&self) {
        let Some(dir) = self.data_dir() else { return };
        self.deleted.lock().unwrap().save(dir);
    }

    pub fn persist_outbox(&self) {
        let Some(dir) = self.data_dir() else { return };
        self.write_outbox(|outbox| outbox.save(dir));
    }

    fn write_outbox(&self, write: impl FnOnce(&crate::outbox::Outbox) -> anyhow::Result<()>) {
        let outbox = self.outbox.lock().unwrap();
        warn_unsaved("outbox", write(&outbox));
    }

    pub fn persist_dms(&self) {
        let Some(dir) = self.data_dir() else { return };
        let json = serde_json::to_vec_pretty(&*self.dms.lock().unwrap()).map_err(anyhow::Error::from);
        warn_unsaved("dms", json.and_then(|json| crate::persist::save_dms(dir, &json)));
    }
}

fn warn_unsaved(what: &str, saved: anyhow::Result<()>) {
    if let Err(e) = saved {
        tracing::warn!("failed to persist {what}: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos();
        let dir = std::env::temp_dir().join("commy-state-tests").join(format!("{tag}-{nanos:x}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reload_identity_puts_the_key_everywhere_we_sign() {
        let dir = temp_dir("identity");
        let app = App::default();
        app.data_dir.set(dir.clone()).unwrap();
        assert_eq!(app.reload_identity().unwrap(), None);
        let first = app.identity().unwrap().public();

        let profile = UserProfile::new("me");
        let me = profile.id;
        *app.profile.lock().unwrap() = Some(profile.clone());
        let server = commy_core::Server::new("ours", me);
        let sid = server.id;
        let mut state = ServerState::new(server);
        let mut stale = profile.clone();
        stale.identity = Some(Identity::generate().public());
        state.profiles.insert(me, stale);
        app.servers.lock().unwrap().insert(sid, state);

        let reloaded = app.reload_identity().unwrap().unwrap();
        assert_eq!(reloaded.identity, Some(first), "the file is stable across loads");
        assert_eq!(app.servers.lock().unwrap()[&sid].identity_of(me), Some(first));
        assert_eq!(crate::persist::load_profile(&dir).unwrap().identity, Some(first));

        let adopted = Identity::generate();
        std::fs::write(
            dir.join(commy_core::identity::IDENTITY_FILE),
            data_encoding::HEXLOWER.encode(&adopted.to_bytes()),
        )
        .unwrap();
        let swapped = app.reload_identity().unwrap().unwrap().identity;
        assert_eq!(swapped, Some(adopted.public()));
        assert_eq!(app.identity().unwrap().public(), adopted.public());
        assert_eq!(app.servers.lock().unwrap()[&sid].identity_of(me), Some(adopted.public()));
        assert_eq!(crate::persist::load_profile(&dir).unwrap().identity, Some(adopted.public()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn appearing_offline_is_announced_as_offline() {
        let app = App::default();
        let mut me = UserProfile::new("me");
        me.presence = commy_core::Presence::Invisible;
        me.activity = Some("Playing Chess".into());
        let announced = app.announced(me);
        assert_eq!(announced.presence, commy_core::Presence::Offline);
        assert_eq!(announced.activity, None);
        assert!(!serde_json::to_string(&announced).unwrap().contains("invisible"));
    }

    #[test]
    fn a_friends_only_profile_reaches_servers_without_details() {
        let app = App::default();
        let mut me = UserProfile::new("me");
        me.about = Some("hello".into());
        me.banner = Some("blob:abc".into());
        me.activity = Some("Playing Chess".into());
        assert_eq!(app.announced(me.clone()), me);
        app.safety.lock().unwrap().profile_policy = crate::safety::ProfilePolicy::SharedServers;
        assert_eq!(app.announced(me.clone()), me, "everyone on a server shares it with us");
        app.safety.lock().unwrap().profile_policy = crate::safety::ProfilePolicy::Friends;
        let announced = app.announced(me.clone());
        assert_eq!((announced.about, announced.banner, announced.activity), (None, None, None));
        assert_eq!(announced.username, "me");
    }

    #[test]
    fn a_dm_carries_details_only_to_those_allowed() {
        let app = App::default();
        let mut me = UserProfile::new("me");
        me.about = Some("hello".into());
        let friend = UserId::new();
        let member = UserId::new();
        let stranger = UserId::new();
        app.safety.lock().unwrap().friends.insert(friend);
        let mut server = commy_core::Server::new("s", me.id);
        server.members.push(commy_core::Member { user: member, nickname: None, roles: Vec::new(), joined_at: chrono::Utc::now(), timeout: None });
        let sid = server.id;
        app.servers.lock().unwrap().insert(sid, ServerState::new(server));
        let about = |to: UserId| app.profile_for(&me, to).about.is_some();
        assert!(about(friend) && about(member) && about(stranger));
        app.safety.lock().unwrap().profile_policy = crate::safety::ProfilePolicy::SharedServers;
        assert!(about(friend) && about(member) && !about(stranger));
        app.safety.lock().unwrap().profile_policy = crate::safety::ProfilePolicy::Friends;
        assert!(about(friend) && !about(member) && !about(stranger));
    }

    #[test]
    fn a_key_held_back_stays_out_of_announcements() {
        let app = App::default();
        let mut me = UserProfile::new("me");
        me.identity = Some(Identity::generate().public());
        assert_eq!(app.announced(me.clone()), me, "settled unless held");
        app.hold_key(Instant::now() + std::time::Duration::from_secs(120));
        assert!(!app.key_settled());
        assert_eq!(app.announced(me.clone()).identity, None);
        assert!(app.settle_key());
        assert!(!app.settle_key(), "settled once");
        assert_eq!(app.announced(me.clone()), me);
        app.hold_key(Instant::now());
        assert!(app.key_settled());
        assert_eq!(app.announced(me.clone()), me);
    }

    #[test]
    fn a_key_not_settled_is_held_again_after_a_restart() {
        let dir = temp_dir("unsettled");
        let max = std::time::Duration::from_secs(120);
        let first = App::default();
        first.data_dir.set(dir.clone()).unwrap();
        assert!(!first.hold_unsettled_key(true, max), "an old key is not held");
        first.mark_key_unsettled();
        assert!(first.hold_unsettled_key(true, max));
        assert!(!first.key_settled());

        let second = App::default();
        second.data_dir.set(dir.clone()).unwrap();
        assert!(second.hold_unsettled_key(true, max));
        assert!(!second.key_settled());
        assert!(second.settle_key());
        assert!(!dir.join(UNSETTLED_FILE).exists());

        let third = App::default();
        third.data_dir.set(dir.clone()).unwrap();
        assert!(!third.hold_unsettled_key(true, max), "settled for good");
        assert!(third.key_settled());

        third.mark_key_unsettled();
        assert!(!third.hold_unsettled_key(false, max));
        assert!(third.key_settled() && !third.key_marked_unsettled());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn servers_awaiting_a_snapshot_survive_a_restart() {
        let dir = temp_dir("awaiting");
        let app = App::default();
        app.data_dir.set(dir.clone()).unwrap();
        let (a, b) = (ServerId::new(), ServerId::new());
        app.set_awaiting_snapshot(a, true);
        app.set_awaiting_snapshot(b, true);
        app.set_awaiting_snapshot(b, false);
        assert!(app.awaits_snapshot(a) && !app.awaits_snapshot(b));
        assert_eq!(crate::persist::load_awaiting(&dir), HashSet::from([a]));

        let timed = HashMap::from([(a, 0i64), (b, chrono::Utc::now().timestamp_millis())]);
        crate::persist::write_json(&dir.join("awaiting-snapshot.json"), &timed).unwrap();
        assert_eq!(crate::persist::load_awaiting(&dir), HashSet::from([a, b]));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn outbox_saves_never_overlap() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;
        let app = Arc::new(App::default());
        let writing = Arc::new(AtomicUsize::new(0));
        let overlapped = Arc::new(AtomicBool::new(false));
        let writers: Vec<_> = (0..4)
            .map(|_| {
                let (app, writing, overlapped) = (app.clone(), writing.clone(), overlapped.clone());
                std::thread::spawn(move || {
                    for _ in 0..5 {
                        app.write_outbox(|_| {
                            if writing.fetch_add(1, Ordering::SeqCst) > 0 {
                                overlapped.store(true, Ordering::SeqCst);
                            }
                            std::thread::sleep(std::time::Duration::from_millis(2));
                            writing.fetch_sub(1, Ordering::SeqCst);
                            Ok(())
                        });
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        assert!(!overlapped.load(Ordering::SeqCst), "two saves wrote the file at once");
    }

    #[test]
    fn own_key_changes_only_our_entry() {
        let me = UserProfile::new("me");
        let other = UserProfile::new("other");
        let mut state = ServerState::new(commy_core::Server::new("s", me.id));
        state.profiles.insert(me.id, me.clone());
        state.profiles.insert(other.id, other.clone());
        let key = Identity::generate().public();
        assert_eq!(set_own_key(&mut state, me.id, key), Some(None));
        assert_eq!(set_own_key(&mut state, me.id, key), None, "no change the second time");
        assert_eq!(state.identity_of(me.id), Some(key));
        assert_eq!(state.identity_of(other.id), None);
        assert_eq!(set_own_key(&mut state, UserId::new(), key), None, "not a member here");
        let next = Identity::generate().public();
        assert_eq!(set_own_key(&mut state, me.id, next), Some(Some(key)));
        assert_eq!(state.identity_of(me.id), Some(next));
    }
}
