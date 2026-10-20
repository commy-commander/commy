use crate::state::App;
use commy_core::channel_key::{self, KeyTask, Keyring};
use commy_core::{Attachment, ChannelId, Identity, Message, Permissions, ServerEvent, ServerId, ServerState, UserId};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

pub const KEY_PENDING: &str = "this channel's key has not reached you yet; try again in a moment";
const TEND_EVERY: Duration = Duration::from_secs(3);

#[derive(Debug, Default)]
pub struct KeyHolder {
    me: Option<(UserId, Identity)>,
    rings: Mutex<HashMap<ChannelId, Keyring>>,
}

impl KeyHolder {
    pub fn of(app: &App) -> Self {
        let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
        Self { me: me.zip(app.identity()), rings: Mutex::default() }
    }

    #[cfg(test)]
    pub fn as_user(me: UserId, identity: Identity) -> Self {
        Self { me: Some((me, identity)), rings: Mutex::default() }
    }

    fn with_ring<T>(&self, state: &ServerState, channel: ChannelId, use_ring: impl FnOnce(&Keyring) -> T) -> Option<T> {
        let (me, identity) = self.me.as_ref()?;
        let mut rings = self.rings.lock().unwrap();
        let ring = rings.entry(channel).or_insert_with(|| Keyring::unlock(state, channel, *me, identity));
        Some(use_ring(ring))
    }

    pub fn open(&self, state: &ServerState, message: &Message) -> Option<Message> {
        if !channel_key::is_sealed(&message.content) {
            return Some(message.clone());
        }
        let mut opened =
            self.with_ring(state, message.channel, |ring| channel_key::read_message(state.server.id, message, ring))??;
        if !state.server.permissions_for(opened.author).allows(Permissions::ATTACH_FILES) {
            opened.attachments.clear();
        }
        Some(opened)
    }

    pub fn hold(&self, state: &ServerState, message: &Message) -> Option<commy_core::Hold> {
        match self.open(state, message) {
            Some(opened) => state.automod().hold(&state.server, state.messages(message.channel), &opened),
            None => state.hold(message),
        }
    }

    pub fn key_ready(&self, state: &ServerState, channel: ChannelId) -> bool {
        let private = state.server.channel(channel).is_some_and(|c| c.is_private());
        !private || self.with_ring(state, channel, |ring| ring.sending_key(state, channel).is_some()).unwrap_or(false)
    }
}

fn is_private(state: &ServerState, channel: ChannelId) -> bool {
    state.server.channel(channel).is_some_and(|c| c.is_private())
}

impl KeyHolder {
    fn ring(&self, state: &ServerState, channel: ChannelId) -> Result<Keyring, String> {
        let (me, identity) = self.me.as_ref().ok_or("your identity is not ready yet")?;
        Ok(Keyring::unlock(state, channel, *me, identity))
    }

    pub fn seal_outgoing(&self, state: &ServerState, message: &mut Message) -> Result<(), String> {
        if !is_private(state, message.channel) {
            return Ok(());
        }
        let ring = self.ring(state, message.channel)?;
        let (key, secret) = ring.sending_key(state, message.channel).ok_or(KEY_PENDING)?;
        channel_key::seal_message(state.server.id, message, key, secret);
        Ok(())
    }

    pub fn seal_edit(&self, state: &ServerState, original: &Message, content: String) -> Result<String, String> {
        let sealed_before = channel_key::is_sealed(&original.content);
        if !sealed_before && !is_private(state, original.channel) {
            return Ok(content);
        }
        let ring = self.ring(state, original.channel)?;
        let attachments: Vec<Attachment> = if sealed_before {
            channel_key::read_message(state.server.id, original, &ring).ok_or(KEY_PENDING)?.attachments
        } else {
            original.attachments.clone()
        };
        let (key, secret) = ring.sending_key(state, original.channel).ok_or(KEY_PENDING)?;
        Ok(channel_key::seal_body(state.server.id, original, &content, &attachments, key, secret))
    }

    fn key_events(&self, state: &ServerState, channel: ChannelId) -> Vec<ServerEvent> {
        let Some((me, _)) = &self.me else { return Vec::new() };
        let Ok(mut ring) = self.ring(state, channel) else { return Vec::new() };
        let server = state.server.id;
        match channel_key::key_task(state, channel, *me, &ring) {
            Some(KeyTask::Rotate { links }) => {
                let record = ring.rotate(server, channel, &links, chrono::Utc::now().timestamp_millis());
                let key = record.id;
                let mut events = vec![ServerEvent::ChannelKeyAdd { server, channel, key: record }];
                events.extend(ring.grant_events(state, channel, key, &state.server.readers_of(channel)));
                events
            }
            Some(KeyTask::Grant { key, to }) => ring.grant_events(state, channel, key, &to),
            None => Vec::new(),
        }
    }

    fn channels_with_work(&self, state: &ServerState) -> Vec<ChannelId> {
        let Some((me, identity)) = &self.me else { return Vec::new() };
        state
            .server
            .channels
            .iter()
            .filter(|c| c.is_private())
            .map(|c| c.id)
            .filter(|channel| {
                let ring = Keyring::unlock(state, *channel, *me, identity);
                channel_key::key_task(state, *channel, *me, &ring).is_some()
            })
            .collect()
    }
}

pub async fn tend_channel(app: &App, handle: &tauri::AppHandle, sid: ServerId, channel: ChannelId) {
    if !app.key_settled() || app.awaits_snapshot(sid) {
        return;
    }
    let holder = KeyHolder::of(app);
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
    let holder = KeyHolder::of(app);
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
    use commy_core::{Role, RoleId, Server, UserProfile};

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
        let found = |me: UserId, keys: &KeyHolder| crate::search::search(&state, &query, Some(me), 10, keys).total;
        assert_eq!(found(owner, &holder), 1);
        assert_eq!(found(outsider, &KeyHolder::as_user(outsider, outsider_identity)), 0);
    }
}
