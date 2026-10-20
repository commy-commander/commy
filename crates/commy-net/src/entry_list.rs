use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use commy_core::{Identity, ServerId, UserProfile};
use serde::{Deserialize, Serialize};

use crate::sealed::{Purpose, SealKey};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub ticket: String,
    pub since: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EntryList {
    #[serde(default)]
    pub profile: Option<UserProfile>,
    #[serde(default)]
    pub profile_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub identity: Option<String>,
    #[serde(default)]
    pub servers: BTreeMap<ServerId, Entry>,
    #[serde(default)]
    pub left: BTreeMap<ServerId, DateTime<Utc>>,
}

impl EntryList {
    pub fn merge(shared: &EntryList, local: &EntryList) -> EntryList {
        let mut left = shared.left.clone();
        for (id, at) in &local.left {
            let keep = left.get(id).map_or(true, |existing| at > existing);
            if keep {
                left.insert(*id, *at);
            }
        }
        let mut servers = shared.servers.clone();
        for (id, entry) in &local.servers {
            let newer = servers.get(id).map_or(true, |existing| entry.since > existing.since);
            if newer {
                servers.insert(*id, entry.clone());
            }
        }
        servers.retain(|id, entry| left.get(id).map_or(true, |at| entry.since > *at));
        let same_person = matches!((&shared.profile, &local.profile), (Some(a), Some(b)) if a.id == b.id);
        let local_newer = same_person && local.profile_at > shared.profile_at;
        let (profile, profile_at) = if shared.profile.is_none() || local_newer {
            (local.profile.clone(), local.profile_at)
        } else {
            (shared.profile.clone(), shared.profile_at)
        };
        EntryList {
            profile,
            profile_at,
            identity: if shared.profile.is_some() {
                shared.identity.clone().or_else(|| local.identity.clone().filter(|_| same_person))
            } else {
                local.identity.clone()
            },
            servers,
            left,
        }
    }

    pub fn seal(&self, key: &SealKey) -> String {
        key.seal(Purpose::EntryList, &serde_json::to_vec(self).expect("entry list serialization cannot fail"))
    }

    pub fn open(key: &SealKey, sealed: &str) -> Result<EntryList, String> {
        let bytes = key
            .open(Purpose::EntryList, sealed)
            .map_err(|_| "Your recovery code or device key does not open this server list".to_string())?;
        serde_json::from_slice(&bytes).map_err(|_| "The server list is damaged".to_string())
    }
}

pub fn decode_identity(encoded: &str) -> Option<Identity> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
    let text = std::str::from_utf8(&bytes).ok()?;
    let secret: [u8; 32] = data_encoding::HEXLOWER_PERMISSIVE.decode(text.trim().as_bytes()).ok()?.try_into().ok()?;
    Some(Identity::from_bytes(&secret))
}

impl EntryList {
    pub fn shared_identity(&self) -> Option<Identity> {
        self.identity.as_deref().and_then(decode_identity)
    }
}
