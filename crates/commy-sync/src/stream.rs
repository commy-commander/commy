use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::fmt;

const DERIVE_SALT: &[u8] = b"commy-sync/v1";
const STREAM_ID_DOMAIN: &[u8] = b"commy-sync-stream-id/v1";
const APPEND_DOMAIN: &[u8] = b"commy-sync-append/v1";
const SEAL_DOMAIN: &[u8] = b"commy-sync-seal/v1";

pub const NONCE_BYTES: usize = 12;
pub const TAG_BYTES: usize = 16;
pub const LENGTH_PREFIX_BYTES: usize = 4;
pub const SIZE_BUCKETS: [usize; 5] = [512, 1024, 4096, 16 * 1024, 64 * 1024];
pub const MAX_PLAIN: usize = SIZE_BUCKETS[SIZE_BUCKETS.len() - 1] - LENGTH_PREFIX_BYTES;
pub const MAX_SEALED: usize = SIZE_BUCKETS[SIZE_BUCKETS.len() - 1] + NONCE_BYTES + TAG_BYTES;
pub const APPEND_NONCE_BYTES: usize = 16;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SealError {
    #[error("the entry is larger than a stream allows")]
    TooLarge,
    #[error("the entry was not sealed for this stream")]
    WrongKey,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StreamId(pub [u8; 32]);

impl StreamId {
    pub fn to_hex(&self) -> String {
        data_encoding::HEXLOWER.encode(&self.0)
    }

    pub fn from_hex(text: &str) -> Option<Self> {
        let bytes = data_encoding::HEXLOWER_PERMISSIVE.decode(text.as_bytes()).ok()?;
        Some(Self(bytes.try_into().ok()?))
    }

    pub fn of(write: &[u8; 32], read: &[u8; 32]) -> Self {
        let read_hash: [u8; 32] = Sha256::digest(read).into();
        let mut hash = Sha256::new();
        hash.update(STREAM_ID_DOMAIN);
        hash.update(write);
        hash.update(read_hash);
        Self(hash.finalize().into())
    }
}

impl fmt::Debug for StreamId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StreamId({})", &self.to_hex()[..12])
    }
}

impl fmt::Display for StreamId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct StreamSecret([u8; 32]);

impl fmt::Debug for StreamSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StreamSecret(..)")
    }
}

impl StreamSecret {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn random() -> Self {
        Self(rand::random())
    }

    pub fn derive(root: &[u8; 32], label: &str, context: &[u8]) -> Self {
        let mut info = Vec::with_capacity(label.len() + context.len() + 1);
        info.extend_from_slice(label.as_bytes());
        info.push(0);
        info.extend_from_slice(context);
        Self(hkdf(root, DERIVE_SALT, &info))
    }

    pub fn keys(&self) -> StreamKeys {
        let write = SigningKey::from_bytes(&hkdf(&self.0, DERIVE_SALT, b"write"));
        let read = hkdf(&self.0, DERIVE_SALT, b"read");
        let seal = hkdf(&self.0, DERIVE_SALT, b"seal");
        let id = StreamId::of(&write.verifying_key().to_bytes(), &read);
        StreamKeys { id, write, read, seal }
    }
}

fn hkdf(key: &[u8; 32], salt: &[u8], info: &[u8]) -> [u8; 32] {
    let mut extract = <Hmac<Sha256> as KeyInit>::new_from_slice(salt).expect("HMAC takes any key length");
    extract.update(key);
    let prk = extract.finalize().into_bytes();
    let mut expand = <Hmac<Sha256> as KeyInit>::new_from_slice(&prk).expect("HMAC takes any key length");
    expand.update(info);
    expand.update(&[1]);
    expand.finalize().into_bytes().into()
}

#[derive(Clone)]
pub struct StreamKeys {
    pub id: StreamId,
    write: SigningKey,
    read: [u8; 32],
    seal: [u8; 32],
}

impl fmt::Debug for StreamKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamKeys").field("id", &self.id).finish_non_exhaustive()
    }
}

impl PartialEq for StreamKeys {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl StreamKeys {
    pub fn write_public(&self) -> [u8; 32] {
        self.write.verifying_key().to_bytes()
    }

    pub fn read_token(&self) -> [u8; 32] {
        self.read
    }

    pub fn sign_append(&self, nonce: &[u8; APPEND_NONCE_BYTES], body: &[u8]) -> [u8; 64] {
        self.write.sign(&append_message(&self.id, nonce, body)).to_bytes()
    }

