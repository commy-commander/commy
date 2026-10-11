use crate::ids::{ChannelId, MessageId, UserId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// File shared in chat. The payload itself transfers peer to peer via
/// iroh-blobs; messages only carry the content hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    /// iroh blob hash (blake3, hex or ticket form).
    pub blob: String,
    pub filename: String,
    pub size_bytes: u64,
    pub mime_type: String,
    /// Hex node id of a peer known to have the blob (the uploader).
    #[serde(default)]
    pub provider: Option<String>,
}

impl Attachment {
    pub fn is_image(&self) -> bool {
        self.mime_type.starts_with("image/")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reaction {
    /// Unicode emoji or custom emoji name prefixed with a colon.
    pub emoji: String,
    pub users: Vec<UserId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub channel: ChannelId,
    pub author: UserId,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    pub attachments: Vec<Attachment>,
    pub reactions: Vec<Reaction>,
    pub reply_to: Option<MessageId>,
}

impl Message {
    pub fn new(channel: ChannelId, author: UserId, content: impl Into<String>) -> Self {
        Self {
            id: MessageId::new(),
            channel,
            author,
            content: content.into(),
            created_at: Utc::now(),
            edited_at: None,
            attachments: vec![],
            reactions: vec![],
            reply_to: None,
        }
    }

    /// Adds or removes a user's reaction, returning true if it was added.
    pub fn toggle_reaction(&mut self, emoji: &str, user: UserId) -> bool {
        if let Some(reaction) = self.reactions.iter_mut().find(|r| r.emoji == emoji) {
            if let Some(pos) = reaction.users.iter().position(|u| *u == user) {
                reaction.users.remove(pos);
                if reaction.users.is_empty() {
                    self.reactions.retain(|r| r.emoji != emoji);
                }
                false
            } else {
                reaction.users.push(user);
                true
            }
        } else {
            self.reactions.push(Reaction {
                emoji: emoji.to_string(),
                users: vec![user],
            });
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_reaction_adds_then_removes() {
        let mut msg = Message::new(ChannelId::new(), UserId::new(), "hi");
        let user = UserId::new();
        assert!(msg.toggle_reaction(":wave:", user));
        assert_eq!(msg.reactions.len(), 1);
        assert!(!msg.toggle_reaction(":wave:", user));
        assert!(msg.reactions.is_empty());
    }

    #[test]
    fn multiple_users_share_a_reaction() {
        let mut msg = Message::new(ChannelId::new(), UserId::new(), "hi");
        let (a, b) = (UserId::new(), UserId::new());
        msg.toggle_reaction(":fire:", a);
        msg.toggle_reaction(":fire:", b);
        assert_eq!(msg.reactions.len(), 1);
        assert_eq!(msg.reactions[0].users.len(), 2);
        msg.toggle_reaction(":fire:", a);
        assert_eq!(msg.reactions[0].users, vec![b]);
    }

    #[test]
    fn attachment_image_detection() {
        let att = Attachment {
            blob: "abc".to_string(),
            filename: "cat.png".to_string(),
            size_bytes: 1024,
            mime_type: "image/png".to_string(),
            provider: None,
        };
        assert!(att.is_image());
        let doc = Attachment {
            mime_type: "application/pdf".to_string(),
            ..att
        };
        assert!(!doc.is_image());
    }
}
