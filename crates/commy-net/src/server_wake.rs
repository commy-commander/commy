use std::collections::{BTreeMap, HashMap};

use commy_core::mention::{mentions_everyone, names_mentioned, role_tokens};
use commy_core::{Attachment, ServerId, ServerState, UserId};
use serde_json::Value;

pub const ALL: &str = "all";
pub const EVERYONE: &str = "everyone";
pub const ALL_RING_GAP_MS: i64 = 60_000;
const PER_BUCKET: usize = 8;
const MAX_BUCKETS: usize = 16;

pub fn role_who(role: impl std::fmt::Display) -> String {
    format!("role:{role}")
}

pub fn buckets(members: usize) -> usize {
    members.div_ceil(PER_BUCKET).next_power_of_two().clamp(1, MAX_BUCKETS)
}

pub fn bucket_of(user: UserId, buckets: usize) -> usize {
    use sha2::{Digest, Sha256};
    Sha256::digest(user.to_string().as_bytes())[0] as usize % buckets
}

fn shared(base: &str, bucket: usize) -> String {
    format!("{base}#{bucket}")
}

fn every_bucket(base: &str, buckets: usize) -> impl Iterator<Item = String> + '_ {
    (0..buckets).map(move |bucket| shared(base, bucket))
}

fn names_of(state: &ServerState, user: UserId) -> Vec<String> {
    let profile = state.profiles.get(&user);
    let nickname = state.server.member(user).and_then(|m| m.nickname.clone());
    [profile.map(|p| p.username.clone()), profile.and_then(|p| p.display_name.clone()), nickname].into_iter().flatten().collect()
}

pub fn ring_whos(state: &ServerState, me: UserId, content: &str, all: bool) -> Vec<String> {
    let server = &state.server;
    let buckets = buckets(server.members.len());
    let mut whos: Vec<String> = if all { every_bucket(ALL, buckets).collect() } else { Vec::new() };
    for member in &server.members {
        if member.user == me {
            continue;
        }
        let names = names_of(state, member.user);
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        if names_mentioned(content, &names) {
            whos.push(member.user.to_string());
        }
    }
    if mentions_everyone(content) && server.can_mention_everyone(me) {
        whos.extend(every_bucket(EVERYONE, buckets));
    }
    for role in role_tokens(content) {
        if server.can_mention_role(me, role) {
            whos.extend(every_bucket(&role_who(role), buckets));
        }
    }
    whos
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub level: String,
    pub muted: bool,
    pub suppress_everyone: bool,
    pub suppress_roles: bool,
}

fn pref<'a>(prefs: &'a BTreeMap<String, Value>, key: &str) -> Option<&'a Value> {
    prefs.get(&format!("notify.{key}")).filter(|v| !v.is_null())
}

fn flag(prefs: &BTreeMap<String, Value>, key: &str) -> bool {
    pref(prefs, key).and_then(Value::as_bool).unwrap_or(false)
}

pub fn wake_text(content: &str, attachments: &[Attachment]) -> String {
    if commy_core::message::only_a_voice_message(content, attachments.iter()) {
        return "Voice message".to_string();
    }
    if !content.trim().is_empty() {
        return content.to_string();
    }
    if attachments.iter().any(|a| !a.is_card()) { "Sent an attachment".to_string() } else { String::new() }
}

pub fn choice(prefs: &BTreeMap<String, Value>, server: ServerId) -> Choice {
    let own = pref(prefs, &format!("servers.{server}")).and_then(Value::as_str).filter(|level| *level != "default");
    let level = own.or_else(|| pref(prefs, "level").and_then(Value::as_str)).unwrap_or(ALL);
    Choice {
        level: level.to_string(),
        muted: flag(prefs, &format!("muted.{server}")),
        suppress_everyone: flag(prefs, &format!("suppressEveryone.{server}")),
        suppress_roles: flag(prefs, &format!("suppressRoles.{server}")),
    }
}

pub fn listen_whos(state: &ServerState, me: UserId, choice: &Choice) -> Vec<String> {
    if choice.muted {
        return Vec::new();
    }
    let bucket = bucket_of(me, buckets(state.server.members.len()));
    match choice.level.as_str() {
        "none" => Vec::new(),
        "mentions" => {
            let mut whos = vec![me.to_string()];
            if !choice.suppress_everyone {
                whos.push(shared(EVERYONE, bucket));
            }
            if !choice.suppress_roles {
                let roles = state.server.member(me).map(|m| m.roles.clone()).unwrap_or_default();
                whos.extend(roles.into_iter().map(|role| shared(&role_who(role), bucket)));
            }
            whos
        }
        _ => vec![shared(ALL, bucket)],
    }
}

