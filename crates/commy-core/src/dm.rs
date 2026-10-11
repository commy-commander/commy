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
}

impl DmMessage {
    pub fn new(from: UserId, to: UserId, content: impl Into<String>) -> Self {
        Self {
            id: MessageId::new(),
            from,
            to,
            content: content.into(),
            created_at: Utc::now(),
        }
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

    pub fn remember_contact(&mut self, profile: UserProfile) {
        self.contacts.insert(profile.id, profile);
    }

    /// Conversation partners, most recently active first.
    pub fn partners(&self) -> Vec<UserId> {
        let mut partners: Vec<(UserId, DateTime<Utc>)> = self
            .conversations
            .iter()
            .filter(|(_, msgs)| !msgs.is_empty())
            .map(|(user, msgs)| (*user, msgs.last().unwrap().created_at))
            .collect();
        partners.sort_by(|a, b| b.1.cmp(&a.1));
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
