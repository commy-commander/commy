use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use chrono::Utc;
use commy_sync::wake::{wake_day, WakeForget, WakeRegister, WakeRing, MAX_DAYS_AHEAD, MAX_RING_TAGS};
use commy_sync::wire::Bytes;
use rand::Rng;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::state::App;
use crate::{dm_sync, server_wake, sync_link};

const RING_DELAY_MS: std::ops::Range<u64> = 1_000..4_000;
const REGISTER_GAP_MS: std::ops::Range<u64> = 300..1_500;
const KEPT_FILE: &str = "wake-tags.json";

type Tag = ([u8; 32], i64);

struct Registered {
    push: String,
    base: String,
    tags: HashSet<Tag>,
    earlier: HashSet<Tag>,
    off_on: Option<i64>,
}

#[derive(Serialize, Deserialize)]
struct Kept {
    push: String,
    base: String,
    tags: Vec<(Bytes<32>, i64)>,
}

static TOKEN: Mutex<Option<String>> = Mutex::new(None);
static REGISTERED: Mutex<Option<Registered>> = Mutex::new(None);
static REGISTERING: AtomicBool = AtomicBool::new(false);
static AGAIN: AtomicBool = AtomicBool::new(false);
static HANDLE: OnceLock<AppHandle> = OnceLock::new();

pub fn wake_url(sync_url: &str) -> Option<String> {
    let (scheme, rest) = match sync_url.strip_prefix("wss://") {
        Some(rest) => ("https", rest),
        None => ("http", sync_url.strip_prefix("ws://")?),
    };
    Some(format!("{scheme}://{}/wake", rest.trim_end_matches('/')))
}

pub fn today() -> i64 {
    wake_day(Utc::now().timestamp_millis())
}

pub async fn ring(http: reqwest::Client, tags: Vec<[u8; 32]>) {
    let Some(base) = sync_link::url().as_deref().and_then(wake_url) else { return };
    if tags.is_empty() {
        return;
    }
    let delay = rand::thread_rng().gen_range(RING_DELAY_MS);
    tokio::time::sleep(Duration::from_millis(delay)).await;
    for chunk in tags.chunks(MAX_RING_TAGS) {
        let body = WakeRing { tags: chunk.iter().map(|tag| Bytes(*tag)).collect() };
        match http.post(format!("{base}/ring")).json(&body).send().await {
            Ok(res) if res.status().is_success() => tracing::info!("rang {} wake-up tags", chunk.len()),
            Ok(res) => tracing::info!("the sync service refused a wake-up ring: {}", res.status()),
            Err(e) => tracing::info!("could not ring a wake-up: {e}"),
        }
    }
}

#[cfg(target_os = "android")]
pub fn set_token(handle: &AppHandle, token: Option<String>) {
    let token = token.filter(|t| commy_sync::wake::push_target(&format!("fcm:{t}")).is_some());
    let changed = {
        let mut held = TOKEN.lock().unwrap();
        let changed = *held != token;
        *held = token;
        changed
    };
    if changed {
        register(handle);
    }
}

pub fn register_again() {
    if let Some(handle) = HANDLE.get() {
        register(handle);
    }
}

fn target() -> Option<(String, String)> {
    let token = TOKEN.lock().unwrap().clone()?;
    let base = sync_link::url().as_deref().and_then(wake_url)?;
    Some((format!("fcm:{token}"), base))
}

pub fn register(handle: &AppHandle) {
    let _ = HANDLE.set(handle.clone());
    if target().is_none() {
        return;
    }
    if REGISTERING.swap(true, Ordering::SeqCst) {
        AGAIN.store(true, Ordering::SeqCst);
        return;
    }
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            AGAIN.store(false, Ordering::SeqCst);
            if let Some((push, base)) = target() {
                register_now(&handle, push, base).await;
            }
            REGISTERING.store(false, Ordering::SeqCst);
            if !AGAIN.load(Ordering::SeqCst) || REGISTERING.swap(true, Ordering::SeqCst) {
                break;
            }
        }
    });
}

fn days_from(today: i64) -> Vec<i64> {
    (0..MAX_DAYS_AHEAD).map(|ahead| today + ahead).collect()
}

fn begin(push: &str, base: &str, earlier: impl FnOnce() -> HashSet<Tag>) {
    let mut guard = REGISTERED.lock().unwrap();
    if guard.as_ref().is_none_or(|r| r.push != push || r.base != base) {
        *guard = Some(Registered { push: push.to_string(), base: base.to_string(), tags: HashSet::new(), earlier: earlier(), off_on: None });
    }
}

fn still_needed(push: &str, today: i64, wanted: Vec<Tag>) -> Vec<Tag> {
    let mut guard = REGISTERED.lock().unwrap();
    let Some(registered) = guard.as_mut().filter(|r| r.push == push) else { return Vec::new() };
    if registered.off_on == Some(today) {
        return Vec::new();
    }
    registered.tags.retain(|(_, day)| *day >= today);
    wanted.into_iter().filter(|tag| !registered.tags.contains(tag)).collect()
}

