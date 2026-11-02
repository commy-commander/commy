use crate::state::App;
use commy_core::ServerId;
use commy_net::voice_gate::{wait_for, GateView, Signals, StateLink, Wait, WaitView};
use commy_net::RelayLink;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

pub const CHECK_EVERY: Duration = Duration::from_secs(1);

static ONLINE_SINCE: Mutex<Option<Instant>> = Mutex::new(None);
static LAST: Mutex<Option<GateView>> = Mutex::new(None);

struct NetSignals {
    up: bool,
    relay: RelayLink,
    peers: BTreeMap<ServerId, usize>,
}

async fn net_signals(app: &App, servers: &[ServerId]) -> NetSignals {
    let net = app.net.lock().await;
    let Some(node) = net.as_ref() else {
        return NetSignals { up: false, relay: RelayLink::NotUsed, peers: BTreeMap::new() };
    };
    let peers = servers.iter().map(|sid| (*sid, node.neighbor_count(*sid))).collect();
    NetSignals { up: true, relay: node.connection_checker().relay(), peers }
}

fn online_for(up: bool) -> Duration {
    let mut since = ONLINE_SINCE.lock().unwrap();
    if !up {
        *since = None;
        return Duration::ZERO;
    }
    since.get_or_insert_with(Instant::now).elapsed()
}

fn state_link(app: &App, server: ServerId) -> StateLink {
    match crate::community_sync::state_link(server) {
        Some(link) => link,
        None if crate::sync_link::url().is_some() && app.key_settled() && app.identity().is_some() => StateLink::Behind,
        None => StateLink::NotUsed,
    }
}

async fn waits(app: &App, servers: &[ServerId]) -> BTreeMap<ServerId, Wait> {
    let net = net_signals(app, servers).await;
    let online_for = online_for(net.up);
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let others: BTreeMap<ServerId, bool> = {
        let states = app.servers.lock().unwrap();
        servers
            .iter()
            .map(|sid| {
                let others = states.get(sid).is_some_and(|state| {
                    state.server.members.iter().any(|m| Some(m.user) != me && state.server.is_current_member(m.user))
                });
                (*sid, others)
            })
            .collect()
    };
    servers
        .iter()
        .filter_map(|sid| {
            let signals = Signals {
                network_up: net.up,
                relay: net.relay,
                loading: app.awaits_snapshot(*sid),
                state: state_link(app, *sid),
                peers: net.peers.get(sid).copied().unwrap_or(0),
                others: others.get(sid).copied().unwrap_or(false),
                online_for,
            };
            wait_for(&signals).map(|wait| (*sid, wait))
        })
        .collect()
}

pub async fn check(app: &App, server: ServerId) -> Option<Wait> {
    if !app.voice.lock().unwrap().wait_for_connection {
        return None;
    }
    waits(app, &[server]).await.remove(&server)
}

async fn view(app: &App) -> GateView {
    let servers: Vec<ServerId> = app.servers.lock().unwrap().keys().copied().collect();
    let enabled = app.voice.lock().unwrap().wait_for_connection;
    let waits = waits(app, &servers)
        .await
        .into_iter()
        .map(|(sid, kind)| (sid.to_string(), WaitView { kind, reason: kind.reason() }))
        .collect();
    GateView { enabled, waits }
}

pub async fn current(app: &App) -> GateView {
    let fresh = view(app).await;
    *LAST.lock().unwrap() = Some(fresh.clone());
    fresh
}

pub async fn run(handle: AppHandle) {
    loop {
        let fresh = view(&handle.state::<App>()).await;
        let changed = LAST.lock().unwrap().replace(fresh.clone()).as_ref() != Some(&fresh);
        if changed {
            let _ = handle.emit("voice-gate", &fresh);
        }
        tokio::time::sleep(CHECK_EVERY).await;
    }
}
