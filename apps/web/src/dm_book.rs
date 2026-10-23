use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use chrono::{DateTime, Utc};
use commy_core::dm::{DmAction, DmMessage};
use commy_core::{DmStore, Identity, IdentityKey, MessageId, ServerId, ServerState, UserId, UserProfile};
use commy_net::sync_streams;
use commy_net::DmEnvelope;
use commy_sync::{StreamId, StreamKeys};
use commy_view::dms::{self, EPOCHS};
use commy_view::safety::{self, DmFilter, FriendSignal, SafetySettings, Sender, Verdict};
use commy_view::{dm_excerpt, dm_message_view, dm_thread_views, DmMessageView, DmThreadView, UNKNOWN_USER};
use serde_json::{json, Value};

pub const PARTNER_RENOTE_MS: i64 = 30 * 24 * 60 * 60 * 1000;

type Servers = HashMap<ServerId, ServerState>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source {
    Pair(UserId, IdentityKey),
    Inbox(ServerId),
}

#[derive(Debug, PartialEq)]
pub enum Received {
    Dropped,
    Changed(UserId),
    Posted { partner: UserId, notice: Value },
    Friends { notice: Value, reply: Option<(UserId, FriendSignal)> },
}

#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    pub follow: Vec<(StreamKeys, u64)>,
    pub unfollow: Vec<StreamId>,
}

#[derive(Default)]
pub struct DmBook {
    pub store: DmStore,
    pub safety: SafetySettings,
    filter: DmFilter,
    noted: BTreeMap<UserId, i64>,
    noted_accepted: BTreeSet<UserId>,
    queued: HashSet<MessageId>,
    unstored: HashSet<MessageId>,
    routes: HashMap<StreamId, Source>,
    after: HashMap<StreamId, u64>,
    live: HashSet<StreamId>,
    dirty: bool,
}

pub fn known_name(store: &DmStore, servers: &Servers, user: UserId) -> String {
    store
        .contacts
        .get(&user)
        .or_else(|| servers.values().find_map(|s| s.profiles.get(&user)))
        .map(|p| p.visible_name().to_string())
        .unwrap_or_else(|| UNKNOWN_USER.to_string())
}

pub fn known_node(store: &DmStore, servers: &Servers, user: UserId) -> Option<String> {
    store
        .contacts
        .get(&user)
        .and_then(|p| p.node.clone())
        .or_else(|| servers.values().find_map(|s| s.profiles.get(&user).and_then(|p| p.node.clone())))
}

impl DmBook {
    pub fn new(safety: SafetySettings) -> Self {
        Self { safety, ..Default::default() }
    }

    pub fn note_partner(&mut self, me: UserId, profile: UserProfile, at: i64, accepted: bool) -> bool {
        if profile.identity.is_none() || profile.id == me {
            return false;
        }
        let partner = profile.id;
        let newer = self.noted.get(&partner).is_none_or(|seen| at > *seen);
        let newly_accepted = accepted && !self.noted_accepted.contains(&partner);
        if !newer && !newly_accepted {
            return false;
        }
        let seen = self.noted.entry(partner).or_insert(at);
        *seen = (*seen).max(at);
        if accepted {
            self.noted_accepted.insert(partner);
        }
        if !self.store.contacts.contains_key(&partner) {
            self.store.remember_contact(profile);
        }
        if self.noted_accepted.contains(&partner) && !self.safety.is_blocked(partner) {
            self.dirty |= self.safety.accepted.insert(partner);
        }
        true
    }

    pub fn should_note(&self, partner: UserId, accepted: bool, now: i64) -> bool {
        self.noted.get(&partner).is_none_or(|at| now - at > PARTNER_RENOTE_MS)
            || (accepted && !self.noted_accepted.contains(&partner))
    }

    pub fn talks_with(&self, me: UserId, partner: UserId) -> bool {
        !self.safety.is_blocked(partner)
            && (self.store.has_sent_to(me, partner) || self.safety.accepted.contains(&partner) || self.safety.friends.contains(&partner))
    }

    pub fn partner_key(&self, servers: &Servers, user: UserId) -> Option<IdentityKey> {
        self.store.pinned_key(user).or_else(|| dms::member_profile(servers, user).and_then(|p| p.identity))
    }

    pub fn partner_profile(&self, servers: &Servers, user: UserId) -> Option<UserProfile> {
        let known = self.store.contacts.get(&user).cloned();
        known.or_else(|| dms::member_profile(servers, user)).filter(|p| p.identity.is_some())
    }

    pub fn known_user(&self, servers: &Servers, user: UserId) -> bool {
        self.store.contacts.contains_key(&user)
            || servers.values().any(|s| s.profiles.contains_key(&user) || s.server.is_current_member(user))
    }

