use std::collections::HashMap;

use commy_core::ServerId;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::account::host_of;
use crate::invite_link::{self, Expiry, InviteLink};
use crate::state::App;

pub const CATEGORIES: &[&str] = &["gaming", "music", "art", "tech", "science", "education", "friends", "languages", "sports", "other"];
pub const MEMBER_BUCKETS: &[u32] = &[10, 25, 50, 100, 250, 500, 1000];
pub const NAME_MAX: usize = 48;
pub const DESCRIPTION_MAX: usize = 300;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    pub origin: String,
    pub code: String,
    pub link: String,
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
    let link = permanent_link(&app, &handle, &server_id, sid, &origin).await?;
    let body = serde_json::json!({
        "code": link.code,
        "key": link.key.clone().unwrap_or_default(),
        "name": name,
        "description": description,
        "category": category,
        "members": members,
    });
    let request = app.http.post(format!("{origin}/api/auth/commy/directory/publish")).json(&body).bearer_auth(token);
    let reply: PublishResponse = invite_link::account_call(request, &origin).await?;
    let listing = Listing {
        origin: origin.clone(),
        code: reply.code,
        link: link.url(),
        category,
        description,
        members,
        listed_at: chrono::Utc::now().timestamp_millis(),
    };
    app.listings.lock().unwrap().insert(sid, listing.clone());
    app.persist_listings();
    Ok(listing)
}

#[tauri::command]
pub async fn unlist_server(app: tauri::State<'_, App>, server_id: String) -> Result<(), String> {
    let sid: ServerId = server_id.parse().map_err(|_| "unknown server".to_string())?;
    crate::require_manage_server(&app, sid, "change this server's Discover listing")?;
    let listing = app.listings.lock().unwrap().get(&sid).cloned().ok_or("This server was not listed from this device")?;
    let token = signed_in_token(&app)?;
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
    let reply: ListResponse = invite_link::account_call(app.http.get(url), &origin).await?;
    Ok(DirectoryPage { entries: entries_from(&origin, reply.listings), next_cursor: reply.next_cursor, host: host_of(&origin) })
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
}
