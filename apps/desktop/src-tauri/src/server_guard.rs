use crate::lock_or_recover::{LockOrRecover, ReadOrRecover};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

use commy_core::home::{fresh_card_fits, request_proof, wants_card, HomeCard};
use commy_core::server_blocklist::Envelope;
use commy_core::server_key::PublishedKey;
use commy_view::homes::{HomeGuard, HomeStatus};
use commy_view::listed::{JoinHost, ListedServers, ListedView};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::invite_link::DEFAULT_ACCOUNT_SERVER;
use crate::state::App;

pub const LIST_FILE: &str = "server-blocklist.json";
pub const SERVER_LIST_FILE: &str = "server-list.json";
pub const TRUSTED_HOSTS_FILE: &str = "trusted-sync-hosts.json";
const MAX_TRUSTED_HOSTS: usize = 500;
const FIRST_REFRESH_DELAY: Duration = Duration::from_secs(5);
const REFRESH_EVERY: Duration = Duration::from_secs(15 * 60);
const RETRY_AFTER: Duration = Duration::from_secs(5 * 60);
const FIRST_CARD_DELAY: Duration = Duration::from_secs(30);
const CARD_CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_HOMES_PER_ASK: usize = 100;
const SIGN_IN_WAIT: Duration = Duration::from_secs(8);
const SIGN_IN_RETRY_AFTER: Duration = Duration::from_secs(60);

static GUARD: RwLock<Option<HomeGuard>> = RwLock::new(None);
static LISTED: RwLock<Option<ListedServers>> = RwLock::new(None);
static TRUSTED: RwLock<Vec<String>> = RwLock::new(Vec::new());
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
static HANDLE: OnceLock<AppHandle> = OnceLock::new();
static LAST_SIGN_IN_FETCH: tokio::sync::Mutex<Option<Instant>> = tokio::sync::Mutex::const_new(None);

fn listed<T>(look: impl FnOnce(&ListedServers) -> T) -> T {
    let held = LISTED.read().unwrap();
    match held.as_ref() {
        Some(listed) => look(listed),
        None => look(&ListedServers::default()),
    }
}

