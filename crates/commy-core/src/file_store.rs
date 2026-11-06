use serde::{Deserialize, Serialize};

use crate::{Identity, Permissions, ServerState};

pub const MAX_FILE_STORE_CHARS: usize = 200;
pub const FILE_STORE_VERSION: u64 = 2;
pub const KEY_HEADER: &str = "x-commy-key";
pub const AT_HEADER: &str = "x-commy-at";
pub const SIG_HEADER: &str = "x-commy-sig";
pub const SETUP_HEADER: &str = "x-commy-setup";
pub const PAIR_PATH: &str = "_commy/pair";
pub const ROSTER_PATH: &str = "_commy/roster";
pub const ROSTER_TARGET: &str = "roster";
pub const MIN_SETUP_CODE: usize = 16;
const REQUEST_DOMAIN: &str = "commy-files/v2";
const ROSTER_DOMAIN: &str = "commy-files-roster/v2";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Roster {
    pub owner: String,
    pub managers: Vec<String>,
    pub members: Vec<String>,
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedRoster {
    pub roster: String,
    pub key: String,
    pub sig: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreInfo {
    pub max_bytes: u64,
    pub paired: bool,
}

pub fn request_message(method: &str, target: &str, at_ms: i64, length: u64) -> String {
    format!("{REQUEST_DOMAIN}\n{method}\n{target}\n{at_ms}\n{length}")
}

pub fn signed_headers(identity: &Identity, method: &str, target: &str, at_ms: i64, length: u64) -> [(&'static str, String); 3] {
    let sig = identity.sign(request_message(method, target, at_ms, length).as_bytes());
    [(KEY_HEADER, identity.public().to_hex()), (AT_HEADER, at_ms.to_string()), (SIG_HEADER, sig.to_hex())]
}

pub fn sign_roster(identity: &Identity, roster: &Roster) -> SignedRoster {
    let text = serde_json::to_string(roster).expect("a roster always serializes");
    let sig = identity.sign(format!("{ROSTER_DOMAIN}\n{text}").as_bytes());
    SignedRoster { roster: text, key: identity.public().to_hex(), sig: sig.to_hex() }
}

pub fn can_manage_storage(perms: Permissions) -> bool {
    perms.allows(Permissions::MANAGE_FILE_STORAGE)
}

pub fn roster_of(state: &ServerState, at_ms: i64) -> Option<Roster> {
    let server = &state.server;
    let owner = state.identity_of(server.owner)?.to_hex();
    let current: Vec<_> = server.members.iter().map(|m| m.user).filter(|u| server.is_current_member(*u) || *u == server.owner).collect();
    let keyed = |users: &mut dyn Iterator<Item = crate::UserId>| {
        let mut keys: Vec<String> = users.filter_map(|u| state.identity_of(u)).map(|k| k.to_hex()).collect();
        keys.sort();
        keys.dedup();
        keys
    };
    let managers = keyed(&mut current.iter().copied().filter(|u| *u != server.owner && can_manage_storage(server.permissions_for(*u))));
    let mut members = keyed(&mut current.iter().copied());
    if !members.contains(&owner) {
        members.push(owner.clone());
        members.sort();
    }
    let managers = managers.into_iter().filter(|k| *k != owner).collect();
    Some(Roster { owner, managers, members, at: at_ms })
}

pub fn same_people(a: &Roster, b: &Roster) -> bool {
    let sorted = |list: &[String]| {
        let mut list = list.to_vec();
        list.sort();
        list
    };
    a.owner == b.owner && sorted(&a.managers) == sorted(&b.managers) && sorted(&a.members) == sorted(&b.members)
}

pub fn roster_to_send(mine: &Roster, kept: &Roster, me: &str) -> Option<Roster> {
    let next = if me == kept.owner && me == mine.owner {
        Roster { at: mine.at.max(kept.at + 1), ..mine.clone() }
    } else if kept.managers.iter().any(|k| k == me) {
        Roster { owner: kept.owner.clone(), managers: kept.managers.clone(), members: mine.members.clone(), at: mine.at.max(kept.at + 1) }
    } else {
        return None;
    };
    (!same_people(&next, kept)).then_some(next)
}

pub fn answers_as_storage(body: &[u8]) -> Option<StoreInfo> {
    let info: serde_json::Value = serde_json::from_slice(body).ok()?;
    if info.get("commy_files")?.as_u64()? != FILE_STORE_VERSION {
        return None;
    }
    let max_bytes = info.get("max_bytes")?.as_u64()?;
    let paired = info.get("paired").and_then(serde_json::Value::as_bool).unwrap_or(false);
    Some(StoreInfo { max_bytes, paired })
}

pub fn setup_code_ok(code: &str) -> bool {
    let code = code.trim();
    code.chars().count() >= MIN_SETUP_CODE && code.len() <= 200 && code.chars().all(|c| c.is_ascii_graphic())
}

fn private_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".lan")
        || host.parse::<std::net::Ipv4Addr>().is_ok()
        || host.starts_with('[')
        || !host.contains('.')
}

fn host_ok(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| !label.is_empty() && label.len() <= 63 && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !label.starts_with('-') && !label.ends_with('-'))
        && !private_host(host)
}

