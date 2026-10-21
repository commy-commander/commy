use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use commy_core::identity::IDENTITY_FILE;
use commy_core::{ServerId, UserProfile};
use commy_net::entry_list::decode_identity;
use commy_net::sealed::SealKey;
use commy_net::{EntryKeyAsk, EntryKeyReply, EntryKeyRequest, ServerTicket};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::App;

const KEY_FILE: &str = "entry-key.json";
const LOCAL_FILE: &str = "entry-list.json";
const VAULT_ITEM: &str = "servers";
const NO_VAULT: &str = "Your account server cannot keep a server list yet.";
pub(crate) const SIGNED_OUT: &str = "Your account server signed this device out.";

pub use commy_net::entry_list::{Entry, EntryList};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredKey {
    server: String,
    account: String,
    key: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LocalRecord {
    #[serde(default)]
    joined: BTreeMap<ServerId, DateTime<Utc>>,
    #[serde(default)]
    left: BTreeMap<ServerId, DateTime<Utc>>,
    #[serde(default)]
    profile_edited: Option<DateTime<Utc>>,
}

pub(crate) fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_str(&commy_core::at_rest::read_to_string(path).ok()?).ok()
}

pub(crate) fn write<T: Serialize>(path: &Path, value: &T) {
    let Ok(json) = serde_json::to_string_pretty(value) else { return };
    let tmp = path.with_extension("json.tmp");
    if commy_core::at_rest::write(&tmp, json.as_bytes()).is_ok() {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::rename(&tmp, path);
    }
}

