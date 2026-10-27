use crate::identity::Signature;
use crate::ids::{ChannelId, MessageId, RoleId, ServerId, TagId, UserId};
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
    AutoModSet {
        server: ServerId,
        rules: crate::automod::AutoMod,
    },
    BotSet {
        server: ServerId,
        user: UserId,
        bot: bool,
    },
    SoundSharingSet {
        server: ServerId,
        shareable: bool,
    },
    SharedSoundPlay {
        server: ServerId,
        channel: ChannelId,
        user: UserId,
        name: String,
        blob: String,
        volume: f32,
    },
    PronounsSet {
        server: ServerId,
        user: UserId,
        pronouns: Option<String>,
    },
    ServerDelete {
        server: ServerId,
    },
    InviteTrack {
        server: ServerId,
        invite: crate::invites::TrackedInvite,
    },
    InviteRevoke {
        server: ServerId,
        origin: String,
        code: String,
        creator: UserId,
    },
    InviteBackgroundSet {
        server: ServerId,
        background: Option<String>,
    },
    StickerAdd {
        server: ServerId,
        sticker: crate::server::Sticker,
    },
    StickerRemove {
        server: ServerId,
        name: String,
    },
    EmoticonAdd {
        server: ServerId,
        emoticon: crate::server::Emoticon,
    },
    EmoticonRemove {
        server: ServerId,
        name: String,
    },
    ExpressionSharingSet {
        server: ServerId,
        stickers: bool,
        emoticons: bool,
    },
    ChannelAccessSet {
        server: ServerId,
        channel: ChannelId,
        view: Vec<RoleId>,
        send: Vec<RoleId>,
    },
    ChannelKeyAdd {
        server: ServerId,
        channel: ChannelId,
        key: crate::channel_key::KeyRecord,
    },
    ChannelKeyGrant {
        server: ServerId,
        channel: ChannelId,
        user: UserId,
        grant: crate::channel_key::SealedKey,
    },
    MemberTimeout {
        server: ServerId,
        user: UserId,
        by: UserId,
        at: DateTime<Utc>,
        until: Option<DateTime<Utc>>,
    },
    MemberNickname {
        server: ServerId,
        user: UserId,
        nickname: Option<String>,
    },
    OwnerTransfer {
        server: ServerId,
        user: UserId,
    },
    ChannelSlowMode {
        server: ServerId,
        channel: ChannelId,
        seconds: u32,
    },
    PollVote {
        server: ServerId,
        channel: ChannelId,
        message: MessageId,
        user: UserId,
        choices: Vec<u8>,
    },
    PollClose {
        server: ServerId,
        channel: ChannelId,
        message: MessageId,
        by: UserId,
    },
    VoiceMove {
        server: ServerId,
        user: UserId,
        channel: Option<ChannelId>,
    },
    ChannelSensitiveSet {
        server: ServerId,
        channel: ChannelId,
        sensitive: bool,
    },
    OnboardingSet {
        server: ServerId,
        onboarding: Option<crate::onboarding::Onboarding>,
    },
    ThreadSet {
        server: ServerId,
        channel: ChannelId,
        root: MessageId,
        name: Option<String>,
        archived: bool,
        locked: bool,
        by: UserId,
    },
    VoiceStatusSet {
        server: ServerId,
        channel: ChannelId,
        text: String,
        by: UserId,
    },
    ForumTagsSet {
        server: ServerId,
        channel: ChannelId,
        tags: Vec<crate::server::ForumTag>,
        by: UserId,
    },
    ForumPost {
        server: ServerId,
        channel: ChannelId,
        message: MessageId,
        title: String,
        tags: Vec<TagId>,
        by: UserId,
    },
    StageSet {
        server: ServerId,
        channel: ChannelId,
        user: UserId,
        speaker: bool,
        by: UserId,
    },
    HandRaise {
        server: ServerId,
        channel: ChannelId,
        user: UserId,
        raised: bool,
    },
    BotCommandsSet {
        server: ServerId,
        bot: UserId,
        commands: Vec<crate::command::BotCommand>,
        by: UserId,
    },
    ActivitySet {
        server: ServerId,
        channel: ChannelId,
        activity: Option<crate::activity::Activity>,
        by: UserId,
    },
    ActivityInput {
        server: ServerId,
        channel: ChannelId,
        seq: u64,
        input: crate::activity::ActivityInput,
        by: UserId,
    },
    ActivitiesSet {
        server: ServerId,
        enabled: bool,
    },
    DeviceAnnounce {
        server: ServerId,
        user: UserId,
        node: String,
        voice: Option<ChannelId>,
        since: Option<i64>,
    },
    CommunityKeyAdd {
        server: ServerId,
        key: crate::channel_key::KeyRecord,
    },
    CommunityKeyGrant {
        server: ServerId,
        user: UserId,
        grant: crate::channel_key::SealedKey,
    },
    JoinSoundSet {
        server: ServerId,
        user: UserId,
        sound: Option<String>,
    },
    JoinSoundsAllowed {
        server: ServerId,
        allowed: bool,
    },
    ServerTagSet {
        server: ServerId,
        tag: Option<crate::server_tag::ServerTag>,
    },
    JoinApprovalSet {
        server: ServerId,
        on: bool,
        at: i64,
    },
    JoinAsk {
        server: ServerId,
        profile: UserProfile,
        note: String,
        at: i64,
    },
    JoinAnswer {
        server: ServerId,
        user: UserId,
        admit: bool,
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
        let rules = crate::automod::AutoMod::default();
        assert_eq!(wire_tag(ServerEvent::AutoModSet { server, rules }), 48);
        assert_eq!(wire_tag(ServerEvent::BotSet { server, user, bot: true }), 49);
        assert_eq!(wire_tag(ServerEvent::SoundSharingSet { server, shareable: true }), 50);
        let play = ServerEvent::SharedSoundPlay { server, channel, user, name: "n".into(), blob: "b".into(), volume: 1.0 };
        assert_eq!(wire_tag(play), 51);
        assert_eq!(wire_tag(ServerEvent::PronounsSet { server, user, pronouns: None }), 52);
        assert_eq!(wire_tag(ServerEvent::ServerDelete { server }), 53);
        let revoke = ServerEvent::InviteRevoke { server, origin: "o".into(), code: "c".into(), creator: user };
        assert_eq!(wire_tag(revoke), 55);
        assert_eq!(wire_tag(ServerEvent::InviteBackgroundSet { server, background: None }), 56);
        let sharing = ServerEvent::ExpressionSharingSet { server, stickers: true, emoticons: false };
        assert_eq!(wire_tag(sharing), 61);
        assert_eq!(wire_tag(ServerEvent::ChannelAccessSet { server, channel, view: vec![], send: vec![] }), 62);
        let timeout = ServerEvent::MemberTimeout { server, user, by: user, at: chrono::Utc::now(), until: None };
        assert_eq!(wire_tag(timeout), 65);
        assert_eq!(wire_tag(ServerEvent::MemberNickname { server, user, nickname: None }), 66);
        let message = MessageId::new();
        assert_eq!(wire_tag(ServerEvent::OwnerTransfer { server, user }), 67);
        assert_eq!(wire_tag(ServerEvent::ChannelSlowMode { server, channel, seconds: 5 }), 68);
        assert_eq!(wire_tag(ServerEvent::PollVote { server, channel, message, user, choices: vec![0] }), 69);
        assert_eq!(wire_tag(ServerEvent::PollClose { server, channel, message, by: user }), 70);
        assert_eq!(wire_tag(ServerEvent::ChannelSensitiveSet { server, channel, sensitive: true }), 72);
        assert_eq!(wire_tag(ServerEvent::OnboardingSet { server, onboarding: None }), 73);
        let root = crate::ids::MessageId::new();
        let thread = ServerEvent::ThreadSet { server, channel, root, name: None, archived: false, locked: false, by: user };
        assert_eq!(wire_tag(thread), 74);
        assert_eq!(wire_tag(ServerEvent::VoiceStatusSet { server, channel, text: "t".into(), by: user }), 75);
        assert_eq!(wire_tag(ServerEvent::ForumTagsSet { server, channel, tags: vec![], by: user }), 76);
        let post = ServerEvent::ForumPost { server, channel, message: MessageId::new(), title: "t".into(), tags: vec![], by: user };
        assert_eq!(wire_tag(post), 77);
        assert_eq!(wire_tag(ServerEvent::StageSet { server, channel, user, speaker: true, by: user }), 78);
        assert_eq!(wire_tag(ServerEvent::HandRaise { server, channel, user, raised: true }), 79);
        assert_eq!(wire_tag(ServerEvent::BotCommandsSet { server, bot: user, commands: vec![], by: user }), 80);
        assert_eq!(wire_tag(ServerEvent::ActivitySet { server, channel, activity: None, by: user }), 81);
        let input = crate::activity::ActivityInput::Join;
        assert_eq!(wire_tag(ServerEvent::ActivityInput { server, channel, seq: 1, input, by: user }), 82);
        assert_eq!(wire_tag(ServerEvent::ActivitiesSet { server, enabled: true }), 83);
        let device = ServerEvent::DeviceAnnounce { server, user, node: "n".into(), voice: Some(channel), since: Some(1) };
        assert_eq!(wire_tag(device), 84);
        assert_eq!(wire_tag(ServerEvent::JoinSoundSet { server, user, sound: None }), 87);
        assert_eq!(wire_tag(ServerEvent::JoinSoundsAllowed { server, allowed: false }), 88);
        assert_eq!(wire_tag(ServerEvent::ServerTagSet { server, tag: None }), 89);
        assert_eq!(wire_tag(ServerEvent::JoinApprovalSet { server, on: true, at: 1 }), 90);
        let profile = UserProfile::new("asker");
        assert_eq!(wire_tag(ServerEvent::JoinAsk { server, profile, note: String::new(), at: 1 }), 91);
        assert_eq!(wire_tag(ServerEvent::JoinAnswer { server, user, admit: true }), 92);
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
