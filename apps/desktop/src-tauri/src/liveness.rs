use crate::catchup::AppAccess;
use crate::state::App;
use commy_core::ids::CALL_SERVER;
use commy_core::{ChannelId, Presence, ServerEvent, ServerId, ServerState, UserId};
use commy_net::WirePayload;
use std::time::{Duration, Instant};

pub use commy_view::liveness::{seat_news, speaker, Liveness, BEAT, EXPIRE, SERVICE_BEAT, SWEEP_EVERY};
#[cfg(test)]
use commy_view::liveness::SERVICE_EXPIRE;

const FAREWELL_WAIT: Duration = Duration::from_millis(1500);

const FAREWELL_LINGER: Duration = Duration::from_millis(300);

pub fn heard(app: &App, server: ServerId, user: UserId, at: Instant) {
    app.liveness.lock().unwrap().heard(server, user, at);
}

pub fn heard_through_service(app: &App, server: ServerId, user: UserId, now: Instant) {
    app.liveness.lock().unwrap().heard_through_service(server, user, now);
}

pub fn heard_device(app: &App, server: ServerId, user: UserId, node: &str, now: Instant) {
    app.liveness.lock().unwrap().heard_device(server, user, node, now);
}

pub fn heard_seat_news(app: &App, state: &ServerState, news: Option<(UserId, ChannelId)>, now: Instant) {
    app.liveness.lock().unwrap().heard_seat_news(state, news, now);
}

pub fn settle_device_presence(
    app: &App,
    state: &mut ServerState,
    user: UserId,
    sender: &[u8; 32],
    presence: Presence,
    now: Instant,
    said_at: Option<i64>,
) -> bool {
    app.liveness.lock().unwrap().settle_device_presence(state, user, sender, presence, now, said_at)
}

pub fn sweep(app: &App, me: UserId, now: Instant, wall_ms: i64, expire: Duration) -> Vec<ServerId> {
    let mut servers = app.servers.lock().unwrap();
    let mut live = app.liveness.lock().unwrap();
    live.sweep(servers.iter_mut(), me, now, wall_ms, expire)
}

fn hear_voice_peers(app: &App, now: Instant) {
    let voice = app.voice_session.lock().unwrap().as_ref().map(|s| {
        let peers: Vec<(UserId, Option<String>)> = s.live_peers().into_iter().map(|user| (user, s.peer_node(user))).collect();
        (s.server, peers)
    });
    let Some((server, peers)) = voice else { return };
    let mut live = app.liveness.lock().unwrap();
    for (user, node) in peers {
        live.heard(server, user, now);
        live.heard_seat(server, user, now);
        if let Some(node) = node {
            live.heard_device(server, user, &node, now);
        }
    }
}

pub fn rest(app: &App) {
    app.liveness.lock().unwrap().rest();
}

fn own_beat(app: &App) -> Option<(UserId, Presence)> {
    let profile = app.profile.lock().unwrap();
    let profile = profile.as_ref()?;
    (profile.presence != Presence::Invisible).then_some((profile.id, profile.presence))
}

fn server_ids(app: &App) -> Vec<ServerId> {
    app.servers.lock().unwrap().keys().copied().collect()
}

pub fn here(app: &App, server: ServerId) -> Vec<WirePayload> {
    let Some((me, presence)) = own_beat(app) else { return Vec::new() };
    let mut events: Vec<ServerEvent> = crate::devices::announcement(app, server).into_iter().collect();
    events.push(ServerEvent::PresenceUpdate { user: me, presence });
    events.extend(own_seat(app, me, server));
    events.into_iter().map(|event| app.wire_for(server, event)).collect()
}

fn own_seat(app: &App, me: UserId, server: ServerId) -> Option<ServerEvent> {
    let channel = app.voice_session.lock().unwrap().as_ref().filter(|s| s.server == server).map(|s| s.channel)?;
    let (muted, deafened) = {
        let voice = app.voice.lock().unwrap();
        (voice.muted, voice.deafened)
    };
    let since = app.servers.lock().unwrap().get(&server).and_then(|s| s.voice_since(me));
    Some(ServerEvent::VoiceState { user: me, channel: Some(channel), muted, deafened, since })
}

