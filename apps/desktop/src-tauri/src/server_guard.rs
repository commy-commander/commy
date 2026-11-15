use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::Duration;

use commy_core::home::{fresh_card_fits, request_proof, wants_card, HomeCard};
use commy_core::server_blocklist::Envelope;
use commy_core::server_key::PublishedKey;
use commy_view::homes::{HomeGuard, HomeStatus};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::invite_link::DEFAULT_ACCOUNT_SERVER;
use crate::state::App;

pub const LIST_FILE: &str = "server-blocklist.json";
const FIRST_REFRESH_DELAY: Duration = Duration::from_secs(5);
const REFRESH_EVERY: Duration = Duration::from_secs(15 * 60);
const RETRY_AFTER: Duration = Duration::from_secs(5 * 60);
const FIRST_CARD_DELAY: Duration = Duration::from_secs(30);
const CARD_CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_HOMES_PER_ASK: usize = 100;

static GUARD: RwLock<Option<HomeGuard>> = RwLock::new(None);

fn read<T>(look: impl FnOnce(&HomeGuard) -> T) -> T {
    let guard = GUARD.read().unwrap();
    look(guard.as_ref().unwrap_or(&HomeGuard::default()))
}

fn write<T>(change: impl FnOnce(&mut HomeGuard) -> T) -> T {
    let mut guard = GUARD.write().unwrap();
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

fn load_saved(dir: &Path) {
    let Ok(text) = std::fs::read_to_string(list_file(dir)) else { return };
    let Ok(saved) = serde_json::from_str::<SavedList>(&text) else { return };
    if saved.origin == DEFAULT_ACCOUNT_SERVER {
        let _ = write(|guard| guard.take_list(&saved.origin, &saved.key, &saved.envelope));
    }
}

pub fn start(handle: &AppHandle, dir: &Path) {
    load_saved(dir);
    let handle = handle.clone();
    let dir = dir.to_path_buf();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_REFRESH_DELAY).await;
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
    let pointer = doc.server_blocklist.ok_or("the official server publishes no blocklist yet")?;
    if doc.issuer != origin || !pointer.url.starts_with(&format!("{origin}/")) {
        return Err("the blocklist address is not on the official server".into());
    }
    let envelope: Envelope = app.http.get(&pointer.url).timeout(REQUEST_TIMEOUT).send().await.map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    let changed = write(|guard| guard.take_list(origin, &pointer.key, &envelope))?;
    if changed {
        let saved = SavedList { origin: origin.to_string(), key: pointer.key, envelope };
        if let Ok(text) = serde_json::to_string(&saved) {
            let _ = std::fs::write(list_file(dir), text.as_bytes());
        }
        crate::community_sync::refresh_soon(handle);
        let _ = handle.emit("homes-changed", ());
    }
    Ok(())
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
    let (server, token) = app.account.lock().unwrap().credentials("home_key")?;
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
    let Some((server, token)) = app.account.lock().unwrap().credentials("home_card") else { return Ok(None) };
    let Some(identity) = app.identity() else { return Ok(None) };
    let Some((user, current)) = app.profile.lock().unwrap().as_ref().map(|p| (p.id, p.home.clone())) else { return Ok(None) };
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
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CARD_DELAY).await;
        loop {
            renew_home_card(&handle).await;
            tokio::time::sleep(CARD_CHECK_EVERY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_in_is_refused_where_the_adult_check_is_off() {
        assert!(sign_in_refusal("https://friends.org", false).unwrap().contains("18 or older"));
        assert_eq!(sign_in_refusal("https://friends.org", true), None);
        assert_eq!(sign_in_refusal("http://127.0.0.1:3001", false), None);
    }
}
