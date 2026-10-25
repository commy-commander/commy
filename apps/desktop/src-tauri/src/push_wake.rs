use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use chrono::Utc;
use commy_sync::wake::{wake_day, WakeForget, WakeRegister, WakeRing, MAX_DAYS_AHEAD, MAX_RING_TAGS};
use commy_core::ServerId;
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
const SEND_RETRIES: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(10)];
const RETRY_LATER: Duration = Duration::from_secs(60);

type Tag = ([u8; 32], i64);
type Tagged = Vec<(Tag, Source)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Source {
    Dm,
    Server(ServerId),
}

struct Registered {
    push: String,
    base: String,
    tags: HashMap<Tag, Source>,
    earlier: HashMap<Tag, Source>,
    swept: HashSet<Tag>,
    kept_whole: bool,
    off_on: Option<i64>,
}

#[derive(Serialize, Deserialize)]
struct Kept {
    push: String,
    base: String,
    tags: Vec<(Bytes<32>, i64, Source)>,
}

static TOKEN: Mutex<Option<String>> = Mutex::new(None);
static REGISTERED: Mutex<Option<Registered>> = Mutex::new(None);
static REGISTERING: AtomicBool = AtomicBool::new(false);
static AGAIN: AtomicBool = AtomicBool::new(false);
static RETRY_SOON: AtomicBool = AtomicBool::new(false);
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
        ask_token(handle);
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
            let done = match target() {
                Some((push, base)) => register_now(&handle, push, base).await,
                None => true,
            };
            if !done {
                retry_later();
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

fn begin(push: &str, base: &str, earlier: impl FnOnce() -> Option<HashMap<Tag, Source>>) {
    let mut guard = REGISTERED.lock().unwrap();
    if guard.as_ref().is_none_or(|r| r.push != push || r.base != base) {
        let earlier = earlier();
        let kept_whole = earlier.is_some();
        *guard = Some(Registered {
            push: push.to_string(),
            base: base.to_string(),
            tags: HashMap::new(),
            earlier: earlier.unwrap_or_default(),
            swept: HashSet::new(),
            kept_whole,
            off_on: None,
        });
    }
}

fn still_needed(push: &str, today: i64, wanted: Vec<(Tag, Source)>) -> Vec<(Tag, Source)> {
    let mut guard = REGISTERED.lock().unwrap();
    let Some(registered) = guard.as_mut().filter(|r| r.push == push) else { return Vec::new() };
    if registered.off_on == Some(today) {
        return Vec::new();
    }
    registered.tags.retain(|(_, day), _| *day >= today);
    wanted.into_iter().filter(|(tag, _)| !registered.tags.contains_key(tag)).collect()
}

fn fresh_first(push: &str, needed: Vec<(Tag, Source)>) -> (Tagged, Tagged) {
    let guard = REGISTERED.lock().unwrap();
    let Some(registered) = guard.as_ref().filter(|r| r.push == push) else { return (needed, Vec::new()) };
    needed.into_iter().partition(|(tag, _)| !registered.earlier.contains_key(tag))
}

fn no_longer_wanted(push: &str, today: i64, wanted: &[(Tag, Source)], settled: impl Fn(&Source) -> bool) -> Vec<Tag> {
    let guard = REGISTERED.lock().unwrap();
    let Some(registered) = guard.as_ref().filter(|r| r.push == push) else { return Vec::new() };
    let wanted: HashSet<&Tag> = wanted.iter().map(|(tag, _)| tag).collect();
    let stale: HashSet<Tag> = registered
        .tags
        .iter()
        .chain(&registered.earlier)
        .filter(|(tag, source)| tag.1 >= today && !wanted.contains(tag) && settled(source))
        .map(|(tag, _)| *tag)
        .collect();
    stale.into_iter().collect()
}

fn to_sweep(push: &str, today: i64, wanted: &[(Tag, Source)], unwanted: Vec<Tag>) -> Vec<Tag> {
    let guard = REGISTERED.lock().unwrap();
    let Some(registered) = guard.as_ref().filter(|r| r.push == push && !r.kept_whole) else { return Vec::new() };
    let wanted: HashSet<&Tag> = wanted.iter().map(|(tag, _)| tag).collect();
    let sweep: HashSet<Tag> = unwanted.into_iter().filter(|tag| tag.1 >= today && !wanted.contains(tag) && !registered.swept.contains(tag)).collect();
    sweep.into_iter().collect()
}

fn kept_now(push: &str, today: i64) -> Option<Kept> {
    let guard = REGISTERED.lock().unwrap();
    let registered = guard.as_ref().filter(|r| r.push == push)?;
    let held: HashMap<&Tag, &Source> = registered.earlier.iter().chain(&registered.tags).filter(|(tag, _)| tag.1 >= today).collect();
    Some(Kept { push: registered.push.clone(), base: registered.base.clone(), tags: held.into_iter().map(|((tag, day), source)| (Bytes(*tag), *day, *source)).collect() })
}

fn kept_path(app: &App) -> Option<PathBuf> {
    app.base_dir().map(|dir| dir.join(KEPT_FILE))
}

fn load_kept(app: &App, push: &str, base: &str) -> Option<HashMap<Tag, Source>> {
    let kept = kept_path(app).and_then(|path| crate::persist::read_json::<Kept>(&path))?;
    if kept.push != push || kept.base != base {
        tracing::info!("the wake-up tags kept from an earlier run were for another push token or sync service");
        return None;
    }
    tracing::info!("{} wake-up tags kept from an earlier run", kept.tags.len());
    Some(kept.tags.into_iter().map(|(tag, day, source)| ((tag.0, day), source)).collect())
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

fn retry_later() {
    if RETRY_SOON.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(RETRY_LATER).await;
        RETRY_SOON.store(false, Ordering::SeqCst);
        register_again();
    });
}

