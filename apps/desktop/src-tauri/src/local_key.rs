use crate::account_folder;
use commy_core::at_rest;
use std::path::{Path, PathBuf};

pub const KEY_FILE: &str = "device-data.key";
pub const SET_ASIDE_DIR: &str = "unreadable";

const SHARED_FILES: &[&str] = &["account.json"];

#[derive(Debug)]
enum KeyProblem {
    Lost(String),
    Unavailable(String),
}

trait KeyStore {
    fn read(&self, dir: &Path) -> Result<Option<[u8; 32]>, KeyProblem>;
    fn store(&self, dir: &Path, key: &[u8; 32]) -> Result<(), String>;
    fn forget(&self, _dir: &Path) {}
}

pub fn unlock(dir: &Path) -> Result<(), String> {
    let Some(key) = prepare(dir, &os_key_store())? else {
        return Ok(());
    };
    at_rest::install(key);
    seal_older_files(dir, &key);
    Ok(())
}

pub fn carry(from: &Path, to: &Path) -> Result<(), String> {
    carry_with(from, to, &os_key_store())
}

fn carry_with(from: &Path, to: &Path, keys: &impl KeyStore) -> Result<(), String> {
    let key = match keys.read(from) {
        Ok(Some(key)) => key,
        Ok(None) => return Ok(()),
        Err(KeyProblem::Lost(why) | KeyProblem::Unavailable(why)) => {
            return Err(format!("The key for your local files could not be read ({why})."))
        }
    };
    if !matches!(keys.read(to), Ok(Some(found)) if found == key) {
        keys.store(to, &key).map_err(|why| format!("The key for your local files could not be saved for the new folder ({why})."))?;
    }
    match keys.read(to) {
        Ok(Some(found)) if found == key => Ok(()),
        _ => Err("The key for your local files did not carry over to the new folder.".into()),
    }
}

pub fn forget(dir: &Path) {
    os_key_store().forget(dir);
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
    let mut files = device_sealed_files(dir);
    for account in account_folder::folders(dir).into_iter().filter(|folder| account_folder::is_locked(folder)) {
        files.extend(account_files(&account));
    }
    files
}

fn device_sealed_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> =
        SHARED_FILES.iter().map(|name| dir.join(name)).filter(|path| path.is_file()).collect();
    files.extend(account_files(dir));
    for account in account_folder::folders(dir) {
        files.extend(account_folder::lock_files(&account));
        if !account_folder::is_locked(&account) {
            files.extend(account_files(&account));
        }
    }
    files
}

