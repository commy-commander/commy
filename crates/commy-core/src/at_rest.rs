use std::io::{self, Write};
use std::path::Path;
use std::sync::OnceLock;

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

pub const MAGIC: &[u8] = b"commy-sealed-v1\n";
const NONCE_BYTES: usize = 12;

static DEVICE_KEY: OnceLock<[u8; 32]> = OnceLock::new();

pub fn install(key: [u8; 32]) -> bool {
    DEVICE_KEY.set(key).is_ok()
}

pub fn installed() -> bool {
    DEVICE_KEY.get().is_some()
}

pub fn is_sealed(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

pub fn seal_with(key: &[u8; 32], plain: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new_from_slice(key).expect("32 byte key");
    let nonce: [u8; NONCE_BYTES] = rand::random();
    let body = cipher
        .encrypt(&Nonce::from(nonce), Payload { msg: plain, aad: MAGIC })
        .expect("in-memory encryption cannot fail");
    [MAGIC, &nonce, &body].concat()
}

pub fn open_with(key: Option<&[u8; 32]>, bytes: Vec<u8>) -> io::Result<Vec<u8>> {
    if !is_sealed(&bytes) {
        return Ok(bytes);
    }
    let locked = || io::Error::new(io::ErrorKind::InvalidData, "sealed with a key this device does not have");
    let key = key.ok_or_else(locked)?;
    let rest = &bytes[MAGIC.len()..];
    if rest.len() < NONCE_BYTES {
        return Err(locked());
    }
    let (nonce, body) = rest.split_at(NONCE_BYTES);
    let nonce: [u8; NONCE_BYTES] = nonce.try_into().expect("split at the nonce length");
    let cipher = Aes256Gcm::new_from_slice(key).expect("32 byte key");
    cipher.decrypt(&Nonce::from(nonce), Payload { msg: body, aad: MAGIC }).map_err(|_| locked())
}

pub fn seal(plain: &[u8]) -> Vec<u8> {
    match DEVICE_KEY.get() {
        Some(key) => seal_with(key, plain),
        None => plain.to_vec(),
    }
}

pub fn open(bytes: Vec<u8>) -> io::Result<Vec<u8>> {
    open_with(DEVICE_KEY.get(), bytes)
}

pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    open(std::fs::read(path)?)
}

pub fn read_to_string(path: &Path) -> io::Result<String> {
    String::from_utf8(read(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn write(path: &Path, plain: &[u8]) -> io::Result<()> {
    std::fs::write(path, seal(plain))
}

pub fn write_private(path: &Path, plain: &[u8]) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(&seal(plain))?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7; 32];

    #[test]
    fn sealed_bytes_open_with_the_same_key_only() {
        let sealed = seal_with(&KEY, b"hello");
        assert!(is_sealed(&sealed));
        assert!(!sealed.windows(5).any(|w| w == b"hello"));
        assert_eq!(open_with(Some(&KEY), sealed.clone()).unwrap(), b"hello");
        assert_eq!(open_with(Some(&[8; 32]), sealed.clone()).unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(open_with(None, sealed).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn sealing_twice_gives_different_bytes() {
        assert_ne!(seal_with(&KEY, b"same"), seal_with(&KEY, b"same"));
    }

    #[test]
    fn plain_files_from_older_versions_still_read() {
        assert_eq!(open_with(Some(&KEY), b"{\"a\":1}".to_vec()).unwrap(), b"{\"a\":1}");
        assert_eq!(open_with(None, b"plain".to_vec()).unwrap(), b"plain");
    }

    #[test]
    fn a_cut_off_file_is_refused() {
        let sealed = seal_with(&KEY, b"hello");
        assert!(open_with(Some(&KEY), sealed[..MAGIC.len() + 4].to_vec()).is_err());
        assert!(open_with(Some(&KEY), sealed[..sealed.len() - 1].to_vec()).is_err());
    }
}
