use crate::state::App;
use commy_core::ids::CALL_SERVER;
use commy_core::{ChannelId, ServerEvent, ServerId, ServerState, UserId, UserProfile};
use tauri::{Emitter, Manager};

fn announcement(app: &App, server: ServerId) -> Option<ServerEvent> {
    let user = app.profile.lock().unwrap().as_ref().map(|p| p.id)?;
    let node = app.node_hex.lock().unwrap().clone()?;
    let (voice, since) = match app.voice_session.lock().unwrap().as_ref().filter(|s| s.server == server) {
        Some(session) => (Some(session.channel), Some(session.joined_ms)),
        None => (None, None),
    };
    Some(ServerEvent::DeviceAnnounce { server, user, node, voice, since })
}

pub(crate) async fn announce(app: &App, server: ServerId) {
    if server == CALL_SERVER {
        return;
    }
    let Some(event) = announcement(app, server) else { return };
    if let Some(state) = app.servers.lock().unwrap().get_mut(&server) {
        let _ = state.apply(event.clone());
    }
    crate::broadcast_event(app, server, event).await;
}

fn has_sibling(state: &ServerState, me: UserId, ours: &str) -> bool {
    state.devices_of(me).iter().any(|d| !d.node.eq_ignore_ascii_case(ours))
}

pub(crate) async fn claim_profile_node(app: &App, server: ServerId) {
    let Some(me) = app.profile.lock().unwrap().clone() else { return };
    let Some(ours) = app.node_hex.lock().unwrap().clone() else { return };
    let siblings = app.servers.lock().unwrap().get(&server).is_some_and(|s| has_sibling(s, me.id, &ours));
    if siblings {
        let profile = app.announced(UserProfile { node: Some(ours), ..me });
        crate::broadcast_event(app, server, ServerEvent::MemberJoin { server, profile }).await;
    }
}

pub(crate) fn point_at_voice(state: &ServerState, profile: &mut UserProfile) {
    if let Some(node) = state.voice_device(profile.id) {
        profile.node = Some(node.to_string());
    }
}

pub(crate) fn takes_over(me: UserId, ours: &str, joined_ms: i64, event: &ServerEvent) -> bool {
    let ServerEvent::DeviceAnnounce { user, node, voice: Some(_), since: Some(since), .. } = event else { return false };
    *user == me && !node.eq_ignore_ascii_case(ours) && *since > joined_ms
}

pub(crate) async fn on_announce(handle: &tauri::AppHandle, server: ServerId, event: &ServerEvent) {
    let ServerEvent::DeviceAnnounce { user, node, voice, .. } = event else { return };
    let app = handle.state::<App>();
    let Some(me) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { return };
    let ours = app.node_hex.lock().unwrap().clone().unwrap_or_default();
    let joined = app.voice_session.lock().unwrap().as_ref().filter(|s| s.server != CALL_SERVER).map(|s| s.joined_ms);
    if let Some(joined_ms) = joined {
        if takes_over(me, &ours, joined_ms, event) {
            hand_over(&app, handle).await;
            return;
        }
    }
    if *user != me {
        follow_device(&app, handle, server, *user, node, *voice);
    }
}

fn follow_device(app: &tauri::State<'_, App>, handle: &tauri::AppHandle, server: ServerId, user: UserId, node: &str, voice: Option<ChannelId>) {
    {
        let mut session = app.voice_session.lock().unwrap();
        let Some(session) = session.as_mut().filter(|s| s.server == server && Some(s.channel) == voice) else { return };
        if session.peer_node(user).is_some_and(|linked| !linked.eq_ignore_ascii_case(node)) {
            session.detach_peer(user);
        }
    }
    crate::react_to_voice_change(app, handle, server, user, voice);
}

async fn hand_over(app: &App, handle: &tauri::AppHandle) {
    let Some(server) = app.voice_session.lock().unwrap().as_ref().map(|s| s.server) else { return };
    let Some(me) = app.profile.lock().unwrap().clone() else { return };
    crate::stop_voice_here(app, server, &me).await;
    announce(app, server).await;
    let _ = handle.emit("voice-elsewhere", server.to_string());
    let _ = handle.emit("server-event", server.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn announce_from(user: UserId, node: &str, voice: Option<ChannelId>, since: Option<i64>) -> ServerEvent {
        ServerEvent::DeviceAnnounce { server: ServerId::new(), user, node: node.into(), voice, since }
    }

    #[test]
    fn a_later_join_on_another_of_our_devices_takes_voice_over() {
        let (me, room) = (UserId::new(), ChannelId::new());
        let (ours, tablet) = ("ab".repeat(32), "cd".repeat(32));
        assert!(takes_over(me, &ours, 1_000, &announce_from(me, &tablet, Some(room), Some(2_000))));
        assert!(!takes_over(me, &ours, 3_000, &announce_from(me, &tablet, Some(room), Some(2_000))), "we joined after it");
        assert!(!takes_over(me, &ours, 1_000, &announce_from(me, &tablet, None, None)), "it is not in voice");
        assert!(!takes_over(me, &ours, 1_000, &announce_from(me, &ours.to_uppercase(), Some(room), Some(2_000))), "that is us");
        assert!(!takes_over(me, &ours, 1_000, &announce_from(UserId::new(), &tablet, Some(room), Some(2_000))), "someone else");
    }

    #[test]
    fn our_profile_points_at_the_device_in_voice() {
        let owner = UserProfile::new("owner");
        let mut state = ServerState::new(commy_core::Server::new("s", owner.id));
        state.profiles.insert(owner.id, owner.clone());
        let sid = state.server.id;
        let room = state.server.channels.iter().find(|c| c.kind.is_voice()).unwrap().id;
        let tablet = "cd".repeat(32);
        let mut profile = UserProfile { node: Some("ab".repeat(32)), ..owner.clone() };
        point_at_voice(&state, &mut profile);
        assert_eq!(profile.node, Some("ab".repeat(32)), "nobody is in voice");
        state.apply(ServerEvent::VoiceState { user: owner.id, channel: Some(room), muted: false, deafened: false, since: None }).unwrap();
        state.apply(announce_from_server(sid, owner.id, &tablet, Some(room))).unwrap();
        assert!(has_sibling(&state, owner.id, &"ab".repeat(32)));
        assert!(!has_sibling(&state, owner.id, &tablet));
        point_at_voice(&state, &mut profile);
        assert_eq!(profile.node, Some(tablet));
    }

    fn announce_from_server(server: ServerId, user: UserId, node: &str, voice: Option<ChannelId>) -> ServerEvent {
        ServerEvent::DeviceAnnounce { server, user, node: node.into(), voice, since: Some(1) }
    }
}
