use std::collections::HashMap;
use std::rc::Rc;

use chrono::Utc;
use commy_core::file_store::{self as store, Roster, StoreInfo};
use commy_core::{Identity, ServerId};
use commy_net::node::is_blob_of;
use commy_sync::file::fits_plain;
use commy_sync::FileKey;
use serde_json::{json, Value};
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::spawn_local;

use super::Session;

const ASK_WAIT_MS: u32 = 5_000;
const FETCH_WAIT_MS: u32 = 90_000;
const STORE_WAIT_MS: u32 = 180_000;
const ROSTER_WAIT_MS: u32 = 30_000;
const ASK_AGAIN_AFTER_MS: i64 = 10 * 60 * 1000;
const ROSTER_CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(120);
const ROSTER_RECHECK_AFTER_MS: i64 = 30 * 60 * 1000;
const MAX_ANSWERS: usize = 256;
const NOT_FILE_STORAGE: &str = "That address did not answer as Commy file storage";
const OLD_WORKER: &str = "That storage runs an older Commy worker. Deploy the latest one, then try again";
const NEEDS_SETUP_CODE: &str = "Enter the setup code you gave the worker on Cloudflare";
const WRONG_SETUP_CODE: &str = "The worker did not accept that setup code";
const SETUP_FAILED: &str = "The worker did not take this server's member list";
const NOT_IN_BROWSER: &str = "The browser app reaches file storage on a workers.dev address; this one works in the desktop and phone apps";

#[derive(Default)]
pub(super) struct Answers {
    by_base: HashMap<String, (bool, i64)>,
    rosters: HashMap<(ServerId, String), (Roster, i64)>,
}

impl Answers {
    fn fresh(&self, base: &str, now: i64) -> Option<bool> {
        self.by_base.get(base).filter(|(_, at)| now - at < ASK_AGAIN_AFTER_MS).map(|(on, _)| *on)
    }

    fn remember(&mut self, base: &str, on: bool, now: i64) {
        if self.by_base.len() >= MAX_ANSWERS {
            self.by_base.clear();
        }
        self.by_base.insert(base.to_string(), (on, now));
    }

    fn roster_fresh(&self, server: ServerId, base: &str, roster: &Roster, now: i64) -> bool {
        self.rosters.get(&(server, base.to_string())).is_some_and(|(sent, at)| store::same_people(sent, roster) && now - at < ROSTER_RECHECK_AFTER_MS)
    }

    fn sent_roster(&mut self, server: ServerId, base: &str, roster: &Roster, now: i64) {
        self.rosters.insert((server, base.to_string()), (roster.clone(), now));
    }
}

pub(super) fn opened(key: &FileKey, blob: &str, body: &[u8], max: u64) -> Option<Vec<u8>> {
    let plain = key.open(body).ok()?;
    (plain.len() as u64 <= max && is_blob_of(blob, &plain)).then_some(plain)
}

fn is_old_worker(body: &[u8]) -> bool {
    serde_json::from_slice::<Value>(body).ok().and_then(|info| info.get("commy_files")?.as_u64()) == Some(1)
}

fn present(value: &JsValue) -> bool {
    !value.is_null() && !value.is_undefined()
}

struct Answer {
    status: u16,
    body: Option<Vec<u8>>,
}

impl Answer {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

fn answer_of(value: &JsValue) -> Answer {
    let field = |name: &str| js_sys::Reflect::get(value, &JsValue::from_str(name)).unwrap_or(JsValue::NULL);
    let status = field("status").as_f64().unwrap_or(0.0) as u16;
    let body = field("body");
    Answer { status, body: present(&body).then(|| js_sys::Uint8Array::new(&body).to_vec()) }
}

fn signed_headers(identity: &Identity, method: &str, target: &str, length: u64) -> Value {
    let headers = store::signed_headers(identity, method, target, Utc::now().timestamp_millis(), length);
    Value::Object(headers.into_iter().map(|(name, value)| (name.to_string(), Value::String(value))).collect())
}

impl Session {
    pub(super) fn file_store_of(&self, server: Option<ServerId>) -> Option<String> {
        let server = server?;
        let client = self.client.borrow();
        let servers = client.servers.lock().unwrap();
        servers.get(&server)?.server.file_store.clone().filter(|url| commy_core::file_store::reaches_in_browser(url))
    }

    fn storage_identity(&self) -> Option<Identity> {
        self.client.borrow().me.lock().unwrap().as_ref().map(|(_, identity)| identity.clone())
    }

    async fn call_storage(&self, method: &str, url: &str, headers: Value, body: &[u8], wait_ms: u32) -> Answer {
        match self.host.storage_request(method, url, &headers.to_string(), body, wait_ms).await {
            Ok(value) if present(&value) => answer_of(&value),
            _ => Answer { status: 0, body: None },
        }
    }

