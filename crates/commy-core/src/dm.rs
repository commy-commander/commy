use crate::identity::{Identity, IdentityKey, Signature};
use crate::ids::{MessageId, UserId};
use crate::user::UserProfile;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
}

const DM_DOMAIN: &str = "commy-dm/v1";

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
        }
    }

    pub fn signing_bytes(&self) -> Vec<u8> {
        let fields = (
            DM_DOMAIN,
            self.id,
            self.from,
            self.to,
            self.content.as_str(),
            self.created_at.timestamp_millis(),
            self.from_node.as_str(),
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

    pub fn conversation(&self, partner: UserId) -> &[DmMessage] {
        self.conversations
            .get(&partner)
            .map(Vec::as_slice)
            .unwrap_or(&[])
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
            .filter(|(_, msgs)| !msgs.is_empty())
            .map(|(user, msgs)| (*user, msgs.last().unwrap().created_at))
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
}
