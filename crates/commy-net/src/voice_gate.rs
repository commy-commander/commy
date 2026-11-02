use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use commy_sync::StreamId;

use crate::connection_check::RelayLink;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateLink {
    NotUsed,
    Behind,
    Lost,
    Current,
}

pub fn link_of(streams: &[StreamId], current: &HashSet<StreamId>, lost: &HashSet<StreamId>) -> StateLink {
    if streams.is_empty() {
        StateLink::NotUsed
    } else if streams.iter().any(|stream| current.contains(stream)) {
        StateLink::Current
    } else if streams.iter().any(|stream| lost.contains(stream)) {
        StateLink::Lost
    } else {
        StateLink::Behind
    }
}

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
    fn a_server_is_current_only_while_its_state_stream_is_caught_up_on_a_live_link() {
        let (state, other) = (StreamId([1; 32]), StreamId([2; 32]));
        let none = HashSet::new();
        assert_eq!(link_of(&[], &none, &none), StateLink::NotUsed);
        assert_eq!(link_of(&[state], &none, &none), StateLink::Behind);
        assert_eq!(link_of(&[state], &HashSet::from([state]), &none), StateLink::Current);
        assert_eq!(link_of(&[state], &none, &HashSet::from([state])), StateLink::Lost);
        assert_eq!(link_of(&[state], &HashSet::from([other]), &HashSet::from([other])), StateLink::Behind);
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
