use crate::identity::Signature;
use crate::ids::{ChannelId, MessageId, RoleId, ServerId, UserId};
use crate::message::Message;
use crate::server::Channel;
use crate::user::{Presence, UserProfile};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Events replicated between peers in a server. Every event is applied
/// to the local `ServerState`, whether it originated locally or arrived
/// over gossip.
///
/// Externally tagged on purpose: internally tagged enums cannot be
/// decoded by postcard, which carries these events on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerEvent {
    MessageCreate {
        message: Message,
    },
    /// `edited_at` is the author's edit time; `sig` signs the whole
    /// message as it reads after the edit.
    MessageEdit {
        channel: ChannelId,
        message: MessageId,
        content: String,
        edited_at: DateTime<Utc>,
        sig: Signature,
    },
    /// Signed by `by`, see [`crate::message::TombstoneProof`].
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
    /// A user joined or left a voice channel. `channel: None` means left.
    /// `since` is when they joined that channel (unix seconds, by their
    /// clock), so everyone shows the same time in channel.
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
    /// A user started or stopped sharing their screen in a voice channel.
    ScreenShare {
        user: UserId,
        channel: ChannelId,
        active: bool,
    },
    /// The server was renamed (owner action).
    ServerRename { server: ServerId, name: String },
    /// A member changed their profile (display name, status, presence).
    ProfileUpdate { profile: UserProfile },
    /// A role was created.
    RoleCreate { server: ServerId, role: crate::server::Role },
    /// A role was deleted; it is removed from every member.
    RoleDelete { server: ServerId, role: RoleId },
    /// A role was granted to a member.
    RoleAssign {
        server: ServerId,
        user: UserId,
        role: RoleId,
    },
    /// A role was taken from a member.
    RoleUnassign {
        server: ServerId,
        user: UserId,
        role: RoleId,
    },
    /// The AFK voice channel was chosen; None turns the AFK channel off.
    /// Appended last so older variants keep their wire tags.
    AfkChannelSet {
        server: ServerId,
        channel: Option<ChannelId>,
    },
    /// A channel was renamed or its message retention changed.
    ChannelUpdate {
        server: ServerId,
        channel: ChannelId,
        name: String,
        retention_days: Option<u32>,
    },
    /// A user turned their camera on or off in a voice channel.
    CameraState {
        user: UserId,
        channel: ChannelId,
        active: bool,
    },
    /// Server avatar and banner changed; values are image refs, None clears.
    ServerImages {
        server: ServerId,
        icon: Option<String>,
        banner: Option<String>,
    },
    /// A user added (active) or removed a reaction. Unlike ReactionToggle
    /// this is idempotent, so redelivered gossip cannot flip it back.
    ReactionSet {
        channel: ChannelId,
        message: MessageId,
        emoji: String,
        user: UserId,
        active: bool,
    },
    /// A custom emoji was added (or replaced by name).
    EmojiAdd { server: ServerId, emoji: crate::server::CustomEmoji },
    EmojiRemove { server: ServerId, name: String },
    /// A soundboard clip was added (or replaced by name).
    SoundAdd { server: ServerId, sound: crate::server::SoundboardSound },
    SoundRemove { server: ServerId, name: String },
    /// Server-wide sharing rules for emojis and sounds.
    ExpressionSettings { server: ServerId, emojis_shareable: bool, soundboard_open: bool },
    /// The server was marked public, or private again.
    PublicSet { server: ServerId, public: bool },
    /// Someone played a soundboard clip in a voice channel. Transient:
    /// peers play it, nothing is stored.
    SoundboardPlay { server: ServerId, channel: ChannelId, user: UserId, name: String },
    /// Games the server is dedicated to.
    GameTagsSet { server: ServerId, tags: Vec<String>, meta: Vec<crate::server::GameTagMeta> },
    /// Rules new members accept before posting; None removes them.
    RulesSet { server: ServerId, rules: Option<String> },
    RulesAccept { server: ServerId, user: UserId },
    /// A calendar event was created or edited. RSVPs travel separately.
    EventUpsert { server: ServerId, event: crate::server::ScheduledEvent },
    EventDelete { server: ServerId, event: String },
    EventRsvp { server: ServerId, event: String, user: UserId, rsvp: Option<crate::server::Rsvp> },
    /// A channel moved to another category or position.
    ChannelLayout { server: ServerId, channel: ChannelId, parent: Option<ChannelId>, position: u32 },
    /// Voice channel member cap; 0 is unlimited.
    ChannelLimit { server: ServerId, channel: ChannelId, user_limit: u32 },
    /// A role's name, color, tag or self-assign setting changed.
    RoleUpdate { server: ServerId, role: crate::server::Role },
    /// A moderator muted or deafened someone in voice for everyone.
    VoiceModerate { server: ServerId, user: UserId, muted: bool, deafened: bool },
    MemberBan { server: ServerId, user: UserId },
    MemberUnban { server: ServerId, user: UserId },
    /// Server-wide voice idle limit; None turns it off.
    AfkTimeoutSet { server: ServerId, minutes: Option<u32> },
    /// A member's device announces where it listens for missed-message
    /// wake-ups and the key that addresses them (see [`crate::doorbell`]);
    /// None turns it off. Signed by `user` over
    /// [`crate::doorbell::set_signing_bytes`]. Devices keep these outside
    /// the replicated state.
    DoorbellSet {
        server: ServerId,
        user: UserId,
        node: String,
        url: String,
        key: Option<crate::doorbell::DoorbellKey>,
        sig: Signature,
    },
    /// A channel's banner image changed; None clears it.
    ChannelBanner {
        server: ServerId,
        channel: ChannelId,
        banner: Option<String>,
    },
    /// A channel's name-band colour changed; None clears it.
    ChannelColor {
        server: ServerId,
        channel: ChannelId,
        color: Option<String>,
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
    fn event_postcard_roundtrip() {
        let event = ServerEvent::TypingStart {
            channel: ChannelId::new(),
            user: UserId::new(),
        };
        let bytes = postcard::to_stdvec(&event).unwrap();
        let back: ServerEvent = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(event, back);
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
