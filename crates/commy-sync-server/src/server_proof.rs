use commy_core::lock_or_recover::LockOrRecover;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::Duration;

use sha2::{Digest, Sha256};

pub const SIGNED_PREFIX: &str = "commy-server/v1";
pub const PUSH_PURPOSE: &str = "push";
pub const DISCOVERY_PATH: &str = "/sync/about";
pub const KEY_KEPT_MS: i64 = 60 * 60 * 1000;
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_DISCOVERY_BYTES: usize = 64 * 1024;
const MAX_ORIGIN_CHARS: usize = 253 + "https://".len();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedKey {
    pub id: String,
    pub public: [u8; 32],
}

pub fn key_id(public: &[u8; 32]) -> String {
    data_encoding::HEXLOWER.encode(&Sha256::digest(public)[..8])
}

pub fn clean_origin(text: &str) -> Option<String> {
    let host = text.trim().strip_prefix("https://")?.trim_end_matches('/').to_ascii_lowercase();
    let labels: Vec<&str> = host.split('.').collect();
    let plain = labels.len() >= 2
        && labels.iter().all(|label| !label.is_empty() && label.len() <= 63 && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        && labels.iter().all(|label| !label.starts_with('-') && !label.ends_with('-'));
    let numeric = labels.last().is_some_and(|tld| tld.bytes().all(|b| b.is_ascii_digit()));
    let private = ["localhost", "local", "internal", "lan", "home", "arpa", "test", "invalid", "example"].contains(labels.last()?);
    let origin = format!("https://{host}");
    (plain && !numeric && !private && origin.len() <= MAX_ORIGIN_CHARS).then_some(origin)
}

pub fn signed_bytes(purpose: &str, origin: &str, payload: &str) -> Vec<u8> {
    format!("{SIGNED_PREFIX}\n{purpose}\n{origin}\n{payload}").into_bytes()
}

pub fn push_payload(at_ms: i64, body: &[u8]) -> String {
    format!("{at_ms}\n{}", data_encoding::HEXLOWER.encode(&Sha256::digest(body)))
}

pub fn verify(key: &PublishedKey, purpose: &str, origin: &str, payload: &str, signature_hex: &str) -> bool {
    let Ok(signature) = data_encoding::HEXLOWER_PERMISSIVE.decode(signature_hex.trim().as_bytes()) else { return false };
    let public = ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key.public);
    public.verify(&signed_bytes(purpose, origin, payload), &signature).is_ok()
}

pub fn parse_discovery(body: &[u8]) -> Option<PublishedKey> {
    let doc: serde_json::Value = serde_json::from_slice(body).ok()?;
    let key = doc.get("serverKey")?;
    if key.get("alg")?.as_str()? != "ed25519" {
        return None;
    }
    let public: [u8; 32] = data_encoding::HEXLOWER_PERMISSIVE.decode(key.get("public")?.as_str()?.as_bytes()).ok()?.try_into().ok()?;
    let id = key_id(&public);
    (key.get("id")?.as_str()? == id).then_some(PublishedKey { id, public })
}

pub fn publicly_routable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_documentation()
                || a == 0
                || (a == 100 && (64..128).contains(&b))
                || (a == 198 && (18..20).contains(&b))
                || a >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return publicly_routable(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            !(v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80 || first == 0x2001 && v6.segments()[1] == 0x0db8)
        }
    }
}

pub enum KeySource {
    Web,
    Fixed(HashMap<String, PublishedKey>),
}

impl KeySource {
    async fn fetch(&self, origin: &str) -> Result<PublishedKey, String> {
        match self {
            KeySource::Fixed(keys) => keys.get(origin).cloned().ok_or_else(|| "no key published".to_string()),
            KeySource::Web => fetch_published_key(origin).await,
        }
    }
}

async fn fetch_published_key(origin: &str) -> Result<PublishedKey, String> {
    let host = origin.strip_prefix("https://").ok_or("not an https origin")?;
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, 443)).await.map_err(|e| format!("cannot resolve {host}: {e}"))?.collect();
    let addr = match addrs.as_slice() {
        [] => return Err(format!("{host} has no address")),
        all if all.iter().all(|a| publicly_routable(a.ip())) => all[0],
        _ => return Err(format!("{host} points at a private address")),
    };
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(FETCH_TIMEOUT)
        .resolve(host, addr)
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client.get(format!("{origin}{DISCOVERY_PATH}")).send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("{DISCOVERY_PATH} answered {}", response.status()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_DISCOVERY_BYTES {
            return Err(format!("{DISCOVERY_PATH} is too large"));
        }
    }
    parse_discovery(&body).ok_or_else(|| format!("{DISCOVERY_PATH} publishes no usable serverKey"))
}

pub struct KeyBook {
    source: KeySource,
    kept: Mutex<HashMap<String, (PublishedKey, i64)>>,
}

impl KeyBook {
    pub fn new(source: KeySource) -> Self {
        Self { source, kept: Mutex::new(HashMap::new()) }
    }

    async fn fresh(&self, origin: &str, now_ms: i64) -> Result<PublishedKey, String> {
        let key = self.source.fetch(origin).await?;
        self.kept.lock_or_recover().insert(origin.to_string(), (key.clone(), now_ms));
        Ok(key)
    }

    fn kept(&self, origin: &str, now_ms: i64) -> Option<PublishedKey> {
        let kept = self.kept.lock_or_recover();
        kept.get(origin).filter(|(_, at)| now_ms.saturating_sub(*at) < KEY_KEPT_MS).map(|(key, _)| key.clone())
    }