fn path_ok(path: &str) -> bool {
    path.split('/').all(|part| part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) && part != "." && part != "..")
}

pub fn clean_file_store(input: &str) -> Option<String> {
    let text = input.trim();
    if text.chars().count() > MAX_FILE_STORE_CHARS {
        return None;
    }
    let rest = text.strip_prefix("https://")?;
    let rest = rest.trim_end_matches('/');
    let (host, path) = match rest.split_once('/') {
        Some((host, path)) => (host, Some(path)),
        None => (rest, None),
    };
    if !host_ok(host) || path.is_some_and(|p| p.is_empty() || !path_ok(p)) {
        return None;
    }
    let host = host.to_ascii_lowercase();
    Some(match path {
        Some(path) => format!("https://{host}/{path}"),
        None => format!("https://{host}"),
    })
}

pub fn is_file_store(url: &str) -> bool {
    clean_file_store(url).as_deref() == Some(url)
}

pub fn reaches_in_browser(url: &str) -> bool {
    url.strip_prefix("https://").and_then(|rest| rest.split('/').next()).is_some_and(|host| host.ends_with(".workers.dev"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_https_address_becomes_one_form() {
        let want = Some("https://commy-files.rockets.workers.dev".to_string());
        for input in [" https://commy-files.rockets.workers.dev ", "https://commy-files.rockets.workers.dev/", "https://Commy-Files.Rockets.workers.dev"] {
            assert_eq!(clean_file_store(input), want, "{input}");
        }
        assert_eq!(clean_file_store("https://files.example.org/commy/").as_deref(), Some("https://files.example.org/commy"));
        assert!(is_file_store("https://commy-files.rockets.workers.dev"));
        assert!(!is_file_store("https://commy-files.rockets.workers.dev/"));
    }

    #[test]
    fn only_a_public_https_address_is_accepted() {
        for input in [
            "",
            "http://files.example.org",
            "files.example.org",
            "javascript:alert(1)",
            "https://user@files.example.org",
            "https://files.example.org:8443",
            "https://files.example.org/x?y=1",
            "https://files.example.org/x#y",
            "https://files.example.org/../x",
            "https://localhost",
            "https://nas.local",
            "https://192.168.1.10",
            "https://[::1]",
            "https://intranet",
            "https://-bad.example.org",
        ] {
            assert_eq!(clean_file_store(input), None, "{input}");
        }
        assert_eq!(clean_file_store(&format!("https://{}.example.org", "a".repeat(MAX_FILE_STORE_CHARS))), None);
    }

    fn storage_role(state: &mut ServerState, user: crate::UserId, permissions: Permissions) {
        let server = state.server.id;
        let role = crate::Role {
            id: crate::RoleId::new(),
            name: "storage".into(),
            color: None,
            permissions,
            position: 1,
            hoist: false,
            self_assignable: false,
            mentionable: false,
            tag: None,
            banner: None,
        };
        let id = role.id;
        state.apply(crate::ServerEvent::RoleCreate { server, role }).unwrap();
        state.apply(crate::ServerEvent::RoleAssign { server, user, role: id }).unwrap();
    }

    #[test]
    fn the_member_list_names_the_owner_storage_managers_and_everyone_else() {
        use crate::test_support::{fixture, identity_for, join};
        let (mut state, _, owner) = fixture();
        let keeper = join(&mut state, "keeper");
        let admin = join(&mut state, "admin");
        let manager = join(&mut state, "manager");
        let friend = join(&mut state, "friend");
        storage_role(&mut state, keeper, Permissions::MANAGE_FILE_STORAGE);
        storage_role(&mut state, admin, Permissions::ADMINISTRATOR);
        storage_role(&mut state, manager, Permissions::MANAGE_SERVER);
        let key = |u| identity_for(u).public().to_hex();
        let roster = roster_of(&state, 7).unwrap();
        assert_eq!(roster.owner, key(owner));
        let mut managers = vec![key(keeper), key(admin)];
        managers.sort();
        assert_eq!(roster.managers, managers, "Manage Server alone does not reach storage");
        assert_eq!(roster.members.len(), 5);
        assert!(roster.members.contains(&key(friend)) && roster.members.contains(&key(owner)));
        assert_eq!(roster.at, 7);
        let set = crate::ServerEvent::FileStoreSet { server: state.server.id, url: None };
        assert!(crate::action::authorized(&state.server, keeper, &set));
        assert!(crate::action::authorized(&state.server, admin, &set));
        assert!(!crate::action::authorized(&state.server, manager, &set), "connecting storage needs its own permission");
        state.apply(crate::ServerEvent::MemberLeave { server: state.server.id, user: friend }).unwrap();
        assert!(!roster_of(&state, 8).unwrap().members.contains(&key(friend)), "someone who leaves drops off the list");
    }

    #[test]
    fn requests_and_lists_are_signed_by_the_members_identity() {
        use crate::test_support::identity_for;
        let me = identity_for(crate::UserId::new());
        let [(k, key), (a, at), (s, sig)] = signed_headers(&me, "PUT", "ab", 1_000, 65_552);
        assert_eq!((k, a, s), (KEY_HEADER, AT_HEADER, SIG_HEADER));
        assert_eq!((key.as_str(), at.as_str()), (me.public().to_hex().as_str(), "1000"));
        let sig = crate::Signature::from_hex(&sig).unwrap();
        assert!(me.public().verify(b"commy-files/v2\nPUT\nab\n1000\n65552", &sig));
        let roster = Roster { owner: key.clone(), managers: vec![], members: vec![key.clone()], at: 5 };
        let signed = sign_roster(&me, &roster);
        assert_eq!(serde_json::from_str::<Roster>(&signed.roster).unwrap(), roster);
        let sig = crate::Signature::from_hex(&signed.sig).unwrap();
        assert!(me.public().verify(format!("commy-files-roster/v2\n{}", signed.roster).as_bytes(), &sig));
    }

    #[test]
    fn signatures_match_the_ones_the_worker_checks() {
        let me = Identity::from_bytes(&[7u8; 32]);
        let [(_, signer), _, (_, sig)] = signed_headers(&me, "GET", &"ab".repeat(32), 1_762_430_400_000, 0);
        assert_eq!(signer, "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c");
        assert_eq!(sig, "ac92251a45380696c83571842764f431fb3987334cfe4078b6b56e5be7a2ab0399775d6cd2f8f69d7db0d92d3ce2d6768189f73c30975946de0b555bbc145702");
        let roster = Roster { owner: signer.clone(), managers: vec![], members: vec![signer], at: 1_762_430_400_000 };
        assert_eq!(sign_roster(&me, &roster).sig, "8dcdd3d0c9dc197d99394d9cea0005708b1576c4a379a815cc4942955ad9c5573ff32db33c4bbaafcfb099bc27ed41761b55054ec031da1768deb656f2df190a");
    }

    #[test]
    fn owners_send_the_whole_list_and_managers_only_members() {
        let kept = Roster { owner: "o".into(), managers: vec!["m".into()], members: vec!["o".into(), "m".into(), "a".into()], at: 50 };
        assert_eq!(roster_to_send(&Roster { at: 10, ..kept.clone() }, &kept, "o"), None, "nothing changed");
        let mine = Roster { owner: "o".into(), managers: vec!["m".into(), "n".into()], members: vec!["o".into(), "m".into(), "n".into()], at: 10 };
        let sent = roster_to_send(&mine, &kept, "o").unwrap();
        assert_eq!((sent.managers.len(), sent.at), (2, 51), "a later time than the kept list");
        let sent = roster_to_send(&mine, &kept, "m").unwrap();
        assert_eq!(sent.managers, kept.managers, "managers keep the owner's managers");
        assert_eq!(sent.members, mine.members);
        assert_eq!(roster_to_send(&mine, &kept, "a"), None, "members never send a list");
        assert_eq!(roster_to_send(&mine, &kept, "n"), None, "a new manager waits for the owner");
    }

    #[test]
    fn storage_answers_with_its_version_size_and_setup() {
        assert_eq!(answers_as_storage(br#"{"commy_files":2,"max_bytes":31457280,"paired":true}"#), Some(StoreInfo { max_bytes: 31_457_280, paired: true }));
        assert_eq!(answers_as_storage(br#"{"commy_files":2,"max_bytes":1}"#), Some(StoreInfo { max_bytes: 1, paired: false }));
        assert_eq!(answers_as_storage(br#"{"commy_files":1,"max_bytes":1}"#), None, "the first worker must be updated");
        assert_eq!(answers_as_storage(b"<html>"), None);
    }

    #[test]
    fn setup_codes_are_long_and_plain() {
        assert!(setup_code_ok(" correct-horse-battery-staple "));
        assert!(!setup_code_ok("short"));
        assert!(!setup_code_ok("has spaces in the middle"));
    }

    #[test]
    fn browsers_reach_workers_dev_addresses() {
        assert!(reaches_in_browser("https://commy-files.rockets.workers.dev"));
        assert!(!reaches_in_browser("https://files.example.org"));
        assert!(!reaches_in_browser("https://workers.dev.example.org"));
    }
}