async fn post(http: &reqwest::Client, url: &str, body: &impl Serialize) -> Result<StatusCode, reqwest::Error> {
    let mut waits = SEND_RETRIES.iter();
    loop {
        match http.post(url).json(body).send().await {
            Ok(res) => return Ok(res.status()),
            Err(e) => match waits.next() {
                Some(wait) => {
                    tracing::info!("retrying a wake-up request: {e}");
                    tokio::time::sleep(*wait).await;
                }
                None => return Err(e),
            },
        }
    }
}

async fn register_now(handle: &AppHandle, push: String, base: String) -> bool {
    let app = handle.state::<App>();
    let today = today();
    begin(&push, &base, || load_kept(&app, &push, &base));
    let done = sync_tags(&app, &push, &base, today).await;
    save_kept(&app, &push, today);
    done
}

async fn sync_tags(app: &App, push: &str, base: &str, today: i64) -> bool {
    let push = push.to_string();
    let days = days_from(today);
    let dm = dm_sync::wake_tags(app, &days);
    let servers = server_wake::wake_tags(app, &days);
    let dm_settled = dm.is_some();
    let mut wanted: Vec<(Tag, Source)> = dm.unwrap_or_default().into_iter().map(|tag| (tag, Source::Dm)).collect();
    wanted.extend(servers.tags.into_iter().map(|(server, tag)| (tag, Source::Server(server))));
    let stale = no_longer_wanted(&push, today, &wanted, |source| match source {
        Source::Dm => dm_settled,
        Source::Server(server) => !servers.unsettled.contains(server),
    });
    let http = app.http.clone();
    let (fresh, kept) = fresh_first(&push, still_needed(&push, today, wanted.clone()));
    if let Err(done) = register_tags(app, &http, &push, base, today, fresh).await {
        return done;
    }
    let forgotten = forget(app, &http, &push, base, today, stale).await;
    let sweep = to_sweep(&push, today, &wanted, servers.unwanted.into_iter().map(|(_, tag)| tag).collect());
    let swept = forget(app, &http, &push, base, today, sweep).await;
    if let Err(done) = register_tags(app, &http, &push, base, today, kept).await {
        return done;
    }
    forgotten && swept
}

