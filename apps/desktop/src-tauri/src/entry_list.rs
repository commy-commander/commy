//! Your server list and identity, shared between your own devices.
//!
//! Each account has an entry key that only its devices hold. The list of
//! servers you are in (each with its join ticket), your profile and your
//! identity are sealed with that key and kept on the account server as one
//! item it cannot read (`/commy/vault`, item "servers"). A device that signs in
//! downloads it, gets the key from one of your other devices after you click
//! Allow there (or from your recovery code), and joins your servers as you.
//!
//! A device that already has servers under its own identity keeps that
//! identity and stays out of the shared list, so nobody shows up twice.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use commy_core::{Identity, ServerId, UserProfile};
use commy_net::sealed::{Purpose, SealKey};
use commy_net::{EntryKeyAsk, EntryKeyReply, EntryKeyRequest, ServerTicket};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::App;

const KEY_FILE: &str = "entry-key.json";
const LOCAL_FILE: &str = "entry-list.json";
/// Written by the identity module when present; carried as opaque bytes.
const IDENTITY_FILE: &str = "identity.key";
const VAULT_ITEM: &str = "servers";

// ---------- What is sealed ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub ticket: String,
    /// When a device of yours joined; a later leave removes the entry.
    pub since: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EntryList {
    /// The account's identity. The first device to share its list sets it.
    #[serde(default)]
    pub profile: Option<UserProfile>,
    /// When that profile was last edited, so the newest edit wins.
    #[serde(default)]
    pub profile_at: Option<DateTime<Utc>>,
    /// The identity secret file, base64, so new devices sign as you too.
    #[serde(default)]
    pub identity: Option<String>,
    #[serde(default)]
    pub servers: BTreeMap<ServerId, Entry>,
    /// Servers you left, and when, so other devices do not add them back.
    #[serde(default)]
    pub left: BTreeMap<ServerId, DateTime<Utc>>,
}

impl EntryList {
    /// Combines two lists. Joins and leaves are kept by time, so a leave
    /// after the latest join wins; the identity already shared wins. A
    /// shared list without a key takes this device's when it is the same
    /// person, so the first device to sync publishes it.
    pub fn merge(shared: &EntryList, local: &EntryList) -> EntryList {
        let mut left = shared.left.clone();
        for (id, at) in &local.left {
            let keep = left.get(id).map_or(true, |existing| at > existing);
            if keep {
                left.insert(*id, *at);
            }
        }
        let mut servers = shared.servers.clone();
        for (id, entry) in &local.servers {
            let newer = servers.get(id).map_or(true, |existing| entry.since > existing.since);
            if newer {
                servers.insert(*id, entry.clone());
            }
        }
        servers.retain(|id, entry| left.get(id).map_or(true, |at| entry.since > *at));
        let same_person = matches!((&shared.profile, &local.profile), (Some(a), Some(b)) if a.id == b.id);
        let local_newer = same_person && local.profile_at > shared.profile_at;
        let (profile, profile_at) = if shared.profile.is_none() || local_newer {
            (local.profile.clone(), local.profile_at)
        } else {
            (shared.profile.clone(), shared.profile_at)
        };
        EntryList {
            profile,
            profile_at,
            identity: if shared.profile.is_some() {
                shared.identity.clone().or_else(|| local.identity.clone().filter(|_| same_person))
            } else {
                local.identity.clone()
            },
            servers,
            left,
        }
    }

    fn seal(&self, key: &SealKey) -> String {
        key.seal(Purpose::EntryList, &serde_json::to_vec(self).expect("entry list serialization cannot fail"))
    }

    fn open(key: &SealKey, sealed: &str) -> Result<EntryList, String> {
        let bytes = key
            .open(Purpose::EntryList, sealed)
            .map_err(|_| "Your recovery code or device key does not open this server list".to_string())?;
        serde_json::from_slice(&bytes).map_err(|_| "The server list is damaged".to_string())
    }
}

// ---------- Files on this device ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredKey {
    server: String,
    account: String,
    key: String,
}

/// This device's own record of when it joined and left servers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LocalRecord {
    #[serde(default)]
    joined: BTreeMap<ServerId, DateTime<Utc>>,
    #[serde(default)]
    left: BTreeMap<ServerId, DateTime<Utc>>,
    /// When the name or avatar was last edited, here or taken from another device.
    #[serde(default)]
    profile_edited: Option<DateTime<Utc>>,
}

fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn write<T: Serialize>(path: &Path, value: &T) {
    let Ok(json) = serde_json::to_string_pretty(value) else { return };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::rename(&tmp, path);
    }
}

fn load_key(dir: &Path, server: &str, account: &str) -> Option<SealKey> {
    let stored: StoredKey = read(&dir.join(KEY_FILE))?;
    if stored.server != server || stored.account != account {
        return None;
    }
    SealKey::from_text(&stored.key).ok()
}

fn save_key(dir: &Path, server: &str, account: &str, key: &SealKey) {
    let stored = StoredKey { server: server.into(), account: account.into(), key: key.to_text() };
    if let Err(e) = crate::persist::write_secret_json(&dir.join(KEY_FILE), &stored) {
        tracing::warn!("could not save the entry key: {e}");
    }
}

/// Call when this device joins or creates a server.
pub fn record_join(app: &App, id: ServerId) {
    let Some(dir) = app.data_dir() else { return };
    let path = dir.join(LOCAL_FILE);
    let mut record: LocalRecord = read(&path).unwrap_or_default();
    record.joined.insert(id, Utc::now());
    record.left.remove(&id);
    write(&path, &record);
}

/// Call when the name or avatar is edited on this device.
pub fn record_profile_edit(app: &App) {
    set_profile_edited(app, Utc::now());
}

