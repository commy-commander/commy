use std::collections::{BTreeMap, BTreeSet};

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

pub const MASS_LEAVE: usize = 3;
pub const EARLIER_PATH: &str = "vault/earlier";

pub fn is_mass_leave(leaving: usize) -> bool {
    leaving >= MASS_LEAVE
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct EarlierCopies {
    #[serde(default)]
    pub items: Vec<EarlierCopy>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EarlierCopy {
    pub version: u64,
    pub data: String,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub writer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EarlierView {
    pub version: u64,
    pub saved_at: Option<String>,
    pub writer: Option<String>,
    pub servers: usize,
    pub brings_back: Vec<String>,
}

pub fn open_with_any(keys: &[SealKey], sealed: &str) -> Option<EntryList> {
    keys.iter().find_map(|key| EntryList::open(key, sealed).ok())
}

pub fn earlier_views(copies: &[EarlierCopy], keys: &[SealKey], present: &BTreeSet<ServerId>) -> Vec<EarlierView> {
    let mut views: Vec<EarlierView> = Vec::new();
    let mut last_servers: Option<BTreeSet<ServerId>> = None;
    for copy in copies {
        let Some(list) = open_with_any(keys, &copy.data) else { continue };
        let servers: BTreeSet<ServerId> = list.servers.keys().copied().collect();
        if last_servers.as_ref() == Some(&servers) {
            continue;
        }
        views.push(EarlierView {
            version: copy.version,
            saved_at: copy.updated_at.clone(),
            writer: copy.writer.clone(),
            servers: servers.len(),
            brings_back: list.missing_from(present).into_iter().map(|(_, entry)| entry.name.clone()).collect(),
        });
        last_servers = Some(servers);
    }
    views
}

impl EntryList {
    pub fn missing_from(&self, present: &BTreeSet<ServerId>) -> Vec<(ServerId, &Entry)> {
        self.servers.iter().filter(|(id, _)| !present.contains(id)).map(|(id, entry)| (*id, entry)).collect()
    }

    pub fn brought_back(entry: &Entry, now: DateTime<Utc>) -> Entry {
        Entry { since: now.max(entry.since), ..entry.clone() }
    }
}

pub fn encode_identity(identity: &Identity) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data_encoding::HEXLOWER.encode(&identity.to_bytes()))
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

    pub fn fresh(username: &str, identity: &Identity) -> EntryList {
        let profile = UserProfile { identity: Some(identity.public()), ..UserProfile::new(username) };
        EntryList { profile: Some(profile), identity: Some(encode_identity(identity)), ..Default::default() }
    }

    pub fn is_empty_list(&self) -> bool {
        self.servers.is_empty() && self.left.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_list_carries_an_identity_every_device_can_read() {
        let identity = Identity::generate();
        let list = EntryList::fresh("ada", &identity);
        assert_eq!(list.shared_identity().map(|i| i.public()), Some(identity.public()));
        let profile = list.profile.as_ref().unwrap();
        assert_eq!(profile.username, "ada");
        assert_eq!(profile.identity, Some(identity.public()));
        assert!(list.is_empty_list());
        let key = SealKey::generate();
        assert_eq!(EntryList::open(&key, &list.seal(&key)).unwrap(), list);
    }

    fn entry(name: &str, since: DateTime<Utc>) -> Entry {
        Entry { name: name.into(), ticket: format!("ticket-{name}"), since }
    }

    fn list_of(servers: &[(ServerId, &str)]) -> EntryList {
        let since = Utc::now() - chrono::Duration::days(3);
        EntryList { servers: servers.iter().map(|(id, name)| (*id, entry(name, since))).collect(), ..Default::default() }
    }

    fn copy(version: u64, list: &EntryList, key: &SealKey) -> EarlierCopy {
        EarlierCopy { version, data: list.seal(key), updated_at: Some(format!("v{version}")), writer: Some("Commy test".into()) }
    }

    #[test]
    fn three_leaves_at_once_are_a_mass_leave() {
        assert!(!is_mass_leave(0));
        assert!(!is_mass_leave(2));
        assert!(is_mass_leave(3));
        assert!(is_mass_leave(40));
    }

    #[test]
    fn an_earlier_list_names_the_servers_it_brings_back() {
        let (a, b, c) = (ServerId::new(), ServerId::new(), ServerId::new());
        let key = SealKey::generate();
        let earlier = list_of(&[(a, "Rust Club"), (b, "Review"), (c, "Art")]);
        let present: BTreeSet<ServerId> = [a].into();
        let views = earlier_views(&[copy(4, &earlier, &key)], &[key], &present);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].version, 4);
        assert_eq!(views[0].servers, 3);
        let mut back = views[0].brings_back.clone();
        back.sort();
        assert_eq!(back, vec!["Art".to_string(), "Review".to_string()]);
    }

    #[test]
    fn copies_sealed_with_an_older_key_still_open_and_unreadable_ones_are_skipped() {
        let (a, b) = (ServerId::new(), ServerId::new());
        let (old, new, stranger) = (SealKey::generate(), SealKey::generate(), SealKey::generate());
        let copies = [copy(3, &list_of(&[(a, "A")]), &new), copy(2, &list_of(&[(a, "A"), (b, "B")]), &old), copy(1, &list_of(&[(b, "B")]), &stranger)];
        let views = earlier_views(&copies, &[new, old], &BTreeSet::new());
        assert_eq!(views.iter().map(|v| v.version).collect::<Vec<_>>(), vec![3, 2]);
    }

    #[test]
    fn copies_with_the_same_servers_in_a_row_show_once() {
        let a = ServerId::new();
        let key = SealKey::generate();
        let list = list_of(&[(a, "A")]);
        let views = earlier_views(&[copy(3, &list, &key), copy(2, &list, &key), copy(1, &EntryList::default(), &key)], &[key], &BTreeSet::new());
        assert_eq!(views.iter().map(|v| v.version).collect::<Vec<_>>(), vec![3, 1]);
    }

    #[test]
    fn a_brought_back_server_wins_over_the_leave_that_dropped_it() {
        let a = ServerId::new();
        let now = Utc::now();
        let earlier = list_of(&[(a, "A")]);
        let bad = EntryList { left: [(a, now - chrono::Duration::minutes(5))].into(), ..Default::default() };
        let back = EntryList::brought_back(&earlier.servers[&a], now);
        let restored = EntryList { servers: [(a, back)].into(), ..Default::default() };
        assert!(EntryList::merge(&bad, &restored).servers.contains_key(&a));
        assert!(!EntryList::merge(&bad, &earlier).servers.contains_key(&a));
    }

    #[test]
    fn the_earlier_copies_reply_reads_from_the_account_server() {
        let reply = r#"{"name":"servers","items":[{"version":2,"data":"sealed1.x","updatedAt":"2026-11-18T15:29:36.000Z","replacedAt":"2026-11-19T00:00:00.000Z","writer":null}]}"#;
        let copies: EarlierCopies = serde_json::from_str(reply).unwrap();
        assert_eq!(copies.items.len(), 1);
        assert_eq!(copies.items[0].version, 2);
        assert_eq!(copies.items[0].updated_at.as_deref(), Some("2026-11-18T15:29:36.000Z"));
        assert_eq!(copies.items[0].writer, None);
    }

    #[test]
    fn the_identity_reads_back_like_the_desktop_identity_file() {
        use base64::Engine;
        let identity = Identity::generate();
        let file = format!("{}\n", data_encoding::HEXLOWER.encode(&identity.to_bytes()));
        let from_file = base64::engine::general_purpose::STANDARD.encode(file);
        assert_eq!(decode_identity(&from_file).unwrap().public(), identity.public());
        assert_eq!(decode_identity(&encode_identity(&identity)).unwrap().public(), identity.public());
    }
}