pub(crate) fn load_key(dir: &Path, server: &str, account: &str) -> Option<SealKey> {
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

fn update_record(app: &App, edit: impl FnOnce(&mut LocalRecord)) {
    let Some(dir) = app.data_dir() else { return };
    let path = dir.join(LOCAL_FILE);
    let mut record: LocalRecord = read(&path).unwrap_or_default();
    edit(&mut record);
    write(&path, &record);
}

pub fn record_join(app: &App, id: ServerId) {
    update_record(app, |record| {
        record.joined.insert(id, Utc::now());
        record.left.remove(&id);
    });
}

pub fn record_profile_edit(app: &App) {
    set_profile_edited(app, Utc::now());
}

fn set_profile_edited(app: &App, at: DateTime<Utc>) {
    update_record(app, |record| record.profile_edited = Some(at));
}

pub fn record_leave(app: &App, id: ServerId) {
    update_record(app, |record| {
        record.joined.remove(&id);
        record.left.insert(id, Utc::now());
    });
}

fn read_identity(dir: &Path) -> Option<String> {
    use base64::Engine;
    let bytes = commy_core::at_rest::read(&dir.join(IDENTITY_FILE)).ok()?;
    Some(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn write_identity(dir: &Path, encoded: &str) -> bool {
    use base64::Engine;
    let bytes = match base64::engine::general_purpose::STANDARD.decode(encoded) {
        Ok(bytes) if decode_identity(encoded).is_some() => bytes,
        _ => {
            tracing::warn!("the shared identity is not a valid key; keeping ours");
            return false;
        }
    };
    let tmp = dir.join(format!("{IDENTITY_FILE}.tmp"));
    let _ = std::fs::remove_file(&tmp);
    let written =
        commy_core::identity::write_private(&tmp, &bytes).and_then(|_| std::fs::rename(&tmp, dir.join(IDENTITY_FILE)));
    if let Err(e) = written {
        tracing::warn!("could not write the shared identity: {e}");
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    true
}

#[derive(Debug, PartialEq)]
enum IdentityStep {
    Keep,
    UseShared(String),
    Adopt,
    WaitForKey,
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
        p.node = None;
        p.activity = None;
        p
    });
    EntryList { profile, profile_at: record.profile_edited, identity: read_identity(dir), servers, left: record.left }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct EntryStatus {
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

pub(crate) struct Session {
    pub(crate) server: String,
    pub(crate) token: String,
    pub(crate) account: String,
}

fn signed_in(app: &App) -> Result<(PathBuf, Session), String> {
    let dir = app.data_dir().cloned().ok_or("No data folder")?;
    let s = session(app).ok_or("Sign in first")?;
    Ok((dir, s))
}

pub(crate) fn session(app: &App) -> Option<Session> {
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
pub(crate) struct VaultItem {
    pub(crate) data: Option<String>,
    pub(crate) version: u64,
    #[serde(default)]
    pub(crate) conflict: bool,
}

pub(crate) enum Fetched {
    Item(VaultItem),
    Unsupported,
}

fn account_api(app: &App, s: &Session, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
    app.http.request(method, format!("{}/api/auth/commy/{path}", s.server)).bearer_auth(&s.token)
}

async fn reach(request: reqwest::RequestBuilder) -> Result<reqwest::Response, String> {
    request.send().await.map_err(|_| "Could not reach your account server".to_string())
}

fn answered(res: &reqwest::Response) -> Result<(), String> {
    if res.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(SIGNED_OUT.to_string());
    }
    if !res.status().is_success() {
        return Err(format!("Your account server answered {}", res.status()));
    }
    Ok(())
}

async fn vault_item(res: reqwest::Response) -> Result<VaultItem, String> {
    res.json().await.map_err(|_| "Your account server sent an invalid reply".to_string())
}

pub(crate) async fn fetch(app: &App, s: &Session, name: &str) -> Result<Fetched, String> {
    let res = reach(account_api(app, s, reqwest::Method::GET, "vault").query(&[("name", name)])).await?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Fetched::Unsupported);
    }
    answered(&res)?;
    vault_item(res).await.map(Fetched::Item)
}

pub(crate) enum Put {
    Done,
    Conflict,
}

pub(crate) async fn put(app: &App, s: &Session, name: &str, data: String, version: u64) -> Result<Put, String> {
    let body = serde_json::json!({ "name": name, "data": data, "version": version });
    let res = reach(account_api(app, s, reqwest::Method::POST, "vault/put").json(&body)).await?;
    if res.status() == reqwest::StatusCode::CONFLICT {
        let item = vault_item(res).await?;
        return Ok(if item.conflict { Put::Conflict } else { Put::Done });
    }
    answered(&res)?;
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

pub(crate) async fn account_devices(app: &App, s: &Session) -> Result<Vec<String>, String> {
    let res = reach(account_api(app, s, reqwest::Method::GET, "devices")).await?;
    answered(&res)?;
    let list: DeviceList = res.json().await.map_err(|_| "Could not list your devices".to_string())?;
    Ok(list.devices.into_iter().map(|d| d.node_id).collect())
}

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
        return finish(handle, "off", "", 0, false).await;
    };
    if app.account.lock().unwrap().features.is_some_and(|f| !f.vault) {
        return finish(handle, "unsupported", NO_VAULT, 0, false).await;
    }
    if app.profile.lock().unwrap().is_none() {
        return Ok(());
    }

    let mut generated = false;
    for _ in 0..4 {
        let item = match fetch(&app, &s, VAULT_ITEM).await {
            Ok(Fetched::Item(item)) => item,
            Ok(Fetched::Unsupported) => return finish(handle, "unsupported", NO_VAULT, 0, false).await,
            Err(e) if e == SIGNED_OUT => {
                crate::account::verify_saved_session(handle.clone()).await;
                return Err(e);
            }
            Err(e) => {
                set_status(handle, "error", e.clone(), 0, load_key(&dir, &s.server, &s.account).is_some());
                retry_after_failure(handle);
                return Err(e);
            }
        };
        let key = load_key(&dir, &s.server, &s.account);
        let local = local_list(&app, &dir);

        let (key, shared) = match (key, item.data.as_deref()) {
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
            AfterStep::StayOut => {
                return finish(
                    handle,
                    "own_identity",
                    "This device already had servers under its own identity, so it keeps them and does not share your list.",
                    shared.servers.len(),
                    true,
                )
                .await;
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
            return finish(handle, "synced", "", merged.servers.len(), true).await;
        }
        match put(&app, &s, VAULT_ITEM, merged.seal(&key), item.version).await? {
            Put::Done => return finish(handle, "synced", "", merged.servers.len(), true).await,
            Put::Conflict => {
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

async fn finish(handle: &AppHandle, state: &str, detail: &str, servers: usize, has_key: bool) -> Result<(), String> {
    FAILED_TRIES.store(0, Ordering::SeqCst);
    set_status(handle, state, detail, servers, has_key);
    if state == "synced" {
        crate::read_state::schedule(handle);
    }
    crate::settle::settle_key(handle).await;
    Ok(())
}

const KEY_WAIT_RETRY: std::time::Duration = std::time::Duration::from_secs(120);
const FAILURE_RETRY_SECS: [u64; 3] = [15, 60, 300];

static FAILED_TRIES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static FAILURE_RETRY_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn failure_retry_delay(tries: usize) -> std::time::Duration {
    std::time::Duration::from_secs(FAILURE_RETRY_SECS[tries.min(FAILURE_RETRY_SECS.len() - 1)])
}

fn retry_after_failure(handle: &AppHandle) {
    if FAILURE_RETRY_PENDING.swap(true, Ordering::SeqCst) {
        return;
    }
    let delay = failure_retry_delay(FAILED_TRIES.fetch_add(1, Ordering::SeqCst));
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        FAILURE_RETRY_PENDING.store(false, Ordering::SeqCst);
        schedule(&handle);
    });
}

struct KeyWait(std::sync::atomic::AtomicBool);

impl KeyWait {
    fn start(&self) -> bool {
        !self.0.swap(true, Ordering::SeqCst)
    }

    fn done(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

static KEY_WAIT: KeyWait = KeyWait(std::sync::atomic::AtomicBool::new(false));

fn schedule_later(handle: &AppHandle, delay: std::time::Duration) {
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        KEY_WAIT.done();
        schedule(&handle);
    });
}

async fn identity_part(app: &App, dir: &Path, local: &EntryList, shared: &EntryList, wait: &KeyWait) -> (AfterStep, bool) {
    let after = take_identity_step(app, dir, identity_step(local, shared), shared).await;
    let retry = after == AfterStep::Wait && wait.start();
    (after, retry)
}

#[derive(Debug, PartialEq)]
enum AfterStep {
    Merge,
    Announce,
    Wait,
    StayOut,
}

async fn take_identity_step(app: &App, dir: &Path, step: IdentityStep, shared: &EntryList) -> AfterStep {
    let swap = match step {
        IdentityStep::Keep => return AfterStep::Merge,
        IdentityStep::WaitForKey => return AfterStep::Wait,
        IdentityStep::OwnIdentity => return AfterStep::StayOut,
        IdentityStep::UseShared(encoded) => {
            let swap = app.signing.write().await;
            if !write_identity(dir, &encoded) {
                return AfterStep::Merge;
            }
            swap
        }
        IdentityStep::Adopt => {
            let swap = app.signing.write().await;
            if !adopt_identity(app, dir, shared) {
                return AfterStep::Wait;
            }
            swap
        }
    };
    if let Err(e) = crate::settle::settle_loaded_key(app, &swap) {
        tracing::warn!("could not load the account's identity: {e}");
        return AfterStep::Wait;
    }
    AfterStep::Announce
}

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

#[cfg(desktop)]
fn device_name() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "Commy desktop".into())
}

#[cfg(target_os = "android")]
fn device_name() -> String {
    ["ro.product.marketname", "ro.product.model"]
        .iter()
        .filter_map(|prop| std::process::Command::new("getprop").arg(prop).output().ok())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .find(|name| !name.is_empty())
        .unwrap_or_else(|| "Android phone or tablet".into())
}

#[cfg(target_os = "ios")]
fn device_name() -> String {
    "iPhone or iPad".into()
}

fn ask_other_devices(handle: &AppHandle) {
    if ASKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        let result = ask(&handle).await;
        ASKING.store(false, Ordering::SeqCst);
        let detail = match result {
            Ok(Asked::Got) => return schedule(&handle),
            Ok(Asked::NobodyHasKey) => "None of your devices has the key to your server list. Enter your recovery code, or start a new list from this device.".to_string(),
            Ok(Asked::NotApproved) => "None of your other devices approved this one yet. Open Commy on one of them, or enter your recovery code.".to_string(),
            Err(e) => e,
        };
        set_status(&handle, "waiting", detail, 0, false);
        if KEY_WAIT.start() {
            schedule_later(&handle, KEY_WAIT_RETRY);
        }
    });
}