fn set_profile_edited(app: &App, at: DateTime<Utc>) {
    let Some(dir) = app.data_dir() else { return };
    let path = dir.join(LOCAL_FILE);
    let mut record: LocalRecord = read(&path).unwrap_or_default();
    record.profile_edited = Some(at);
    write(&path, &record);
}

/// Call when this device leaves a server.
pub fn record_leave(app: &App, id: ServerId) {
    let Some(dir) = app.data_dir() else { return };
    let path = dir.join(LOCAL_FILE);
    let mut record: LocalRecord = read(&path).unwrap_or_default();
    record.joined.remove(&id);
    record.left.insert(id, Utc::now());
    write(&path, &record);
}

fn read_identity(dir: &Path) -> Option<String> {
    use base64::Engine;
    let bytes = std::fs::read(dir.join(IDENTITY_FILE)).ok()?;
    Some(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// The identity a shared value holds: the identity file, base64.
fn decode_identity(encoded: &str) -> Option<Identity> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
    let text = std::str::from_utf8(&bytes).ok()?;
    let secret: [u8; 32] = data_encoding::HEXLOWER_PERMISSIVE.decode(text.trim().as_bytes()).ok()?.try_into().ok()?;
    Some(Identity::from_bytes(&secret))
}

/// Writes the account's identity file, readable by this user only. False
/// when the shared value is not a key, which is then never written.
fn write_identity(dir: &Path, encoded: &str) -> bool {
    use base64::Engine;
    use std::io::Write;
    let bytes = match base64::engine::general_purpose::STANDARD.decode(encoded) {
        Ok(bytes) if decode_identity(encoded).is_some() => bytes,
        _ => {
            tracing::warn!("the shared identity is not a valid key; keeping ours");
            return false;
        }
    };
    let tmp = dir.join(format!("{IDENTITY_FILE}.tmp"));
    let _ = std::fs::remove_file(&tmp);
    let written = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        // Replaces the old file in one step, so a key is always there.
        std::fs::rename(&tmp, dir.join(IDENTITY_FILE))
    })();
    if let Err(e) = written {
        tracing::warn!("could not write the shared identity: {e}");
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    true
}

/// What a sync does about who this device is, from both lists.
#[derive(Debug, PartialEq)]
enum IdentityStep {
    /// Same person with the same key, or no key shared yet: merge as usual.
    Keep,
    /// Same person, but the account signs with another key: switch to it.
    UseShared(String),
    /// A device without servers of its own becomes the account.
    Adopt,
    /// The account has no key to give yet, so this device waits for the
    /// one that has it instead of signing as the account with its own.
    WaitForKey,
    /// This device has servers of its own and keeps its identity.
    OwnIdentity,
}

fn identity_step(local: &EntryList, shared: &EntryList) -> IdentityStep {
    let mine = local.profile.as_ref().map(|p| p.id);
    let theirs = shared.profile.as_ref().map(|p| p.id);
    let (Some(mine), Some(theirs)) = (mine, theirs) else { return IdentityStep::Keep };
    if mine != theirs {
        return if !local.servers.is_empty() || !local.left.is_empty() {
            IdentityStep::OwnIdentity
        } else if shared.identity.as_deref().and_then(decode_identity).is_none() {
            IdentityStep::WaitForKey
        } else {
            IdentityStep::Adopt
        };
    }
    let Some(encoded) = &shared.identity else { return IdentityStep::Keep };
    let Some(account) = decode_identity(encoded) else { return IdentityStep::Keep };
    let ours = local.identity.as_deref().and_then(decode_identity);
    if ours.is_some_and(|ours| ours.public() == account.public()) {
        IdentityStep::Keep
    } else {
        IdentityStep::UseShared(encoded.clone())
    }
}

/// This device's servers and identity as a list.
fn local_list(app: &App, dir: &Path) -> EntryList {
    let path = dir.join(LOCAL_FILE);
    let mut record: LocalRecord = read(&path).unwrap_or_default();
    let invites = app.invites.lock().unwrap().clone();
    let mut servers = BTreeMap::new();
    let mut recorded = false;
    for state in app.servers.lock().unwrap().values() {
        let id = state.server.id;
        let Some(ticket) = invites.get(&id) else { continue };
        let since = *record.joined.entry(id).or_insert_with(|| {
            recorded = true;
            Utc::now()
        });
        servers.insert(id, Entry { name: state.server.name.clone(), ticket: ticket.clone(), since });
    }
    if recorded {
        write(&path, &record);
    }
    let profile = app.profile.lock().unwrap().clone().map(|mut p| {
        // Live, per device state stays out of the shared copy
        p.node = None;
        p.activity = None;
        p
    });
    EntryList { profile, profile_at: record.profile_edited, identity: read_identity(dir), servers, left: record.left }
}

// ---------- Status for the UI ----------

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct EntryStatus {
    /// off, synced, waiting, own_identity, unsupported or error
    pub state: String,
    pub detail: String,
    pub servers: usize,
    pub has_key: bool,
}

static STATUS: Mutex<Option<EntryStatus>> = Mutex::new(None);
static SYNC: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static ASKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn set_status(handle: &AppHandle, state: &str, detail: impl Into<String>, servers: usize, has_key: bool) {
    let status = EntryStatus { state: state.into(), detail: detail.into(), servers, has_key };
    *STATUS.lock().unwrap() = Some(status.clone());
    let _ = handle.emit("entry-list-changed", status);
}

#[tauri::command]
pub fn entry_list_status() -> EntryStatus {
    STATUS.lock().unwrap().clone().unwrap_or_else(|| EntryStatus { state: "off".into(), ..Default::default() })
}

// ---------- Account server ----------

struct Session {
    server: String,
    token: String,
    account: String,
}

fn session(app: &App) -> Option<Session> {
    let state = app.account.lock().unwrap();
    if !crate::account::is_signed_in(&state) {
        return None;
    }
    Some(Session {
        server: state.signed_in_server.clone()?,
        token: state.token.clone()?,
        account: state.user.as_ref()?.id.clone(),
    })
}