    async fn ask_file_store(&self, base: &str) -> Result<StoreInfo, String> {
        let body = self.host.fetch_bytes_within(base, ASK_WAIT_MS).await.ok().filter(present).ok_or(NOT_FILE_STORAGE)?;
        let body = js_sys::Uint8Array::new(&body).to_vec();
        if is_old_worker(&body) {
            return Err(OLD_WORKER.into());
        }
        store::answers_as_storage(&body).ok_or_else(|| NOT_FILE_STORAGE.into())
    }

    fn browser_base(address: &str) -> Result<String, String> {
        let base = commy_core::file_store::clean_file_store(address).ok_or(commy_view::file_store::FILE_STORE_INVALID)?;
        if !commy_core::file_store::reaches_in_browser(&base) {
            return Err(NOT_IN_BROWSER.into());
        }
        Ok(base)
    }

    pub(super) async fn test_file_store(&self, address: &str) -> Result<Value, String> {
        let base = Self::browser_base(address)?;
        let info = self.ask_file_store(&base).await?;
        Ok(json!({ "max_bytes": info.max_bytes, "paired": info.paired }))
    }

    pub(super) async fn pair_file_store(&self, server: ServerId, address: Option<&str>, setup_code: Option<&str>) -> Result<(), String> {
        let Some(address) = address.map(str::trim).filter(|a| !a.is_empty()) else { return Ok(()) };
        let base = Self::browser_base(address)?;
        let info = self.ask_file_store(&base).await?;
        let Some(code) = setup_code.map(str::trim).filter(|c| !c.is_empty()) else {
            return if info.paired { Ok(()) } else { Err(NEEDS_SETUP_CODE.into()) };
        };
        if !store::setup_code_ok(code) {
            return Err(WRONG_SETUP_CODE.into());
        }
        let identity = self.storage_identity().ok_or("Your servers are not unlocked yet")?;
        let roster = self.storage_roster(server).ok_or("the server owner's identity key is not known yet")?;
        let body = serde_json::to_vec(&store::sign_roster(&identity, &roster)).map_err(|e| e.to_string())?;
        let headers = json!({ store::SETUP_HEADER: code, "content-type": "application/json" });
        let answer = self.call_storage("POST", &format!("{base}/{}", store::PAIR_PATH), headers, &body, ROSTER_WAIT_MS).await;
        self.files_answer.borrow_mut().by_base.remove(&base);
        match answer.status {
            200..=299 => {
                self.files_answer.borrow_mut().sent_roster(server, &base, &roster, Utc::now().timestamp_millis());
                Ok(())
            }
            403 => Err(WRONG_SETUP_CODE.into()),
            0 => Err(NOT_FILE_STORAGE.into()),
            _ => Err(SETUP_FAILED.into()),
        }
    }

    fn storage_roster(&self, server: ServerId) -> Option<Roster> {
        let client = self.client.borrow();
        let servers = client.servers.lock().unwrap();
        store::roster_of(servers.get(&server)?, Utc::now().timestamp_millis())
    }

    async fn kept_at(&self, base: &str) -> bool {
        let now = Utc::now().timestamp_millis();
        if let Some(on) = self.files_answer.borrow().fresh(base, now) {
            return on;
        }
        let on = self.ask_file_store(base).await.is_ok_and(|info| info.paired);
        self.files_answer.borrow_mut().remember(base, on, now);
        on
    }

    pub(super) async fn kept_file(&self, base: Option<&str>, blob: &str, max: u64) -> Option<Vec<u8>> {
        let base = base?;
        let identity = self.storage_identity()?;
        if !self.kept_at(base).await {
            return None;
        }
        let key = FileKey::for_blob(blob);
        let id = key.id_hex();
        let answer = self.call_storage("GET", &format!("{base}/{id}"), signed_headers(&identity, "GET", &id, 0), &[], FETCH_WAIT_MS).await;
        if !answer.ok() {
            return None;
        }
        opened(&key, blob, &answer.body?, max)
    }

    pub(super) fn offer_file(self: &Rc<Self>, server: ServerId, blob: &str, bytes: Vec<u8>) {
        let Some(base) = self.file_store_of(Some(server)) else { return };
        if bytes.is_empty() || !fits_plain(bytes.len()) || !is_blob_of(blob, &bytes) {
            return;
        }
        let (this, blob) = (self.clone(), blob.to_string());
        spawn_local(async move {
            this.hand_over_file(&base, &blob, &bytes).await;
        });
    }

    async fn hand_over_file(&self, base: &str, blob: &str, bytes: &[u8]) -> bool {
        let Some(identity) = self.storage_identity() else { return false };
        if !self.kept_at(base).await {
            return false;
        }
        let key = FileKey::for_blob(blob);
        let id = key.id_hex();
        let url = format!("{base}/{id}");
        if self.call_storage("HEAD", &url, signed_headers(&identity, "HEAD", &id, 0), &[], ASK_WAIT_MS).await.ok() {
            return true;
        }
        let Ok(sealed) = key.seal(bytes) else { return false };
        let headers = signed_headers(&identity, "PUT", &id, sealed.len() as u64);
        self.call_storage("PUT", &url, headers, &sealed, STORE_WAIT_MS).await.ok()
    }