enum Asked {
    Got,
    NotApproved,
    NobodyHasKey,
}

async fn ask(handle: &AppHandle) -> Result<Asked, String> {
    let app = handle.state::<App>();
    let Some(dir) = app.data_dir().cloned() else { return Ok(Asked::NotApproved) };
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
    let mut keyless = true;
    while let Some(reply) = rx.recv().await {
        match reply {
            Ok(EntryKeyReply::Key(text)) => {
                if let Ok(key) = SealKey::from_text(&text) {
                    save_key(&dir, &s.server, &s.account, &key);
                    return Ok(Asked::Got);
                }
            }
            Ok(EntryKeyReply::Unknown) => {}
            _ => keyless = false,
        }
    }
    Ok(if keyless { Asked::NobodyHasKey } else { Asked::NotApproved })
}

static NEXT_ASK: AtomicU64 = AtomicU64::new(1);
static PENDING: Mutex<BTreeMap<u64, tokio::sync::oneshot::Sender<EntryKeyReply>>> = Mutex::new(BTreeMap::new());

#[derive(Debug, Clone, Serialize)]
struct KeyAskView {
    id: u64,
    device_name: String,
}

pub async fn run_entry_key_requests(handle: AppHandle, mut asks: tokio::sync::mpsc::UnboundedReceiver<EntryKeyAsk>) {
    while let Some(ask) = asks.recv().await {
        let handle = handle.clone();
        tauri::async_runtime::spawn(async move {
            let reply = answer(&handle, ask.peer, &ask.request).await;
            match reply {
                Some((id, rx)) => {
                    let reply = rx.await.unwrap_or(EntryKeyReply::Denied);
                    PENDING.lock().unwrap().remove(&id);
                    let _ = ask.reply.send(reply);
                }
                None => {
                    let _ = ask.reply.send(EntryKeyReply::Unknown);
                }
            }
        });
    }
}

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
    PENDING.lock().unwrap().insert(id, tx);
    let name: String = request.device_name.chars().filter(|c| !c.is_control()).take(64).collect();
    crate::tray::show_main_window(handle);
    let _ = handle.emit("entry-key-request", KeyAskView { id, device_name: name });
    Some((id, rx))
}