fn voice_server(app: &App) -> Option<ServerId> {
    app.voice_session.lock().unwrap().as_ref().map(|s| s.server).filter(|s| *s != CALL_SERVER)
}

fn me(app: &App) -> Option<UserId> {
    app.profile.lock().unwrap().as_ref().map(|p| p.id)
}

async fn keep_seat(app: &App) {
    let (Some(me), Some(server)) = (me(app), voice_server(app)) else { return };
    let news = crate::devices::announcement(app, server).into_iter().chain(own_seat(app, me, server));
    for event in news.collect::<Vec<_>>() {
        crate::broadcast_event(app, server, event).await;
    }
}

async fn leave_seat(app: &App) {
    let (Some(me), Some(server)) = (me(app), voice_server(app)) else { return };
    let Some(node) = app.node_hex.lock().unwrap().clone() else { return };
    let device = ServerEvent::DeviceAnnounce { server, user: me, node, voice: None, since: None };
    let seat = ServerEvent::VoiceState { user: me, channel: None, muted: false, deafened: false, since: None };
    for event in [device, seat] {
        crate::broadcast_event(app, server, event).await;
    }
}

async fn send_all(app: &App, user: UserId, presence: Presence, through_service: bool) {
    let payloads: Vec<(ServerId, WirePayload)> = server_ids(app)
        .into_iter()
        .map(|sid| (sid, app.wire_for(sid, ServerEvent::PresenceUpdate { user, presence })))
        .collect();
    if through_service {
        for (sid, payload) in &payloads {
            crate::community_sync::signal(*sid, payload);
        }
    }
    if !through_service && presence.is_visible_online() {
        for (sid, _) in &payloads {
            crate::community_sync::leave_word(app, *sid);
        }
    }
    let net = app.net.lock().await;
    let Some(node) = net.as_ref() else { return };
    for (sid, payload) in payloads {
        if node.neighbor_count(sid) == 0 {
            continue;
        }
        if let Err(e) = node.broadcast(sid, payload).await {
            tracing::debug!("presence beat did not go out: {e}");
        }
    }
}

pub async fn beat(app: &App, through_service: bool) {
    if let Some((me, presence)) = own_beat(app) {
        send_all(app, me, presence, through_service).await;
    }
    keep_seat(app).await;
}

pub async fn farewell(app: &App) {
    leave_seat(app).await;
    if let Some((me, _)) = own_beat(app) {
        send_all(app, me, Presence::Offline, true).await;
    }
    tokio::time::sleep(FAREWELL_LINGER).await;
}

pub fn say_goodbye(app: &App) {
    tauri::async_runtime::block_on(async {
        let _ = tokio::time::timeout(FAREWELL_WAIT, farewell(app)).await;
    });
}

