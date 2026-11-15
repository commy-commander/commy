use crate::identity::{IdentityKey, Signature};
use crate::server_key::PublishedKey;
use serde::{Deserialize, Serialize};

pub const LIST_PURPOSE: &str = "server-blocklist";
pub const LIST_VERSION: u32 = 1;
pub const PATH: &str = "/commy/server-blocklist";
pub const MAX_LIST_BYTES: usize = 2 * 1024 * 1024;

pub const OFFICIAL_LIST_KEYS: &[(&str, &str)] = &[];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub list: String,
    pub key: String,
    pub sig: Signature,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockedServer {
    pub domain: String,
    #[serde(default)]
    pub since: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownHome {
    pub home: String,
    pub key: PublishedKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ServerBlocklist {
    pub v: u32,
    pub issued: i64,
    pub expires: i64,
    #[serde(default)]
    pub domains: Vec<BlockedServer>,
    #[serde(default)]
    pub homes: Vec<KnownHome>,
}

pub fn official_key(origin: &str) -> Option<IdentityKey> {
    OFFICIAL_LIST_KEYS.iter().find(|(o, _)| *o == origin).and_then(|(_, hex)| IdentityKey::from_hex(hex))
}

impl Envelope {
    pub fn open(&self, origin: &str, key: &IdentityKey) -> Option<ServerBlocklist> {
        let published = PublishedKey::of(*key);
        if self.list.len() > MAX_LIST_BYTES || self.key != published.id {
            return None;
        }
        if !published.verifies(LIST_PURPOSE, origin, self.list.as_bytes(), &self.sig) {
            return None;
        }
        let list: ServerBlocklist = serde_json::from_str(&self.list).ok()?;
        (list.v == LIST_VERSION).then_some(list)
    }
}

impl ServerBlocklist {
    pub fn blocks(&self, domain: &str) -> bool {
        self.domains.iter().any(|blocked| crate::home::covers(&blocked.domain, domain))
    }

    pub fn blocks_origin(&self, origin: &str) -> bool {
        crate::home::clean_home_domain(origin).is_some_and(|domain| self.blocks(&domain))
    }

    pub fn known_key(&self, home: &str) -> Option<&PublishedKey> {
        self.homes.iter().find(|known| known.home == home).map(|known| &known.key)
    }

    pub fn newer_than(&self, other: &ServerBlocklist) -> bool {
        self.issued > other.issued
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::identity::Identity;

    pub(crate) fn sealed(signer: &Identity, origin: &str, list: &ServerBlocklist) -> Envelope {
        let text = serde_json::to_string(list).unwrap();
        let sig = crate::server_key::sign(signer, LIST_PURPOSE, origin, text.as_bytes());
        Envelope { list: text, key: crate::server_key::key_id(&signer.public()), sig }
    }

    fn sample() -> ServerBlocklist {
        ServerBlocklist {
            v: LIST_VERSION,
            issued: 10,
            expires: 20,
            domains: vec![BlockedServer { domain: "bad.example".into(), since: 5 }],
            homes: vec![KnownHome { home: "https://commy.chat".into(), key: PublishedKey::of(Identity::from_bytes(&[8; 32]).public()) }],
        }
    }

    #[test]
    fn a_list_opens_only_with_its_origins_key() {
        let signer = Identity::from_bytes(&[7; 32]);
        let envelope = sealed(&signer, "https://commy.chat", &sample());
        assert_eq!(envelope.open("https://commy.chat", &signer.public()), Some(sample()));
        assert_eq!(envelope.open("https://staging.commy.chat", &signer.public()), None);
        assert_eq!(envelope.open("https://commy.chat", &Identity::from_bytes(&[1; 32]).public()), None);
        let edited = Envelope { list: envelope.list.replace("bad.example", "good.example"), ..envelope.clone() };
        assert_eq!(edited.open("https://commy.chat", &signer.public()), None);
    }

    #[test]
    fn a_list_from_a_future_version_is_not_read() {
        let signer = Identity::from_bytes(&[7; 32]);
        let future = ServerBlocklist { v: LIST_VERSION + 1, ..sample() };
        assert_eq!(sealed(&signer, "https://commy.chat", &future).open("https://commy.chat", &signer.public()), None);
    }

    #[test]
    fn a_listed_domain_blocks_its_subdomains_and_origins() {
        let list = sample();
        assert!(list.blocks("bad.example") && list.blocks("sync.bad.example"));
        assert!(!list.blocks("commy.chat"));
        assert!(list.blocks_origin("https://sync.bad.example"));
        assert!(list.blocks_origin("wss://sync.bad.example/sync"));
        assert!(!list.blocks_origin("https://commy.chat"));
    }

    #[test]
    fn a_list_names_the_keys_of_homes_it_knows() {
        let list = sample();
        assert!(list.known_key("https://commy.chat").is_some());
        assert!(list.known_key("https://bad.example").is_none());
    }

    #[test]
    fn official_keys_are_well_formed() {
        for (origin, hex) in OFFICIAL_LIST_KEYS {
            assert!(crate::sync_host::is_sync_host(origin), "{origin}");
            assert!(IdentityKey::from_hex(hex).is_some(), "{origin}");
        }
    }
}