#[tauri::command]
pub fn answer_entry_key_request(app: tauri::State<'_, App>, id: u64, allow: bool) -> Result<(), String> {
    let sender = PENDING.lock().unwrap().remove(&id).ok_or("That request has expired")?;
    let reply = if allow {
        let (dir, s) = signed_in(&app)?;
        let key = load_key(&dir, &s.server, &s.account).ok_or("This device has no server list key")?;
        EntryKeyReply::Key(key.to_text())
    } else {
        EntryKeyReply::Denied
    };
    let _ = sender.send(reply);
    Ok(())
}

#[tauri::command]
pub fn entry_list_recovery_code(app: tauri::State<'_, App>) -> Result<String, String> {
    let (dir, s) = signed_in(&app)?;
    let key = load_key(&dir, &s.server, &s.account).ok_or("This device has no server list key yet")?;
    Ok(key.to_text())
}

#[tauri::command]
pub async fn entry_list_use_recovery_code(handle: AppHandle, code: String) -> Result<EntryStatus, String> {
    let app = handle.state::<App>();
    let (dir, s) = signed_in(&app)?;
    let key = SealKey::from_text(code.trim()).map_err(|_| "That is not a recovery code".to_string())?;
    if let Fetched::Item(VaultItem { data: Some(data), .. }) = fetch(&app, &s, VAULT_ITEM).await? {
        EntryList::open(&key, &data)?;
    }
    save_key(&dir, &s.server, &s.account, &key);
    sync(&handle).await?;
    Ok(entry_list_status())
}