    pub fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, SealError> {
        let padded = pad(plain)?;
        let nonce: [u8; NONCE_BYTES] = rand::random();
        let cipher = Aes256Gcm::new_from_slice(&self.seal).expect("32 byte key");
        let sealed = cipher
            .encrypt(&Nonce::from(nonce), Payload { msg: &padded, aad: &self.aad() })
            .expect("in-memory encryption cannot fail");
        let mut out = Vec::with_capacity(NONCE_BYTES + sealed.len());
        out.extend_from_slice(&nonce);
        out.extend(sealed);
        Ok(out)
    }

    pub fn open(&self, body: &[u8]) -> Result<Vec<u8>, SealError> {
        if body.len() < NONCE_BYTES + TAG_BYTES {
            return Err(SealError::WrongKey);
        }
        let (nonce, sealed) = body.split_at(NONCE_BYTES);
        let nonce = Nonce::try_from(nonce).map_err(|_| SealError::WrongKey)?;
        let cipher = Aes256Gcm::new_from_slice(&self.seal).expect("32 byte key");
        let padded = cipher
            .decrypt(&nonce, Payload { msg: sealed, aad: &self.aad() })
            .map_err(|_| SealError::WrongKey)?;
        unpad(&padded).ok_or(SealError::WrongKey)
    }

    fn aad(&self) -> Vec<u8> {
        let mut aad = SEAL_DOMAIN.to_vec();
        aad.extend_from_slice(&self.id.0);
        aad
    }
}

pub fn append_message(stream: &StreamId, nonce: &[u8; APPEND_NONCE_BYTES], body: &[u8]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(APPEND_DOMAIN.len() + 32 + APPEND_NONCE_BYTES + body.len());
    msg.extend_from_slice(APPEND_DOMAIN);
    msg.extend_from_slice(&stream.0);
    msg.extend_from_slice(nonce);
    msg.extend_from_slice(body);
    msg
}

pub fn verify_append(write: &[u8; 32], stream: &StreamId, nonce: &[u8; APPEND_NONCE_BYTES], body: &[u8], sig: &[u8; 64]) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(write) else { return false };
    key.verify(&append_message(stream, nonce, body), &ed25519_dalek::Signature::from_bytes(sig)).is_ok()
}

pub fn bucket_for(len: usize) -> Option<usize> {
    SIZE_BUCKETS.iter().copied().find(|bucket| len + LENGTH_PREFIX_BYTES <= *bucket)
}

fn pad(plain: &[u8]) -> Result<Vec<u8>, SealError> {
    let bucket = bucket_for(plain.len()).ok_or(SealError::TooLarge)?;
    let mut padded = Vec::with_capacity(bucket);
    padded.extend_from_slice(&(plain.len() as u32).to_le_bytes());
    padded.extend_from_slice(plain);
    padded.resize(bucket, 0);
    Ok(padded)
}

