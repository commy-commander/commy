use crate::event::ServerEvent;
use crate::identity::{Identity, IdentityKey, Signature};
use crate::ids::{ChannelId, MessageId, ServerId, UserId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub blob: String,
    pub filename: String,
    pub size_bytes: u64,
    pub mime_type: String,
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
    #[serde(default)]
    pub sig: Option<Signature>,
}

const GOSSIP_FRAME_BYTES: usize = 3800;
const SMALLEST_MESSAGE_CREATE_ENVELOPE_BYTES: usize = 36;
pub const MAX_MESSAGE_BYTES: usize = GOSSIP_FRAME_BYTES - SMALLEST_MESSAGE_CREATE_ENVELOPE_BYTES;

pub const MAX_RELAYED_REACTIONS: usize = 64;
pub const MAX_RELAYED_REACTORS: usize = 2000;

const MESSAGE_DOMAIN: &str = "commy-message/v1";
const DELETE_DOMAIN: &str = "commy-delete/v1";

#[derive(Serialize)]
struct SignedMessage<'a> {
    domain: &'static str,
    server: ServerId,
    id: MessageId,
    channel: ChannelId,
    author: UserId,
    content: &'a str,
    created_at: i64,
    edited_at: Option<i64>,
    attachments: Vec<(&'a str, &'a str, u64, &'a str, Option<&'a str>)>,
    reply_to: Option<MessageId>,
}

fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    postcard::to_stdvec(value).expect("postcard encodes plain structs")
}

impl Message {
    pub fn new(channel: ChannelId, author: UserId, content: impl Into<String>) -> Self {
        Self {
            id: MessageId::for_author(author),
            channel,
            author,
            content: content.into(),
            created_at: Utc::now(),
            edited_at: None,
            attachments: vec![],
            reactions: vec![],
            reply_to: None,
            sig: None,
        }
    }

    pub fn signing_bytes(&self, server: ServerId) -> Vec<u8> {
        self.signing_bytes_with(server, &self.content, self.edited_at)
    }

    fn signing_bytes_with(&self, server: ServerId, content: &str, edited_at: Option<DateTime<Utc>>) -> Vec<u8> {
        encode(&SignedMessage {
            domain: MESSAGE_DOMAIN,
            server,
            id: self.id,
            channel: self.channel,
            author: self.author,
            content,
            created_at: self.created_at.timestamp_millis(),
            edited_at: edited_at.map(|t| t.timestamp_millis()),
            attachments: self
                .attachments
                .iter()
                .map(|a| (a.blob.as_str(), a.filename.as_str(), a.size_bytes, a.mime_type.as_str(), a.provider.as_deref()))
                .collect(),
            reply_to: self.reply_to,
        })
    }

    pub fn authored_size(&self) -> usize {
        encode(&self.without_reactions()).len()
    }

    pub(crate) fn without_reactions(&self) -> Message {
        Message {
            id: self.id,
            channel: self.channel,
            author: self.author,
            content: self.content.clone(),
            created_at: self.created_at,
            edited_at: self.edited_at,
            attachments: self.attachments.clone(),
            reactions: Vec::new(),
            reply_to: self.reply_to,
            sig: self.sig,
        }
    }

    pub fn sign(&mut self, server: ServerId, identity: &Identity) {
        self.sig = Some(identity.sign(&self.signing_bytes(server)));
    }

    pub fn verify(&self, server: ServerId, key: &IdentityKey) -> bool {
        self.sig.is_some_and(|sig| key.verify(&self.signing_bytes(server), &sig))
    }

    pub fn verify_edit(
        &self,
        server: ServerId,
        content: &str,
        edited_at: DateTime<Utc>,
        sig: &Signature,
        key: &IdentityKey,
    ) -> bool {
        key.verify(&self.signing_bytes_with(server, content, Some(edited_at)), sig)
    }

