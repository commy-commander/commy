use crate::identity::{Identity, IdentityKey, Signature};
use crate::ids::{MessageId, UserId};
use crate::user::UserProfile;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A direct message between two users. DMs are not replicated through
/// servers; they travel over a dedicated peer connection and each side
/// keeps its own copy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DmMessage {
    pub id: MessageId,
    pub from: UserId,
    pub to: UserId,
    pub content: String,
    pub created_at: DateTime<Utc>,
    /// Sender's signature over [`DmMessage::signing_bytes`]. None on
    /// messages from before signing existed.
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
            sig: None,
        }
    }

    /// Bytes the sender signs: every field but the signature.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let fields = (
            DM_DOMAIN,
            self.id,
            self.from,
            self.to,
            self.content.as_str(),
            self.created_at.timestamp_millis(),
        );
        postcard::to_stdvec(&fields).expect("postcard encodes plain tuples")
    }

    pub fn sign(&mut self, identity: &Identity) {
        self.sig = Some(identity.sign(&self.signing_bytes()));
    }

    /// False when unsigned or not signed by `key`.
    pub fn verify(&self, key: &IdentityKey) -> bool {
        self.sig
            .is_some_and(|sig| key.verify(&self.signing_bytes(), &sig))
    }
}

/// Local DM history, keyed by the other participant. Also caches the
/// partner's last known profile so conversations survive leaving the
/// server you met them in.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DmStore {
    conversations: HashMap<UserId, Vec<DmMessage>>,
    pub contacts: HashMap<UserId, UserProfile>,
}

impl DmStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a message into the conversation with `partner`,
    /// deduplicating by id and keeping chronological order.
    /// Returns false for duplicates.
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

    /// Drops a whole conversation, e.g. a declined message request.
    pub fn remove_conversation(&mut self, partner: UserId) -> bool {
        self.conversations.remove(&partner).is_some()
    }

    /// True if we have ever sent `partner` a message.
    pub fn has_sent_to(&self, me: UserId, partner: UserId) -> bool {
        self.conversation(partner).iter().any(|m| m.from == me)
    }

    /// Identity key pinned for a contact: the first one seen for them.
    pub fn pinned_key(&self, user: UserId) -> Option<IdentityKey> {
        self.contacts.get(&user).and_then(|p| p.identity)
    }

    /// Caches a contact's profile, keeping the pinned key: a profile
    /// without a key keeps the stored one, and one with a different key
    /// is refused (returns false and changes nothing).
    pub fn remember_contact(&mut self, mut profile: UserProfile) -> bool {
        match (self.pinned_key(profile.id), profile.identity) {
            (Some(pinned), Some(key)) if pinned != key => return false,
            (Some(pinned), None) => profile.identity = Some(pinned),
            _ => {}
        }
        self.contacts.insert(profile.id, profile);
        true
    }

    /// Conversation partners, most recently active first.
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
        let (alice, bob) = (Identity::generate(), Identity::generate());
        let mut msg = DmMessage::new(UserId::new(), UserId::new(), "signed");
        assert!(!msg.verify(&alice.public()), "unsigned");
        msg.sign(&alice);
        assert!(msg.verify(&alice.public()));
        assert!(!msg.verify(&bob.public()), "wrong key");
    }

    #[test]
    fn tampering_any_signed_field_breaks_the_signature() {
        let alice = Identity::generate();
        let mut msg = DmMessage::new(UserId::new(), UserId::new(), "original");
        msg.sign(&alice);
        let key = alice.public();
        let tampered: [fn(&mut DmMessage); 6] = [
            |m| m.id = MessageId::new(),
            |m| m.from = UserId::new(),
            |m| m.to = UserId::new(),
            |m| m.content.push('!'),
            |m| m.created_at += chrono::Duration::milliseconds(1),
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
        let msg = DmMessage::new(UserId::new(), UserId::new(), "spec");
        let expected = postcard::to_stdvec(&(
            "commy-dm/v1",
            msg.id,
            msg.from,
            msg.to,
            "spec",
            msg.created_at.timestamp_millis(),
        ))
        .unwrap();
        assert_eq!(msg.signing_bytes(), expected);
    }

    #[test]
    fn signature_survives_both_encodings() {
        let alice = Identity::generate();
        let mut msg = DmMessage::new(UserId::new(), UserId::new(), "wire and disk");
        msg.sign(&alice);
        let wire: DmMessage = postcard::from_bytes(&postcard::to_stdvec(&msg).unwrap()).unwrap();
        assert_eq!(wire, msg);
        assert!(wire.verify(&alice.public()));
        let json = serde_json::to_string(&msg).unwrap();
        let disk: DmMessage = serde_json::from_str(&json).unwrap();
        assert!(disk.verify(&alice.public()));
    }

    #[test]
    fn stored_messages_without_a_signature_still_load() {
        let msg = DmMessage::new(UserId::new(), UserId::new(), "old");
        let mut json = serde_json::to_value(&msg).unwrap();
        json.as_object_mut().unwrap().remove("sig");
        let back: DmMessage = serde_json::from_value(json).unwrap();
        assert_eq!(back.sig, None);
        assert_eq!(back.content, "old");
    }

    #[test]
    fn first_contact_key_stays_pinned() {
        let mut store = DmStore::new();
        let mut profile = UserProfile::new("partner");
        let id = profile.id;
        assert_eq!(store.pinned_key(id), None);

        // A profile without a key pins nothing yet.
        assert!(store.remember_contact(profile.clone()));
        assert_eq!(store.pinned_key(id), None);

        let first = Identity::generate().public();
        profile.identity = Some(first);
        assert!(store.remember_contact(profile.clone()));
        assert_eq!(store.pinned_key(id), Some(first));

        // A keyless update keeps the pin and still refreshes the profile.
        let mut keyless = profile.clone();
        keyless.identity = None;
        keyless.display_name = Some("renamed".into());
        assert!(store.remember_contact(keyless));
        assert_eq!(store.pinned_key(id), Some(first));
        assert_eq!(store.contacts[&id].display_name.as_deref(), Some("renamed"));

        // A different key is refused and changes nothing.
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
