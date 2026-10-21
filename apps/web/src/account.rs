use commy_net::sealed::SealKey;
use serde::{Deserialize, Serialize};

pub const SESSION_PATH: &str = "/api/auth/get-session";
pub const SIGN_OUT_PATH: &str = "/api/auth/sign-out";
pub const DEVICES_PATH: &str = "/api/auth/commy/devices";
pub const LINK_CHALLENGE_PATH: &str = "/api/auth/commy/devices/challenge";
pub const LINK_REGISTER_PATH: &str = "/api/auth/commy/devices/register";
pub const VAULT_PATH: &str = "/api/auth/commy/vault?name=servers";

pub const DEVICE_SECRET_KEY: &str = "commy.device";
pub const ENTRY_KEY_KEY: &str = "commy.entry-key";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountUser {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
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

pub fn parse_session(reply: &Reply) -> Option<AccountUser> {
    #[derive(Deserialize)]
    struct Session {
        user: AccountUser,
    }
    if !reply.ok() {
        return None;
    }
    serde_json::from_str::<Option<Session>>(&reply.body).ok().flatten().map(|s| s.user)
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

pub fn device_secret_key(account: &str) -> String {
    format!("{DEVICE_SECRET_KEY}.{account}")
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(parse_session(&reply(200, "null")), None);
        assert_eq!(parse_session(&reply(401, r#"{"user":{"id":"u1"}}"#)), None);
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
    fn reads_the_vault_item() {
        assert_eq!(vault_data(&reply(200, r#"{"name":"servers","data":"sealed1.x","version":3}"#)), Ok(Some("sealed1.x".into())));
        assert_eq!(vault_data(&reply(200, r#"{"name":"servers","data":null,"version":0}"#)), Ok(None));
        assert!(vault_data(&reply(404, "")).is_err());
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
}
