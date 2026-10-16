//! Test helpers: servers whose members hold identity keys, and an
//! in-memory catch-up store.

use crate::catchup::{apply_items, check_peer, collect, ApplyReport, CatchupStore, PeerCheck};
use chrono::{DateTime, Utc};
use commy_core::catchup::{digests, items, BucketItems};
use commy_core::{
    BucketDigest, BucketKey, ChannelId, Identity, IdentityKey, Message, MessageId, Server,
    ServerEvent, ServerId, ServerState, TombstoneProof, UserId, UserProfile,
};
use std::sync::Mutex;

/// Deterministic identity per user, so tests can sign as anyone.
pub fn identity_for(user: UserId) -> Identity {
    let mut secret = [0u8; 32];
    secret[..16].copy_from_slice(user.0.as_bytes());
    secret[16..].copy_from_slice(user.0.as_bytes());
    Identity::from_bytes(&secret)
}

/// New profile carrying its test identity key.
pub fn keyed_profile(username: &str) -> UserProfile {
    let mut profile = UserProfile::new(username);
    profile.identity = Some(identity_for(profile.id).public());
    profile
}

/// Server whose owner has a keyed profile, plus its text channel.
pub fn fixture() -> (ServerState, ChannelId, UserId) {
    let owner = keyed_profile("owner");
    let owner_id = owner.id;
    let server = Server::new("test", owner_id);
    let channel = server.channels[0].id;
    let mut state = ServerState::new(server);
    state.profiles.insert(owner_id, owner);
    (state, channel, owner_id)
}

/// Adds a keyed member and returns their id.
pub fn join(state: &mut ServerState, username: &str) -> UserId {
    let profile = keyed_profile(username);
    let id = profile.id;
    let server = state.server.id;
    state
        .apply(ServerEvent::MemberJoin { server, profile })
        .unwrap();
    id
}

/// New message from `author`, signed.
pub fn post(state: &ServerState, channel: ChannelId, author: UserId, content: &str) -> Message {
    post_at(state, channel, author, content, Utc::now())
}

/// Signed message created at `created_at`.
pub fn post_at(
    state: &ServerState,
    channel: ChannelId,
    author: UserId,
    content: &str,
    created_at: DateTime<Utc>,
) -> Message {
    let mut message = Message::new(channel, author, content);
    message.created_at = created_at;
    message.sign(state.server.id, &identity_for(author));
    message
}

/// One device's copy of a server, as the app would expose it.
pub struct MemStore {
    pub state: Mutex<ServerState>,
    pub me: UserId,
    pub identity: Identity,
    /// Events set aside for an unknown author key.
    pub held: Mutex<Vec<ServerEvent>>,
}

impl MemStore {
    pub fn new(state: ServerState, me: UserId) -> Self {
        Self {
            state: Mutex::new(state),
            me,
            identity: identity_for(me),
            held: Mutex::new(Vec::new()),
        }
    }

    pub fn server(&self) -> ServerId {
        self.state.lock().unwrap().server.id
    }

    pub fn with<T>(&self, f: impl FnOnce(&mut ServerState) -> T) -> T {
        f(&mut self.state.lock().unwrap())
    }

    /// Posts and stores a signed message as `author`.
    pub fn post_as(&self, author: UserId, channel: ChannelId, content: &str) -> Message {
        self.with(|s| {
            let m = post(s, channel, author, content);
            s.apply(ServerEvent::MessageCreate { message: m.clone() })
                .unwrap();
            m
        })
    }

    pub fn post(&self, channel: ChannelId, content: &str) -> Message {
        self.post_as(self.me, channel, content)
    }

    pub fn add(&self, message: &Message) {
        self.with(|s| {
            s.apply(ServerEvent::MessageCreate {
                message: message.clone(),
            })
            .unwrap()
        });
    }

    pub fn has(&self, id: MessageId) -> bool {
        self.with(|s| s.has_message(id))
    }

    pub fn content(&self, id: MessageId) -> Option<String> {
        self.with(|s| s.message(id).map(|m| m.content.clone()))
    }

    pub fn digests(&self) -> Vec<BucketDigest> {
        self.with(|s| digests(s, Utc::now()))
    }
}

impl CatchupStore for MemStore {
    fn local_identity(&self, server: ServerId) -> Option<(UserId, Identity)> {
        self.with(|s| {
            (s.server.id == server && s.server.is_member(self.me))
                .then(|| (self.me, self.identity.clone()))
        })
    }

    fn verify_peer(&self, server: ServerId, user: UserId, key: &IdentityKey) -> PeerCheck {
        self.with(|s| {
            if s.server.id == server {
                check_peer(s, user, key)
            } else {
                PeerCheck::Denied
            }
        })
    }

    fn digests(&self, _: ServerId) -> Vec<BucketDigest> {
        self.with(|s| digests(s, Utc::now()))
    }

    fn items(&self, _: ServerId, keys: &[BucketKey]) -> Vec<BucketItems> {
        self.with(|s| items(s, keys, Utc::now()))
    }

    fn get(&self, _: ServerId, ids: &[MessageId]) -> (Vec<Message>, Vec<TombstoneProof>) {
        self.with(|s| collect(s, ids))
    }

    fn apply(
        &self,
        _: ServerId,
        messages: Vec<Message>,
        tombstones: Vec<TombstoneProof>,
    ) -> ApplyReport {
        let (report, held) = self.with(|s| apply_items(s, messages, tombstones));
        self.held.lock().unwrap().extend(held);
        report
    }
}
