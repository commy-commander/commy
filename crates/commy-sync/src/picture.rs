use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::stream::{SealError, LENGTH_PREFIX_BYTES, NONCE_BYTES, TAG_BYTES};

const PICTURE_DOMAIN: &[u8] = b"commy-sync-picture/v1";

pub const MAX_PICTURE_PLAIN: usize = 1024 * 1024;
pub const PICTURE_BUCKETS: [usize; 4] = [16 * 1024, 64 * 1024, 256 * 1024, MAX_PICTURE_PLAIN + LENGTH_PREFIX_BYTES];
pub const MAX_PICTURE_SEALED: usize = PICTURE_BUCKETS[PICTURE_BUCKETS.len() - 1] + TAG_BYTES;

fn mac(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

#[derive(Clone, PartialEq, Eq)]
pub struct PictureKey {
    pub id: [u8; 32],
    seal: [u8; 32],
}

impl std::fmt::Debug for PictureKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PictureKey").field("id", &self.id_hex()).finish_non_exhaustive()
    }
}

impl PictureKey {
    pub fn for_blob(blob: &str) -> Self {
        let root = mac(PICTURE_DOMAIN, blob.as_bytes());
        Self { id: mac(&root, b"id"), seal: mac(&root, b"seal") }
    }

    pub fn id_hex(&self) -> String {
        data_encoding::HEXLOWER.encode(&self.id)
    }

    pub fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, SealError> {
        let bucket = PICTURE_BUCKETS.iter().copied().find(|b| plain.len() + LENGTH_PREFIX_BYTES <= *b).ok_or(SealError::TooLarge)?;
        let mut padded = Vec::with_capacity(bucket);
        padded.extend_from_slice(&(plain.len() as u32).to_le_bytes());
        padded.extend_from_slice(plain);
        padded.resize(bucket, 0);
        let cipher = Aes256Gcm::new_from_slice(&self.seal).expect("32 byte key");
        Ok(cipher
            .encrypt(&Nonce::from([0; NONCE_BYTES]), Payload { msg: &padded, aad: &self.id })
            .expect("in-memory encryption cannot fail"))
    }

    pub fn open(&self, body: &[u8]) -> Result<Vec<u8>, SealError> {
        if body.len() > MAX_PICTURE_SEALED {
            return Err(SealError::TooLarge);
        }
        let cipher = Aes256Gcm::new_from_slice(&self.seal).expect("32 byte key");
        let padded = cipher
            .decrypt(&Nonce::from([0; NONCE_BYTES]), Payload { msg: body, aad: &self.id })
            .map_err(|_| SealError::WrongKey)?;
        let len = u32::from_le_bytes(padded.get(..LENGTH_PREFIX_BYTES).ok_or(SealError::WrongKey)?.try_into().map_err(|_| SealError::WrongKey)?) as usize;
        padded.get(LENGTH_PREFIX_BYTES..LENGTH_PREFIX_BYTES + len).map(<[u8]>::to_vec).ok_or(SealError::WrongKey)
    }
}

pub fn picture_id(text: &str) -> Option<[u8; 32]> {
    let bytes = data_encoding::HEXLOWER.decode(text.as_bytes()).ok()?;
    bytes.try_into().ok()
}

pub fn fits(sealed_len: usize) -> bool {
    PICTURE_BUCKETS.iter().any(|bucket| bucket + TAG_BYTES == sealed_len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_opens_only_with_the_reference_it_was_sealed_for() {
        let key = PictureKey::for_blob("abc123");
        assert_eq!(key, PictureKey::for_blob("abc123"));
        let other = PictureKey::for_blob("abc124");
        assert_ne!(key.id, other.id);
        let sealed = key.seal(b"a tiny picture").unwrap();
        assert_eq!(sealed, key.seal(b"a tiny picture").unwrap(), "the same picture seals the same way on every device");
        assert!(fits(sealed.len()));
        assert_eq!(sealed.len(), PICTURE_BUCKETS[0] + TAG_BYTES, "sizes are rounded up so the service learns little");
        assert_eq!(key.open(&sealed).unwrap(), b"a tiny picture");
        assert_eq!(other.open(&sealed), Err(SealError::WrongKey));
        let mut bent = sealed.clone();
        bent[3] ^= 1;
        assert_eq!(key.open(&bent), Err(SealError::WrongKey));
        assert_eq!(picture_id(&key.id_hex()), Some(key.id));
        assert_eq!(picture_id("zz"), None);
    }

    #[test]
    fn pictures_up_to_a_megabyte_fit() {
        let key = PictureKey::for_blob("big");
        let largest = vec![7u8; MAX_PICTURE_PLAIN];
        let sealed = key.seal(&largest).unwrap();
        assert_eq!(sealed.len(), MAX_PICTURE_SEALED);
        assert_eq!(key.open(&sealed).unwrap(), largest);
        assert_eq!(key.seal(&vec![0u8; MAX_PICTURE_PLAIN + 1]), Err(SealError::TooLarge));
        assert!(!fits(MAX_PICTURE_SEALED + 1));
    }
}
