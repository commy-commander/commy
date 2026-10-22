use commy_core::channel_key::{self, ChannelSecret, KeyId, KeyScope, Keyring};
use commy_core::{Attachment, ChannelId, Identity, Message, Permissions, ServerEvent, ServerState, UserId};
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

    pub fn key_events(&self, state: &ServerState, channel: ChannelId) -> Vec<ServerEvent> {
        self.scope_events(state, KeyScope::Channel(channel))
    }

    pub fn scope_events(&self, state: &ServerState, scope: KeyScope) -> Vec<ServerEvent> {
        let Some((me, identity)) = &self.me else { return Vec::new() };
        let mut ring = Keyring::unlock_scope(state, scope, *me, identity);
        match task_for(state, scope, *me, &ring) {
            Some(task) => ring.task_events(state, scope, task, chrono::Utc::now().timestamp_millis()),
            None => Vec::new(),
        }
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
                task_for(state, *scope, *me, &ring).is_some()
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

fn task_for(state: &ServerState, scope: KeyScope, me: UserId, ring: &Keyring) -> Option<channel_key::KeyTask> {
    match scope {
        KeyScope::Channel(channel) => channel_key::key_task(state, channel, me, ring),
        KeyScope::Community => channel_key::community_key_task(state, me, ring),
    }
}
