use crate::event::ServerEvent;
use crate::identity::{to_hex, Identity, IdentityKey};
use crate::ids::{ChannelId, MessageId, ServerId, UserId};
use crate::message::{Attachment, Message};
use crate::store::ServerState;
use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use curve25519_dalek::montgomery::MontgomeryPoint;
use data_encoding::BASE64URL_NOPAD;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::{HashMap, HashSet};
use std::fmt;

const GRANT_DOMAIN: &str = "commy-channel-grant/v1";
const LINK_DOMAIN: &str = "commy-channel-link/v1";
const BODY_DOMAIN: &str = "commy-channel-body/v1";
const SEALED_PREFIX: &str = "commy-sealed:v1:";
const TAG_BYTES: usize = 16;
const SEALED_SECRET_BYTES: usize = 32 + TAG_BYTES;

pub const MAX_KEYS_PER_CHANNEL: usize = 64;
pub const MAX_LINKS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct KeyId(pub [u8; 16]);

impl KeyId {
    pub fn random() -> Self {
        Self(rand::random())
    }

    pub fn to_hex(&self) -> String {
        to_hex(&self.0)
    }
}

impl fmt::Debug for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KeyId({})", self.to_hex())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ChannelSecret([u8; 32]);

impl ChannelSecret {
    pub fn random() -> Self {
        Self(rand::random())
    }

    fn from_plain(bytes: Vec<u8>) -> Option<Self> {
        bytes.try_into().ok().map(Self)
    }
}

impl fmt::Debug for ChannelSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChannelSecret(..)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyLink {
    pub prev: KeyId,
    pub nonce: [u8; 12],
    pub sealed: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyRecord {
    pub id: KeyId,
    pub at: i64,
    pub links: Vec<KeyLink>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedKey {
    pub key: KeyId,
    pub ephemeral: [u8; 32],
    pub nonce: [u8; 12],
    pub sealed: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelKeys {
    pub keys: Vec<KeyRecord>,
    pub grants: Vec<(UserId, SealedKey)>,
}

fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    postcard::to_stdvec(value).expect("postcard encodes plain tuples")
}

fn seal(key: &[u8; 32], nonce: [u8; 12], aad: &[u8], plain: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new_from_slice(key).expect("32 byte key");
    cipher
        .encrypt(&Nonce::from(nonce), Payload { msg: plain, aad })
        .expect("in-memory encryption cannot fail")
}

fn unseal(key: &[u8; 32], nonce: [u8; 12], aad: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key).expect("32 byte key");
    cipher.decrypt(&Nonce::from(nonce), Payload { msg: sealed, aad }).ok()
}

fn wrapping_key(shared: &[u8; 32], info: &[u8]) -> [u8; 32] {
    let mut extract = <Hmac<Sha256> as KeyInit>::new_from_slice(GRANT_DOMAIN.as_bytes()).expect("any key length");
    extract.update(shared);
    let prk = extract.finalize().into_bytes();
    let mut expand = <Hmac<Sha256> as KeyInit>::new_from_slice(&prk).expect("any key length");
    expand.update(info);
    expand.update(&[1]);
    expand.finalize().into_bytes().into()
}

fn grant_info(server: ServerId, channel: ChannelId, user: UserId, key: KeyId, ephemeral: [u8; 32], point: [u8; 32]) -> Vec<u8> {
    encode(&(GRANT_DOMAIN, server, channel, user, key, ephemeral, point))
}

impl SealedKey {
    pub fn seal(
        server: ServerId,
        channel: ChannelId,
        user: UserId,
        recipient: &IdentityKey,
        key: KeyId,
        secret: &ChannelSecret,
    ) -> Option<Self> {
        let point = recipient.agreement_point()?;
        let ephemeral_secret: [u8; 32] = rand::random();
        let ephemeral = MontgomeryPoint::mul_base_clamped(ephemeral_secret).to_bytes();
        let shared = MontgomeryPoint(point).mul_clamped(ephemeral_secret).to_bytes();
        if shared == [0u8; 32] {
            return None;
        }
        let wrap = wrapping_key(&shared, &grant_info(server, channel, user, key, ephemeral, point));
        let nonce = rand::random();
        Some(Self { key, ephemeral, nonce, sealed: seal(&wrap, nonce, &[], &secret.0) })
    }

