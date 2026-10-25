use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use data_encoding::BASE64URL_NOPAD;
use rand::Rng;

pub const SEALED_PREFIX: &str = "sealed1.";
const NONCE_LEN: usize = 12;
pub const KEY_TEXT_LEN: usize = 43;
pub const SHORT_KEY_TEXT_LEN: usize = 22;
const SHORT_KEY_BYTES: usize = 16;
const SHORT_KEY_DOMAIN: &[u8] = b"commy-short-key-v1";
const INVITE_LINK_KEY_DOMAIN: &[u8] = b"commy-invite-link-key-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Invite,
    EntryList,
    InviteBackground,
    ReadState,
    KeyHandoff,
    EntryKeyEscrow,
}

impl Purpose {
    fn aad(self) -> &'static [u8] {
        match self {
            Purpose::Invite => b"commy-invite-v1",
            Purpose::EntryList => b"commy-entry-list-v1",
            Purpose::InviteBackground => b"commy-invite-bg-v1",
            Purpose::ReadState => b"commy-read-state-v1",
            Purpose::KeyHandoff => b"commy-key-handoff-v1",
            Purpose::EntryKeyEscrow => b"commy-entry-key-escrow-v1",
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SealError {
    #[error("this is not a sealed value")]
    NotSealed,
    #[error("the key does not open this value")]
    WrongKey,
    #[error("not a valid key")]
    BadKey,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SealKey([u8; 32]);

impl std::fmt::Debug for SealKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SealKey(..)")
    }
}

impl SealKey {
    pub fn generate() -> Self {
        Self(rand::rngs::OsRng.gen())
    }

    pub(crate) fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn generate_short() -> (Self, String) {
        let seed: [u8; SHORT_KEY_BYTES] = rand::rngs::OsRng.gen();
        let text = BASE64URL_NOPAD.encode(&seed);
        (Self::stretch(&seed), text)
    }

    pub fn for_invite_link(revoke_key: &str, short: bool) -> (Self, String) {
        use sha2::Digest;
        let mut hash = sha2::Sha256::new();
        hash.update(INVITE_LINK_KEY_DOMAIN);
        hash.update(revoke_key.as_bytes());
        let bytes: [u8; 32] = hash.finalize().into();
        if short {
            let seed = &bytes[..SHORT_KEY_BYTES];
            (Self::stretch(seed), BASE64URL_NOPAD.encode(seed))
        } else {
            let key = Self(bytes);
            let text = key.to_text();
            (key, text)
        }
    }

    fn stretch(seed: &[u8]) -> Self {
        use sha2::Digest;
        let mut hash = sha2::Sha256::new();
        hash.update(SHORT_KEY_DOMAIN);
        hash.update(seed);
        Self(hash.finalize().into())
    }

    pub fn bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_text(&self) -> String {
        BASE64URL_NOPAD.encode(&self.0)
    }

    pub fn from_text(text: &str) -> Result<Self, SealError> {
        let text = text.trim();
        if text.len() == SHORT_KEY_TEXT_LEN {
            let seed = BASE64URL_NOPAD.decode(text.as_bytes()).map_err(|_| SealError::BadKey)?;
            return Ok(Self::stretch(&seed));
        }
        if text.len() != KEY_TEXT_LEN {
            return Err(SealError::BadKey);
        }
        let bytes = BASE64URL_NOPAD
            .decode(text.as_bytes())
            .map_err(|_| SealError::BadKey)?;
        Ok(Self(bytes.try_into().map_err(|_| SealError::BadKey)?))
    }

    pub fn seal(&self, purpose: Purpose, plaintext: &[u8]) -> String {
        let nonce: [u8; NONCE_LEN] = rand::rngs::OsRng.gen();
        let cipher = Aes256Gcm::new_from_slice(&self.0).expect("32 byte key");
        let ciphertext = cipher
            .encrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: plaintext,
                    aad: purpose.aad(),
                },
            )
            .expect("AES-GCM encryption cannot fail for in-memory input");
        let mut bytes = nonce.to_vec();
        bytes.extend(ciphertext);
        format!("{SEALED_PREFIX}{}", BASE64URL_NOPAD.encode(&bytes))
    }

    pub fn open(&self, purpose: Purpose, sealed: &str) -> Result<Vec<u8>, SealError> {
        let encoded = sealed
            .trim()
            .strip_prefix(SEALED_PREFIX)
            .ok_or(SealError::NotSealed)?;
        let bytes = BASE64URL_NOPAD
            .decode(encoded.as_bytes())
            .map_err(|_| SealError::NotSealed)?;
        if bytes.len() < NONCE_LEN + 16 {
            return Err(SealError::NotSealed);
        }
        let (nonce, ciphertext) = bytes.split_at(NONCE_LEN);
        let nonce = Nonce::try_from(nonce).map_err(|_| SealError::NotSealed)?;
        let cipher = Aes256Gcm::new_from_slice(&self.0).expect("32 byte key");
        cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: ciphertext,
                    aad: purpose.aad(),
                },
            )
            .map_err(|_| SealError::WrongKey)
    }
}