pub fn account_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> =
        account_folder::SEALED_FILES.iter().map(|name| dir.join(name)).filter(|path| path.is_file()).collect();
    for sub in account_folder::SEALED_DIRS {
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
    for path in device_sealed_files(dir) {
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

#[cfg(any(test, target_os = "macos"))]
const KEYCHAIN_ITEM_NOT_FOUND: i32 = -25300;

#[cfg(any(test, target_os = "macos"))]
fn from_keychain(found: Result<Vec<u8>, (i32, String)>) -> Result<Option<[u8; 32]>, KeyProblem> {
    match found {
        Ok(bytes) => <[u8; 32]>::try_from(bytes.as_slice())
            .map(Some)
            .map_err(|_| KeyProblem::Lost("the keychain item has the wrong length".into())),
        Err((KEYCHAIN_ITEM_NOT_FOUND, _)) => Ok(None),
        Err((_, why)) => Err(KeyProblem::Unavailable(format!("the keychain refused: {why}"))),
    }
}

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
        from_keychain(
            security_framework::passwords::get_generic_password(crate::toast::APP_ID, &keychain_account(dir))
                .map_err(|e| (e.code(), e.to_string())),
        )
    }

    fn store(&self, dir: &Path, key: &[u8; 32]) -> Result<(), String> {
        security_framework::passwords::set_generic_password(crate::toast::APP_ID, &keychain_account(dir), key)
            .map_err(|e| format!("the keychain refused: {e}"))
    }

    fn forget(&self, dir: &Path) {
        let _ = security_framework::passwords::delete_generic_password(crate::toast::APP_ID, &keychain_account(dir));
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
    fn every_account_folder_is_sealed_and_set_aside_with_the_key() {
        let dir = temp_dir("accounts");
        let account = account_folder::folder(&dir, "https://commy.chat", "user-a");
        std::fs::create_dir_all(account.join("servers")).unwrap();
        std::fs::write(account.join("dms.json"), b"{}").unwrap();
        std::fs::write(account.join("servers").join("a.json"), b"{}").unwrap();
        std::fs::write(account.join("node.port"), b"4242").unwrap();

        let key = prepare(&dir, &FileKey).unwrap().unwrap();
        assert_eq!(seal_older_files(&dir, &key), 2);
        assert_eq!(open(&account.join("dms.json"), &key), b"{}");
        assert_eq!(std::fs::read(account.join("node.port")).unwrap(), b"4242");

        std::fs::write(dir.join(KEY_FILE), "not a key").unwrap();
        prepare(&dir, &FileKey).unwrap().unwrap();
        assert!(!account.join("dms.json").exists());
        let aside = std::fs::read_dir(dir.join(SET_ASIDE_DIR)).unwrap().flatten().next().unwrap().path();
        let relative = account.strip_prefix(&dir).unwrap();
        assert!(aside.join(relative).join("dms.json").exists());
        assert!(aside.join(relative).join("servers").join("a.json").exists());
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

    struct Fake {
        answer: fn() -> Result<Option<[u8; 32]>, KeyProblem>,
        accepts: bool,
        stored: std::cell::Cell<Option<[u8; 32]>>,
    }

    impl Fake {
        fn new(answer: fn() -> Result<Option<[u8; 32]>, KeyProblem>, accepts: bool) -> Self {
            Fake { answer, accepts, stored: std::cell::Cell::new(None) }
        }
    }

    impl KeyStore for Fake {
        fn read(&self, _: &Path) -> Result<Option<[u8; 32]>, KeyProblem> {
            match self.stored.get() {
                Some(key) => Ok(Some(key)),
                None => (self.answer)(),
            }
        }
        fn store(&self, _: &Path, key: &[u8; 32]) -> Result<(), String> {
            if !self.accepts {
                return Err("the keychain refused".into());
            }
            self.stored.set(Some(*key));
            Ok(())
        }
    }

    fn set_aside_runs(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir.join(SET_ASIDE_DIR))
            .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn a_key_the_os_cannot_open_sets_every_sealed_file_aside_once_and_starts_fresh() {
        let dir = temp_dir("os-lost");
        let old = [11u8; 32];
        std::fs::write(dir.join("dms.json"), at_rest::seal_with(&old, b"{}")).unwrap();
        std::fs::write(dir.join("node.key"), at_rest::seal_with(&old, b"aa")).unwrap();
        std::fs::create_dir_all(dir.join("servers")).unwrap();
        std::fs::write(dir.join("servers").join("s.json"), at_rest::seal_with(&old, b"{}")).unwrap();
        std::fs::write(dir.join("profile.json"), b"{}").unwrap();
        std::fs::write(dir.join("node.port"), b"4242").unwrap();

        let keys = Fake::new(|| Err(KeyProblem::Lost("Windows could not unprotect it".into())), true);
        let key = prepare(&dir, &keys).unwrap().unwrap();
        assert_ne!(key, old);
        assert_eq!(keys.stored.get(), Some(key), "the new key is handed to the OS");

        let runs = set_aside_runs(&dir);
        assert_eq!(runs.len(), 1);
        for (name, plain) in [("dms.json", &b"{}"[..]), ("node.key", b"aa"), ("servers/s.json", b"{}")] {
            assert!(!dir.join(name).exists(), "{name} left in place");
            assert_eq!(open(&runs[0].join(name), &old), plain, "{name} kept aside intact for the old key");
        }
        assert_eq!(seal_older_files(&dir, &key), 1, "plain files are kept and sealed with the new key");
        assert_eq!(open(&dir.join("profile.json"), &key), b"{}");
        assert_eq!(std::fs::read(dir.join("node.port")).unwrap(), b"4242");

        assert_eq!(prepare(&dir, &keys).unwrap(), Some(key));
        assert_eq!(set_aside_runs(&dir).len(), 1, "the next start moves nothing again");
    }

    #[test]
    fn files_no_key_can_open_read_as_missing_and_are_replaced() {
        let dir = temp_dir("unopenable");
        let other = [12u8; 32];
        std::fs::write(dir.join("node.key"), at_rest::seal_with(&other, b"00")).unwrap();
        std::fs::write(dir.join("identity.key"), at_rest::seal_with(&other, b"00")).unwrap();
        std::fs::write(dir.join("dms.json"), at_rest::seal_with(&other, b"{}")).unwrap();

        assert!(crate::persist::read_json::<serde_json::Value>(&dir.join("dms.json")).is_none());
        let node = crate::persist::load_or_create_node_key(&dir).unwrap();
        assert_eq!(crate::persist::load_or_create_node_key(&dir), Some(node));
        let identity = commy_core::identity::load_or_create(&dir).unwrap();
        assert_eq!(commy_core::identity::load_or_create(&dir).unwrap().public(), identity.public());
    }

    #[test]
    fn a_keychain_that_will_not_store_a_new_key_keeps_plain_files_working() {
        let dir = temp_dir("no-store");
        std::fs::write(dir.join("dms.json"), b"{}").unwrap();
        let keys = Fake::new(|| Ok(None), false);
        assert_eq!(prepare(&dir, &keys).unwrap(), None);
        assert_eq!(std::fs::read(dir.join("dms.json")).unwrap(), b"{}");
        assert!(set_aside_runs(&dir).is_empty());
    }

    #[test]
    fn a_missing_key_and_a_keychain_that_will_not_store_sets_sealed_files_aside_and_runs_unsealed() {
        let dir = temp_dir("missing-no-store");
        std::fs::write(dir.join("account.json"), at_rest::seal_with(&[13; 32], b"{}")).unwrap();
        std::fs::write(dir.join("dms.json"), b"{}").unwrap();
        let keys = Fake::new(|| Ok(None), false);
        assert_eq!(prepare(&dir, &keys).unwrap(), None);
        assert!(!dir.join("account.json").exists());
        assert!(set_aside_runs(&dir)[0].join("account.json").exists());
        assert_eq!(std::fs::read(dir.join("dms.json")).unwrap(), b"{}");
    }

    #[test]
    fn a_keychain_that_refuses_reads_stops_the_app_for_sealed_files_in_sub_folders_too() {
        let dir = temp_dir("refusing-sub");
        std::fs::create_dir_all(dir.join(crate::SERVER_IMAGE_DIR)).unwrap();
        let image = dir.join(crate::SERVER_IMAGE_DIR).join("a.img");
        std::fs::write(&image, at_rest::seal_with(&[14; 32], b"png")).unwrap();
        let keys = Fake::new(|| Err(KeyProblem::Unavailable("the keychain refused: locked".into())), true);
        assert_eq!(prepare(&dir, &keys).unwrap_err(), "the keychain refused: locked");
        assert!(image.exists());
        assert!(set_aside_runs(&dir).is_empty());
        assert_eq!(keys.stored.get(), None, "no new key replaces one that may come back");
    }

    #[test]
    fn keychain_answers_map_to_found_missing_lost_or_refused() {
        assert_eq!(from_keychain(Ok(vec![6; 32])).unwrap(), Some([6; 32]));
        assert_eq!(from_keychain(Err((KEYCHAIN_ITEM_NOT_FOUND, "not found".into()))).unwrap(), None);
        assert!(matches!(from_keychain(Ok(vec![6; 16])), Err(KeyProblem::Lost(_))));
        assert!(matches!(
            from_keychain(Err((-25293, "user canceled".into()))),
            Err(KeyProblem::Unavailable(why)) if why.contains("user canceled")
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_reports_a_key_file_it_cannot_unprotect_as_lost() {
        let dir = temp_dir("dpapi-lost");
        assert!(matches!(WindowsKey.read(&dir), Ok(None)));
        std::fs::write(dir.join(KEY_FILE), b"not protected").unwrap();
        assert!(matches!(WindowsKey.read(&dir), Err(KeyProblem::Lost(_))));
        std::fs::write(dir.join(KEY_FILE), dpapi::protect(&[5; 16]).unwrap()).unwrap();
        assert!(matches!(WindowsKey.read(&dir), Err(KeyProblem::Lost(_))));
        std::fs::remove_file(dir.join(KEY_FILE)).unwrap();
        WindowsKey.store(&dir, &[5; 32]).unwrap();
        assert!(matches!(WindowsKey.read(&dir), Ok(Some(key)) if key == [5; 32]));
    }

    #[cfg(windows)]
    #[test]
    fn windows_protects_the_key_for_this_user() {
        let protected = dpapi::protect(&[5; 32]).unwrap();
        assert!(!protected.windows(32).any(|w| w == [5; 32]));
        assert_eq!(dpapi::unprotect(&protected).unwrap(), vec![5; 32]);
        assert!(dpapi::unprotect(b"not protected").is_none());
    }

    struct ByPath(std::cell::RefCell<std::collections::HashMap<PathBuf, [u8; 32]>>);

    impl KeyStore for ByPath {
        fn read(&self, dir: &Path) -> Result<Option<[u8; 32]>, KeyProblem> {
            Ok(self.0.borrow().get(dir).copied())
        }

        fn store(&self, dir: &Path, key: &[u8; 32]) -> Result<(), String> {
            self.0.borrow_mut().insert(dir.to_path_buf(), *key);
            Ok(())
        }
    }

    #[test]
    fn a_key_kept_by_folder_path_follows_a_moved_folder() {
        let keys = ByPath(Default::default());
        keys.store(Path::new("/old"), &[9; 32]).unwrap();
        carry_with(Path::new("/old"), Path::new("/new"), &keys).unwrap();
        assert!(matches!(keys.read(Path::new("/new")), Ok(Some(key)) if key == [9; 32]));
        carry_with(Path::new("/nothing"), Path::new("/elsewhere"), &keys).unwrap();
        assert!(matches!(keys.read(Path::new("/elsewhere")), Ok(None)));
    }

    #[test]
    fn a_key_file_copied_with_the_folder_needs_nothing_more() {
        let from = temp_dir("carry-from");
        let to = temp_dir("carry-to");
        FileKey.store(&from, &[4; 32]).unwrap();
        std::fs::copy(from.join(KEY_FILE), to.join(KEY_FILE)).unwrap();
        carry_with(&from, &to, &FileKey).unwrap();
        assert!(matches!(FileKey.read(&to), Ok(Some(key)) if key == [4; 32]));
    }
}
