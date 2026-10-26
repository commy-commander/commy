use serde::{Deserialize, Serialize};

use crate::invite::{parse, InviteLink};

pub const LIST_PATH: &str = "/api/auth/commy/directory/list";
pub const PUBLISH_PATH: &str = "/api/auth/commy/directory/publish";
pub const UNLIST_PATH: &str = "/api/auth/commy/directory/unlist";

pub const CATEGORIES: &[&str] = &["gaming", "music", "art", "tech", "science", "education", "friends", "languages", "sports", "other"];
pub const MEMBER_BUCKETS: &[u32] = &[10, 25, 50, 100, 250, 500, 1000];
pub const NAME_MAX: usize = 48;
pub const DESCRIPTION_MAX: usize = 300;
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
pub struct ListResponse {
    pub listings: Vec<ListedRow>,
    pub next_cursor: Option<String>,
}

#[derive(Deserialize)]
pub struct ListedRow {
    pub code: String,
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub category: String,
    pub members: u32,
}

#[derive(Deserialize)]
pub struct PublishResponse {
    pub code: String,
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

pub fn entries_from(origin: &str, rows: Vec<ListedRow>) -> Vec<DirectoryEntry> {
    rows.into_iter()
        .filter(|row| parse(&joinable_link(origin, &row.code, &row.key)).is_some())
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

pub fn current_listing(listing: &Listing, server_name: &str, member_count: usize) -> Listing {
    Listing { name: listing_name(server_name), members: member_bucket(member_count), ..listing.clone() }
}

pub fn list_path(q: Option<&str>, category: Option<&str>, cursor: Option<&str>) -> Result<String, String> {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    if let Some(q) = q.map(str::trim).filter(|q| !q.is_empty()) {
        query.append_pair("q", &q.chars().take(80).collect::<String>());
    }
    if let Some(category) = category.map(str::trim).filter(|c| !c.is_empty()) {
        query.append_pair("category", &clean_category(category)?);
    }
    if let Some(cursor) = cursor.filter(|c| !c.is_empty() && c.chars().all(|ch| ch.is_ascii_digit())) {
        query.append_pair("cursor", cursor);
    }
    let query = query.finish();
    Ok(if query.is_empty() { LIST_PATH.to_string() } else { format!("{LIST_PATH}?{query}") })
}

pub fn publish_body(listing: &Listing) -> Option<serde_json::Value> {
    let link = parse(&listing.link)?;
    Some(serde_json::json!({
        "code": link.code,
        "key": link.key.unwrap_or_default(),
        "name": listing.name,
        "description": listing.description,
        "category": listing.category,
        "members": listing.members,
    }))
}

pub fn directory_missing(status: u16) -> bool {
    status == 404 || status == 405 || (300..400).contains(&status)
}

pub fn page(origin: &str, host: String, reply: Option<ListResponse>) -> DirectoryPage {
    match reply {
        Some(reply) => DirectoryPage { entries: entries_from(origin, reply.listings), next_cursor: reply.next_cursor, host, available: true },
        None => DirectoryPage { entries: Vec::new(), next_cursor: None, host, available: false },
    }
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
        assert!(directory_missing(404));
        assert!(directory_missing(405));
        assert!(directory_missing(302));
        assert!(directory_missing(307));
        assert!(!directory_missing(200));
        assert!(!directory_missing(400));
        assert!(!directory_missing(500));
    }

    #[test]
    fn list_queries_are_trimmed_checked_and_encoded() {
        assert_eq!(list_path(None, None, None).unwrap(), LIST_PATH);
        assert_eq!(list_path(Some("  rust & crabs "), Some("Tech"), Some("40")).unwrap(), format!("{LIST_PATH}?q=rust+%26+crabs&category=tech&cursor=40"));
        assert_eq!(list_path(Some(" "), None, Some("x1")).unwrap(), LIST_PATH);
        assert!(list_path(None, Some("crypto"), None).is_err());
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