pub fn is_sealed(value: &str) -> bool {
    value.trim().starts_with(SEALED_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let key = SealKey::generate();
        let sealed = key.seal(Purpose::Invite, b"hello");
        assert!(is_sealed(&sealed));
        assert!(!sealed.contains("hello"));
        assert_eq!(key.open(Purpose::Invite, &sealed).unwrap(), b"hello");
    }

    #[test]
    fn short_keys_open_what_they_sealed_and_nothing_else() {
        let (key, text) = SealKey::generate_short();
        assert_eq!(text.len(), SHORT_KEY_TEXT_LEN);
        let sealed = key.seal(Purpose::Invite, b"hello");
        assert_eq!(SealKey::from_text(&text).unwrap().open(Purpose::Invite, &sealed).unwrap(), b"hello");
        let (_, other) = SealKey::generate_short();
        assert_eq!(SealKey::from_text(&other).unwrap().open(Purpose::Invite, &sealed), Err(SealError::WrongKey));
        assert_eq!(SealKey::from_text(&"*".repeat(SHORT_KEY_TEXT_LEN)), Err(SealError::BadKey));
    }

    #[test]
    fn short_keys_stretch_the_same_way_the_website_does() {
        let seed: Vec<u8> = (0u8..16).collect();
        let key = SealKey::from_text(&BASE64URL_NOPAD.encode(&seed)).unwrap();
        assert_eq!(key.to_text(), "DGRsa06xoR5OUNXHMW5Duet9o5gSIQIugDHjgdk8l3k");
    }

    #[test]
    fn invite_link_keys_come_back_from_the_same_revoke_key() {
        for short in [true, false] {
            let (key, text) = SealKey::for_invite_link("revoke", short);
            assert_eq!(text.len(), if short { SHORT_KEY_TEXT_LEN } else { KEY_TEXT_LEN });
            assert_eq!(SealKey::from_text(&text).unwrap(), key);
            assert_eq!(SealKey::for_invite_link("revoke", short).1, text);
            assert_ne!(SealKey::for_invite_link("other", short).1, text);
        }
    }

    #[test]
    fn purposes_do_not_mix() {
        let key = SealKey::generate();
        let sealed = key.seal(Purpose::Invite, b"hello");
        assert_eq!(
            key.open(Purpose::EntryList, &sealed),
            Err(SealError::WrongKey)
        );
    }

    #[test]
    fn each_seal_differs() {
        let key = SealKey::generate();
        assert_ne!(
            key.seal(Purpose::Invite, b"x"),
            key.seal(Purpose::Invite, b"x")
        );
    }

    #[test]
    fn other_keys_and_tampering_fail() {
        let key = SealKey::generate();
        let sealed = key.seal(Purpose::Invite, b"hello");
        assert_eq!(
            SealKey::generate().open(Purpose::Invite, &sealed),
            Err(SealError::WrongKey)
        );
        let mut tampered = sealed.clone().into_bytes();
        let last = tampered.len() - 1;
        tampered[last] = if tampered[last] == b'A' { b'B' } else { b'A' };
        assert_eq!(
            key.open(Purpose::Invite, std::str::from_utf8(&tampered).unwrap()),
            Err(SealError::WrongKey)
        );
        assert_eq!(
            key.open(Purpose::Invite, "commyabc"),
            Err(SealError::NotSealed)
        );
        assert_eq!(
            key.open(Purpose::Invite, "sealed1.AAAA"),
            Err(SealError::NotSealed)
        );
    }

    #[test]
    fn keys_round_trip_as_text() {
        let key = SealKey::generate();
        let text = key.to_text();
        assert_eq!(text.len(), KEY_TEXT_LEN);
        assert!(text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_eq!(SealKey::from_text(&text).unwrap(), key);
        assert_eq!(SealKey::from_text("short"), Err(SealError::BadKey));
        assert_eq!(
            SealKey::from_text(&"!".repeat(KEY_TEXT_LEN)),
            Err(SealError::BadKey)
        );
    }

    #[test]
    fn opens_a_webcrypto_vector() {
        let key = SealKey((0u8..32).collect::<Vec<_>>().try_into().unwrap());
        let sealed = "sealed1.BwcHBwcHBwcHBwcHdEjIflUth52--H-g0Dib9cQ5_wCqijSd9uZ65jYZBCaQN0hGsV0";
        assert_eq!(
            key.open(Purpose::Invite, sealed).unwrap(),
            br#"{"n":"Pixel Pioneers"}"#
        );
    }
}
