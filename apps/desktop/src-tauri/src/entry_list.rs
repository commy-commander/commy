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
use commy_core::{ServerId, UserProfile};
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
    /// after the latest join wins; the identity already shared wins.
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
            identity: if shared.profile.is_some() { shared.identity.clone() } else { local.identity.clone() },
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
    write(&dir.join(KEY_FILE), &StoredKey { server: server.into(), account: account.into(), key: key.to_text() });
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

fn write_identity(dir: &Path, encoded: &str) {
    use base64::Engine;
    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded) {
        let tmp = dir.join(format!("{IDENTITY_FILE}.tmp"));
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::remove_file(dir.join(IDENTITY_FILE));
            let _ = std::fs::rename(&tmp, dir.join(IDENTITY_FILE));
        }
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
        return Ok(());
    };
    if app.account.lock().unwrap().features.is_some_and(|f| !f.vault) {
        set_status(handle, "unsupported", "Your account server cannot keep a server list yet.", 0, false);
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

        // A device with its own identity and its own servers stays out
        let mine = local.profile.as_ref().map(|p| p.id);
        let theirs = shared.profile.as_ref().map(|p| p.id);
        if let (Some(mine), Some(theirs)) = (mine, theirs) {
            if mine != theirs {
                if local.servers.is_empty() && local.left.is_empty() {
                    adopt_identity(handle, &app, &dir, &shared);
                } else {
                    set_status(
                        handle,
                        "own_identity",
                        "This device already had servers under its own identity, so it keeps them and does not share your list.",
                        shared.servers.len(),
                        true,
                    );
                    return Ok(());
                }
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
            return Ok(());
        }
        match put(&app, &s, merged.seal(&key), item.version).await? {
            Put::Done => {
                set_status(handle, "synced", "", merged.servers.len(), true);
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

/// Takes on the account's identity on a device that has no servers yet.
fn adopt_identity(handle: &AppHandle, app: &App, dir: &Path, shared: &EntryList) {
    let Some(profile) = shared.profile.clone() else { return };
    if let Some(identity) = &shared.identity {
        write_identity(dir, identity);
    }
    let node = app.node_hex.lock().unwrap().clone();
    let adopted = UserProfile { node, ..profile };
    *app.profile.lock().unwrap() = Some(adopted.clone());
    app.persist_profile();
    // Sign with the adopted key from now on.
    let adopted = match app.reload_identity() {
        Ok(Some(profile)) => profile,
        Ok(None) => adopted,
        Err(e) => {
            tracing::warn!("could not load the adopted identity: {e}");
            adopted
        }
    };
    let _ = handle.emit("profile-changed", &adopted);
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
