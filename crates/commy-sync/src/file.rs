use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::stream::{SealError, LENGTH_PREFIX_BYTES, NONCE_BYTES, TAG_BYTES};

const FILE_DOMAIN: &[u8] = b"commy-sync-file/v1";
const KIB: usize = 1024;
const MIB: usize = 1024 * KIB;

pub const MAX_FILE_PLAIN: usize = 30 * MIB;
pub const FILE_BUCKETS: [usize; 10] = [64 * KIB, 256 * KIB, MIB, 2 * MIB, 4 * MIB, 8 * MIB, 12 * MIB, 16 * MIB, 24 * MIB, MAX_FILE_PLAIN + LENGTH_PREFIX_BYTES];
pub const MAX_FILE_SEALED: usize = FILE_BUCKETS[FILE_BUCKETS.len() - 1] + TAG_BYTES;

fn mac(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

#[derive(Clone, PartialEq, Eq)]
pub struct FileKey {
    pub id: [u8; 32],
    seal: [u8; 32],
}

impl std::fmt::Debug for FileKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileKey").field("id", &self.id_hex()).finish_non_exhaustive()
    }
}

impl FileKey {
    pub fn for_blob(blob: &str) -> Self {
        let root = mac(blob.as_bytes(), FILE_DOMAIN);
        Self { id: mac(&root, b"id"), seal: mac(&root, b"seal") }
    }

    pub fn id_hex(&self) -> String {
        data_encoding::HEXLOWER.encode(&self.id)
    }

    pub fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, SealError> {
        let bucket = sealed_bucket(plain.len()).ok_or(SealError::TooLarge)?;
        let mut padded = Vec::with_capacity(bucket + TAG_BYTES);
        padded.extend_from_slice(&(plain.len() as u32).to_le_bytes());
        padded.extend_from_slice(plain);
        padded.resize(bucket, 0);
        let cipher = Aes256Gcm::new_from_slice(&self.seal).expect("32 byte key");
        Ok(cipher
            .encrypt(&Nonce::from([0; NONCE_BYTES]), Payload { msg: &padded, aad: &self.id })
            .expect("in-memory encryption cannot fail"))
    }

    pub fn open(&self, body: &[u8]) -> Result<Vec<u8>, SealError> {
        if body.len() > MAX_FILE_SEALED {
            return Err(SealError::TooLarge);
        }
        let cipher = Aes256Gcm::new_from_slice(&self.seal).expect("32 byte key");
        let mut padded = cipher
            .decrypt(&Nonce::from([0; NONCE_BYTES]), Payload { msg: body, aad: &self.id })
            .map_err(|_| SealError::WrongKey)?;
        let prefix: [u8; LENGTH_PREFIX_BYTES] = padded.get(..LENGTH_PREFIX_BYTES).ok_or(SealError::WrongKey)?.try_into().map_err(|_| SealError::WrongKey)?;
        let len = u32::from_le_bytes(prefix) as usize;
        if padded.len() < LENGTH_PREFIX_BYTES + len {
            return Err(SealError::WrongKey);
        }
        padded.truncate(LENGTH_PREFIX_BYTES + len);
        padded.drain(..LENGTH_PREFIX_BYTES);
        Ok(padded)
    }
}

fn sealed_bucket(plain_len: usize) -> Option<usize> {
    FILE_BUCKETS.iter().copied().find(|b| plain_len + LENGTH_PREFIX_BYTES <= *b)
}

pub fn file_id(text: &str) -> Option<[u8; 32]> {
    let bytes = data_encoding::HEXLOWER.decode(text.as_bytes()).ok()?;
    bytes.try_into().ok()
}

pub fn fits(sealed_len: usize) -> bool {
    FILE_BUCKETS.iter().any(|bucket| bucket + TAG_BYTES == sealed_len)
}

pub fn fits_plain(plain_len: usize) -> bool {
    sealed_bucket(plain_len).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_opens_only_with_the_reference_it_was_sealed_for() {
        let key = FileKey::for_blob("abc123");
        assert_eq!(key, FileKey::for_blob("abc123"));
        let other = FileKey::for_blob("abc124");
        assert_ne!(key.id, other.id);
        let sealed = key.seal(b"a pasted picture").unwrap();
        assert_eq!(sealed, key.seal(b"a pasted picture").unwrap(), "every member who has the file seals it the same way");
        assert!(fits(sealed.len()));
        assert_eq!(sealed.len(), FILE_BUCKETS[0] + TAG_BYTES, "sizes are rounded up so the service learns little");
        assert_eq!(key.open(&sealed).unwrap(), b"a pasted picture");
        assert_eq!(other.open(&sealed), Err(SealError::WrongKey));
        let mut bent = sealed.clone();
        bent[3] ^= 1;
        assert_eq!(key.open(&bent), Err(SealError::WrongKey));
        assert_eq!(file_id(&key.id_hex()), Some(key.id));
        assert_eq!(file_id("zz"), None);
    }

    #[test]
    fn files_and_pictures_never_share_an_id() {
        assert_ne!(FileKey::for_blob("same").id, crate::PictureKey::for_blob("same").id);
    }

    #[test]
    fn files_up_to_the_limit_fit() {
        let key = FileKey::for_blob("big");
        let largest = vec![7u8; MAX_FILE_PLAIN];
        let sealed = key.seal(&largest).unwrap();
        assert_eq!(sealed.len(), MAX_FILE_SEALED);
        assert_eq!(key.open(&sealed).unwrap(), largest);
        assert!(fits_plain(MAX_FILE_PLAIN));
        assert!(!fits_plain(MAX_FILE_PLAIN + 1));
        assert_eq!(key.seal(&vec![0u8; MAX_FILE_PLAIN + 1]), Err(SealError::TooLarge));
        assert!(!fits(MAX_FILE_SEALED + 1));
        assert!(!fits(1000));
    }

    #[test]
    fn a_middle_sized_file_lands_in_the_next_bucket_up() {
        let key = FileKey::for_blob("photo");
        let photo = vec![1u8; 3 * MIB];
        let sealed = key.seal(&photo).unwrap();
        assert_eq!(sealed.len(), 4 * MIB + TAG_BYTES);
        assert_eq!(key.open(&sealed).unwrap(), photo);
    }

    #[test]
    fn the_storage_worker_takes_the_same_sizes() {
        let worker = include_str!("../../../apps/file-store/worker.js");
        assert!(worker.contains("export const MAX_FILE_PLAIN = 30 * MIB;"));
        assert!(worker.contains("[64 * KIB, 256 * KIB, MIB, 2 * MIB, 4 * MIB, 8 * MIB, 12 * MIB, 16 * MIB, 24 * MIB, MAX_FILE_PLAIN + LENGTH_PREFIX_BYTES]"));
        assert_eq!(MAX_FILE_PLAIN, 30 * MIB);
        assert_eq!(FILE_BUCKETS, [64 * KIB, 256 * KIB, MIB, 2 * MIB, 4 * MIB, 8 * MIB, 12 * MIB, 16 * MIB, 24 * MIB, MAX_FILE_PLAIN + LENGTH_PREFIX_BYTES]);
    }
}
