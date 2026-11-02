use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use commy_view::links::{self, Blocklist, LinkCheck};
use reqwest::header::{ETAG, IF_NONE_MATCH};
use tauri::{AppHandle, Manager};

use crate::state::App;

pub const LIST_FILE: &str = "link-blocklist.txt";
const LIST_PATH: &str = "/api/auth/commy/link-blocklist";
const FIRST_REFRESH_DELAY: Duration = Duration::from_secs(20);
const REFRESH_EVERY: Duration = Duration::from_secs(12 * 60 * 60);
const RETRY_AFTER: Duration = Duration::from_secs(30 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_LIST_BYTES: usize = 16 * 1024 * 1024;
const MAX_TRUSTED_DOMAINS: usize = 500;

static BLOCKLIST: RwLock<Option<Arc<Blocklist>>> = RwLock::new(None);
static LIST_ETAG: Mutex<Option<String>> = Mutex::new(None);

fn current() -> Arc<Blocklist> {
    BLOCKLIST.read().unwrap().clone().unwrap_or_default()
}

fn replace(list: Blocklist) {
    *BLOCKLIST.write().unwrap() = Some(Arc::new(list));
}

fn list_file(dir: &Path) -> PathBuf {
    dir.join(LIST_FILE)
}

pub fn start(handle: &AppHandle, dir: &Path) {
    if let Ok(saved) = std::fs::read_to_string(list_file(dir)) {
        replace(Blocklist::parse(&saved));
    }
    let handle = handle.clone();
    let dir = dir.to_path_buf();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_REFRESH_DELAY).await;
        loop {
            let wait = match refresh(&handle, &dir).await {
                Ok(()) => REFRESH_EVERY,
                Err(why) => {
                    tracing::debug!("link safety list not refreshed: {why}");
                    RETRY_AFTER
                }
            };
            tokio::time::sleep(wait).await;
        }
    });
}

async fn refresh(handle: &AppHandle, dir: &Path) -> Result<(), String> {
    let app = handle.state::<App>();
    let credentials = app.account.lock().unwrap().credentials("link_blocklist");
    let Some((server, _)) = credentials else { return Err("no account server offers it".into()) };
    let mut request = app.http.get(format!("{server}{LIST_PATH}")).timeout(REQUEST_TIMEOUT);
    if let Some(etag) = LIST_ETAG.lock().unwrap().clone() {
        request = request.header(IF_NONE_MATCH, etag);
    }
    let res = request.send().await.map_err(|e| e.to_string())?;
    if res.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(());
    }
    if !res.status().is_success() {
        return Err(format!("answered {}", res.status()));
    }
    if res.content_length().unwrap_or(0) as usize > MAX_LIST_BYTES {
        return Err("list too large".into());
    }
    let etag = res.headers().get(ETAG).and_then(|v| v.to_str().ok()).map(str::to_string);
    let body = res.bytes().await.map_err(|e| e.to_string())?;
    if body.len() > MAX_LIST_BYTES {
        return Err("list too large".into());
    }
    let text = String::from_utf8_lossy(&body);
    let list = Blocklist::parse(&text);
    if list.is_empty() {
        return Err("empty list".into());
    }
    std::fs::write(list_file(dir), text.as_bytes()).map_err(|e| e.to_string())?;
    replace(list);
    *LIST_ETAG.lock().unwrap() = etag;
    Ok(())
}

#[tauri::command]
pub fn check_link(url: String, label: Option<String>, trusted: Option<Vec<String>>) -> Result<LinkCheck, String> {
    let mut trusted = trusted.unwrap_or_default();
    trusted.truncate(MAX_TRUSTED_DOMAINS);
    links::check_link(&url, label.as_deref(), &trusted, &current()).ok_or_else(|| "only http and https links can be opened".to_string())
}