pub fn unwanted_whos(state: &ServerState, me: UserId, choice: &Choice) -> Vec<String> {
    let bucket = bucket_of(me, buckets(state.server.members.len()));
    let roles = state.server.member(me).map(|m| m.roles.clone()).unwrap_or_default();
    let mut every = vec![shared(ALL, bucket), me.to_string(), shared(EVERYONE, bucket)];
    every.extend(roles.into_iter().map(|role| shared(&role_who(role), bucket)));
    let wanted = listen_whos(state, me, choice);
    every.retain(|who| !wanted.contains(who));
    every
}

#[derive(Debug, Default)]
pub struct AllRings {
    last: HashMap<ServerId, i64>,
}

impl AllRings {
    pub fn due(&mut self, server: ServerId, now_ms: i64) -> bool {
        self.last.retain(|_, at| now_ms - *at < ALL_RING_GAP_MS);
        if self.last.contains_key(&server) {
            return false;
        }
        self.last.insert(server, now_ms);
        true
    }
}

pub fn wake_url(sync_url: &str) -> Option<String> {
    let (scheme, rest) = match sync_url.strip_prefix("wss://") {
        Some(rest) => ("https", rest),
        None => ("http", sync_url.strip_prefix("ws://")?),
    };
    Some(format!("{scheme}://{}/wake", rest.trim_end_matches('/')))
}

