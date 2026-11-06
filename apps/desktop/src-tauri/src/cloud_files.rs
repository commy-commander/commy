use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use commy_core::file_store::{self as store, Roster, SignedRoster, StoreInfo};
use commy_core::Identity;
use commy_net::node::is_blob_of;
use commy_sync::file::fits_plain;
use commy_sync::FileKey;

const ASK_WAIT: Duration = Duration::from_secs(5);
const FETCH_WAIT: Duration = Duration::from_secs(90);
const STORE_WAIT: Duration = Duration::from_secs(180);
const ROSTER_WAIT: Duration = Duration::from_secs(30);
const ASK_AGAIN_AFTER: Duration = Duration::from_secs(10 * 60);
const MAX_OFFERED: usize = 4096;
const MAX_ANSWERS: usize = 256;
pub const NOT_FILE_STORAGE: &str = "That address did not answer as Commy file storage";
pub const OLD_WORKER: &str = "That storage runs an older Commy worker. Deploy the latest one, then try again";
pub const NEEDS_SETUP_CODE: &str = "Enter the setup code you gave the worker on Cloudflare";
pub const WRONG_SETUP_CODE: &str = "The worker did not accept that setup code";
pub const SETUP_FAILED: &str = "The worker did not take this server's member list";

static ANSWERS: Mutex<Option<HashMap<String, (bool, Instant)>>> = Mutex::new(None);
static OFFERED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn remembered(base: &str, now: Instant) -> Option<bool> {
    let answers = ANSWERS.lock().unwrap();
    answers.as_ref()?.get(base).filter(|(_, at)| now.saturating_duration_since(*at) < ASK_AGAIN_AFTER).map(|(on, _)| *on)
}

fn remember(base: &str, on: bool) {
    let mut guard = ANSWERS.lock().unwrap();
    let answers = guard.get_or_insert_with(HashMap::new);
    if answers.len() >= MAX_ANSWERS {
        answers.clear();
    }
    answers.insert(base.to_string(), (on, Instant::now()));
}

pub fn forget_answer(base: &str) {
    if let Some(answers) = ANSWERS.lock().unwrap().as_mut() {
        answers.remove(base);
    }
}

fn is_old_worker(body: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(body).ok().and_then(|info| info.get("commy_files")?.as_u64()) == Some(1)
}

pub async fn ask(http: &reqwest::Client, base: &str) -> Result<StoreInfo, String> {
    let response = http.get(base).timeout(ASK_WAIT).send().await.map_err(|_| NOT_FILE_STORAGE.to_string())?;
    if !response.status().is_success() {
        return Err(NOT_FILE_STORAGE.into());
    }
    let body = response.bytes().await.map_err(|_| NOT_FILE_STORAGE.to_string())?;
    if is_old_worker(&body) {
        return Err(OLD_WORKER.into());
    }
    store::answers_as_storage(&body).ok_or_else(|| NOT_FILE_STORAGE.into())
}

