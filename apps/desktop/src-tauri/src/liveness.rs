use crate::catchup::AppAccess;
use crate::state::App;
use commy_core::{action, devices, Action, Presence, ServerEvent, ServerId, ServerState, UserId};
use commy_net::WirePayload;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

pub const BEAT: Duration = Duration::from_secs(45);

pub const EXPIRE: Duration = Duration::from_secs(120);

const SWEEP_EVERY: Duration = Duration::from_secs(15);

const FAREWELL_WAIT: Duration = Duration::from_millis(1500);

const FAREWELL_LINGER: Duration = Duration::from_millis(300);

type DeviceSaid = HashMap<String, (Presence, Instant)>;

#[derive(Debug, Default)]
pub struct Liveness {
    heard: HashMap<(ServerId, UserId), Instant>,
    devices: HashMap<(ServerId, UserId), DeviceSaid>,
    last_sweep_ms: Option<i64>,
}

pub fn speaker(event: &ServerEvent, signed_by: Option<&Action>) -> Option<UserId> {
    if action::relayable(event) {
        return None;
    }
    let actor = action::actor(event)?;
    signed_by.is_none_or(|a| a.by == actor).then_some(actor)
}

pub fn heard(app: &App, server: ServerId, user: UserId, at: Instant) {
    app.liveness.lock().unwrap().heard.insert((server, user), at);
}

pub fn settle_device_presence(app: &App, state: &mut ServerState, user: UserId, sender: &[u8; 32], presence: Presence, now: Instant) -> bool {
    let node = data_encoding::HEXLOWER.encode(sender);
    if !state.devices_of(user).iter().any(|d| d.node == node) {
        return false;
    }
    let mut live = app.liveness.lock().unwrap();
    let said = live.devices.entry((state.server.id, user)).or_default();
    said.insert(node, (presence, now));
    said.retain(|_, (_, at)| now.duration_since(*at) <= EXPIRE);
    if said.len() < 2 {
        return false;
    }
    let shown = devices::combined_presence(said.values().map(|(p, _)| *p));
    match state.profiles.get_mut(&user) {
        Some(profile) if profile.presence != shown => {
            profile.presence = shown;
            true
        }
        _ => false,
    }
}

pub fn sweep(app: &App, me: UserId, now: Instant, wall_ms: i64, expire: Duration) -> Vec<ServerId> {
    let mut servers = app.servers.lock().unwrap();
    let mut live = app.liveness.lock().unwrap();
    let asleep = live.last_sweep_ms.is_some_and(|last| wall_ms.saturating_sub(last) > expire.as_millis() as i64);
    live.last_sweep_ms = Some(wall_ms);
    if asleep {
        live.heard.clear();
    }
    let mut changed = Vec::new();
    let mut online = HashSet::new();
    for (sid, state) in servers.iter_mut() {
        let shown: Vec<UserId> = state
            .profiles
            .values()
            .filter(|p| p.id != me && p.presence.is_visible_online())
            .map(|p| p.id)
            .collect();
        let mut gone = false;
        for user in shown {
            let last = *live.heard.entry((*sid, user)).or_insert(now);
            if now.duration_since(last) > expire {
                gone |= state.apply(ServerEvent::PresenceUpdate { user, presence: Presence::Offline }) == Ok(true);
            } else {
                online.insert((*sid, user));
            }
        }
        if gone {
            changed.push(*sid);
        }
    }
    live.heard.retain(|key, _| online.contains(key));
    live.devices.retain(|key, _| online.contains(key));
    changed
}

fn hear_voice_peers(app: &App, now: Instant) {
    let voice = app.voice_session.lock().unwrap().as_ref().map(|s| (s.server, s.live_peers()));
    let Some((server, peers)) = voice else { return };
    for user in peers {
        heard(app, server, user, now);
    }
}

pub fn rest(app: &App) {
    let mut live = app.liveness.lock().unwrap();
    live.heard.clear();
    live.devices.clear();
}

fn own_beat(app: &App) -> Option<(UserId, Presence)> {
    let profile = app.profile.lock().unwrap();
    let profile = profile.as_ref()?;
    (profile.presence != Presence::Invisible).then_some((profile.id, profile.presence))
}

fn server_ids(app: &App) -> Vec<ServerId> {
    app.servers.lock().unwrap().keys().copied().collect()
}

async fn send_all(app: &App, user: UserId, presence: Presence) {
    let payloads: Vec<(ServerId, WirePayload)> = server_ids(app)
        .into_iter()
        .map(|sid| (sid, app.wire_for(sid, ServerEvent::PresenceUpdate { user, presence })))
        .collect();
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

pub async fn beat(app: &App) {
    if let Some((me, presence)) = own_beat(app) {
        send_all(app, me, presence).await;
    }
}

pub async fn farewell(app: &App) {
    let Some((me, _)) = own_beat(app) else { return };
    send_all(app, me, Presence::Offline).await;
    tokio::time::sleep(FAREWELL_LINGER).await;
}

pub fn say_goodbye(app: &App) {
    tauri::async_runtime::block_on(async {
        let _ = tokio::time::timeout(FAREWELL_WAIT, farewell(app)).await;
    });
}

pub async fn run(access: impl AppAccess) {
    let mut beat_due = Instant::now() + BEAT;
    loop {
        tokio::time::sleep(SWEEP_EVERY).await;
        let app = access.app();
        if crate::wake::is_resting(app) {
            rest(app);
            continue;
        }
        if Instant::now() >= beat_due {
            beat_due = Instant::now() + BEAT;
            beat(app).await;
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
        let by = |user: UserId| Action { by: user, at: 0, sig: commy_core::Signature([0; 64]) };
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
            settle_device_presence(&app, state, friend.id, node, presence, at);
        };
        says(&desktop, Presence::Online, now);
        says(&tablet, Presence::Idle, now);
        assert_eq!(presence(&app, sid, friend.id), Presence::Online, "the desktop is in use");
        says(&tablet, Presence::Offline, now);
        assert_eq!(presence(&app, sid, friend.id), Presence::Online, "the tablet quitting leaves the desktop");
        says(&stranger, Presence::Offline, now);
        assert_eq!(presence(&app, sid, friend.id), Presence::Offline, "an unknown device is not one of theirs");
        says(&desktop, Presence::Online, now);
        says(&tablet, Presence::Offline, now + EXPIRE + Duration::from_secs(1));
        assert_eq!(presence(&app, sid, friend.id), Presence::Offline, "the desktop has gone quiet too");
    }
}
