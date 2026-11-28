use commy_core::channel_key::{self, ChannelSecret, DriveField, KeyId, KeyScope, Keyring};
use commy_core::day_keys::{self, DayKeyring};
use commy_core::day_chain;
use commy_core::ids::DriveId;
use commy_core::search_keys::{GameDbKey, GifKey};
use commy_core::{Attachment, ChannelId, Identity, Message, MessageId, Permissions, ServerEvent, ServerState, UserId};
use std::collections::HashMap;
use std::sync::Mutex;

pub const KEY_PENDING: &str = "this channel's key has not reached you yet; try again in a moment";

#[derive(Debug, Default)]
pub struct KeyHolder {
    me: Option<(UserId, Identity)>,
    rings: Mutex<HashMap<ChannelId, Keyring>>,
}

impl KeyHolder {
    pub fn new(me: Option<(UserId, Identity)>) -> Self {
        Self { me, rings: Mutex::default() }
    }

    pub fn as_user(me: UserId, identity: Identity) -> Self {
        Self { me: Some((me, identity)), rings: Mutex::default() }
    }

    pub fn user(&self) -> Option<UserId> {
        self.me.as_ref().map(|(me, _)| *me)
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
        let perms = state.server.permissions_for(opened.author);
        if !perms.allows(Permissions::ATTACH_FILES) {
            opened.attachments.retain(|a| !a.is_plain_file());
        }
        if !perms.allows(Permissions::SEND_VOICE_MESSAGES) {
            opened.attachments.retain(|a| !a.is_voice_message());
        }
        Some(opened)
    }

    pub fn hold(&self, state: &ServerState, message: &Message) -> Option<commy_core::Hold> {
        match self.open(state, message) {
            Some(opened) => state.automod().hold(&state.server, state.messages(message.channel), &opened),
            None => state.hold(message),
        }
    }

    pub fn open_server_text(&self, state: &ServerState, purpose: &str, text: &str) -> Option<String> {
        if !channel_key::is_sealed(text) {
            return Some(text.to_string());
        }
        let (me, identity) = self.me.as_ref()?;
        let ring = Keyring::unlock_community(state, *me, identity);
        channel_key::read_server_text(state.server.id, purpose, text, &ring)
    }

    pub fn opened_search_keys(&self, state: &ServerState) -> (Option<GifKey>, Option<GameDbKey>) {
        let gifs = state.server.gif_key.clone().and_then(|g| {
            let key = self.open_server_text(state, commy_core::search_keys::GIF_KEY_PURPOSE, &g.key)?;
            Some(GifKey { key, ..g })
        });
        let games = state.server.game_db.clone().and_then(|g| {
            let client_secret = self.open_server_text(state, commy_core::search_keys::GAME_DB_SECRET_PURPOSE, &g.client_secret)?;
            Some(GameDbKey { client_secret, ..g })
        });
        (gifs, games)
    }