fn change_listed<T>(change: impl FnOnce(&mut ListedServers) -> T) -> T {
    let mut held = LISTED.write().unwrap();
    change(held.get_or_insert_with(ListedServers::default))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn listed_homes() -> Vec<ListedView> {
    listed(|listed| listed.homes(now_ms(), blocks_origin))
}

pub fn is_listed_home(origin: &str) -> bool {
    listed(|listed| listed.current(now_ms()).lists_home(origin))
}

pub fn join_host(sync_host: Option<&str>, account_server: &str) -> Option<JoinHost> {
    let trusted = TRUSTED.read().unwrap().clone();
    listed(|listed| listed.join_host(sync_host, account_server, DEFAULT_ACCOUNT_SERVER, &trusted, now_ms()))
}

fn load_trusted(dir: &Path) {
    let Ok(text) = std::fs::read_to_string(dir.join(TRUSTED_HOSTS_FILE)) else { return };
    let Ok(hosts) = serde_json::from_str::<Vec<String>>(&text) else { return };
    *TRUSTED.write().unwrap() = hosts.into_iter().filter(|h| commy_core::sync_host::is_sync_host(h)).take(MAX_TRUSTED_HOSTS).collect();
}

#[tauri::command]
pub fn trust_sync_host(host: String) -> Result<(), String> {
    let origin = commy_core::sync_host::clean_sync_host(&host).ok_or("That is not a server address")?;
    let saved = {
        let mut trusted = TRUSTED.write().unwrap();
        if trusted.contains(&origin) {
            return Ok(());
        }
        trusted.push(origin);
        let excess = trusted.len().saturating_sub(MAX_TRUSTED_HOSTS);
        trusted.drain(..excess);
        serde_json::to_string(&*trusted).map_err(|e| e.to_string())?
    };
    if let Some(dir) = DATA_DIR.get() {
        std::fs::write(dir.join(TRUSTED_HOSTS_FILE), saved).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn listed_servers() -> Vec<ListedView> {
    listed_homes()
}

fn read<T>(look: impl FnOnce(&HomeGuard) -> T) -> T {
    let guard = GUARD.read_or_recover();
    look(guard.as_ref().unwrap_or(&HomeGuard::default()))
}

fn write<T>(change: impl FnOnce(&mut HomeGuard) -> T) -> T {
    let mut guard = GUARD.write_or_recover();
    change(guard.get_or_insert_with(HomeGuard::default))
}

pub fn blocks_origin(origin: &str) -> bool {
    read(|guard| guard.blocks_origin(origin))
}

pub fn blocks_profile(profile: &commy_core::UserProfile) -> bool {
    read(|guard| guard.hides(profile))
}

#[derive(Serialize, Deserialize)]
struct SavedList {
    origin: String,
    key: PublishedKey,
    envelope: Envelope,
}

fn list_file(dir: &Path) -> PathBuf {
    dir.join(LIST_FILE)
}

fn read_saved(path: &Path) -> Option<SavedList> {
    let saved = serde_json::from_str::<SavedList>(&std::fs::read_to_string(path).ok()?).ok()?;
    (saved.origin == DEFAULT_ACCOUNT_SERVER).then_some(saved)
}

fn load_saved(dir: &Path) {
    if let Some(saved) = read_saved(&list_file(dir)) {
        let _ = write(|guard| guard.take_list(&saved.origin, &saved.key, &saved.envelope));
    }
    if let Some(saved) = read_saved(&dir.join(SERVER_LIST_FILE)) {
        let _ = change_listed(|listed| listed.take_list(&saved.origin, &saved.key, &saved.envelope));
    }
}

fn save(path: &Path, origin: &str, key: PublishedKey, envelope: Envelope) {
    let saved = SavedList { origin: origin.to_string(), key, envelope };
    if let Ok(text) = serde_json::to_string(&saved) {
        let _ = std::fs::write(path, text.as_bytes());
    }
}

fn holds_blocklist() -> bool {
    read(|guard| guard.list().is_some())
}

fn fetch_before_sign_in(server: &str, holds_list: bool, last_try: Option<Instant>, now: Instant) -> bool {
    server != DEFAULT_ACCOUNT_SERVER && !holds_list && last_try.is_none_or(|at| now.duration_since(at) >= SIGN_IN_RETRY_AFTER)
}

pub async fn load_blocklist_before_sign_in(server: &str) {
    if !fetch_before_sign_in(server, holds_blocklist(), None, Instant::now()) {
        return;
    }
    let (Some(handle), Some(dir)) = (HANDLE.get(), DATA_DIR.get()) else { return };
    let mut last_try = LAST_SIGN_IN_FETCH.lock().await;
    let now = Instant::now();
    if !fetch_before_sign_in(server, holds_blocklist(), *last_try, now) {
        return;
    }
    *last_try = Some(now);
    match tokio::time::timeout(SIGN_IN_WAIT, refresh(handle, dir)).await {
        Ok(Ok(())) => {}
        Ok(Err(why)) => tracing::warn!("server blocklist not loaded before signing in to {server}: {why}"),
        Err(_) => tracing::warn!("server blocklist took too long to load before signing in to {server}"),
    }
}

pub fn start(handle: &AppHandle, dir: &Path) {
    let _ = DATA_DIR.set(dir.to_path_buf());
    let _ = HANDLE.set(handle.clone());
    load_saved(dir);
    load_trusted(dir);
    let handle = handle.clone();
    let dir = dir.to_path_buf();
    tauri::async_runtime::spawn(async move {
        if holds_blocklist() {
            tokio::time::sleep(FIRST_REFRESH_DELAY).await;
        }
        loop {
            let wait = match refresh(&handle, &dir).await {
                Ok(()) => REFRESH_EVERY,
                Err(why) => {
                    tracing::debug!("server blocklist not refreshed: {why}");
                    RETRY_AFTER
                }
            };
            tokio::time::sleep(wait).await;
        }
    });
}

#[derive(Deserialize)]
struct ListPointer {
    url: String,
    key: PublishedKey,
}

#[derive(Deserialize)]
struct OfficialDiscovery {
    issuer: String,
    #[serde(rename = "serverBlocklist")]
    server_blocklist: Option<ListPointer>,
    #[serde(rename = "serverList")]
    server_list: Option<ListPointer>,
}

fn on_official_server(pointer: &ListPointer, origin: &str) -> bool {
    pointer.url.starts_with(&format!("{origin}/"))
}

async fn fetch_envelope(app: &App, url: &str) -> Result<Envelope, String> {
    app.http.get(url).timeout(REQUEST_TIMEOUT).send().await.map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())
}

async fn refresh_listed(handle: &AppHandle, dir: &Path, pointer: Option<ListPointer>, origin: &str) -> Result<(), String> {
    let Some(pointer) = pointer else { return Ok(()) };
    if !on_official_server(&pointer, origin) {
        return Err("the server list address is not on the official server".into());
    }
    let envelope = fetch_envelope(&handle.state::<App>(), &pointer.url).await?;
    if change_listed(|listed| listed.take_list(origin, &pointer.key, &envelope))? {
        save(&dir.join(SERVER_LIST_FILE), origin, pointer.key, envelope);
        let _ = handle.emit("listed-servers-changed", ());
    }
    Ok(())
}

async fn refresh(handle: &AppHandle, dir: &Path) -> Result<(), String> {
    let app = handle.state::<App>();
    let origin = DEFAULT_ACCOUNT_SERVER;
    let doc: OfficialDiscovery = app
        .http
        .get(format!("{origin}/.well-known/commy-auth.json"))
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if doc.issuer != origin {
        return Err("the official discovery document names another server".into());
    }
    let listed = refresh_listed(handle, dir, doc.server_list, origin).await;
    let pointer = doc.server_blocklist.ok_or("the official server publishes no blocklist yet")?;
    if !on_official_server(&pointer, origin) {
        return Err("the blocklist address is not on the official server".into());
    }
    let envelope = fetch_envelope(&app, &pointer.url).await?;
    let changed = write(|guard| guard.take_list(origin, &pointer.key, &envelope))?;
    if changed {
        save(&list_file(dir), origin, pointer.key, envelope);
        crate::community_sync::refresh_soon(handle);
        let _ = handle.emit("homes-changed", ());
    }
    listed
}

pub fn sign_in_refusal(server: &str, adults_only: bool) -> Option<String> {
    let host = crate::account::host_of(server);
    if blocks_origin(server) {
        return Some(format!("{host} is on Commy's list of blocked servers, so Commy will not sign in there."));
    }
    let checked = server.starts_with("https://") && !crate::account::is_bench_server(server);
    (checked && !adults_only).then(|| format!("{host} does not check that people are 18 or older, so Commy will not sign in there."))
}

#[derive(Deserialize)]
struct HomeKeyReply {
    key: PublishedKey,
}

async fn look_up(app: &App, home: &str) -> Option<PublishedKey> {
    let (server, token) = app.account.lock_or_recover().credentials("home_key")?;
    let res = app
        .http
        .get(format!("{server}/api/auth/commy/home-key"))
        .query(&[("home", home)])
        .bearer_auth(token)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    res.json::<HomeKeyReply>().await.ok().map(|reply| reply.key)
}

#[tauri::command]
pub async fn home_status(app: tauri::State<'_, App>, homes: Vec<String>) -> Result<Vec<HomeStatus>, String> {
    let mut asked: Vec<String> = homes.into_iter().filter(|home| commy_core::sync_host::is_sync_host(home)).collect();
    asked.sort();
    asked.dedup();
    asked.truncate(MAX_HOMES_PER_ASK);
    for home in &asked {
        if read(|guard| guard.needs_lookup(home)) {
            let key = look_up(&app, home).await;
            write(|guard| guard.remember_key(home, key));
        }
    }
    Ok(asked.iter().map(|home| read(|guard| guard.status(home))).collect())
}

#[derive(Deserialize)]
struct CardReply {
    card: HomeCard,
}

async fn fetch_card(app: &App) -> Result<Option<HomeCard>, String> {
    let Some((server, token)) = app.account.lock_or_recover().credentials("home_card") else { return Ok(None) };
    let Some(identity) = app.identity() else { return Ok(None) };
    let Some((user, current)) = app.profile.lock_or_recover().as_ref().map(|p| (p.id, p.home.clone())) else { return Ok(None) };
    let now = chrono::Utc::now().timestamp_millis();
    let current = current.filter(|card| card.user == user && card.identity == identity.public());
    if !wants_card(current.as_ref(), &server, now) {
        return Ok(None);
    }
    let body = serde_json::json!({
        "user": user.to_string(),
        "identity": identity.public().to_hex(),
        "at": now,
        "proof": request_proof(&identity, &server, user, now),
    });
    let res = app
        .http
        .post(format!("{server}/api/auth/commy/home-card"))
        .bearer_auth(token)
        .json(&body)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("answered {}", res.status()));
    }
    let card = res.json::<CardReply>().await.map_err(|e| e.to_string())?.card;
    Ok(fresh_card_fits(&card, &server, user, &identity.public(), now).then_some(card))
}

