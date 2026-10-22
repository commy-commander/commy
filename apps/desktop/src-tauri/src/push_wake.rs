use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use chrono::Utc;
use commy_sync::wake::{wake_day, WakeRegister, WakeRing, MAX_DAYS_AHEAD, MAX_RING_TAGS};
use commy_sync::wire::Bytes;
use rand::Rng;
use reqwest::StatusCode;
use tauri::{AppHandle, Manager};

use crate::state::App;
use crate::{dm_sync, sync_link};

const RING_DELAY_MS: std::ops::Range<u64> = 1_000..4_000;
const REGISTER_GAP_MS: std::ops::Range<u64> = 300..1_500;

struct Registered {
    push: String,
    base: String,
    tags: HashSet<([u8; 32], i64)>,
    off_on: Option<i64>,
}

static TOKEN: Mutex<Option<String>> = Mutex::new(None);
static REGISTERED: Mutex<Option<Registered>> = Mutex::new(None);
static REGISTERING: AtomicBool = AtomicBool::new(false);
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

pub fn register(handle: &AppHandle) {
    let _ = HANDLE.set(handle.clone());
    let Some(token) = TOKEN.lock().unwrap().clone() else { return };
    let Some(base) = sync_link::url().as_deref().and_then(wake_url) else { return };
    if REGISTERING.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        register_now(&handle, format!("fcm:{token}"), base).await;
        REGISTERING.store(false, Ordering::SeqCst);
    });
}

fn days_from(today: i64) -> Vec<i64> {
    (0..MAX_DAYS_AHEAD).map(|ahead| today + ahead).collect()
}

fn still_needed(push: &str, base: &str, today: i64, wanted: Vec<([u8; 32], i64)>) -> Vec<([u8; 32], i64)> {
    let mut guard = REGISTERED.lock().unwrap();
    let fresh = guard.as_ref().is_none_or(|r| r.push != push || r.base != base);
    if fresh {
        *guard = Some(Registered { push: push.to_string(), base: base.to_string(), tags: HashSet::new(), off_on: None });
    }
    let Some(registered) = guard.as_mut() else { return Vec::new() };
    if registered.off_on == Some(today) {
        return Vec::new();
    }
    registered.tags.retain(|(_, day)| *day >= today);
    wanted.into_iter().filter(|tag| !registered.tags.contains(tag)).collect()
}

fn note(push: &str, update: impl FnOnce(&mut Registered)) {
    if let Some(registered) = REGISTERED.lock().unwrap().as_mut().filter(|r| r.push == push) {
        update(registered);
    }
}

async fn register_now(handle: &AppHandle, push: String, base: String) {
    let app = handle.state::<App>();
    let today = today();
    let wanted = dm_sync::wake_tags(&app, &days_from(today));
    let needed = still_needed(&push, &base, today, wanted);
    let http = app.http.clone();
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
        assert_eq!(days_from(20_000), vec![20_000, 20_001]);
        let wanted = vec![([1; 32], 20_000), ([2; 32], 20_001)];
        assert_eq!(still_needed("fcm:a", "https://x/sync/wake", 20_000, wanted.clone()), wanted);
        note("fcm:a", |r| {
            r.tags.insert(([1; 32], 20_000));
        });
        assert_eq!(still_needed("fcm:a", "https://x/sync/wake", 20_000, wanted.clone()), vec![([2; 32], 20_001)]);
        note("fcm:a", |r| r.off_on = Some(20_000));
        assert!(still_needed("fcm:a", "https://x/sync/wake", 20_000, wanted.clone()).is_empty(), "a service without wake-ups is not asked again that day");
        assert_eq!(still_needed("fcm:b", "https://x/sync/wake", 20_000, wanted.clone()), wanted, "a new token starts over");
    }
}