    pub fn open(&self, server: ServerId, channel: ChannelId, me: UserId, identity: &Identity) -> Option<ChannelSecret> {
        let point = identity.public().agreement_point()?;
        let shared = identity.agree(self.ephemeral)?;
        let wrap = wrapping_key(&shared, &grant_info(server, channel, me, self.key, self.ephemeral, point));
        unseal(&wrap, self.nonce, &[], &self.sealed).and_then(ChannelSecret::from_plain)
    }

    fn well_formed(&self) -> bool {
        self.sealed.len() == SEALED_SECRET_BYTES
    }
}

fn link_aad(server: ServerId, channel: ChannelId, from: KeyId, prev: KeyId) -> Vec<u8> {
    encode(&(LINK_DOMAIN, server, channel, from, prev))
}

impl KeyLink {
    fn seal(server: ServerId, channel: ChannelId, from: KeyId, secret: &ChannelSecret, prev: KeyId, prev_secret: &ChannelSecret) -> Self {
        let nonce = rand::random();
        let sealed = seal(&secret.0, nonce, &link_aad(server, channel, from, prev), &prev_secret.0);
        Self { prev, nonce, sealed }
    }

    fn open(&self, server: ServerId, channel: ChannelId, from: KeyId, secret: &ChannelSecret) -> Option<ChannelSecret> {
        unseal(&secret.0, self.nonce, &link_aad(server, channel, from, self.prev), &self.sealed)
            .and_then(ChannelSecret::from_plain)
    }
}

impl KeyRecord {
    fn well_formed(&self) -> bool {
        self.links.len() <= MAX_LINKS
            && self.links.iter().all(|l| l.sealed.len() == SEALED_SECRET_BYTES && l.prev != self.id)
    }
}

impl ChannelKeys {
    pub fn record(&self, id: KeyId) -> Option<&KeyRecord> {
        self.keys.iter().find(|k| k.id == id)
    }

    pub fn newest(&self) -> Option<&KeyRecord> {
        self.keys.iter().max_by_key(|k| (k.at, k.id))
    }

    pub fn heads(&self) -> Vec<KeyId> {
        let linked: HashSet<KeyId> = self.keys.iter().flat_map(|k| k.links.iter().map(|l| l.prev)).collect();
        self.keys.iter().map(|k| k.id).filter(|id| !linked.contains(id)).collect()
    }

    pub fn grant_for(&self, user: UserId) -> Option<&SealedKey> {
        self.grants.iter().find(|(u, _)| *u == user).map(|(_, g)| g)
    }

    pub(crate) fn add_key(&mut self, record: KeyRecord) -> bool {
        if !record.well_formed() || self.record(record.id).is_some() {
            return false;
        }
        self.keys.push(record);
        while self.keys.len() > MAX_KEYS_PER_CHANNEL {
            let Some(oldest) = self.keys.iter().enumerate().min_by_key(|(_, k)| (k.at, k.id)).map(|(i, _)| i) else {
                break;
            };
            self.keys.remove(oldest);
        }
        true
    }

