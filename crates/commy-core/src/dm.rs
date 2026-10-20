use crate::identity::{Identity, IdentityKey, Signature};
use crate::ids::{MessageId, UserId};
use crate::message::{Attachment, Reaction};
use crate::user::UserProfile;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DmAction {
    #[default]
    Post,
    Edit {
        target: MessageId,
    },
    Delete {
        target: MessageId,
    },
    React {
        target: MessageId,
        emoji: String,
        active: bool,
    },
}

impl DmAction {
    pub fn target(&self) -> Option<MessageId> {
        match self {
            DmAction::Post => None,
            DmAction::Edit { target } | DmAction::Delete { target } | DmAction::React { target, .. } => Some(*target),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DmMessage {
    pub id: MessageId,
    pub from: UserId,
    pub to: UserId,
    pub content: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub from_node: String,
    #[serde(default)]
    pub sig: Option<Signature>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub reply_to: Option<MessageId>,
    #[serde(default)]
    pub action: DmAction,
    #[serde(default)]
    pub edited_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub reactions: Vec<Reaction>,
}

const DM_DOMAIN: &str = "commy-dm/v1";
const DM_RICH_DOMAIN: &str = "commy-dm/v2";

impl DmMessage {
    pub fn new(from: UserId, to: UserId, content: impl Into<String>) -> Self {
        Self {
            id: MessageId::new(),
            from,
            to,
            content: content.into(),
            created_at: Utc::now(),
            from_node: String::new(),
            sig: None,
            attachments: Vec::new(),
            reply_to: None,
            action: DmAction::Post,
            edited_at: None,
            reactions: Vec::new(),
        }
    }

    pub fn acting(from: UserId, to: UserId, action: DmAction, content: impl Into<String>) -> Self {
        Self { action, ..Self::new(from, to, content) }
    }

    pub fn is_post(&self) -> bool {
        self.action == DmAction::Post
    }

    pub fn is_plain(&self) -> bool {
        self.is_post() && self.attachments.is_empty() && self.reply_to.is_none()
    }

    pub fn signing_bytes(&self) -> Vec<u8> {
        if self.is_plain() {
            let fields = (
                DM_DOMAIN,
                self.id,
                self.from,
                self.to,
                self.content.as_str(),
                self.created_at.timestamp_millis(),
                self.from_node.as_str(),
            );
            return postcard::to_stdvec(&fields).expect("postcard encodes plain tuples");
        }
        let attachments: Vec<(&str, &str, u64, &str, Option<&str>)> = self
            .attachments
            .iter()
            .map(|a| (a.blob.as_str(), a.filename.as_str(), a.size_bytes, a.mime_type.as_str(), a.provider.as_deref()))
            .collect();
        let fields = (
            DM_RICH_DOMAIN,
            self.id,
            self.from,
            self.to,
            self.content.as_str(),
            self.created_at.timestamp_millis(),
            self.from_node.as_str(),
            attachments,
            self.reply_to,
            &self.action,
        );
        postcard::to_stdvec(&fields).expect("postcard encodes plain tuples")
    }

    pub fn sign(&mut self, node: &str, identity: &Identity) {
        self.from_node = node.to_string();
        self.sig = Some(identity.sign(&self.signing_bytes()));
    }

    pub fn verify(&self, key: &IdentityKey) -> bool {
        self.sig
            .is_some_and(|sig| key.verify(&self.signing_bytes(), &sig))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DmStore {
    conversations: HashMap<UserId, Vec<DmMessage>>,
    pub contacts: HashMap<UserId, UserProfile>,
}

impl DmStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, partner: UserId, message: DmMessage) -> bool {
        let list = self.conversations.entry(partner).or_default();
        if list.iter().any(|m| m.id == message.id) {
            return false;
        }
        let pos = list.partition_point(|m| m.created_at <= message.created_at);
        list.insert(pos, message);
        true
    }

    pub fn apply(&mut self, partner: UserId, mut message: DmMessage) -> bool {
        let Some(target) = message.action.target() else {
            message.edited_at = None;
            message.reactions.clear();
            return self.insert(partner, message);
        };
        let list = self.conversations.entry(partner).or_default();
        if list.iter().any(|m| m.id == message.id) {
            return false;
        }
        let Some(pos) = list.iter().position(|m| m.id == target && m.is_post()) else {
            return false;
        };
        let changed = match &message.action {
            DmAction::Edit { .. } => {
                let original = &mut list[pos];
                if original.from != message.from || original.edited_at.is_some_and(|at| at >= message.created_at) {
                    false
                } else {
                    original.content = message.content.clone();
                    original.edited_at = Some(message.created_at);
                    true
                }
            }
            DmAction::Delete { .. } => {
                if list[pos].from != message.from {
                    false
                } else {
                    list.remove(pos);
                    list.retain(|m| m.action.target() != Some(target));
                    true
                }
            }
            DmAction::React { emoji, active, .. } => {
                let reactions = &mut list[pos].reactions;
                let slot = reactions.iter().position(|r| &r.emoji == emoji);
                match (slot, active) {
                    (Some(i), true) if !reactions[i].users.contains(&message.from) => {
                        reactions[i].users.push(message.from);
                        true
                    }
                    (None, true) => {
                        reactions.push(Reaction { emoji: emoji.clone(), users: vec![message.from] });
                        true
                    }
                    (Some(i), false) if reactions[i].users.contains(&message.from) => {
                        reactions[i].users.retain(|u| *u != message.from);
                        if reactions[i].users.is_empty() {
                            reactions.remove(i);
                        }
                        true
                    }
                    _ => false,
                }
            }
            DmAction::Post => false,
        };
        if !changed {
            return false;
        }
        let pos = list.partition_point(|m| m.created_at <= message.created_at);
        list.insert(pos, message);
        true
    }

    pub fn conversation(&self, partner: UserId) -> &[DmMessage] {
        self.conversations
            .get(&partner)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn posts(&self, partner: UserId) -> impl Iterator<Item = &DmMessage> {
        self.conversation(partner).iter().filter(|m| m.is_post())
    }

    pub fn post(&self, partner: UserId, id: MessageId) -> Option<&DmMessage> {
        self.posts(partner).find(|m| m.id == id)
    }

    pub fn last_post(&self, partner: UserId) -> Option<&DmMessage> {
        self.posts(partner).last()
    }

    pub fn remove_conversation(&mut self, partner: UserId) -> bool {
        self.conversations.remove(&partner).is_some()
    }

    pub fn has_sent_to(&self, me: UserId, partner: UserId) -> bool {
        self.conversation(partner).iter().any(|m| m.from == me)
    }

    pub fn pinned_key(&self, user: UserId) -> Option<IdentityKey> {
        self.contacts.get(&user).and_then(|p| p.identity)
    }

    pub fn remember_contact(&mut self, mut profile: UserProfile) -> bool {
        if !profile.pin_identity(self.pinned_key(profile.id)) {
            return false;
        }
        self.contacts.insert(profile.id, profile);
        true
    }

    pub fn partners(&self) -> Vec<UserId> {
        let mut partners: Vec<(UserId, DateTime<Utc>)> = self
            .conversations
            .iter()
            .filter_map(|(user, msgs)| msgs.iter().rev().find(|m| m.is_post()).map(|m| (*user, m.created_at)))
            .collect();
        partners.sort_by_key(|p| std::cmp::Reverse(p.1));
        partners.into_iter().map(|(user, _)| user).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NODE: &str = "0101010101010101010101010101010101010101010101010101010101010101";

    #[test]
    fn insert_dedupes_and_orders() {
        let mut store = DmStore::new();
        let (me, them) = (UserId::new(), UserId::new());
        let first = DmMessage::new(me, them, "hi");
        let second = DmMessage::new(them, me, "hey back");
        assert!(store.insert(them, second.clone()));
        assert!(store.insert(them, first.clone()));
        assert!(!store.insert(them, first.clone()));
        let convo = store.conversation(them);
        assert_eq!(convo.len(), 2);
        assert!(convo[0].created_at <= convo[1].created_at);
    }

    #[test]
    fn partners_sorted_by_recency() {
        let mut store = DmStore::new();
        let me = UserId::new();
        let (a, b) = (UserId::new(), UserId::new());
        store.insert(a, DmMessage::new(me, a, "older"));
        store.insert(b, DmMessage::new(me, b, "newer"));
        assert_eq!(store.partners(), vec![b, a]);
    }

    #[test]
    fn remove_and_sent_checks() {
        let mut store = DmStore::new();
        let (me, them) = (UserId::new(), UserId::new());
        store.insert(them, DmMessage::new(them, me, "hello?"));
        assert!(!store.has_sent_to(me, them));
        store.insert(them, DmMessage::new(me, them, "hi"));
        assert!(store.has_sent_to(me, them));
        assert!(store.remove_conversation(them));
        assert!(store.conversation(them).is_empty() && !store.remove_conversation(them));
    }

    #[test]
    fn empty_conversation_is_empty_slice() {
        let store = DmStore::new();
        assert!(store.conversation(UserId::new()).is_empty());
    }

    #[test]
    fn signed_message_verifies() {
        let (signer, stranger) = (Identity::generate(), Identity::generate());
        let mut msg = DmMessage::new(UserId::new(), UserId::new(), "signed");
        assert!(!msg.verify(&signer.public()), "unsigned");
        msg.sign(NODE, &signer);
        assert_eq!(msg.from_node, NODE);
        assert!(msg.verify(&signer.public()));
        assert!(!msg.verify(&stranger.public()), "wrong key");
    }

    #[test]
    fn tampering_any_signed_field_breaks_the_signature() {
        let signer = Identity::generate();
        let mut msg = DmMessage::new(UserId::new(), UserId::new(), "original");
        msg.sign(NODE, &signer);
        let key = signer.public();
        let tampered: [fn(&mut DmMessage); 7] = [
            |m| m.id = MessageId::new(),
            |m| m.from = UserId::new(),
            |m| m.to = UserId::new(),
            |m| m.content.push('!'),
            |m| m.created_at += chrono::Duration::milliseconds(1),
            |m| m.from_node = "ff".repeat(32),
            |m| m.sig.as_mut().unwrap().0[0] ^= 1,
        ];
        for change in tampered {
            let mut copy = msg.clone();
            change(&mut copy);
            assert!(!copy.verify(&key));
        }
    }

    #[test]
    fn signing_bytes_match_the_spec() {
        let mut msg = DmMessage::new(UserId::new(), UserId::new(), "spec");
        msg.from_node = NODE.into();
        let expected = postcard::to_stdvec(&(
            "commy-dm/v1",
            msg.id,
            msg.from,
            msg.to,
            "spec",
            msg.created_at.timestamp_millis(),
            NODE,
        ))
        .unwrap();
        assert_eq!(msg.signing_bytes(), expected);
    }

    #[test]
    fn signature_survives_both_encodings() {
        let signer = Identity::generate();
        let mut msg = DmMessage::new(UserId::new(), UserId::new(), "wire and disk");
        msg.sign(NODE, &signer);
        let wire: DmMessage = postcard::from_bytes(&postcard::to_stdvec(&msg).unwrap()).unwrap();
        assert_eq!(wire, msg);
        assert!(wire.verify(&signer.public()));
        let json = serde_json::to_string(&msg).unwrap();
        let disk: DmMessage = serde_json::from_str(&json).unwrap();
        assert!(disk.verify(&signer.public()));
    }

    #[test]
    fn stored_messages_without_a_signature_still_load() {
        let msg = DmMessage::new(UserId::new(), UserId::new(), "old");
        let mut json = serde_json::to_value(&msg).unwrap();
        json.as_object_mut().unwrap().remove("sig");
        json.as_object_mut().unwrap().remove("from_node");
        let back: DmMessage = serde_json::from_value(json).unwrap();
        assert_eq!(back.sig, None);
        assert_eq!(back.from_node, "");
        assert_eq!(back.content, "old");
    }

    #[test]
    fn first_contact_key_stays_pinned() {
        let mut store = DmStore::new();
        let mut profile = UserProfile::new("partner");
        let id = profile.id;
        assert_eq!(store.pinned_key(id), None);

        assert!(store.remember_contact(profile.clone()));
        assert_eq!(store.pinned_key(id), None);

        let first = Identity::generate().public();
        profile.identity = Some(first);
        assert!(store.remember_contact(profile.clone()));
        assert_eq!(store.pinned_key(id), Some(first));

        let mut keyless = profile.clone();
        keyless.identity = None;
        keyless.display_name = Some("renamed".into());
        assert!(store.remember_contact(keyless));
        assert_eq!(store.pinned_key(id), Some(first));
        assert_eq!(store.contacts[&id].display_name.as_deref(), Some("renamed"));

        let mut other = profile.clone();
        other.identity = Some(Identity::generate().public());
        other.display_name = Some("impostor".into());
        assert!(!store.remember_contact(other));
        assert_eq!(store.pinned_key(id), Some(first));
        assert_eq!(store.contacts[&id].display_name.as_deref(), Some("renamed"));
    }

    #[test]
    fn serde_roundtrip() {
        let mut store = DmStore::new();
        let (me, them) = (UserId::new(), UserId::new());
        store.insert(them, DmMessage::new(me, them, "persist me"));
        store.remember_contact(UserProfile::new("partner"));
        let json = serde_json::to_string(&store).unwrap();
        let back: DmStore = serde_json::from_str(&json).unwrap();
        assert_eq!(back.conversation(them).len(), 1);
        assert_eq!(back.contacts.len(), 1);
    }

    #[test]
    fn edits_deletes_and_reactions_apply_to_their_target() {
        let mut store = DmStore::new();
        let (me, them) = (UserId::new(), UserId::new());
        let post = DmMessage::new(them, me, "first draft");
        let target = post.id;
        assert!(store.apply(them, post));

        let edit = DmMessage::acting(them, me, DmAction::Edit { target }, "final draft");
        assert!(store.apply(them, edit.clone()));
        assert!(!store.apply(them, edit.clone()), "replayed edits are ignored");
        let seen = store.post(them, target).unwrap();
        assert_eq!(seen.content, "final draft");
        assert_eq!(seen.edited_at, Some(edit.created_at));
        assert_eq!(store.posts(them).count(), 1, "control messages are not posts");

        let foreign = DmMessage::acting(me, them, DmAction::Edit { target }, "not mine");
        assert!(!store.apply(them, foreign));
        assert_eq!(store.post(them, target).unwrap().content, "final draft");

        let react = DmAction::React { target, emoji: "🔥".into(), active: true };
        assert!(store.apply(them, DmMessage::acting(me, them, react.clone(), "")));
        assert!(!store.apply(them, DmMessage::acting(me, them, react, "")), "already reacted");
        assert!(store.apply(them, DmMessage::acting(them, me, DmAction::React { target, emoji: "🔥".into(), active: true }, "")));
        let reactions = &store.post(them, target).unwrap().reactions;
        assert_eq!(reactions.len(), 1);
        assert_eq!(reactions[0].users, vec![me, them]);
        assert!(store.apply(them, DmMessage::acting(me, them, DmAction::React { target, emoji: "🔥".into(), active: false }, "")));
        assert!(store.apply(them, DmMessage::acting(them, me, DmAction::React { target, emoji: "🔥".into(), active: false }, "")));
        assert!(store.post(them, target).unwrap().reactions.is_empty());

        assert!(!store.apply(them, DmMessage::acting(me, them, DmAction::Delete { target }, "")));
        assert!(store.post(them, target).is_some());
        assert!(store.apply(them, DmMessage::acting(them, me, DmAction::Delete { target }, "")));
        assert!(store.post(them, target).is_none());
        assert_eq!(store.conversation(them).iter().filter(|m| m.action.target() == Some(target)).count(), 1);
        assert!(!store.apply(them, DmMessage::acting(them, me, DmAction::Edit { target }, "gone")));
    }

    #[test]
    fn incoming_posts_start_without_derived_state() {
        let mut store = DmStore::new();
        let (me, them) = (UserId::new(), UserId::new());
        let mut post = DmMessage::new(them, me, "hi");
        post.edited_at = Some(Utc::now());
        post.reactions.push(Reaction { emoji: "👍".into(), users: vec![them] });
        assert!(store.apply(them, post.clone()));
        let stored = store.post(them, post.id).unwrap();
        assert_eq!(stored.edited_at, None);
        assert!(stored.reactions.is_empty());
    }

    #[test]
    fn thread_recency_ignores_control_messages() {
        let mut store = DmStore::new();
        let me = UserId::new();
        let (a, b) = (UserId::new(), UserId::new());
        let older = DmMessage::new(me, a, "older");
        let target = older.id;
        store.apply(a, older);
        store.apply(b, DmMessage::new(me, b, "newer"));
        store.apply(a, DmMessage::acting(me, a, DmAction::React { target, emoji: "👀".into(), active: true }, ""));
        assert_eq!(store.partners(), vec![b, a]);
        assert_eq!(store.last_post(a).unwrap().content, "older");
    }

    #[test]
    fn rich_messages_sign_every_field() {
        let signer = Identity::generate();
        let mut msg = DmMessage::new(UserId::new(), UserId::new(), "look");
        msg.attachments.push(Attachment {
            blob: "abc".into(),
            filename: "cat.png".into(),
            size_bytes: 3,
            mime_type: "image/png".into(),
            provider: Some(NODE.into()),
        });
        msg.reply_to = Some(MessageId::new());
        msg.sign(NODE, &signer);
        let key = signer.public();
        assert!(msg.verify(&key));
        let tampered: [fn(&mut DmMessage); 4] = [
            |m| m.attachments[0].blob.push('!'),
            |m| m.attachments.clear(),
            |m| m.reply_to = None,
            |m| m.action = DmAction::Delete { target: MessageId::new() },
        ];
        for change in tampered {
            let mut copy = msg.clone();
            change(&mut copy);
            assert!(!copy.verify(&key));
        }
        let mut control = DmMessage::acting(msg.from, msg.to, DmAction::Edit { target: msg.id }, "edited");
        control.sign(NODE, &signer);
        assert!(control.verify(&key));
        let mut retargeted = control.clone();
        retargeted.action = DmAction::Edit { target: MessageId::new() };
        assert!(!retargeted.verify(&key));
    }

    #[test]
    fn plain_messages_keep_the_original_signature_shape() {
        let msg = DmMessage::new(UserId::new(), UserId::new(), "plain");
        assert!(msg.is_plain());
        let expected = postcard::to_stdvec(&(
            "commy-dm/v1",
            msg.id,
            msg.from,
            msg.to,
            "plain",
            msg.created_at.timestamp_millis(),
            "",
        ))
        .unwrap();
        assert_eq!(msg.signing_bytes(), expected);
        let json = r#"{"id":"00000000-0000-0000-0000-000000000001","from":"00000000-0000-0000-0000-000000000002","to":"00000000-0000-0000-0000-000000000003","content":"old","created_at":"2026-01-01T00:00:00Z"}"#;
        let back: DmMessage = serde_json::from_str(json).unwrap();
        assert!(back.is_plain());
        assert_eq!(back.action, DmAction::Post);
        assert!(back.attachments.is_empty() && back.reactions.is_empty());
    }
}
