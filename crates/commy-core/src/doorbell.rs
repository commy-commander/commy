//! Message doorbell: content-free wake-ups for messages a device missed.
//!
//! Each device keeps a random key per server and shares it with that
//! server's members only. Senders ring the device's daily token on its
//! account server with blinded ids of the messages it missed; the device
//! takes them and fetches the messages from its peers. Tokens and blinded
//! ids are keyed hashes, so the account server never learns which server,
//! account or message they belong to.

use crate::identity::{IdentityKey, Signature};
use crate::ids::{MessageId, ServerId, UserId};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub type DoorbellKey = [u8; 32];

const TOKEN_LABEL: &[u8] = b"commy-doorbell/v1/token";
const ID_LABEL: &[u8] = b"commy-doorbell/v1/id";
const SET_LABEL: &str = "commy-doorbell-set/v1";

/// Most blinded ids one ring carries; the account server refuses more.
pub const MAX_RING_IDS: usize = 50;

/// Days the account server keeps a ring before deleting it.
pub const KEEP_DAYS: u32 = 14;

/// Days since the Unix epoch, in UTC: the day a token belongs to.
pub fn day_of(time: DateTime<Utc>) -> u32 {
    time.timestamp().div_euclid(86_400).max(0) as u32
}

fn keyed(key: &DoorbellKey, label: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(label);
    mac.update(data);
    mac.finalize().into_bytes().into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The token rings for `day` go to, 32 hex characters.
pub fn token(key: &DoorbellKey, day: u32) -> String {
    hex(&keyed(key, TOKEN_LABEL, &day.to_be_bytes())[..16])
}

/// A message id as the doorbell carries it, 24 hex characters. Only
/// holders of the key can tell which message it stands for.
pub fn blinded_id(key: &DoorbellKey, id: MessageId) -> String {
    hex(&keyed(key, ID_LABEL, id.0.as_bytes())[..12])
}

/// What a doorbell announcement signs, see [`crate::ServerEvent::DoorbellSet`].
pub fn set_signing_bytes(server: ServerId, user: UserId, node: &str, url: &str, key: Option<&DoorbellKey>) -> Vec<u8> {
    postcard::to_allocvec(&(SET_LABEL, server, user, node, url, key)).expect("postcard encodes plain data")
}

/// Whether a doorbell announcement was signed by `signer`.
pub fn verify_set(
    signer: &IdentityKey,
    server: ServerId,
    user: UserId,
    node: &str,
    url: &str,
    key: Option<&DoorbellKey>,
    sig: &Signature,
) -> bool {
    signer.verify(&set_signing_bytes(server, user, node, url, key), sig)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;

    #[test]
    fn tokens_and_ids_have_the_wire_shape() {
        let key = [7u8; 32];
        let t = token(&key, 20_000);
        assert_eq!(t.len(), 32);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(t, token(&key, 20_001), "each day has its own token");
        assert_ne!(t, token(&[8u8; 32], 20_000), "each key has its own token");
        let id = MessageId::new();
        let b = blinded_id(&key, id);
        assert_eq!(b.len(), 24);
        assert_eq!(b, blinded_id(&key, id));
        assert_ne!(b, blinded_id(&[8u8; 32], id));
        assert_ne!(&t[..24], b, "tokens and ids are hashed apart");
    }

    #[test]
    fn days_are_utc_days_since_the_epoch() {
        let t = DateTime::parse_from_rfc3339("2026-10-16T23:59:59Z").unwrap().with_timezone(&Utc);
        assert_eq!(day_of(t), 20_742);
        assert_eq!(day_of(t + chrono::Duration::seconds(1)), 20_743);
    }

    #[test]
    fn announcements_verify_only_unchanged() {
        let me = Identity::generate();
        let (server, user) = (ServerId::new(), UserId::new());
        let key = [3u8; 32];
        let sig = me.sign(&set_signing_bytes(server, user, "ab", "https://commy.chat", Some(&key)));
        assert!(verify_set(&me.public(), server, user, "ab", "https://commy.chat", Some(&key), &sig));
        assert!(!verify_set(&me.public(), server, user, "ab", "https://evil.example", Some(&key), &sig));
        assert!(!verify_set(&me.public(), server, user, "ab", "https://commy.chat", None, &sig));
        assert!(!verify_set(&me.public(), ServerId::new(), user, "ab", "https://commy.chat", Some(&key), &sig));
        assert!(!verify_set(&Identity::generate().public(), server, user, "ab", "https://commy.chat", Some(&key), &sig));
    }
}