    pub(crate) fn set_grant(&mut self, user: UserId, grant: SealedKey, room: usize) -> Option<bool> {
        if !grant.well_formed() {
            return Some(false);
        }
        let full = self.grants.len() >= room;
        match self.grants.iter_mut().find(|(u, _)| *u == user) {
            Some((_, held)) if *held == grant => Some(false),
            Some((_, held)) => {
                *held = grant;
                Some(true)
            }
            None if full => None,
            None => {
                self.grants.push((user, grant));
                Some(true)
            }
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Keyring {
    secrets: HashMap<KeyId, ChannelSecret>,
}

impl Keyring {
    pub fn unlock(state: &ServerState, channel: ChannelId, me: UserId, identity: &Identity) -> Self {
        let mut ring = Self::default();
        let Some(keys) = state.channel_keys(channel) else { return ring };
        let server = state.server.id;
        if let Some(grant) = keys.grant_for(me) {
            if let Some(secret) = grant.open(server, channel, me, identity) {
                ring.secrets.insert(grant.key, secret);
            }
        }
        ring.follow_links(server, channel, keys);
        ring
    }

    fn follow_links(&mut self, server: ServerId, channel: ChannelId, keys: &ChannelKeys) {
        let mut unvisited: Vec<KeyId> = self.secrets.keys().copied().collect();
        while let Some(id) = unvisited.pop() {
            let (Some(record), Some(secret)) = (keys.record(id), self.secrets.get(&id).cloned()) else { continue };
            for link in &record.links {
                if self.secrets.contains_key(&link.prev) {
                    continue;
                }
                if let Some(prev) = link.open(server, channel, id, &secret) {
                    self.secrets.insert(link.prev, prev);
                    unvisited.push(link.prev);
                }
            }
        }
    }

    pub fn holds(&self, id: KeyId) -> bool {
        self.secrets.contains_key(&id)
    }

    pub fn secret(&self, id: KeyId) -> Option<&ChannelSecret> {
        self.secrets.get(&id)
    }

    pub fn sending_key(&self, state: &ServerState, channel: ChannelId) -> Option<(KeyId, &ChannelSecret)> {
        let newest = state.channel_keys(channel)?.newest()?.id;
        self.secret(newest).map(|secret| (newest, secret))
    }

    pub fn rotate(&mut self, server: ServerId, channel: ChannelId, links: &[KeyId], at: i64) -> KeyRecord {
        let id = KeyId::random();
        let secret = ChannelSecret::random();
        let links = links
            .iter()
            .take(MAX_LINKS)
            .filter_map(|prev| self.secret(*prev).map(|s| KeyLink::seal(server, channel, id, &secret, *prev, s)))
            .collect();
        self.secrets.insert(id, secret);
        KeyRecord { id, at, links }
    }

    pub fn grant_events(&self, state: &ServerState, channel: ChannelId, key: KeyId, to: &[UserId]) -> Vec<ServerEvent> {
        let Some(secret) = self.secret(key) else { return Vec::new() };
        let server = state.server.id;
        to.iter()
            .filter_map(|user| {
                let recipient = state.identity_of(*user)?;
                let grant = SealedKey::seal(server, channel, *user, &recipient, key, secret)?;
                Some(ServerEvent::ChannelKeyGrant { server, channel, user: *user, grant })
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyTask {
    Rotate { links: Vec<KeyId> },
    Grant { key: KeyId, to: Vec<UserId> },
}

pub fn key_task(state: &ServerState, channel: ChannelId, me: UserId, ring: &Keyring) -> Option<KeyTask> {
    let server = &state.server;
    let found = server.channel(channel)?;
    if !found.is_private() || !server.is_current_member(me) || !server.can_view(channel, me) {
        return None;
    }
    let empty = ChannelKeys::default();
    let keys = state.channel_keys(channel).unwrap_or(&empty);
    let heads = keys.heads();
    if heads.is_empty() {
        return Some(KeyTask::Rotate { links: Vec::new() });
    }
    let held: Vec<KeyId> = heads.iter().copied().filter(|k| ring.holds(*k)).collect();
    let reader = |user: UserId| server.is_current_member(user) && server.can_view(channel, user);
    let exposed = keys.grants.iter().any(|(user, g)| heads.contains(&g.key) && !reader(*user));
    let forked = heads.len() > 1 && held.len() == heads.len();
    if (exposed || forked) && !held.is_empty() {
        return Some(KeyTask::Rotate { links: held });
    }
    let newest = keys.newest()?.id;
    if !ring.holds(newest) {
        return None;
    }
    let to: Vec<UserId> = server
        .readers_of(channel)
        .into_iter()
        .filter(|u| state.identity_of(*u).is_some() && keys.grant_for(*u).map(|g| g.key) != Some(newest))
        .collect();
    (!to.is_empty()).then_some(KeyTask::Grant { key: newest, to })
}

#[derive(Serialize, Deserialize)]
struct Body {
    content: String,
    attachments: Vec<Attachment>,
}

fn body_aad(server: ServerId, channel: ChannelId, message: MessageId, author: UserId) -> Vec<u8> {
    encode(&(BODY_DOMAIN, server, channel, message, author))
}

pub fn is_sealed(content: &str) -> bool {
    content.starts_with(SEALED_PREFIX)
}

pub fn seal_body(
    server: ServerId,
    message: &Message,
    content: &str,
    attachments: &[Attachment],
    key: KeyId,
    secret: &ChannelSecret,
) -> String {
    let nonce: [u8; 12] = rand::random();
    let body = encode(&Body { content: content.to_string(), attachments: attachments.to_vec() });
    let aad = body_aad(server, message.channel, message.id, message.author);
    let mut bytes = Vec::with_capacity(28 + body.len() + TAG_BYTES);
    bytes.extend_from_slice(&key.0);
    bytes.extend_from_slice(&nonce);
    bytes.extend(seal(&secret.0, nonce, &aad, &body));
    format!("{SEALED_PREFIX}{}", BASE64URL_NOPAD.encode(&bytes))
}

pub fn seal_message(server: ServerId, message: &mut Message, key: KeyId, secret: &ChannelSecret) {
    let attachments = std::mem::take(&mut message.attachments);
    message.content = seal_body(server, message, &message.content, &attachments, key, secret);
}

pub fn read_message(server: ServerId, message: &Message, ring: &Keyring) -> Option<Message> {
    let Some(encoded) = message.content.strip_prefix(SEALED_PREFIX) else {
        return Some(message.clone());
    };
    let bytes = BASE64URL_NOPAD.decode(encoded.as_bytes()).ok()?;
    if bytes.len() < 28 + TAG_BYTES {
        return None;
    }
    let key = KeyId(bytes[..16].try_into().ok()?);
    let nonce: [u8; 12] = bytes[16..28].try_into().ok()?;
    let secret = ring.secret(key)?;
    let aad = body_aad(server, message.channel, message.id, message.author);
    let plain = unseal(&secret.0, nonce, &aad, &bytes[28..])?;
    let body: Body = postcard::from_bytes(&plain).ok()?;
    let mut opened = message.clone();
    opened.content = body.content;
    opened.attachments = body.attachments;
    Some(opened)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::RoleId;
    use crate::permissions::Permissions;
    use crate::server::Role;
    use crate::test_support::{fixture, identity_for, join, post};

    fn private_channel(state: &mut ServerState, channel: ChannelId) -> RoleId {
        let server = state.server.id;
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
        let id = role.id;
        state.apply(ServerEvent::RoleCreate { server, role }).unwrap();
        state
            .apply(ServerEvent::ChannelAccessSet { server, channel, view: vec![id], send: Vec::new() })
            .unwrap();
        id
    }

    fn run(state: &mut ServerState, channel: ChannelId, me: UserId, ring: &mut Keyring) -> Option<KeyTask> {
        let task = key_task(state, channel, me, ring)?;
        let server = state.server.id;
        let (key, to) = match &task {
            KeyTask::Rotate { links } => {
                let record = ring.rotate(server, channel, links, chrono::Utc::now().timestamp_millis());
                let id = record.id;
                state.apply(ServerEvent::ChannelKeyAdd { server, channel, key: record }).unwrap();
                (id, state.server.readers_of(channel))
            }
            KeyTask::Grant { key, to } => (*key, to.clone()),
        };
        for event in ring.grant_events(state, channel, key, &to) {
            state.apply(event).unwrap();
        }
        Some(task)
    }

    fn settle(state: &mut ServerState, channel: ChannelId, me: UserId) -> Keyring {
        let identity = identity_for(me);
        for _ in 0..4 {
            let mut ring = Keyring::unlock(state, channel, me, &identity);
            if run(state, channel, me, &mut ring).is_none() {
                return ring;
            }
        }
        panic!("key work did not settle");
    }

    #[test]
    fn readers_open_sealed_messages_and_others_do_not() {
        let (mut state, channel, owner) = fixture();
        let server = state.server.id;
        let crew = private_channel(&mut state, channel);
        let reader = join(&mut state, "reader");
        let outsider = join(&mut state, "outsider");
        state.apply(ServerEvent::RoleAssign { server, user: reader, role: crew }).unwrap();
        let ring = settle(&mut state, channel, owner);
        let (key, secret) = ring.sending_key(&state, channel).unwrap();
        let mut message = Message::new(channel, owner, "plans");
        seal_message(server, &mut message, key, secret);
        assert!(is_sealed(&message.content) && !message.content.contains("plans"));

        let theirs = Keyring::unlock(&state, channel, reader, &identity_for(reader));
        assert_eq!(read_message(server, &message, &theirs).unwrap().content, "plans");
        let none = Keyring::unlock(&state, channel, outsider, &identity_for(outsider));
        assert!(read_message(server, &message, &none).is_none());
        assert!(state.channel_keys(channel).unwrap().grant_for(outsider).is_none());
    }

    #[test]
    fn losing_the_role_rotates_the_key_and_keeps_history_for_readers() {
        let (mut state, channel, owner) = fixture();
        let server = state.server.id;
        let crew = private_channel(&mut state, channel);
        let leaver = join(&mut state, "leaver");
        let stayer = join(&mut state, "stayer");
        for user in [leaver, stayer] {
            state.apply(ServerEvent::RoleAssign { server, user, role: crew }).unwrap();
        }
        let ring = settle(&mut state, channel, owner);
        let (key, secret) = ring.sending_key(&state, channel).unwrap();
        let mut before = Message::new(channel, owner, "before");
        seal_message(server, &mut before, key, secret);

        state.apply(ServerEvent::RoleUnassign { server, user: leaver, role: crew }).unwrap();
        let ring = Keyring::unlock(&state, channel, owner, &identity_for(owner));
        assert!(matches!(key_task(&state, channel, owner, &ring), Some(KeyTask::Rotate { .. })));
        let ring = settle(&mut state, channel, owner);
        let (new_key, secret) = ring.sending_key(&state, channel).unwrap();
        assert_ne!(new_key, key);
        let mut after = Message::new(channel, owner, "after");
        seal_message(server, &mut after, new_key, secret);

        let left = Keyring::unlock(&state, channel, leaver, &identity_for(leaver));
        assert!(read_message(server, &after, &left).is_none());
        let stays = Keyring::unlock(&state, channel, stayer, &identity_for(stayer));
        assert_eq!(read_message(server, &after, &stays).unwrap().content, "after");
        assert_eq!(read_message(server, &before, &stays).unwrap().content, "before");

        let newcomer = join(&mut state, "newcomer");
        state.apply(ServerEvent::RoleAssign { server, user: newcomer, role: crew }).unwrap();
        settle(&mut state, channel, stayer);
        let fresh = Keyring::unlock(&state, channel, newcomer, &identity_for(newcomer));
        assert_eq!(read_message(server, &before, &fresh).unwrap().content, "before", "history reaches new readers");
    }

    #[test]
    fn concurrent_keys_merge_into_one() {
        let (mut state, channel, owner) = fixture();
        let server = state.server.id;
        private_channel(&mut state, channel);
        let mut ring = Keyring::default();
        let now = chrono::Utc::now().timestamp_millis();
        for at in [now, now + 1] {
            let record = ring.rotate(server, channel, &[], at);
            state.apply(ServerEvent::ChannelKeyAdd { server, channel, key: record }).unwrap();
        }
        for event in ring.grant_events(&state, channel, state.channel_keys(channel).unwrap().keys[0].id, &[owner]) {
            state.apply(event).unwrap();
        }
        let first = state.channel_keys(channel).unwrap().keys[0].id;
        let second = state.channel_keys(channel).unwrap().keys[1].id;
        let mut both = Keyring::default();
        both.secrets.insert(first, ring.secret(first).unwrap().clone());
        both.secrets.insert(second, ring.secret(second).unwrap().clone());
        assert_eq!(key_task(&state, channel, owner, &both), Some(KeyTask::Rotate { links: vec![first, second] }));
        run(&mut state, channel, owner, &mut both);
        assert_eq!(state.channel_keys(channel).unwrap().heads().len(), 1);
        let opened = Keyring::unlock(&state, channel, owner, &identity_for(owner));
        assert!(opened.holds(first) && opened.holds(second));
    }

    #[test]
    fn only_role_holders_post_react_or_join_voice_in_locked_channels() {
        let (mut state, channel, owner) = fixture();
        let server = state.server.id;
        let crew = private_channel(&mut state, channel);
        let member = join(&mut state, "member");
        let insider = join(&mut state, "insider");
        state.apply(ServerEvent::RoleAssign { server, user: insider, role: crew }).unwrap();
        assert!(!state.server.can_view(channel, member));
        assert!(state.server.can_view(channel, insider) && state.server.can_view(channel, owner));

        let theirs = post(&state, channel, member, "let me in");
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: theirs }), Err(crate::store::StoreError::NotAllowed));
        let ours = post(&state, channel, insider, "hello crew");
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: ours.clone() }), Ok(true));

        let react = ServerEvent::ReactionSet { channel, message: ours.id, emoji: "🔥".into(), user: member, active: true };
        assert!(!crate::action::authorized(&state.server, member, &react));
        let voice = ServerEvent::VoiceState { user: member, channel: Some(channel), muted: false, deafened: false, since: None };
        assert!(!crate::action::authorized(&state.server, member, &voice));
        let typing = ServerEvent::TypingStart { channel, user: insider };
        assert!(crate::action::authorized(&state.server, insider, &typing));
        let add = ServerEvent::ChannelKeyAdd { server, channel, key: Keyring::default().rotate(server, channel, &[], 1) };
        assert!(!crate::action::authorized(&state.server, member, &add));
        assert!(crate::action::authorized(&state.server, insider, &add));
    }

    #[test]
    fn send_roles_limit_posting_but_not_reading() {
        let (mut state, channel, _) = fixture();
        let server = state.server.id;
        let crew = private_channel(&mut state, channel);
        state.apply(ServerEvent::ChannelAccessSet { server, channel, view: vec![], send: vec![crew] }).unwrap();
        let member = join(&mut state, "member");
        assert!(state.server.can_view(channel, member));
        assert!(!state.server.can_post_in(channel, member));
        state.apply(ServerEvent::RoleAssign { server, user: member, role: crew }).unwrap();
        assert!(state.server.can_post_in(channel, member));
    }

    #[test]
    fn losing_sight_of_a_voice_channel_drops_you_from_it() {
        let (mut state, _, _) = fixture();
        let server = state.server.id;
        let voice_channel = state.server.channels[1].id;
        let crew = private_channel(&mut state, voice_channel);
        let member = join(&mut state, "member");
        state.apply(ServerEvent::RoleAssign { server, user: member, role: crew }).unwrap();
        let voice = ServerEvent::VoiceState { user: member, channel: Some(voice_channel), muted: false, deafened: false, since: None };
        state.apply(voice).unwrap();
        assert!(state.voice_states.contains_key(&member));
        state.apply(ServerEvent::RoleUnassign { server, user: member, role: crew }).unwrap();
        assert!(!state.voice_states.contains_key(&member));
    }

    #[test]
    fn keys_and_grants_survive_a_snapshot() {
        let (mut state, channel, owner) = fixture();
        private_channel(&mut state, channel);
        settle(&mut state, channel, owner);
        let restored = ServerState::from_snapshot(serde_json::from_str(&serde_json::to_string(&state.snapshot()).unwrap()).unwrap());
        assert_eq!(restored.channel_keys(channel), state.channel_keys(channel));
        assert!(Keyring::unlock(&restored, channel, owner, &identity_for(owner)).sending_key(&restored, channel).is_some());
    }

    #[test]
    fn a_grant_opens_only_for_its_recipient() {
        let (state, channel, owner) = fixture();
        let server = state.server.id;
        let other = UserId::new();
        let secret = ChannelSecret::random();
        let key = KeyId::random();
        let grant = SealedKey::seal(server, channel, owner, &identity_for(owner).public(), key, &secret).unwrap();
        assert_eq!(grant.open(server, channel, owner, &identity_for(owner)), Some(secret));
        assert_eq!(grant.open(server, channel, other, &identity_for(other)), None);
        assert_eq!(grant.open(server, ChannelId::new(), owner, &identity_for(owner)), None, "bound to the channel");
    }

    #[test]
    fn sealed_bodies_are_bound_to_their_message() {
        let (state, channel, owner) = fixture();
        let server = state.server.id;
        let mut ring = Keyring::default();
        let record = ring.rotate(server, channel, &[], 1);
        let secret = ring.secret(record.id).unwrap().clone();
        let mut message = post(&state, channel, owner, "hi");
        message.attachments.push(Attachment {
            blob: "b".into(),
            filename: "f.png".into(),
            size_bytes: 3,
            mime_type: "image/png".into(),
            provider: None,
        });
        seal_message(server, &mut message, record.id, &secret);
        assert!(message.attachments.is_empty());
        let opened = read_message(server, &message, &ring).unwrap();
        assert_eq!(opened.attachments[0].filename, "f.png");
        let moved = Message { id: MessageId::for_author(owner), ..message.clone() };
        assert!(read_message(server, &moved, &ring).is_none());
        let plain = post(&state, channel, owner, "open");
        assert_eq!(read_message(server, &plain, &Keyring::default()).unwrap().content, "open");
    }
}
