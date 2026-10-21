use std::collections::HashMap;
use std::time::{Duration, Instant};

use commy_core::ServerId;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::account::host_of;
use crate::invite_link::{self, Expiry, InviteLink};
use crate::state::App;

pub const CATEGORIES: &[&str] = &["gaming", "music", "art", "tech", "science", "education", "friends", "languages", "sports", "other"];
pub const MEMBER_BUCKETS: &[u32] = &[10, 25, 50, 100, 250, 500, 1000];
pub const NAME_MAX: usize = 48;
pub const DESCRIPTION_MAX: usize = 300;
const REFRESH_EVERY: Duration = Duration::from_secs(300);
const RETRY_AFTER: Duration = Duration::from_secs(600);

static LISTING_CHANGES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    pub origin: String,
    pub code: String,
    pub link: String,
    #[serde(default)]
    pub name: String,
    pub category: String,
    pub description: String,
    pub members: u32,
    pub listed_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DirectoryEntry {
    pub code: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub members: u32,
    pub link: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DirectoryPage {
    pub entries: Vec<DirectoryEntry>,
    pub next_cursor: Option<String>,
    pub host: String,
    pub available: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListResponse {
    listings: Vec<ListedRow>,
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
struct ListedRow {
    code: String,
    key: String,
    name: String,
    #[serde(default)]
    description: String,
    category: String,
    members: u32,
}

#[derive(Deserialize)]
struct PublishResponse {
    code: String,
}

pub fn member_bucket(count: usize) -> u32 {
    let mut bucket = MEMBER_BUCKETS[0];
    for &b in MEMBER_BUCKETS {
        if count >= b as usize {
            bucket = b;
        }
    }
    bucket
}

pub fn clean_category(value: &str) -> Result<String, String> {
    let category = value.trim().to_ascii_lowercase();
    if CATEGORIES.contains(&category.as_str()) {
        Ok(category)
    } else {
        Err("Pick a category from the list".into())
    }
}

pub fn clean_description(value: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut blank_lines = 0;
    for line in value.replace("\r\n", "\n").lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.is_empty() {
            blank_lines += 1;
            if blank_lines > 1 {
                continue;
            }
        } else {
            blank_lines = 0;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&words.join(" "));
    }
    let description = out.trim().to_string();
    if description.chars().any(|c| c.is_control() && c != '\n') {
        return Err("The description holds characters that cannot be shown".into());
    }
    if description.chars().count() > DESCRIPTION_MAX {
        return Err(format!("The description can hold up to {DESCRIPTION_MAX} characters"));
    }
    Ok(description)
}

pub fn listing_name(server_name: &str) -> String {
    let flat: Vec<&str> = server_name.split_whitespace().collect();
    let name: String = flat.join(" ").chars().filter(|c| !c.is_control()).take(NAME_MAX).collect();
    if name.trim().is_empty() {
        "Server".into()
    } else {
        name.trim().to_string()
    }
}

pub fn joinable_link(origin: &str, code: &str, key: &str) -> String {
    InviteLink { origin: origin.to_string(), code: code.to_string(), key: Some(key.to_string()) }.url()
}

fn entries_from(origin: &str, rows: Vec<ListedRow>) -> Vec<DirectoryEntry> {
    rows.into_iter()
        .filter(|row| invite_link::parse(&joinable_link(origin, &row.code, &row.key)).is_some())
        .map(|row| DirectoryEntry {
            link: joinable_link(origin, &row.code, &row.key),
            code: row.code,
            name: row.name.chars().filter(|c| !c.is_control()).take(NAME_MAX).collect(),
            description: row.description.chars().take(DESCRIPTION_MAX).collect(),
            category: if CATEGORIES.contains(&row.category.as_str()) { row.category } else { "other".into() },
            members: member_bucket(row.members as usize),
        })
        .collect()
}

fn signed_in_token(app: &App) -> Result<String, String> {
    crate::account::require_account(app)?;
    invite_link::signed_in_token(app).ok_or_else(|| "Sign in to your Commy account first".to_string())
}

fn reusable_link(app: &App, sid: ServerId, origin: &str) -> Option<String> {
    let listing = app.listings.lock().unwrap().get(&sid).cloned()?;
    if listing.origin != origin {
        return None;
    }
    let link = invite_link::parse(&listing.link)?;
    let servers = app.servers.lock().unwrap();
    servers.get(&sid)?.tracked_invite(&link.origin, &link.code)?;
    Some(listing.link)
}

async fn permanent_link(app: &App, handle: &AppHandle, server_id: &str, sid: ServerId, origin: &str) -> Result<InviteLink, String> {
    let url = match reusable_link(app, sid, origin) {
        Some(url) => url,
        None => invite_link::create(app, handle, server_id, Expiry::Never, None).await?.url,
    };
    invite_link::parse(&url)
        .filter(|link| link.origin == origin && link.key.is_some())
        .ok_or_else(|| "The invite server sent an unexpected link".to_string())
}

#[tauri::command]
pub async fn publish_listing(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    server_id: String,
    description: String,
    category: String,
) -> Result<Listing, String> {
    let sid: ServerId = server_id.parse().map_err(|_| "unknown server".to_string())?;
    crate::require_manage_server(&app, sid, "list this server in Discover")?;
    let category = clean_category(&category)?;
    let description = clean_description(&description)?;
    let (name, members, public) = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        (listing_name(&state.server.name), member_bucket(state.server.members.len()), state.server.public)
    };
    if !public {
        return Err("Make the server public before listing it in Discover".into());
    }
    let token = signed_in_token(&app)?;
    let origin = crate::account::effective_server();
    require_directory(&app, &origin).await?;
    let link = permanent_link(&app, &handle, &server_id, sid, &origin).await?;
    let mut listing = Listing {
        origin,
        code: link.code.clone(),
        link: link.url(),
        name,
        category,
        description,
        members,
        listed_at: chrono::Utc::now().timestamp_millis(),
    };
    let _serial = LISTING_CHANGES.lock().await;
    listing.code = send_listing(&app, &listing, &token).await?;
    app.listings.lock().unwrap().insert(sid, listing.clone());
    app.persist_listings();
    Ok(listing)
}

async fn send_listing(app: &App, listing: &Listing, token: &str) -> Result<String, String> {
    let link = invite_link::parse(&listing.link).ok_or("This listing lost its invite link")?;
    let body = serde_json::json!({
        "code": link.code,
        "key": link.key.unwrap_or_default(),
        "name": listing.name,
        "description": listing.description,
        "category": listing.category,
        "members": listing.members,
    });
    let request = app.http.post(format!("{}/api/auth/commy/directory/publish", listing.origin)).json(&body).bearer_auth(token);
    let reply: PublishResponse = invite_link::account_call(request, &listing.origin).await?;
    Ok(reply.code)
}

pub fn server_changed(app: &App, sid: ServerId) {
    if app.listings.lock().unwrap().contains_key(&sid) {
        app.listing_wake.notify_one();
    }
}

pub async fn keep_listings_current(handle: AppHandle) {
    let mut failed = HashMap::new();
    loop {
        let app = handle.state::<App>();
        refresh_listings(&app, &mut failed).await;
        tokio::select! {
            _ = tokio::time::sleep(REFRESH_EVERY) => {}
            _ = app.listing_wake.notified() => {}
        }
    }
}

async fn refresh_listings(app: &App, failed: &mut HashMap<ServerId, Instant>) {
    let origin = crate::account::effective_server();
    let Some(token) = invite_link::signed_in_token(app) else { return };
    let _serial = LISTING_CHANGES.lock().await;
    let now = Instant::now();
    failed.retain(|_, at| now.duration_since(*at) < RETRY_AFTER);
    for (sid, listing) in outdated_listings(app, &origin) {
        if failed.contains_key(&sid) {
            continue;
        }
        match send_listing(app, &listing, &token).await {
            Ok(code) => store_refreshed(app, sid, Listing { code, ..listing }),
            Err(e) => {
                tracing::warn!("Discover listing not refreshed: {e}");
                failed.insert(sid, now);
            }
        }
    }
}

fn outdated_listings(app: &App, origin: &str) -> Vec<(ServerId, Listing)> {
    let listings = app.listings.lock().unwrap().clone();
    let servers = app.servers.lock().unwrap();
    listings
        .into_iter()
        .filter(|(_, listing)| listing.origin == origin)
        .filter_map(|(sid, listing)| {
            let server = &servers.get(&sid)?.server;
            let current = current_listing(&listing, &server.name, server.members.len());
            (server.public && current != listing).then_some((sid, current))
        })
        .collect()
}

fn current_listing(listing: &Listing, server_name: &str, member_count: usize) -> Listing {
    Listing { name: listing_name(server_name), members: member_bucket(member_count), ..listing.clone() }
}

fn store_refreshed(app: &App, sid: ServerId, refreshed: Listing) {
    if let Some(stored) = app.listings.lock().unwrap().get_mut(&sid) {
        *stored = refreshed;
    }
    app.persist_listings();
}

#[tauri::command]
pub async fn unlist_server(app: tauri::State<'_, App>, server_id: String) -> Result<(), String> {
    let sid: ServerId = server_id.parse().map_err(|_| "unknown server".to_string())?;
    crate::require_manage_server(&app, sid, "change this server's Discover listing")?;
    let token = signed_in_token(&app)?;
    let _serial = LISTING_CHANGES.lock().await;
    let listing = app.listings.lock().unwrap().get(&sid).cloned().ok_or("This server was not listed from this device")?;
    let request = app
        .http
        .post(format!("{}/api/auth/commy/directory/unlist", listing.origin))
        .json(&serde_json::json!({ "code": listing.code }))
        .bearer_auth(token);
    let res = request.send().await.map_err(|_| format!("Could not reach {}", host_of(&listing.origin)))?;
    if !res.status().is_success() && res.status() != reqwest::StatusCode::NOT_FOUND {
        return Err(invite_link::error_message(res).await);
    }
    app.listings.lock().unwrap().remove(&sid);
    app.persist_listings();
    Ok(())
}

#[tauri::command]
pub fn server_listing(app: tauri::State<'_, App>, server_id: String) -> Option<Listing> {
    let sid: ServerId = server_id.parse().ok()?;
    let listing = app.listings.lock().unwrap().get(&sid).cloned()?;
    (listing.origin == crate::account::effective_server()).then_some(listing)
}

#[tauri::command]
pub async fn browse_directory(
    app: tauri::State<'_, App>,
    q: Option<String>,
    category: Option<String>,
    cursor: Option<String>,
) -> Result<DirectoryPage, String> {
    let origin = crate::account::effective_server();
    let mut url = Url::parse(&format!("{origin}/api/auth/commy/directory/list")).map_err(|e| e.to_string())?;
    {
        let mut pairs = url.query_pairs_mut();
        if let Some(q) = q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
            pairs.append_pair("q", &q.chars().take(80).collect::<String>());
        }
        if let Some(category) = category.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
            pairs.append_pair("category", &clean_category(category)?);
        }
        if let Some(cursor) = cursor.as_deref().filter(|c| !c.is_empty() && c.chars().all(|ch| ch.is_ascii_digit())) {
            pairs.append_pair("cursor", cursor);
        }
    }
    let host = host_of(&origin);
    Ok(match fetch_list(&app, url, &origin).await? {
        Some(reply) => DirectoryPage { entries: entries_from(&origin, reply.listings), next_cursor: reply.next_cursor, host, available: true },
        None => DirectoryPage { entries: Vec::new(), next_cursor: None, host, available: false },
    })
}

