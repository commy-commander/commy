use std::collections::{HashMap, HashSet};

use chrono::Utc;
use commy_core::identity::IdentityKey;
use commy_core::{ServerId, ServerState};
use commy_net::mirror::MAX_SNAPSHOT_BYTES;
use commy_net::{MirrorAsk, MirrorRequest, MirroredServer};
use tauri::{AppHandle, Manager};

use crate::state::App;

const MESSAGES_PER_CHANNEL: usize = 500;

pub async fn run_mirror_requests(handle: AppHandle, mut asks: tokio::sync::mpsc::UnboundedReceiver<MirrorAsk>) {
    while let Some(ask) = asks.recv().await {
        let app = handle.state::<App>();
        let own = app.identity().map(|identity| identity.public());
        let awaiting = app.awaiting_snapshot.lock().unwrap().clone();
        let servers = app.servers.lock().unwrap();
        let reply = answer(own, &servers, &awaiting, &ask.request, Utc::now().timestamp_millis());
        drop(servers);
        if reply.is_none() {
            tracing::warn!("refused a server copy request from another device");
        }
        let _ = ask.reply.send(reply);
    }
}

fn answer(
    own: Option<IdentityKey>,
    servers: &HashMap<ServerId, ServerState>,
    awaiting: &HashSet<ServerId>,
    request: &MirrorRequest,
    now_ms: i64,
) -> Option<Vec<MirroredServer>> {
    if !request.is_from(&own?, now_ms) {
        return None;
    }
    let copies = request
        .servers
        .iter()
        .filter(|id| !awaiting.contains(id))
        .filter_map(|id| servers.get(id))
        .filter_map(|state| mirror_of(state, MESSAGES_PER_CHANNEL, MAX_SNAPSHOT_BYTES))
        .collect();
    Some(copies)
}

fn mirror_of(state: &ServerState, per_channel: usize, max_bytes: usize) -> Option<MirroredServer> {
    let mut keep = per_channel;
    loop {
        let mut snapshot = state.snapshot();
        for (_, list) in snapshot.messages.iter_mut() {
            let excess = list.len().saturating_sub(keep);
            list.drain(..excess);
        }
        let bytes = serde_json::to_vec(&snapshot).ok()?;
        if bytes.len() <= max_bytes {
            return Some(MirroredServer { server: state.server.id, snapshot: bytes });
        }
        if keep == 0 {
            return None;
        }
        keep /= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Identity, Message, Server, ServerEvent, Snapshot, UserProfile};

    fn server_with(messages: usize) -> (ServerState, Identity) {
        let identity = Identity::generate();
        let mut owner = UserProfile::new("owner");
        owner.identity = Some(identity.public());
        let server = Server::new("s", owner.id);
        let channel = server.channels[0].id;
        let mut state = ServerState::new(server);
        state.profiles.insert(owner.id, owner.clone());
        for i in 0..messages {
            let mut message = Message::new(channel, owner.id, format!("message {i}"));
            message.sign(state.server.id, &identity);
            state.apply(ServerEvent::MessageCreate { message }).unwrap();
        }
        (state, identity)
    }

    fn request(identity: &Identity, servers: Vec<ServerId>) -> MirrorRequest {
        MirrorRequest::new(identity, [1; 32], [2; 32], 1_000, servers)
    }

    #[test]
    fn copies_requested_servers_for_the_same_account() {
        let (state, identity) = server_with(3);
        let id = state.server.id;
        let servers = HashMap::from([(id, state)]);
        let copies = answer(Some(identity.public()), &servers, &HashSet::new(), &request(&identity, vec![id]), 1_000).unwrap();
        assert_eq!(copies.len(), 1);
        let snapshot: Snapshot = serde_json::from_slice(&copies[0].snapshot).unwrap();
        assert_eq!(snapshot.messages.iter().map(|(_, l)| l.len()).sum::<usize>(), 3);
    }

    #[test]
    fn refuses_another_account_and_skips_unsettled_servers() {
        let (state, identity) = server_with(1);
        let id = state.server.id;
        let servers = HashMap::from([(id, state)]);
        let stranger = Identity::generate();
        assert!(answer(Some(identity.public()), &servers, &HashSet::new(), &request(&stranger, vec![id]), 1_000).is_none());
        assert!(answer(None, &servers, &HashSet::new(), &request(&identity, vec![id]), 1_000).is_none());
        let awaiting = HashSet::from([id]);
        let copies = answer(Some(identity.public()), &servers, &awaiting, &request(&identity, vec![id]), 1_000).unwrap();
        assert!(copies.is_empty());
    }

    #[test]
    fn trims_history_until_the_copy_fits() {
        let (state, _) = server_with(40);
        let whole = mirror_of(&state, 1_000, usize::MAX).unwrap();
        let trimmed = mirror_of(&state, 1_000, whole.snapshot.len() / 2).unwrap();
        let snapshot: Snapshot = serde_json::from_slice(&trimmed.snapshot).unwrap();
        let channel = state.server.channels[0].id;
        let list = snapshot.messages.iter().find(|(id, _)| *id == channel).map(|(_, l)| l).unwrap();
        let kept: Vec<&str> = list.iter().map(|m| m.content.as_str()).collect();
        assert!(kept.len() < 40 && !kept.is_empty());
        assert_eq!(kept.last(), Some(&"message 39"));
    }
}
