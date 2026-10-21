use commy_core::at_rest;
use std::path::{Path, PathBuf};

const KEY_FILE: &str = "device-data.key";
const SET_ASIDE_DIR: &str = "unreadable";

const STORED_FILES: &[&str] = &[
    "account.json",
    "avatar.img",
    "awaiting-snapshot.json",
    "deleted-servers.json",
    "directory.json",
    "dms.json",
    "doorbell.json",
    "entry-key.json",
    "entry-list.json",
    "identity.key",
    "integrations.json",
    "invites.json",
    "node.key",
    "outbox.json",
    "pins.json",
    "profile-banner.img",
    "profile.json",
    "read-state.json",
    "safety.json",
    "saved.json",
    "storage.json",
    "stream-deck.json",
    "voice.json",
];

const STORED_DIRS: &[&str] = &["servers", crate::profile_images::PROFILE_IMAGE_DIR, crate::SERVER_IMAGE_DIR];

#[derive(Debug)]
enum KeyProblem {
    Lost(String),
    Unavailable(String),
}

trait KeyStore {
    fn read(&self, dir: &Path) -> Result<Option<[u8; 32]>, KeyProblem>;
    fn store(&self, dir: &Path, key: &[u8; 32]) -> Result<(), String>;
}

pub fn unlock(dir: &Path) -> Result<(), String> {
    let Some(key) = prepare(dir, &os_key_store())? else {
        return Ok(());
    };
    at_rest::install(key);
    seal_older_files(dir, &key);
    Ok(())
}

fn prepare(dir: &Path, keys: &impl KeyStore) -> Result<Option<[u8; 32]>, String> {
    let found = match keys.read(dir) {
        Ok(found) => found,
        Err(KeyProblem::Lost(why)) => {
            tracing::warn!("the key for local files cannot be read ({why}); moving the files it sealed aside");
            set_aside_sealed_files(dir);
            None
        }
        Err(KeyProblem::Unavailable(why)) => return keep_running_unsealed_if_nothing_is_sealed(dir, why),
    };
    if let Some(key) = found {
        return Ok(Some(key));
    }
    set_aside_sealed_files(dir);
    let key = commy_net::CommyNode::generate_secret();
    match keys.store(dir, &key) {
        Ok(()) => Ok(Some(key)),
        Err(why) => keep_running_unsealed_if_nothing_is_sealed(dir, why),
    }
}

fn keep_running_unsealed_if_nothing_is_sealed(dir: &Path, why: String) -> Result<Option<[u8; 32]>, String> {
    if stored_files(dir).iter().any(|path| is_sealed_file(path)) {
        return Err(why);
    }
    tracing::warn!("local files stay unencrypted: {why}");
    Ok(None)
}

fn stored_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> =
        STORED_FILES.iter().map(|name| dir.join(name)).filter(|path| path.is_file()).collect();
    for sub in STORED_DIRS {
        let Ok(entries) = std::fs::read_dir(dir.join(sub)) else {
            continue;
        };
        files.extend(
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext != "tmp")),
        );
    }
    files
}

fn is_sealed_file(path: &Path) -> bool {
    std::fs::read(path).is_ok_and(|bytes| at_rest::is_sealed(&bytes))
}

fn seal_older_files(dir: &Path, key: &[u8; 32]) -> usize {
    let mut sealed = 0;
    for path in stored_files(dir) {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if at_rest::is_sealed(&bytes) {
            continue;
        }
        match crate::persist::write_private_bytes(&path, &at_rest::seal_with(key, &bytes)) {
            Ok(()) => sealed += 1,
            Err(e) => tracing::warn!("could not encrypt {}: {e}", path.display()),
        }
    }
    if sealed > 0 {
        tracing::info!("encrypted {sealed} local files");
    }
    sealed
}

fn set_aside_sealed_files(dir: &Path) {
    let target = dir.join(SET_ASIDE_DIR).join(chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string());
    let mut moving: Vec<PathBuf> = stored_files(dir).into_iter().filter(|path| is_sealed_file(path)).collect();
    moving.extend(Some(dir.join(KEY_FILE)).filter(|path| path.is_file()));
    for path in moving {
        let Ok(relative) = path.strip_prefix(dir) else {
            continue;
        };
        let destination = target.join(relative);
        let moved = destination
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::rename(&path, &destination));
        if let Err(e) = moved {
            tracing::warn!("could not move {} aside: {e}", path.display());
        }
    }
}

#[cfg(windows)]
struct WindowsKey;

