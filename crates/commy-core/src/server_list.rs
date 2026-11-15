use crate::identity::IdentityKey;
use crate::server_blocklist::Envelope;
use crate::server_key::PublishedKey;
use serde::{Deserialize, Serialize};

pub const LIST_PURPOSE: &str = "server-list";
pub const LIST_VERSION: u32 = 1;
pub const PATH: &str = "/commy/server-list";
pub const HOME: &str = "home";
pub const SYNC: &str = "sync";
const BUNDLED: &str = include_str!("../listed-servers.json");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedServer {
    pub origin: String,
    pub name: String,
    #[serde(default)]
    pub about: Option<String>,
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub key: Option<PublishedKey>,
    #[serde(default)]
    pub contact: Option<String>,
    #[serde(default)]
    pub terms: Option<String>,
    #[serde(default)]
    pub privacy: Option<String>,
    #[serde(default)]
    pub listed: i64,
    #[serde(default)]
    pub checked: i64,
}

impl ListedServer {
    pub fn hosts_accounts(&self) -> bool {
        self.kinds.iter().any(|kind| kind == HOME)
    }

    pub fn hosts_communities(&self) -> bool {
        self.kinds.iter().any(|kind| kind == SYNC)
    }

    fn is_usable(&self) -> bool {
        crate::sync_host::is_sync_host(&self.origin) && !self.name.trim().is_empty() && self.key.as_ref().is_none_or(PublishedKey::is_well_formed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ServerList {
    pub v: u32,
    pub issued: i64,
    pub expires: i64,
    #[serde(default)]
    pub servers: Vec<ListedServer>,
}

pub fn open(envelope: &Envelope, origin: &str, key: &IdentityKey) -> Option<ServerList> {
    let list: ServerList = serde_json::from_str(envelope.verified_text(LIST_PURPOSE, origin, key)?).ok()?;
    (list.v == LIST_VERSION).then(|| list.usable())
}

pub fn bundled() -> ServerList {
    serde_json::from_str::<ServerList>(BUNDLED).map(ServerList::usable).unwrap_or_default()
}

impl ServerList {
    fn usable(mut self) -> Self {
        self.servers.retain(ListedServer::is_usable);
        self
    }

    pub fn is_current(&self, now_ms: i64) -> bool {
        self.expires > now_ms
    }

    pub fn newer_than(&self, other: &ServerList) -> bool {
        self.issued > other.issued
    }

    pub fn listed(&self, origin: &str) -> Option<&ListedServer> {
        let clean = crate::sync_host::clean_sync_host(origin)?;
        self.servers.iter().find(|server| server.origin == clean)
    }

    pub fn homes(&self) -> impl Iterator<Item = &ListedServer> {
        self.servers.iter().filter(|server| server.hosts_accounts())
    }

    pub fn lists_home(&self, origin: &str) -> bool {
        self.listed(origin).is_some_and(ListedServer::hosts_accounts)
    }

    pub fn lists_community_host(&self, origin: &str) -> bool {
        self.listed(origin).is_some_and(ListedServer::hosts_communities)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;

    fn server(origin: &str, kinds: &[&str]) -> ListedServer {
        ListedServer {
            origin: origin.into(),
            name: "Example Chat".into(),
            about: None,
            kinds: kinds.iter().map(|k| k.to_string()).collect(),
            key: None,
            contact: Some("abuse@example.org".into()),
            terms: None,
            privacy: None,
            listed: 1,
            checked: 2,
        }
    }

    fn sample() -> ServerList {
        ServerList {
            v: LIST_VERSION,
            issued: 10,
            expires: 20,
            servers: vec![server("https://chat.example.org", &[HOME, SYNC]), server("https://rooms.example.net", &[SYNC])],
        }
    }

    fn sealed(signer: &Identity, origin: &str, list: &ServerList) -> Envelope {
        let text = serde_json::to_string(list).unwrap();
        let sig = crate::server_key::sign(signer, LIST_PURPOSE, origin, text.as_bytes());
        Envelope { list: text, key: crate::server_key::key_id(&signer.public()), sig }
    }

    #[test]
    fn a_list_opens_only_with_its_origins_key_and_purpose() {
        let signer = Identity::from_bytes(&[7; 32]);
        let envelope = sealed(&signer, "https://commy.chat", &sample());
        assert_eq!(open(&envelope, "https://commy.chat", &signer.public()), Some(sample()));
        assert_eq!(open(&envelope, "https://staging.commy.chat", &signer.public()), None);
        assert_eq!(open(&envelope, "https://commy.chat", &Identity::from_bytes(&[1; 32]).public()), None);
        let edited = Envelope { list: envelope.list.replace("chat.example.org", "chat.example.com"), ..envelope.clone() };
        assert_eq!(open(&edited, "https://commy.chat", &signer.public()), None);
    }

    #[test]
    fn a_blocklist_signature_does_not_open_as_a_server_list() {
        let signer = Identity::from_bytes(&[7; 32]);
        let text = serde_json::to_string(&sample()).unwrap();
        let sig = crate::server_key::sign(&signer, crate::server_blocklist::LIST_PURPOSE, "https://commy.chat", text.as_bytes());
        let envelope = Envelope { list: text, key: crate::server_key::key_id(&signer.public()), sig };
        assert_eq!(open(&envelope, "https://commy.chat", &signer.public()), None);
    }

    #[test]
    fn a_list_from_a_future_version_is_not_read() {
        let signer = Identity::from_bytes(&[7; 32]);
        let future = ServerList { v: LIST_VERSION + 1, ..sample() };
        assert_eq!(open(&sealed(&signer, "https://commy.chat", &future), "https://commy.chat", &signer.public()), None);
    }

    #[test]
    fn entries_without_a_clean_address_or_name_are_dropped() {
        let signer = Identity::from_bytes(&[7; 32]);
        let mut list = sample();
        list.servers.push(server("http://plain.example.org", &[HOME]));
        list.servers.push(ListedServer { name: " ".into(), ..server("https://nameless.example.org", &[HOME]) });
        list.servers.push(server("https://LOUD.example.org", &[HOME]));
        let opened = open(&sealed(&signer, "https://commy.chat", &list), "https://commy.chat", &signer.public()).unwrap();
        assert_eq!(opened, sample());
    }

    #[test]
    fn unknown_fields_and_kinds_are_ignored() {
        let text = r#"{"v":1,"issued":1,"expires":2,"later":true,"servers":[{"origin":"https://chat.example.org","name":"Example","kinds":["home","video"],"badge":"gold"}]}"#;
        let list: ServerList = serde_json::from_str(text).unwrap();
        assert!(list.lists_home("https://chat.example.org"));
        assert!(!list.lists_community_host("https://chat.example.org"));
    }

    #[test]
    fn lookups_clean_the_address_first() {
        let list = sample();
        assert!(list.lists_home("chat.example.org"));
        assert!(list.lists_home("https://CHAT.example.org/"));
        assert!(list.lists_community_host("wss://rooms.example.net/sync"));
        assert!(!list.lists_home("https://rooms.example.net"));
        assert!(list.listed("https://other.example.org").is_none());
        assert_eq!(list.homes().count(), 1);
    }

    #[test]
    fn a_list_counts_only_until_it_expires() {
        let list = sample();
        assert!(list.is_current(19));
        assert!(!list.is_current(20));
        assert!(ServerList { issued: 11, ..sample() }.newer_than(&list));
    }

    #[test]
    fn the_bundled_list_reads() {
        assert_eq!(bundled().v, LIST_VERSION);
    }
}