    pub fn plan(&mut self, identity: &Identity, servers: &Servers, inboxes: Vec<(ServerId, StreamKeys)>, now: DateTime<Utc>) -> Plan {
        let now_ms = now.timestamp_millis();
        let local: Vec<(UserId, i64)> = self
            .store
            .partners()
            .into_iter()
            .filter_map(|user| Some((user, self.store.conversation(user).last()?.created_at.timestamp_millis())))
            .collect();
        let epochs = sync_streams::recent_dm_epochs(now, EPOCHS);
        let mut wanted: HashMap<StreamId, (Source, StreamKeys)> = HashMap::new();
        for user in dms::pick_partners(&self.noted, &local, now_ms) {
            let Some(key) = self.partner_key(servers, user) else { continue };
            for epoch in &epochs {
                if let Some(keys) = sync_streams::dm_stream(identity, &key, epoch) {
                    wanted.insert(keys.id, (Source::Pair(user, key), keys));
                }
            }
        }
        for (server, keys) in inboxes {
            wanted.entry(keys.id).or_insert((Source::Inbox(server), keys));
        }
        let mut plan = Plan::default();
        let gone: Vec<StreamId> = self.routes.keys().filter(|id| !wanted.contains_key(id)).copied().collect();
        for stream in gone {
            self.routes.remove(&stream);
            self.live.remove(&stream);
            plan.unfollow.push(stream);
        }
        for (stream, (source, keys)) in wanted {
            if self.routes.get(&stream) == Some(&source) {
                continue;
            }
            self.routes.insert(stream, source);
            plan.follow.push((keys, self.after.get(&stream).copied().unwrap_or(0)));
        }
        plan
    }

    pub fn forget_streams(&mut self) {
        self.routes.clear();
        self.live.clear();
    }

    pub fn source(&self, stream: StreamId) -> Option<Source> {
        self.routes.get(&stream).copied()
    }

    pub fn advance(&mut self, stream: StreamId, seq: u64) {
        if self.routes.contains_key(&stream) {
            self.after.insert(stream, seq);
        }
    }

    pub fn caught_up(&mut self, stream: StreamId, seq: u64) -> bool {
        if !self.routes.contains_key(&stream) {
            return false;
        }
        self.after.insert(stream, seq);
        self.live.insert(stream);
        true
    }

    pub fn is_live(&self, stream: StreamId) -> bool {
        self.live.contains(&stream)
    }

    pub fn trusted(&self, servers: &Servers, envelope: &DmEnvelope) -> bool {
        let Some(key) = envelope.sender.identity.filter(|_| envelope.signed_by_sender()) else {
            return false;
        };
        let user = envelope.sender.id;
        if let Some(pinned) = self.store.pinned_key(user) {
            return key == pinned;
        }
        dms::key_matches_pins(key, dms::member_entries(servers, user).map(|p| p.identity))
    }

    pub fn receive(&mut self, me: UserId, servers: &Servers, envelope: DmEnvelope, node: [u8; 32], at_secs: i64) -> Received {
        if envelope.message.to != me || !envelope.sent_from(&node) || !self.trusted(servers, &envelope) {
            return Received::Dropped;
        }
        let mut sender = envelope.sender;
        sender.node = Some(data_encoding::HEXLOWER.encode(&node));
        let (note, message) = (envelope.note, envelope.message);
        let partner = sender.id;
        if message.content.starts_with(safety::REPORT_PREFIX) {
            return Received::Dropped;
        }
        if let Some(signal) = FriendSignal::decode(&message.content) {
            if !safety::note_friend_signal(&mut self.safety, partner, message.id, message.created_at) {
                return Received::Dropped;
            }
            let reply = safety::apply_friend_signal(&mut self.safety, partner, signal, note.as_deref());
            self.dirty = true;
            let name = sender.visible_name().to_string();
            self.store.remember_contact(sender);
            let note = self.safety.friend_notes.get(&partner).cloned();
            let notice = json!({ "user": partner.to_string(), "name": name, "signal": signal, "note": note });
            return Received::Friends { notice, reply: reply.map(|signal| (partner, signal)) };
        }
        if matches!(message.action, DmAction::Call { .. } | DmAction::GroupInvite { .. }) {
            return Received::Dropped;
        }
        if !message.is_post() {
            let applied = self.store.has_sent_to(me, partner) && self.store.apply(partner, message);
            return if applied { Received::Changed(partner) } else { Received::Dropped };
        }
        let from = Sender {
            blocked: self.safety.is_blocked(partner),
            accepted: self.store.has_sent_to(me, partner)
                || self.safety.accepted.contains(&partner)
                || self.safety.friends.contains(&partner),
            shares_server: servers.values().any(|s| s.server.is_member(partner)),
            node,
        };
        let verdict =
            self.filter.classify(self.safety.dm_policy, self.safety.spam_filter, &from, partner, &message.content, at_secs);
        match verdict {
            Verdict::Drop => return Received::Dropped,
            Verdict::Spam => {
                self.dirty |= self.safety.spam.insert(partner);
                return Received::Dropped;
            }
            Verdict::Request => {
                let members: HashSet<UserId> = servers.values().flat_map(|s| s.server.members.iter().map(|m| m.user)).collect();
                let known = |user: UserId| {
                    self.safety.accepted.contains(&user) || self.safety.friends.contains(&user) || members.contains(&user)
                };
                let open = safety::open_stranger_requests(&self.store, me, known);
                if !known(partner) && self.store.conversation(partner).is_empty() && open >= safety::MAX_STRANGER_REQUESTS {
                    return Received::Dropped;
                }
            }
            Verdict::Inbox => {}
        }
        let notice = json!({
            "user": partner.to_string(),
            "name": sender.visible_name(),
            "content": dm_excerpt(&message),
            "request": verdict != Verdict::Inbox,
            "spam": false,
        });
        self.store.remember_contact(sender);
        if self.store.apply(partner, message) {
            Received::Posted { partner, notice }
        } else {
            Received::Dropped
        }
    }

