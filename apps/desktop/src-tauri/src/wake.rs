use crate::catchup::AppAccess;
use crate::state::App;
use commy_core::{Presence, ServerId, UserId};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

const SEND_SPACING: Duration = Duration::from_secs(30);

const MAX_WAKES: usize = 64;

const HANDLE_SPACING: Duration = Duration::from_secs(10);

const AWAKE_AT_LEAST: Duration = Duration::from_secs(60);

#[derive(Debug, Default)]
pub struct Rest {
    resting: bool,
    awake_until: Option<Instant>,
    woken_at: HashMap<String, Instant>,
    handled_at: HashMap<ServerId, Instant>,
}

pub(crate) fn is_resting(app: &App) -> bool {
    app.rest.lock().unwrap().resting
}

fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

fn may_rest(presence: Presence) -> bool {
    !matches!(presence, Presence::Online | Presence::DoNotDisturb)
}

pub(crate) fn targets(app: &App, server: ServerId) -> Vec<String> {
    let our_node = app.node_hex.lock().unwrap().clone();
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let servers = app.servers.lock().unwrap();
    let Some(state) = servers.get(&server) else { return Vec::new() };
    let resting: HashSet<UserId> = state
        .profiles
        .values()
        .filter(|p| Some(p.id) != me && may_rest(p.presence) && crate::doorbell::member_ok(state, p.id))
        .map(|p| p.id)
        .collect();
    let mut nodes: Vec<String> = state
        .profiles
        .values()
        .filter(|p| resting.contains(&p.id))
        .filter_map(|p| p.node.clone())
        .chain(
            app.doorbell
                .lock()
                .unwrap()
                .devices(server)
                .into_iter()
                .filter(|(_, user)| resting.contains(user))
                .map(|(node, _)| node),
        )
        .map(|n| n.to_ascii_lowercase())
        .filter(|n| Some(n.as_str()) != our_node.as_deref())
        .collect();
    nodes.sort();
    nodes.dedup();
    nodes.truncate(MAX_WAKES);
    nodes
}

pub(crate) async fn wake_members(app: &App, server: ServerId) -> Vec<(String, bool)> {
    let wakes = {
        let net = app.net.lock().await;
        let Some(node) = net.as_ref() else { return Vec::new() };
        let now = Instant::now();
        let due: Vec<String> = {
            let mut rest = app.rest.lock().unwrap();
            rest.woken_at.retain(|_, at| now.duration_since(*at) < SEND_SPACING);
            targets(app, server)
                .into_iter()
                .filter(|n| rest.woken_at.insert(n.clone(), now).is_none())
                .collect()
        };
        due.into_iter().map(|n| (n.clone(), tokio::spawn(node.wake(&n)))).collect::<Vec<_>>()
    };
    let mut out = Vec::new();
    for (node, wake) in wakes {
        let landed = match wake.await {
            Ok(Ok(())) => true,
            Ok(Err(e)) => {
                tracing::debug!("a member's device did not take a wake: {e:#}");
                false
            }
            Err(e) => {
                tracing::debug!("a wake did not finish: {e}");
                false
            }
        };
        out.push((node, landed));
    }
    out
}

pub fn after_send(handle: &tauri::AppHandle, server: ServerId, message: commy_core::MessageId) {
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        let missed: Vec<String> = {
            let app = handle.app();
            wake_members(app, server).await.into_iter().filter(|(_, ok)| !ok).map(|(n, _)| n).collect()
        };
        for node in missed {
            crate::doorbell::ring_device(&handle, server, &node, message);
        }
    });
}

pub(crate) fn on_wake(app: &App, from: [u8; 32]) -> Vec<ServerId> {
    if !is_resting(app) {
        return Vec::new();
    }
    let from = hex(&from);
    let mut shared: Vec<ServerId> = {
        let servers = app.servers.lock().unwrap();
        let doorbell = app.doorbell.lock().unwrap();
        servers
            .values()
            .filter(|state| {
                let id = state.server.id;
                let by_profile = state
                    .profiles
                    .values()
                    .any(|p| p.node.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(&from)) && crate::doorbell::member_ok(state, p.id));
                let by_doorbell = doorbell
                    .devices(id)
                    .iter()
                    .any(|(n, user)| n.eq_ignore_ascii_case(&from) && crate::doorbell::member_ok(state, *user));
                by_profile || by_doorbell
            })
            .map(|state| state.server.id)
            .collect()
    };
    if shared.is_empty() {
        tracing::debug!("dropped a wake from a device we do not know");
        return shared;
    }
    let now = Instant::now();
    let mut rest = app.rest.lock().unwrap();
    shared.retain(|s| {
        let due = rest.handled_at.get(s).is_none_or(|at| now.duration_since(*at) >= HANDLE_SPACING);
        if due {
            rest.handled_at.insert(*s, now);
        }
        due
    });
    if !shared.is_empty() {
        let stay = rest_after().map_or(AWAKE_AT_LEAST, |d| d.max(AWAKE_AT_LEAST));
        rest.awake_until = Some(now + stay);
    }
    shared
}