#[derive(Debug, Deserialize)]
struct VaultItem {
    data: Option<String>,
    version: u64,
    #[serde(default)]
    conflict: bool,
}

enum Fetched {
    Item(VaultItem),
    /// The account server predates the vault.
    Unsupported,
}

async fn fetch(app: &App, s: &Session) -> Result<Fetched, String> {
    let res = app
        .http
        .get(format!("{}/api/auth/commy/vault", s.server))
        .query(&[("name", VAULT_ITEM)])
        .bearer_auth(&s.token)
        .send()
        .await
        .map_err(|_| "Could not reach your account server".to_string())?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Fetched::Unsupported);
    }
    if !res.status().is_success() {
        return Err(format!("Your account server answered {}", res.status()));
    }
    res.json().await.map(Fetched::Item).map_err(|_| "Your account server sent an invalid reply".to_string())
}

enum Put {
    Done,
    /// Another device wrote first; merge its list and try again.
    Conflict,
}

async fn put(app: &App, s: &Session, data: String, version: u64) -> Result<Put, String> {
    let res = app
        .http
        .post(format!("{}/api/auth/commy/vault/put", s.server))
        .bearer_auth(&s.token)
        .json(&serde_json::json!({ "name": VAULT_ITEM, "data": data, "version": version }))
        .send()
        .await
        .map_err(|_| "Could not reach your account server".to_string())?;
    if res.status() == reqwest::StatusCode::CONFLICT {
        let item: VaultItem = res.json().await.map_err(|_| "Your account server sent an invalid reply".to_string())?;
        return Ok(if item.conflict { Put::Conflict } else { Put::Done });
    }
    if !res.status().is_success() {
        return Err(format!("Your account server answered {}", res.status()));
    }
    Ok(Put::Done)
}

#[derive(Deserialize)]
struct DeviceList {
    devices: Vec<DeviceRow>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceRow {
    node_id: String,
}

async fn account_devices(app: &App, s: &Session) -> Result<Vec<String>, String> {
    let res = app
        .http
        .get(format!("{}/api/auth/commy/devices", s.server))
        .bearer_auth(&s.token)
        .send()
        .await
        .map_err(|_| "Could not reach your account server".to_string())?;
    let list: DeviceList = res.json().await.map_err(|_| "Could not list your devices".to_string())?;
    Ok(list.devices.into_iter().map(|d| d.node_id).collect())
}

// ---------- Sync ----------

/// Brings this device and the shared list in line. Safe to call often;
/// runs one at a time.
pub fn schedule(handle: &AppHandle) {
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = sync(&handle).await {
            tracing::warn!("server list sync: {e}");
        }
    });
}

#[tauri::command]
pub async fn entry_list_sync(handle: AppHandle) -> Result<EntryStatus, String> {
    sync(&handle).await?;
    Ok(entry_list_status())
}

async fn sync(handle: &AppHandle) -> Result<(), String> {
    let _guard = SYNC.lock().await;
    let app = handle.state::<App>();
    let Some(dir) = app.data_dir().cloned() else { return Ok(()) };
    let Some(s) = session(&app) else {
        set_status(handle, "off", "", 0, false);
        crate::settle::settle_key(handle).await;
        return Ok(());
    };
    if app.account.lock().unwrap().features.is_some_and(|f| !f.vault) {
        set_status(handle, "unsupported", "Your account server cannot keep a server list yet.", 0, false);
        crate::settle::settle_key(handle).await;
        return Ok(());
    }
    if app.profile.lock().unwrap().is_none() {
        return Ok(());
    }

    // A key made in this run that lost a race to another new device
    let mut generated = false;
    for _ in 0..4 {
        let item = match fetch(&app, &s).await {
            Ok(Fetched::Item(item)) => item,
            Ok(Fetched::Unsupported) => {
                set_status(handle, "unsupported", "Your account server cannot keep a server list yet.", 0, false);
                crate::settle::settle_key(handle).await;
                return Ok(());
            }
            Err(e) => {
                set_status(handle, "error", e.clone(), 0, load_key(&dir, &s.server, &s.account).is_some());
                return Err(e);
            }
        };
        let key = load_key(&dir, &s.server, &s.account);
        let local = local_list(&app, &dir);

        let (key, shared) = match (key, item.data.as_deref()) {
            // First device on this account: it starts the list
            (None, None) => {
                let key = SealKey::generate();
                save_key(&dir, &s.server, &s.account, &key);
                generated = true;
                (key, EntryList::default())
            }
            (None, Some(_)) => {
                set_status(
                    handle,
                    "waiting",
                    "Approve this device on one of your other devices, or enter your recovery code.",
                    0,
                    false,
                );
                ask_other_devices(handle);
                return Ok(());
            }
            (Some(key), None) => (key, EntryList::default()),
            (Some(key), Some(data)) => match EntryList::open(&key, data) {
                Ok(list) => (key, list),
                Err(e) => {
                    set_status(handle, "error", e.clone(), 0, true);
                    return Err(e);
                }
            },
        };

        let (after, retry) = identity_part(&app, &dir, &local, &shared, &KEY_WAIT).await;
        match after {
            AfterStep::Merge => {}
            AfterStep::Announce => crate::settle::use_settled_key(handle).await,
            AfterStep::Wait => {
                set_status(
                    handle,
                    "waiting",
                    "Your other device has not shared your identity yet. Open Commy there and this device follows.",
                    shared.servers.len(),
                    true,
                );
                if retry {
                    schedule_later(handle, KEY_WAIT_RETRY);
                }
                return Ok(());
            }
            // A device with its own identity and its own servers stays out
            AfterStep::StayOut => {
                set_status(
                    handle,
                    "own_identity",
                    "This device already had servers under its own identity, so it keeps them and does not share your list.",
                    shared.servers.len(),
                    true,
                );
                crate::settle::settle_key(handle).await;
                return Ok(());
            }
        }

        let local = local_list(&app, &dir);
        let merged = EntryList::merge(&shared, &local);
        if merged.profile_at != local.profile_at && !same_shared_fields(merged.profile.as_ref(), local.profile.as_ref()) {
            if take_profile_edits(handle, &app, merged.profile.as_ref()) {
                if let Some(at) = merged.profile_at {
                    set_profile_edited(&app, at);
                }
            }
        }
        join_missing(handle, &app, &merged, &local).await;

        if merged == shared && item.data.is_some() {
            set_status(handle, "synced", "", merged.servers.len(), true);
            crate::settle::settle_key(handle).await;
            return Ok(());
        }
        match put(&app, &s, merged.seal(&key), item.version).await? {
            Put::Done => {
                // Our key is the account's now, or already was.
                set_status(handle, "synced", "", merged.servers.len(), true);
                crate::settle::settle_key(handle).await;
                return Ok(());
            }
            Put::Conflict => {
                // Another device started the list first: its key is the one
                if generated {
                    let _ = std::fs::remove_file(dir.join(KEY_FILE));
                    generated = false;
                }
                continue;
            }
        }
    }
    Err("Your devices kept changing the server list at the same time; it will sync again soon".into())
}

