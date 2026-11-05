use std::collections::HashMap;
use std::rc::Rc;

use chrono::Utc;
use commy_core::ServerId;
use commy_net::node::is_blob_of;
use commy_sync::file::fits_plain;
use commy_sync::FileKey;
use serde_json::Value;
use wasm_bindgen_futures::spawn_local;

use super::Session;

const ASK_WAIT_MS: u32 = 5_000;
const FETCH_WAIT_MS: u32 = 90_000;
const STORE_WAIT_MS: u32 = 180_000;
const ASK_AGAIN_AFTER_MS: i64 = 10 * 60 * 1000;
const MAX_ANSWERS: usize = 256;
const NOT_FILE_STORAGE: &str = "That address did not answer as Commy file storage";
const NOT_IN_BROWSER: &str = "The browser app reaches file storage on a workers.dev address; this one works in the desktop and phone apps";

#[derive(Default)]
pub(super) struct Answers {
    by_base: HashMap<String, (bool, i64)>,
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
}

pub(super) fn opened(key: &FileKey, blob: &str, body: &[u8], max: u64) -> Option<Vec<u8>> {
    let plain = key.open(body).ok()?;
    (plain.len() as u64 <= max && is_blob_of(blob, &plain)).then_some(plain)
}

pub(super) fn answers_as_storage(body: &[u8]) -> Option<u64> {
    let info: Value = serde_json::from_slice(body).ok()?;
    (info.get("commy_files")?.as_u64()? == 1).then(|| info.get("max_bytes").and_then(Value::as_u64)).flatten()
}

fn present(value: &wasm_bindgen::JsValue) -> bool {
    !value.is_null() && !value.is_undefined()
}

impl Session {
    pub(super) fn file_store_of(&self, server: Option<ServerId>) -> Option<String> {
        let server = server?;
        let client = self.client.borrow();
        let servers = client.servers.lock().unwrap();
        servers.get(&server)?.server.file_store.clone().filter(|url| commy_core::file_store::reaches_in_browser(url))
    }

    async fn ask_file_store(&self, base: &str) -> Option<u64> {
        let body = self.host.fetch_bytes_within(base, ASK_WAIT_MS).await.ok().filter(present)?;
        answers_as_storage(&js_sys::Uint8Array::new(&body).to_vec())
    }

    pub(super) async fn test_file_store(&self, address: &str) -> Result<Value, String> {
        let base = commy_core::file_store::clean_file_store(address).ok_or(commy_view::file_store::FILE_STORE_INVALID)?;
        if !commy_core::file_store::reaches_in_browser(&base) {
            return Err(NOT_IN_BROWSER.into());
        }
        let max = self.ask_file_store(&base).await.ok_or(NOT_FILE_STORAGE)?;
        Ok(Value::from(max))
    }

    async fn kept_at(&self, base: &str) -> bool {
        let now = Utc::now().timestamp_millis();
        if let Some(on) = self.files_answer.borrow().fresh(base, now) {
            return on;
        }
        let on = self.ask_file_store(base).await.is_some();
        self.files_answer.borrow_mut().remember(base, on, now);
        on
    }

    pub(super) async fn kept_file(&self, base: Option<&str>, blob: &str, max: u64) -> Option<Vec<u8>> {
        let base = base?;
        if !self.kept_at(base).await {
            return None;
        }
        let key = FileKey::for_blob(blob);
        let body = self.host.fetch_bytes_within(&format!("{base}/{}", key.id_hex()), FETCH_WAIT_MS).await.ok().filter(present)?;
        opened(&key, blob, &js_sys::Uint8Array::new(&body).to_vec(), max)
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
        if !self.kept_at(base).await {
            return false;
        }
        let key = FileKey::for_blob(blob);
        let url = format!("{base}/{}", key.id_hex());
        if self.host.has_bytes(&url, ASK_WAIT_MS).await.ok().and_then(|v| v.as_bool()).unwrap_or(false) {
            return true;
        }
        let Ok(sealed) = key.seal(bytes) else { return false };
        self.host.put_bytes_within(&url, &sealed, STORE_WAIT_MS).await.ok().and_then(|v| v.as_bool()).unwrap_or(false)
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
    fn only_a_commy_storage_answer_counts() {
        assert_eq!(answers_as_storage(br#"{"commy_files":1,"max_bytes":26214400}"#), Some(26_214_400));
        assert_eq!(answers_as_storage(br#"{"max_bytes":1}"#), None);
    }
}
