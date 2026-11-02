use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use commy_core::ServerId;
use commy_net::voice_gate::{wait_for, GateView, Signals, StateLink, Wait, WaitView};
use commy_net::RelayLink;
use serde_json::json;
use wasm_bindgen_futures::spawn_local;

use super::Session;

const CHECK_EVERY: Duration = Duration::from_secs(1);

struct NetSignals {
    up: bool,
    relay: RelayLink,
    peers: BTreeMap<ServerId, usize>,
}

impl Session {
    pub(super) fn start_voice_gate(self: &Rc<Self>) {
        let already = std::mem::replace(&mut self.voice.borrow_mut().gate_tending, true);
        if !already {
            spawn_local(self.clone().tend_voice_gate());
        }
    }

    async fn tend_voice_gate(self: Rc<Self>) {
        let mut last: Option<GateView> = None;
        loop {
            let fresh = self.voice_gate_view().await;
            if last.as_ref() != Some(&fresh) {
                self.emit("voice-gate", json!(fresh));
                last = Some(fresh);
            }
            n0_future::time::sleep(CHECK_EVERY).await;
        }
    }

    pub(super) async fn voice_gate_view(&self) -> GateView {
        let servers = self.gate_servers();
        let waits = self
            .gate_waits(&servers)
            .await
            .into_iter()
            .map(|(sid, kind)| (sid.to_string(), WaitView { kind, reason: kind.reason() }))
            .collect();
        GateView { enabled: true, waits }
    }

    pub(super) async fn voice_wait(&self, server: ServerId) -> Option<Wait> {
        self.gate_waits(&[server]).await.remove(&server)
    }

    fn gate_servers(&self) -> Vec<ServerId> {
        let client = self.client.borrow();
        let mut servers: Vec<ServerId> = client.servers.lock().unwrap().keys().copied().collect();
        servers.extend(client.tickets.keys().copied());
        servers.sort();
        servers.dedup();
        servers
    }

    async fn net_signals(&self, servers: &[ServerId]) -> NetSignals {
        let node = self.node.lock().await;
        match node.as_ref() {
            Some(node) => NetSignals {
                up: true,
                relay: node.connection_checker().relay(),
                peers: servers.iter().map(|sid| (*sid, node.neighbor_count(*sid))).collect(),
            },
            None => NetSignals { up: false, relay: RelayLink::NotUsed, peers: BTreeMap::new() },
        }
    }

    fn online_for(&self, up: bool) -> Duration {
        let mut voice = self.voice.borrow_mut();
        if !up {
            voice.online_since = None;
            return Duration::ZERO;
        }
        voice.online_since.get_or_insert_with(web_time::Instant::now).elapsed()
    }

    async fn gate_waits(&self, servers: &[ServerId]) -> BTreeMap<ServerId, Wait> {
        let net = self.net_signals(servers).await;
        let online_for = self.online_for(net.up);
        let client = self.client.borrow();
        let me = client.profile().map(|p| p.id);
        let states = client.servers.lock().unwrap();
        let sync = self.sync.borrow();
        servers
            .iter()
            .filter_map(|sid| {
                let state = states.get(sid);
                let others = state.is_some_and(|state| state.server.members.iter().any(|m| Some(m.user) != me && state.server.is_current_member(m.user)));
                let signals = Signals {
                    network_up: net.up,
                    relay: net.relay,
                    loading: state.is_none(),
                    state: sync.as_ref().map_or(StateLink::NotUsed, |live| live.book.state_link(*sid)),
                    peers: net.peers.get(sid).copied().unwrap_or(0),
                    others,
                    online_for,
                };
                wait_for(&signals).map(|wait| (*sid, wait))
            })
            .collect()
    }
}