/// How long a device waiting for the account's key waits before it looks again.
const KEY_WAIT_RETRY: std::time::Duration = std::time::Duration::from_secs(120);

/// Allows one retry at a time while waiting for the account's key, however
/// many syncs end up waiting meanwhile.
struct KeyWait(std::sync::atomic::AtomicBool);

impl KeyWait {
    /// True for the first caller; the others find a retry already waiting.
    fn start(&self) -> bool {
        !self.0.swap(true, Ordering::SeqCst)
    }

    /// The retry is running, so a wait after it may start the next one.
    fn done(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

static KEY_WAIT: KeyWait = KeyWait(std::sync::atomic::AtomicBool::new(false));

/// Syncs again after `delay`, ending the key wait's retry.
fn schedule_later(handle: &AppHandle, delay: std::time::Duration) {
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        KEY_WAIT.done();
        schedule(&handle);
    });
}

/// The identity part of a sync: takes the step these lists call for
/// (see [`take_identity_step`]). When that waits for the account's key,
/// true as well if a retry is to be scheduled, which only the first wait
/// gets until that retry runs (see [`KeyWait`]).
async fn identity_part(app: &App, dir: &Path, local: &EntryList, shared: &EntryList, wait: &KeyWait) -> (AfterStep, bool) {
    let after = take_identity_step(app, dir, identity_step(local, shared), shared).await;
    let retry = after == AfterStep::Wait && wait.start();
    (after, retry)
}

/// What sync does after [`take_identity_step`].
#[derive(Debug, PartialEq)]
enum AfterStep {
    /// Merge and share the lists.
    Merge,
    /// Our key was replaced, what we signed is signed with it and it has
    /// settled: announce it, then merge.
    Announce,
    /// Wait for the account's key and look again later, joining nothing.
    Wait,
    /// Keep our own identity and stay out of the shared list.
    StayOut,
}

/// The part of an identity step done on this device: writes the account's
/// key, takes on its profile when adopting it, then loads the key, signs
/// again with it what we signed while ours was held and settles it (see
/// [`crate::settle::settle_loaded_key`]), so this device signs as the
/// account from now on. Sends signing with our key wait meanwhile.
async fn take_identity_step(app: &App, dir: &Path, step: IdentityStep, shared: &EntryList) -> AfterStep {
    let swap = match step {
        IdentityStep::Keep => return AfterStep::Merge,
        IdentityStep::WaitForKey => return AfterStep::Wait,
        IdentityStep::OwnIdentity => return AfterStep::StayOut,
        IdentityStep::UseShared(encoded) => {
            let swap = app.signing.write().await;
            // Still the same person, so merging under our own key does no harm.
            if !write_identity(dir, &encoded) {
                return AfterStep::Merge;
            }
            swap
        }
        IdentityStep::Adopt => {
            let swap = app.signing.write().await;
            // Joining the account's servers as ourselves would leave this
            // device unable to adopt it later, so it waits instead.
            if !adopt_identity(app, dir, shared) {
                return AfterStep::Wait;
            }
            swap
        }
    };
    if let Err(e) = crate::settle::settle_loaded_key(app, &swap) {
        // Not settled: ours stays held back until a later sync loads it.
        tracing::warn!("could not load the account's identity: {e}");
        return AfterStep::Wait;
    }
    AfterStep::Announce
}

/// Takes on the account's identity on a device that has no servers yet:
/// its key and its profile, with this device's node. Only called with a
/// key to take (see [`identity_step`]). False when it could not be written.
fn adopt_identity(app: &App, dir: &Path, shared: &EntryList) -> bool {
    let (Some(profile), Some(identity)) = (shared.profile.clone(), shared.identity.as_deref()) else { return false };
    if !write_identity(dir, identity) {
        return false;
    }
    let node = app.node_hex.lock().unwrap().clone();
    *app.profile.lock().unwrap() = Some(UserProfile { node, ..profile });
    app.persist_profile();
    true
}

fn same_shared_fields(a: Option<&UserProfile>, b: Option<&UserProfile>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            a.id == b.id && a.username == b.username && a.display_name == b.display_name && a.avatar == b.avatar
        }
        (None, None) => true,
        _ => false,
    }
}