    pub async fn check(&self, origin: &str, payload: &str, signature: &str, now_ms: i64) -> Result<(), String> {
        if let Some(key) = self.kept(origin, now_ms) {
            if verify(&key, PUSH_PURPOSE, origin, payload, signature) {
                return Ok(());
            }
        }
        let key = self.fresh(origin, now_ms).await?;
        if verify(&key, PUSH_PURPOSE, origin, payload, signature) {
            Ok(())
        } else {
            Err("the signature does not match the server's published key".into())
        }
    }

    pub fn forget_old(&self, now_ms: i64) {
        self.kept.lock_or_recover().retain(|_, (_, at)| now_ms.saturating_sub(*at) < KEY_KEPT_MS);
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    pub fn key_pair(seed: u8) -> (Ed25519KeyPair, PublishedKey) {
        let pair = Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap();
        let public: [u8; 32] = pair.public_key().as_ref().try_into().unwrap();
        (pair, PublishedKey { id: key_id(&public), public })
    }

    pub fn sign(pair: &Ed25519KeyPair, origin: &str, payload: &str) -> String {
        data_encoding::HEXLOWER.encode(pair.sign(&signed_bytes(PUSH_PURPOSE, origin, payload)).as_ref())
    }

    #[test]
    fn origins_are_public_https_domains() {
        assert_eq!(clean_origin(" https://Home.Example.org/ ").as_deref(), Some("https://home.example.org"));
        for bad in [
            "http://home.example.org",
            "https://home",
            "https://home.example.org:8443",
            "https://home.example.org/path",
            "https://192.168.1.1",
            "https://box.local",
            "https://localhost",
            "https://u@home.example.org",
            "https://-bad.example.org",
            "https://a..b.org",
            "https://home.example",
        ] {
            assert_eq!(clean_origin(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_signature_covers_the_purpose_origin_time_and_body() {
        let (pair, key) = key_pair(7);
        let origin = "https://home.example.org";
        let payload = push_payload(1_000, b"{\"pushes\":[]}");
        let signature = sign(&pair, origin, &payload);
        assert!(verify(&key, PUSH_PURPOSE, origin, &payload, &signature));
        assert!(!verify(&key, "home-card", origin, &payload, &signature), "a signature for another use");
        assert!(!verify(&key, PUSH_PURPOSE, "https://other.example.org", &payload, &signature));
        assert!(!verify(&key, PUSH_PURPOSE, origin, &push_payload(1_001, b"{\"pushes\":[]}"), &signature));
        assert!(!verify(&key, PUSH_PURPOSE, origin, &push_payload(1_000, b"{}"), &signature));
        assert!(!verify(&key, PUSH_PURPOSE, origin, &payload, "zz"));
        let (_, other) = key_pair(8);
        assert!(!verify(&other, PUSH_PURPOSE, origin, &payload, &signature));
    }

    #[test]
    fn keys_published_by_a_commy_server_are_read_and_their_signatures_hold() {
        let identity = commy_core::Identity::from_bytes(&[5; 32]);
        let about = serde_json::json!({ "operator": "x", "serverKey": commy_core::server_key::PublishedKey::of(identity.public()) });
        let key = parse_discovery(&serde_json::to_vec(&about).unwrap()).expect("the published key is read");
        assert_eq!(key, key_pair(5).1);
        let payload = push_payload(1_700_000_000_000, b"{}");
        let signature = commy_core::server_key::sign(&identity, PUSH_PURPOSE, "https://sync.example.org", payload.as_bytes());
        assert!(verify(&key, PUSH_PURPOSE, "https://sync.example.org", &payload, &signature.to_hex()));
    }

    #[test]
    fn the_discovery_document_must_name_its_own_key() {
        let (_, key) = key_pair(3);
        let public = data_encoding::HEXLOWER.encode(&key.public);
        let doc = |alg: &str, id: &str| serde_json::json!({ "serverKey": { "alg": alg, "id": id, "public": public } }).to_string();
        assert_eq!(parse_discovery(doc("ed25519", &key.id).as_bytes()), Some(key.clone()));
        assert_eq!(parse_discovery(doc("ed25519", "0000000000000000").as_bytes()), None);
        assert_eq!(parse_discovery(doc("rsa", &key.id).as_bytes()), None);
        assert_eq!(parse_discovery(b"{}"), None);
        assert_eq!(parse_discovery(b"not json"), None);
    }

    #[test]
    fn only_public_addresses_are_fetched() {
        for private in ["10.0.0.1", "127.0.0.1", "169.254.1.1", "192.168.0.1", "172.16.0.1", "100.64.0.1", "0.0.0.0", "::1", "fd00::1", "fe80::1", "::ffff:10.0.0.1"] {
            assert!(!publicly_routable(private.parse().unwrap()), "{private}");
        }
        for public in ["1.1.1.1", "2606:4700:4700::1111"] {
            assert!(publicly_routable(public.parse().unwrap()), "{public}");
        }
    }

    #[tokio::test]
    async fn a_rotated_key_is_fetched_again_once() {
        let origin = "https://home.example.org";
        let (old_pair, old) = key_pair(1);
        let (new_pair, new) = key_pair(2);
        let book = KeyBook::new(KeySource::Fixed(HashMap::from([(origin.to_string(), new)])));
        book.kept.lock().unwrap().insert(origin.to_string(), (old, 0));
        let payload = push_payload(5, b"x");
        assert!(book.check(origin, &payload, &sign(&old_pair, origin, &payload), 10).await.is_ok(), "the kept key still works");
        assert!(book.check(origin, &payload, &sign(&new_pair, origin, &payload), 10).await.is_ok(), "a miss fetches the new key");
        assert!(book.check(origin, &payload, &sign(&old_pair, origin, &payload), 10).await.is_err(), "the old key is gone");
        assert!(book.check("https://unknown.example.org", &payload, "00", 10).await.is_err());
    }
}