fn no_longer_wanted(push: &str, today: i64, wanted: &[Tag]) -> Vec<Tag> {
    let guard = REGISTERED.lock().unwrap();
    let Some(registered) = guard.as_ref().filter(|r| r.push == push) else { return Vec::new() };
    let held: HashSet<&Tag> = registered.tags.iter().chain(&registered.earlier).collect();
    held.into_iter().filter(|tag| tag.1 >= today && !wanted.contains(tag)).copied().collect()
}

fn kept_now(push: &str, today: i64) -> Option<Kept> {
    let guard = REGISTERED.lock().unwrap();
    let registered = guard.as_ref().filter(|r| r.push == push)?;
    let held: HashSet<&Tag> = registered.tags.iter().chain(&registered.earlier).filter(|tag| tag.1 >= today).collect();
    Some(Kept { push: registered.push.clone(), base: registered.base.clone(), tags: held.into_iter().map(|(tag, day)| (Bytes(*tag), *day)).collect() })
}

fn kept_path(app: &App) -> Option<PathBuf> {
    app.base_dir().map(|dir| dir.join(KEPT_FILE))
}

fn load_kept(app: &App, push: &str, base: &str) -> HashSet<Tag> {
    let Some(path) = kept_path(app) else { return HashSet::new() };
    crate::persist::read_json::<Kept>(&path)
        .filter(|kept| kept.push == push && kept.base == base)
        .map(|kept| kept.tags.into_iter().map(|(tag, day)| (tag.0, day)).collect())
        .unwrap_or_default()
}

fn save_kept(app: &App, push: &str, today: i64) {
    let (Some(path), Some(kept)) = (kept_path(app), kept_now(push, today)) else { return };
    if let Err(e) = crate::persist::write_secret_json(&path, &kept) {
        tracing::info!("could not remember the wake-up tags: {e}");
    }
}

fn note(push: &str, update: impl FnOnce(&mut Registered)) {
    if let Some(registered) = REGISTERED.lock().unwrap().as_mut().filter(|r| r.push == push) {
        update(registered);
    }
}

async fn register_now(handle: &AppHandle, push: String, base: String) {
    let app = handle.state::<App>();
    let today = today();
    begin(&push, &base, || load_kept(&app, &push, &base));
    sync_tags(&app, &push, &base, today).await;
    save_kept(&app, &push, today);
}

async fn sync_tags(app: &App, push: &str, base: &str, today: i64) {
    let push = push.to_string();
    let days = days_from(today);
    let mut wanted = dm_sync::wake_tags(app, &days);
    wanted.extend(server_wake::wake_tags(app, &days));
    let stale = no_longer_wanted(&push, today, &wanted);
    let needed = still_needed(&push, today, wanted);
    let http = app.http.clone();
    forget(&http, &push, base, stale).await;
    let mut registered = 0;
    for (tag, day) in needed {
        let gap = rand::thread_rng().gen_range(REGISTER_GAP_MS);
        tokio::time::sleep(Duration::from_millis(gap)).await;
        let body = WakeRegister { tag: Bytes(tag), push: push.clone(), until: day };
        match http.post(format!("{base}/register")).json(&body).send().await.map(|res| res.status()) {
            Ok(status) if status.is_success() => {
                registered += 1;
                note(&push, |r| {
                    r.tags.insert((tag, day));
                });
            }
            Ok(StatusCode::NOT_FOUND) => {
                tracing::info!("the sync service does not offer wake-ups");
                note(&push, |r| r.off_on = Some(today));
                return;
            }
            Ok(StatusCode::TOO_MANY_REQUESTS) => {
                tracing::info!("wake-up registration paused after {registered}: too many at once");
                return;
            }
            Ok(status) => tracing::info!("the sync service refused a wake-up registration: {status}"),
            Err(e) => {
                tracing::info!("could not register for wake-ups: {e}");
                return;
            }
        }
    }
    if registered > 0 {
        tracing::info!("registered {registered} wake-up tags");
    }
}

async fn forget(http: &reqwest::Client, push: &str, base: &str, stale: Vec<Tag>) {
    let mut forgotten = 0;
    for (tag, day) in stale {
        let gap = rand::thread_rng().gen_range(REGISTER_GAP_MS);
        tokio::time::sleep(Duration::from_millis(gap)).await;
        let body = WakeForget { tag: Bytes(tag), push: push.to_string() };
        match http.post(format!("{base}/forget")).json(&body).send().await.map(|res| res.status()) {
            Ok(status) if status.is_success() || status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED => {
                forgotten += 1;
                note(push, |r| {
                    r.tags.remove(&(tag, day));
                    r.earlier.remove(&(tag, day));
                });
            }
            Ok(status) => {
                tracing::info!("the sync service did not drop a wake-up tag: {status}");
                return;
            }
            Err(e) => {
                tracing::info!("could not drop a wake-up tag: {e}");
                return;
            }
        }
    }
    if forgotten > 0 {
        tracing::info!("dropped {forgotten} wake-up tags no longer wanted");
    }
}