async fn fetch_list(app: &App, url: Url, origin: &str) -> Result<Option<ListResponse>, String> {
    let res = app.http.get(url).send().await.map_err(|_| format!("Could not reach {}", host_of(origin)))?;
    if directory_missing(res.status()) {
        return Ok(None);
    }
    if !res.status().is_success() {
        return Err(invite_link::error_message(res).await);
    }
    Ok(res.json::<ListResponse>().await.ok())
}

async fn require_directory(app: &App, origin: &str) -> Result<(), String> {
    let url = Url::parse(&format!("{origin}/api/auth/commy/directory/list")).map_err(|e| e.to_string())?;
    match fetch_list(app, url, origin).await? {
        Some(_) => Ok(()),
        None => Err(format!("{} does not list public servers yet", host_of(origin))),
    }
}

fn directory_missing(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::METHOD_NOT_ALLOWED || status.is_redirection()
}

pub fn load(dir: &std::path::Path) -> HashMap<ServerId, Listing> {
    crate::persist::load_listings(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_round_down_to_fixed_sizes() {
        assert_eq!(member_bucket(0), 10);
        assert_eq!(member_bucket(9), 10);
        assert_eq!(member_bucket(37), 25);
        assert_eq!(member_bucket(250), 250);
        assert_eq!(member_bucket(999), 500);
        assert_eq!(member_bucket(5000), 1000);
    }

    #[test]
    fn categories_come_from_the_fixed_list() {
        assert_eq!(clean_category(" Tech "), Ok("tech".into()));
        assert!(clean_category("crypto").is_err());
        assert!(clean_category("").is_err());
    }

    #[test]
    fn descriptions_are_flattened_and_bounded() {
        assert_eq!(clean_description("  two   spaces \r\n\r\n\r\n\r\nmany lines  "), Ok("two spaces\n\nmany lines".into()));
        assert_eq!(clean_description(""), Ok(String::new()));
        assert_eq!(clean_description(&"x".repeat(DESCRIPTION_MAX)).map(|d| d.len()), Ok(DESCRIPTION_MAX));
        assert!(clean_description(&"x".repeat(DESCRIPTION_MAX + 1)).is_err());
        assert!(clean_description("bell\u{7}").is_err());
    }

    #[test]
    fn listing_names_follow_the_server_name() {
        assert_eq!(listing_name("  Rust   Hangout "), "Rust Hangout");
        assert_eq!(listing_name("   "), "Server");
        assert_eq!(listing_name(&"n".repeat(100)).chars().count(), NAME_MAX);
    }

    #[test]
    fn account_servers_without_a_directory_read_as_unavailable() {
        use reqwest::StatusCode;
        assert!(directory_missing(StatusCode::NOT_FOUND));
        assert!(directory_missing(StatusCode::METHOD_NOT_ALLOWED));
        assert!(directory_missing(StatusCode::FOUND));
        assert!(directory_missing(StatusCode::TEMPORARY_REDIRECT));
        assert!(!directory_missing(StatusCode::OK));
        assert!(!directory_missing(StatusCode::BAD_REQUEST));
        assert!(!directory_missing(StatusCode::INTERNAL_SERVER_ERROR));
    }

    #[test]
    fn directory_rows_become_joinable_links() {
        let key = "k".repeat(43);
        let rows = vec![
            ListedRow { code: "KQ7M3N4P".into(), key: key.clone(), name: "Rust Hangout".into(), description: "Crabs".into(), category: "tech".into(), members: 25 },
            ListedRow { code: "pixel-club".into(), key: "Zx9-_Zx9-_Zx9-_Zx9-_aA".into(), name: "Pixel Club".into(), description: String::new(), category: "weird".into(), members: 37 },
            ListedRow { code: "../bad".into(), key: key.clone(), name: "Bad".into(), description: String::new(), category: "art".into(), members: 10 },
        ];
        let entries = entries_from("https://commy.chat", rows);
        assert_eq!(entries.len(), 2, "rows that do not form a valid invite link are dropped");
        assert_eq!(entries[0].link, format!("https://commy.chat/i/KQ7M3N4P#{key}"));
        assert_eq!(entries[1].category, "other");
        assert_eq!(entries[1].members, 25);
        assert_eq!(entries[1].link, "https://commy.chat/i/pixel-club#Zx9-_Zx9-_Zx9-_Zx9-_aA");
    }

    fn listed(app: &App, name: &str, public: bool) -> (ServerId, Listing) {
        let owner = commy_core::UserProfile::new("owner");
        let mut server = commy_core::Server::new(name, owner.id);
        server.public = public;
        let sid = server.id;
        let listing = Listing {
            origin: "https://commy.chat".into(),
            code: "KQ7M3N4P".into(),
            link: format!("https://commy.chat/i/KQ7M3N4P#{}", "k".repeat(43)),
            name: listing_name(name),
            category: "tech".into(),
            description: "Crabs".into(),
            members: member_bucket(server.members.len()),
            listed_at: 1,
        };
        app.servers.lock().unwrap().insert(sid, commy_core::ServerState::new(server));
        app.listings.lock().unwrap().insert(sid, listing.clone());
        (sid, listing)
    }

    #[test]
    fn renamed_servers_are_republished_with_their_new_name() {
        let app = App::default();
        let (sid, listing) = listed(&app, "Rust Hangout", true);
        assert!(outdated_listings(&app, "https://commy.chat").is_empty());
        app.servers.lock().unwrap().get_mut(&sid).unwrap().server.name = "  Crab   Club ".into();
        let outdated = outdated_listings(&app, "https://commy.chat");
        assert_eq!(outdated, vec![(sid, Listing { name: "Crab Club".into(), ..listing })]);
        assert!(outdated_listings(&app, "https://staging.commy.chat").is_empty(), "listings on another account server wait");
    }

    #[test]
    fn listings_from_before_names_were_kept_are_refreshed_once() {
        let app = App::default();
        let (sid, _) = listed(&app, "Rust Hangout", true);
        app.listings.lock().unwrap().get_mut(&sid).unwrap().name.clear();
        let outdated = outdated_listings(&app, "https://commy.chat");
        assert_eq!(outdated[0].1.name, "Rust Hangout");
        store_refreshed(&app, sid, outdated[0].1.clone());
        assert!(outdated_listings(&app, "https://commy.chat").is_empty());
    }

    #[test]
    fn private_servers_are_left_alone() {
        let app = App::default();
        let (sid, _) = listed(&app, "Rust Hangout", false);
        app.servers.lock().unwrap().get_mut(&sid).unwrap().server.name = "Hidden".into();
        assert!(outdated_listings(&app, "https://commy.chat").is_empty());
    }

    #[test]
    fn stored_listings_without_a_name_still_load() {
        let old = r#"{"origin":"https://commy.chat","code":"KQ7M3N4P","link":"https://commy.chat/i/KQ7M3N4P","category":"tech","description":"","members":10,"listed_at":1}"#;
        let listing: Listing = serde_json::from_str(old).unwrap();
        assert_eq!(listing.name, "");
    }
}
