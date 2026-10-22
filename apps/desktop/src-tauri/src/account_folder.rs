use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const ACCOUNTS_DIR: &str = "accounts";
const UNCLAIMED_DIR: &str = "unclaimed";
const SHOW_WINDOW_FILE: &str = "show-window-on-start";

pub const SEALED_FILES: &[&str] = &[
    "avatar.img",
    "awaiting-snapshot.json",
    "community-sync.json",
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
    "sync-state.json",
    "voice.json",
];

pub const SEALED_DIRS: &[&str] = &["servers", crate::profile_images::PROFILE_IMAGE_DIR, crate::SERVER_IMAGE_DIR];

const PLAIN_FILES: &[&str] = &["node.port", crate::state::UNSETTLED_FILE];

fn account_entries() -> Vec<String> {
    let icons = crate::app_icon::TARGETS.iter().map(|target| format!("custom-{target}-icon.png"));
    SEALED_FILES
        .iter()
        .chain(SEALED_DIRS)
        .chain(PLAIN_FILES)
        .map(|name| name.to_string())
        .chain(icons)
        .collect()
}

pub fn folder_name(server: &str, account: &str) -> String {
    let digest = Sha256::digest(format!("commy-account-folder:v1\n{server}\n{account}").as_bytes());
    data_encoding::HEXLOWER.encode(&digest[..16])
}

pub fn folder(base: &Path, server: &str, account: &str) -> PathBuf {
    base.join(ACCOUNTS_DIR).join(folder_name(server, account))
}

pub fn folders(base: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(base.join(ACCOUNTS_DIR)) else { return Vec::new() };
    entries.flatten().map(|entry| entry.path()).filter(|path| path.is_dir()).collect()
}

fn unclaimed_dir(base: &Path) -> PathBuf {
    base.join(ACCOUNTS_DIR).join(UNCLAIMED_DIR)
}

