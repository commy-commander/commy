use std::collections::{BTreeMap, HashSet};
use std::sync::Mutex;

use commy_core::{ChannelId, Message, ServerId, ServerState, UserId};
use commy_net::server_wake::{choice, listen_whos, ring_whos, unwanted_whos, wake_text, AllRings};

use crate::state::App;
use crate::{account_log, channel_lock, community_sync, push_wake};

static LAST_ALL: Mutex<Option<AllRings>> = Mutex::new(None);

fn all_due(server: ServerId) -> bool {
    LAST_ALL.lock().unwrap().get_or_insert_with(AllRings::default).due(server, chrono::Utc::now().timestamp_millis())
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
            let text = wake_text(&opened.content, &message.attachments);
            Some((private, ring_whos(state, me, &opened.content, all_due(server)), text))
        })
    };
    let Some((private, whos, text)) = found else { return };
    let Some(keys) = community_sync::wake_keys(server, private.then_some(message.channel)) else { return };
    let today = push_wake::today();
    let tags: Vec<[u8; 32]> = whos.iter().map(|who| keys.wake_tag(who, today)).collect();
    if tags.is_empty() {
        return;
    }
    let preview = push_wake::preview(&keys, &me.to_string(), Some(message.channel.to_string()), &text, false, false);
    tauri::async_runtime::spawn(push_wake::ring(app.http.clone(), tags, preview));
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

pub fn preview_keys(app: &App, days: &[i64]) -> Vec<(ServerId, [u8; 32])> {
    let Some(me) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { return Vec::new() };
    let points = community_sync::wake_points();
    let servers = app.servers.lock().unwrap();
    points
        .into_iter()
        .filter(|(server, channel, _)| servers.get(server).is_some_and(|state| channel.is_none_or(|c| state.server.can_view(c, me))))
        .flat_map(|(server, _, keys)| days.iter().map(move |day| (server, keys.preview_key(*day))))
        .collect()
}

#[derive(Debug, PartialEq, serde::Serialize)]
pub struct Label {
    #[serde(rename = "n")]
    name: String,
    #[serde(rename = "g")]
    group: bool,
    #[serde(rename = "c")]
    channels: BTreeMap<String, String>,
    #[serde(rename = "m")]
    members: BTreeMap<String, String>,
}

pub fn label(state: &ServerState, me: UserId) -> Option<Label> {
    let server = &state.server;
    if !server.is_current_member(me) || state.is_deleted() {
        return None;
    }
    let channels = server.channels.iter().filter(|c| c.kind.holds_messages() && server.can_view(c.id, me)).map(|c| (c.id.to_string(), c.name.clone())).collect();
    let members = server
        .members
        .iter()
        .filter_map(|m| commy_view::known_name_of(state, m.user).map(|name| (m.user.to_string(), name)))
        .collect();
    Some(Label { name: commy_core::server::shown_server_name(&server.name), group: server.group, channels, members })
}

pub fn labels(app: &App) -> BTreeMap<String, Label> {
    let Some(me) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { return BTreeMap::new() };
    app.servers.lock().unwrap().iter().filter_map(|(id, state)| label(state, me).map(|l| (id.to_string(), l))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn a_phone_learns_only_the_channels_it_can_read() {
        let (mut state, ids) = server_with(&["kit", "sam"]);
        let (kit, sam) = (ids[1], ids[2]);
        let role = crew(&mut state, kit);
        let mut locked = commy_core::server::Channel::new("crew", commy_core::ChannelKind::Text, 9);
        locked.view_roles = vec![role];
        let channel = locked.id.to_string();
        state.server.channels.push(locked);
        let for_kit = label(&state, kit).unwrap();
        let for_sam = label(&state, sam).unwrap();
        assert_eq!(for_kit.channels.get(&channel).map(String::as_str), Some("crew"));
        assert!(!for_sam.channels.contains_key(&channel));
        assert_eq!(for_sam.members.get(&kit.to_string()).map(String::as_str), Some("kit"));
        assert_eq!(for_sam.name, "test");
        assert!(label(&state, UserId::new()).is_none(), "a server we are not in shows nothing");
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
}
