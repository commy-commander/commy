use commy_net::sealed::SealKey;
use serde::{Deserialize, Serialize};

pub const SESSION_PATH: &str = "/api/auth/get-session";
pub const SIGN_OUT_PATH: &str = "/api/auth/sign-out";
pub const DEVICES_PATH: &str = "/api/auth/commy/devices";
pub const LINK_CHALLENGE_PATH: &str = "/api/auth/commy/devices/challenge";
pub const LINK_REGISTER_PATH: &str = "/api/auth/commy/devices/register";
pub const LIST_ITEM: &str = "servers";
pub const VAULT_PATH: &str = "/api/auth/commy/vault?name=servers";
pub const VAULT_PUT_PATH: &str = "/api/auth/commy/vault/put";
pub const CONFLICT: u16 = 409;
pub const DISCOVERY_PATH: &str = "/.well-known/commy-auth.json";
pub const MOVED_PATH: &str = "/api/auth/commy/moved";
pub const DELETE_START_PATH: &str = "/api/auth/commy/account/delete/start";
pub const DELETE_FINISH_PATH: &str = "/api/auth/commy/account/delete/finish";

pub const DEVICE_SECRET_KEY: &str = "commy.device";
pub const ENTRY_KEY_KEY: &str = "commy.entry-key";
pub const PINS_KEY: &str = "commy.pins";
pub const REMINDERS_KEY: &str = "commy.reminders";
pub const LISTINGS_KEY: &str = "commy.listings";
pub const SAFETY_KEY: &str = "commy.safety";
pub const PERSONAL_KEY: &str = "commy.personal";
pub const NO_LIST_KEY: &str = "This browser has no server list key yet";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountUser {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
    #[serde(default, alias = "createdAt", skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub body: String,
}

