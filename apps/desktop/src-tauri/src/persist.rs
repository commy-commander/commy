use crate::state::VoiceSettings;
use anyhow::{Context, Result};
use commy_core::{DmStore, ServerId, ServerState, Snapshot, UserProfile};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(value)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &json)?;
    remove_target_windows_rename_cannot_replace(path)?;
    fs::rename(&tmp, path).context("atomic rename failed")?;
    Ok(())
}

fn remove_target_windows_rename_cannot_replace(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub(crate) fn write_secret(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tag = data_encoding::HEXLOWER.encode(&commy_net::CommyNode::generate_secret()[..6]);
    let tmp = path.with_extension(format!("{tag}.tmp"));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = (|| {
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        drop(file);
        if path.exists() {
            fs::remove_file(path)?;
        }
        fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

pub(crate) fn write_secret_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let json = serde_json::to_vec_pretty(value)?;
    write_secret(path, &json).with_context(|| format!("could not write {}", path.display()))
}

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(value) => Some(value),
        Err(e) => {
            tracing::warn!("ignoring corrupt state file {}: {e}", path.display());
            None
        }
    }
}

pub fn save_profile(dir: &Path, profile: &UserProfile) -> Result<()> {
    write_json(&dir.join("profile.json"), profile)
}

pub fn load_profile(dir: &Path) -> Option<UserProfile> {
    read_json(&dir.join("profile.json"))
}

pub fn save_voice(dir: &Path, voice: &VoiceSettings) -> Result<()> {
    write_json(&dir.join("voice.json"), voice)
}

pub fn load_voice(dir: &Path) -> Option<VoiceSettings> {
    read_json(&dir.join("voice.json"))
}

pub fn save_dms(dir: &Path, dms: &DmStore) -> Result<()> {
    write_secret_json(&dir.join("dms.json"), dms)
}

pub fn load_dms(dir: &Path) -> Option<DmStore> {
    read_json(&dir.join("dms.json"))
}

pub fn save_integrations(dir: &Path, creds: &crate::igdb::IgdbCredentials) -> Result<()> {
    write_json(&dir.join("integrations.json"), creds)
}

pub fn load_integrations(dir: &Path) -> Option<crate::igdb::IgdbCredentials> {
    read_json(&dir.join("integrations.json"))
}

pub fn save_safety(dir: &Path, safety: &crate::safety::SafetySettings) -> Result<()> {
    write_json(&dir.join("safety.json"), safety)
}

pub fn load_safety(dir: &Path) -> Option<crate::safety::SafetySettings> {
    read_json(&dir.join("safety.json"))
}

pub fn load_or_create_node_key(dir: &Path) -> Option<[u8; 32]> {
    let path = dir.join("node.key");
    if let Ok(hex) = fs::read_to_string(&path) {
        if let Ok(bytes) = data_encoding::HEXLOWER.decode(hex.trim().as_bytes()) {
            if let Ok(key) = <[u8; 32]>::try_from(bytes.as_slice()) {
                return Some(key);
            }
        }
        tracing::warn!("node.key is corrupt; generating a new identity");
    }
    let key = commy_net::CommyNode::generate_secret();
    write_secret(&path, data_encoding::HEXLOWER.encode(&key).as_bytes()).ok()?;
    Some(key)
}

pub fn save_awaiting(dir: &Path, servers: &HashSet<ServerId>) -> Result<()> {
    write_json(&dir.join("awaiting-snapshot.json"), servers)
}

pub fn load_awaiting(dir: &Path) -> HashSet<ServerId> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Saved {
        Listed(HashSet<ServerId>),
        Timed(HashMap<ServerId, i64>),
    }
    match read_json(&dir.join("awaiting-snapshot.json")) {
        Some(Saved::Listed(set)) => set,
        Some(Saved::Timed(map)) => map.into_keys().collect(),
        None => HashSet::new(),
    }
}

pub fn save_invites(dir: &Path, invites: &HashMap<ServerId, String>) -> Result<()> {
    write_secret_json(&dir.join("invites.json"), invites)
}

pub fn load_invites(dir: &Path) -> HashMap<ServerId, String> {
    read_json(&dir.join("invites.json")).unwrap_or_default()
}

