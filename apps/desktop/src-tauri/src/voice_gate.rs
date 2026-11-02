use crate::community_sync::StateLink;
use crate::state::App;
use commy_core::ServerId;
use commy_net::RelayLink;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

pub const CHECK_EVERY: Duration = Duration::from_secs(1);
pub const PEER_WAIT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Wait {
    Starting,
    NoRelay,
    Loading,
    SyncLost,
    CatchingUp,
    NoPeers,
}

impl Wait {
    pub fn reason(self) -> &'static str {
        match self {
            Wait::Starting => "Commy is still starting its connection",
            Wait::NoRelay => "Commy can't reach its relay yet",
            Wait::Loading => "This server is still loading",
            Wait::SyncLost => "Reconnecting to the server",
            Wait::CatchingUp => "Getting the latest from the server",
            Wait::NoPeers => "Waiting for other members to answer",
        }
    }

    pub fn refusal(self) -> String {
        format!("Voice opens once you are connected. {}.", self.reason())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signals {
    pub network_up: bool,
    pub relay: RelayLink,
    pub loading: bool,
    pub state: StateLink,
    pub peers: usize,
    pub others: bool,
    pub online_for: Duration,
}

pub fn wait_for(signals: &Signals) -> Option<Wait> {
    if !signals.network_up {
        return Some(Wait::Starting);
    }
    if signals.relay == RelayLink::Unreachable {
        return Some(Wait::NoRelay);
    }
    if signals.loading {
        return Some(Wait::Loading);
    }
    match signals.state {
        StateLink::Current => None,
        StateLink::Lost => Some(Wait::SyncLost),
        StateLink::Behind => Some(Wait::CatchingUp),
        StateLink::NotUsed => {
            let nobody_answered = signals.others && signals.peers == 0 && signals.online_for < PEER_WAIT;
            nobody_answered.then_some(Wait::NoPeers)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WaitView {
    pub kind: Wait,
    pub reason: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct GateView {
    pub enabled: bool,
    pub waits: BTreeMap<String, WaitView>,
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> Signals {
        Signals {
            network_up: true,
            relay: RelayLink::Connected,
            loading: false,
            state: StateLink::Current,
            peers: 2,
            others: true,
            online_for: Duration::from_secs(60),
        }
    }

    #[test]
    fn a_connected_and_current_server_lets_you_join() {
        assert_eq!(wait_for(&ready()), None);
        assert_eq!(wait_for(&Signals { relay: RelayLink::NotUsed, ..ready() }), None);
    }

    #[test]
    fn joining_waits_while_the_network_starts_or_the_relay_is_out_of_reach() {
        assert_eq!(wait_for(&Signals { network_up: false, ..ready() }), Some(Wait::Starting));
        assert_eq!(wait_for(&Signals { relay: RelayLink::Unreachable, ..ready() }), Some(Wait::NoRelay));
    }

    #[test]
    fn joining_waits_until_the_server_state_is_current() {
        assert_eq!(wait_for(&Signals { loading: true, ..ready() }), Some(Wait::Loading));
        assert_eq!(wait_for(&Signals { state: StateLink::Behind, ..ready() }), Some(Wait::CatchingUp));
        assert_eq!(wait_for(&Signals { state: StateLink::Lost, ..ready() }), Some(Wait::SyncLost));
    }

    #[test]
    fn the_sync_service_being_current_is_enough_without_peers() {
        assert_eq!(wait_for(&Signals { peers: 0, online_for: Duration::ZERO, ..ready() }), None);
    }

    #[test]
    fn without_the_sync_service_a_short_wait_gives_peers_a_chance_to_answer() {
        let alone = Signals { state: StateLink::NotUsed, peers: 0, online_for: Duration::from_secs(3), ..ready() };
        assert_eq!(wait_for(&alone), Some(Wait::NoPeers));
        assert_eq!(wait_for(&Signals { peers: 1, ..alone }), None);
        assert_eq!(wait_for(&Signals { others: false, ..alone }), None);
        assert_eq!(wait_for(&Signals { online_for: PEER_WAIT, ..alone }), None);
    }

    #[test]
    fn every_wait_explains_itself() {
        for wait in [Wait::Starting, Wait::NoRelay, Wait::Loading, Wait::SyncLost, Wait::CatchingUp, Wait::NoPeers] {
            assert!(!wait.reason().is_empty());
            assert!(wait.refusal().starts_with("Voice opens once you are connected. "));
        }
    }

    #[test]
    fn waits_reach_the_app_in_plain_words() {
        let view = WaitView { kind: Wait::CatchingUp, reason: Wait::CatchingUp.reason() };
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["kind"], "catching_up");
        assert_eq!(json["reason"], "Getting the latest from the server");
    }
}