pub fn set_aside_unowned_data(base: &Path) -> usize {
    let target = unclaimed_dir(base);
    let mut moved = 0;
    for name in account_entries() {
        let from = base.join(&name);
        if !from.exists() {
            continue;
        }
        let to = target.join(&name);
        if to.exists() {
            tracing::warn!("{name} is in both the shared folder and the unclaimed profile; keeping both");
            continue;
        }
        let result = std::fs::create_dir_all(&target).and_then(|_| std::fs::rename(&from, &to));
        match result {
            Ok(()) => moved += 1,
            Err(e) => tracing::warn!("could not move {name} into the unclaimed profile: {e}"),
        }
    }
    if moved > 0 {
        tracing::info!("set aside {moved} entries from before accounts had their own folders");
    }
    moved
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UnclaimedProfile {
    pub username: String,
    pub display_name: Option<String>,
    pub servers: usize,
}

pub fn unclaimed(base: &Path) -> Option<UnclaimedProfile> {
    let dir = unclaimed_dir(base);
    if !dir.is_dir() {
        return None;
    }
    let profile = crate::persist::load_profile(&dir);
    let servers = std::fs::read_dir(dir.join("servers"))
        .map(|entries| {
            entries.flatten().filter(|e| e.path().extension().is_some_and(|ext| ext == "json")).count()
        })
        .unwrap_or(0);
    if profile.is_none() && servers == 0 {
        return None;
    }
    Some(UnclaimedProfile {
        username: profile.as_ref().map(|p| p.username.clone()).unwrap_or_default(),
        display_name: profile.and_then(|p| p.display_name),
        servers,
    })
}

pub fn unclaimed_node_secret(base: &Path) -> Option<[u8; 32]> {
    let hex = commy_core::at_rest::read_to_string(&unclaimed_dir(base).join("node.key")).ok()?;
    let bytes = data_encoding::HEXLOWER.decode(hex.trim().as_bytes()).ok()?;
    bytes.try_into().ok()
}

#[derive(Debug, PartialEq)]
pub enum Opening {
    Ready(PathBuf),
    AskAboutUnclaimed,
}

pub fn choose(base: &Path, server: &str, account: &str) -> Opening {
    let dir = folder(base, server, account);
    if dir.is_dir() || unclaimed(base).is_none() {
        Opening::Ready(dir)
    } else {
        Opening::AskAboutUnclaimed
    }
}

pub fn claim(base: &Path, server: &str, account: &str, keep: bool) -> std::io::Result<PathBuf> {
    let dir = folder(base, server, account);
    if dir.is_dir() {
        return Ok(dir);
    }
    std::fs::create_dir_all(base.join(ACCOUNTS_DIR))?;
    let from = unclaimed_dir(base);
    if keep && from.is_dir() {
        std::fs::rename(&from, &dir)?;
    } else {
        std::fs::create_dir_all(&dir)?;
    }
    Ok(dir)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FolderLock {
    pub lock_id: String,
    pub wrapped: String,
}

fn lock_path(dir: &Path) -> PathBuf {
    dir.with_extension("lock")
}

fn unlocked_path(dir: &Path) -> PathBuf {
    dir.with_extension("unlocked")
}

fn resealing_path(dir: &Path) -> PathBuf {
    dir.with_extension("resealing")
}

pub fn lock_files(dir: &Path) -> Vec<PathBuf> {
    [lock_path(dir), unlocked_path(dir)].into_iter().filter(|path| path.is_file()).collect()
}

pub fn is_locked(dir: &Path) -> bool {
    lock_path(dir).is_file()
}

pub fn read_lock(dir: &Path) -> Option<FolderLock> {
    crate::persist::read_json(&lock_path(dir))
}

pub fn new_lock_id() -> String {
    data_encoding::HEXLOWER.encode(&commy_net::CommyNode::generate_secret()[..16])
}

fn key_encryption_key(server_key: &str) -> Option<[u8; 32]> {
    let bytes = data_encoding::HEXLOWER_PERMISSIVE.decode(server_key.trim().as_bytes()).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(b"commy-folder-kek:v1\n");
    hasher.update(&bytes);
    Some(hasher.finalize().into())
}

pub fn wrap(server_key: &str, lock_id: &str, folder_key: &[u8; 32]) -> Option<FolderLock> {
    let kek = key_encryption_key(server_key)?;
    let wrapped = data_encoding::HEXLOWER.encode(&commy_core::at_rest::seal_with(&kek, folder_key));
    Some(FolderLock { lock_id: lock_id.to_string(), wrapped })
}

pub fn unwrap(server_key: &str, lock: &FolderLock) -> Option<[u8; 32]> {
    let kek = key_encryption_key(server_key)?;
    let sealed = data_encoding::HEXLOWER_PERMISSIVE.decode(lock.wrapped.as_bytes()).ok()?;
    commy_core::at_rest::open_with(Some(&kek), sealed).ok()?.try_into().ok()
}

pub fn lock(dir: &Path, lock: &FolderLock, folder_key: &[u8; 32]) -> std::io::Result<usize> {
    std::fs::write(resealing_path(dir), b"")?;
    crate::persist::write_secret_json(&lock_path(dir), lock).map_err(std::io::Error::other)?;
    reseal(dir, folder_key)
}

pub fn finish_resealing(dir: &Path, folder_key: &[u8; 32]) -> std::io::Result<usize> {
    if !resealing_path(dir).exists() {
        return Ok(0);
    }
    reseal(dir, folder_key)
}

fn reseal(dir: &Path, folder_key: &[u8; 32]) -> std::io::Result<usize> {
    let mut moved = 0;
    for path in crate::local_key::account_files(dir) {
        match commy_core::at_rest::reseal_for_folder(&path, folder_key) {
            Ok(true) => moved += 1,
            Ok(false) => {}
            Err(e) => tracing::warn!("could not move {} to the account key: {e}", path.display()),
        }
    }
    std::fs::remove_file(resealing_path(dir))?;
    Ok(moved)
}

pub fn cached_key(dir: &Path) -> Option<[u8; 32]> {
    let hex = commy_core::at_rest::read_to_string(&unlocked_path(dir)).ok()?;
    data_encoding::HEXLOWER.decode(hex.trim().as_bytes()).ok()?.try_into().ok()
}

pub fn remember_key(dir: &Path, folder_key: &[u8; 32]) {
    let written = crate::persist::write_secret(&unlocked_path(dir), data_encoding::HEXLOWER.encode(folder_key).as_bytes());
    if let Err(e) = written {
        tracing::warn!("could not keep this account unlocked between starts: {e}");
    }
}

pub fn forget_key(dir: &Path) {
    if let Err(e) = std::fs::remove_file(unlocked_path(dir)) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("could not forget this account's key: {e}");
        }
    }
}

