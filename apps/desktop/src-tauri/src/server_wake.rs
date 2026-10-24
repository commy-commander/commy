use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use commy_core::mention::{mentions_everyone, names_mentioned, role_tokens};
use commy_core::{ChannelId, Message, ServerId, ServerState, UserId};
use serde_json::Value;

use crate::state::App;
use crate::{account_log, channel_lock, community_sync, push_wake};

pub const ALL: &str = "all";
pub const EVERYONE: &str = "everyone";
const ALL_RING_GAP: Duration = Duration::from_secs(60);
const PER_BUCKET: usize = 8;
const MAX_BUCKETS: usize = 16;

static LAST_ALL: Mutex<Option<HashMap<ServerId, Instant>>> = Mutex::new(None);

pub fn role_who(role: impl std::fmt::Display) -> String {
    format!("role:{role}")
}

pub fn buckets(members: usize) -> usize {
    members.div_ceil(PER_BUCKET).next_power_of_two().clamp(1, MAX_BUCKETS)
}

fn bucket_of(user: UserId, buckets: usize) -> usize {
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

fn all_due(server: ServerId, now: Instant) -> bool {
    let mut guard = LAST_ALL.lock().unwrap();
    let last = guard.get_or_insert_with(HashMap::new);
    last.retain(|_, at| now.duration_since(*at) < ALL_RING_GAP);
    if last.contains_key(&server) {
        return false;
    }
    last.insert(server, now);
    true
}

pub fn ring(app: &App, server: ServerId, message: &Message) {
    let Some(me) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { return };
    if message.author != me {
        return;
    }
    let found = {
        let servers = app.servers.lock().unwrap();
        servers.get(&server).and_then(|state| {
            let private = state.server.channel(message.channel).is_some_and(|c| c.is_private());
            let opened = channel_lock::holder_of(app).open(state, message)?;
            Some((private, ring_whos(state, me, &opened.content, all_due(server, Instant::now()))))
        })
    };
    let Some((private, whos)) = found else { return };
    let Some(keys) = community_sync::wake_keys(server, private.then_some(message.channel)) else { return };
    let today = push_wake::today();
    let tags: Vec<[u8; 32]> = whos.iter().map(|who| keys.wake_tag(who, today)).collect();
    if tags.is_empty() {
        return;
    }
    tauri::async_runtime::spawn(push_wake::ring(app.http.clone(), tags));
}

#[derive(Default)]
pub struct ServerTags {
    pub tags: Vec<(ServerId, ([u8; 32], i64))>,
    pub unsettled: HashSet<ServerId>,
    pub unwanted: Vec<(ServerId, ([u8; 32], i64))>,
}

fn settled(state: &ServerState, me: UserId, live: impl Fn(Option<ChannelId>) -> bool) -> bool {
    let server = &state.server;
    if !server.is_current_member(me) || state.is_deleted() {
        return true;
    }
    live(None) && server.channels.iter().filter(|c| c.kind.holds_messages() && c.is_private() && server.can_view(c.id, me)).all(|c| live(Some(c.id)))
}

pub fn wake_tags(app: &App, days: &[i64]) -> ServerTags {
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let Some(me) = me else { return ServerTags { unsettled: app.servers.lock().unwrap().keys().copied().collect(), ..Default::default() } };
    let prefs = account_log::pref_values(app);
    let points = community_sync::wake_points();
    let servers = app.servers.lock().unwrap();
    let live: HashSet<(ServerId, Option<ChannelId>)> = points.iter().map(|(server, channel, _)| (*server, *channel)).collect();
    let unsettled = servers.iter().filter(|(server, state)| !settled(state, me, |channel| live.contains(&(**server, channel)))).map(|(server, _)| *server).collect();
    let mut tags = Vec::new();
    let mut unwanted = Vec::new();
    for (server, channel, keys) in points {
        let Some(state) = servers.get(&server) else { continue };
        if channel.is_some_and(|c: ChannelId| !state.server.can_view(c, me)) {
            continue;
        }
        let chosen = choice(&prefs, server);
        let tagged = |whos: Vec<String>| -> Vec<(ServerId, ([u8; 32], i64))> {
            whos.iter().flat_map(|who| days.iter().map(|day| (server, (keys.wake_tag(who, *day), *day)))).collect()
        };
        tags.extend(tagged(listen_whos(state, me, &chosen)));
        unwanted.extend(tagged(unwanted_whos(state, me, &chosen)));
    }
    ServerTags { tags, unsettled, unwanted }
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
    fn a_server_counts_as_settled_only_once_every_stream_it_listens_on_is_live() {
        let (mut state, ids) = server_with(&["kit", "sam"]);
        let (kit, sam) = (ids[1], ids[2]);
        let role = crew(&mut state, kit);
        let mut locked = commy_core::server::Channel::new("crew", commy_core::ChannelKind::Text, 9);
        locked.view_roles = vec![role];
        let channel = locked.id;
        state.server.channels.push(locked);
        assert!(!settled(&state, kit, |_| false));
        assert!(!settled(&state, kit, |c| c == Some(channel)), "the server's own stream is not live yet");
        assert!(!settled(&state, kit, |c| c.is_none()), "a locked channel kit can read is not live yet");
        assert!(settled(&state, kit, |_| true));
        assert!(settled(&state, sam, |c| c.is_none()), "sam cannot read the locked channel");
        let stranger = UserId::new();
        assert!(settled(&state, stranger, |_| false), "a server we left wants nothing");
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
        let start = Instant::now();
        assert!(all_due(a, start));
        assert!(!all_due(a, start + Duration::from_secs(30)));
        assert!(all_due(b, start + Duration::from_secs(30)));
        assert!(all_due(a, start + ALL_RING_GAP));
    }
}