pub fn ring_url(sync_url: &str) -> Option<String> {
    wake_url(sync_url).map(|base| format!("{base}/ring"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::mention::role_token;
    use commy_core::permissions::Permissions;
    use commy_core::server::Role;
    use commy_core::{RoleId, ServerEvent, UserProfile};

    fn server_with(names: &[&str]) -> (ServerState, Vec<UserId>) {
        let owner = UserProfile::new("owner");
        let mut state = ServerState::new(commy_core::Server::new("test", owner.id));
        state.profiles.insert(owner.id, owner.clone());
        let mut ids = vec![owner.id];
        for name in names {
            let profile = UserProfile::new(*name);
            ids.push(profile.id);
            state.apply(ServerEvent::MemberJoin { server: state.server.id, profile }).unwrap();
        }
        (state, ids)
    }

    #[test]
    fn a_wake_preview_says_what_was_sent() {
        let voice = Attachment::voice_message("b".into(), 10, None);
        let file = Attachment { blob: "b".into(), filename: "cat.png".into(), size_bytes: 10, mime_type: "image/png".into(), provider: None };
        assert_eq!(wake_text("hello", &[]), "hello");
        assert_eq!(wake_text("", std::slice::from_ref(&voice)), "Voice message");
        assert_eq!(wake_text(" ", std::slice::from_ref(&file)), "Sent an attachment");
        assert_eq!(wake_text("look", &[file]), "look");
        assert_eq!(wake_text("", &[Attachment::tts_marker()]), "");
    }

    fn crew(state: &mut ServerState, holder: UserId) -> RoleId {
        let role = Role {
            id: RoleId::new(),
            name: "crew".into(),
            color: None,
            permissions: Permissions::NONE,
            position: 1,
            hoist: false,
            self_assignable: false,
            tag: None,
            banner: None,
            mentionable: true,
        };
        let id = role.id;
        let server = state.server.id;
        state.apply(ServerEvent::RoleCreate { server, role }).unwrap();
        state.apply(ServerEvent::RoleAssign { server, user: holder, role: id }).unwrap();
        id
    }

    #[test]
    fn a_message_rings_everyone_listening_and_each_person_it_names() {
        let (mut state, ids) = server_with(&["sam", "kit"]);
        let (owner, sam, kit) = (ids[0], ids[1], ids[2]);
        let role = crew(&mut state, kit);
        let all = shared(ALL, 0);
        assert_eq!(ring_whos(&state, owner, "hello", true), vec![all.clone()]);
        assert!(ring_whos(&state, owner, "hello", false).is_empty(), "a busy server rings everyone at most once a minute");
        assert_eq!(ring_whos(&state, owner, "hey @sam", true), vec![all.clone(), sam.to_string()]);
        assert_eq!(ring_whos(&state, owner, "@here and @owner", false), vec![shared(EVERYONE, 0)], "you never ring yourself");
        assert!(ring_whos(&state, sam, "@everyone", false).is_empty(), "only members allowed to may wake everyone");
        let ping = format!("{} up?", role_token(role));
        assert_eq!(ring_whos(&state, sam, &ping, false), vec![shared(&role_who(role), 0)]);
    }

    #[test]
    fn a_phone_listens_for_what_its_settings_ask_for() {
        let (mut state, ids) = server_with(&["kit"]);
        let kit = ids[1];
        let role = crew(&mut state, kit);
        let server = state.server.id;
        let mut prefs = BTreeMap::new();
        let listen = |prefs: &BTreeMap<String, Value>| listen_whos(&state, kit, &choice(prefs, server));
        assert_eq!(listen(&prefs), vec![shared(ALL, 0)], "everything by default");
        prefs.insert("notify.level".into(), Value::from("mentions"));
        assert_eq!(listen(&prefs), vec![kit.to_string(), shared(EVERYONE, 0), shared(&role_who(role), 0)]);
        prefs.insert(format!("notify.suppressEveryone.{server}"), Value::from(true));
        prefs.insert(format!("notify.suppressRoles.{server}"), Value::from(true));
        assert_eq!(listen(&prefs), vec![kit.to_string()]);
        prefs.insert(format!("notify.servers.{server}"), Value::from("all"));
        assert_eq!(listen(&prefs), vec![shared(ALL, 0)], "the server's own level wins");
        prefs.insert(format!("notify.servers.{server}"), Value::from("default"));
        assert_eq!(listen(&prefs), vec![kit.to_string()]);
        prefs.insert(format!("notify.muted.{server}"), Value::from(true));
        assert!(listen(&prefs).is_empty());
        prefs.insert(format!("notify.muted.{server}"), Value::Null);
        prefs.insert(format!("notify.servers.{server}"), Value::from("none"));
        assert!(listen(&prefs).is_empty());
    }

    #[test]
    fn a_phone_drops_what_its_settings_no_longer_ask_for() {
        let (mut state, ids) = server_with(&["kit"]);
        let kit = ids[1];
        let role = crew(&mut state, kit);
        let server = state.server.id;
        let mut prefs = BTreeMap::new();
        let unwanted = |prefs: &BTreeMap<String, Value>| unwanted_whos(&state, kit, &choice(prefs, server));
        assert_eq!(unwanted(&prefs), vec![kit.to_string(), shared(EVERYONE, 0), shared(&role_who(role), 0)]);
        prefs.insert("notify.level".into(), Value::from("mentions"));
        assert_eq!(unwanted(&prefs), vec![shared(ALL, 0)]);
        prefs.insert(format!("notify.suppressEveryone.{server}"), Value::from(true));
        assert_eq!(unwanted(&prefs), vec![shared(ALL, 0), shared(EVERYONE, 0)]);
        prefs.insert(format!("notify.muted.{server}"), Value::from(true));
        assert_eq!(unwanted(&prefs), vec![shared(ALL, 0), kit.to_string(), shared(EVERYONE, 0), shared(&role_who(role), 0)]);
    }

    #[test]
    fn big_servers_split_shared_tags_so_each_holds_few_phones() {
        assert_eq!(buckets(1), 1);
        assert_eq!(buckets(8), 1);
        assert_eq!(buckets(9), 2);
        assert_eq!(buckets(40), 8);
        assert_eq!(buckets(500), MAX_BUCKETS);
        let (state, ids) = server_with(&(0..20).map(|n| format!("m{n}")).collect::<Vec<_>>().iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(buckets(state.server.members.len()), 4);
        let rung = ring_whos(&state, ids[0], "hi", true);
        assert_eq!(rung.len(), 4);
        for member in &ids[1..] {
            let heard = listen_whos(&state, *member, &Choice { level: ALL.into(), muted: false, suppress_everyone: false, suppress_roles: false });
            assert_eq!(heard.len(), 1);
            assert!(rung.contains(&heard[0]), "every member listens on a tag the sender rings");
        }
    }

    #[test]
    fn the_all_messages_ring_waits_a_minute_per_server() {
        let (a, b) = (ServerId::new(), ServerId::new());
        let mut rings = AllRings::default();
        let start = 1_000_000;
        assert!(rings.due(a, start));
        assert!(!rings.due(a, start + 30_000));
        assert!(rings.due(b, start + 30_000));
        assert!(rings.due(a, start + ALL_RING_GAP_MS));
    }

    #[test]
    fn wake_calls_go_to_the_sync_service_over_https() {
        assert_eq!(wake_url("wss://commy.chat/sync").as_deref(), Some("https://commy.chat/sync/wake"));
        assert_eq!(ring_url("ws://localhost:3012/").as_deref(), Some("http://localhost:3012/wake/ring"));
        assert_eq!(ring_url("https://commy.chat/sync"), None);
    }
}
