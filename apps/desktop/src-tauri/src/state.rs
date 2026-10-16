use crate::media_engine::{CameraSession, MicTest, ScreenShareSession, ScreenWatch, VoiceSession};
use commy_audio::VoiceProcessor;
use commy_core::{DmStore, Identity, ServerId, ServerState, UserId, UserProfile};
use commy_net::CommyNode;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

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

/// Lies next to `identity.key` while that key may still give way to the
/// account's, from before the key is made until it settles. A restart
/// in between holds the key back again.
pub const UNSETTLED_FILE: &str = "identity.unsettled";

/// Puts our current key on our own entry of a server. Returns None when
/// nothing changed, else the key the entry held before, if any.
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
    /// Our identity key; read it with [`App::identity`].
    pub identity: Mutex<Option<Identity>>,
    /// Sends waiting for someone to come online.
    pub outbox: Mutex<crate::outbox::Outbox>,
    pub dm_backoff: Mutex<crate::outbox::DmBackoff>,
    /// Wakes the DM outbox task early, e.g. when a partner comes online.
    pub dm_wake: tokio::sync::Notify,
    /// Catch-up sessions, and events held for an unknown author key.
    pub catchup: Mutex<crate::catchup::Catchup>,
    /// Servers joined from a ticket that still wait for their first
    /// snapshot. Only these take one (see `catchup::apply_snapshot`).
    pub awaiting_snapshot: Mutex<HashSet<ServerId>>,
    /// While set, our key was made in this run and may still give way to
    /// the account's key, so it is not announced before that time.
    pub key_hold: Mutex<Option<Instant>>,
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
            identity: Mutex::new(None),
            outbox: Mutex::new(crate::outbox::Outbox::default()),
            dm_backoff: Mutex::new(crate::outbox::DmBackoff::default()),
            dm_wake: tokio::sync::Notify::new(),
            catchup: Mutex::new(crate::catchup::Catchup::default()),
            awaiting_snapshot: Mutex::new(HashSet::new()),
            key_hold: Mutex::new(None),
        }
    }
}

impl App {
    pub fn data_dir(&self) -> Option<&PathBuf> {
        self.data_dir.get()
    }

    /// The identity key we sign with, once loaded.
    pub fn identity(&self) -> Option<Identity> {
        self.identity.lock().unwrap().clone()
    }

    /// Keeps our key to ourselves until `until`, or until [`App::settle_key`].
    pub fn hold_key(&self, until: Instant) {
        *self.key_hold.lock().unwrap() = Some(until);
    }

    /// True once our key may be announced and pinned by others.
    pub fn key_settled(&self) -> bool {
        self.key_hold.lock().unwrap().is_none_or(|until| Instant::now() >= until)
    }

    /// Our key is final. True when it was held back until now.
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

    /// Records that our key may still be replaced, before it is made.
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

    /// At startup: holds our key back for at most `max` when it has not
    /// settled since it was made, on a device signed in to an account
    /// whose key may replace it. Without an account there is nothing to
    /// wait for, so the key settles at once. True when it is held.
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

    /// Our profile as we announce it: without the key while it may still
    /// be replaced, so nobody pins one we are about to drop.
    pub fn announced(&self, mut profile: UserProfile) -> UserProfile {
        if !self.key_settled() {
            profile.identity = None;
        }
        profile
    }

    pub fn awaits_snapshot(&self, server: ServerId) -> bool {
        self.awaiting_snapshot.lock().unwrap().contains(&server)
    }

    /// Marks a server as waiting for its first snapshot, or not any more.
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
        if let Err(e) = crate::persist::save_awaiting(dir, &set) {
            tracing::warn!("failed to persist servers awaiting a snapshot: {e}");
        }
    }

    /// Re-reads `identity.key` (another module may have replaced it),
    /// swaps the key in and puts it on our profile, here and in our own
    /// entry of every server. Returns the updated profile, or None before
    /// a username exists. Peers learn the key when the profile is announced.
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
        // Locally we are the authority on our own key, so this bypasses
        // the pinning that guards other members' entries.
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

    pub fn persist_outbox(&self) {
        let Some(dir) = self.data_dir() else { return };
        self.write_outbox(|outbox| outbox.save(dir));
    }

    /// Writes the outbox while holding its lock, so saves from several
    /// tasks land one at a time and the last one is the newest state.
    fn write_outbox(&self, write: impl FnOnce(&crate::outbox::Outbox) -> anyhow::Result<()>) {
        let outbox = self.outbox.lock().unwrap();
        if let Err(e) = write(&outbox) {
            tracing::warn!("failed to persist outbox: {e}");
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
        // Before a username exists only the key is loaded.
        assert_eq!(app.reload_identity().unwrap(), None);
        let first = app.identity().unwrap().public();

        let profile = UserProfile::new("me");
        let me = profile.id;
        *app.profile.lock().unwrap() = Some(profile.clone());
        let server = commy_core::Server::new("ours", me);
        let sid = server.id;
        let mut state = ServerState::new(server);
        // A stale key on our own entry is replaced, not refused.
        let mut stale = profile.clone();
        stale.identity = Some(Identity::generate().public());
        state.profiles.insert(me, stale);
        app.servers.lock().unwrap().insert(sid, state);

        let reloaded = app.reload_identity().unwrap().unwrap();
        assert_eq!(reloaded.identity, Some(first), "the file is stable across loads");
        assert_eq!(app.servers.lock().unwrap()[&sid].identity_of(me), Some(first));
        assert_eq!(crate::persist::load_profile(&dir).unwrap().identity, Some(first));

        // Another module rewrites the file: the next reload swaps the key.
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
        // The hold ends by itself when its time is up.
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

        // Quit before it settled: the next run holds it again.
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

        // Signed out, there is no account key to wait for.
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
        // A different key pinned for us is reported, then replaced.
        let next = Identity::generate().public();
        assert_eq!(set_own_key(&mut state, me.id, next), Some(Some(key)));
        assert_eq!(state.identity_of(me.id), Some(next));
    }
}
