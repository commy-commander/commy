use crate::state::App;
use commy_core::{ChannelId, ServerId, ServerState, UserId};
pub use commy_view::keys::KeyHolder;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

const TEND_EVERY: Duration = Duration::from_secs(3);

pub fn holder_of(app: &App) -> KeyHolder {
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    KeyHolder::new(me.zip(app.identity()))
}

pub async fn tend_channel(app: &App, handle: &tauri::AppHandle, sid: ServerId, channel: ChannelId) {
    if !app.key_settled() || app.awaits_snapshot(sid) {
        return;
    }
    let holder = holder_of(app);
    let _signing = app.signing.read().await;
    let events = {
        let mut servers = app.servers.lock().unwrap();
        let Some(state) = servers.get_mut(&sid) else { return };
        let events = holder.key_events(state, channel);
        let mut applied = Vec::new();
        for event in events {
            let Some(signed) = app.sign_event(sid, event.clone()) else { break };
            match state.apply_own(&signed) {
                Ok(_) => applied.push(event),
                Err(e) => tracing::warn!("channel key event refused locally: {e}"),
            }
        }
        applied
    };
    if events.is_empty() {
        return;
    }
    for event in events {
        crate::outbox::broadcast_local(app, sid, event).await;
    }
    app.persist_server(sid);
    let _ = handle.emit("server-event", &sid.to_string());
}

fn channels_with_work(app: &App, sid: ServerId) -> Vec<ChannelId> {
    let holder = holder_of(app);
    let servers = app.servers.lock().unwrap();
    servers.get(&sid).map(|state| holder.channels_with_work(state)).unwrap_or_default()
}

pub async fn tend_server(app: &App, handle: &tauri::AppHandle, sid: ServerId) {
    for channel in channels_with_work(app, sid) {
        tend_channel(app, handle, sid, channel).await;
    }
}

fn patience(state: &ServerState, me: UserId, channel: ChannelId) -> Duration {
    if state.server.owner == me {
        return Duration::ZERO;
    }
    let mixed = me.0.as_u128() ^ channel.0.as_u128();
    Duration::from_millis(1000 + (mixed % 5000) as u64)
}

pub async fn run(handle: tauri::AppHandle) {
    let mut noticed: HashMap<(ServerId, ChannelId), Instant> = HashMap::new();
    loop {
        tokio::time::sleep(TEND_EVERY).await;
        let app = handle.state::<App>();
        if crate::wake::is_resting(&app) {
            continue;
        }
        let Some(me) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { continue };
        let sids: Vec<ServerId> = app.servers.lock().unwrap().keys().copied().collect();
        let now = Instant::now();
        let mut pending = HashMap::new();
        let mut due = Vec::new();
        for sid in sids {
            for channel in channels_with_work(&app, sid) {
                let since = *noticed.get(&(sid, channel)).unwrap_or(&now);
                let wait = app.servers.lock().unwrap().get(&sid).map(|s| patience(s, me, channel)).unwrap_or_default();
                if now.duration_since(since) >= wait {
                    due.push((sid, channel));
                } else {
                    pending.insert((sid, channel), since);
                }
            }
        }
        noticed = pending;
        for (sid, channel) in due {
            tend_channel(&app, &handle, sid, channel).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::channel_key;
    use commy_core::{Identity, Message, Permissions, Role, RoleId, Server, ServerEvent, UserProfile};

    fn member(state: &mut ServerState, name: &str) -> (UserId, Identity) {
        let identity = Identity::generate();
        let mut profile = UserProfile::new(name);
        profile.identity = Some(identity.public());
        let id = profile.id;
        if state.server.owner == id || state.server.is_member(id) {
            state.profiles.insert(id, profile);
        } else {
            state.apply(ServerEvent::MemberJoin { server: state.server.id, profile }).unwrap();
        }
        (id, identity)
    }

    #[test]
    fn readers_see_sealed_text_and_outsiders_see_nothing() {
        let owner_identity = Identity::generate();
        let mut owner_profile = UserProfile::new("owner");
        owner_profile.identity = Some(owner_identity.public());
        let owner = owner_profile.id;
        let mut state = ServerState::new(Server::new("s", owner));
        state.profiles.insert(owner, owner_profile);
        let server = state.server.id;
        let channel = state.server.channels[0].id;
        let role = Role {
            id: RoleId::new(),
            name: "crew".into(),
            color: None,
            permissions: Permissions::NONE,
            position: 1,
            hoist: false,
            self_assignable: false,
            mentionable: false,
            tag: None,
            banner: None,
        };
        let crew = role.id;
        state.apply(ServerEvent::RoleCreate { server, role }).unwrap();
        state.apply(ServerEvent::ChannelAccessSet { server, channel, view: vec![crew], send: vec![] }).unwrap();
        let (outsider, outsider_identity) = member(&mut state, "outsider");
        let holder = KeyHolder::as_user(owner, owner_identity.clone());
        assert!(!holder.key_ready(&state, channel));
        assert_eq!(holder.channels_with_work(&state), vec![channel]);

        for event in holder.key_events(&state, channel) {
            state.apply(event).unwrap();
        }
        let holder = KeyHolder::as_user(owner, owner_identity.clone());
        assert!(holder.key_ready(&state, channel));
        assert!(holder.channels_with_work(&state).is_empty());

        let mut message = Message::new(channel, owner, "secret plans");
        holder.seal_outgoing(&state, &mut message).unwrap();
        assert!(channel_key::is_sealed(&message.content));
        assert_eq!(holder.open(&state, &message).unwrap().content, "secret plans");
        let edited = holder.seal_edit(&state, &message, "new plans".into()).unwrap();
        assert!(channel_key::is_sealed(&edited));
        assert!(KeyHolder::as_user(outsider, outsider_identity.clone()).open(&state, &message).is_none());
        assert!(KeyHolder::default().open(&state, &message).is_none());

        message.sign(server, &owner_identity);
        state.apply(ServerEvent::MessageCreate { message }).unwrap();
        let query = crate::search::SearchQuery::new(&["plans".to_string()], None, None, None);
        let found = |me: UserId, keys: &KeyHolder| crate::search::search(&state, &query, Some(me), crate::search::Page { offset: 0, limit: 10 }, keys).total;
        assert_eq!(found(owner, &holder), 1);
        assert_eq!(found(outsider, &KeyHolder::as_user(outsider, outsider_identity)), 0);
    }
}