    fn storage_to_tend(&self, now: i64) -> Vec<(ServerId, String, Roster)> {
        let client = self.client.borrow();
        let Some(me) = client.me.lock().unwrap().as_ref().map(|(profile, _)| profile.id) else { return Vec::new() };
        let servers = client.servers.lock().unwrap();
        let answers = self.files_answer.borrow();
        servers
            .values()
            .filter(|state| store::can_manage_storage(state.server.permissions_for(me)))
            .filter_map(|state| {
                let base = state.server.file_store.clone().filter(|url| store::reaches_in_browser(url))?;
                Some((state.server.id, base, store::roster_of(state, now)?))
            })
            .filter(|(server, base, roster)| !answers.roster_fresh(*server, base, roster, now))
            .collect()
    }

    async fn send_roster(&self, base: &str, identity: &Identity, mine: &Roster) -> Result<(), String> {
        let url = format!("{base}/{}", store::ROSTER_PATH);
        let answer = self.call_storage("GET", &url, signed_headers(identity, "GET", store::ROSTER_TARGET, 0), &[], ROSTER_WAIT_MS).await;
        let kept: Roster = answer.ok().then_some(answer.body).flatten().and_then(|body| serde_json::from_slice(&body).ok()).ok_or(SETUP_FAILED)?;
        let Some(next) = store::roster_to_send(mine, &kept, &identity.public().to_hex()) else { return Ok(()) };
        let body = serde_json::to_vec(&store::sign_roster(identity, &next)).map_err(|e| e.to_string())?;
        let answer = self.call_storage("PUT", &url, json!({ "content-type": "application/json" }), &body, ROSTER_WAIT_MS).await;
        if answer.ok() {
            Ok(())
        } else {
            Err(SETUP_FAILED.into())
        }
    }

    pub(super) async fn tend_file_storage(self: Rc<Self>) {
        loop {
            n0_future::time::sleep(ROSTER_CHECK_EVERY).await;
            if self.sync.borrow().is_none() {
                return;
            }
            let Some(identity) = self.storage_identity() else { continue };
            let now = Utc::now().timestamp_millis();
            for (server, base, roster) in self.storage_to_tend(now) {
                if self.send_roster(&base, &identity, &roster).await.is_ok() {
                    self.files_answer.borrow_mut().sent_roster(server, &base, &roster, now);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_storage_answer_is_trusted_for_ten_minutes() {
        let mut answers = Answers::default();
        answers.remember("https://a.workers.dev", true, 1_000);
        assert_eq!(answers.fresh("https://a.workers.dev", 1_000 + ASK_AGAIN_AFTER_MS - 1), Some(true));
        assert_eq!(answers.fresh("https://a.workers.dev", 1_000 + ASK_AGAIN_AFTER_MS), None);
        assert_eq!(answers.fresh("https://b.workers.dev", 1_000), None);
    }

    #[test]
    fn a_kept_file_counts_only_when_it_is_the_one_referenced() {
        let real = b"pasted in the browser".to_vec();
        let blob = commy_net::node::blob_hash(&real);
        let key = FileKey::for_blob(&blob);
        assert_eq!(opened(&key, &blob, &key.seal(&real).unwrap(), 1024), Some(real.clone()));
        assert_eq!(opened(&key, &blob, &key.seal(b"swapped").unwrap(), 1024), None);
        assert_eq!(opened(&key, &blob, &key.seal(&real).unwrap(), 3), None);
    }

    #[test]
    fn an_older_worker_is_told_apart() {
        assert!(is_old_worker(br#"{"commy_files":1,"max_bytes":26214400}"#));
        assert!(!is_old_worker(br#"{"commy_files":2,"max_bytes":1,"paired":true}"#));
        assert_eq!(store::answers_as_storage(br#"{"max_bytes":1}"#), None);
    }

    #[test]
    fn a_sent_member_list_is_trusted_until_people_change_or_half_an_hour_passes() {
        let server = ServerId::new();
        let roster = Roster { owner: "o".into(), managers: vec![], members: vec!["o".into()], at: 1 };
        let mut answers = Answers::default();
        assert!(!answers.roster_fresh(server, "https://a.workers.dev", &roster, 10));
        answers.sent_roster(server, "https://a.workers.dev", &roster, 10);
        assert!(answers.roster_fresh(server, "https://a.workers.dev", &Roster { at: 99, ..roster.clone() }, 20));
        let joined = Roster { members: vec!["o".into(), "a".into()], ..roster.clone() };
        assert!(!answers.roster_fresh(server, "https://a.workers.dev", &joined, 20));
        assert!(!answers.roster_fresh(server, "https://a.workers.dev", &roster, 10 + ROSTER_RECHECK_AFTER_MS));
    }
}
