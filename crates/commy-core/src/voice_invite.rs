use crate::dm::DmMessage;
use crate::ids::{ChannelId, ServerId};
use crate::message::{Attachment, Message};
use serde::{Deserialize, Serialize};

pub const VOICE_INVITE_MIME: &str = "application/vnd.commy.voice-invite+json";
pub const MAX_INVITE_LINK_CHARS: usize = 600;
pub const MAX_NAME_CHARS: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceInvite {
    pub server: ServerId,
    pub channel: ChannelId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invite_link: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub server_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub channel_name: String,
}

impl VoiceInvite {
    pub fn new(server: ServerId, channel: ChannelId) -> Self {
        Self { server, channel, invite_link: None, server_name: String::new(), channel_name: String::new() }
    }

    pub fn tidy(mut self) -> Self {
        self.invite_link = self
            .invite_link
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && l.chars().count() <= MAX_INVITE_LINK_CHARS && !l.chars().any(char::is_whitespace));
        self.server_name = clip(&self.server_name);
        self.channel_name = clip(&self.channel_name);
        self
    }

    pub fn into_attachment(self) -> Attachment {
        Attachment {
            blob: String::new(),
            filename: serde_json::to_string(&self.tidy()).expect("ids and strings encode"),
            size_bytes: 0,
            mime_type: VOICE_INVITE_MIME.to_string(),
            provider: None,
        }
    }
}

fn clip(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_NAME_CHARS).collect()
}

impl Attachment {
    pub fn is_voice_invite(&self) -> bool {
        self.mime_type == VOICE_INVITE_MIME
    }

    pub fn voice_invite(&self) -> Option<VoiceInvite> {
        if !self.is_voice_invite() {
            return None;
        }
        serde_json::from_str::<VoiceInvite>(&self.filename).ok().map(VoiceInvite::tidy)
    }
}

impl Message {
    pub fn voice_invite(&self) -> Option<VoiceInvite> {
        self.attachments.iter().find_map(Attachment::voice_invite)
    }
}

