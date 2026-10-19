use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use data_encoding::BASE64URL_NOPAD;
use rand::RngCore;

pub const SEALED_PREFIX: &str = "sealed1.";
const NONCE_LEN: usize = 12;
pub const KEY_TEXT_LEN: usize = 43;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Invite,
    EntryList,
}

impl Purpose {
    fn aad(self) -> &'static [u8] {
        match self {
            Purpose::Invite => b"commy-invite-v1",
            Purpose::EntryList => b"commy-entry-list-v1",
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
        let mut key = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut key);
        Self(key)
    }

    pub fn to_text(&self) -> String {
        BASE64URL_NOPAD.encode(&self.0)
    }

    pub fn from_text(text: &str) -> Result<Self, SealError> {
        let text = text.trim();
        if text.len() != KEY_TEXT_LEN {
            return Err(SealError::BadKey);
        }
        let bytes = BASE64URL_NOPAD
            .decode(text.as_bytes())
            .map_err(|_| SealError::BadKey)?;
        Ok(Self(bytes.try_into().map_err(|_| SealError::BadKey)?))
    }

    pub fn seal(&self, purpose: Purpose, plaintext: &[u8]) -> String {
        let mut nonce = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
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