/// Your name and avatar as last edited on another device.
fn take_profile_edits(handle: &AppHandle, app: &App, shared: Option<&UserProfile>) -> bool {
    let Some(shared) = shared else { return false };
    let updated = {
        let mut current = app.profile.lock().unwrap();
        let Some(profile) = current.as_mut() else { return false };
        if profile.id != shared.id {
            return false;
        }
        profile.username = shared.username.clone();
        profile.display_name = shared.display_name.clone();
        profile.avatar = shared.avatar.clone();
        profile.clone()
    };
    app.persist_profile();
    let _ = handle.emit("profile-changed", &updated);
    true
}

async fn join_missing(handle: &AppHandle, app: &App, merged: &EntryList, local: &EntryList) {
    let present: Vec<ServerId> = app.servers.lock().unwrap().keys().copied().collect();
    for (id, entry) in &merged.servers {
        if present.contains(id) || local.left.contains_key(id) {
            continue;
        }
        let Ok(ticket) = ServerTicket::decode(&entry.ticket) else { continue };
        match crate::join_ticket(app, handle, ticket).await {
            Ok(_) => record_join(app, *id),
            Err(e) => tracing::warn!("could not join {} from your server list: {e}", entry.name),
        }
    }
}

// ---------- Getting the key from another device ----------

fn device_name() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "Commy desktop".into())
}

/// Asks every other device on the account for the key at once; the first
/// one approved wins.
fn ask_other_devices(handle: &AppHandle) {
    if ASKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        let result = ask(&handle).await;
        ASKING.store(false, Ordering::SeqCst);
        match result {
            Ok(true) => schedule(&handle),
            Ok(false) => set_status(
                &handle,
                "waiting",
                "None of your other devices approved this one. Open Commy on one of them and try again, or enter your recovery code.",
                0,
                false,
            ),
            Err(e) => set_status(&handle, "waiting", e, 0, false),
        }
    });
}

async fn ask(handle: &AppHandle) -> Result<bool, String> {
    let app = handle.state::<App>();
    let Some(dir) = app.data_dir().cloned() else { return Ok(false) };
    let s = session(&app).ok_or("Sign in first")?;
    let me = app.node_hex.lock().unwrap().clone().ok_or("Still connecting")?;
    let endpoint = app.net.lock().await.as_ref().map(|n| n.endpoint()).ok_or("Still connecting")?;
    let others: Vec<String> = account_devices(&app, &s).await?.into_iter().filter(|d| *d != me).collect();
    if others.is_empty() {
        return Err("No other device is linked to your account. Enter your recovery code instead.".into());
    }
    let request = EntryKeyRequest { server: s.server.clone(), account: s.account.clone(), device_name: device_name() };
    let (tx, mut rx) = tokio::sync::mpsc::channel(others.len());
    for node in others {
        let (endpoint, request, tx) = (endpoint.clone(), request.clone(), tx.clone());
        tauri::async_runtime::spawn(async move {
            let reply = commy_net::entry_key::request(endpoint, &node, &request).await;
            let _ = tx.send(reply).await;
        });
    }
    drop(tx);
    while let Some(reply) = rx.recv().await {
        if let Ok(EntryKeyReply::Key(text)) = reply {
            if let Ok(key) = SealKey::from_text(&text) {
                save_key(&dir, &s.server, &s.account, &key);
                return Ok(true);
            }
        }
    }
    Ok(false)
}

static NEXT_ASK: AtomicU64 = AtomicU64::new(1);
static PENDING: Mutex<Option<HashMap<u64, tokio::sync::oneshot::Sender<EntryKeyReply>>>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
struct KeyAskView {
    id: u64,
    device_name: String,
}

/// Answers other devices asking for the key, once the person here allows it.
pub async fn run_entry_key_requests(handle: AppHandle, mut asks: tokio::sync::mpsc::UnboundedReceiver<EntryKeyAsk>) {
    while let Some(ask) = asks.recv().await {
        let handle = handle.clone();
        tauri::async_runtime::spawn(async move {
            let reply = answer(&handle, ask.peer, &ask.request).await;
            match reply {
                Some(view_rx) => {
                    let (id, rx) = view_rx;
                    let reply = rx.await.unwrap_or(EntryKeyReply::Denied);
                    PENDING.lock().unwrap().get_or_insert_with(HashMap::new).remove(&id);
                    let _ = ask.reply.send(reply);
                }
                None => {
                    let _ = ask.reply.send(EntryKeyReply::Unknown);
                }
            }
        });
    }
}

/// Checks the asking device belongs to this account, then shows the prompt.
async fn answer(
    handle: &AppHandle,
    peer: [u8; 32],
    request: &EntryKeyRequest,
) -> Option<(u64, tokio::sync::oneshot::Receiver<EntryKeyReply>)> {
    let app = handle.state::<App>();
    let dir = app.data_dir()?.clone();
    let s = session(&app)?;
    if request.server != s.server || request.account != s.account {
        return None;
    }
    load_key(&dir, &s.server, &s.account)?;
    let peer_hex = data_encoding::HEXLOWER.encode(&peer);
    if !account_devices(&app, &s).await.ok()?.contains(&peer_hex) {
        tracing::warn!("a device outside this account asked for the server list key");
        return None;
    }
    let id = NEXT_ASK.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = tokio::sync::oneshot::channel();
    PENDING.lock().unwrap().get_or_insert_with(HashMap::new).insert(id, tx);
    let name: String = request.device_name.chars().filter(|c| !c.is_control()).take(64).collect();
    if let Some(window) = handle.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    let _ = handle.emit("entry-key-request", KeyAskView { id, device_name: name });
    Some((id, rx))
}

#[tauri::command]
pub fn answer_entry_key_request(app: tauri::State<'_, App>, id: u64, allow: bool) -> Result<(), String> {
    let sender = PENDING.lock().unwrap().get_or_insert_with(HashMap::new).remove(&id).ok_or("That request has expired")?;
    let reply = if allow {
        let dir = app.data_dir().ok_or("No data folder")?;
        let s = session(&app).ok_or("Sign in first")?;
        let key = load_key(dir, &s.server, &s.account).ok_or("This device has no server list key")?;
        EntryKeyReply::Key(key.to_text())
    } else {
        EntryKeyReply::Denied
    };
    let _ = sender.send(reply);
    Ok(())
}

