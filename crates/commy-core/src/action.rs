
use crate::event::ServerEvent;
use crate::identity::{Identity, IdentityKey, Signature};
use crate::ids::{ServerId, UserId};
use crate::permissions::Permissions;
use crate::server::Server;
use serde::{Deserialize, Serialize};

const ACTION_DOMAIN: &str = "commy-action/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub by: UserId,
    pub at: i64,
    pub sig: Signature,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignedEvent {
    pub action: Action,
    pub event: ServerEvent,
}

#[derive(Serialize)]
struct SignedBytes<'a> {
    domain: &'static str,
    server: ServerId,
    by: UserId,
    at: i64,
    event: &'a ServerEvent,
}

pub fn signing_bytes(server: ServerId, by: UserId, at: i64, event: &ServerEvent) -> Vec<u8> {
    postcard::to_stdvec(&SignedBytes { domain: ACTION_DOMAIN, server, by, at, event })
        .expect("action serialization cannot fail")
}

impl SignedEvent {
    pub fn sign(server: ServerId, by: UserId, at: i64, event: ServerEvent, identity: &Identity) -> Self {
        let sig = identity.sign(&signing_bytes(server, by, at, &event));
        Self { action: Action { by, at, sig }, event }
    }

    pub fn verify(&self, server: ServerId, key: &IdentityKey) -> bool {
        let Action { by, at, sig } = self.action;
        key.verify(&signing_bytes(server, by, at, &self.event), &sig)
    }
}

pub fn self_signed(event: &ServerEvent) -> bool {
    matches!(
        event,
        ServerEvent::MessageCreate { .. }
            | ServerEvent::MessageEdit { .. }
            | ServerEvent::MessageDelete { .. }
            | ServerEvent::DoorbellSet { .. }
    )
}

pub fn actor(event: &ServerEvent) -> Option<UserId> {
    use ServerEvent as E;
    match event {
        E::MemberJoin { profile, .. } | E::ProfileUpdate { profile } => Some(profile.id),
        E::PresenceUpdate { user, .. }
        | E::VoiceState { user, .. }
        | E::TypingStart { user, .. }
        | E::ScreenShare { user, .. }
        | E::CameraState { user, .. }
        | E::ReactionToggle { user, .. }
        | E::ReactionSet { user, .. }
        | E::SoundboardPlay { user, .. }
        | E::SharedSoundPlay { user, .. }
        | E::RulesAccept { user, .. }
        | E::EventRsvp { user, .. }
        | E::DoorbellSet { user, .. }
        | E::BotSet { user, .. }
        | E::PollVote { user, .. }
        | E::PronounsSet { user, .. } => Some(*user),
        E::MessagePin { by, .. }
        | E::MemberTimeout { by, .. }
        | E::PollClose { by, .. }
        | E::ThreadSet { by, .. }
        | E::VoiceStatusSet { by, .. } => Some(*by),
        _ => None,
    }
}

pub fn may_go_unsigned(event: &ServerEvent) -> bool {
    use ServerEvent as E;
    match event {
        E::MemberJoin { profile, .. } | E::ProfileUpdate { profile } => profile.identity.is_none(),
        E::PresenceUpdate { .. }
        | E::TypingStart { .. }
        | E::ReactionSet { .. } => true,
        _ => false,
    }
}

pub fn subject(event: &ServerEvent) -> Option<String> {
    use ServerEvent as E;
    let s = match event {
        E::MemberJoin { profile, .. } => format!("member:{}", profile.id),
        E::MemberLeave { user, .. } => format!("member:{user}"),
        E::MemberBan { user, .. } | E::MemberUnban { user, .. } => format!("ban:{user}"),
        E::ProfileUpdate { profile } => format!("profile:{}", profile.id),
        E::ChannelCreate { channel, .. } => format!("channel:{}", channel.id),
        E::ChannelDelete { channel, .. } => format!("channel:{channel}"),
        E::ChannelUpdate { channel, .. } => format!("channel.name:{channel}"),
        E::ChannelLayout { channel, .. } => format!("channel.layout:{channel}"),
        E::ChannelLimit { channel, .. } => format!("channel.limit:{channel}"),
        E::ChannelSlowMode { channel, .. } => format!("channel.slow_mode:{channel}"),
        E::ThreadSet { channel, root, .. } => format!("thread:{channel}:{root}"),
        E::ChannelBanner { channel, .. } => format!("channel.banner:{channel}"),
        E::ChannelColor { channel, .. } => format!("channel.color:{channel}"),
        E::ChannelKindSet { channel, .. } => format!("channel.kind:{channel}"),
        E::ChannelAccessSet { channel, .. } => format!("channel.access:{channel}"),
        E::ChannelSensitiveSet { channel, .. } => format!("channel.sensitive:{channel}"),
        E::VoiceStatusSet { channel, .. } => format!("voice-status:{channel}"),
        E::ChannelKeyAdd { channel, key, .. } => format!("channel.key:{channel}:{}", key.id.to_hex()),
        E::ChannelKeyGrant { channel, user, .. } => format!("channel.grant:{channel}:{user}"),
        E::AfkChannelSet { .. } => "afk_channel".into(),
        E::AfkTimeoutSet { .. } => "afk_timeout".into(),
        E::ServerRename { .. } => "name".into(),
        E::ServerImages { .. } => "images".into(),
        E::PublicSet { .. } => "public".into(),
        E::GameTagsSet { .. } => "game_tags".into(),
        E::RulesSet { .. } => "rules".into(),
        E::OnboardingSet { .. } => "onboarding".into(),
        E::AutoModSet { .. } => "automod".into(),
        E::BotSet { user, .. } => format!("bot:{user}"),
        E::PronounsSet { user, .. } => format!("pronouns:{user}"),
        E::RulesAccept { user, .. } => format!("rules_accept:{user}"),
        E::ExpressionSettings { .. } => "expressions".into(),
        E::SoundSharingSet { .. } => "sound_sharing".into(),
        E::ExpressionSharingSet { .. } => "expression_sharing".into(),
        E::StickerAdd { sticker, .. } => format!("sticker:{}", sticker.name),
        E::StickerRemove { name, .. } => format!("sticker:{name}"),
        E::EmoticonAdd { emoticon, .. } => format!("emoticon:{}", emoticon.name),
        E::EmoticonRemove { name, .. } => format!("emoticon:{name}"),
        E::ServerDelete { .. } => "deleted".into(),
        E::OwnerTransfer { .. } => "owner".into(),
        E::InviteTrack { invite, .. } => format!("invite:{}", invite.id()),
        E::InviteRevoke { origin, code, .. } => format!("invite:{}", crate::invites::invite_id(origin, code)),
        E::InviteBackgroundSet { .. } => "invite_background".into(),
        E::EmojiAdd { emoji, .. } => format!("emoji:{}", emoji.name),
        E::EmojiRemove { name, .. } => format!("emoji:{name}"),
        E::SoundAdd { sound, .. } => format!("sound:{}", sound.name),
        E::SoundRemove { name, .. } => format!("sound:{name}"),
        E::EventUpsert { event, .. } => format!("event:{}", event.id),
        E::EventDelete { event, .. } => format!("event:{event}"),
        E::EventRsvp { event, user, .. } => format!("rsvp:{event}:{user}"),
        E::RoleCreate { role, .. } | E::RoleUpdate { role, .. } => format!("role:{}", role.id),
        E::RoleDelete { role, .. } => format!("role:{role}"),
        E::RoleAssign { user, role, .. } | E::RoleUnassign { user, role, .. } => format!("role:{role}:{user}"),
        E::VoiceModerate { user, .. } => format!("voice_moderate:{user}"),
        E::MemberTimeout { user, .. } => format!("timeout:{user}"),
        E::MemberNickname { user, .. } => format!("nickname:{user}"),
        E::VoiceMove { user, .. } => format!("voice_move:{user}"),
        E::MessagePin { message, .. } => format!("pin:{message}"),
        E::PollVote { message, user, .. } => format!("vote:{message}:{user}"),
        E::PollClose { message, .. } => format!("poll:{message}"),
        E::ReactionToggle { message, emoji, user, .. } | E::ReactionSet { message, emoji, user, .. } => {
            format!("reaction:{message}:{emoji}:{user}")
        }
        E::VoiceState { user, .. } => format!("voice:{user}"),
        E::ScreenShare { user, .. } => format!("share:{user}"),
        E::CameraState { user, .. } => format!("camera:{user}"),
        E::PresenceUpdate { user, .. } => format!("presence:{user}"),
        E::MessageCreate { .. }
        | E::MessageEdit { .. }
        | E::MessageDelete { .. }
        | E::DoorbellSet { .. }
        | E::TypingStart { .. }
        | E::SoundboardPlay { .. }
        | E::SharedSoundPlay { .. } => return None,
    };
    Some(s)
}