impl DmMessage {
    pub fn voice_invite(&self) -> Option<VoiceInvite> {
        self.attachments.iter().find_map(Attachment::voice_invite)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::UserId;

    fn sample() -> VoiceInvite {
        VoiceInvite {
            server: ServerId::new(),
            channel: ChannelId::new(),
            invite_link: Some("https://commy.chat/i/KQ7M3N4P#key".into()),
            server_name: "Rust Hangout".into(),
            channel_name: "General".into(),
        }
    }

    #[test]
    fn a_voice_invite_rides_on_a_message_as_a_card_without_a_file() {
        let invite = sample();
        let att = invite.clone().into_attachment();
        assert!(att.is_voice_invite());
        assert!(att.is_card());
        assert!(!att.carries_a_file());
        assert!(!att.is_poll());
        assert!(!att.is_link_preview());
        assert_eq!(att.blob, "");
        assert_eq!(att.size_bytes, 0);
        assert_eq!(att.voice_invite(), Some(invite.clone()));
        let mut message = Message::new(ChannelId::new(), UserId::new(), "");
        assert_eq!(message.voice_invite(), None);
        message.attachments.push(att.clone());
        assert_eq!(message.voice_invite(), Some(invite.clone()));
        assert_eq!(message.files().count(), 0, "an invite is not a file to download");
        let mut dm = DmMessage::new(UserId::new(), UserId::new(), "");
        assert_eq!(dm.voice_invite(), None);
        dm.attachments.push(att);
        assert_eq!(dm.voice_invite(), Some(invite));
    }

    #[test]
    fn the_card_json_carries_only_ids_and_names_and_old_fields_default() {
        let bare = VoiceInvite::new(ServerId::new(), ChannelId::new());
        let json = serde_json::to_string(&bare).unwrap();
        assert!(!json.contains("invite_link") && !json.contains("server_name") && !json.contains("channel_name"));
        let text = format!(r#"{{"server":"{}","channel":"{}"}}"#, bare.server, bare.channel);
        let loaded: VoiceInvite = serde_json::from_str(&text).unwrap();
        assert_eq!(loaded, bare);
        let json = serde_json::to_string(&sample()).unwrap();
        assert!(!json.contains("users") && !json.contains("participants"));
    }

    #[test]
    fn broken_json_or_another_mime_reads_as_no_invite() {
        let mut att = sample().into_attachment();
        att.filename = "{not json".into();
        assert_eq!(att.voice_invite(), None);
        let mut att = sample().into_attachment();
        att.mime_type = "text/plain".into();
        assert_eq!(att.voice_invite(), None);
        assert!(att.carries_a_file());
    }

    #[test]
    fn tidy_drops_links_with_whitespace_or_too_long_and_squashes_names() {
        let spaced = VoiceInvite { invite_link: Some("https://commy.chat/i/a b".into()), ..sample() }.tidy();
        assert_eq!(spaced.invite_link, None);
        let long = VoiceInvite { invite_link: Some(format!("https://commy.chat/i/{}", "x".repeat(MAX_INVITE_LINK_CHARS))), ..sample() }.tidy();
        assert_eq!(long.invite_link, None);
        let padded = VoiceInvite { invite_link: Some("  https://commy.chat/i/abc#k  ".into()), server_name: "  Rust \n Hangout ".into(), ..sample() }.tidy();
        assert_eq!(padded.invite_link.as_deref(), Some("https://commy.chat/i/abc#k"));
        assert_eq!(padded.server_name, "Rust Hangout");
        let empty = VoiceInvite { invite_link: Some("   ".into()), ..sample() }.tidy();
        assert_eq!(empty.invite_link, None);
    }

    #[derive(Debug, PartialEq, Eq, Deserialize)]
    struct OlderAttachment {
        blob: String,
        filename: String,
        size_bytes: u64,
        mime_type: String,
        provider: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct OlderMessage {
        id: crate::ids::MessageId,
        channel: ChannelId,
        author: UserId,
        content: String,
        created_at: chrono::DateTime<chrono::Utc>,
        edited_at: Option<chrono::DateTime<chrono::Utc>>,
        attachments: Vec<OlderAttachment>,
        reactions: Vec<crate::message::Reaction>,
        reply_to: Option<crate::ids::MessageId>,
        sig: Option<crate::identity::Signature>,
    }

    #[test]
    fn a_build_without_voice_invites_still_decodes_a_message_carrying_one() {
        let identity = crate::identity::Identity::generate();
        let server = ServerId::new();
        let mut message = Message::new(ChannelId::new(), UserId::new(), "");
        message.attachments.push(sample().into_attachment());
        message.sign(server, &identity);
        let bytes = postcard::to_stdvec(&message).unwrap();
        let older: OlderMessage = postcard::from_bytes(&bytes).unwrap();
        assert_eq!((older.id, older.channel, older.author), (message.id, message.channel, message.author));
        assert_eq!((older.content.as_str(), older.created_at, older.edited_at), ("", message.created_at, None));
        assert_eq!((older.reactions.len(), older.reply_to), (0, None));
        assert_eq!(older.attachments.len(), 1);
        assert_eq!(older.attachments[0].mime_type, VOICE_INVITE_MIME);
        assert_eq!((older.attachments[0].blob.as_str(), older.attachments[0].size_bytes, older.attachments[0].provider.as_deref()), ("", 0, None));
        assert_eq!(older.attachments[0].filename, message.attachments[0].filename);
        assert_eq!(older.sig, message.sig);
        let back: Message = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(back, message);
        assert!(back.verify(server, &identity.public()));
        let json = serde_json::to_value(&message).unwrap();
        assert_eq!(serde_json::from_value::<Message>(json).unwrap(), message);
    }
}
