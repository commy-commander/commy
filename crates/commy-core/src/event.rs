use crate::identity::Signature;
use crate::ids::{ChannelId, MessageId, RoleId, ServerId, UserId};
use crate::message::Message;
use crate::server::Channel;
use crate::user::{Presence, UserProfile};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerEvent {
    MessageCreate {
        message: Message,
    },
    MessageEdit {
        channel: ChannelId,
        message: MessageId,
        content: String,
        edited_at: DateTime<Utc>,
        sig: Signature,
    },
    MessageDelete {
        channel: ChannelId,
        message: MessageId,
        by: UserId,
        at: DateTime<Utc>,
        sig: Signature,
    },
    ReactionToggle {
        channel: ChannelId,
        message: MessageId,
        emoji: String,
        user: UserId,
    },
    MemberJoin {
        server: ServerId,
        profile: UserProfile,
    },
    MemberLeave {
        server: ServerId,
        user: UserId,
    },
    PresenceUpdate {
        user: UserId,
        presence: Presence,
    },
    ChannelCreate {
        server: ServerId,
        channel: Channel,
    },
    ChannelDelete {
        server: ServerId,
        channel: ChannelId,
    },
    VoiceState {
        user: UserId,
        channel: Option<ChannelId>,
        muted: bool,
        deafened: bool,
        #[serde(default)]
        since: Option<i64>,
    },
    TypingStart {
        channel: ChannelId,
        user: UserId,
    },
    ScreenShare {
        user: UserId,
        channel: ChannelId,
        active: bool,
    },
    ServerRename { server: ServerId, name: String },
    ProfileUpdate { profile: UserProfile },
    RoleCreate { server: ServerId, role: crate::server::Role },
    RoleDelete { server: ServerId, role: RoleId },
    RoleAssign {
        server: ServerId,
        user: UserId,
        role: RoleId,
    },
    RoleUnassign {
        server: ServerId,
        user: UserId,
        role: RoleId,
    },
    AfkChannelSet {
        server: ServerId,
        channel: Option<ChannelId>,
    },
    ChannelUpdate {
        server: ServerId,
        channel: ChannelId,
        name: String,
        retention_days: Option<u32>,
    },
    CameraState {
        user: UserId,
        channel: ChannelId,
        active: bool,
    },
    ServerImages {
        server: ServerId,
        icon: Option<String>,
        banner: Option<String>,
    },
    ReactionSet {
        channel: ChannelId,
        message: MessageId,
        emoji: String,
        user: UserId,
        active: bool,
    },
    EmojiAdd { server: ServerId, emoji: crate::server::CustomEmoji },
    EmojiRemove { server: ServerId, name: String },
    SoundAdd { server: ServerId, sound: crate::server::SoundboardSound },
    SoundRemove { server: ServerId, name: String },
    ExpressionSettings { server: ServerId, emojis_shareable: bool, soundboard_open: bool },
    PublicSet { server: ServerId, public: bool },
    SoundboardPlay { server: ServerId, channel: ChannelId, user: UserId, name: String },
    GameTagsSet { server: ServerId, tags: Vec<String>, meta: Vec<crate::server::GameTagMeta> },
    RulesSet { server: ServerId, rules: Option<String> },
    RulesAccept { server: ServerId, user: UserId },
    EventUpsert { server: ServerId, event: crate::server::ScheduledEvent },
    EventDelete { server: ServerId, event: String },
    EventRsvp { server: ServerId, event: String, user: UserId, rsvp: Option<crate::server::Rsvp> },
    ChannelLayout { server: ServerId, channel: ChannelId, parent: Option<ChannelId>, position: u32 },
    ChannelLimit { server: ServerId, channel: ChannelId, user_limit: u32 },
    RoleUpdate { server: ServerId, role: crate::server::Role },
    VoiceModerate { server: ServerId, user: UserId, muted: bool, deafened: bool },
    MemberBan { server: ServerId, user: UserId },
    MemberUnban { server: ServerId, user: UserId },
    AfkTimeoutSet { server: ServerId, minutes: Option<u32> },
    DoorbellSet {
        server: ServerId,
        user: UserId,
        node: String,
        url: String,
        key: Option<crate::doorbell::DoorbellKey>,
        sig: Signature,
    },
    ChannelBanner {
        server: ServerId,
        channel: ChannelId,
        banner: Option<String>,
    },
    ChannelColor {
        server: ServerId,
        channel: ChannelId,
        color: Option<String>,
    },
    MessagePin {
        server: ServerId,
        channel: ChannelId,
        message: MessageId,
        by: UserId,
        at: DateTime<Utc>,
        pinned: bool,
    },
    ChannelKindSet {
        server: ServerId,
        channel: ChannelId,
        kind: crate::server::ChannelKind,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ChannelId;

    #[test]
    fn event_json_roundtrip() {
        let event = ServerEvent::TypingStart {
            channel: ChannelId::new(),
            user: UserId::new(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("typing_start"));
        let back: ServerEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn externally_tagged_events_roundtrip_through_postcard() {
        let event = ServerEvent::TypingStart {
            channel: ChannelId::new(),
            user: UserId::new(),
        };
        let bytes = postcard::to_stdvec(&event).unwrap();
        let back: ServerEvent = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn appended_variants_leave_earlier_wire_tags_unchanged() {
        let wire_tag = |event: ServerEvent| postcard::to_stdvec(&event).unwrap()[0];
        let (server, channel, user) = (ServerId::new(), ChannelId::new(), UserId::new());
        assert_eq!(wire_tag(ServerEvent::TypingStart { channel, user }), 10);
        assert_eq!(wire_tag(ServerEvent::ServerRename { server, name: "s".into() }), 12);
        assert_eq!(wire_tag(ServerEvent::AfkChannelSet { server, channel: None }), 18);
        let kind = crate::server::ChannelKind::Text;
        assert_eq!(wire_tag(ServerEvent::ChannelKindSet { server, channel, kind }), 47);
    }

    #[test]
    fn signed_events_roundtrip_both_formats() {
        let identity = crate::identity::Identity::generate();
        let server = ServerId::new();
        let mut message = Message::new(ChannelId::new(), UserId::new(), "hi");
        message.sign(server, &identity);
        let proof = crate::message::TombstoneProof::sign(
            server,
            message.channel,
            message.id,
            message.author,
            chrono::Utc::now(),
            &identity,
        );
        let events = [
            ServerEvent::MessageCreate { message: message.clone() },
            message.edit_event(server, "edited", chrono::Utc::now(), &identity),
            proof.into_event(),
        ];
        for event in events {
            let json = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<ServerEvent>(&json).unwrap(), event);
            let bytes = postcard::to_stdvec(&event).unwrap();
            assert_eq!(postcard::from_bytes::<ServerEvent>(&bytes).unwrap(), event);
        }
    }
}