#[cfg(target_os = "android")]
mod android {
    use std::sync::Mutex;

    use serde::Deserialize;
    use tauri::plugin::PluginHandle;
    use tauri::{AppHandle, Wry};

    #[derive(Deserialize, Default)]
    struct TokenAnswer {
        #[serde(default)]
        token: Option<String>,
    }

    static PLUGIN: Mutex<Option<PluginHandle<Wry>>> = Mutex::new(None);

    pub fn init() -> tauri::plugin::TauriPlugin<Wry> {
        tauri::plugin::Builder::new("push-wake")
            .setup(|app, api| {
                let plugin = api.register_android_plugin("chat.commy.mobile", "PushPlugin")?;
                *PLUGIN.lock().unwrap() = Some(plugin);
                fetch_token(app.clone());
                Ok(())
            })
            .build()
    }

    pub fn fetch_token(handle: AppHandle) {
        let Some(plugin) = PLUGIN.lock().unwrap().clone() else { return };
        let _ = std::thread::Builder::new().name("push-token".into()).spawn(move || {
            match plugin.run_mobile_plugin::<TokenAnswer>("token", serde_json::json!({})) {
                Ok(answer) => super::set_token(&handle, answer.token),
                Err(e) => tracing::debug!("no push token: {e}"),
            }
        });
    }
}

#[cfg(target_os = "android")]
pub use android::init;

#[cfg(target_os = "android")]
pub fn on_resumed(handle: &AppHandle) {
    android::fetch_token(handle.clone());
}

#[cfg(not(target_os = "android"))]
pub fn on_resumed(_handle: &AppHandle) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_ups_live_beside_the_sync_socket() {
        assert_eq!(wake_url("wss://commy.chat/sync").as_deref(), Some("https://commy.chat/sync/wake"));
        assert_eq!(wake_url("wss://commy.chat:8443/sync/").as_deref(), Some("https://commy.chat:8443/sync/wake"));
        assert_eq!(wake_url("ws://localhost:3002/sync").as_deref(), Some("http://localhost:3002/sync/wake"));
        assert_eq!(wake_url("https://commy.chat/sync"), None);
    }

    #[test]
    fn a_phone_registers_today_and_tomorrow_and_only_once() {
        let base = "https://x/sync/wake";
        assert_eq!(days_from(20_000), vec![20_000, 20_001]);
        let wanted = vec![([1; 32], 20_000), ([2; 32], 20_001)];
        begin("fcm:a", base, HashSet::new);
        assert_eq!(still_needed("fcm:a", 20_000, wanted.clone()), wanted);
        note("fcm:a", |r| {
            r.tags.insert(([1; 32], 20_000));
        });
        assert_eq!(still_needed("fcm:a", 20_000, wanted.clone()), vec![([2; 32], 20_001)]);
        note("fcm:a", |r| r.off_on = Some(20_000));
        assert!(still_needed("fcm:a", 20_000, wanted.clone()).is_empty(), "a service without wake-ups is not asked again that day");
        begin("fcm:b", base, HashSet::new);
        assert_eq!(still_needed("fcm:b", 20_000, wanted.clone()), wanted, "a new token starts over");
        note("fcm:b", |r| {
            r.tags.insert(([1; 32], 20_000));
            r.tags.insert(([3; 32], 20_001));
            r.tags.insert(([4; 32], 19_999));
        });
        assert_eq!(no_longer_wanted("fcm:b", 20_000, &wanted), vec![([3; 32], 20_001)], "settings that changed drop their tags");
        assert!(no_longer_wanted("fcm:a", 20_000, &wanted).is_empty());

        let earlier = HashSet::from([([5; 32], 20_000), ([1; 32], 20_000), ([6; 32], 19_999)]);
        begin("fcm:c", base, || earlier.clone());
        begin("fcm:c", base, || panic!("tags kept from an earlier run are read once"));
        assert_eq!(still_needed("fcm:c", 20_000, wanted.clone()), wanted, "a restarted app registers what it wants again");
        assert_eq!(no_longer_wanted("fcm:c", 20_000, &wanted), vec![([5; 32], 20_000)], "tags from before a restart are dropped when no longer wanted");
        note("fcm:c", |r| {
            r.tags.insert(([2; 32], 20_001));
            r.earlier.remove(&([5; 32], 20_000));
        });
        let mut kept: Vec<Tag> = kept_now("fcm:c", 20_000).unwrap().tags.into_iter().map(|(tag, day)| (tag.0, day)).collect();
        kept.sort();
        assert_eq!(kept, vec![([1; 32], 20_000), ([2; 32], 20_001)], "what is kept for the next run holds both, without past days");
        assert!(kept_now("fcm:a", 20_000).is_none());
    }
}