pub async fn run(access: impl AppAccess) {
    let mut beat_due = Instant::now() + BEAT;
    let mut service_due = Instant::now() + SERVICE_BEAT;
    loop {
        tokio::time::sleep(SWEEP_EVERY).await;
        let app = access.app();
        if crate::wake::is_resting(app) {
            rest(app);
            continue;
        }
        if Instant::now() >= beat_due {
            beat_due = Instant::now() + BEAT;
            let through_service = Instant::now() >= service_due;
            if through_service {
                service_due = Instant::now() + SERVICE_BEAT;
            }
            beat(app, through_service).await;
        }
        let Some(me) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { continue };
        hear_voice_peers(app, Instant::now());
        for sid in sweep(app, me, Instant::now(), chrono::Utc::now().timestamp_millis(), EXPIRE) {
            tracing::info!("a member went quiet and is shown offline");
            access.emit("server-event", serde_json::json!(sid.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Server, UserProfile};

    fn app_with(members: &[&UserProfile]) -> (App, ServerId) {
        let app = App::default();
        let owner = members[0];
        let server = Server::new("Moonbase", owner.id);
        let sid = server.id;
        let mut state = ServerState::new(server);
        for profile in members {
            state.apply(ServerEvent::MemberJoin { server: sid, profile: (*profile).clone() }).unwrap();
        }
        app.servers.lock().unwrap().insert(sid, state);
        (app, sid)
    }

    fn presence(app: &App, sid: ServerId, user: UserId) -> Presence {
        app.servers.lock().unwrap()[&sid].profiles[&user].presence
    }

    #[test]
    fn a_member_nobody_hears_from_is_shown_offline() {
        let me = UserProfile::new("me");
        let quiet = UserProfile::new("quiet");
        let chatty = UserProfile::new("chatty");
        let (app, sid) = app_with(&[&me, &quiet, &chatty]);
        let start = Instant::now();
        assert!(sweep(&app, me.id, start, 0, EXPIRE).is_empty());
        let later = start + EXPIRE + Duration::from_secs(1);
        heard(&app, sid, chatty.id, later);
        assert_eq!(sweep(&app, me.id, later, 15_000, EXPIRE), vec![sid]);
        assert_eq!(presence(&app, sid, quiet.id), Presence::Offline);
        assert_eq!(presence(&app, sid, chatty.id), Presence::Online);
        assert_eq!(presence(&app, sid, me.id), Presence::Online, "we never time ourselves out");
    }

    #[test]
    fn a_member_who_speaks_again_comes_back() {
        let me = UserProfile::new("me");
        let friend = UserProfile::new("friend");
        let (app, sid) = app_with(&[&me, &friend]);
        let start = Instant::now();
        sweep(&app, me.id, start, 0, EXPIRE);
        let later = start + EXPIRE + Duration::from_secs(1);
        sweep(&app, me.id, later, 15_000, EXPIRE);
        assert_eq!(presence(&app, sid, friend.id), Presence::Offline);
        let back = ServerEvent::PresenceUpdate { user: friend.id, presence: Presence::Idle };
        assert_eq!(speaker(&back, None), Some(friend.id));
        app.servers.lock().unwrap().get_mut(&sid).unwrap().apply(back).unwrap();
        heard(&app, sid, friend.id, later);
        assert!(sweep(&app, me.id, later + SWEEP_EVERY, 30_000, EXPIRE).is_empty());
        assert_eq!(presence(&app, sid, friend.id), Presence::Idle);
    }

    #[test]
    fn a_member_heard_through_the_service_stays_online_between_its_slower_beats() {
        let me = UserProfile::new("me");
        let friend = UserProfile::new("friend");
        let (app, sid) = app_with(&[&me, &friend]);
        let start = Instant::now();
        sweep(&app, me.id, start, 0, EXPIRE);
        heard_through_service(&app, sid, friend.id, start);
        assert!(sweep(&app, me.id, start + SERVICE_BEAT, 15_000, EXPIRE).is_empty());
        assert_eq!(presence(&app, sid, friend.id), Presence::Online);
        assert_eq!(sweep(&app, me.id, start + SERVICE_EXPIRE + SWEEP_EVERY, 30_000, EXPIRE), vec![sid]);
        assert_eq!(presence(&app, sid, friend.id), Presence::Offline);
    }

    #[test]
    fn saying_we_are_here_sends_presence_and_nothing_while_invisible() {
        let mut me = UserProfile::new("me");
        let (app, sid) = app_with(&[&me]);
        *app.profile.lock().unwrap() = Some(me.clone());
        let said: Vec<_> = here(&app, sid).iter().filter_map(|p| commy_net::community_streams::event_of(p).map(|(e, _)| e.clone())).collect();
        assert_eq!(said, vec![ServerEvent::PresenceUpdate { user: me.id, presence: Presence::Online }]);
        me.presence = Presence::Invisible;
        *app.profile.lock().unwrap() = Some(me);
        assert!(here(&app, sid).is_empty());
    }

    #[test]
    fn waking_from_sleep_gives_everyone_a_fresh_window() {
        let me = UserProfile::new("me");
        let friend = UserProfile::new("friend");
        let (app, sid) = app_with(&[&me, &friend]);
        let start = Instant::now();
        sweep(&app, me.id, start, 0, EXPIRE);
        let after_sleep = start + Duration::from_secs(3600);
        assert!(sweep(&app, me.id, after_sleep, 3_600_000, EXPIRE).is_empty());
        assert_eq!(presence(&app, sid, friend.id), Presence::Online);
    }

    #[test]
    fn only_live_words_from_the_member_count() {
        let friend = UserProfile::new("friend");
        let other = UserProfile::new("other");
        let sid = ServerId::new();
        let typing = ServerEvent::TypingStart { channel: commy_core::ChannelId::new(), user: friend.id };
        assert_eq!(speaker(&typing, None), Some(friend.id));
        let by = |user: UserId| commy_core::Action { by: user, at: 0, sig: commy_core::Signature([0; 64]) };
        let beat = ServerEvent::PresenceUpdate { user: friend.id, presence: Presence::Online };
        assert_eq!(speaker(&beat, Some(&by(friend.id))), Some(friend.id));
        assert_eq!(speaker(&beat, Some(&by(other.id))), None);
        let relayed = ServerEvent::RulesAccept { server: sid, user: friend.id };
        assert_eq!(speaker(&relayed, Some(&by(friend.id))), None);
    }

    #[test]
    fn saying_goodbye_works_from_the_exit_handler_outside_the_runtime() {
        let me = UserProfile::new("me");
        let (app, _) = app_with(&[&me]);
        *app.profile.lock().unwrap() = Some(me);
        let started = Instant::now();
        std::thread::spawn(move || say_goodbye(&app)).join().expect("no panic outside the runtime");
        assert!(started.elapsed() >= FAREWELL_LINGER);
    }
    #[test]
    fn one_device_leaving_keeps_a_person_online_while_another_is_heard() {
        let me = UserProfile::new("me");
        let friend = UserProfile::new("friend");
        let (app, sid) = app_with(&[&me, &friend]);
        let (desktop, tablet, stranger) = ([1u8; 32], [2u8; 32], [3u8; 32]);
        {
            let mut servers = app.servers.lock().unwrap();
            let state = servers.get_mut(&sid).unwrap();
            for node in [desktop, tablet] {
                let node = data_encoding::HEXLOWER.encode(&node);
                state.apply(ServerEvent::DeviceAnnounce { server: sid, user: friend.id, node, voice: None, since: None }).unwrap();
            }
        }
        let now = Instant::now();
        let says = |node: &[u8; 32], presence: Presence, at: Instant| {
            let mut servers = app.servers.lock().unwrap();
            let state = servers.get_mut(&sid).unwrap();
            state.apply(ServerEvent::PresenceUpdate { user: friend.id, presence }).unwrap();
            settle_device_presence(&app, state, friend.id, node, presence, at, None);
        };
        says(&desktop, Presence::Online, now);
        says(&tablet, Presence::Idle, now);
        assert_eq!(presence(&app, sid, friend.id), Presence::Online, "the desktop is in use");
        says(&tablet, Presence::Offline, now);
        assert_eq!(presence(&app, sid, friend.id), Presence::Online, "the tablet quitting leaves the desktop");
        says(&stranger, Presence::Offline, now);
        assert_eq!(presence(&app, sid, friend.id), Presence::Offline, "an unknown device is not one of theirs");
        says(&desktop, Presence::Online, now);
        says(&tablet, Presence::Offline, now + SERVICE_EXPIRE + Duration::from_secs(1));
        assert_eq!(presence(&app, sid, friend.id), Presence::Offline, "the desktop has gone quiet too");
    }
}