fn server_path(dir: &Path, id: ServerId) -> PathBuf {
    dir.join("servers").join(format!("{id}.json"))
}

pub fn save_server(dir: &Path, state: &ServerState) -> Result<()> {
    write_json(&server_path(dir, state.server.id), &state.snapshot())
}

pub fn delete_server(dir: &Path, id: ServerId) -> Result<()> {
    let path = server_path(dir, id);
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn load_servers(dir: &Path) -> Vec<ServerState> {
    let servers_dir = dir.join("servers");
    let Ok(entries) = fs::read_dir(&servers_dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
        .filter_map(|e| read_json::<Snapshot>(&e.path()))
        .map(ServerState::from_snapshot)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Message, Server, ServerEvent, UserId};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("commy-persist-tests")
            .join(format!("{tag}-{}", uuid_like()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn uuid_like() -> String {
        format!("{:x}", std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos())
    }

    #[test]
    fn profile_roundtrip() {
        let dir = temp_dir("profile");
        assert!(load_profile(&dir).is_none());
        let profile = UserProfile::new("commy");
        save_profile(&dir, &profile).unwrap();
        assert_eq!(load_profile(&dir).unwrap(), profile);
        let mut updated = profile.clone();
        updated.custom_status = Some("building".to_string());
        save_profile(&dir, &updated).unwrap();
        assert_eq!(load_profile(&dir).unwrap(), updated);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn server_roundtrip_with_messages() {
        let dir = temp_dir("server");
        let identity = commy_core::Identity::generate();
        let mut profile = UserProfile::new("owner");
        profile.identity = Some(identity.public());
        let owner = profile.id;
        let server = Server::new("persisted", owner);
        let channel = server.channels[0].id;
        let mut state = ServerState::new(server);
        state.profiles.insert(owner, profile);
        for i in 0..3 {
            let mut message = Message::new(channel, owner, format!("m{i}"));
            message.sign(state.server.id, &identity);
            state
                .apply(ServerEvent::MessageCreate { message })
                .unwrap();
        }
        save_server(&dir, &state).unwrap();

        let loaded = load_servers(&dir);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].server.id, state.server.id);
        assert_eq!(loaded[0].messages(channel).len(), 3);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn corrupt_files_are_skipped() {
        let dir = temp_dir("corrupt");
        fs::create_dir_all(dir.join("servers")).unwrap();
        fs::write(dir.join("servers").join("bad.json"), b"{not json").unwrap();
        assert!(load_servers(&dir).is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn node_key_is_stable_across_loads() {
        let dir = temp_dir("nodekey");
        let first = load_or_create_node_key(&dir).unwrap();
        let second = load_or_create_node_key(&dir).unwrap();
        assert_eq!(first, second);
        fs::write(dir.join("node.key"), "not hex").unwrap();
        let third = load_or_create_node_key(&dir).unwrap();
        assert_ne!(first, third);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn secrets_are_written_privately_and_replaced_whole() {
        let dir = temp_dir("secret");
        let path = dir.join("nested").join("account.json");
        write_secret(&path, b"first").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"first");
        write_secret(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        let names: Vec<_> = fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1, "no temp file stays behind");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let key_path = dir.join("node.key");
        let key = load_or_create_node_key(&dir).unwrap();
        assert_eq!(load_or_create_node_key(&dir), Some(key));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&key_path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = key_path;
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dm_store_roundtrip() {
        let dir = temp_dir("dms");
        assert!(load_dms(&dir).is_none());
        let mut dms = DmStore::new();
        let (me, them) = (UserId::new(), UserId::new());
        dms.insert(them, commy_core::DmMessage::new(me, them, "hello"));
        save_dms(&dir, &dms).unwrap();
        let loaded = load_dms(&dir).unwrap();
        assert_eq!(loaded.conversation(them).len(), 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn delete_server_removes_file() {
        let dir = temp_dir("delete");
        let state = ServerState::new(Server::new("gone", UserId::new()));
        save_server(&dir, &state).unwrap();
        assert_eq!(load_servers(&dir).len(), 1);
        delete_server(&dir, state.server.id).unwrap();
        assert!(load_servers(&dir).is_empty());
        fs::remove_dir_all(&dir).ok();
    }
}
