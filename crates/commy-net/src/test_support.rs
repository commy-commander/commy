use crate::catchup::{
    apply_items, apply_profiles, check_peer, collect, profiles_of, ApplyReport, CatchupStore,
    PeerCheck,
};
use chrono::Utc;
use commy_core::catchup::{apply_reaction_changes, digests, items, reaction_changes, BucketItems, ReactionChange};
pub use commy_core::test_support::{
    fixture, identity_for, join, join_moderator, keyed_profile, post, post_at, proof as proof_at,
};
use commy_core::{
    BucketDigest, BucketKey, ChannelId, Identity, IdentityKey, Message, MessageId, ServerEvent, ServerId,
    ServerState, TombstoneProof, UserId, UserProfile,
};
use std::sync::Mutex;

pub struct MemStore {
    pub state: Mutex<ServerState>,
    pub me: UserId,
    pub identity: Identity,
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

    fn profiles(&self, _: ServerId, users: &[UserId]) -> Vec<UserProfile> {
        self.with(|s| profiles_of(s, users))
    }

    fn apply_profiles(&self, _: ServerId, profiles: Vec<UserProfile>) {
        self.with(|s| apply_profiles(s, profiles));
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

    fn reaction_changes(&self, _: ServerId, user: UserId) -> Vec<ReactionChange> {
        self.with(|s| reaction_changes(s, user, Utc::now()))
    }

    fn apply_reaction_changes(&self, _: ServerId, user: UserId, changes: Vec<ReactionChange>) -> usize {
        self.with(|s| apply_reaction_changes(s, user, changes, Utc::now()))
    }
}

pub struct MultiStore(pub Vec<MemStore>);

impl MultiStore {
    fn of(&self, server: ServerId) -> Option<&MemStore> {
        self.0.iter().find(|s| s.server() == server)
    }
}

impl CatchupStore for MultiStore {
    fn local_identity(&self, server: ServerId) -> Option<(UserId, Identity)> {
        self.of(server)?.local_identity(server)
    }

    fn verify_peer(&self, server: ServerId, user: UserId, key: &IdentityKey) -> PeerCheck {
        self.of(server)
            .map_or(PeerCheck::Denied, |s| s.verify_peer(server, user, key))
    }

    fn digests(&self, server: ServerId) -> Vec<BucketDigest> {
        self.of(server)
            .map(|s| CatchupStore::digests(s, server))
            .unwrap_or_default()
    }

    fn items(&self, server: ServerId, keys: &[BucketKey]) -> Vec<BucketItems> {
        self.of(server)
            .map(|s| s.items(server, keys))
            .unwrap_or_default()
    }

    fn get(&self, server: ServerId, ids: &[MessageId]) -> (Vec<Message>, Vec<TombstoneProof>) {
        self.of(server)
            .map(|s| s.get(server, ids))
            .unwrap_or_default()
    }

    fn profiles(&self, server: ServerId, users: &[UserId]) -> Vec<UserProfile> {
        self.of(server)
            .map(|s| s.profiles(server, users))
            .unwrap_or_default()
    }

    fn apply_profiles(&self, server: ServerId, profiles: Vec<UserProfile>) {
        if let Some(s) = self.of(server) {
            s.apply_profiles(server, profiles);
        }
    }

    fn apply(
        &self,
        server: ServerId,
        messages: Vec<Message>,
        tombstones: Vec<TombstoneProof>,
    ) -> ApplyReport {
        self.of(server)
            .map(|s| s.apply(server, messages, tombstones))
            .unwrap_or_default()
    }

    fn reaction_changes(&self, server: ServerId, user: UserId) -> Vec<ReactionChange> {
        self.of(server)
            .map(|s| s.reaction_changes(server, user))
            .unwrap_or_default()
    }

    fn apply_reaction_changes(&self, server: ServerId, user: UserId, changes: Vec<ReactionChange>) -> usize {
        self.of(server)
            .map_or(0, |s| s.apply_reaction_changes(server, user, changes))
    }
}