    pub fn open_label(&self, state: &ServerState, channel: ChannelId, root: MessageId, label: &str) -> Option<String> {
        if !channel_key::is_sealed(label) {
            return Some(label.to_string());
        }
        self.with_ring(state, channel, |ring| channel_key::read_label(state.server.id, channel, root, label, ring))?
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

    pub fn seal_search_keys(&self, state: &ServerState, event: ServerEvent) -> ServerEvent {
        let Some((me, identity)) = self.me.as_ref() else { return event };
        let ring = Keyring::unlock_community(state, *me, identity);
        let Some((key, secret)) = ring.newest_in(state, KeyScope::Community) else { return event };
        let server = state.server.id;
        let seal = |purpose: &str, text: &str| channel_key::seal_server_text(server, purpose, text, key, secret);
        match event {
            ServerEvent::GifKeySet { server, gifs } => ServerEvent::GifKeySet {
                server,
                gifs: gifs.map(|g| GifKey { key: seal(commy_core::search_keys::GIF_KEY_PURPOSE, &g.key), ..g }),
            },
            ServerEvent::GameDbSet { server, games } => ServerEvent::GameDbSet {
                server,
                games: games.map(|g| GameDbKey { client_secret: seal(commy_core::search_keys::GAME_DB_SECRET_PURPOSE, &g.client_secret), ..g }),
            },
            other => other,
        }
    }

    pub fn open_drive_text(&self, state: &ServerState, channel: ChannelId, item: DriveId, field: DriveField, text: &str) -> Option<String> {
        if !channel_key::is_sealed(text) {
            return Some(text.to_string());
        }
        self.with_ring(state, channel, |ring| channel_key::read_drive_text(state.server.id, channel, item, field, text, ring))?
    }

    pub fn seal_drive(&self, state: &ServerState, event: ServerEvent) -> Result<ServerEvent, String> {
        let (channel, item) = match &event {
            ServerEvent::DriveAdd { item, .. } => (item.channel, item.id),
            ServerEvent::DriveEdit { item, .. } => match commy_core::drive::find(&state.server.drive, *item) {
                Some(known) => (known.channel, *item),
                None => return Ok(event),
            },
            _ => return Ok(event),
        };
        if !is_private(state, channel) {
            return Ok(event);
        }
        let ring = self.ring(state, channel)?;
        let (key, secret) = ring.sending_key(state, channel).ok_or(KEY_PENDING)?;
        let seal = |field: DriveField, text: &str| channel_key::seal_drive_text(state.server.id, channel, item, field, text, key, secret);
        Ok(match event {
            ServerEvent::DriveAdd { server, mut item } => {
                item.name = seal(DriveField::Name, &item.name);
                if let Some(file) = item.file.as_mut() {
                    file.mime = seal(DriveField::Mime, &file.mime);
                }
                ServerEvent::DriveAdd { server, item }
            }
            ServerEvent::DriveEdit { server, item, name, parent } => ServerEvent::DriveEdit { server, item, name: seal(DriveField::Name, &name), parent },
            other => other,
        })
    }

    pub fn seal_labels(&self, state: &ServerState, event: ServerEvent) -> Result<ServerEvent, String> {
        let (channel, root) = match &event {
            ServerEvent::ThreadSet { channel, root, name: Some(_), .. } => (*channel, *root),
            ServerEvent::ForumPost { channel, message, .. } => (*channel, *message),
            _ => return Ok(event),
        };
        if !is_private(state, channel) {
            return Ok(event);
        }
        let ring = self.ring(state, channel)?;
        let (key, secret) = ring.sending_key(state, channel).ok_or(KEY_PENDING)?;
        let seal = |label: &str| channel_key::seal_label(state.server.id, channel, root, label, key, secret);
        Ok(match event {
            ServerEvent::ThreadSet { server, channel, root, name, archived, locked, by } => {
                ServerEvent::ThreadSet { server, channel, root, name: name.map(|n| seal(&n)), archived, locked, by }
            }
            ServerEvent::ForumPost { server, channel, message, title, tags, by } => {
                ServerEvent::ForumPost { server, channel, message, title: seal(&title), tags, by }
            }
            other => other,
        })
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

    pub fn key_events(&self, state: &ServerState, channel: ChannelId) -> Vec<ServerEvent> {
        self.scope_events(state, KeyScope::Channel(channel))
    }

    pub fn scope_events(&self, state: &ServerState, scope: KeyScope) -> Vec<ServerEvent> {
        let Some((me, identity)) = &self.me else { return Vec::new() };
        let mut ring = Keyring::unlock_scope(state, scope, *me, identity);
        let now = chrono::Utc::now().timestamp_millis();
        let mut events = match task_for(state, scope, *me, &ring) {
            Some(task) => ring.task_events(state, scope, task, now),
            None => Vec::new(),
        };
        if scope == KeyScope::Community && events.is_empty() {
            events = self.day_events(state, now);
        }
        events
    }

    pub fn day_ring(&self, state: &ServerState) -> DayKeyring {
        match &self.me {
            Some((me, identity)) => DayKeyring::unlock(state, *me, identity),
            None => DayKeyring::default(),
        }
    }

    fn day_work(&self, state: &ServerState, now_ms: i64) -> Option<DayWork> {
        let (me, _) = self.me.as_ref()?;
        if let Some(ready) = day_chain::ready_event(&state.server, *me) {
            return Some(DayWork::Say(ready));
        }
        if let Some(switch) = day_chain::auto_switch_event(&state.server, *me, now_ms) {
            return Some(DayWork::Say(switch));
        }
        let ring = self.day_ring(state);
        day_keys::day_key_task(state, *me, &ring, now_ms).map(|task| DayWork::Keys(ring, task))
    }

    pub fn day_events(&self, state: &ServerState, now_ms: i64) -> Vec<ServerEvent> {
        match self.day_work(state, now_ms) {
            Some(DayWork::Say(event)) => vec![event],
            Some(DayWork::Keys(mut ring, task)) => ring.task_events(state, task, now_ms),
            None => Vec::new(),
        }
    }

    pub fn rotation_events(&self, state: &ServerState, scope: KeyScope) -> Vec<ServerEvent> {
        let Some((me, identity)) = &self.me else { return Vec::new() };
        let mut ring = Keyring::unlock_scope(state, scope, *me, identity);
        let heads = match scope {
            KeyScope::Community => state.community_keys().heads(),
            KeyScope::Channel(channel) => state.channel_keys(channel).map(|keys| keys.heads()).unwrap_or_default(),
        };
        let links: Vec<_> = heads.into_iter().filter(|key| ring.holds(*key)).collect();
        if links.is_empty() {
            return Vec::new();
        }
        ring.task_events(state, scope, channel_key::KeyTask::Rotate { links }, chrono::Utc::now().timestamp_millis())
    }

    pub fn channels_with_work(&self, state: &ServerState) -> Vec<ChannelId> {
        self.scopes_with_work(state)
            .into_iter()
            .filter_map(|scope| match scope {
                KeyScope::Channel(channel) => Some(channel),
                KeyScope::Community => None,
            })
            .collect()
    }

    pub fn scopes_with_work(&self, state: &ServerState) -> Vec<KeyScope> {
        let Some((me, identity)) = &self.me else { return Vec::new() };
        let private = state.server.channels.iter().filter(|c| c.is_private()).map(|c| KeyScope::Channel(c.id));
        std::iter::once(KeyScope::Community)
            .chain(private)
            .filter(|scope| {
                let ring = Keyring::unlock_scope(state, *scope, *me, identity);
                let days = *scope == KeyScope::Community && self.day_work(state, chrono::Utc::now().timestamp_millis()).is_some();
                days || task_for(state, *scope, *me, &ring).is_some()
            })
            .collect()
    }

    pub fn community_ring(&self, state: &ServerState, invited: Option<(KeyId, ChannelSecret)>) -> Option<Keyring> {
        let (me, identity) = self.me.as_ref()?;
        let mut ring = Keyring::unlock_community(state, *me, identity);
        if let Some((id, secret)) = invited {
            ring.admit(state, KeyScope::Community, id, secret);
        }
        Some(ring)
    }
}

enum DayWork {
    Say(ServerEvent),
    Keys(DayKeyring, day_keys::DayTask),
}

fn task_for(state: &ServerState, scope: KeyScope, me: UserId, ring: &Keyring) -> Option<channel_key::KeyTask> {
    match scope {
        KeyScope::Channel(channel) => channel_key::key_task(state, channel, me, ring),
        KeyScope::Community => channel_key::community_key_task(state, me, ring),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Identity, Server, UserProfile};

    #[test]
    fn an_owner_can_move_the_community_to_a_fresh_key_on_demand() {
        let identity = Identity::generate();
        let mut profile = UserProfile::new("owner");
        profile.identity = Some(identity.public());
        let owner = profile.id;
        let mut state = ServerState::new(Server::new("s", owner));
        state.profiles.insert(owner, profile);
        let holder = KeyHolder::as_user(owner, identity);
        assert!(holder.rotation_events(&state, KeyScope::Community).is_empty(), "nothing to move on from yet");
        for event in holder.scope_events(&state, KeyScope::Community) {
            state.apply(event).unwrap();
        }
        let first = state.community_keys().newest().unwrap().id;
        for event in holder.rotation_events(&state, KeyScope::Community) {
            state.apply(event).unwrap();
        }
        let second = state.community_keys().newest().unwrap().id;
        assert_ne!(first, second);
        assert!(holder.community_ring(&state, None).unwrap().holds(second));
        assert!(KeyHolder::default().rotation_events(&state, KeyScope::Community).is_empty());
    }

    #[test]
    fn an_updated_app_says_it_is_ready_and_the_owner_switches_on_once_everyone_has() {
        use commy_core::test_support::{fixture, identity_for, join};
        let (mut state, _, owner) = fixture();
        let member = join(&mut state, "member");
        let owners = KeyHolder::as_user(owner, identity_for(owner));
        let members = KeyHolder::as_user(member, identity_for(member));
        let tend = |holder: &KeyHolder, state: &mut ServerState| {
            let events = holder.scope_events(state, KeyScope::Community);
            for event in &events {
                state.apply(event.clone()).unwrap();
            }
            events
        };
        while !tend(&owners, &mut state).is_empty() {}
        assert!(!day_chain::is_on(&state.server), "the member hasn't said their app is ready");
        assert!(day_chain::reads_day_keys(&state.server, owner));
        assert_eq!(owners.scopes_with_work(&state), Vec::<KeyScope>::new());
        assert_eq!(members.scopes_with_work(&state), vec![KeyScope::Community]);
        while !tend(&members, &mut state).is_empty() {}
        assert!(day_chain::reads_day_keys(&state.server, member));
        while !tend(&owners, &mut state).is_empty() {}
        assert!(day_chain::is_on(&state.server));
        let newest = state.day_chain().newest().unwrap().id;
        assert!(members.day_ring(&state).holds(newest));
        assert!(owners.scopes_with_work(&state).is_empty() && members.scopes_with_work(&state).is_empty());
        assert!(KeyHolder::default().day_ring(&state).newest(&state).is_none());
    }
}