    pub fn take_mine(&mut self, partner: UserId, message: DmMessage) -> bool {
        self.store.apply(partner, message)
    }

    pub fn sent(&mut self, partner: UserId, message: DmMessage) -> bool {
        let id = message.id;
        if !self.store.apply(partner, message) {
            return false;
        }
        self.queued.insert(id);
        self.unstored.insert(id);
        if !self.safety.is_blocked(partner) {
            self.dirty |= self.safety.accepted.insert(partner);
        }
        true
    }

    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    pub fn stored(&mut self, id: MessageId) {
        self.unstored.remove(&id);
    }

    pub fn delivered(&mut self, id: MessageId) {
        self.queued.remove(&id);
        self.unstored.remove(&id);
    }

    pub fn follows(&self, partner: UserId, now_ms: i64) -> bool {
        dms::follows(&self.store, partner, &self.queued, now_ms)
    }

    fn waiting(&self, partner: UserId, now_ms: i64) -> &HashSet<MessageId> {
        if self.follows(partner, now_ms) {
            &self.unstored
        } else {
            &self.queued
        }
    }

    pub fn threads(&self, me: UserId) -> Vec<DmThreadView> {
        let mut views = dm_thread_views(&self.store, me);
        dms::sort_threads(&mut views, &self.store, &self.safety, me);
        views
    }

