use commy_core::lock_or_recover::ReadOrRecover;
use std::sync::RwLock;
use std::time::Duration;

use commy_core::server_blocklist::{official_key, Envelope, ServerBlocklist, MAX_LIST_BYTES};
use commy_core::server_key::PublishedKey;
use commy_core::IdentityKey;

use crate::server_proof::clean_origin;

pub const REFRESH_EVERY: Duration = Duration::from_secs(15 * 60);
const DISCOVERY_PATH: &str = "/.well-known/commy-auth.json";
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_DISCOVERY_BYTES: usize = 64 * 1024;

pub struct Blocklist {
    from: Option<String>,
    list: RwLock<Option<ServerBlocklist>>,
    http: reqwest::Client,
}

impl Blocklist {
    pub fn new(from: Option<String>) -> Self {
        let http = reqwest::Client::builder().timeout(FETCH_TIMEOUT).redirect(reqwest::redirect::Policy::none()).build().unwrap_or_default();
        Self { from, list: RwLock::new(None), http }
    }

    pub fn from(&self) -> Option<&str> {
        self.from.as_deref()
    }

    pub fn blocks(&self, origin: &str) -> bool {
        self.list.read_or_recover().as_ref().is_some_and(|list| list.blocks_origin(origin))
    }

    pub fn blocked_count(&self) -> Option<usize> {
        self.list.read_or_recover().as_ref().map(|list| list.domains.len())
    }

    pub fn keep(&self, list: ServerBlocklist) -> bool {
        let mut kept = self.list.write_or_recover();
        if kept.as_ref().is_some_and(|old| !list.newer_than(old) && list != *old) {
            return false;
        }
        *kept = Some(list);
        true
    }

    pub fn take(&self, envelope: &Envelope, key: &IdentityKey) -> Result<(), String> {
        let from = self.from.as_deref().ok_or("no blocklist source is set")?;
        let list = envelope.open(from, key).ok_or("the blocklist's signature does not hold")?;
        self.keep(list);
        Ok(())
    }

    pub async fn refresh(&self) -> Result<(), String> {
        let Some(from) = self.from.as_deref() else {
            return Ok(());
        };
        let discovery: serde_json::Value = serde_json::from_slice(&self.fetch(&format!("{from}{DISCOVERY_PATH}"), MAX_DISCOVERY_BYTES).await?).map_err(|e| e.to_string())?;
        let published = discovery.get("serverBlocklist").ok_or("the discovery document names no serverBlocklist")?;
        let url = published.get("url").and_then(|u| u.as_str()).filter(|u| u.starts_with(&format!("{from}/"))).ok_or("the blocklist lives on another site")?;
        let key = match official_key(from) {
            Some(key) => key,
            None => serde_json::from_value::<PublishedKey>(published.get("key").cloned().unwrap_or_default()).ok().filter(PublishedKey::is_well_formed).ok_or("the blocklist key is not well formed")?.public,
        };
        let envelope: Envelope = serde_json::from_slice(&self.fetch(url, MAX_LIST_BYTES + MAX_DISCOVERY_BYTES).await?).map_err(|e| e.to_string())?;
        self.take(&envelope, &key)
    }

    async fn fetch(&self, url: &str, limit: usize) -> Result<Vec<u8>, String> {
        let mut response = self.http.get(url).send().await.map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("{url} answered {}", response.status()));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            body.extend_from_slice(&chunk);
            if body.len() > limit {
                return Err(format!("{url} is too large"));
            }
        }
        Ok(body)
    }
}

pub fn source(text: Option<String>) -> Result<Option<String>, String> {
    match text.as_deref().map(str::trim) {
        Some("off") => Ok(None),
        None | Some("") => Ok(Some(DEFAULT_SOURCE.to_string())),
        Some(text) => clean_origin(text).map(Some).ok_or_else(|| "SYNC_GATEWAY_BLOCKLIST must be off or an https:// address such as https://commy.chat".to_string()),
    }
}

pub const DEFAULT_SOURCE: &str = "https://commy.chat";

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::server_blocklist::{BlockedServer, LIST_PURPOSE};
    use commy_core::Identity;

    fn signed(key: &Identity, from: &str, issued: i64, domains: &[&str]) -> Envelope {
        let list = ServerBlocklist {
            v: 1,
            issued,
            expires: issued + 1,
            domains: domains.iter().map(|d| BlockedServer { domain: d.to_string(), since: 0 }).collect(),
            homes: Vec::new(),
        };
        let text = serde_json::to_string(&list).unwrap();
        let sig = commy_core::server_key::sign(key, LIST_PURPOSE, from, text.as_bytes());
        Envelope { list: text, key: PublishedKey::of(key.public()).id, sig }
    }

    #[test]
    fn a_listed_server_and_its_subdomains_are_blocked() {
        let key = Identity::from_bytes(&[4; 32]);
        let list = Blocklist::new(Some("https://commy.chat".into()));
        assert!(!list.blocks("https://sync.bad.example"), "nothing is blocked before a list arrives");
        list.take(&signed(&key, "https://commy.chat", 10, &["bad.example"]), &key.public()).unwrap();
        assert!(list.blocks("https://sync.bad.example"));
        assert!(list.blocks("https://bad.example"));
        assert!(!list.blocks("https://good.example"));
        assert_eq!(list.blocked_count(), Some(1));
    }

    #[test]
    fn a_forged_or_older_list_is_not_taken() {
        let key = Identity::from_bytes(&[4; 32]);
        let forger = Identity::from_bytes(&[5; 32]);
        let list = Blocklist::new(Some("https://commy.chat".into()));
        list.take(&signed(&key, "https://commy.chat", 10, &["bad.example"]), &key.public()).unwrap();
        assert!(list.take(&signed(&forger, "https://commy.chat", 11, &[]), &key.public()).is_err());
        assert!(list.take(&signed(&key, "https://other.example", 12, &[]), &key.public()).is_err(), "signed for another site");
        list.take(&signed(&key, "https://commy.chat", 9, &[]), &key.public()).unwrap();
        assert!(list.blocks("https://bad.example"), "an older list does not replace a newer one");
        list.take(&signed(&key, "https://commy.chat", 11, &[]), &key.public()).unwrap();
        assert!(!list.blocks("https://bad.example"));
    }

    #[test]
    fn the_source_defaults_to_commy_chat_and_can_be_turned_off() {
        assert_eq!(source(None), Ok(Some(DEFAULT_SOURCE.into())));
        assert_eq!(source(Some("off".into())), Ok(None));
        assert_eq!(source(Some("https://Staging.Commy.chat/".into())), Ok(Some("https://staging.commy.chat".into())));
        assert!(source(Some("http://commy.chat".into())).is_err());
    }
}
