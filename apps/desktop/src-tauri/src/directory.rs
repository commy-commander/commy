use std::collections::HashMap;
use std::time::{Duration, Instant};

use commy_core::ServerId;
use reqwest::Url;
use tauri::{AppHandle, Manager};

use crate::account::host_of;
use crate::invite_link::{self, Expiry, InviteLink};
use crate::state::App;

pub use commy_net::directory::{clean_category, clean_description, current_listing, listing_name, member_bucket, DirectoryPage, Listing};
use commy_net::directory::{self as shared, ListResponse, PublishResponse};

const REFRESH_EVERY: Duration = Duration::from_secs(300);
const RETRY_AFTER: Duration = Duration::from_secs(600);

static LISTING_CHANGES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    let body = shared::publish_body(listing).ok_or("This listing lost its invite link")?;
    let request = app.http.post(format!("{}{}", listing.origin, shared::PUBLISH_PATH)).json(&body).bearer_auth(token);
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
        .post(format!("{}{}", listing.origin, shared::UNLIST_PATH))
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
    let path = shared::list_path(q.as_deref(), category.as_deref(), cursor.as_deref())?;
    let url = Url::parse(&format!("{origin}{path}")).map_err(|e| e.to_string())?;
    let host = host_of(&origin);
    Ok(shared::page(&origin, host, fetch_list(&app, url, &origin).await?))
}

async fn fetch_list(app: &App, url: Url, origin: &str) -> Result<Option<ListResponse>, String> {
    let res = app.http.get(url).send().await.map_err(|_| format!("Could not reach {}", host_of(origin)))?;
    if shared::directory_missing(res.status().as_u16()) {
        return Ok(None);
    }
    if !res.status().is_success() {
        return Err(invite_link::error_message(res).await);
    }
    Ok(res.json::<ListResponse>().await.ok())
}

async fn require_directory(app: &App, origin: &str) -> Result<(), String> {
    let url = Url::parse(&format!("{origin}{}", shared::LIST_PATH)).map_err(|e| e.to_string())?;
    match fetch_list(app, url, origin).await? {
        Some(_) => Ok(()),
        None => Err(format!("{} does not list public servers yet", host_of(origin))),
    }
}

pub fn load(dir: &std::path::Path) -> HashMap<ServerId, Listing> {
    crate::persist::load_listings(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

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