pub fn settles_authority(subject: &str) -> bool {
    const LIVE: [&str; 8] = ["presence:", "voice:", "share:", "camera:", "reaction:", "rsvp:", "profile:", "voice_move:"];
    !LIVE.iter().any(|p| subject.starts_with(p))
}

pub fn snapshot_omits(subject: &str) -> bool {
    const OMITTED: [&str; 13] = [
        "emoji:", "sticker:", "emoticon:", "sound:", "event:", "pin:", "bot:", "invite:", "channel.key:", "channel.grant:", "timeout:", "vote:", "poll:",
    ];
    const SINGLE: [&str; 4] = ["rules", "automod", "deleted", "invite_background"];
    SINGLE.contains(&subject) || OMITTED.iter().any(|p| subject.starts_with(p))
}

pub fn relayable(event: &ServerEvent) -> bool {
    !matches!(event, ServerEvent::MemberJoin { .. }) && subject(event).is_some_and(|s| settles_authority(&s))
}

fn may_add(perms: Permissions, member: bool, kind: Permissions, taken: bool) -> bool {
    perms.allows(Permissions::MANAGE_EXPRESSIONS) || (member && !taken && perms.allows(kind))
}

fn channel_of(event: &ServerEvent) -> Option<crate::ids::ChannelId> {
    use ServerEvent as E;
    match event {
        E::VoiceState { channel, .. } | E::VoiceMove { channel, .. } => *channel,
        E::TypingStart { channel, .. }
        | E::ScreenShare { channel, .. }
        | E::CameraState { channel, .. }
        | E::ReactionToggle { channel, .. }
        | E::ReactionSet { channel, .. }
        | E::SoundboardPlay { channel, .. }
        | E::SharedSoundPlay { channel, .. }
        | E::PollVote { channel, .. }
        | E::PollClose { channel, .. }
        | E::MessagePin { channel, .. }
        | E::ThreadSet { channel, .. }
        | E::VoiceStatusSet { channel, .. } => Some(*channel),
        _ => None,
    }
}

fn replies_into_locked_thread(server: &Server, message: &crate::message::Message) -> bool {
    let Some(parent) = message.reply_to else { return false };
    server.channel(message.channel).and_then(|c| c.threads.get(&parent)).is_some_and(|t| t.locked)
}

