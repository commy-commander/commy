use crate::event::ServerEvent;
use crate::identity::{to_hex, Identity, IdentityKey};
use crate::ids::{ChannelId, MessageId, ServerId, UserId};
use crate::message::{Attachment, Message};
use crate::permissions::Permissions;
use crate::server::Server;
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
const COMMUNITY_GRANT_DOMAIN: &str = "commy-community-grant/v1";
const COMMUNITY_LINK_DOMAIN: &str = "commy-community-link/v1";
const BODY_DOMAIN: &str = "commy-channel-body/v1";
const SEALED_PREFIX: &str = "commy-sealed:v1:";
const SERVER_TEXT_DOMAIN: &str = "commy-server-text/v1";
pub const MAX_SEALED_TEXT_BYTES: usize = 1024;
const LABEL_DOMAIN: &str = "commy-channel-label/v1";
const DRIVE_TEXT_DOMAIN: &str = "commy-drive-text/v1";
pub const MAX_SEALED_LABEL_BYTES: usize = 1024;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyScope {
    Channel(ChannelId),
    Community,
}

impl From<ChannelId> for KeyScope {
    fn from(channel: ChannelId) -> Self {
        KeyScope::Channel(channel)
    }
}

impl KeyScope {
    fn grant_info(self, server: ServerId, user: UserId, key: KeyId, ephemeral: [u8; 32], point: [u8; 32]) -> Vec<u8> {
        match self {
            KeyScope::Channel(channel) => encode(&(GRANT_DOMAIN, server, channel, user, key, ephemeral, point)),
            KeyScope::Community => encode(&(COMMUNITY_GRANT_DOMAIN, server, user, key, ephemeral, point)),
        }
    }

    fn link_aad(self, server: ServerId, from: KeyId, prev: KeyId) -> Vec<u8> {
        match self {
            KeyScope::Channel(channel) => encode(&(LINK_DOMAIN, server, channel, from, prev)),
            KeyScope::Community => encode(&(COMMUNITY_LINK_DOMAIN, server, from, prev)),
        }
    }

    fn add_event(self, server: ServerId, key: KeyRecord) -> ServerEvent {
        match self {
            KeyScope::Channel(channel) => ServerEvent::ChannelKeyAdd { server, channel, key },
            KeyScope::Community => ServerEvent::CommunityKeyAdd { server, key },
        }
    }

    fn grant_subject(self, user: UserId) -> String {
        match self {
            KeyScope::Channel(channel) => format!("channel.grant:{channel}:{user}"),
            KeyScope::Community => format!("community.grant:{user}"),
        }
    }