#[cfg(windows)]
fn os_key_store() -> WindowsKey {
    WindowsKey
}

#[cfg(windows)]
impl KeyStore for WindowsKey {
    fn read(&self, dir: &Path) -> Result<Option<[u8; 32]>, KeyProblem> {
        let protected = match std::fs::read(dir.join(KEY_FILE)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(KeyProblem::Unavailable(format!("could not read {KEY_FILE}: {e}"))),
        };
        let plain =
            dpapi::unprotect(&protected).ok_or_else(|| KeyProblem::Lost("Windows could not unprotect it".into()))?;
        <[u8; 32]>::try_from(plain.as_slice()).map(Some).map_err(|_| KeyProblem::Lost("it has the wrong length".into()))
    }

    fn store(&self, dir: &Path, key: &[u8; 32]) -> Result<(), String> {
        let protected = dpapi::protect(key).ok_or("Windows could not protect the key")?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        crate::persist::write_private_bytes(&dir.join(KEY_FILE), &protected).map_err(|e| e.to_string())
    }
}

#[cfg(windows)]
mod dpapi {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    fn input(bytes: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_ptr() as *mut u8 }
    }

    fn empty() -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() }
    }

    fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        // SAFETY: after a successful call the API owns a LocalAlloc buffer of cbData bytes at pbData; it is copied once and freed once.
        unsafe {
            let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            LocalFree(out.pbData as _);
            bytes
        }
    }

    pub fn protect(plain: &[u8]) -> Option<Vec<u8>> {
        let data = input(plain);
        let mut out = empty();
        // SAFETY: data borrows plain for the whole call, every optional pointer is null, and out is only read after success.
        let ok = unsafe {
            CryptProtectData(
                &data,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        (ok != 0).then(|| take(out))
    }

    pub fn unprotect(protected: &[u8]) -> Option<Vec<u8>> {
        let data = input(protected);
        let mut out = empty();
        // SAFETY: data borrows protected for the whole call, every optional pointer is null, and out is only read after success.
        let ok = unsafe {
            CryptUnprotectData(
                &data,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        (ok != 0).then(|| take(out))
    }
}

#[cfg(target_os = "macos")]
const KEYCHAIN_ITEM_NOT_FOUND: i32 = -25300;

#[cfg(target_os = "macos")]
fn keychain_account(dir: &Path) -> String {
    format!("local data {}", dir.display())
}

#[cfg(target_os = "macos")]
struct KeychainKey;

#[cfg(target_os = "macos")]
fn os_key_store() -> KeychainKey {
    KeychainKey
}

#[cfg(target_os = "macos")]
impl KeyStore for KeychainKey {
    fn read(&self, dir: &Path) -> Result<Option<[u8; 32]>, KeyProblem> {
        match security_framework::passwords::get_generic_password(crate::toast::APP_ID, &keychain_account(dir)) {
            Ok(bytes) => <[u8; 32]>::try_from(bytes.as_slice())
                .map(Some)
                .map_err(|_| KeyProblem::Lost("the keychain item has the wrong length".into())),
            Err(e) if e.code() == KEYCHAIN_ITEM_NOT_FOUND => Ok(None),
            Err(e) => Err(KeyProblem::Unavailable(format!("the keychain refused: {e}"))),
        }
    }

    fn store(&self, dir: &Path, key: &[u8; 32]) -> Result<(), String> {
        security_framework::passwords::set_generic_password(crate::toast::APP_ID, &keychain_account(dir), key)
            .map_err(|e| format!("the keychain refused: {e}"))
    }
}

#[cfg(any(test, not(any(windows, target_os = "macos"))))]
struct FileKey;

#[cfg(not(any(windows, target_os = "macos")))]
fn os_key_store() -> FileKey {
    FileKey
}

#[cfg(any(test, not(any(windows, target_os = "macos"))))]
impl KeyStore for FileKey {
    fn read(&self, dir: &Path) -> Result<Option<[u8; 32]>, KeyProblem> {
        match std::fs::read_to_string(dir.join(KEY_FILE)) {
            Ok(hex) => data_encoding::HEXLOWER
                .decode(hex.trim().as_bytes())
                .ok()
                .and_then(|bytes| <[u8; 32]>::try_from(bytes.as_slice()).ok())
                .map(Some)
                .ok_or_else(|| KeyProblem::Lost(format!("{KEY_FILE} is not a key"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(KeyProblem::Unavailable(format!("could not read {KEY_FILE}: {e}"))),
        }
    }

    fn store(&self, dir: &Path, key: &[u8; 32]) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        crate::persist::write_private_bytes(&dir.join(KEY_FILE), data_encoding::HEXLOWER.encode(key).as_bytes())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("commy-local-key-tests-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn open(path: &Path, key: &[u8; 32]) -> Vec<u8> {
        at_rest::open_with(Some(key), std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn older_plain_files_are_sealed_and_still_read() {
        let dir = temp_dir("older");
        std::fs::write(dir.join("dms.json"), br#"{"hello":"there"}"#).unwrap();
        std::fs::create_dir_all(dir.join("servers")).unwrap();
        std::fs::write(dir.join("servers").join("a.json"), b"{}").unwrap();
        std::fs::write(dir.join("node.port"), b"4242").unwrap();

        let key = prepare(&dir, &FileKey).unwrap().unwrap();
        assert_eq!(seal_older_files(&dir, &key), 2);
        assert!(is_sealed_file(&dir.join("dms.json")));
        assert_eq!(open(&dir.join("dms.json"), &key), br#"{"hello":"there"}"#);
        assert_eq!(open(&dir.join("servers").join("a.json"), &key), b"{}");
        assert_eq!(std::fs::read(dir.join("node.port")).unwrap(), b"4242", "files read by other processes stay plain");
        assert_eq!(seal_older_files(&dir, &key), 0);
        assert_eq!(prepare(&dir, &FileKey).unwrap(), Some(key), "the same key comes back on the next start");
    }

    #[test]
    fn files_sealed_with_a_lost_key_are_moved_aside_not_deleted() {
        let dir = temp_dir("lost");
        let old = [9u8; 32];
        std::fs::write(dir.join("dms.json"), at_rest::seal_with(&old, b"{}")).unwrap();
        std::fs::write(dir.join("profile.json"), b"{}").unwrap();

        let key = prepare(&dir, &FileKey).unwrap().unwrap();
        assert_ne!(key, old);
        assert!(!dir.join("dms.json").exists());
        assert!(dir.join("profile.json").exists(), "plain files are kept and sealed with the new key");
        let aside: Vec<_> = std::fs::read_dir(dir.join(SET_ASIDE_DIR)).unwrap().flatten().collect();
        assert_eq!(aside.len(), 1);
        assert!(aside[0].path().join("dms.json").exists());
    }

    #[test]
    fn a_damaged_key_file_sets_the_sealed_files_aside_with_it() {
        let dir = temp_dir("damaged");
        std::fs::write(dir.join(KEY_FILE), "not a key").unwrap();
        std::fs::write(dir.join("account.json"), at_rest::seal_with(&[3; 32], b"{}")).unwrap();
        let key = prepare(&dir, &FileKey).unwrap().unwrap();
        assert!(!dir.join("account.json").exists());
        assert_eq!(FileKey.read(&dir).unwrap(), Some(key));
        let aside = std::fs::read_dir(dir.join(SET_ASIDE_DIR)).unwrap().flatten().next().unwrap().path();
        assert!(aside.join("account.json").exists());
        assert!(aside.join(KEY_FILE).exists());
    }

    #[test]
    fn a_keychain_that_refuses_only_stops_the_app_when_files_are_sealed() {
        struct Refusing;
        impl KeyStore for Refusing {
            fn read(&self, _: &Path) -> Result<Option<[u8; 32]>, KeyProblem> {
                Err(KeyProblem::Unavailable("locked".into()))
            }
            fn store(&self, _: &Path, _: &[u8; 32]) -> Result<(), String> {
                Err("locked".into())
            }
        }
        let dir = temp_dir("refusing");
        std::fs::write(dir.join("dms.json"), b"{}").unwrap();
        assert_eq!(prepare(&dir, &Refusing).unwrap(), None, "plain files keep working without a key");
        std::fs::write(dir.join("account.json"), at_rest::seal_with(&[4; 32], b"{}")).unwrap();
        assert_eq!(prepare(&dir, &Refusing).unwrap_err(), "locked");
        assert!(dir.join("account.json").exists(), "nothing is moved while the keychain may come back");
    }

    #[cfg(windows)]
    #[test]
    fn windows_protects_the_key_for_this_user() {
        let protected = dpapi::protect(&[5; 32]).unwrap();
        assert!(!protected.windows(32).any(|w| w == [5; 32]));
        assert_eq!(dpapi::unprotect(&protected).unwrap(), vec![5; 32]);
        assert!(dpapi::unprotect(b"not protected").is_none());
    }
}