async fn register_tags(app: &App, http: &reqwest::Client, push: &str, base: &str, today: i64, tags: Vec<(Tag, Source)>) -> Result<(), bool> {
    let mut registered = 0;
    for ((tag, day), source) in tags {
        let gap = rand::thread_rng().gen_range(REGISTER_GAP_MS);
        tokio::time::sleep(Duration::from_millis(gap)).await;
        let body = WakeRegister { tag: Bytes(tag), push: push.to_string(), until: day };
        match post(http, &format!("{base}/register"), &body).await {
            Ok(status) if status.is_success() => {
                registered += 1;
                note(push, |r| {
                    r.tags.insert((tag, day), source);
                    r.swept.remove(&(tag, day));
                });
                save_kept(app, push, today);
                if AGAIN.load(Ordering::SeqCst) {
                    tracing::info!("registered {registered} wake-up tags, then started over for newer settings");
                    return Err(true);
                }
            }
            Ok(StatusCode::NOT_FOUND) => {
                tracing::info!("the sync service does not offer wake-ups");
                note(push, |r| r.off_on = Some(today));
                return Err(true);
            }
            Ok(StatusCode::TOO_MANY_REQUESTS) => {
                tracing::info!("wake-up registration paused after {registered}: too many at once");
                return Err(false);
            }
            Ok(status) => tracing::info!("the sync service refused a wake-up registration: {status}"),
            Err(e) => {
                tracing::info!("could not register for wake-ups: {e}");
                return Err(false);
            }
        }
    }
    if registered > 0 {
        tracing::info!("registered {registered} wake-up tags");
    }
    Ok(())
}

async fn forget(app: &App, http: &reqwest::Client, push: &str, base: &str, today: i64, stale: Vec<Tag>) -> bool {
    let mut forgotten = 0;
    for (tag, day) in stale {
        let gap = rand::thread_rng().gen_range(REGISTER_GAP_MS);
        tokio::time::sleep(Duration::from_millis(gap)).await;
        let body = WakeForget { tag: Bytes(tag), push: push.to_string() };
        match post(http, &format!("{base}/forget"), &body).await {
            Ok(status) if status.is_success() || status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED => {
                forgotten += 1;
                note(push, |r| {
                    r.tags.remove(&(tag, day));
                    r.earlier.remove(&(tag, day));
                    r.swept.insert((tag, day));
                });
                save_kept(app, push, today);
            }
            Ok(status) => {
                tracing::info!("the sync service did not drop a wake-up tag: {status}");
                return false;
            }
            Err(e) => {
                tracing::info!("could not drop a wake-up tag: {e}");
                return false;
            }
        }
    }
    if forgotten > 0 {
        tracing::info!("dropped {forgotten} wake-up tags no longer wanted");
    }
    true
}

#[cfg(target_os = "android")]
mod android {
    use std::sync::atomic::{AtomicBool, Ordering};
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
    static ASKING: AtomicBool = AtomicBool::new(false);

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
        if ASKING.swap(true, Ordering::SeqCst) {
            return;
        }
        let asked = std::thread::Builder::new().name("push-token".into()).spawn(move || {
            let answer = plugin.run_mobile_plugin::<TokenAnswer>("token", serde_json::json!({}));
            ASKING.store(false, Ordering::SeqCst);
            match answer {
                Ok(answer) => super::set_token(&handle, answer.token),
                Err(e) => tracing::info!("no push token: {e}"),
            }
        });
        if asked.is_err() {
            ASKING.store(false, Ordering::SeqCst);
        }
    }
}

#[cfg(target_os = "android")]
pub use android::init;

#[cfg(target_os = "android")]
fn ask_token(handle: &AppHandle) {
    if TOKEN.lock().unwrap().is_none() {
        android::fetch_token(handle.clone());
    }
}