// ---------- Recovery code ----------

/// The key, for keeping somewhere safe in case every device is lost.
#[tauri::command]
pub fn entry_list_recovery_code(app: tauri::State<'_, App>) -> Result<String, String> {
    let dir = app.data_dir().ok_or("No data folder")?;
    let s = session(&app).ok_or("Sign in first")?;
    let key = load_key(dir, &s.server, &s.account).ok_or("This device has no server list key yet")?;
    Ok(key.to_text())
}

#[tauri::command]
pub async fn entry_list_use_recovery_code(handle: AppHandle, code: String) -> Result<EntryStatus, String> {
    let app = handle.state::<App>();
    let dir = app.data_dir().cloned().ok_or("No data folder")?;
    let s = session(&app).ok_or("Sign in first")?;
    let key = SealKey::from_text(code.trim()).map_err(|_| "That is not a recovery code".to_string())?;
    if let Fetched::Item(VaultItem { data: Some(data), .. }) = fetch(&app, &s).await? {
        EntryList::open(&key, &data)?;
    }
    save_key(&dir, &s.server, &s.account, &key);
    sync(&handle).await?;
    Ok(entry_list_status())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn entry(name: &str, since: DateTime<Utc>) -> Entry {
        Entry { name: name.into(), ticket: format!("commy{name}"), since }
    }

    #[test]
    fn merge_keeps_both_sides_and_honours_later_leaves() {
        let t0 = Utc::now();
        let (a, b, c) = (ServerId::new(), ServerId::new(), ServerId::new());
        let shared = EntryList {
            servers: BTreeMap::from([(a, entry("a", t0)), (b, entry("b", t0))]),
            ..Default::default()
        };
        let local = EntryList {
            servers: BTreeMap::from([(c, entry("c", t0))]),
            left: BTreeMap::from([(b, t0 + Duration::seconds(5))]),
            ..Default::default()
        };
        let merged = EntryList::merge(&shared, &local);
        assert!(merged.servers.contains_key(&a));
        assert!(!merged.servers.contains_key(&b), "left after joining");
        assert!(merged.servers.contains_key(&c));

        // Joining again after leaving brings it back
        let rejoined = EntryList { servers: BTreeMap::from([(b, entry("b", t0 + Duration::seconds(9)))]), ..Default::default() };
        assert!(EntryList::merge(&merged, &rejoined).servers.contains_key(&b));
        // An older join does not undo a leave
        let stale = EntryList { servers: BTreeMap::from([(b, entry("b", t0))]), ..Default::default() };
        assert!(!EntryList::merge(&merged, &stale).servers.contains_key(&b));
    }

    #[test]
    fn the_shared_identity_wins() {
        let shared = EntryList { profile: Some(UserProfile::new("first")), identity: Some("aWQ=".into()), ..Default::default() };
        let local = EntryList { profile: Some(UserProfile::new("second")), identity: Some("b3RoZXI=".into()), ..Default::default() };
        let merged = EntryList::merge(&shared, &local);
        assert_eq!(merged.profile.unwrap().username, "first");
        assert_eq!(merged.identity.as_deref(), Some("aWQ="));
        // An empty shared list takes this device's identity
        let merged = EntryList::merge(&EntryList::default(), &local);
        assert_eq!(merged.profile.unwrap().username, "second");
        assert_eq!(merged.identity.as_deref(), Some("b3RoZXI="));
    }

    /// An identity file's contents as a shared list carries them.
    fn shared_identity(identity: &Identity) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(data_encoding::HEXLOWER.encode(&identity.to_bytes()))
    }

    #[test]
    fn the_first_device_to_sync_publishes_the_missing_key() {
        // Lists shared before keys existed name the account but carry none.
        let account = UserProfile::new("account");
        let shared = EntryList { profile: Some(account.clone()), ..Default::default() };
        let first = shared_identity(&Identity::generate());
        let local = EntryList { profile: Some(account.clone()), identity: Some(first.clone()), ..Default::default() };
        assert_eq!(EntryList::merge(&shared, &local).identity, Some(first.clone()));

        // Once it is there, it wins over another device's key.
        let published = EntryList::merge(&shared, &local);
        let second = EntryList { identity: Some(shared_identity(&Identity::generate())), ..local.clone() };
        assert_eq!(EntryList::merge(&published, &second).identity, Some(first));

        // Someone else's key is never published as the account's.
        let stranger = EntryList { profile: Some(UserProfile::new("other")), ..second };
        assert_eq!(EntryList::merge(&shared, &stranger).identity, None);
    }

    #[test]
    fn devices_take_the_account_key_only_when_there_is_one() {
        let account = UserProfile::new("account");
        let key = Identity::generate();
        let ours = Identity::generate();
        let list = |profile: &UserProfile, identity: Option<&Identity>| EntryList {
            profile: Some(profile.clone()),
            identity: identity.map(shared_identity),
            ..Default::default()
        };
        let shared = list(&account, Some(&key));

        // Same person and key, or no key shared yet: nothing to do.
        assert_eq!(identity_step(&list(&account, Some(&key)), &shared), IdentityStep::Keep);
        assert_eq!(identity_step(&list(&account, Some(&ours)), &list(&account, None)), IdentityStep::Keep);
        assert_eq!(identity_step(&list(&account, Some(&ours)), &EntryList::default()), IdentityStep::Keep);
        // Same person, another key: the account's wins.
        assert_eq!(
            identity_step(&list(&account, Some(&ours)), &shared),
            IdentityStep::UseShared(shared_identity(&key))
        );
        assert_eq!(identity_step(&list(&account, None), &shared), IdentityStep::UseShared(shared_identity(&key)));

        // A new device adopts the account only along with its key.
        let device = UserProfile::new("new device");
        assert_eq!(identity_step(&list(&device, Some(&ours)), &shared), IdentityStep::Adopt);
        assert_eq!(identity_step(&list(&device, Some(&ours)), &list(&account, None)), IdentityStep::WaitForKey);
        let damaged = EntryList { identity: Some("bm90IGEga2V5".into()), ..shared.clone() };
        assert_eq!(identity_step(&list(&device, Some(&ours)), &damaged), IdentityStep::WaitForKey);
        assert_eq!(identity_step(&list(&account, Some(&ours)), &damaged), IdentityStep::Keep, "never a damaged key");
        // One with servers of its own keeps its identity.
        let busy = EntryList {
            servers: BTreeMap::from([(ServerId::new(), entry("mine", Utc::now()))]),
            ..list(&device, Some(&ours))
        };
        assert_eq!(identity_step(&busy, &shared), IdentityStep::OwnIdentity);
    }

    #[test]
    fn the_shared_key_is_written_for_this_user_only() {
        let dir = std::env::temp_dir().join(format!("commy-entry-identity-{}-{:x}", std::process::id(), rand_tag()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(IDENTITY_FILE);
        std::fs::write(&path, "old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        let key = Identity::generate();
        assert!(write_identity(&dir, &shared_identity(&key)));
        let loaded = commy_core::identity::load_or_create(&dir).unwrap();
        assert_eq!(loaded.public(), key.public());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        // Something that is not a key is never written.
        assert!(!write_identity(&dir, "bm90IGEga2V5"));
        assert!(!write_identity(&dir, "%%%"));
        assert_eq!(commy_core::identity::load_or_create(&dir).unwrap().public(), key.public());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temp file left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn rand_tag() -> u128 {
        std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos()
    }

    /// A device as it starts: signed in as `profile` with a key of its own
    /// it just made, held back until it settles.
    fn new_device(tag: &str, profile: &UserProfile) -> (App, std::path::PathBuf, Identity) {
        let dir = std::env::temp_dir().join(format!("commy-entry-step-{tag}-{}-{:x}", std::process::id(), rand_tag()));
        std::fs::create_dir_all(&dir).unwrap();
        let app = App::default();
        app.data_dir.set(dir.clone()).unwrap();
        *app.profile.lock().unwrap() = Some(profile.clone());
        *app.node_hex.lock().unwrap() = Some("ab".repeat(32));
        app.mark_key_unsettled();
        app.reload_identity().unwrap();
        assert!(app.hold_unsettled_key(true, std::time::Duration::from_secs(120)));
        let ours = app.identity().unwrap();
        (app, dir, ours)
    }

    fn list_of(profile: &UserProfile, identity: Option<&Identity>) -> EntryList {
        EntryList { profile: Some(profile.clone()), identity: identity.map(shared_identity), ..Default::default() }
    }

    fn file_key(dir: &Path) -> commy_core::IdentityKey {
        commy_core::identity::load_or_create(dir).unwrap().public()
    }

    #[tokio::test]
    async fn the_same_person_switches_to_the_account_key_and_settles() {
        let account = UserProfile::new("account");
        let key = Identity::generate();
        let (app, dir, ours) = new_device("shared", &account);
        let shared = list_of(&account, Some(&key));
        let step = identity_step(&list_of(&account, Some(&ours)), &shared);
        assert_eq!(step, IdentityStep::UseShared(shared_identity(&key)));

        assert_eq!(take_identity_step(&app, &dir, step, &shared).await, AfterStep::Announce);
        assert_eq!(file_key(&dir), key.public());
        assert_eq!(app.identity().unwrap().public(), key.public(), "signs with it now");
        assert_eq!(app.profile.lock().unwrap().as_ref().unwrap().identity, Some(key.public()));
        assert!(app.key_settled() && !app.key_marked_unsettled());
        let _ = std::fs::remove_dir_all(&dir);

        // A value that is not a key changes nothing and settles nothing.
        let (app, dir, ours) = new_device("damaged", &account);
        let step = IdentityStep::UseShared("bm90IGEga2V5".into());
        assert_eq!(take_identity_step(&app, &dir, step, &shared).await, AfterStep::Merge);
        assert_eq!(file_key(&dir), ours.public());
        assert!(!app.key_settled() && app.key_marked_unsettled());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_new_device_becomes_the_account_with_its_key() {
        let account = UserProfile::new("account");
        let key = Identity::generate();
        let device = UserProfile::new("device");
        let (app, dir, ours) = new_device("adopt", &device);
        let shared = list_of(&account, Some(&key));
        let step = identity_step(&list_of(&device, Some(&ours)), &shared);
        assert_eq!(step, IdentityStep::Adopt);

        assert_eq!(take_identity_step(&app, &dir, step, &shared).await, AfterStep::Announce);
        let profile = app.profile.lock().unwrap().clone().unwrap();
        assert_eq!((profile.id, profile.identity), (account.id, Some(key.public())));
        assert_eq!(profile.node, Some("ab".repeat(32)), "with this device's node");
        assert_eq!(file_key(&dir), key.public());
        assert!(app.key_settled() && !app.key_marked_unsettled());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_new_device_without_the_account_key_waits_as_itself() {
        let account = UserProfile::new("account");
        let device = UserProfile::new("device");
        let (app, dir, ours) = new_device("wait", &device);
        let shared = list_of(&account, None);
        let step = identity_step(&list_of(&device, Some(&ours)), &shared);
        assert_eq!(step, IdentityStep::WaitForKey);
        assert_eq!(take_identity_step(&app, &dir, step, &shared).await, AfterStep::Wait);
        // Asked to adopt a list that lost its key meanwhile: it waits too.
        assert_eq!(take_identity_step(&app, &dir, IdentityStep::Adopt, &shared).await, AfterStep::Wait);
        assert_eq!(app.profile.lock().unwrap().as_ref().unwrap().id, device.id);
        assert_eq!(file_key(&dir), ours.public());
        assert!(!app.key_settled() && app.key_marked_unsettled(), "still held back");

        // The other steps write nothing either.
        assert_eq!(take_identity_step(&app, &dir, IdentityStep::Keep, &shared).await, AfterStep::Merge);
        assert_eq!(take_identity_step(&app, &dir, IdentityStep::OwnIdentity, &shared).await, AfterStep::StayOut);
        assert_eq!(file_key(&dir), ours.public());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_retry_waits_for_the_account_key_at_a_time() {
        let wait = KeyWait(std::sync::atomic::AtomicBool::new(false));
        assert!(wait.start());
        assert!(!wait.start(), "a retry is already on its way");
        assert!(!wait.start());
        wait.done();
        assert!(wait.start(), "once it ran, the next wait schedules another");
    }

    #[tokio::test]
    async fn syncs_waiting_for_the_account_key_schedule_one_retry() {
        let account = UserProfile::new("account");
        let device = UserProfile::new("device");
        let (app, dir, ours) = new_device("retry", &device);
        let local = list_of(&device, Some(&ours));
        let shared = list_of(&account, None);
        let wait = KeyWait(std::sync::atomic::AtomicBool::new(false));

        // Syncs that do not wait leave the guard alone.
        assert_eq!(identity_part(&app, &dir, &local, &local, &wait).await, (AfterStep::Merge, false));
        assert_eq!(identity_part(&app, &dir, &local, &shared, &wait).await, (AfterStep::Wait, true));
        // Another sync while that retry is on its way starts none.
        assert_eq!(identity_part(&app, &dir, &local, &shared, &wait).await, (AfterStep::Wait, false));
        assert_eq!(identity_part(&app, &dir, &local, &shared, &wait).await, (AfterStep::Wait, false));
        // The retry ran: the next wait schedules the next one.
        wait.done();
        assert_eq!(identity_part(&app, &dir, &local, &shared, &wait).await, (AfterStep::Wait, true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn switching_to_the_account_key_signs_our_work_again_before_settling() {
        let account = UserProfile::new("account");
        let key = Identity::generate();
        let (app, dir, ours) = new_device("resign", &account);
        let server = commy_core::Server::new("s", account.id);
        let (sid, channel) = (server.id, server.channels[0].id);
        let mut state = commy_core::ServerState::new(server);
        state.profiles.insert(account.id, app.profile.lock().unwrap().clone().unwrap());
        let mut message = commy_core::Message::new(channel, account.id, "written while held");
        message.sign(sid, &ours);
        let create = commy_core::ServerEvent::MessageCreate { message: message.clone() };
        state.apply(create.clone()).unwrap();
        app.servers.lock().unwrap().insert(sid, state);
        app.outbox.lock().unwrap().note_local(sid, &create, false);

        let shared = list_of(&account, Some(&key));
        let step = identity_step(&list_of(&account, Some(&ours)), &shared);
        assert_eq!(take_identity_step(&app, &dir, step, &shared).await, AfterStep::Announce);
        // Settled, and the message waiting for it carries the new key.
        assert!(app.key_settled());
        let state = app.servers.lock().unwrap()[&sid].clone();
        assert!(state.message(message.id).unwrap().verify(sid, &key.public()));
        assert_eq!(state.identity_of(account.id), Some(key.public()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_newest_edit_of_the_same_identity_wins() {
        let t0 = Utc::now();
        let mut older = UserProfile::new("ferris");
        let mut newer = older.clone();
        newer.display_name = Some("Ferris the Crab".into());
        let shared = EntryList { profile: Some(older.clone()), profile_at: Some(t0), ..Default::default() };
        let local = EntryList { profile: Some(newer.clone()), profile_at: Some(t0 + Duration::seconds(1)), ..Default::default() };
        assert_eq!(EntryList::merge(&shared, &local).profile, Some(newer.clone()));
        let stale = EntryList { profile: Some(older.clone()), profile_at: Some(t0 - Duration::seconds(1)), ..Default::default() };
        let shared = EntryList { profile: Some(newer.clone()), profile_at: Some(t0), ..Default::default() };
        assert_eq!(EntryList::merge(&shared, &stale).profile, Some(newer));
        // Another identity never replaces the shared one, however new
        older.id = commy_core::UserId::new();
        let other = EntryList { profile: Some(older), profile_at: Some(t0 + Duration::days(1)), ..Default::default() };
        assert_eq!(EntryList::merge(&shared, &other).profile, shared.profile);
    }

    #[test]
    fn sealed_lists_reveal_nothing_and_need_the_key() {
        let key = SealKey::generate();
        let list = EntryList {
            profile: Some(UserProfile::new("ferris")),
            servers: BTreeMap::from([(ServerId::new(), entry("Pixel Pioneers", Utc::now()))]),
            ..Default::default()
        };
        let sealed = list.seal(&key);
        assert!(!sealed.contains("Pixel") && !sealed.contains("ferris"));
        assert_eq!(EntryList::open(&key, &sealed).unwrap(), list);
        assert!(EntryList::open(&SealKey::generate(), &sealed).is_err());
    }

    #[test]
    fn keys_belong_to_one_account() {
        let dir = std::env::temp_dir().join(format!("commy-entry-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = SealKey::generate();
        save_key(&dir, "https://commy.chat", "acct-1", &key);
        assert_eq!(load_key(&dir, "https://commy.chat", "acct-1"), Some(key));
        assert_eq!(load_key(&dir, "https://commy.chat", "acct-2"), None);
        assert_eq!(load_key(&dir, "https://other.example", "acct-1"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