async fn kept_at(http: &reqwest::Client, base: &str) -> bool {
    if let Some(on) = remembered(base, Instant::now()) {
        return on;
    }
    let on = ask(http, base).await.is_ok_and(|info| info.paired);
    remember(base, on);
    on
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn signed(request: reqwest::RequestBuilder, identity: &Identity, method: &str, target: &str, length: u64) -> reqwest::RequestBuilder {
    store::signed_headers(identity, method, target, now_ms(), length).into_iter().fold(request, |request, (name, value)| request.header(name, value))
}

pub fn opened(key: &FileKey, blob: &str, body: &[u8], max: u64) -> Option<Vec<u8>> {
    let plain = key.open(body).ok()?;
    (plain.len() as u64 <= max && is_blob_of(blob, &plain)).then_some(plain)
}

pub enum Fetched {
    Found(Vec<u8>),
    NotKept,
    Off,
}

pub async fn fetch(http: &reqwest::Client, base: Option<&str>, identity: Option<&Identity>, blob: &str, max: u64) -> Fetched {
    let (Some(base), Some(identity)) = (base, identity) else { return Fetched::Off };
    if !kept_at(http, base).await {
        return Fetched::Off;
    }
    let key = FileKey::for_blob(blob);
    let id = key.id_hex();
    let fetched = async {
        let request = signed(http.get(format!("{base}/{id}")), identity, "GET", &id, 0);
        let response = request.timeout(FETCH_WAIT).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.bytes().await.ok()
    }
    .await;
    match fetched.and_then(|body| opened(&key, blob, &body, max)) {
        Some(plain) => Fetched::Found(plain),
        None => Fetched::NotKept,
    }
}

fn first_offer(base: &str, blob: &str) -> bool {
    let mut guard = OFFERED.lock().unwrap();
    let offered = guard.get_or_insert_with(HashSet::new);
    if offered.len() >= MAX_OFFERED {
        offered.clear();
    }
    offered.insert(format!("{base} {blob}"))
}

fn withdraw_offer(base: &str, blob: &str) {
    if let Some(offered) = OFFERED.lock().unwrap().as_mut() {
        offered.remove(&format!("{base} {blob}"));
    }
}

pub fn worth_offering(blob: &str, bytes: &[u8]) -> bool {
    !bytes.is_empty() && fits_plain(bytes.len()) && is_blob_of(blob, bytes)
}

pub fn offer(http: &reqwest::Client, base: Option<String>, identity: Option<Identity>, blob: &str, bytes: Vec<u8>) {
    let (Some(base), Some(identity)) = (base, identity) else { return };
    if !worth_offering(blob, &bytes) || !first_offer(&base, blob) {
        return;
    }
    let (http, blob) = (http.clone(), blob.to_string());
    tauri::async_runtime::spawn(async move {
        if !hand_over(&http, &base, &identity, &blob, bytes).await {
            withdraw_offer(&base, &blob);
        }
    });
}

async fn hand_over(http: &reqwest::Client, base: &str, identity: &Identity, blob: &str, bytes: Vec<u8>) -> bool {
    if !kept_at(http, base).await {
        return false;
    }
    let key = FileKey::for_blob(blob);
    let id = key.id_hex();
    let url = format!("{base}/{id}");
    let head = signed(http.head(&url), identity, "HEAD", &id, 0);
    if head.timeout(ASK_WAIT).send().await.is_ok_and(|r| r.status().is_success()) {
        return true;
    }
    let Ok(Ok(sealed)) = tauri::async_runtime::spawn_blocking(move || key.seal(&bytes)).await else { return false };
    let put = signed(http.put(&url), identity, "PUT", &id, sealed.len() as u64);
    match put.timeout(STORE_WAIT).body(sealed).send().await {
        Ok(response) if response.status().is_success() => true,
        Ok(response) => {
            tracing::debug!("the server's file storage did not keep a file: {}", response.status());
            false
        }
        Err(_) => {
            tracing::debug!("could not reach the server's file storage");
            false
        }
    }
}

fn roster_body(signed: &SignedRoster) -> Vec<u8> {
    serde_json::to_vec(signed).expect("a signed roster always serializes")
}

pub async fn pair(http: &reqwest::Client, base: &str, code: &str, identity: &Identity, roster: &Roster) -> Result<(), String> {
    let body = roster_body(&store::sign_roster(identity, roster));
    let response = http
        .post(format!("{base}/{}", store::PAIR_PATH))
        .header(store::SETUP_HEADER, code.trim())
        .header("content-type", "application/json")
        .timeout(ROSTER_WAIT)
        .body(body)
        .send()
        .await
        .map_err(|_| NOT_FILE_STORAGE.to_string())?;
    forget_answer(base);
    match response.status().as_u16() {
        200..=299 => Ok(()),
        403 => Err(WRONG_SETUP_CODE.into()),
        _ => Err(SETUP_FAILED.into()),
    }
}

pub async fn kept_roster(http: &reqwest::Client, base: &str, identity: &Identity) -> Option<Roster> {
    let request = signed(http.get(format!("{base}/{}", store::ROSTER_PATH)), identity, "GET", store::ROSTER_TARGET, 0);
    let response = request.timeout(ROSTER_WAIT).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    serde_json::from_slice(&response.bytes().await.ok()?).ok()
}

pub async fn send_roster(http: &reqwest::Client, base: &str, identity: &Identity, mine: &Roster) -> Result<bool, String> {
    let kept = kept_roster(http, base, identity).await.ok_or_else(|| SETUP_FAILED.to_string())?;
    let Some(next) = store::roster_to_send(mine, &kept, &identity.public().to_hex()) else { return Ok(false) };
    let response = http
        .put(format!("{base}/{}", store::ROSTER_PATH))
        .header("content-type", "application/json")
        .timeout(ROSTER_WAIT)
        .body(roster_body(&store::sign_roster(identity, &next)))
        .send()
        .await
        .map_err(|_| NOT_FILE_STORAGE.to_string())?;
    if response.status().is_success() {
        Ok(true)
    } else {
        Err(format!("{SETUP_FAILED}: {}", response.status()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use commy_net::node::blob_hash as blob_of;

    #[test]
    fn a_fetched_file_counts_only_when_it_is_the_one_referenced() {
        let real = b"the real pasted picture".to_vec();
        let blob = blob_of(&real);
        let key = FileKey::for_blob(&blob);
        assert_eq!(opened(&key, &blob, &key.seal(&real).unwrap(), 1024), Some(real.clone()));
        assert_eq!(opened(&key, &blob, &key.seal(b"something else").unwrap(), 1024), None, "nobody can swap in another file");
        assert_eq!(opened(&key, &blob, &key.seal(&real).unwrap(), 4), None, "size limits still hold");
        assert_eq!(opened(&key, &blob, b"garbage", 1024), None);
    }

    #[test]
    fn only_files_that_fit_and_match_are_offered() {
        let bytes = b"a voice message".to_vec();
        let blob = blob_of(&bytes);
        assert!(worth_offering(&blob, &bytes));
        assert!(!worth_offering(&blob, b"other bytes"));
        assert!(!worth_offering(&blob_of(b""), b""));
    }

    #[test]
    fn each_file_is_offered_once_per_storage_until_an_offer_fails() {
        let blob = blob_of(b"offered file");
        assert!(first_offer("https://a.workers.dev", &blob));
        assert!(!first_offer("https://a.workers.dev", &blob));
        assert!(first_offer("https://b.workers.dev", &blob), "another server's storage is asked on its own");
        withdraw_offer("https://a.workers.dev", &blob);
        assert!(first_offer("https://a.workers.dev", &blob));
    }

    #[test]
    fn storage_is_asked_again_after_a_while() {
        let now = Instant::now();
        remember("https://c.workers.dev", true);
        assert_eq!(remembered("https://c.workers.dev", now), Some(true));
        assert_eq!(remembered("https://d.workers.dev", now), None);
        assert_eq!(remembered("https://c.workers.dev", now + ASK_AGAIN_AFTER + Duration::from_secs(1)), None);
    }

    #[test]
    fn only_a_current_commy_storage_answer_counts() {
        assert!(is_old_worker(br#"{"commy_files":1,"max_bytes":26214400}"#));
        assert!(!is_old_worker(br#"{"commy_files":2,"max_bytes":1}"#));
        assert_eq!(store::answers_as_storage(br#"{"max_bytes":1}"#), None);
        assert_eq!(store::answers_as_storage(b"<html>"), None);
    }
}