pub async fn renew_home_card(handle: &AppHandle) {
    let app = handle.state::<App>();
    match fetch_card(&app).await {
        Ok(Some(card)) => crate::set_home_card(&app, handle, card).await,
        Ok(None) => {}
        Err(why) => tracing::debug!("home card not renewed: {why}"),
    }
}

pub fn keep_home_card(handle: &AppHandle) {
    let handle = handle.clone();
    crate::account_session::spawn(async move {
        tokio::time::sleep(FIRST_CARD_DELAY).await;
        loop {
            renew_home_card(&handle).await;
            crate::account_move::check_moved(&handle).await;
            tokio::time::sleep(CARD_CHECK_EVERY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_app_fetches_the_blocklist_before_signing_in_elsewhere() {
        let now = Instant::now();
        assert!(fetch_before_sign_in("https://dev.commy.chat", false, None, now));
        assert!(!fetch_before_sign_in("https://dev.commy.chat", true, None, now), "a held list is enough");
        assert!(!fetch_before_sign_in(DEFAULT_ACCOUNT_SERVER, false, None, now), "Commy's own server is never blocked");
        assert!(!fetch_before_sign_in("https://dev.commy.chat", false, Some(now), now + Duration::from_secs(5)), "offline sign-ins do not wait every time");
        assert!(fetch_before_sign_in("https://dev.commy.chat", false, Some(now), now + SIGN_IN_RETRY_AFTER));
    }

    #[test]
    fn a_server_on_the_held_blocklist_is_refused() {
        let signer = commy_core::Identity::from_bytes(&[9; 32]);
        let list = commy_core::server_blocklist::ServerBlocklist {
            v: commy_core::server_blocklist::LIST_VERSION,
            issued: 1,
            expires: i64::MAX,
            domains: vec![commy_core::server_blocklist::BlockedServer { domain: "blocked.example".into(), since: 0 }],
            homes: Vec::new(),
        };
        let text = serde_json::to_string(&list).unwrap();
        let sig = commy_core::server_key::sign(&signer, commy_core::server_blocklist::LIST_PURPOSE, DEFAULT_ACCOUNT_SERVER, text.as_bytes());
        let envelope = Envelope { list: text, key: commy_core::server_key::key_id(&signer.public()), sig };
        write(|guard| guard.take_list(DEFAULT_ACCOUNT_SERVER, &PublishedKey::of(signer.public()), &envelope)).unwrap();
        assert!(holds_blocklist());
        let refusal = sign_in_refusal("https://blocked.example", true).unwrap();
        assert!(refusal.contains("list of blocked servers"), "{refusal}");
        assert_eq!(sign_in_refusal("https://friends.example", true), None);
    }

    #[test]
    fn sign_in_is_refused_where_the_adult_check_is_off() {
        assert!(sign_in_refusal("https://friends.org", false).unwrap().contains("18 or older"));
        assert_eq!(sign_in_refusal("https://friends.org", true), None);
        assert_eq!(sign_in_refusal("http://127.0.0.1:3001", false), None);
    }
}