pub fn authorized(server: &Server, by: UserId, event: &ServerEvent) -> bool {
    use ServerEvent as E;
    let perms = server.permissions_for(by);
    let allows = |p: Permissions| perms.allows(p);
    let member = server.is_member(by);
    let sees = |event: &ServerEvent| channel_of(event).is_none_or(|channel| server.can_view(channel, by));
    let speaks = !server.is_timed_out(by, chrono::Utc::now());
    match event {
        E::MessageCreate { message } => allows(Permissions::MANAGE_MESSAGES) || !replies_into_locked_thread(server, message),
        E::MessageEdit { .. } | E::MessageDelete { .. } | E::DoorbellSet { .. } => true,
        E::MemberJoin { .. } => actor(event) == Some(by),
        E::MemberLeave { user, .. } => *user == by || (allows(Permissions::KICK_MEMBERS) && *user != server.owner),
        E::ProfileUpdate { .. } => actor(event) == Some(by),
        E::BotSet { .. } | E::PronounsSet { .. } => member && actor(event) == Some(by),
        E::PresenceUpdate { .. } | E::RulesAccept { .. } | E::EventRsvp { .. } => member && actor(event) == Some(by),
        E::VoiceState { .. } | E::ScreenShare { .. } | E::CameraState { .. } => member && actor(event) == Some(by) && sees(event),
        E::TypingStart { .. } | E::ReactionToggle { .. } | E::ReactionSet { .. } => {
            member && speaks && actor(event) == Some(by) && sees(event)
        }
        E::SoundboardPlay { .. } | E::SharedSoundPlay { .. } => {
            member && speaks && actor(event) == Some(by) && allows(Permissions::USE_SOUNDBOARD) && sees(event)
        }
        E::MessagePin { .. } => actor(event) == Some(by) && perms.can_pin() && sees(event),
        E::PollVote { .. } => member && speaks && actor(event) == Some(by) && sees(event),
        E::PollClose { .. } => member && actor(event) == Some(by) && sees(event),
        E::ThreadSet { root, .. } => {
            member && actor(event) == Some(by) && sees(event) && (root.is_bound_to(by) || allows(Permissions::MANAGE_MESSAGES))
        }
        E::VoiceStatusSet { .. } => {
            member && speaks && actor(event) == Some(by) && sees(event) && perms.can_set_voice_status(true)
        }
        E::ChannelKeyAdd { channel, .. } | E::ChannelKeyGrant { channel, .. } => {
            member && server.channel(*channel).is_some_and(|c| c.is_private()) && server.can_view(*channel, by)
        }
        E::ChannelCreate { .. }
        | E::ChannelDelete { .. }
        | E::ChannelUpdate { .. }
        | E::ChannelLayout { .. }
        | E::ChannelLimit { .. }
        | E::ChannelSlowMode { .. }
        | E::ChannelBanner { .. }
        | E::ChannelColor { .. }
        | E::ChannelKindSet { .. }
        | E::ChannelAccessSet { .. }
        | E::ChannelSensitiveSet { .. } => allows(Permissions::MANAGE_CHANNELS),
        E::ServerRename { .. }
        | E::ServerImages { .. }
        | E::PublicSet { .. }
        | E::GameTagsSet { .. }
        | E::RulesSet { .. }
        | E::OnboardingSet { .. }
        | E::AutoModSet { .. }
        | E::AfkChannelSet { .. }
        | E::AfkTimeoutSet { .. }
        | E::ExpressionSettings { .. }
        | E::SoundSharingSet { .. }
        | E::ExpressionSharingSet { .. } => allows(Permissions::MANAGE_SERVER),
        E::RoleCreate { .. } | E::RoleDelete { .. } | E::RoleUpdate { .. } => allows(Permissions::MANAGE_ROLES),
        E::RoleAssign { user, role, .. } | E::RoleUnassign { user, role, .. } => {
            allows(Permissions::MANAGE_ROLES)
                || (*user == by && server.roles.iter().any(|r| r.id == *role && r.self_assignable))
                || (*user == by && crate::onboarding::may_self_assign(server, by, *role))
        }
        E::EmojiRemove { .. } | E::StickerRemove { .. } | E::EmoticonRemove { .. } | E::SoundRemove { .. } => {
            allows(Permissions::MANAGE_EXPRESSIONS)
        }
        E::EmojiAdd { emoji, .. } => {
            let taken = server.emojis.iter().any(|e| e.name == emoji.name);
            may_add(perms, member, Permissions::ADD_EMOJIS, taken)
        }
        E::StickerAdd { sticker, .. } => {
            let taken = server.stickers.iter().any(|s| s.name == sticker.name);
            may_add(perms, member, Permissions::ADD_STICKERS, taken)
        }
        E::EmoticonAdd { emoticon, .. } => {
            let taken = server.emoticons.iter().any(|e| e.name == emoticon.name);
            may_add(perms, member, Permissions::ADD_EMOTICONS, taken)
        }
        E::SoundAdd { sound, .. } => {
            allows(Permissions::MANAGE_EXPRESSIONS)
                || (server.soundboard_open && member && !server.soundboard.iter().any(|s| s.name == sound.name))
        }
        E::EventUpsert { event, .. } => {
            allows(Permissions::MANAGE_EVENTS)
                || (event.creator == by && server.events.iter().any(|e| e.id == event.id && e.creator == by))
        }
        E::EventDelete { event, .. } => {
            allows(Permissions::MANAGE_EVENTS) || server.events.iter().any(|e| e.id == *event && e.creator == by)
        }
        E::VoiceModerate { .. } => allows(Permissions::MUTE_MEMBERS) || allows(Permissions::DEAFEN_MEMBERS),
        E::MemberNickname { user, .. } => {
            let own = *user == by && allows(Permissions::CHANGE_NICKNAME);
            let others = *user != server.owner && allows(Permissions::MANAGE_NICKNAMES);
            member && (own || others)
        }
        E::VoiceMove { user, .. } => allows(Permissions::MOVE_MEMBERS) && *user != by && sees(event),
        E::MemberBan { .. } | E::MemberUnban { .. } => allows(Permissions::BAN_MEMBERS),
        E::MemberTimeout { user, .. } => *user != by && actor(event) == Some(by) && allows(Permissions::TIME_OUT_MEMBERS),
        E::ServerDelete { .. } => by == server.owner,
        E::OwnerTransfer { user, .. } => by == server.owner && *user != by && server.is_member(*user),
        E::InviteTrack { invite, .. } => member && invite.creator == by && allows(Permissions::CREATE_INVITES),
        E::InviteRevoke { creator, .. } => allows(Permissions::MANAGE_SERVER) || (member && *creator == by),
        E::InviteBackgroundSet { .. } => allows(Permissions::MANAGE_SERVER),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ChannelId;
    use crate::ids::RoleId;
    use crate::server::{Channel, ChannelKind, Role};
    use crate::test_support::{fixture, identity_for, join};
    use crate::user::{Presence, UserProfile};
    use crate::store::StoreError;

    fn role(perms: Permissions, self_assignable: bool) -> Role {
        Role {
            id: RoleId::new(),
            name: "r".into(),
            color: None,
            permissions: perms,
            position: 1,
            hoist: false,
            self_assignable,
            mentionable: false,
            tag: None,
            banner: None,
        }
    }

    #[test]
    fn signature_covers_server_actor_time_and_event() {
        let server = ServerId::new();
        let me = UserId::new();
        let identity = identity_for(me);
        let event = ServerEvent::ServerRename { server, name: "new".into() };
        let signed = SignedEvent::sign(server, me, 1_000, event.clone(), &identity);
        assert!(signed.verify(server, &identity.public()));
        assert!(!signed.verify(ServerId::new(), &identity.public()), "another server");
        assert!(!signed.verify(server, &Identity::generate().public()), "another key");
        let mut other = signed.clone();
        other.action.at += 1;
        assert!(!other.verify(server, &identity.public()), "another time");
        let mut other = signed.clone();
        other.action.by = UserId::new();
        assert!(!other.verify(server, &identity.public()), "another actor");
        let mut other = signed;
        other.event = ServerEvent::ServerRename { server, name: "evil".into() };
        assert!(!other.verify(server, &identity.public()), "another event");
    }

    #[test]
    fn signed_events_roundtrip_both_formats() {
        let server = ServerId::new();
        let me = UserId::new();
        let event = ServerEvent::PublicSet { server, public: true };
        let signed = SignedEvent::sign(server, me, 7, event, &identity_for(me));
        let json = serde_json::to_string(&signed).unwrap();
        assert_eq!(serde_json::from_str::<SignedEvent>(&json).unwrap(), signed);
        let bytes = postcard::to_stdvec(&signed).unwrap();
        assert_eq!(postcard::from_bytes::<SignedEvent>(&bytes).unwrap(), signed);
    }

    #[test]
    fn bare_events_are_the_self_signed_and_the_keyless_live_ones() {
        let server = ServerId::new();
        let keyless = UserProfile::new("new");
        let mut keyed = UserProfile::new("old");
        keyed.identity = Some(Identity::generate().public());
        assert!(may_go_unsigned(&ServerEvent::MemberJoin { server, profile: keyless.clone() }));
        assert!(!may_go_unsigned(&ServerEvent::MemberJoin { server, profile: keyed.clone() }));
        assert!(may_go_unsigned(&ServerEvent::ProfileUpdate { profile: keyless }));
        assert!(!may_go_unsigned(&ServerEvent::ProfileUpdate { profile: keyed }));
        assert!(may_go_unsigned(&ServerEvent::PresenceUpdate { user: UserId::new(), presence: Presence::Online }));
        assert!(!may_go_unsigned(&ServerEvent::ServerRename { server, name: "x".into() }));
        assert!(!may_go_unsigned(&ServerEvent::MemberBan { server, user: UserId::new() }));
        let message = crate::message::Message::new(ChannelId::new(), UserId::new(), "hi");
        assert!(self_signed(&ServerEvent::MessageCreate { message }));
        assert!(!self_signed(&ServerEvent::ServerRename { server, name: "x".into() }));
    }

    #[test]
    fn subjects_pair_the_changes_that_undo_each_other() {
        let server = ServerId::new();
        let user = UserId::new();
        let role = RoleId::new();
        let pair = |a: ServerEvent, b: ServerEvent| {
            assert!(subject(&a).is_some());
            assert_eq!(subject(&a), subject(&b));
        };
        pair(ServerEvent::MemberBan { server, user }, ServerEvent::MemberUnban { server, user });
        pair(ServerEvent::RoleAssign { server, user, role }, ServerEvent::RoleUnassign { server, user, role });
        let channel = Channel::new("c", ChannelKind::Text, 3);
        pair(
            ServerEvent::ChannelCreate { server, channel: channel.clone() },
            ServerEvent::ChannelDelete { server, channel: channel.id },
        );
        assert_ne!(
            subject(&ServerEvent::MemberBan { server, user }),
            subject(&ServerEvent::MemberBan { server, user: UserId::new() })
        );
        assert!(subject(&ServerEvent::VoiceState { user, channel: None, muted: false, deafened: false, since: None }).is_some());
        assert_eq!(subject(&ServerEvent::TypingStart { channel: channel.id, user }), None);
    }

    #[test]
    fn owner_and_administrators_may_do_anything() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let admin = join(&mut state, "admin");
        let plain = join(&mut state, "plain");
        let admin_role = role(Permissions::ADMINISTRATOR, false);
        let rid = admin_role.id;
        state.apply(ServerEvent::RoleCreate { server: sid, role: admin_role }).unwrap();
        state.apply(ServerEvent::RoleAssign { server: sid, user: admin, role: rid }).unwrap();
        let events = [
            ServerEvent::ServerRename { server: sid, name: "n".into() },
            ServerEvent::MemberBan { server: sid, user: plain },
            ServerEvent::ChannelDelete { server: sid, channel: state.server.channels[0].id },
            ServerEvent::RoleDelete { server: sid, role: rid },
            ServerEvent::MemberLeave { server: sid, user: admin },
            ServerEvent::VoiceModerate { server: sid, user: plain, muted: true, deafened: false },
            ServerEvent::MemberNickname { server: sid, user: admin, nickname: Some("Ad".into()) },
            ServerEvent::VoiceMove { server: sid, user: plain, channel: None },
        ];
        for event in &events {
            assert!(authorized(&state.server, owner, event), "{event:?}");
            assert!(authorized(&state.server, admin, event), "{event:?}");
            assert!(!authorized(&state.server, plain, event), "{event:?}");
            assert!(!authorized(&state.server, UserId::new(), event), "stranger {event:?}");
        }
    }

    #[test]
    fn only_the_owner_hands_the_server_over_and_only_to_a_member() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let admin = join(&mut state, "admin");
        let plain = join(&mut state, "plain");
        let admin_role = role(Permissions::ADMINISTRATOR, false);
        let rid = admin_role.id;
        state.apply(ServerEvent::RoleCreate { server: sid, role: admin_role }).unwrap();
        state.apply(ServerEvent::RoleAssign { server: sid, user: admin, role: rid }).unwrap();
        let transfer = |user| ServerEvent::OwnerTransfer { server: sid, user };
        assert!(authorized(&state.server, owner, &transfer(plain)));
        assert!(!authorized(&state.server, admin, &transfer(admin)), "administrators cannot take the server");
        assert!(!authorized(&state.server, owner, &transfer(owner)));
        assert!(!authorized(&state.server, owner, &transfer(UserId::new())), "only to a member");
        assert_eq!(state.apply(transfer(plain)), Ok(true));
        assert_eq!(state.server.owner, plain);
        assert_eq!(state.apply(transfer(plain)), Ok(false));
        assert!(authorized(&state.server, plain, &ServerEvent::ServerDelete { server: sid }));
        assert!(!authorized(&state.server, owner, &ServerEvent::ServerDelete { server: sid }), "the old owner is a member now");
    }

    #[test]
    fn votes_come_from_members_who_see_the_channel_and_are_not_timed_out() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let a = join(&mut state, "a");
        let b = join(&mut state, "b");
        let message = crate::ids::MessageId::new();
        let vote = |user| ServerEvent::PollVote { server: sid, channel, message, user, choices: vec![0] };
        let close = |by| ServerEvent::PollClose { server: sid, channel, message, by };
        assert!(authorized(&state.server, a, &vote(a)));
        assert!(authorized(&state.server, a, &close(a)));
        assert!(!authorized(&state.server, a, &vote(b)), "nobody votes for someone else");
        assert!(!authorized(&state.server, b, &close(a)));
        assert!(!authorized(&state.server, UserId::new(), &vote(UserId::new())), "strangers do not vote");
        let until = Some(chrono::Utc::now() + chrono::Duration::minutes(10));
        let timeout = ServerEvent::MemberTimeout { server: sid, user: a, by: owner, at: chrono::Utc::now(), until };
        state.apply(timeout).unwrap();
        assert!(!authorized(&state.server, a, &vote(a)), "timed out members sit polls out");
        assert!(authorized(&state.server, a, &close(a)), "but may still close their own poll");
        let hidden = ChannelId::new();
        let elsewhere = ServerEvent::PollVote { server: sid, channel: hidden, message, user: b, choices: vec![0] };
        assert!(!authorized(&state.server, b, &elsewhere), "a channel they cannot see");
    }

    #[test]
    fn members_act_only_for_themselves() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let a = join(&mut state, "a");
        let b = join(&mut state, "b");
        let presence = |user| ServerEvent::PresenceUpdate { user, presence: Presence::Idle };
        assert!(authorized(&state.server, a, &presence(a)));
        assert!(!authorized(&state.server, a, &presence(b)));
        assert!(!authorized(&state.server, owner, &presence(b)), "not even the owner speaks for others");
        let leave = |user| ServerEvent::MemberLeave { server: sid, user };
        assert!(authorized(&state.server, a, &leave(a)));
        assert!(!authorized(&state.server, a, &leave(b)));
        assert!(authorized(&state.server, owner, &leave(b)), "a kick");
        let profile = state.profiles[&b].clone();
        assert!(authorized(&state.server, b, &ServerEvent::ProfileUpdate { profile: profile.clone() }));
        assert!(!authorized(&state.server, a, &ServerEvent::ProfileUpdate { profile }));
        let voice = |user| ServerEvent::VoiceState { user, channel: Some(channel), muted: false, deafened: false, since: None };
        assert!(authorized(&state.server, a, &voice(a)));
        assert!(!authorized(&state.server, b, &voice(a)));
        let pin = |by| ServerEvent::MessagePin { server: sid, channel, message: crate::ids::MessageId::new(), by, at: chrono::Utc::now(), pinned: true };
        assert!(authorized(&state.server, owner, &pin(owner)));
        assert!(!authorized(&state.server, a, &pin(a)), "no pin permission");
        assert!(!authorized(&state.server, owner, &pin(a)), "pins name their pinner");
    }

    #[test]
    fn nicknames_are_your_own_unless_you_manage_them() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let a = join(&mut state, "a");
        let b = join(&mut state, "b");
        let nick = |user| ServerEvent::MemberNickname { server: sid, user, nickname: Some("Nick".into()) };
        assert!(authorized(&state.server, a, &nick(a)), "members may rename themselves");
        assert!(!authorized(&state.server, a, &nick(b)));
        assert!(!authorized(&state.server, a, &nick(owner)));
        assert!(authorized(&state.server, owner, &nick(a)));
        assert!(authorized(&state.server, owner, &nick(owner)));
        assert!(!authorized(&state.server, UserId::new(), &nick(a)), "strangers");
        let managers = role(Permissions::MANAGE_NICKNAMES, false);
        let rid = managers.id;
        state.apply(ServerEvent::RoleCreate { server: sid, role: managers }).unwrap();
        state.apply(ServerEvent::RoleAssign { server: sid, user: b, role: rid }).unwrap();
        assert!(authorized(&state.server, b, &nick(a)));
        assert!(!authorized(&state.server, b, &nick(owner)), "only the owner renames the owner");
        let everyone = state.server.roles.iter_mut().find(|r| r.is_everyone()).unwrap();
        everyone.permissions.remove(Permissions::CHANGE_NICKNAME);
        assert!(!authorized(&state.server, a, &nick(a)), "no change nickname permission");
        assert!(authorized(&state.server, b, &nick(b)), "managing covers one's own");
    }

    #[test]
    fn voice_status_comes_from_speaking_members_or_channel_managers() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let voice = state.server.channels[1].id;
        let a = join(&mut state, "a");
        let b = join(&mut state, "b");
        let set = |by| ServerEvent::VoiceStatusSet { server: sid, channel: voice, text: "Ranked grind".into(), by };
        assert!(authorized(&state.server, a, &set(a)));
        assert!(authorized(&state.server, owner, &set(owner)));
        assert!(!authorized(&state.server, a, &set(b)), "statuses name who set them");
        assert!(!authorized(&state.server, UserId::new(), &set(UserId::new())), "strangers");
        assert_eq!(subject(&set(a)), Some(format!("voice-status:{voice}")));
        assert_eq!(subject(&set(a)), subject(&set(b)));
        assert!(settles_authority(&subject(&set(a)).unwrap()));
        assert!(!snapshot_omits(&subject(&set(a)).unwrap()));
        let everyone = state.server.roles.iter_mut().find(|r| r.is_everyone()).unwrap();
        everyone.permissions.remove(Permissions::SPEAK);
        assert!(!authorized(&state.server, a, &set(a)), "no speak permission");
        let managers = role(Permissions::MANAGE_CHANNELS, false);
        let rid = managers.id;
        state.apply(ServerEvent::RoleCreate { server: sid, role: managers }).unwrap();
        state.apply(ServerEvent::RoleAssign { server: sid, user: b, role: rid }).unwrap();
        assert!(authorized(&state.server, b, &set(b)), "channel managers need no voice");
        let until = Some(chrono::Utc::now() + chrono::Duration::minutes(10));
        state.apply(ServerEvent::MemberTimeout { server: sid, user: a, by: owner, at: chrono::Utc::now(), until }).unwrap();
        let everyone = state.server.roles.iter_mut().find(|r| r.is_everyone()).unwrap();
        everyone.permissions.insert(Permissions::SPEAK);
        assert!(!authorized(&state.server, a, &set(a)), "timed out members stay quiet");
        let hidden = ServerEvent::VoiceStatusSet { server: sid, channel: ChannelId::new(), text: "x".into(), by: b };
        assert!(!authorized(&state.server, b, &hidden), "a channel they cannot see");
    }

    #[test]
    fn roles_grant_what_they_say() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let mod_ = join(&mut state, "mod");
        let user = join(&mut state, "user");
        let mods = role(Permissions::BAN_MEMBERS | Permissions::MANAGE_CHANNELS, false);
        let picks = role(Permissions::NONE, true);
        let (mods_id, picks_id) = (mods.id, picks.id);
        state.apply(ServerEvent::RoleCreate { server: sid, role: mods }).unwrap();
        state.apply(ServerEvent::RoleCreate { server: sid, role: picks }).unwrap();
        state.apply(ServerEvent::RoleAssign { server: sid, user: mod_, role: mods_id }).unwrap();
        let ban = ServerEvent::MemberBan { server: sid, user };
        assert!(authorized(&state.server, mod_, &ban));
        assert!(!authorized(&state.server, user, &ban));
        let rename = ServerEvent::ServerRename { server: sid, name: "x".into() };
        assert!(!authorized(&state.server, mod_, &rename), "manage channels is not manage server");
        let grant = |who, role| ServerEvent::RoleAssign { server: sid, user: who, role };
        assert!(!authorized(&state.server, mod_, &grant(user, mods_id)), "no manage roles");
        assert!(authorized(&state.server, user, &grant(user, picks_id)), "self-assignable");
        assert!(!authorized(&state.server, user, &grant(user, mods_id)), "not that one");
        assert!(!authorized(&state.server, user, &grant(mod_, picks_id)), "only for oneself");
        assert!(authorized(&state.server, user, &ServerEvent::RoleUnassign { server: sid, user, role: picks_id }));
    }

    #[test]
    fn onboarding_answers_let_members_take_only_the_roles_they_hand_out() {
        use crate::onboarding::{Answer, Onboarding, Question};
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let user = join(&mut state, "user");
        let other = join(&mut state, "other");
        let gamers = role(Permissions::NONE, false);
        let artists = role(Permissions::NONE, false);
        let admins = role(Permissions::ADMINISTRATOR, false);
        let (gamers_id, artists_id, admins_id) = (gamers.id, artists.id, admins.id);
        for r in [gamers, artists, admins] {
            state.apply(ServerEvent::RoleCreate { server: sid, role: r }).unwrap();
        }
        let questions = Onboarding {
            questions: vec![Question {
                title: "What do you do?".into(),
                description: String::new(),
                multiple: true,
                answers: vec![
                    Answer { label: "Play".into(), roles: vec![gamers_id] },
                    Answer { label: "Run things".into(), roles: vec![admins_id] },
                ],
            }],
        };
        let set = ServerEvent::OnboardingSet { server: sid, onboarding: Some(questions) };
        assert!(authorized(&state.server, owner, &set));
        assert!(!authorized(&state.server, user, &set), "manage server only");
        let grant = |who, role| ServerEvent::RoleAssign { server: sid, user: who, role };
        assert!(!authorized(&state.server, user, &grant(user, gamers_id)), "before any questions exist");
        state.apply(set).unwrap();
        assert!(authorized(&state.server, user, &grant(user, gamers_id)));
        assert!(authorized(&state.server, user, &ServerEvent::RoleUnassign { server: sid, user, role: gamers_id }));
        assert!(!authorized(&state.server, user, &grant(user, artists_id)), "no answer grants it");
        assert!(!authorized(&state.server, user, &grant(user, admins_id)), "never administrator");
        assert!(!authorized(&state.server, user, &grant(other, gamers_id)), "only for oneself");
        assert!(!authorized(&state.server, UserId::new(), &grant(UserId::new(), gamers_id)), "members only");
        let signed = SignedEvent::sign(sid, user, 1_000, grant(user, gamers_id), &identity_for(user));
        assert_eq!(state.apply_signed(&signed), Ok(true));
        assert!(state.server.member(user).unwrap().roles.contains(&gamers_id));
        let forged = SignedEvent::sign(sid, user, 2_000, grant(user, admins_id), &identity_for(user));
        assert_eq!(state.apply_signed(&forged), Err(StoreError::NotAllowed));
        assert_eq!(subject(&ServerEvent::OnboardingSet { server: sid, onboarding: None }).as_deref(), Some("onboarding"));
        assert!(!snapshot_omits("onboarding"), "the server snapshot carries the questions");
    }

    #[test]
    fn open_soundboards_and_own_events() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let user = join(&mut state, "user");
        let sound = |name: &str| crate::server::SoundboardSound { name: name.into(), blob: format!("{}@{}", "a1".repeat(32), "b2".repeat(32)), volume: 1.0, emoji: None };
        let add = ServerEvent::SoundAdd { server: sid, sound: sound("boom") };
        assert!(!authorized(&state.server, user, &add));
        state.server.soundboard_open = true;
        assert!(authorized(&state.server, user, &add));
        assert!(!authorized(&state.server, UserId::new(), &add), "members only");
        state.apply(add.clone()).unwrap();
        assert!(!authorized(&state.server, user, &add), "replacing takes manage expressions");
        assert!(!authorized(&state.server, user, &ServerEvent::SoundRemove { server: sid, name: "boom".into() }));
        let event = crate::server::ScheduledEvent {
            id: "e1".into(),
            title: "night".into(),
            description: String::new(),
            start: chrono::Utc::now(),
            end: None,
            channel: Some(channel),
            creator: user,
            rsvps: Vec::new(),
        };
        let upsert = ServerEvent::EventUpsert { server: sid, event: event.clone() };
        assert!(!authorized(&state.server, user, &upsert), "creating takes manage events");
        assert!(authorized(&state.server, owner, &upsert));
        state.apply(upsert.clone()).unwrap();
        assert!(authorized(&state.server, user, &upsert), "editing one's own");
        assert!(authorized(&state.server, user, &ServerEvent::EventDelete { server: sid, event: "e1".into() }));
        let other = join(&mut state, "other");
        assert!(!authorized(&state.server, other, &upsert));
        assert!(authorized(&state.server, user, &ServerEvent::EventRsvp { server: sid, event: "e1".into(), user, rsvp: None }));
        assert!(!authorized(&state.server, other, &ServerEvent::EventRsvp { server: sid, event: "e1".into(), user, rsvp: None }));
    }

    #[test]
    fn the_store_takes_signed_actions_from_members_allowed_to_make_them() {

        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let user = join(&mut state, "user");
        let victim = join(&mut state, "victim");
        let ban = ServerEvent::MemberBan { server: sid, user: victim };
        let forged = SignedEvent::sign(sid, user, 1_000, ban.clone(), &identity_for(user));
        assert_eq!(state.apply_signed(&forged), Err(StoreError::NotAllowed));
        assert!(state.server.is_member(victim));
        let as_owner = SignedEvent::sign(sid, owner, 1_000, ban.clone(), &identity_for(user));
        assert_eq!(state.apply_signed(&as_owner), Err(StoreError::BadSignature));
        let real = SignedEvent::sign(sid, owner, 1_000, ban, &identity_for(owner));
        assert_eq!(state.apply_signed(&real), Ok(true));
        assert!(!state.server.is_member(victim));
        assert_eq!(state.apply_signed(&real), Ok(false), "a copy changes nothing");
        let stranger = UserId::new();
        let unknown = SignedEvent::sign(sid, stranger, 1_000, ServerEvent::ServerRename { server: sid, name: "x".into() }, &identity_for(stranger));
        assert_eq!(state.apply_signed(&unknown), Err(StoreError::UnknownAuthorKey(stranger)));
        let far = SignedEvent::sign(sid, owner, chrono::Utc::now().timestamp_millis() + 2 * crate::store::MAX_AHEAD_MS, ServerEvent::ServerRename { server: sid, name: "x".into() }, &identity_for(owner));
        assert_eq!(state.apply_signed(&far), Err(StoreError::TooFarAhead));
    }

    #[test]
    fn an_older_change_to_the_same_thing_loses_wherever_it_arrives() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let user = join(&mut state, "user");
        let ban = SignedEvent::sign(sid, owner, 1_000, ServerEvent::MemberBan { server: sid, user }, &identity_for(owner));
        let unban = SignedEvent::sign(sid, owner, 2_000, ServerEvent::MemberUnban { server: sid, user }, &identity_for(owner));
        let mut forward = state.clone();
        assert_eq!(forward.apply_signed(&ban), Ok(true));
        assert_eq!(forward.apply_signed(&unban), Ok(true));
        assert_eq!(forward.apply_signed(&ban), Ok(false), "a replayed ban");
        assert!(forward.server.banned.is_empty());
        let mut backward = state.clone();
        assert_eq!(backward.apply_signed(&unban), Ok(false));
        assert_eq!(backward.apply_signed(&ban), Ok(false), "the unban came first");
        assert!(backward.server.banned.is_empty());
        let restored = crate::store::ServerState::from_snapshot(forward.snapshot());
        let mut restored = restored;
        assert_eq!(restored.apply_signed(&ban), Ok(false), "remembered across a restart");
        let mine = SignedEvent::sign(sid, owner, 500, ServerEvent::ServerRename { server: sid, name: "mine".into() }, &identity_for(owner));
        assert_eq!(state.apply_own(&mine), Ok(true));
        let theirs = SignedEvent::sign(sid, owner, 400, ServerEvent::ServerRename { server: sid, name: "theirs".into() }, &identity_for(owner));
        assert_eq!(state.apply_signed(&theirs), Ok(false));
        assert_eq!(state.server.name, "mine");
    }

    #[test]
    fn bare_events_are_taken_only_from_members_without_a_key() {

        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let keyed = join(&mut state, "keyed");
        let presence = |user| ServerEvent::PresenceUpdate { user, presence: Presence::Idle };
        assert_eq!(state.apply_unsigned(presence(keyed)), Err(StoreError::Unsigned));
        let fresh = UserProfile::new("fresh");
        let fresh_id = fresh.id;
        assert_eq!(state.apply_unsigned(ServerEvent::MemberJoin { server: sid, profile: fresh }), Ok(true));
        assert_eq!(state.apply_unsigned(presence(fresh_id)), Ok(true));
        let mut carrying = state.profiles[&fresh_id].clone();
        carrying.identity = Some(identity_for(fresh_id).public());
        assert_eq!(state.apply_unsigned(ServerEvent::ProfileUpdate { profile: carrying.clone() }), Err(StoreError::Unsigned));
        let signed = SignedEvent::sign(sid, fresh_id, 1, ServerEvent::ProfileUpdate { profile: carrying }, &identity_for(fresh_id));
        assert_eq!(state.apply_signed(&signed), Ok(true), "a key proves itself");
        assert_eq!(state.identity_of(fresh_id), Some(identity_for(fresh_id).public()));
        assert_eq!(state.apply_unsigned(presence(fresh_id)), Err(StoreError::Unsigned));
        assert_eq!(state.apply_unsigned(ServerEvent::MemberBan { server: sid, user: keyed }), Err(StoreError::Unsigned));
        let message = crate::test_support::post(&state, channel, owner, "hi");
        assert_eq!(state.apply_unsigned(ServerEvent::MessageCreate { message }), Ok(true));
        let mut other_key = state.profiles[&keyed].clone();
        other_key.identity = Some(Identity::generate().public());
        let swap = SignedEvent::sign(sid, keyed, 1, ServerEvent::ProfileUpdate { profile: other_key }, &identity_for(keyed));
        assert_eq!(state.apply_signed(&swap), Err(StoreError::IdentityMismatch));
    }

    #[test]
    fn sound_sharing_is_the_managers_call_and_shared_plays_need_a_member() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let user = join(&mut state, "user");
        let share = ServerEvent::SoundSharingSet { server: sid, shareable: true };
        assert!(authorized(&state.server, owner, &share));
        assert!(!authorized(&state.server, user, &share));
        assert_eq!(state.apply(share), Ok(true));
        assert!(state.server.sounds_shareable);
        let play = ServerEvent::SharedSoundPlay { server: sid, channel, user, name: "boop".into(), blob: "b".into(), volume: 1.0 };
        assert!(authorized(&state.server, user, &play));
        assert!(!may_go_unsigned(&play));
        let stranger = ServerEvent::SharedSoundPlay { server: sid, channel, user: UserId::new(), name: "boop".into(), blob: "b".into(), volume: 1.0 };
        assert!(!authorized(&state.server, user, &stranger), "a play names its own player");
    }

    #[test]
    fn live_events_come_only_from_current_members() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let user = join(&mut state, "user");
        let voice = ServerEvent::VoiceState { user, channel: Some(channel), muted: false, deafened: false, since: None };
        let message = {
            let mut m = crate::Message::new(channel, owner, "hi");
            m.sign(sid, &identity_for(owner));
            m
        };
        state.apply(ServerEvent::MessageCreate { message: message.clone() }).unwrap();
        let react = ServerEvent::ReactionSet { channel, message: message.id, emoji: "x".into(), user, active: true };
        let play = ServerEvent::SoundboardPlay { server: sid, channel, user, name: "boop".into() };
        assert!(authorized(&state.server, user, &voice));
        assert!(authorized(&state.server, user, &react));
        assert!(!may_go_unsigned(&play), "sounds are signed");
        let stranger = UserId::new();
        let bare = ServerEvent::SoundboardPlay { server: sid, channel, user: stranger, name: "boop".into() };
        assert_eq!(state.apply_unsigned(bare), Err(StoreError::Unsigned));
        let ban = SignedEvent::sign(sid, owner, 1_000, ServerEvent::MemberBan { server: sid, user }, &identity_for(owner));
        assert_eq!(state.apply_signed(&ban), Ok(true));
        assert!(!authorized(&state.server, user, &voice), "banned members leave voice");
        assert!(!authorized(&state.server, user, &react));
        let signed = SignedEvent::sign(sid, user, 2_000, voice, &identity_for(user));
        assert_eq!(state.apply_signed(&signed), Err(StoreError::NotAllowed));
        let presence = ServerEvent::PresenceUpdate { user: stranger, presence: Presence::Online };
        assert_eq!(state.apply_unsigned(presence), Err(StoreError::Unsigned), "strangers have no presence here");
    }

    #[test]
    fn no_op_changes_about_unknown_things_leave_no_clock_behind() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let rsvp = SignedEvent::sign(sid, owner, 1_000, ServerEvent::EventRsvp { server: sid, event: "nope".into(), user: owner, rsvp: None }, &identity_for(owner));
        assert_eq!(state.apply_signed(&rsvp), Ok(false));
        assert!(state.snapshot().action_times.iter().all(|(s, _)| !s.starts_with("rsvp:")));
        let real = SignedEvent::sign(sid, owner, 1_000, ServerEvent::ChannelColor { server: sid, channel, color: None }, &identity_for(owner));
        assert_eq!(state.apply_signed(&real), Ok(false), "already that color");
        assert!(state.snapshot().action_times.iter().any(|(s, t)| s.ends_with(&channel.to_string()) && *t == 1_000), "but the clock is kept");
        let older = SignedEvent::sign(sid, owner, 500, ServerEvent::ChannelColor { server: sid, channel, color: Some("ff0000".into()) }, &identity_for(owner));
        assert_eq!(state.apply_signed(&older), Ok(false), "an older paint loses");
    }

    #[test]
    fn a_snapshot_floor_keeps_old_authority_changes_out() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let user = join(&mut state, "user");
        let now = chrono::Utc::now().timestamp_millis();
        state.set_action_floor(now);
        let stale = SignedEvent::sign(sid, owner, now - 2 * crate::store::MAX_ACTION_FLOOR_MS, ServerEvent::MemberBan { server: sid, user }, &identity_for(owner));
        assert_eq!(state.apply_signed(&stale), Ok(false));
        assert!(state.server.is_member(user));
        let recent = SignedEvent::sign(sid, owner, now - crate::store::MAX_ACTION_FLOOR_MS / 2, ServerEvent::MemberBan { server: sid, user }, &identity_for(owner));
        assert_eq!(state.apply_signed(&recent), Ok(true));
        let presence = SignedEvent::sign(sid, owner, now - 10 * crate::store::MAX_ACTION_FLOOR_MS, ServerEvent::PresenceUpdate { user: owner, presence: Presence::Idle }, &identity_for(owner));
        assert_eq!(state.apply_signed(&presence), Ok(true), "live state is not held to the floor");
        let restored = crate::store::ServerState::from_snapshot(state.snapshot());
        assert_eq!(restored.action_floor(), now);
    }

    fn emoji(sid: ServerId, name: &str) -> ServerEvent {
        ServerEvent::EmojiAdd { server: sid, emoji: crate::server::CustomEmoji { name: name.into(), blob: format!("{}@{}", "b".repeat(52), "0".repeat(64)) } }
    }

    #[test]
    fn the_latest_signed_change_is_kept_for_others_to_relay() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let add = SignedEvent::sign(sid, owner, 1_000, emoji(sid, "wave"), &identity_for(owner));
        assert_eq!(state.apply_signed(&add), Ok(true));
        assert_eq!(state.signed_actions(), vec![&add]);
        assert!(state.holds_signed_action("emoji:wave"));
        let restored = crate::store::ServerState::from_snapshot(state.snapshot());
        assert_eq!(restored.signed_actions(), vec![&add], "kept across restarts");

        let remove = SignedEvent::sign(sid, owner, 2_000, ServerEvent::EmojiRemove { server: sid, name: "wave".into() }, &identity_for(owner));
        assert_eq!(state.apply_signed(&remove), Ok(true));
        assert_eq!(state.signed_actions(), vec![&remove], "a later change replaces it");
        assert_eq!(state.apply_signed(&add), Ok(false), "the old one relayed again loses");
        assert!(state.server.emojis.is_empty());

        let presence = SignedEvent::sign(sid, owner, 3_000, ServerEvent::PresenceUpdate { user: owner, presence: Presence::Idle }, &identity_for(owner));
        assert_eq!(state.apply_signed(&presence), Ok(true));
        let mut profile = state.profiles[&owner].clone();
        profile.custom_status = Some("here".into());
        let join = SignedEvent::sign(sid, owner, 3_000, ServerEvent::MemberJoin { server: sid, profile }, &identity_for(owner));
        state.apply_signed(&join).unwrap();
        assert_eq!(state.signed_actions(), vec![&remove], "live state and joins are not relayed");
    }

    #[test]
    fn a_relayed_change_counts_as_its_author_made_it() {
        let (mut author, _, owner) = fixture();
        let sid = author.server.id;
        let relayer = join(&mut author, "relayer");
        let mut other = author.clone();
        let add = SignedEvent::sign(sid, owner, 1_000, emoji(sid, "wave"), &identity_for(owner));
        assert_eq!(author.apply_signed(&add), Ok(true));
        let relayed = author.signed_actions()[0].clone();
        assert_eq!(other.apply_signed(&relayed), Ok(true));
        assert_eq!(other.server.emojis.len(), 1);
        let own = SignedEvent::sign(sid, relayer, 2_000, emoji(sid, "wave"), &identity_for(relayer));
        assert_eq!(other.apply_signed(&own), Err(StoreError::NotAllowed), "relaying gives no right to change it");
    }

    #[test]
    fn what_snapshots_leave_out_is_not_held_to_the_floor() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let now = chrono::Utc::now().timestamp_millis();
        state.set_action_floor(now);
        let old = now - 10 * crate::store::MAX_ACTION_FLOOR_MS;
        let add = SignedEvent::sign(sid, owner, old, emoji(sid, "wave"), &identity_for(owner));
        assert_eq!(state.apply_signed(&add), Ok(true), "a joiner learns emojis made long ago");
        let rename = SignedEvent::sign(sid, owner, old, ServerEvent::ChannelUpdate { server: sid, channel, name: "old".into(), retention_days: None }, &identity_for(owner));
        assert_eq!(state.apply_signed(&rename), Ok(false), "what the snapshot carries stays as it said");
        assert!(snapshot_omits("rules") && snapshot_omits("pin:x") && !snapshot_omits("rules_accept:x"));
        assert!(snapshot_omits("automod"));
    }

    #[test]
    fn members_add_new_expressions_but_only_managers_replace_or_remove_them() {
        use crate::server::{Emoticon, Sticker};
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let member = join(&mut state, "member");
        let image = format!("{}@{}", "c".repeat(52), "0".repeat(64));
        let sticker = |name: &str| ServerEvent::StickerAdd { server: sid, sticker: Sticker { name: name.into(), blob: image.clone() } };
        let emoticon = |name: &str, text: &str| ServerEvent::EmoticonAdd { server: sid, emoticon: Emoticon { name: name.into(), text: text.into() } };
        let by = |user, at, event| SignedEvent::sign(sid, user, at, event, &identity_for(user));
        assert_eq!(state.apply_signed(&by(member, 1_000, emoji(sid, "wave"))), Ok(true));
        assert_eq!(state.apply_signed(&by(member, 1_000, sticker("hi"))), Ok(true));
        assert_eq!(state.apply_signed(&by(member, 1_000, emoticon("shrug", "¯\\_(ツ)_/¯"))), Ok(true));
        assert_eq!(state.apply_signed(&by(member, 2_000, emoticon("shrug", "o_O"))), Err(StoreError::NotAllowed));
        let gone = ServerEvent::StickerRemove { server: sid, name: "hi".into() };
        assert_eq!(state.apply_signed(&by(member, 2_000, gone.clone())), Err(StoreError::NotAllowed));
        assert_eq!(state.apply_signed(&by(owner, 3_000, emoticon("shrug", "o_O"))), Ok(true));
        assert_eq!(state.apply_signed(&by(owner, 3_000, gone)), Ok(true));
        assert_eq!(state.server.emoticons[0].text, "o_O");
        assert!(state.server.stickers.is_empty());

        let mut everyone = state.server.roles[0].clone();
        everyone.permissions.remove(Permissions::ADD_STICKERS);
        state.apply(ServerEvent::RoleUpdate { server: sid, role: everyone }).unwrap();
        assert_eq!(state.apply_signed(&by(member, 4_000, sticker("late"))), Err(StoreError::NotAllowed));
        let artists = role(Permissions::ADD_STICKERS, false);
        let rid = artists.id;
        state.apply(ServerEvent::RoleCreate { server: sid, role: artists }).unwrap();
        state.apply(ServerEvent::RoleAssign { server: sid, user: member, role: rid }).unwrap();
        assert_eq!(state.apply_signed(&by(member, 5_000, sticker("late"))), Ok(true));
        assert!(snapshot_omits("sticker:late") && snapshot_omits("emoticon:shrug"));
    }

    #[test]
    fn only_the_owner_deletes_a_server_and_members_relay_it() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let admin = join(&mut state, "admin");
        let admin_role = role(Permissions::ADMINISTRATOR, false);
        let rid = admin_role.id;
        state.apply(ServerEvent::RoleCreate { server: sid, role: admin_role }).unwrap();
        state.apply(ServerEvent::RoleAssign { server: sid, user: admin, role: rid }).unwrap();
        let delete = ServerEvent::ServerDelete { server: sid };
        assert!(authorized(&state.server, owner, &delete));
        assert!(!authorized(&state.server, admin, &delete), "not even an administrator");
        assert!(relayable(&delete));
        assert!(snapshot_omits(&subject(&delete).unwrap()));
        let by_admin = SignedEvent::sign(sid, admin, 1_000, delete.clone(), &identity_for(admin));
        assert_eq!(state.apply_signed(&by_admin), Err(StoreError::NotAllowed));
        assert!(!state.is_deleted());
        let real = SignedEvent::sign(sid, owner, 1_000, delete, &identity_for(owner));
        assert_eq!(state.apply_signed(&real), Ok(true));
        assert!(state.is_deleted());
        assert_eq!(state.signed_actions(), vec![&real]);
        let rename = SignedEvent::sign(sid, owner, 2_000, ServerEvent::ServerRename { server: sid, name: "after".into() }, &identity_for(owner));
        assert_eq!(state.apply_signed(&rename), Ok(false), "a deleted server takes no more changes");
        assert!(crate::store::ServerState::from_snapshot(state.snapshot()).is_deleted());
    }

    #[test]
    fn our_own_stale_change_is_refused_locally_too() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let newer = SignedEvent::sign(sid, owner, 2_000, ServerEvent::ServerRename { server: sid, name: "new".into() }, &identity_for(owner));
        assert_eq!(state.apply_signed(&newer), Ok(true));
        let mine = SignedEvent::sign(sid, owner, 1_000, ServerEvent::ServerRename { server: sid, name: "mine".into() }, &identity_for(owner));
        assert_eq!(state.apply_own(&mine), Ok(false));
        assert_eq!(state.server.name, "new");
    }
}