    pub fn messages(&self, me: UserId, partner: UserId, now_ms: i64) -> Vec<DmMessageView> {
        let waiting = self.waiting(partner, now_ms);
        self.store.posts(partner).map(|m| dm_message_view(m, me, &self.store, waiting.contains(&m.id))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Server, ServerEvent};

    fn person(name: &str) -> (UserProfile, Identity) {
        let identity = Identity::generate();
        let mut profile = UserProfile::new(name);
        profile.identity = Some(identity.public());
        (profile, identity)
    }

    const NODE: [u8; 32] = [9; 32];

    fn from(sender: &(UserProfile, Identity), to: UserId, text: &str) -> DmEnvelope {
        let mut message = DmMessage::new(sender.0.id, to, text);
        message.sign(&data_encoding::HEXLOWER.encode(&NODE), &sender.1);
        DmEnvelope::new(message, sender.0.clone())
    }

    fn shared(a: &UserProfile, b: &UserProfile) -> Servers {
        let server = Server::new("Moonbase", a.id);
        let sid = server.id;
        let mut state = ServerState::new(server);
        for profile in [a, b] {
            state.apply(ServerEvent::MemberJoin { server: sid, profile: profile.clone() }).unwrap();
        }
        HashMap::from([(sid, state)])
    }

    #[test]
    fn a_first_message_from_a_server_friend_lands_and_a_forged_one_does_not() {
        let me = person("me");
        let friend = person("friend");
        let servers = shared(&me.0, &friend.0);
        let mut book = DmBook::default();
        let at = Utc::now().timestamp();

        let hello = from(&friend, me.0.id, "hello there");
        match book.receive(me.0.id, &servers, hello, NODE, at) {
            Received::Posted { partner, notice } => {
                assert_eq!(partner, friend.0.id);
                assert_eq!(notice["request"], true, "a first message from someone we never answered is a request");
            }
            other => panic!("expected a post, got {other:?}"),
        }
        assert_eq!(book.threads(me.0.id)[0].kind, "request");

        let impostor = person("friend");
        let mut forged = from(&impostor, me.0.id, "send me your password");
        forged.sender.id = friend.0.id;
        forged.message.from = friend.0.id;
        assert_eq!(book.receive(me.0.id, &servers, forged, NODE, at), Received::Dropped);
        assert_eq!(book.store.posts(friend.0.id).count(), 1);
    }

    #[test]
    fn replying_accepts_the_conversation_and_blocking_hides_it() {
        let me = person("me");
        let friend = person("friend");
        let servers = shared(&me.0, &friend.0);
        let mut book = DmBook::default();
        book.receive(me.0.id, &servers, from(&friend, me.0.id, "hi"), NODE, Utc::now().timestamp());
        let reply = DmMessage::new(me.0.id, friend.0.id, "hey!");
        let id = reply.id;
        assert!(book.sent(friend.0.id, reply));
        assert_eq!(book.threads(me.0.id)[0].kind, "inbox");
        let now = Utc::now().timestamp_millis();
        assert!(book.follows(friend.0.id, now), "they wrote recently, so they read our conversation stream");
        assert!(book.messages(me.0.id, friend.0.id, now).iter().any(|m| m.id == id.to_string() && m.pending));
        book.stored(id);
        assert!(book.messages(me.0.id, friend.0.id, now).iter().all(|m| !m.pending));
        book.safety.set_blocked(friend.0.id, true);
        assert!(book.threads(me.0.id).is_empty());
    }

    #[test]
    fn a_partner_noted_by_another_device_is_followed_on_every_recent_month() {
        let me = person("me");
        let friend = person("friend");
        let mut book = DmBook::default();
        let now = Utc::now();
        assert!(book.note_partner(me.0.id, friend.0.clone(), now.timestamp_millis(), true));
        assert!(!book.note_partner(me.0.id, friend.0.clone(), now.timestamp_millis(), true), "an echo changes nothing");
        assert!(book.safety.accepted.contains(&friend.0.id));
        let plan = book.plan(&me.1, &HashMap::new(), Vec::new(), now);
        assert_eq!(plan.follow.len(), EPOCHS as usize);
        let stream = plan.follow[0].0.id;
        assert_eq!(book.source(stream), Some(Source::Pair(friend.0.id, friend.1.public())));
        let expected = sync_streams::dm_stream(&friend.1, &me.1.public(), &sync_streams::dm_epoch(now)).unwrap().id;
        assert!(plan.follow.iter().any(|(keys, _)| keys.id == expected), "both sides land on the same stream");
        assert_eq!(book.plan(&me.1, &HashMap::new(), Vec::new(), now), Plan::default());
    }

    #[test]
    fn edits_and_reactions_count_only_in_a_conversation_we_joined() {
        let me = person("me");
        let stranger = person("stranger");
        let servers = shared(&me.0, &stranger.0);
        let mut book = DmBook::default();
        let at = Utc::now().timestamp();
        let first = from(&stranger, me.0.id, "hi");
        let target = first.message.id;
        book.receive(me.0.id, &servers, first, NODE, at);
        let mut edit = DmMessage::acting(stranger.0.id, me.0.id, DmAction::Edit { target }, "hi again");
        edit.sign(&data_encoding::HEXLOWER.encode(&NODE), &stranger.1);
        let edit = DmEnvelope::new(edit, stranger.0.clone());
        assert_eq!(book.receive(me.0.id, &servers, edit.clone(), NODE, at), Received::Dropped);
        book.sent(stranger.0.id, DmMessage::new(me.0.id, stranger.0.id, "hello"));
        assert_eq!(book.receive(me.0.id, &servers, edit, NODE, at), Received::Changed(stranger.0.id));
        assert_eq!(book.store.post(stranger.0.id, target).unwrap().content, "hi again");
    }

    #[test]
    fn a_friend_request_waits_for_us_and_ours_crossing_theirs_makes_friends() {
        let me = person("me");
        let friend = person("friend");
        let servers = shared(&me.0, &friend.0);
        let mut book = DmBook::default();
        let at = Utc::now().timestamp();
        let mut request = from(&friend, me.0.id, &FriendSignal::Request.encode());
        request.note = Some("we met in voice".into());
        match book.receive(me.0.id, &servers, request, NODE, at) {
            Received::Friends { notice, reply } => {
                assert_eq!(notice["note"], "we met in voice");
                assert_eq!(reply, None);
            }
            other => panic!("expected a friend signal, got {other:?}"),
        }
        assert!(book.safety.friend_in.contains(&friend.0.id));
        assert!(book.store.conversation(friend.0.id).is_empty(), "a friend request is not a message");
        book.safety.friend_in.remove(&friend.0.id);
        book.safety.friend_out.insert(friend.0.id);
        let again = from(&friend, me.0.id, &FriendSignal::Request.encode());
        match book.receive(me.0.id, &servers, again, NODE, at) {
            Received::Friends { reply, .. } => assert_eq!(reply, Some((friend.0.id, FriendSignal::Accept))),
            other => panic!("expected a friend signal, got {other:?}"),
        }
        assert!(book.safety.friends.contains(&friend.0.id));
    }
}