    pub fn edit_event(
        &self,
        server: ServerId,
        content: impl Into<String>,
        edited_at: DateTime<Utc>,
        identity: &Identity,
    ) -> ServerEvent {
        let content = content.into();
        let sig = identity.sign(&self.signing_bytes_with(server, &content, Some(edited_at)));
        ServerEvent::MessageEdit {
            channel: self.channel,
            message: self.id,
            content,
            edited_at,
            sig,
        }
    }

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TombstoneProof {
    pub channel: ChannelId,
    pub message: MessageId,
    pub by: UserId,
    pub at: DateTime<Utc>,
    pub sig: Signature,
}

impl TombstoneProof {
    pub fn sign(
        server: ServerId,
        channel: ChannelId,
        message: MessageId,
        by: UserId,
        at: DateTime<Utc>,
        identity: &Identity,
    ) -> Self {
        let mut proof = Self { channel, message, by, at, sig: Signature([0; 64]) };
        proof.sig = identity.sign(&proof.signing_bytes(server));
        proof
    }

    pub fn signing_bytes(&self, server: ServerId) -> Vec<u8> {
        encode(&(DELETE_DOMAIN, server, self.channel, self.message, self.by, self.at.timestamp_millis()))
    }

    pub fn verify(&self, server: ServerId, key: &IdentityKey) -> bool {
        key.verify(&self.signing_bytes(server), &self.sig)
    }

