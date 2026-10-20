use std::collections::HashSet;

use chrono::Utc;
use commy_core::catchup::{apply_reaction_changes, digests, items, reaction_changes, BucketItems, ReactionChange};
use commy_core::{
    BucketDigest, BucketKey, Identity, IdentityKey, Message, MessageId, ServerId, TombstoneProof, UserId, UserProfile,
};
use commy_net::catchup::{
    apply_items, apply_profiles, check_peer, collect, profiles_of, ApplyReport, CatchupStore, PeerCheck,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::client::{Me, Servers};

pub struct WebStore {
    pub servers: Servers,
    pub me: Me,
    pub changed: UnboundedSender<ServerId>,
}

impl WebStore {
    fn with_state<T>(&self, server: ServerId, f: impl FnOnce(&commy_core::ServerState) -> T) -> Option<T> {
        self.servers.lock().unwrap().get(&server).map(f)
    }

    fn with_state_mut<T>(&self, server: ServerId, f: impl FnOnce(&mut commy_core::ServerState) -> T) -> Option<T> {
        self.servers.lock().unwrap().get_mut(&server).map(f)
    }
}

impl CatchupStore for WebStore {
    fn local_identity(&self, server: ServerId) -> Option<(UserId, Identity)> {
        let (profile, identity) = self.me.lock().unwrap().clone()?;
        self.with_state(server, |s| s.server.is_member(profile.id))?
            .then_some((profile.id, identity))
    }

    fn verify_peer(&self, server: ServerId, user: UserId, key: &IdentityKey) -> PeerCheck {
        self.with_state(server, |s| check_peer(s, user, key)).unwrap_or(PeerCheck::Denied)
    }

    fn digests(&self, server: ServerId) -> Vec<BucketDigest> {
        self.with_state(server, |s| digests(s, Utc::now())).unwrap_or_default()
    }

    fn items(&self, server: ServerId, keys: &[BucketKey]) -> Vec<BucketItems> {
        self.with_state(server, |s| items(s, keys, Utc::now())).unwrap_or_default()
    }

    fn get(&self, server: ServerId, ids: &[MessageId]) -> (Vec<Message>, Vec<TombstoneProof>) {
        self.with_state(server, |s| collect(s, ids)).unwrap_or_default()
    }

    fn profiles(&self, server: ServerId, users: &[UserId]) -> Vec<UserProfile> {
        self.with_state(server, |s| profiles_of(s, users)).unwrap_or_default()
    }

    fn apply_profiles(&self, server: ServerId, profiles: Vec<UserProfile>) {
        let learned = self.with_state_mut(server, |s| apply_profiles(s, profiles)).unwrap_or_default();
        if !learned.is_empty() {
            let _ = self.changed.send(server);
        }
    }

    fn apply(&self, server: ServerId, messages: Vec<Message>, tombstones: Vec<TombstoneProof>) -> ApplyReport {
        let report = self.with_state_mut(server, |s| apply_items(s, messages.clone(), tombstones.clone()).0);
        match report {
            Some(report) => {
                if !report.applied.is_empty() {
                    let _ = self.changed.send(server);
                }
                report
            }
            None => {
                let mut seen = HashSet::new();
                let rejected = messages
                    .iter()
                    .map(|m| m.id)
                    .chain(tombstones.iter().map(|p| p.message))
                    .filter(|id| seen.insert(*id))
                    .collect();
                ApplyReport { rejected, ..Default::default() }
            }
        }
    }

    fn reaction_changes(&self, server: ServerId, user: UserId) -> Vec<ReactionChange> {
        self.with_state(server, |s| reaction_changes(s, user, Utc::now())).unwrap_or_default()
    }

    fn apply_reaction_changes(&self, server: ServerId, user: UserId, changes: Vec<ReactionChange>) -> usize {
        let changed = self
            .with_state_mut(server, |s| apply_reaction_changes(s, user, changes, Utc::now()))
            .unwrap_or(0);
        if changed > 0 {
            let _ = self.changed.send(server);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Server, ServerState};
    use std::sync::{Arc, Mutex};

    #[test]
    fn answers_only_for_servers_we_belong_to() {
        let identity = Identity::generate();
        let mut me = UserProfile::new("me");
        me.identity = Some(identity.public());
        let server = Server::new("s", me.id);
        let id = server.id;
        let mut state = ServerState::new(server);
        state.profiles.insert(me.id, me.clone());
        let (changed, _rx) = tokio::sync::mpsc::unbounded_channel();
        let store = WebStore {
            servers: Arc::new(Mutex::new([(id, state)].into())),
            me: Arc::new(Mutex::new(Some((me.clone(), identity)))),
            changed,
        };
        assert_eq!(store.local_identity(id).map(|(user, _)| user), Some(me.id));
        assert!(store.local_identity(ServerId::new()).is_none());
        assert_eq!(store.verify_peer(id, me.id, &me.identity.unwrap()), PeerCheck::Allowed);
        let channel = store.servers.lock().unwrap()[&id].server.channels[0].id;
        let mut message = Message::new(channel, me.id, "caught up");
        message.sign(id, &store.me.lock().unwrap().as_ref().unwrap().1);
        let report = store.apply(id, vec![message.clone()], vec![]);
        assert_eq!(report.applied, vec![message.id]);
    }
}