#[cfg(not(target_os = "android"))]
fn ask_token(_handle: &AppHandle) {}

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

    #[tokio::test]
    async fn a_dropped_connection_is_tried_again() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/sync/wake/forget", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (first, _) = listener.accept().await.unwrap();
            drop(first);
            let (mut second, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let _ = second.read(&mut request).await;
            second.write_all(b"HTTP/1.1 204 No Content\r\nconnection: close\r\n\r\n").await.unwrap();
        });
        let http = reqwest::Client::new();
        let status = post(&http, &url, &serde_json::json!({ "tag": "x" })).await.unwrap();
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    #[test]
    fn a_phone_registers_today_and_tomorrow_and_only_once() {
        let base = "https://x/sync/wake";
        let server = ServerId::new();
        let s = Source::Server(server);
        let all = |_: &Source| true;
        assert_eq!(days_from(20_000), vec![20_000, 20_001]);
        let wanted = vec![(([1; 32], 20_000), Source::Dm), (([2; 32], 20_001), s)];
        begin("fcm:a", base, || None);
        assert_eq!(still_needed("fcm:a", 20_000, wanted.clone()), wanted);
        note("fcm:a", |r| {
            r.tags.insert(([1; 32], 20_000), Source::Dm);
        });
        assert_eq!(still_needed("fcm:a", 20_000, wanted.clone()), vec![(([2; 32], 20_001), s)]);
        note("fcm:a", |r| r.off_on = Some(20_000));
        assert!(still_needed("fcm:a", 20_000, wanted.clone()).is_empty(), "a service without wake-ups is not asked again that day");
        begin("fcm:b", base, || None);
        assert_eq!(still_needed("fcm:b", 20_000, wanted.clone()), wanted, "a new token starts over");
        note("fcm:b", |r| {
            r.tags.insert(([1; 32], 20_000), Source::Dm);
            r.tags.insert(([3; 32], 20_001), s);
            r.tags.insert(([4; 32], 19_999), s);
        });
        assert_eq!(no_longer_wanted("fcm:b", 20_000, &wanted, all), vec![([3; 32], 20_001)], "settings that changed drop their tags");
        assert!(no_longer_wanted("fcm:a", 20_000, &wanted, all).is_empty());

        let earlier = HashMap::from([(([5; 32], 20_000), s), (([7; 32], 20_000), Source::Dm), (([1; 32], 20_000), Source::Dm), (([6; 32], 19_999), s)]);
        begin("fcm:c", base, || Some(earlier.clone()));
        begin("fcm:c", base, || panic!("tags kept from an earlier run are read once"));
        assert_eq!(still_needed("fcm:c", 20_000, wanted.clone()), wanted, "a restarted app registers what it wants again");
        assert_eq!(fresh_first("fcm:c", wanted.clone()), (vec![(([2; 32], 20_001), s)], vec![(([1; 32], 20_000), Source::Dm)]), "new tags go first and kept ones are confirmed after the drops");
        let mut stale = no_longer_wanted("fcm:c", 20_000, &wanted, all);
        stale.sort();
        assert_eq!(stale, vec![([5; 32], 20_000), ([7; 32], 20_000)], "tags from before a restart are dropped when no longer wanted");
        assert_eq!(no_longer_wanted("fcm:c", 20_000, &wanted, |source| *source != s), vec![([7; 32], 20_000)], "a server not synced yet keeps its tags");
        assert!(no_longer_wanted("fcm:c", 20_000, &wanted, |source| *source != Source::Dm).iter().all(|tag| *tag != ([7; 32], 20_000)), "so do direct messages until they are");
        note("fcm:c", |r| {
            r.tags.insert(([2; 32], 20_001), s);
            r.earlier.remove(&([5; 32], 20_000));
            r.earlier.remove(&([7; 32], 20_000));
        });
        let mut kept: Vec<(Tag, Source)> = kept_now("fcm:c", 20_000).unwrap().tags.into_iter().map(|(tag, day, source)| ((tag.0, day), source)).collect();
        kept.sort_by_key(|(tag, _)| *tag);
        assert_eq!(kept, vec![(([1; 32], 20_000), Source::Dm), (([2; 32], 20_001), s)], "what is kept for the next run holds both, without past days");
        assert!(kept_now("fcm:a", 20_000).is_none());
        let saved = serde_json::to_string(&kept_now("fcm:c", 20_000).unwrap()).unwrap();
        let back: Kept = serde_json::from_str(&saved).unwrap();
        assert_eq!(back.tags.len(), 2);

        let unwanted = vec![([8; 32], 20_000), ([1; 32], 20_000), ([9; 32], 19_999)];
        assert!(to_sweep("fcm:c", 20_000, &wanted, unwanted.clone()).is_empty(), "a run that kept its tags drops only what it kept");
        begin("fcm:d", base, || None);
        assert_eq!(to_sweep("fcm:d", 20_000, &wanted, unwanted.clone()), vec![([8; 32], 20_000)], "tags the settings rule out are dropped when no run remembers them");
        note("fcm:d", |r| {
            r.swept.insert(([8; 32], 20_000));
        });
        assert!(to_sweep("fcm:d", 20_000, &wanted, unwanted).is_empty(), "and only once");
    }
}
