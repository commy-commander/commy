use crate::identity::{Identity, IdentityKey, Signature};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const ALGORITHM: &str = "ed25519";
pub const SIGNING_DOMAIN: &str = "commy-server/v1";
const KEY_ID_BYTES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedKey {
    pub alg: String,
    pub id: String,
    pub public: IdentityKey,
}

impl PublishedKey {
    pub fn of(public: IdentityKey) -> Self {
        Self { alg: ALGORITHM.into(), id: key_id(&public), public }
    }

    pub fn is_well_formed(&self) -> bool {
        self.alg == ALGORITHM && self.id == key_id(&self.public)
    }

    pub fn verifies(&self, purpose: &str, origin: &str, payload: &[u8], sig: &Signature) -> bool {
        self.is_well_formed() && verify(&self.public, purpose, origin, payload, sig)
    }
}

pub fn key_id(public: &IdentityKey) -> String {
    let digest = Sha256::digest(public.0);
    crate::identity::to_hex(&digest[..KEY_ID_BYTES])
}

pub fn signed_bytes(purpose: &str, origin: &str, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(SIGNING_DOMAIN.len() + purpose.len() + origin.len() + payload.len() + 3);
    for part in [SIGNING_DOMAIN.as_bytes(), purpose.as_bytes(), origin.as_bytes()] {
        bytes.extend_from_slice(part);
        bytes.push(b'\n');
    }
    bytes.extend_from_slice(payload);
    bytes
}

pub fn sign(key: &Identity, purpose: &str, origin: &str, payload: &[u8]) -> Signature {
    key.sign(&signed_bytes(purpose, origin, payload))
}

pub fn verify(public: &IdentityKey, purpose: &str, origin: &str, payload: &[u8], sig: &Signature) -> bool {
    public.verify(&signed_bytes(purpose, origin, payload), sig)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_published_key_names_itself_by_its_hash() {
        let key = Identity::from_bytes(&[3; 32]);
        let published = PublishedKey::of(key.public());
        assert_eq!(published.id.len(), KEY_ID_BYTES * 2);
        assert!(published.is_well_formed());
        let renamed = PublishedKey { id: "0000000000000000".into(), ..published.clone() };
        assert!(!renamed.is_well_formed());
        let other_alg = PublishedKey { alg: "rsa".into(), ..published };
        assert!(!other_alg.is_well_formed());
    }

    #[test]
    fn a_signature_only_counts_for_its_purpose_origin_and_payload() {
        let key = Identity::from_bytes(&[4; 32]);
        let published = PublishedKey::of(key.public());
        let sig = sign(&key, "home-card", "https://commy.chat", b"payload");
        assert!(published.verifies("home-card", "https://commy.chat", b"payload", &sig));
        assert!(!published.verifies("push", "https://commy.chat", b"payload", &sig));
        assert!(!published.verifies("home-card", "https://cornmy.chat", b"payload", &sig));
        assert!(!published.verifies("home-card", "https://commy.chat", b"payloaD", &sig));
        let stranger = PublishedKey::of(Identity::from_bytes(&[5; 32]).public());
        assert!(!stranger.verifies("home-card", "https://commy.chat", b"payload", &sig));
    }

    #[test]
    fn the_signed_bytes_match_the_account_server() {
        let bytes = signed_bytes("home-card", "https://commy.chat", b"x");
        assert_eq!(bytes, b"commy-server/v1\nhome-card\nhttps://commy.chat\nx");
    }

    #[test]
    fn keys_and_signatures_match_the_account_server() {
        let key = Identity::from_bytes(&[3; 32]);
        let published = PublishedKey::of(key.public());
        assert_eq!(published.public.to_hex(), "ed4928c628d1c2c6eae90338905995612959273a5c63f93636c14614ac8737d1");
        assert_eq!(published.id, "b62e867fa2f33afe");
        let from_node = Signature::from_hex(concat!(
            "b9ff402b0c6de502e75c273b8a296d8b77dd4d1ce51f2728d822c2def4373add",
            "344cb18838dc148765f69f45d2d57ddf27cfcbb5eaef3cb635ff95f18e1dca08"
        ))
        .unwrap();
        assert!(published.verifies("home-card", "https://commy.chat", b"x", &from_node));
        assert_eq!(sign(&key, "home-card", "https://commy.chat", b"x"), from_node);
    }

    #[test]
    fn a_published_key_reads_the_discovery_shape() {
        let key = Identity::from_bytes(&[6; 32]).public();
        let json = format!(r#"{{"alg":"ed25519","id":"{}","public":"{}"}}"#, key_id(&key), key.to_hex());
        let parsed: PublishedKey = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, PublishedKey::of(key));
    }
}