fn unpad(padded: &[u8]) -> Option<Vec<u8>> {
    let len = u32::from_le_bytes(padded.get(..LENGTH_PREFIX_BYTES)?.try_into().ok()?) as usize;
    padded.get(LENGTH_PREFIX_BYTES..LENGTH_PREFIX_BYTES + len).map(<[u8]>::to_vec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_secret_always_gives_the_same_stream() {
        let secret = StreamSecret::from_bytes([7; 32]);
        assert_eq!(secret.keys().id, secret.keys().id);
        assert_eq!(secret.keys().write_public(), secret.keys().write_public());
        assert_ne!(secret.keys().id, StreamSecret::from_bytes([8; 32]).keys().id);
    }

    #[test]
    fn the_stream_id_commits_to_the_write_key_and_read_token() {
        let keys = StreamSecret::random().keys();
        assert_eq!(StreamId::of(&keys.write_public(), &keys.read_token()), keys.id);
        assert_ne!(StreamId::of(&keys.write_public(), &[0; 32]), keys.id);
        let other = StreamSecret::random().keys();
        assert_ne!(StreamId::of(&other.write_public(), &keys.read_token()), keys.id);
    }

    #[test]
    fn derived_secrets_differ_by_label_and_context() {
        let root = [1u8; 32];
        let a = StreamSecret::derive(&root, "account", b"");
        assert_eq!(a, StreamSecret::derive(&root, "account", b""));
        assert_ne!(a, StreamSecret::derive(&root, "dm", b""));
        assert_ne!(StreamSecret::derive(&root, "dm", b"2026-10"), StreamSecret::derive(&root, "dm", b"2026-11"));
        assert_ne!(StreamSecret::derive(&root, "ab", b"c"), StreamSecret::derive(&root, "a", b"bc"));
        assert_ne!(a, StreamSecret::derive(&[2u8; 32], "account", b""));
    }

    #[test]
    fn derivation_is_pinned() {
        let keys = StreamSecret::derive(&[0u8; 32], "account", b"").keys();
        assert_eq!(keys.id.to_hex(), "037d876122d2a6f6ce99516c5bb19a9711f089d8010c349b77adc0786fcaf653");
        assert_eq!(data_encoding::HEXLOWER.encode(&keys.write_public()), "12f6f8168371f919392ff137517c9b405b96d38b5e8d1d41e0961df0d6fba65b");
    }

    #[test]
    fn sealed_entries_open_only_with_their_stream() {
        let keys = StreamSecret::random().keys();
        let sealed = keys.seal(b"hello stream").unwrap();
        assert_eq!(keys.open(&sealed).unwrap(), b"hello stream");
        assert!(!sealed.windows(5).any(|w| w == b"hello"));
        assert_eq!(StreamSecret::random().keys().open(&sealed), Err(SealError::WrongKey));
        let mut tampered = sealed.clone();
        tampered[NONCE_BYTES + 3] ^= 1;
        assert_eq!(keys.open(&tampered), Err(SealError::WrongKey));
        assert_eq!(keys.open(&sealed[..10]), Err(SealError::WrongKey));
    }

    #[test]
    fn sealed_sizes_fall_into_buckets() {
        let keys = StreamSecret::random().keys();
        let overhead = NONCE_BYTES + TAG_BYTES;
        assert_eq!(keys.seal(b"").unwrap().len(), 512 + overhead);
        assert_eq!(keys.seal(&[1; 300]).unwrap().len(), 512 + overhead);
        assert_eq!(keys.seal(&[1; 508]).unwrap().len(), 512 + overhead);
        assert_eq!(keys.seal(&[1; 509]).unwrap().len(), 1024 + overhead);
        assert_eq!(keys.seal(&[1; 2000]).unwrap().len(), 4096 + overhead);
        assert_eq!(keys.seal(&[1; MAX_PLAIN]).unwrap().len(), MAX_SEALED);
        assert_eq!(keys.seal(&vec![1; MAX_PLAIN + 1]), Err(SealError::TooLarge));
        let big = vec![9u8; 20_000];
        assert_eq!(keys.open(&keys.seal(&big).unwrap()).unwrap(), big);
    }

    #[test]
    fn each_seal_differs() {
        let keys = StreamSecret::random().keys();
        assert_ne!(keys.seal(b"x").unwrap(), keys.seal(b"x").unwrap());
    }

    #[test]
    fn appends_verify_only_for_their_stream_nonce_and_body() {
        let keys = StreamSecret::random().keys();
        let nonce = [3u8; APPEND_NONCE_BYTES];
        let sig = keys.sign_append(&nonce, b"body");
        let write = keys.write_public();
        assert!(verify_append(&write, &keys.id, &nonce, b"body", &sig));
        assert!(!verify_append(&write, &keys.id, &nonce, b"bodY", &sig));
        assert!(!verify_append(&write, &keys.id, &[4u8; APPEND_NONCE_BYTES], b"body", &sig));
        assert!(!verify_append(&write, &StreamSecret::random().keys().id, &nonce, b"body", &sig));
        assert!(!verify_append(&StreamSecret::random().keys().write_public(), &keys.id, &nonce, b"body", &sig));
        assert!(!verify_append(&[0xff; 32], &keys.id, &nonce, b"body", &sig));
    }

    #[test]
    fn stream_ids_round_trip_as_hex() {
        let id = StreamSecret::random().keys().id;
        assert_eq!(StreamId::from_hex(&id.to_hex()), Some(id));
        assert_eq!(StreamId::from_hex(&id.to_hex().to_uppercase()), Some(id));
        assert_eq!(StreamId::from_hex("abcd"), None);
        assert_eq!(StreamId::from_hex(&"zz".repeat(32)), None);
        assert!(!format!("{:?}", StreamSecret::random()).contains(|c: char| c.is_ascii_digit()));
    }
}
