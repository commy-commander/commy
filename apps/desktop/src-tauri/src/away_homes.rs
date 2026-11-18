use crate::lock_or_recover::LockOrRecover;
use std::sync::Mutex;
use std::time::Duration;

use commy_core::UserProfile;
use commy_view::away::{self, HomeBook};
use tauri::{AppHandle, Manager};

use crate::state::App;

const ASK_WAIT: Duration = Duration::from_secs(15);

static BOOK: Mutex<Option<HomeBook>> = Mutex::new(None);

fn with_book<T>(look: impl FnOnce(&mut HomeBook) -> T) -> T {
    let mut book = BOOK.lock_or_recover();
    look(book.get_or_insert_with(HomeBook::default))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn own_home(app: &App) -> Option<String> {
    app.account.lock_or_recover().signed_in_server.clone()
}

pub fn away_home(app: &App, profile: &UserProfile) -> Option<String> {
    let mine = own_home(app)?;
    away::away_home(&mine, profile).filter(|home| !crate::server_guard::blocks_origin(home))
}

pub fn known_sync(home: &str) -> Option<String> {
    with_book(|book| book.url(home).map(str::to_string))
}

async fn ask(http: &reqwest::Client, home: &str) -> Option<String> {
    let response = http.get(away::discovery_url(home)).timeout(ASK_WAIT).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let doc = response.text().await.ok()?;
    away::sync_url_of(home, &doc)
}

async fn learn(http: &reqwest::Client, home: &str) -> bool {
    let url = ask(http, home).await;
    if url.is_none() {
        tracing::info!("could not find the sync service of {home}");
    }
    with_book(|book| book.learn(home, url, now_ms()))
}

pub async fn sync_now(app: &App, home: &str) -> Option<String> {
    if let Some(url) = known_sync(home) {
        return Some(url);
    }
    if crate::server_guard::blocks_origin(home) {
        return None;
    }
    let asking = with_book(|book| book.to_ask([home], now_ms()));
    if !asking.is_empty() {
        learn(&app.http, home).await;
    }
    known_sync(home)
}

pub fn look_up(handle: &AppHandle, homes: Vec<String>) {
    let due = with_book(|book| book.to_ask(homes.iter().map(String::as_str), now_ms()));
    if due.is_empty() {
        return;
    }
    let handle = handle.clone();
    crate::account_session::spawn(async move {
        let http = handle.state::<App>().http.clone();
        let mut changed = false;
        for home in due {
            changed |= learn(&http, &home).await;
        }
        if changed {
            crate::dm_sync::refresh(&handle);
            crate::community_sync::refresh_soon(&handle);
        }
    });
}

pub fn forget() {
    *BOOK.lock_or_recover() = None;
}