    fn grant_event(self, server: ServerId, user: UserId, grant: SealedKey) -> ServerEvent {
        match self {
            KeyScope::Channel(channel) => ServerEvent::ChannelKeyGrant { server, channel, user, grant },
            KeyScope::Community => ServerEvent::CommunityKeyGrant { server, user, grant },
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ChannelSecret([u8; 32]);

impl ChannelSecret {
    pub fn random() -> Self {
        Self(rand::random())
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn bytes(&self) -> &[u8; 32] {
        &self.0
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
    let mut extract = <Hmac<Sha256> as KeyInit>::new_from_slice(shared).expect("any key length");
    extract.update(GRANT_DOMAIN.as_bytes());
    let prk = extract.finalize().into_bytes();
    let mut expand = <Hmac<Sha256> as KeyInit>::new_from_slice(&prk).expect("any key length");
    expand.update(info);
    expand.update(&[1]);
    expand.finalize().into_bytes().into()
}

impl SealedKey {
    pub fn seal(
        server: ServerId,
        scope: impl Into<KeyScope>,
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
        let wrap = wrapping_key(&shared, &scope.into().grant_info(server, user, key, ephemeral, point));
        let nonce = rand::random();
        Some(Self { key, ephemeral, nonce, sealed: seal(&wrap, nonce, &[], &secret.0) })
    }

    pub fn open(&self, server: ServerId, scope: impl Into<KeyScope>, me: UserId, identity: &Identity) -> Option<ChannelSecret> {
        let point = identity.public().agreement_point()?;
        let shared = identity.agree(self.ephemeral)?;
        let wrap = wrapping_key(&shared, &scope.into().grant_info(server, me, self.key, self.ephemeral, point));
        unseal(&wrap, self.nonce, &[], &self.sealed).and_then(ChannelSecret::from_plain)
    }

    fn well_formed(&self) -> bool {
        self.sealed.len() == SEALED_SECRET_BYTES
    }

    fn released() -> Self {
        Self { key: KeyId::random(), ephemeral: [0; 32], nonce: [0; 12], sealed: vec![0; SEALED_SECRET_BYTES] }
    }
}

impl KeyLink {
    fn seal(server: ServerId, scope: KeyScope, from: KeyId, secret: &ChannelSecret, prev: KeyId, prev_secret: &ChannelSecret) -> Self {
        let nonce = rand::random();
        let sealed = seal(&secret.0, nonce, &scope.link_aad(server, from, prev), &prev_secret.0);
        Self { prev, nonce, sealed }
    }

    fn open(&self, server: ServerId, scope: KeyScope, from: KeyId, secret: &ChannelSecret) -> Option<ChannelSecret> {
        unseal(&secret.0, self.nonce, &scope.link_aad(server, from, self.prev), &self.sealed).and_then(ChannelSecret::from_plain)
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
        let heads = self.heads();
        let (rotated, first): (Vec<&KeyRecord>, Vec<&KeyRecord>) = self.keys.iter().filter(|k| heads.contains(&k.id)).partition(|k| !k.links.is_empty());
        let current = rotated.into_iter().max_by_key(|k| (k.at, k.id)).or_else(|| first.into_iter().max_by_key(|k| (k.at, k.id)));
        current.or_else(|| self.keys.iter().max_by_key(|k| (k.at, k.id)))
    }

    pub fn heads(&self) -> Vec<KeyId> {
        let linked: HashSet<KeyId> = self.keys.iter().flat_map(|k| k.links.iter().map(|l| l.prev)).collect();
        self.keys.iter().map(|k| k.id).filter(|id| !linked.contains(id)).collect()
    }

    fn opens_its_links(&self, server: ServerId, scope: KeyScope, id: KeyId, secret: &ChannelSecret) -> bool {
        self.record(id).is_none_or(|record| record.links.is_empty() || record.links.iter().any(|l| l.open(server, scope, id, secret).is_some()))
    }

    fn latest_grant_time(&self, state: &ServerState, scope: KeyScope) -> i64 {
        self.grants.iter().filter_map(|(user, _)| state.action_time(&scope.grant_subject(*user))).max().unwrap_or(i64::MIN)
    }

    fn exposed_to(&self, state: &ServerState, scope: KeyScope, cut_off: impl Fn(UserId) -> bool) -> bool {
        let heads = self.heads();
        let since = self.newest().map_or(i64::MAX, |k| k.at);
        self.grants.iter().any(|(user, g)| {
            let written = state.action_time(&scope.grant_subject(*user)).unwrap_or(i64::MIN);
            cut_off(*user) && (heads.contains(&g.key) || written >= since)
        })
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

    pub(crate) fn make_room(&mut self, room: usize, member: impl Fn(UserId) -> bool) {
        if self.grants.len() < room {
            return;
        }
        let heads = self.heads();
        self.grants.retain(|(user, g)| member(*user) || heads.contains(&g.key));
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
        Self::unlock_scope(state, KeyScope::Channel(channel), me, identity)
    }

    pub fn unlock_community(state: &ServerState, me: UserId, identity: &Identity) -> Self {
        Self::unlock_scope(state, KeyScope::Community, me, identity)
    }

    pub fn unlock_scope(state: &ServerState, scope: KeyScope, me: UserId, identity: &Identity) -> Self {
        let mut ring = Self::default();
        let Some(keys) = state.key_chain(scope) else { return ring };
        let server = state.server.id;
        if let Some(grant) = keys.grant_for(me) {
            if let Some(secret) = grant.open(server, scope, me, identity).filter(|s| keys.opens_its_links(server, scope, grant.key, s)) {
                ring.secrets.insert(grant.key, secret);
            }
        }
        ring.follow_links(server, scope, keys);
        ring
    }

    pub fn admit(&mut self, state: &ServerState, scope: KeyScope, id: KeyId, secret: ChannelSecret) {
        if self.secrets.contains_key(&id) {
            return;
        }
        self.secrets.insert(id, secret);
        if let Some(keys) = state.key_chain(scope) {
            self.follow_links(state.server.id, scope, keys);
        }
    }

    fn follow_links(&mut self, server: ServerId, scope: KeyScope, keys: &ChannelKeys) {
        let mut unvisited: Vec<KeyId> = self.secrets.keys().copied().collect();
        while let Some(id) = unvisited.pop() {
            let (Some(record), Some(secret)) = (keys.record(id), self.secrets.get(&id).cloned()) else { continue };
            for link in &record.links {
                if self.secrets.contains_key(&link.prev) {
                    continue;
                }
                if let Some(prev) = link.open(server, scope, id, &secret) {
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
        self.newest_in(state, KeyScope::Channel(channel))
    }

    pub fn newest_in(&self, state: &ServerState, scope: KeyScope) -> Option<(KeyId, &ChannelSecret)> {
        let newest = state.key_chain(scope)?.newest()?.id;
        self.secret(newest).map(|secret| (newest, secret))
    }

    pub fn held_records<'a>(&'a self, state: &'a ServerState, scope: KeyScope) -> Vec<(&'a KeyRecord, &'a ChannelSecret)> {
        let Some(keys) = state.key_chain(scope) else { return Vec::new() };
        let heads = keys.heads();
        let mut held: Vec<(&KeyRecord, &ChannelSecret)> =
            keys.keys.iter().filter_map(|record| Some((record, self.secret(record.id)?))).collect();
        held.sort_by_key(|(record, _)| std::cmp::Reverse((record.at, heads.contains(&record.id), record.id)));
        held
    }

    pub fn rotate(&mut self, server: ServerId, scope: impl Into<KeyScope>, links: &[KeyId], at: i64) -> KeyRecord {
        let scope = scope.into();
        let id = KeyId::random();
        let secret = ChannelSecret::random();
        let links = links
            .iter()
            .take(MAX_LINKS)
            .filter_map(|prev| self.secret(*prev).map(|s| KeyLink::seal(server, scope, id, &secret, *prev, s)))
            .collect();
        self.secrets.insert(id, secret);
        KeyRecord { id, at, links }
    }

    pub fn grant_events(&self, state: &ServerState, scope: impl Into<KeyScope>, key: KeyId, to: &[UserId]) -> Vec<ServerEvent> {
        let scope = scope.into();
        let Some(secret) = self.secret(key) else { return Vec::new() };
        let server = state.server.id;
        to.iter()
            .filter_map(|user| {
                let recipient = state.identity_of(*user)?;
                let grant = SealedKey::seal(server, scope, *user, &recipient, key, secret)?;
                Some(scope.grant_event(server, *user, grant))
            })
            .collect()
    }

    pub fn task_events(&mut self, state: &ServerState, scope: KeyScope, task: KeyTask, at: i64) -> Vec<ServerEvent> {
        let server = state.server.id;
        match task {
            KeyTask::Rotate { links } => {
                let record = self.rotate(server, scope, &links, rotation_time(state, scope, at));
                let key = record.id;
                let mut events = vec![scope.add_event(server, record)];
                events.extend(self.grant_events(state, scope, key, &recipients(state, scope)));
                events
            }
            KeyTask::Grant { key, to } => self.grant_events(state, scope, key, &to),
            KeyTask::Release { me } => vec![scope.grant_event(server, me, SealedKey::released())],
        }
    }
}

pub fn rotation_time(state: &ServerState, scope: KeyScope, now: i64) -> i64 {
    let after_grants = state.key_chain(scope).map_or(i64::MIN, |keys| keys.latest_grant_time(state, scope)).saturating_add(1);
    now.max(after_grants)
}

fn recipients(state: &ServerState, scope: KeyScope) -> Vec<UserId> {
    match scope {
        KeyScope::Channel(channel) => state.server.readers_of(channel),
        KeyScope::Community => state.server.members.iter().map(|m| m.user).filter(|u| state.server.is_current_member(*u)).collect(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyTask {
    Rotate { links: Vec<KeyId> },
    Grant { key: KeyId, to: Vec<UserId> },
    Release { me: UserId },
}

fn unusable_grant(keys: &ChannelKeys, me: UserId, ring: &Keyring) -> bool {
    keys.grant_for(me).is_some_and(|g| keys.record(g.key).is_some() && !ring.holds(g.key))
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
    let exposed = keys.exposed_to(state, KeyScope::Channel(channel), |user| !reader(user));
    let forked = heads.len() > 1 && held.len() == heads.len();
    if (exposed || forked) && !held.is_empty() {
        return Some(KeyTask::Rotate { links: held });
    }
    if unusable_grant(keys, me, ring) {
        return Some(KeyTask::Release { me });
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

pub fn may_rotate_community(server: &Server, user: UserId) -> bool {
    let perms = server.permissions_for(user);
    server.is_current_member(user)
        && (user == server.owner
            || perms.allows(Permissions::MANAGE_SERVER)
            || perms.allows(Permissions::BAN_MEMBERS)
            || perms.allows(Permissions::KICK_MEMBERS))
}

pub(crate) fn community_closed_to(server: &Server, user: UserId) -> bool {
    if server.banned.contains(&user) {
        return true;
    }
    let gone = !server.is_member(user);
    let kicked = server.kicked.iter().any(|kick| kick.user == user);
    gone && (kicked || server.join_approval || !server.public)
}

pub fn community_key_task(state: &ServerState, me: UserId, ring: &Keyring) -> Option<KeyTask> {
    let server = &state.server;
    if !server.is_current_member(me) {
        return None;
    }
    let rotates = may_rotate_community(server, me);
    let keys = state.community_keys();
    let heads = keys.heads();
    if heads.is_empty() {
        return (rotates && !server.channels.is_empty()).then_some(KeyTask::Rotate { links: Vec::new() });
    }
    let held: Vec<KeyId> = heads.iter().copied().filter(|k| ring.holds(*k)).collect();
    let exposed = keys.exposed_to(state, KeyScope::Community, |user| community_closed_to(server, user));
    let forked = heads.len() > 1 && held.len() == heads.len();
    let shared_before_approval = server.join_approval && keys.newest().is_some_and(|k| k.at < server.join_approval_since);
    if rotates && (exposed || forked || shared_before_approval) && !held.is_empty() {
        return Some(KeyTask::Rotate { links: held });
    }
    if unusable_grant(keys, me, ring) {
        return Some(KeyTask::Release { me });
    }
    let newest = keys.newest()?.id;
    if !ring.holds(newest) {
        return None;
    }
    let to: Vec<UserId> = recipients(state, KeyScope::Community)
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

fn label_aad(server: ServerId, channel: ChannelId, root: MessageId) -> Vec<u8> {
    encode(&(LABEL_DOMAIN, server, channel, root))
}

pub fn is_sealed_label(label: &str) -> bool {
    is_sealed(label) && label.len() <= MAX_SEALED_LABEL_BYTES
}

fn seal_bound(aad: &[u8], text: &str, key: KeyId, secret: &ChannelSecret) -> String {
    let nonce: [u8; 12] = rand::random();
    let mut bytes = Vec::with_capacity(28 + text.len() + TAG_BYTES);
    bytes.extend_from_slice(&key.0);
    bytes.extend_from_slice(&nonce);
    bytes.extend(seal(&secret.0, nonce, aad, text.as_bytes()));
    format!("{SEALED_PREFIX}{}", BASE64URL_NOPAD.encode(&bytes))
}

fn read_bound(aad: &[u8], text: &str, ring: &Keyring) -> Option<String> {
    let Some(encoded) = text.strip_prefix(SEALED_PREFIX) else {
        return Some(text.to_string());
    };
    let bytes = BASE64URL_NOPAD.decode(encoded.as_bytes()).ok()?;
    if bytes.len() < 28 + TAG_BYTES {
        return None;
    }
    let key = KeyId(bytes[..16].try_into().ok()?);
    let nonce: [u8; 12] = bytes[16..28].try_into().ok()?;
    let plain = unseal(&ring.secret(key)?.0, nonce, aad, &bytes[28..])?;
    String::from_utf8(plain).ok()
}

pub fn is_sealed_text(text: &str) -> bool {
    is_sealed(text) && text.len() <= MAX_SEALED_TEXT_BYTES
}

fn server_text_aad(server: ServerId, purpose: &str) -> Vec<u8> {
    encode(&(SERVER_TEXT_DOMAIN, server, purpose))
}

pub fn seal_server_text(server: ServerId, purpose: &str, text: &str, key: KeyId, secret: &ChannelSecret) -> String {
    let nonce: [u8; 12] = rand::random();
    let mut bytes = Vec::with_capacity(28 + text.len() + TAG_BYTES);
    bytes.extend_from_slice(&key.0);
    bytes.extend_from_slice(&nonce);
    bytes.extend(seal(&secret.0, nonce, &server_text_aad(server, purpose), text.as_bytes()));
    format!("{SEALED_PREFIX}{}", BASE64URL_NOPAD.encode(&bytes))
}

pub fn read_server_text(server: ServerId, purpose: &str, text: &str, ring: &Keyring) -> Option<String> {
    let Some(encoded) = text.strip_prefix(SEALED_PREFIX) else {
        return Some(text.to_string());
    };
    let bytes = BASE64URL_NOPAD.decode(encoded.as_bytes()).ok()?;
    if bytes.len() < 28 + TAG_BYTES {
        return None;
    }
    let key = KeyId(bytes[..16].try_into().ok()?);
    let nonce: [u8; 12] = bytes[16..28].try_into().ok()?;
    let plain = unseal(&ring.secret(key)?.0, nonce, &server_text_aad(server, purpose), &bytes[28..])?;
    String::from_utf8(plain).ok()
}

pub fn seal_label(server: ServerId, channel: ChannelId, root: MessageId, label: &str, key: KeyId, secret: &ChannelSecret) -> String {
    seal_bound(&label_aad(server, channel, root), label, key, secret)
}

pub fn read_label(server: ServerId, channel: ChannelId, root: MessageId, label: &str, ring: &Keyring) -> Option<String> {
    read_bound(&label_aad(server, channel, root), label, ring)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveField {
    Name,
    Mime,
}

impl DriveField {
    fn label(self) -> &'static str {
        match self {
            DriveField::Name => "name",
            DriveField::Mime => "mime",
        }
    }
}

fn drive_aad(server: ServerId, channel: ChannelId, item: crate::ids::DriveId, field: DriveField) -> Vec<u8> {
    encode(&(DRIVE_TEXT_DOMAIN, server, channel, item, field.label()))
}

pub fn seal_drive_text(server: ServerId, channel: ChannelId, item: crate::ids::DriveId, field: DriveField, text: &str, key: KeyId, secret: &ChannelSecret) -> String {
    seal_bound(&drive_aad(server, channel, item, field), text, key, secret)
}

pub fn read_drive_text(server: ServerId, channel: ChannelId, item: crate::ids::DriveId, field: DriveField, text: &str, ring: &Keyring) -> Option<String> {
    read_bound(&drive_aad(server, channel, item, field), text, ring)
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
            mentionable: false,
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
                let at = rotation_time(state, KeyScope::Channel(channel), chrono::Utc::now().timestamp_millis());
                let record = ring.rotate(server, channel, links, at);
                let id = record.id;
                state.apply(ServerEvent::ChannelKeyAdd { server, channel, key: record }).unwrap();
                (id, state.server.readers_of(channel))
            }
            KeyTask::Grant { key, to } => (*key, to.clone()),
            KeyTask::Release { .. } => {
                for event in ring.task_events(state, KeyScope::Channel(channel), task.clone(), chrono::Utc::now().timestamp_millis()) {
                    state.apply(event).unwrap();
                }
                return Some(task);
            }
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
    fn server_texts_open_for_members_holding_the_community_key() {
        let (mut state, _, owner) = fixture();
        let server = state.server.id;
        let member = join(&mut state, "member");
        while tend_community(&mut state, owner).is_some() {}
        let ring = Keyring::unlock_community(&state, owner, &identity_for(owner));
        let (key, secret) = ring.newest_in(&state, KeyScope::Community).unwrap();
        let sealed = seal_server_text(server, "gif-key", "abc-123", key, secret);
        assert!(is_sealed_text(&sealed) && !sealed.contains("abc-123"));
        let theirs = Keyring::unlock_community(&state, member, &identity_for(member));
        assert_eq!(read_server_text(server, "gif-key", &sealed, &theirs).as_deref(), Some("abc-123"));
        assert_eq!(read_server_text(server, "game-db-secret", &sealed, &theirs), None, "bound to its purpose");
        let outsider = Keyring::unlock_community(&state, UserId::new(), &Identity::generate());
        assert_eq!(read_server_text(server, "gif-key", &sealed, &outsider), None);
    }

    #[test]
    fn sealed_drive_text_opens_only_for_readers_and_only_for_its_item_and_field() {
        let (mut state, channel, owner) = fixture();
        let server = state.server.id;
        let crew = private_channel(&mut state, channel);
        let (reader, outsider) = (join(&mut state, "reader"), join(&mut state, "outsider"));
        state.apply(ServerEvent::RoleAssign { server, user: reader, role: crew }).unwrap();
        let ring = settle(&mut state, channel, owner);
        let (key, secret) = ring.sending_key(&state, channel).unwrap();
        let item = crate::ids::DriveId::new();
        let sealed = seal_drive_text(server, channel, item, DriveField::Name, "Q3 plans.pdf", key, secret);
        assert!(is_sealed_label(&sealed) && !sealed.contains("Q3 plans.pdf"));
        let reader_ring = Keyring::unlock(&state, channel, reader, &identity_for(reader));
        assert_eq!(read_drive_text(server, channel, item, DriveField::Name, &sealed, &reader_ring).as_deref(), Some("Q3 plans.pdf"));
        let outsider_ring = Keyring::unlock(&state, channel, outsider, &identity_for(outsider));
        assert_eq!(read_drive_text(server, channel, item, DriveField::Name, &sealed, &outsider_ring), None);
        assert_eq!(read_drive_text(server, channel, crate::ids::DriveId::new(), DriveField::Name, &sealed, &reader_ring), None, "bound to its item");
        assert_eq!(read_drive_text(server, channel, item, DriveField::Mime, &sealed, &reader_ring), None, "bound to its field");
    }

    #[test]
    fn sealed_labels_open_only_for_readers_and_only_for_their_thread() {
        let (mut state, channel, owner) = fixture();
        let server = state.server.id;
        let crew = private_channel(&mut state, channel);
        let (reader, outsider) = (join(&mut state, "reader"), join(&mut state, "outsider"));
        state.apply(ServerEvent::RoleAssign { server, user: reader, role: crew }).unwrap();
        let ring = settle(&mut state, channel, owner);
        let (key, secret) = ring.sending_key(&state, channel).unwrap();
        let root = MessageId::new();
        let sealed = seal_label(server, channel, root, "Launch plans", key, secret);
        assert!(is_sealed_label(&sealed) && !sealed.contains("Launch"));
        let reader_ring = Keyring::unlock(&state, channel, reader, &identity_for(reader));
        assert_eq!(read_label(server, channel, root, &sealed, &reader_ring).as_deref(), Some("Launch plans"));
        let outsider_ring = Keyring::unlock(&state, channel, outsider, &identity_for(outsider));
        assert_eq!(read_label(server, channel, root, &sealed, &outsider_ring), None);
        assert_eq!(read_label(server, channel, MessageId::new(), &sealed, &reader_ring), None, "bound to its thread");
        assert_eq!(read_label(server, channel, root, "plain", &outsider_ring).as_deref(), Some("plain"));
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

    fn self_grant(state: &ServerState, channel: ChannelId, user: UserId, key: KeyId, at: chrono::DateTime<chrono::Utc>) -> crate::action::SignedEvent {
        let grant = SealedKey { key, ..SealedKey::released() };
        crate::test_support::signed_event(state, user, at, ServerEvent::ChannelKeyGrant { server: state.server.id, channel, user, grant })
    }

    #[test]
    fn a_key_added_without_links_never_outranks_the_rotated_chain() {
        let server = ServerId::new();
        let mut ring = Keyring::default();
        let mut keys = ChannelKeys::default();
        let first = ring.rotate(server, KeyScope::Community, &[], 1);
        keys.add_key(first.clone());
        let second = ring.rotate(server, KeyScope::Community, &[first.id], 2);
        keys.add_key(second.clone());
        let stray = Keyring::default().rotate(server, KeyScope::Community, &[], 3);
        keys.add_key(stray.clone());
        assert_eq!(keys.heads().len(), 2);
        assert_eq!(keys.newest().map(|k| k.id), Some(second.id), "a device that saw no keys made the stray one, and no other member can open it");
        let mut placeholder = crate::ServerState::new(crate::Server::awaiting_copy("", UserId::new()));
        let me = placeholder.server.owner;
        placeholder.server.id = server;
        assert_eq!(community_key_task(&placeholder, me, &Keyring::default()), None, "an empty copy waiting for its owner's never starts a key chain of its own");
        let made = crate::ServerState::new(crate::Server::new("made here", me));
        assert!(matches!(community_key_task(&made, me, &Keyring::default()), Some(KeyTask::Rotate { .. })));
        let mut alone = ChannelKeys::default();
        alone.add_key(first.clone());
        assert_eq!(alone.newest().map(|k| k.id), Some(first.id));
    }

    #[test]
    fn rewriting_your_own_grant_does_not_stop_the_rotation_when_you_lose_access() {
        for point_at_old_key in [false, true] {
            let (mut state, channel, owner) = fixture();
            let server = state.server.id;
            let crew = private_channel(&mut state, channel);
            let leaver = join(&mut state, "leaver");
            state.apply(ServerEvent::RoleAssign { server, user: leaver, role: crew }).unwrap();
            settle(&mut state, channel, owner);
            let old = state.channel_keys(channel).unwrap().newest().unwrap().id;
            state.apply(ServerEvent::ChannelKeyAdd { server, channel, key: Keyring::default().rotate(server, channel, &[], 1) }).unwrap();
            let ring = settle(&mut state, channel, owner);
            let (current, _) = ring.sending_key(&state, channel).unwrap();
            let pointed = if point_at_old_key { old } else { KeyId::random() };
            let now = chrono::Utc::now();
            assert_eq!(state.apply_signed(&self_grant(&state, channel, leaver, pointed, now + chrono::Duration::seconds(60))), Ok(true));
            state.apply(ServerEvent::RoleUnassign { server, user: leaver, role: crew }).unwrap();
            let ring = Keyring::unlock(&state, channel, owner, &identity_for(owner));
            assert!(matches!(key_task(&state, channel, owner, &ring), Some(KeyTask::Rotate { .. })), "old key {point_at_old_key}");
            let ring = settle(&mut state, channel, owner);
            assert_ne!(ring.sending_key(&state, channel).unwrap().0, current);
        }
    }

    #[test]
    fn grants_for_people_who_are_not_members_take_no_slot() {
        let (mut state, channel, owner) = fixture();
        let server = state.server.id;
        private_channel(&mut state, channel);
        settle(&mut state, channel, owner);
        let stranger = UserId::new();
        let grant = SealedKey::released();
        assert_eq!(state.apply(ServerEvent::ChannelKeyGrant { server, channel, user: stranger, grant: grant.clone() }), Ok(false));
        assert_eq!(state.apply(ServerEvent::CommunityKeyGrant { server, user: stranger, grant }), Ok(false));
        assert!(state.channel_keys(channel).unwrap().grant_for(stranger).is_none());
    }

    #[test]
    fn grants_dated_far_ahead_are_refused() {
        let (mut state, channel, owner) = fixture();
        let crew = private_channel(&mut state, channel);
        let reader = join(&mut state, "reader");
        state.apply(ServerEvent::RoleAssign { server: state.server.id, user: reader, role: crew }).unwrap();
        settle(&mut state, channel, owner);
        let far = chrono::Utc::now() + chrono::Duration::milliseconds(crate::store::MAX_GRANT_AHEAD_MS + 60_000);
        assert_eq!(state.apply_signed(&self_grant(&state, channel, reader, KeyId::random(), far)), Err(crate::store::StoreError::TooFarAhead));
    }

    #[test]
    fn a_reader_handed_a_wrong_key_releases_it_and_gets_the_real_one() {
        let (mut state, channel, owner) = fixture();
        let server = state.server.id;
        let crew = private_channel(&mut state, channel);
        let (victim, spoiler) = (join(&mut state, "victim"), join(&mut state, "spoiler"));
        for user in [victim, spoiler] {
            state.apply(ServerEvent::RoleAssign { server, user, role: crew }).unwrap();
        }
        settle(&mut state, channel, owner);
        let first = state.channel_keys(channel).unwrap().newest().unwrap().id;
        let mut owner_ring = Keyring::unlock(&state, channel, owner, &identity_for(owner));
        let record = owner_ring.rotate(server, channel, &[first], chrono::Utc::now().timestamp_millis());
        state.apply(ServerEvent::ChannelKeyAdd { server, channel, key: record.clone() }).unwrap();
        let wrong = SealedKey::seal(server, KeyScope::Channel(channel), victim, &identity_for(victim).public(), record.id, &ChannelSecret::random()).unwrap();
        state.apply(ServerEvent::ChannelKeyGrant { server, channel, user: victim, grant: wrong }).unwrap();
        let ring = Keyring::unlock(&state, channel, victim, &identity_for(victim));
        assert!(!ring.holds(record.id), "a secret that does not open the key's links is not taken");
        assert_eq!(key_task(&state, channel, victim, &ring), Some(KeyTask::Release { me: victim }));
        settle(&mut state, channel, victim);
        assert_ne!(state.channel_keys(channel).unwrap().grant_for(victim).unwrap().key, record.id);
        for event in owner_ring.grant_events(&state, channel, record.id, &[victim]) {
            state.apply(event).unwrap();
        }
        let ring = Keyring::unlock(&state, channel, victim, &identity_for(victim));
        assert!(ring.holds(record.id), "the real grant comes through once the wrong one is released");
    }

    #[test]
    fn a_rotation_from_a_slow_clock_still_replaces_the_old_key() {
        let (mut state, channel, owner) = fixture();
        let server = state.server.id;
        private_channel(&mut state, channel);
        let mut ring = settle(&mut state, channel, owner);
        let old = state.channel_keys(channel).unwrap().newest().unwrap().clone();
        let record = ring.rotate(server, channel, &[old.id], old.at - 60_000);
        let new = record.id;
        state.apply(ServerEvent::ChannelKeyAdd { server, channel, key: record }).unwrap();
        assert_eq!(state.channel_keys(channel).unwrap().newest().unwrap().id, new);
        assert_eq!(ring.sending_key(&state, channel).unwrap().0, new);
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

    fn tend_community(state: &mut ServerState, me: UserId) -> Option<KeyTask> {
        tend_community_at(state, me, chrono::Utc::now().timestamp_millis())
    }

    fn tend_community_at(state: &mut ServerState, me: UserId, at: i64) -> Option<KeyTask> {
        let mut ring = Keyring::unlock_community(state, me, &identity_for(me));
        let task = community_key_task(state, me, &ring)?;
        for event in ring.task_events(state, KeyScope::Community, task.clone(), at) {
            assert!(crate::action::authorized(&state.server, me, &event), "{event:?}");
            state.apply(event).unwrap();
        }
        Some(task)
    }

    fn settle_community(state: &mut ServerState, me: UserId) -> Keyring {
        settle_community_at(state, me, chrono::Utc::now().timestamp_millis())
    }

    fn settle_community_at(state: &mut ServerState, me: UserId, at: i64) -> Keyring {
        for _ in 0..4 {
            if tend_community_at(state, me, at).is_none() {
                return Keyring::unlock_community(state, me, &identity_for(me));
            }
        }
        panic!("community key work did not settle");
    }

    fn community_secret(state: &ServerState, user: UserId) -> Option<KeyId> {
        Keyring::unlock_community(state, user, &identity_for(user)).newest_in(state, KeyScope::Community).map(|(id, _)| id)
    }

    #[test]
    fn the_owner_creates_one_community_key_and_every_member_gets_it() {
        let (mut state, _, owner) = fixture();
        let a = join(&mut state, "a");
        let b = join(&mut state, "b");
        assert_eq!(community_key_task(&state, a, &Keyring::default()), None, "plain members never start a chain");
        assert!(matches!(community_key_task(&state, owner, &Keyring::default()), Some(KeyTask::Rotate { .. })));
        settle_community(&mut state, owner);
        let newest = state.community_keys().newest().unwrap().id;
        for user in [owner, a, b] {
            assert_eq!(community_secret(&state, user), Some(newest));
        }
        let stranger = UserId::new();
        assert_eq!(community_secret(&state, stranger), None);
    }

    #[test]
    fn any_member_holding_the_key_grants_it_to_a_newcomer() {
        let (mut state, _, owner) = fixture();
        let member = join(&mut state, "member");
        settle_community(&mut state, owner);
        let newcomer = join(&mut state, "newcomer");
        let ring = Keyring::unlock_community(&state, member, &identity_for(member));
        assert_eq!(community_key_task(&state, member, &ring), Some(KeyTask::Grant { key: state.community_keys().newest().unwrap().id, to: vec![newcomer] }));
        settle_community(&mut state, member);
        assert!(community_secret(&state, newcomer).is_some());
    }

    fn sign_and_apply(state: &mut ServerState, me: UserId, events: Vec<ServerEvent>, at_of: impl Fn(&ServerState, &ServerEvent) -> i64) -> Vec<bool> {
        let server = state.server.id;
        events
            .into_iter()
            .map(|event| {
                let at = at_of(state, &event);
                let signed = crate::action::SignedEvent::sign(server, me, at, event, &identity_for(me));
                state.apply_signed(&signed).unwrap()
            })
            .collect()
    }

    #[test]
    fn a_new_grant_lands_even_when_the_previous_one_was_signed_with_a_clock_ahead() {
        let (mut state, _, owner) = fixture();
        let now = chrono::Utc::now().timestamp_millis();
        let ahead = now + 60_000;
        let mut ring = Keyring::unlock_community(&state, owner, &identity_for(owner));
        let first = community_key_task(&state, owner, &ring).unwrap();
        let events = ring.task_events(&state, KeyScope::Community, first, ahead);
        sign_and_apply(&mut state, owner, events, |_, _| ahead);
        let held = Keyring::unlock_community(&state, owner, &identity_for(owner)).newest_in(&state, KeyScope::Community).unwrap().0;

        let mut ring = Keyring::unlock_community(&state, owner, &identity_for(owner));
        let rotate = ring.task_events(&state, KeyScope::Community, KeyTask::Rotate { links: vec![held] }, now);
        let mut naive = state.clone();
        let landed = sign_and_apply(&mut naive, owner, rotate.clone(), |_, _| now);
        assert_eq!(landed, vec![true, false], "signed at the wall clock, the new grant loses to the one signed ahead");

        let landed = sign_and_apply(&mut state, owner, rotate, |state, event| state.signing_time(event, now));
        assert_eq!(landed, vec![true, true]);
        let newest = state.community_keys().newest().unwrap().id;
        assert_ne!(newest, held);
        assert_eq!(community_secret(&state, owner), Some(newest), "the owner holds the newest key");
    }

    #[test]
    fn signing_time_is_the_wall_clock_unless_the_same_subject_was_signed_later() {
        let (mut state, _, owner) = fixture();
        let server = state.server.id;
        let grant = ServerEvent::CommunityKeyGrant { server, user: owner, grant: SealedKey::released() };
        assert_eq!(state.signing_time(&grant, 1_000), 1_000);
        let event = ServerEvent::CommunityKeyGrant { server, user: owner, grant: SealedKey::released() };
        sign_and_apply(&mut state, owner, vec![event], |_, _| 5_000);
        assert_eq!(state.signing_time(&grant, 1_000), 5_001);
        assert_eq!(state.signing_time(&grant, 9_000), 9_000);
    }

    #[test]
    fn a_ban_rotates_the_community_key_and_keeps_history_for_the_rest() {
        let (mut state, _, owner) = fixture();
        let server = state.server.id;
        let banned = join(&mut state, "banned");
        let stays = join(&mut state, "stays");
        settle_community(&mut state, owner);
        let before = state.community_keys().newest().unwrap().id;
        state.apply(ServerEvent::MemberBan { server, user: banned }).unwrap();
        let ring = Keyring::unlock_community(&state, stays, &identity_for(stays));
        assert_eq!(community_key_task(&state, stays, &ring), None, "plain members wait for a moderator");
        let ring = Keyring::unlock_community(&state, owner, &identity_for(owner));
        assert_eq!(community_key_task(&state, owner, &ring), Some(KeyTask::Rotate { links: vec![before] }));
        settle_community(&mut state, owner);
        let after = state.community_keys().newest().unwrap().id;
        assert_ne!(after, before);
        assert_eq!(community_secret(&state, stays), Some(after));
        assert!(Keyring::unlock_community(&state, stays, &identity_for(stays)).holds(before), "earlier keys stay readable");
        assert_eq!(community_secret(&state, banned), None, "the banned member cannot open the new key");
        let regrant = ServerEvent::CommunityKeyGrant { server, user: banned, grant: state.community_keys().grant_for(stays).unwrap().clone() };
        assert!(!crate::action::authorized(&state.server, owner, &regrant));
        assert_eq!(state.apply(regrant), Ok(false));
    }

    fn rotates_after(state: &mut ServerState, owner: UserId, removal: impl FnOnce(&mut ServerState)) -> bool {
        settle_community(state, owner);
        let before = state.community_keys().newest().unwrap().id;
        removal(state);
        let ring = Keyring::unlock_community(state, owner, &identity_for(owner));
        community_key_task(state, owner, &ring) == Some(KeyTask::Rotate { links: vec![before] })
    }

    fn leave(state: &mut ServerState, user: UserId) {
        let server = state.server.id;
        state.apply(ServerEvent::MemberLeave { server, user }).unwrap();
    }

    #[test]
    fn leaving_an_open_public_server_keeps_its_community_key() {
        let (mut state, _, owner) = fixture();
        state.server.public = true;
        let leaver = join(&mut state, "leaver");
        assert!(!rotates_after(&mut state, owner, |state| leave(state, leaver)), "anyone may walk back in with the public link");
    }

    #[test]
    fn leaving_a_private_server_moves_it_to_a_fresh_community_key() {
        let (mut state, _, owner) = fixture();
        state.server.public = false;
        let leaver = join(&mut state, "leaver");
        let stays = join(&mut state, "stays");
        assert!(rotates_after(&mut state, owner, |state| leave(state, leaver)));
        settle_community(&mut state, owner);
        assert!(community_secret(&state, stays).is_some());
        assert_eq!(community_secret(&state, leaver), None, "the one who left cannot open the new key");
        assert_eq!(community_key_task(&state, owner, &Keyring::unlock_community(&state, owner, &identity_for(owner))), None, "one rotation settles it");
    }

    #[test]
    fn leaving_a_public_server_that_asks_newcomers_to_wait_moves_it_to_a_fresh_key() {
        let (mut state, _, owner) = fixture();
        state.server.public = true;
        let leaver = join(&mut state, "leaver");
        state.server.join_approval = true;
        state.server.join_approval_since = 1;
        assert!(rotates_after(&mut state, owner, |state| leave(state, leaver)));
    }

    #[test]
    fn a_kick_moves_even_a_public_server_to_a_fresh_community_key() {
        let (mut state, _, owner) = fixture();
        state.server.public = true;
        let kicked = join(&mut state, "kicked");
        let stays = join(&mut state, "stays");
        assert!(rotates_after(&mut state, owner, |state| {
            let server = state.server.id;
            let at = chrono::Utc::now().timestamp_millis();
            let kick = crate::action::SignedEvent::sign(server, owner, at, ServerEvent::MemberLeave { server, user: kicked }, &identity_for(owner));
            assert_eq!(state.apply_signed(&kick), Ok(true));
        }));
        settle_community(&mut state, owner);
        assert!(community_secret(&state, stays).is_some());
        assert_eq!(community_secret(&state, kicked), None, "the kicked member cannot open the new key");
    }

    #[test]
    fn only_moderators_add_community_keys_and_members_grant_them() {
        let (mut state, _, owner) = fixture();
        let server = state.server.id;
        let member = join(&mut state, "member");
        let add = ServerEvent::CommunityKeyAdd { server, key: Keyring::default().rotate(server, KeyScope::Community, &[], 1) };
        assert!(crate::action::authorized(&state.server, owner, &add));
        assert!(!crate::action::authorized(&state.server, member, &add));
        let kicker = join(&mut state, "kicker");
        let role = Role {
            id: RoleId::new(),
            name: "kick".into(),
            color: None,
            permissions: Permissions::KICK_MEMBERS,
            position: 1,
            hoist: false,
            self_assignable: false,
            mentionable: false,
            tag: None,
            banner: None,
        };
        let id = role.id;
        state.apply(ServerEvent::RoleCreate { server, role }).unwrap();
        state.apply(ServerEvent::RoleAssign { server, user: kicker, role: id }).unwrap();
        assert!(crate::action::authorized(&state.server, kicker, &add));
        let ring = settle_community(&mut state, owner);
        let key = state.community_keys().newest().unwrap().id;
        let grants = ring.grant_events(&state, KeyScope::Community, key, &[member]);
        assert!(crate::action::authorized(&state.server, member, &grants[0]));
        assert!(!crate::action::authorized(&state.server, UserId::new(), &grants[0]), "outsiders cannot hand out keys");
    }

    #[test]
    fn community_grants_and_channel_grants_are_kept_apart() {
        let (state, channel, owner) = fixture();
        let server = state.server.id;
        let key = KeyId::random();
        let secret = ChannelSecret::random();
        let grant = SealedKey::seal(server, KeyScope::Community, owner, &identity_for(owner).public(), key, &secret).unwrap();
        assert_eq!(grant.open(server, KeyScope::Community, owner, &identity_for(owner)), Some(secret.clone()));
        assert_eq!(grant.open(server, channel, owner, &identity_for(owner)), None);
        assert_eq!(grant.open(ServerId::new(), KeyScope::Community, owner, &identity_for(owner)), None);
    }

    #[test]
    fn a_key_from_an_invite_opens_the_history_behind_it() {
        let (mut state, _, owner) = fixture();
        let server = state.server.id;
        let banned = join(&mut state, "banned");
        settle_community(&mut state, owner);
        let first = state.community_keys().newest().unwrap().id;
        state.apply(ServerEvent::MemberBan { server, user: banned }).unwrap();
        let owner_ring = settle_community(&mut state, owner);
        let (newest, secret) = owner_ring.newest_in(&state, KeyScope::Community).map(|(id, s)| (id, s.clone())).unwrap();
        let mut joiner = Keyring::default();
        joiner.admit(&state, KeyScope::Community, newest, secret);
        assert!(joiner.holds(newest) && joiner.holds(first));
        let held: Vec<KeyId> = joiner.held_records(&state, KeyScope::Community).iter().map(|(r, _)| r.id).collect();
        assert_eq!(held, vec![newest, first], "newest first");
    }

    #[test]
    fn asking_new_people_to_wait_rotates_a_key_that_old_invite_links_carried() {
        let (mut state, _, owner) = fixture();
        let server = state.server.id;
        let member = join(&mut state, "member");
        settle_community_at(&mut state, owner, 10);
        let shared = state.community_keys().newest().unwrap().id;
        state.apply(ServerEvent::JoinApprovalSet { server, on: true, at: 20 }).unwrap();
        assert!(matches!(tend_community_at(&mut state, member, 30), None | Some(KeyTask::Grant { .. })), "plain members never rotate");
        assert!(matches!(tend_community_at(&mut state, owner, 30), Some(KeyTask::Rotate { .. })));
        settle_community_at(&mut state, owner, 30);
        let fresh = state.community_keys().newest().unwrap().id;
        assert_ne!(fresh, shared);
        assert_eq!(community_secret(&state, member), Some(fresh), "members get the new key and keep the old history");
        assert!(tend_community_at(&mut state, owner, 40).is_none());
    }

    #[test]
    fn keys_made_in_the_same_millisecond_still_list_newest_first() {
        for _ in 0..16 {
            let (mut state, _, owner) = fixture();
            let server = state.server.id;
            let banned = join(&mut state, "banned");
            settle_community_at(&mut state, owner, 1);
            let first = state.community_keys().newest().unwrap().id;
            state.apply(ServerEvent::MemberBan { server, user: banned }).unwrap();
            let ring = settle_community_at(&mut state, owner, 1);
            let newest = state.community_keys().newest().unwrap().id;
            assert_ne!(newest, first);
            let held: Vec<KeyId> = ring.held_records(&state, KeyScope::Community).iter().map(|(r, _)| r.id).collect();
            assert_eq!(held, vec![newest, first]);
        }
    }

    #[test]
    fn signed_community_key_events_apply_like_any_other_action() {
        let (mut state, _, owner) = fixture();
        let server = state.server.id;
        let member = join(&mut state, "member");
        let mut ring = Keyring::default();
        let task = community_key_task(&state, owner, &ring).unwrap();
        let at = chrono::Utc::now().timestamp_millis();
        for event in ring.task_events(&state, KeyScope::Community, task, at) {
            let signed = crate::action::SignedEvent::sign(server, owner, at, event, &identity_for(owner));
            assert_eq!(state.apply_signed(&signed), Ok(true));
        }
        assert!(community_secret(&state, member).is_some());
        let forged = crate::action::SignedEvent::sign(
            server,
            member,
            at + 1,
            ServerEvent::CommunityKeyAdd { server, key: Keyring::default().rotate(server, KeyScope::Community, &[], at + 1) },
            &identity_for(member),
        );
        assert!(state.apply_signed(&forged).is_err());
    }

    #[test]
    fn community_keys_survive_a_snapshot() {
        let (mut state, _, owner) = fixture();
        join(&mut state, "member");
        settle_community(&mut state, owner);
        let restored = ServerState::from_snapshot(serde_json::from_str(&serde_json::to_string(&state.snapshot()).unwrap()).unwrap());
        assert_eq!(restored.community_keys(), state.community_keys());
        assert!(community_secret(&restored, owner).is_some());
    }
}