#[tauri::command]
pub async fn entry_list_start_over(handle: AppHandle) -> Result<EntryStatus, String> {
    {
        let _guard = SYNC.lock().await;
        let app = handle.state::<App>();
        let (dir, s) = signed_in(&app)?;
        if load_key(&dir, &s.server, &s.account).is_some() {
            return Err("This device already has the key to your server list".into());
        }
        let Fetched::Item(item) = fetch(&app, &s, VAULT_ITEM).await? else {
            return Err(NO_VAULT.into());
        };
        let key = SealKey::generate();
        match put(&app, &s, VAULT_ITEM, local_list(&app, &dir).seal(&key), item.version).await? {
            Put::Done => save_key(&dir, &s.server, &s.account, &key),
            Put::Conflict => return Err("Another device changed your server list just now. Try again.".into()),
        }
    }
    sync(&handle).await?;
    Ok(entry_list_status())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use commy_core::Identity;

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

        let rejoined = EntryList { servers: BTreeMap::from([(b, entry("b", t0 + Duration::seconds(9)))]), ..Default::default() };
        assert!(EntryList::merge(&merged, &rejoined).servers.contains_key(&b));
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
        let merged = EntryList::merge(&EntryList::default(), &local);
        assert_eq!(merged.profile.unwrap().username, "second");
        assert_eq!(merged.identity.as_deref(), Some("b3RoZXI="));
    }

    fn shared_identity(identity: &Identity) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(data_encoding::HEXLOWER.encode(&identity.to_bytes()))
    }

    #[test]
    fn the_first_device_to_sync_publishes_the_missing_key() {
        let account = UserProfile::new("account");
        let shared = EntryList { profile: Some(account.clone()), ..Default::default() };
        let first = shared_identity(&Identity::generate());
        let local = EntryList { profile: Some(account.clone()), identity: Some(first.clone()), ..Default::default() };
        assert_eq!(EntryList::merge(&shared, &local).identity, Some(first.clone()));

        let published = EntryList::merge(&shared, &local);
        let second = EntryList { identity: Some(shared_identity(&Identity::generate())), ..local.clone() };
        assert_eq!(EntryList::merge(&published, &second).identity, Some(first));

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

        assert_eq!(identity_step(&list(&account, Some(&key)), &shared), IdentityStep::Keep);
        assert_eq!(identity_step(&list(&account, Some(&ours)), &list(&account, None)), IdentityStep::Keep);
        assert_eq!(identity_step(&list(&account, Some(&ours)), &EntryList::default()), IdentityStep::Keep);
        assert_eq!(
            identity_step(&list(&account, Some(&ours)), &shared),
            IdentityStep::UseShared(shared_identity(&key))
        );
        assert_eq!(identity_step(&list(&account, None), &shared), IdentityStep::UseShared(shared_identity(&key)));

        let device = UserProfile::new("new device");
        assert_eq!(identity_step(&list(&device, Some(&ours)), &shared), IdentityStep::Adopt);
        assert_eq!(identity_step(&list(&device, Some(&ours)), &list(&account, None)), IdentityStep::WaitForKey);
        let damaged = EntryList { identity: Some("bm90IGEga2V5".into()), ..shared.clone() };
        assert_eq!(identity_step(&list(&device, Some(&ours)), &damaged), IdentityStep::WaitForKey);
        assert_eq!(identity_step(&list(&account, Some(&ours)), &damaged), IdentityStep::Keep, "never a damaged key");
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
        assert!(!write_identity(&dir, "bm90IGEga2V5"));
        assert!(!write_identity(&dir, "%%%"));
        assert_eq!(commy_core::identity::load_or_create(&dir).unwrap().public(), key.public());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temp file left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn rand_tag() -> u128 {
        std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos()
    }

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
        assert_eq!(take_identity_step(&app, &dir, IdentityStep::Adopt, &shared).await, AfterStep::Wait);
        assert_eq!(app.profile.lock().unwrap().as_ref().unwrap().id, device.id);
        assert_eq!(file_key(&dir), ours.public());
        assert!(!app.key_settled() && app.key_marked_unsettled(), "still held back");

        assert_eq!(take_identity_step(&app, &dir, IdentityStep::Keep, &shared).await, AfterStep::Merge);
        assert_eq!(take_identity_step(&app, &dir, IdentityStep::OwnIdentity, &shared).await, AfterStep::StayOut);
        assert_eq!(file_key(&dir), ours.public());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_syncs_back_off_and_then_hold_steady() {
        let secs: Vec<u64> = (0..5).map(|n| failure_retry_delay(n).as_secs()).collect();
        assert_eq!(secs, [15, 60, 300, 300, 300]);
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

        assert_eq!(identity_part(&app, &dir, &local, &local, &wait).await, (AfterStep::Merge, false));
        assert_eq!(identity_part(&app, &dir, &local, &shared, &wait).await, (AfterStep::Wait, true));
        assert_eq!(identity_part(&app, &dir, &local, &shared, &wait).await, (AfterStep::Wait, false));
        assert_eq!(identity_part(&app, &dir, &local, &shared, &wait).await, (AfterStep::Wait, false));
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