pub fn show_window_on_next_start(base: &Path) {
    if let Err(e) = std::fs::write(base.join(SHOW_WINDOW_FILE), b"") {
        tracing::warn!("could not ask the next start to show the window: {e}");
    }
}

pub fn take_show_window(base: &Path) -> bool {
    std::fs::remove_file(base.join(SHOW_WINDOW_FILE)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::UserProfile;

    const SERVER: &str = "https://commy.chat";

    fn base(tag: &str) -> PathBuf {
        let nanos = std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("commy-account-folder-{tag}-{}-{nanos:x}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_profile(dir: &Path, name: &str) -> UserProfile {
        let profile = UserProfile::new(name);
        crate::persist::save_profile(dir, &profile).unwrap();
        profile
    }

    #[test]
    fn each_account_gets_its_own_folder() {
        let base = base("names");
        let first = folder(&base, SERVER, "user-a");
        assert_eq!(first, folder(&base, SERVER, "user-a"));
        assert_ne!(first, folder(&base, SERVER, "user-b"));
        assert_ne!(first, folder(&base, "https://staging.commy.chat", "user-a"));
        let name = folder_name(SERVER, "user-a");
        assert_eq!(name.len(), 32);
        assert!(!name.contains("user-a"));
    }

    #[test]
    fn a_second_account_does_not_open_the_first_accounts_data() {
        let base = base("second");
        let Opening::Ready(a) = choose(&base, SERVER, "user-a") else { panic!("nothing to ask about") };
        std::fs::create_dir_all(&a).unwrap();
        let profile = write_profile(&a, "first");
        std::fs::write(a.join("node.key"), b"secret").unwrap();

        let Opening::Ready(b) = choose(&base, SERVER, "user-b") else { panic!("nothing to ask about") };
        assert_ne!(a, b);
        std::fs::create_dir_all(&b).unwrap();
        assert!(crate::persist::load_profile(&b).is_none());
        assert!(!b.join("node.key").exists());
        assert!(crate::persist::load_servers(&b).is_empty());

        let Opening::Ready(again) = choose(&base, SERVER, "user-a") else { panic!("nothing to ask about") };
        assert_eq!(crate::persist::load_profile(&again).unwrap().id, profile.id);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn data_from_before_accounts_had_folders_waits_for_its_owner() {
        let base = base("legacy");
        let profile = write_profile(&base, "old");
        std::fs::create_dir_all(base.join("servers")).unwrap();
        std::fs::write(base.join("servers").join("s.json"), b"{}").unwrap();
        std::fs::write(base.join("node.key"), b"secret").unwrap();
        std::fs::write(base.join("account.json"), b"{}").unwrap();
        std::fs::write(base.join("device-data.key"), b"k").unwrap();
        std::fs::write(base.join("link.port"), b"1").unwrap();

        assert!(set_aside_unowned_data(&base) >= 3);
        assert!(!base.join("profile.json").exists());
        assert!(!base.join("node.key").exists());
        assert!(!base.join("servers").exists());
        assert!(base.join("account.json").exists(), "the sign-in stays shared");
        assert!(base.join("device-data.key").exists());
        assert!(base.join("link.port").exists());
        assert_eq!(set_aside_unowned_data(&base), 0);

        assert_eq!(unclaimed_node_secret(&base), None, "not a key");
        crate::persist::write_secret(&unclaimed_dir(&base).join("node.key"), "22".repeat(32).as_bytes()).unwrap();
        assert_eq!(unclaimed_node_secret(&base), Some([0x22; 32]));
        let found = unclaimed(&base).unwrap();
        assert_eq!(found.username, "old");
        assert_eq!(found.servers, 1);
        assert_eq!(choose(&base, SERVER, "user-b"), Opening::AskAboutUnclaimed);

        let fresh = claim(&base, SERVER, "user-b", false).unwrap();
        assert!(crate::persist::load_profile(&fresh).is_none());
        assert!(unclaimed(&base).is_some(), "declining leaves it for its owner");
        assert_eq!(choose(&base, SERVER, "user-b"), Opening::Ready(fresh));

        assert_eq!(choose(&base, SERVER, "user-a"), Opening::AskAboutUnclaimed);
        let kept = claim(&base, SERVER, "user-a", true).unwrap();
        assert_eq!(crate::persist::load_profile(&kept).unwrap().id, profile.id);
        assert!(kept.join("node.key").exists());
        assert!(unclaimed(&base).is_none());
        assert_eq!(choose(&base, SERVER, "user-c"), Opening::Ready(folder(&base, SERVER, "user-c")));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn claiming_an_existing_folder_keeps_it() {
        let base = base("existing");
        let dir = claim(&base, SERVER, "user-a", false).unwrap();
        let profile = write_profile(&dir, "mine");
        write_profile(&base, "legacy");
        set_aside_unowned_data(&base);
        let again = claim(&base, SERVER, "user-a", true).unwrap();
        assert_eq!(crate::persist::load_profile(&again).unwrap().id, profile.id);
        assert!(unclaimed(&base).is_some());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_locked_folder_opens_only_with_the_servers_key() {
        let base = base("lock");
        let dir = folder(&base, SERVER, "user-a");
        std::fs::create_dir_all(dir.join("servers")).unwrap();
        std::fs::write(dir.join("dms.json"), b"{\"hi\":1}").unwrap();
        std::fs::write(dir.join("servers").join("s.json"), b"{}").unwrap();
        std::fs::write(dir.join("node.port"), b"4242").unwrap();

        let server_key = "ab".repeat(32);
        let folder_key = [5u8; 32];
        let lock_id = new_lock_id();
        assert_eq!(lock_id.len(), 32);
        let made = wrap(&server_key, &lock_id, &folder_key).unwrap();
        assert!(!made.wrapped.contains(&data_encoding::HEXLOWER.encode(&folder_key)));
        assert_eq!(lock(&dir, &made, &folder_key).unwrap(), 2);
        assert!(is_locked(&dir));
        assert_eq!(read_lock(&dir).unwrap(), made);

        let sealed = std::fs::read(dir.join("dms.json")).unwrap();
        assert_eq!(commy_core::at_rest::open_with(Some(&folder_key), sealed.clone()).unwrap(), b"{\"hi\":1}");
        assert!(commy_core::at_rest::open_with(Some(&[0u8; 32]), sealed).is_err());
        assert_eq!(std::fs::read(dir.join("node.port")).unwrap(), b"4242");

        assert_eq!(unwrap(&server_key, &made), Some(folder_key));
        assert_eq!(unwrap(&"cd".repeat(32), &made), None, "another account's key does not open it");
        assert_eq!(unwrap("short", &made), None);
        assert_eq!(finish_resealing(&dir, &folder_key).unwrap(), 0);

        assert_eq!(cached_key(&dir), None);
        remember_key(&dir, &folder_key);
        assert_eq!(cached_key(&dir), Some(folder_key));
        forget_key(&dir);
        assert_eq!(cached_key(&dir), None, "signing out forgets the key");
        forget_key(&dir);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn an_interrupted_reseal_finishes_on_the_next_open() {
        let base = base("reseal");
        let dir = folder(&base, SERVER, "user-a");
        std::fs::create_dir_all(&dir).unwrap();
        let folder_key = [6u8; 32];
        std::fs::write(dir.join("profile.json"), b"{}").unwrap();
        std::fs::write(resealing_path(&dir), b"").unwrap();
        assert_eq!(finish_resealing(&dir, &folder_key).unwrap(), 1);
        assert!(!resealing_path(&dir).exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_window_marker_is_used_once() {
        let base = base("marker");
        assert!(!take_show_window(&base));
        show_window_on_next_start(&base);
        assert!(take_show_window(&base));
        assert!(!take_show_window(&base));
        let _ = std::fs::remove_dir_all(&base);
    }
}
