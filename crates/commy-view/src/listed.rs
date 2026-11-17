use commy_core::server_blocklist::Envelope;
use commy_core::server_key::PublishedKey;
use commy_core::server_list::{self, ListedServer, ServerList};
use serde::Serialize;

pub const LIST_KEY_BAD: &str = "the server list key is not an ed25519 key";
pub const LIST_SIGNATURE_BAD: &str = "the server list signature does not check out";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListedView {
    pub origin: String,
    pub host: String,
    pub name: String,
    pub about: Option<String>,
    pub accounts: bool,
    pub communities: bool,
}

impl ListedView {
    fn of(server: &ListedServer) -> Self {
        let host = commy_core::sync_host::display_host(&server.origin).unwrap_or_default();
        Self {
            origin: server.origin.clone(),
            host: crate::links::unicode_host(host),
            name: server.name.trim().to_string(),
            about: server.about.as_deref().map(str::trim).filter(|a| !a.is_empty()).map(str::to_string),
            accounts: server.hosts_accounts(),
            communities: server.hosts_communities(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostTrust {
    pub origin: String,
    pub host: String,
    pub listed: Option<ListedView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JoinHost {
    #[serde(flatten)]
    pub trust: HostTrust,
    pub ask: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct InvitePreview<T: Serialize> {
    #[serde(flatten)]
    pub invite: T,
    pub community_host: Option<JoinHost>,
}

#[derive(Debug, Clone)]
pub struct ListedServers {
    fetched: Option<ServerList>,
    bundled: ServerList,
}

impl Default for ListedServers {
    fn default() -> Self {
        Self { fetched: None, bundled: server_list::bundled() }
    }
}

impl ListedServers {
    pub fn take_list(&mut self, origin: &str, list_key: &PublishedKey, envelope: &Envelope) -> Result<bool, &'static str> {
        if !list_key.is_well_formed() {
            return Err(LIST_KEY_BAD);
        }
        let list = server_list::open(envelope, origin, &list_key.public).ok_or(LIST_SIGNATURE_BAD)?;
        if self.fetched.as_ref().is_some_and(|held| !list.newer_than(held)) {
            return Ok(false);
        }
        self.fetched = Some(list);
        Ok(true)
    }

    pub fn current(&self, now_ms: i64) -> &ServerList {
        self.fetched.as_ref().filter(|list| list.is_current(now_ms)).unwrap_or(&self.bundled)
    }

    pub fn homes(&self, now_ms: i64, blocked: impl Fn(&str) -> bool) -> Vec<ListedView> {
        let mut homes: Vec<ListedView> = self.current(now_ms).homes().filter(|s| !blocked(&s.origin)).map(ListedView::of).collect();
        homes.sort_by_key(|home| home.name.to_lowercase());
        homes
    }

    pub fn listed(&self, origin: &str, now_ms: i64) -> Option<ListedView> {
        self.current(now_ms).listed(origin).map(ListedView::of)
    }

    pub fn trust_for_host(&self, origin: &str, now_ms: i64) -> Option<HostTrust> {
        let clean = commy_core::sync_host::clean_sync_host(origin)?;
        let host = crate::links::unicode_host(commy_core::sync_host::display_host(&clean).unwrap_or_default());
        let listed = self.current(now_ms).listed(&clean).filter(|s| s.hosts_communities()).map(ListedView::of);
        Some(HostTrust { origin: clean, host, listed })
    }

    pub fn join_host(&self, sync_host: Option<&str>, account_server: &str, official: &str, trusted: &[String], now_ms: i64) -> Option<JoinHost> {
        let trust = self.trust_for_host(sync_host?, now_ms)?;
        let is_host = |server: &str| commy_core::sync_host::clean_sync_host(server).as_deref() == Some(trust.origin.as_str());
        if is_host(account_server) || is_host(official) {
            return None;
        }
        let ask = trust.listed.is_none() && !trusted.iter().any(|t| t == &trust.origin);
        Some(JoinHost { trust, ask })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::identity::Identity;
    use commy_core::server_list::{LIST_PURPOSE, LIST_VERSION};

    fn entry(origin: &str, name: &str, kinds: &[&str]) -> ListedServer {
        ListedServer {
            origin: origin.into(),
            name: name.into(),
            about: Some("  Friendly folks ".into()),
            kinds: kinds.iter().map(|k| k.to_string()).collect(),
            key: None,
            contact: None,
            terms: None,
            privacy: None,
            listed: 1,
            checked: 1,
        }
    }

    fn list(issued: i64, servers: Vec<ListedServer>) -> ServerList {
        ServerList { v: LIST_VERSION, issued, expires: issued + 100, servers }
    }

    fn sealed(signer: &Identity, list: &ServerList) -> Envelope {
        let text = serde_json::to_string(list).unwrap();
        let sig = commy_core::server_key::sign(signer, LIST_PURPOSE, "https://commy.chat", text.as_bytes());
        Envelope { list: text, key: commy_core::server_key::key_id(&signer.public()), sig }
    }

    fn held(signer: &Identity, servers: Vec<ListedServer>) -> ListedServers {
        let mut listed = ListedServers::default();
        listed.take_list("https://commy.chat", &PublishedKey::of(signer.public()), &sealed(signer, &list(10, servers))).unwrap();
        listed
    }

    #[test]
    fn only_a_newer_list_replaces_the_one_held() {
        let signer = Identity::from_bytes(&[2; 32]);
        let key = PublishedKey::of(signer.public());
        let mut listed = held(&signer, vec![entry("https://chat.example.org", "Example", &["home"])]);
        assert_eq!(listed.take_list("https://commy.chat", &key, &sealed(&signer, &list(10, vec![]))), Ok(false));
        assert_eq!(listed.take_list("https://commy.chat", &key, &sealed(&signer, &list(11, vec![]))), Ok(true));
        assert!(listed.homes(12, |_| false).is_empty());
    }

    #[test]
    fn a_list_signed_by_another_key_is_refused() {
        let signer = Identity::from_bytes(&[2; 32]);
        let stranger = PublishedKey::of(Identity::from_bytes(&[3; 32]).public());
        let mut listed = ListedServers::default();
        assert_eq!(listed.take_list("https://commy.chat", &stranger, &sealed(&signer, &list(10, vec![]))), Err(LIST_SIGNATURE_BAD));
    }

    #[test]
    fn homes_come_sorted_by_name_without_blocked_or_community_only_servers() {
        let signer = Identity::from_bytes(&[2; 32]);
        let listed = held(
            &signer,
            vec![
                entry("https://zeta.example.org", "zeta", &["home"]),
                entry("https://alpha.example.org", "Alpha", &["home", "sync"]),
                entry("https://rooms.example.org", "Rooms", &["sync"]),
                entry("https://bad.example.org", "Bad", &["home"]),
            ],
        );
        let homes = listed.homes(11, |origin| origin.contains("bad."));
        assert_eq!(homes.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(), ["Alpha", "zeta"]);
        assert_eq!(homes[0].about.as_deref(), Some("Friendly folks"));
        assert_eq!(homes[0].host, "alpha.example.org");
    }

    #[test]
    fn an_expired_list_falls_back_to_the_bundled_one() {
        let signer = Identity::from_bytes(&[2; 32]);
        let listed = held(&signer, vec![entry("https://chat.example.org", "Example", &["home"])]);
        assert!(listed.listed("https://chat.example.org", 50).is_some());
        assert!(listed.listed("https://chat.example.org", 500).is_none());
    }

    #[test]
    fn a_community_host_is_trusted_only_when_listed_for_communities() {
        let signer = Identity::from_bytes(&[2; 32]);
        let listed = held(&signer, vec![entry("https://rooms.example.org", "Rooms", &["sync"]), entry("https://accounts.example.org", "Accounts", &["home"])]);
        assert!(listed.trust_for_host("rooms.example.org", 11).unwrap().listed.is_some());
        assert!(listed.trust_for_host("https://accounts.example.org", 11).unwrap().listed.is_none());
        assert_eq!(listed.trust_for_host("https://new.example.org", 11).unwrap().host, "new.example.org");
        assert!(listed.trust_for_host("not a host", 11).is_none());
    }

    #[test]
    fn joining_asks_for_trust_only_on_an_unlisted_unconfirmed_host() {
        let signer = Identity::from_bytes(&[2; 32]);
        let listed = held(&signer, vec![entry("https://listed.example.org", "Listed", &["sync"])]);
        let trusted = vec!["https://rooms.example.org".to_string()];
        let ask = |host: Option<&str>| listed.join_host(host, "https://commy.chat", "https://commy.chat", &trusted, 11).map(|j| j.ask);
        assert_eq!(ask(None), None);
        assert_eq!(ask(Some("https://commy.chat")), None);
        assert_eq!(ask(Some("rooms.example.org")), Some(false));
        assert_eq!(ask(Some("https://listed.example.org")), Some(false));
        assert_eq!(ask(Some("https://new.example.org")), Some(true));
    }

    #[test]
    fn joining_a_community_on_the_official_server_never_asks_from_another_home() {
        let signer = Identity::from_bytes(&[2; 32]);
        let listed = held(&signer, vec![entry("https://listed.example.org", "Listed", &["home", "sync"])]);
        let ask = |host: &str, home: &str| listed.join_host(Some(host), home, "https://staging.commy.chat", &[], 11).map(|j| j.ask);
        assert_eq!(ask("https://staging.commy.chat", "https://dev.commy.chat"), None);
        assert_eq!(ask("staging.commy.chat", "https://listed.example.org"), None);
        assert_eq!(ask("https://dev.commy.chat", "https://dev.commy.chat"), None);
        assert_eq!(ask("https://dev.commy.chat", "https://staging.commy.chat"), Some(true));
        assert_eq!(ask("https://listed.example.org", "https://dev.commy.chat"), Some(false));
        assert_eq!(listed.join_host(Some("https://staging.commy.chat"), "https://dev.commy.chat", "https://commy.chat", &[], 11).map(|j| j.ask), Some(true));
    }
}