fn members_by_server(app: &App) -> Vec<(ServerId, Vec<String>)> {
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let servers = app.servers.lock().unwrap();
    let doorbell = app.doorbell.lock().unwrap();
    servers
        .values()
        .map(|s| {
            let mut nodes: Vec<String> = s
                .profiles
                .values()
                .filter(|p| Some(p.id) != me)
                .filter_map(|p| p.node.clone())
                .chain(doorbell.devices(s.server.id).into_iter().map(|(n, _)| n))
                .collect();
            nodes.sort();
            nodes.dedup();
            (s.server.id, nodes)
        })
        .collect()
}

pub(crate) async fn rest(app: &App) -> usize {
    let mut net = app.net.lock().await;
    let Some(node) = net.as_mut() else { return 0 };
    let left = node.rest();
    app.rest.lock().unwrap().resting = true;
    tracing::info!("resting: left the mesh of {} servers, still reachable for wake-ups", left.len());
    left.len()
}

pub(crate) async fn unrest(app: &App) -> usize {
    if !app.rest.lock().unwrap().resting {
        return 0;
    }
    let targets = members_by_server(app);
    let mut net = app.net.lock().await;
    let Some(node) = net.as_mut() else { return 0 };
    let mut joined = 0;
    for (server, peers) in targets {
        match node.rejoin(server, &peers).await {
            Ok(()) => joined += 1,
            Err(e) => tracing::warn!("could not rejoin a server after resting: {e:#}"),
        }
    }
    app.rest.lock().unwrap().resting = false;
    tracing::info!("awake: rejoined the mesh of {joined} servers");
    joined
}

pub fn handle_wake(handle: &tauri::AppHandle, from: [u8; 32]) {
    let servers = on_wake(handle.app(), from);
    if servers.is_empty() {
        return;
    }
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        if unrest(handle.app()).await > 0 {
            show_resting(&handle, false);
        }
        for server in servers {
            crate::catchup::spawn_session(handle.clone(), server, from, None, true);
        }
    });
}

fn rest_after() -> Option<Duration> {
    std::env::var("COMMY_REST_AFTER_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|s| *s > 0)
        .map(Duration::from_secs)
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Rest,
    Wake,
    Stay,
}

fn decide(resting: bool, away: bool, busy: bool, idle: Duration, after: Duration, held_awake: bool) -> Step {
    if resting {
        if !away || busy {
            return Step::Wake;
        }
    } else if away && !busy && idle >= after && !held_awake {
        return Step::Rest;
    }
    Step::Stay
}

fn show_resting(handle: &tauri::AppHandle, resting: bool) {
    use tauri::Manager;
    let label = std::env::var("COMMY_INSTANCE_LABEL").ok();
    let base = crate::instance::window_title(option_env!("COMMY_TEST_BUILD").is_some(), label.as_deref())
        .unwrap_or_else(|| "Commy".to_string());
    let title = if resting { format!("{base} · resting") } else { base };
    if let Some(window) = handle.get_webview_window("main") {
        let _ = window.set_title(&title);
    }
}

pub async fn run_watchdog(handle: tauri::AppHandle) {
    let Some(after) = rest_after() else { return };
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let app = handle.app();
        let away = app.auto_idle.load(std::sync::atomic::Ordering::Relaxed);
        let busy = app.voice_session.lock().unwrap().is_some() || app.screen_share.lock().unwrap().is_some();
        let last = app.last_user_activity.load(std::sync::atomic::Ordering::Relaxed);
        let idle = Duration::from_secs((chrono::Utc::now().timestamp() - last).max(0) as u64);
        let (resting, held_awake) = {
            let rest = app.rest.lock().unwrap();
            (rest.resting, rest.awake_until.is_some_and(|t| Instant::now() < t))
        };
        match decide(resting, away, busy, idle, after, held_awake) {
            Step::Rest => {
                if rest(app).await > 0 {
                    show_resting(&handle, true);
                }
            }
            Step::Wake => {
                unrest(app).await;
                show_resting(&handle, false);
            }
            Step::Stay => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AFTER: Duration = Duration::from_secs(30);

    #[test]
    fn rests_only_when_away_long_enough_and_not_busy() {
        let long = Duration::from_secs(31);
        assert_eq!(decide(false, true, false, long, AFTER, false), Step::Rest);
        assert_eq!(decide(false, false, false, long, AFTER, false), Step::Stay, "not away");
        assert_eq!(decide(false, true, true, long, AFTER, false), Step::Stay, "in voice");
        assert_eq!(decide(false, true, false, Duration::from_secs(10), AFTER, false), Step::Stay);
        assert_eq!(decide(false, true, false, long, AFTER, true), Step::Stay, "held awake by a wake");
    }

    #[test]
    fn wakes_on_return_or_voice() {
        let long = Duration::from_secs(300);
        assert_eq!(decide(true, false, false, Duration::ZERO, AFTER, false), Step::Wake);
        assert_eq!(decide(true, true, true, long, AFTER, false), Step::Wake);
        assert_eq!(decide(true, true, false, long, AFTER, false), Step::Stay);
    }

    #[test]
    fn only_members_who_may_rest_are_woken() {
        assert!(may_rest(Presence::Idle));
        assert!(may_rest(Presence::Invisible));
        assert!(may_rest(Presence::Offline));
        assert!(!may_rest(Presence::Online));
        assert!(!may_rest(Presence::DoNotDisturb));
    }
}