impl Reply {
    pub fn parse(raw: &str) -> Option<Reply> {
        let (status, body) = raw.split_once('\n').unwrap_or((raw, ""));
        Some(Reply { status: status.trim().parse().ok()?, body: body.to_string() })
    }

    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

#[derive(Deserialize)]
struct SessionBody {
    user: AccountUser,
}

pub fn parse_session(reply: &Reply) -> Option<AccountUser> {
    if !reply.ok() {
        return None;
    }
    serde_json::from_str::<Option<SessionBody>>(&reply.body).ok().flatten().map(|s| s.user)
}

pub fn session_ended(reply: &Reply) -> bool {
    match reply.status {
        401 | 403 => true,
        200..=299 => matches!(serde_json::from_str::<Option<SessionBody>>(&reply.body), Ok(None)),
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Challenge {
    pub challenge: String,
    pub message: String,
}

pub fn parse_challenge(reply: &Reply, origin: &str, node_id: &str) -> Result<Challenge, String> {
    let challenge: Challenge = reply
        .ok()
        .then(|| serde_json::from_str(&reply.body).ok())
        .flatten()
        .ok_or("Could not link this browser to your account")?;
    let expected = format!("commy-device-link:v1\n{origin}\n{node_id}\n{}", challenge.challenge);
    if challenge.message != expected {
        return Err("The account server asked this browser to sign an unexpected message".into());
    }
    Ok(challenge)
}

pub fn other_devices(reply: &Reply, own_node: &str) -> Vec<String> {
    #[derive(Deserialize)]
    struct Device {
        #[serde(rename = "nodeId")]
        node_id: String,
    }
    #[derive(Deserialize)]
    struct Devices {
        devices: Vec<Device>,
    }
    if !reply.ok() {
        return Vec::new();
    }
    serde_json::from_str::<Devices>(&reply.body)
        .map(|d| d.devices.into_iter().map(|d| d.node_id.to_lowercase()).filter(|n| n != own_node).collect())
        .unwrap_or_default()
}

pub const KEY_NOT_HERE: &str = "This browser does not have the key to your servers yet. Open Commy on one of your other devices signed in to this account and press Try again, or paste the recovery code from Settings, Commy Account on that device.";

pub const MAX_USERNAME_BYTES: usize = 32;
const FALLBACK_USERNAME: &str = "Commy user";

pub fn username_for(account: &AccountUser) -> String {
    let from_email = account.email.split('@').next().unwrap_or_default();
    let name = [account.name.trim(), from_email.trim()].into_iter().find(|name| !name.is_empty()).unwrap_or(FALLBACK_USERNAME);
    let mut end = name.len().min(MAX_USERNAME_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].trim_end().to_string()
}

pub fn vault_version(reply: &Reply) -> Option<u64> {
    #[derive(Deserialize)]
    struct Item {
        version: u64,
    }
    reply.ok().then(|| serde_json::from_str::<Item>(&reply.body).ok().map(|item| item.version)).flatten()
}

pub fn vault_data(reply: &Reply) -> Result<Option<String>, String> {
    #[derive(Deserialize)]
    struct Item {
        data: Option<String>,
    }
    match reply.status {
        404 => Err("Your account server cannot keep a server list yet".into()),
        s if !(200..300).contains(&s) => Err("Could not fetch your server list".into()),
        _ => serde_json::from_str::<Item>(&reply.body)
            .map(|item| item.data)
            .map_err(|_| "Your account server sent an unreadable server list".into()),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
pub struct Features {
    #[serde(default)]
    pub gifs: bool,
    #[serde(default)]
    pub games: bool,
    #[serde(default)]
    pub emojis: bool,
    #[serde(default)]
    pub media: bool,
    #[serde(default)]
    pub stickers: bool,
}

pub fn features(reply: &Reply) -> Features {
    #[derive(Deserialize)]
    struct Discovery {
        #[serde(default)]
        features: Features,
    }
    if !reply.ok() {
        return Features::default();
    }
    serde_json::from_str::<Discovery>(&reply.body).map(|d| d.features).unwrap_or_default()
}

pub fn encode_component(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub fn search_path(kind: &str, params: &[(&str, String)]) -> String {
    let query: Vec<String> = params.iter().map(|(key, value)| format!("{key}={}", encode_component(value))).collect();
    format!("/api/auth/commy/{kind}/search?{}", query.join("&"))
}

pub fn search_reply<T: serde::de::DeserializeOwned>(reply: &Reply) -> Result<Option<T>, String> {
    match reply.status {
        200 => serde_json::from_str(&reply.body).map(Some).map_err(|_| "Your account server sent an unexpected reply".to_string()),
        404 => Ok(None),
        401 => Err("Sign in again to search".into()),
        429 => Err("Too many searches, try again in a minute".into()),
        503 => Err("Search is busy right now, try again in a minute".into()),
        _ => Err("Your account server could not search right now".into()),
    }
}

pub fn sync_url(reply: &Reply, origin: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Endpoints {
        sync: Option<String>,
    }
    #[derive(Deserialize)]
    struct Discovery {
        endpoints: Endpoints,
    }
    if !reply.ok() {
        return None;
    }
    let discovery: Discovery = serde_json::from_str(&reply.body).ok()?;
    commy_sync::endpoint::accepted_url(origin, discovery.endpoints.sync.as_deref())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredKey {
    account: String,
    key: String,
}

pub fn stored_entry_key(raw: Option<&str>, account: &str) -> Option<SealKey> {
    let stored: StoredKey = serde_json::from_str(raw?).ok()?;
    (stored.account == account).then(|| SealKey::from_text(&stored.key).ok()).flatten()
}

pub fn entry_key_record(account: &str, key: &SealKey) -> String {
    serde_json::to_string(&StoredKey { account: account.into(), key: key.to_text() }).expect("key record serializes")
}

pub fn pins_key(account: &str) -> String {
    format!("{PINS_KEY}.{account}")
}

pub fn listings_key(account: &str) -> String {
    format!("{LISTINGS_KEY}.{account}")
}

pub fn reminders_key(account: &str) -> String {
    format!("{REMINDERS_KEY}.{account}")
}

pub fn safety_key(account: &str) -> String {
    format!("{SAFETY_KEY}.{account}")
}

pub fn personal_key(account: &str) -> String {
    format!("{PERSONAL_KEY}.{account}")
}

pub fn device_secret_key(account: &str) -> String {
    format!("{DEVICE_SECRET_KEY}.{account}")
}

pub fn kept_for_account(account: &str) -> Vec<String> {
    vec![
        pins_key(account),
        listings_key(account),
        reminders_key(account),
        safety_key(account),
        personal_key(account),
        device_secret_key(account),
    ]
}

pub fn deletion_proof(code: Option<&str>, password: Option<&str>) -> serde_json::Value {
    let mut body = serde_json::Map::new();
    if let Some(code) = code.map(str::trim).filter(|c| !c.is_empty()) {
        body.insert("code".into(), code.into());
    }
    if let Some(password) = password.filter(|p| !p.is_empty()) {
        body.insert("password".into(), password.into());
    }
    serde_json::Value::Object(body)
}

pub fn deletion_problem(reply: &Reply) -> String {
    #[derive(Deserialize)]
    struct Problem {
        message: Option<String>,
    }
    match reply.status {
        401 => "You were signed out. Sign in again, then delete the account.".into(),
        429 => "Too many tries. Wait a few minutes, then try again.".into(),
        404 => "This account server does not offer account deletion from the app yet. Delete the account on its website instead.".into(),
        status => serde_json::from_str::<Problem>(&reply.body)
            .ok()
            .and_then(|p| p.message)
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| format!("The account server could not delete the account ({status}). Try again in a moment.")),
    }
}

pub fn shared_secret_belongs_to(entry_key_record: Option<&str>, account: &str) -> bool {
    entry_key_record
        .and_then(|raw| serde_json::from_str::<StoredKey>(raw).ok())
        .is_some_and(|stored| stored.account == account)
}

pub fn parse_device_secret(raw: Option<&str>) -> Option<[u8; 32]> {
    let bytes = data_encoding::HEXLOWER_PERMISSIVE.decode(raw?.trim().as_bytes()).ok()?;
    bytes.try_into().ok()
}

pub fn writer() -> String {
    format!("Commy web {}", option_env!("COMMY_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")))
}

pub fn earlier_path(name: &str) -> String {
    format!("/api/auth/commy/{}?name={name}", commy_net::entry_list::EARLIER_PATH)
}

pub fn earlier_copies(reply: &Reply) -> Result<commy_net::entry_list::EarlierCopies, String> {
    match reply.status {
        200..=299 => serde_json::from_str(&reply.body).map_err(|_| "Your account server sent an invalid reply".to_string()),
        404 => Err("Your account server does not keep earlier server lists yet.".into()),
        _ => Err("Could not fetch earlier server lists. Press Try again.".into()),
    }
}

pub fn entry_status(signed_in: bool, unlocked: bool, has_key: bool, servers: usize) -> serde_json::Value {
    let (state, detail) = match (signed_in, unlocked) {
        (false, _) => ("off", ""),
        (true, true) => ("synced", "Your server list is open in this browser."),
        (true, false) => ("waiting", "This browser opens your servers through your other devices."),
    };
    serde_json::json!({ "state": state, "servers": servers, "has_key": has_key && unlocked, "detail": detail })
}

#[derive(Deserialize)]
struct MovedReply {
    moved: commy_core::moved::MovePointer,
}

pub fn moved_to_of(reply: &Reply, user: commy_core::UserId, identity: &commy_core::IdentityKey, from: &str) -> Option<Option<String>> {
    match reply.status {
        200..=299 => Some(
            serde_json::from_str::<MovedReply>(&reply.body)
                .ok()
                .map(|reply| reply.moved)
                .filter(|pointer| pointer.moves(user, identity, from))
                .map(|pointer| pointer.to),
        ),
        404 => Some(None),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moved_body(identity: &commy_core::Identity, user: commy_core::UserId, from: &str, to: &str) -> String {
        serde_json::json!({ "moved": commy_core::moved::MovePointer::sign(identity, from, to, user, 1_700_000_000_000) }).to_string()
    }

    #[test]
    fn a_signed_move_pointer_for_me_names_the_new_home() {
        let identity = commy_core::Identity::generate();
        let user = commy_core::UserId::new();
        let body = moved_body(&identity, user, "https://staging.example.org", "https://home.example.net");
        let reply = Reply { status: 200, body };
        assert_eq!(moved_to_of(&reply, user, &identity.public(), "https://staging.example.org"), Some(Some("https://home.example.net".to_string())));
    }

    #[test]
    fn a_move_pointer_for_someone_else_or_another_home_is_ignored() {
        let identity = commy_core::Identity::generate();
        let user = commy_core::UserId::new();
        let from = "https://staging.example.org";
        let to = "https://home.example.net";
        for body in [
            moved_body(&commy_core::Identity::generate(), user, from, to),
            moved_body(&identity, user, "https://other.example.org", to),
            moved_body(&identity, commy_core::UserId::new(), from, to),
            "not json".to_string(),
        ] {
            assert_eq!(moved_to_of(&Reply { status: 200, body }, user, &identity.public(), from), Some(None));
        }
    }

    #[test]
    fn not_moved_clears_and_a_failed_check_keeps_what_we_knew() {
        let identity = commy_core::Identity::generate().public();
        let user = commy_core::UserId::new();
        let from = "https://staging.example.org";
        assert_eq!(moved_to_of(&Reply { status: 404, body: String::new() }, user, &identity, from), Some(None));
        for status in [401, 429, 502] {
            assert_eq!(moved_to_of(&Reply { status, body: String::new() }, user, &identity, from), None, "{status}");
        }
    }

    fn reply(status: u16, body: &str) -> Reply {
        Reply { status, body: body.into() }
    }

    #[test]
    fn reads_the_status_line_and_body() {
        assert_eq!(Reply::parse("200\n{\"a\":1}\nmore"), Some(reply(200, "{\"a\":1}\nmore")));
        assert_eq!(Reply::parse("204"), Some(reply(204, "")));
        assert_eq!(Reply::parse("oops"), None);
    }

    #[test]
    fn reads_a_signed_in_session_or_none() {
        let user = parse_session(&reply(200, r#"{"user":{"id":"u1","name":"Ann","email":"a@example.com"},"session":{}}"#));
        assert_eq!(user.map(|u| u.id), Some("u1".into()));
        let made = parse_session(&reply(200, r#"{"user":{"id":"u1","createdAt":"2026-11-17T10:00:00.000Z"},"session":{}}"#)).unwrap();
        assert_eq!(made.created_at.as_deref(), Some("2026-11-17T10:00:00.000Z"));
        assert_eq!(serde_json::to_value(&made).unwrap()["created_at"], "2026-11-17T10:00:00.000Z");
        assert_eq!(parse_session(&reply(200, "null")), None);
        assert_eq!(parse_session(&reply(401, r#"{"user":{"id":"u1"}}"#)), None);
    }

    #[test]
    fn only_a_clear_answer_ends_the_browser_session() {
        assert!(session_ended(&reply(200, "null")));
        assert!(session_ended(&reply(401, "")));
        assert!(session_ended(&reply(403, "")));
        assert!(!session_ended(&reply(200, r#"{"user":{"id":"u1"},"session":{}}"#)));
        assert!(!session_ended(&reply(200, "<html>busy</html>")));
        assert!(!session_ended(&reply(502, "")));
        assert!(!session_ended(&reply(429, "")));
    }

    #[test]
    fn signs_only_the_expected_link_message() {
        let good = reply(200, r#"{"challenge":"c1","message":"commy-device-link:v1\nhttps://commy.chat\nabcd\nc1"}"#);
        assert!(parse_challenge(&good, "https://commy.chat", "abcd").is_ok());
        assert!(parse_challenge(&good, "https://evil.example", "abcd").is_err());
        let other = reply(200, r#"{"challenge":"c1","message":"transfer all funds"}"#);
        assert!(parse_challenge(&other, "https://commy.chat", "abcd").is_err());
    }

    #[test]
    fn lists_the_other_devices_only() {
        let body = r#"{"devices":[{"id":"1","nodeId":"AA11"},{"id":"2","nodeId":"bb22"}]}"#;
        assert_eq!(other_devices(&reply(200, body), "aa11"), vec!["bb22".to_string()]);
        assert!(other_devices(&reply(500, body), "aa11").is_empty());
    }

    #[test]
    fn reads_the_earlier_copies() {
        let copies = earlier_copies(&reply(200, r#"{"name":"servers","items":[{"version":3,"data":"sealed1.x","updatedAt":null,"writer":"Commy 1.0.0 (ios)"}]}"#)).unwrap();
        assert_eq!(copies.items[0].version, 3);
        assert!(earlier_copies(&reply(404, "")).unwrap_err().contains("does not keep"));
        assert!(earlier_copies(&reply(500, "")).is_err());
        assert_eq!(earlier_path("servers"), "/api/auth/commy/vault/earlier?name=servers");
        assert!(writer().starts_with("Commy web "));
    }

    #[test]
    fn reads_the_vault_item() {
        assert_eq!(vault_data(&reply(200, r#"{"name":"servers","data":"sealed1.x","version":3}"#)), Ok(Some("sealed1.x".into())));
        assert_eq!(vault_data(&reply(200, r#"{"name":"servers","data":null,"version":0}"#)), Ok(None));
        assert!(vault_data(&reply(404, "")).is_err());
    }

    #[test]
    fn names_a_new_list_after_the_account() {
        let account = |name: &str, email: &str| AccountUser { id: "u1".into(), name: name.into(), email: email.into(), created_at: None };
        assert_eq!(username_for(&account("  Ada Lovelace ", "ada@example.test")), "Ada Lovelace");
        assert_eq!(username_for(&account("", "grace@example.test")), "grace");
        assert_eq!(username_for(&account(" ", "")), "Commy user");
        let long = username_for(&account(&"é".repeat(40), ""));
        assert!(long.len() <= MAX_USERNAME_BYTES && !long.is_empty());
    }

    #[test]
    fn a_browser_without_the_key_is_pointed_at_a_device_or_the_recovery_code() {
        assert!(KEY_NOT_HERE.contains("Try again"));
        assert!(KEY_NOT_HERE.contains("recovery code"));
        assert!(!KEY_NOT_HERE.to_lowercase().contains("saved with your account"));
    }

    #[test]
    fn finds_the_sync_service_only_on_this_site() {
        let doc = |sync: &str| reply(200, &format!(r#"{{"endpoints":{{"sync":"{sync}","auth":"x"}},"features":{{}}}}"#));
        assert_eq!(sync_url(&doc("wss://commy.chat/sync"), "https://commy.chat").as_deref(), Some("wss://commy.chat/sync"));
        assert_eq!(sync_url(&doc("wss://elsewhere.example/sync"), "https://commy.chat"), None);
        assert_eq!(sync_url(&doc("ws://commy.chat/sync"), "https://commy.chat"), None);
        assert_eq!(sync_url(&reply(200, r#"{"endpoints":{}}"#), "https://commy.chat"), None);
        assert_eq!(sync_url(&reply(404, ""), "https://commy.chat"), None);
    }

    #[test]
    fn keeps_the_entry_key_per_account() {
        let key = SealKey::generate();
        let record = entry_key_record("u1", &key);
        assert_eq!(stored_entry_key(Some(&record), "u1").map(|k| k.to_text()), Some(key.to_text()));
        assert!(stored_entry_key(Some(&record), "u2").is_none());
        assert!(stored_entry_key(None, "u1").is_none());
    }

    #[test]
    fn deleting_forgets_everything_this_browser_kept_for_the_account() {
        let kept = kept_for_account("u1");
        assert!(kept.contains(&pins_key("u1")));
        assert!(kept.contains(&device_secret_key("u1")));
        assert!(kept.contains(&safety_key("u1")));
        assert!(!kept.iter().any(|key| key.ends_with(".u2")));
    }

    #[test]
    fn deletion_sends_only_the_proof_that_was_given() {
        assert_eq!(deletion_proof(Some(" 123456 "), None), serde_json::json!({ "code": "123456" }));
        assert_eq!(deletion_proof(None, Some("pw")), serde_json::json!({ "password": "pw" }));
        assert_eq!(deletion_proof(Some(""), Some("")), serde_json::json!({}));
    }

    #[test]
    fn deletion_problems_read_like_sentences() {
        let wrong = Reply { status: 400, body: r#"{"code":"INVALID_CODE","message":"That code is not right or has expired. Ask for a new one."}"#.into() };
        assert_eq!(deletion_problem(&wrong), "That code is not right or has expired. Ask for a new one.");
        assert!(deletion_problem(&Reply { status: 401, body: String::new() }).contains("Sign in again"));
        assert!(deletion_problem(&Reply { status: 404, body: String::new() }).contains("website"));
        assert!(deletion_problem(&Reply { status: 502, body: "<html>".into() }).contains("502"));
    }

    #[test]
    fn each_account_has_its_own_browser_key() {
        assert_ne!(device_secret_key("u1"), device_secret_key("u2"));
        assert_ne!(device_secret_key("u1"), DEVICE_SECRET_KEY);
        let record = entry_key_record("u1", &SealKey::generate());
        assert!(shared_secret_belongs_to(Some(&record), "u1"));
        assert!(!shared_secret_belongs_to(Some(&record), "u2"));
        assert!(!shared_secret_belongs_to(None, "u1"));
        assert!(!shared_secret_belongs_to(Some("junk"), "u1"));
    }

    #[test]
    fn reads_a_saved_device_secret() {
        let hex = "11".repeat(32);
        assert_eq!(parse_device_secret(Some(&hex)), Some([0x11; 32]));
        assert_eq!(parse_device_secret(Some("zz")), None);
    }

    #[test]
    fn searches_go_to_this_site_with_their_words_encoded() {
        let path = search_path("gifs", &[("q", "cat & dog/ü".to_string())]);
        assert_eq!(path, "/api/auth/commy/gifs/search?q=cat%20%26%20dog%2F%C3%BC");
        let found = Reply { status: 200, body: r#"{"results":[1,2]}"#.into() };
        let hits: Option<commy_view::catalog::Found<u8>> = search_reply(&found).unwrap();
        assert_eq!(hits.unwrap().results, vec![1, 2]);
        assert!(search_reply::<commy_view::catalog::Found<u8>>(&Reply { status: 404, body: String::new() }).unwrap().is_none());
        assert!(search_reply::<commy_view::catalog::Found<u8>>(&Reply { status: 429, body: String::new() }).is_err());
        assert_eq!(
            search_reply::<commy_view::catalog::Found<u8>>(&Reply { status: 503, body: String::new() }).unwrap_err(),
            "Search is busy right now, try again in a minute"
        );
    }

    #[test]
    fn reads_what_the_site_offers() {
        let reply = Reply { status: 200, body: r#"{"features":{"gifs":true,"media":true}}"#.into() };
        assert_eq!(features(&reply), Features { gifs: true, media: true, ..Features::default() });
        assert_eq!(features(&Reply { status: 500, body: String::new() }), Features::default());
    }

    #[test]
    fn the_server_list_card_offers_the_recovery_code_once_this_browser_holds_the_key() {
        let open = entry_status(true, true, true, 4);
        assert_eq!(open["state"], "synced");
        assert_eq!(open["servers"], 4);
        assert_eq!(open["has_key"], true);
        assert_eq!(entry_status(true, false, true, 0)["has_key"], false);
        assert_eq!(entry_status(true, false, false, 0)["state"], "waiting");
        assert_eq!(entry_status(false, false, false, 0)["state"], "off");
    }
}