    pub fn into_event(self) -> ServerEvent {
        ServerEvent::MessageDelete {
            channel: self.channel,
            message: self.message,
            by: self.by,
            at: self.at,
            sig: self.sig,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_messages_get_an_id_bound_to_their_author() {
        let author = UserId::new();
        let msg = Message::new(ChannelId::new(), author, "hi");
        assert!(msg.id.is_bound_to(author));
        assert!(!msg.id.is_bound_to(UserId::new()));
    }

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

    fn sample(identity: &Identity, server: ServerId) -> Message {
        let mut msg = Message::new(ChannelId::new(), UserId::new(), "hi");
        msg.attachments.push(Attachment {
            blob: "abc".to_string(),
            filename: "cat.png".to_string(),
            size_bytes: 1024,
            mime_type: "image/png".to_string(),
            provider: Some("node".to_string()),
        });
        msg.reply_to = Some(MessageId::new());
        msg.edited_at = Some(Utc::now());
        msg.sign(server, identity);
        msg
    }

    #[test]
    fn signing_covers_every_field_but_reactions() {
        let identity = Identity::generate();
        let key = identity.public();
        let server = ServerId::new();
        let msg = sample(&identity, server);
        assert!(msg.verify(server, &key));
        assert!(!msg.verify(ServerId::new(), &key), "bound to the server");
        assert!(!msg.verify(server, &Identity::generate().public()));

        let ms = chrono::Duration::milliseconds(1);
        type Tamper = fn(&mut Message, chrono::Duration);
        let tampers: [(&str, Tamper); 14] = [
            ("id", |m, _| m.id = MessageId::for_author(m.author)),
            ("channel", |m, _| m.channel = ChannelId::new()),
            ("author", |m, _| m.author = UserId::new()),
            ("content", |m, _| m.content.push('!')),
            ("created_at", |m, d| m.created_at += d),
            ("edited_at", |m, d| m.edited_at = m.edited_at.map(|t| t + d)),
            ("edited_at cleared", |m, _| m.edited_at = None),
            ("blob", |m, _| m.attachments[0].blob.push('x')),
            ("filename", |m, _| m.attachments[0].filename.push('x')),
            ("size", |m, _| m.attachments[0].size_bytes += 1),
            ("mime", |m, _| m.attachments[0].mime_type = "image/gif".to_string()),
            ("provider", |m, _| m.attachments[0].provider = None),
            ("attachments", |m, _| m.attachments.clear()),
            ("reply_to", |m, _| m.reply_to = None),
        ];
        for (field, tamper) in tampers {
            let mut changed = msg.clone();
            tamper(&mut changed, ms);
            assert!(!changed.verify(server, &key), "{field} is covered");
        }

        let mut reacted = msg.clone();
        reacted.toggle_reaction(":fire:", UserId::new());
        assert!(reacted.verify(server, &key), "reactions are not covered");
        let mut unsigned = msg;
        unsigned.sig = None;
        assert!(!unsigned.verify(server, &key));
    }

    #[test]
    fn signing_bytes_start_with_the_domain_and_use_milliseconds() {
        let identity = Identity::generate();
        let server = ServerId::new();
        let msg = sample(&identity, server);
        let bytes = msg.signing_bytes(server);
        assert_eq!(bytes[0] as usize, MESSAGE_DOMAIN.len());
        assert_eq!(&bytes[1..1 + MESSAGE_DOMAIN.len()], MESSAGE_DOMAIN.as_bytes());
        let mut finer = msg.clone();
        let base = msg.created_at.timestamp_millis();
        finer.created_at = chrono::DateTime::from_timestamp_millis(base).unwrap() + chrono::Duration::microseconds(500);
        assert_eq!(finer.signing_bytes(server), bytes, "below a millisecond is not signed");
    }

    #[test]
    fn edit_event_signs_the_edited_message() {
        let identity = Identity::generate();
        let key = identity.public();
        let server = ServerId::new();
        let msg = sample(&identity, server);
        let at = Utc::now() + chrono::Duration::seconds(1);
        let ServerEvent::MessageEdit { channel, message, content, edited_at, sig } =
            msg.edit_event(server, "new", at, &identity)
        else {
            panic!("expected an edit");
        };
        assert_eq!((channel, message, content.as_str(), edited_at), (msg.channel, msg.id, "new", at));
        assert!(msg.verify_edit(server, &content, edited_at, &sig, &key));
        assert!(!msg.verify_edit(server, "other", edited_at, &sig, &key));
        assert!(!msg.verify_edit(server, &content, edited_at + chrono::Duration::milliseconds(1), &sig, &key));
        assert!(!msg.verify_edit(ServerId::new(), &content, edited_at, &sig, &key));
        let after = Message { content, edited_at: Some(edited_at), sig: Some(sig), ..msg };
        assert!(after.verify(server, &key));
    }

    #[test]
    fn tombstone_proof_signs_every_field() {
        let identity = Identity::generate();
        let key = identity.public();
        let server = ServerId::new();
        let proof = TombstoneProof::sign(server, ChannelId::new(), MessageId::new(), UserId::new(), Utc::now(), &identity);
        assert!(proof.verify(server, &key));
        assert!(!proof.verify(ServerId::new(), &key));
        assert!(!proof.verify(server, &Identity::generate().public()));
        let bytes = proof.signing_bytes(server);
        assert_eq!(&bytes[1..1 + DELETE_DOMAIN.len()], DELETE_DOMAIN.as_bytes());
        let tampered = [
            TombstoneProof { channel: ChannelId::new(), ..proof.clone() },
            TombstoneProof { message: MessageId::new(), ..proof.clone() },
            TombstoneProof { by: UserId::new(), ..proof.clone() },
            TombstoneProof { at: proof.at + chrono::Duration::milliseconds(1), ..proof.clone() },
        ];
        for t in tampered {
            assert!(!t.verify(server, &key));
        }
        let json = serde_json::to_string(&proof).unwrap();
        assert_eq!(serde_json::from_str::<TombstoneProof>(&json).unwrap(), proof);
        let ServerEvent::MessageDelete { channel, message, by, at, sig } = proof.clone().into_event() else {
            panic!("expected a delete");
        };
        assert_eq!(TombstoneProof { channel, message, by, at, sig }, proof);
    }

    #[test]
    fn signed_message_serializes_and_old_json_still_loads() {
        let identity = Identity::generate();
        let server = ServerId::new();
        let msg = sample(&identity, server);
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(serde_json::from_value::<Message>(json.clone()).unwrap(), msg);
        let bytes = postcard::to_stdvec(&msg).unwrap();
        assert_eq!(postcard::from_bytes::<Message>(&bytes).unwrap(), msg);
        let mut old = json;
        old.as_object_mut().unwrap().remove("sig");
        let loaded: Message = serde_json::from_value(old).unwrap();
        assert_eq!(loaded.sig, None);
    }
}
