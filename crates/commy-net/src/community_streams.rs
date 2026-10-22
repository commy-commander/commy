use std::collections::HashSet;

use commy_core::action::Action;
use commy_core::channel_key::{ChannelSecret, KeyId, KeyScope, Keyring};
use commy_core::{ChannelId, MessageId, ServerEvent, ServerId, ServerState, UserId};
use commy_sync::{StreamKeys, StreamSecret};
use serde::{Deserialize, Serialize};

use crate::protocol::{WirePayload, PROTOCOL_VERSION};

const COMMUNITY_STATE: &str = "commy-community-state/v1";
const COMMUNITY_CHANNEL: &str = "commy-community-channel/v1";
const LOCKED_CHANNEL: &str = "commy-locked-channel/v1";
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

pub const SERVICE_SENDER: [u8; 32] = [0; 32];
pub const OLDER_EPOCH_MS: i64 = 14 * DAY_MS;
pub const RETENTION_MS: i64 = 90 * DAY_MS;
pub const FRESH_TAIL: u32 = 200;
pub const BACKFILL_PER_CHANNEL: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Lane {
    State,
    Channel(ChannelId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Want {
    pub lane: Lane,
    pub scope: KeyScope,
    pub key: KeyId,
    pub keys: StreamKeys,
    pub writes: bool,
}

fn place(server: ServerId, channel: Option<ChannelId>) -> Vec<u8> {
    postcard::to_stdvec(&(server, channel)).expect("ids always encode")
}

pub fn stream_for(secret: &ChannelSecret, server: ServerId, scope: KeyScope, lane: Lane) -> StreamKeys {
    let (label, channel) = match (scope, lane) {
        (KeyScope::Community, Lane::State) => (COMMUNITY_STATE, None),
        (KeyScope::Community, Lane::Channel(channel)) => (COMMUNITY_CHANNEL, Some(channel)),
        (KeyScope::Channel(channel), _) => (LOCKED_CHANNEL, Some(channel)),
    };
    StreamSecret::derive(secret.bytes(), label, &place(server, channel)).keys()
}

pub fn epochs(state: &ServerState, scope: KeyScope, ring: &Keyring, now_ms: i64) -> Vec<(KeyId, ChannelSecret)> {
    let Some(chain) = state.key_chain(scope) else { return Vec::new() };
    let newest = chain.newest().map(|record| record.id);
    let mut held = ring.held_records(state, scope);
    held.sort_by_key(|(record, _)| (Some(record.id) != newest, std::cmp::Reverse((record.at, record.id))));
    let Some((first, secret)) = held.first() else { return Vec::new() };
    let mut out = vec![(first.id, (*secret).clone())];
    if now_ms - first.at < OLDER_EPOCH_MS {
        if let Some((older, secret)) = held.get(1) {
            out.push((older.id, (*secret).clone()));
        }
    }
    out
}

fn wants_of(state: &ServerState, scope: KeyScope, lanes: &[Lane], ring: &Keyring, now_ms: i64) -> Vec<Want> {
    let server = state.server.id;
    let mut out = Vec::new();
    for (at, (key, secret)) in epochs(state, scope, ring, now_ms).into_iter().enumerate() {
        for lane in lanes {
            out.push(Want { lane: *lane, scope, key, keys: stream_for(&secret, server, scope, *lane), writes: at == 0 });
        }
    }
    out
}

pub fn wanted(state: &ServerState, me: UserId, community: &Keyring, locked: impl Fn(ChannelId) -> Keyring, now_ms: i64) -> Vec<Want> {
    let server = &state.server;
    if !server.is_current_member(me) || state.is_deleted() {
        return Vec::new();
    }
    let texts = server.channels.iter().filter(|c| c.kind.holds_messages());
    let lanes: Vec<Lane> =
        std::iter::once(Lane::State).chain(texts.clone().filter(|c| !c.is_private()).map(|c| Lane::Channel(c.id))).collect();
    let mut out = wants_of(state, KeyScope::Community, &lanes, community, now_ms);
    for channel in texts.filter(|c| c.is_private() && server.can_view(c.id, me)) {
        let scope = KeyScope::Channel(channel.id);
        out.extend(wants_of(state, scope, &[Lane::Channel(channel.id)], &locked(channel.id), now_ms));
    }
    out
}

pub fn lane_of(event: &ServerEvent) -> Option<Lane> {
    use ServerEvent as E;
    match event {
        E::MessageCreate { message } => Some(Lane::Channel(message.channel)),
        E::MessageEdit { channel, .. }
        | E::MessageDelete { channel, .. }
        | E::ReactionSet { channel, .. }
        | E::MessagePin { channel, .. }
        | E::PollVote { channel, .. }
        | E::PollClose { channel, .. }
        | E::ThreadSet { channel, .. }
        | E::ForumPost { channel, .. } => Some(Lane::Channel(*channel)),
        E::TypingStart { .. }
        | E::PresenceUpdate { .. }
        | E::VoiceState { .. }
        | E::ScreenShare { .. }
        | E::CameraState { .. }
        | E::SoundboardPlay { .. }
        | E::SharedSoundPlay { .. }
        | E::VoiceModerate { .. }
        | E::VoiceMove { .. }
        | E::VoiceStatusSet { .. }
        | E::StageSet { .. }
        | E::HandRaise { .. }
        | E::ActivitySet { .. }
        | E::ActivityInput { .. }
        | E::DoorbellSet { .. }
        | E::DeviceAnnounce { .. }
        | E::ReactionToggle { .. } => None,
        _ => Some(Lane::State),
    }
}

pub fn is_key_event(event: &ServerEvent) -> bool {
    matches!(
        event,
        ServerEvent::CommunityKeyAdd { .. }
            | ServerEvent::CommunityKeyGrant { .. }
            | ServerEvent::ChannelKeyAdd { .. }
            | ServerEvent::ChannelKeyGrant { .. }
    )
}

#[derive(Serialize, Deserialize)]
struct Entry {
    version: u16,
    payload: WirePayload,
}

pub fn event_of(payload: &WirePayload) -> Option<(&ServerEvent, Option<Action>)> {
    match payload {
        WirePayload::Event(event) => Some((event, None)),
        WirePayload::Signed(signed) => Some((&signed.event, Some(signed.action))),
        _ => None,
    }
}

pub fn encode_entry(payload: &WirePayload) -> Option<Vec<u8>> {
    let (event, _) = event_of(payload)?;
    lane_of(event)?;
    postcard::to_stdvec(&Entry { version: PROTOCOL_VERSION, payload: payload.clone() }).ok()
}

pub fn decode_entry(plain: &[u8], lane: Lane) -> Option<(ServerEvent, Option<Action>)> {
    let entry: Entry = postcard::from_bytes(plain).ok()?;
    if entry.version != PROTOCOL_VERSION {
        return None;
    }
    let (event, action) = event_of(&entry.payload)?;
    (lane_of(event) == Some(lane)).then(|| (event.clone(), action))
}

pub fn backfill_messages(state: &ServerState, channel: ChannelId, seen: &HashSet<MessageId>, now_ms: i64) -> Vec<WirePayload> {
    let oldest = now_ms - RETENTION_MS;
    let recent: Vec<&commy_core::Message> = state
        .messages(channel)
        .iter()
        .rev()
        .filter(|m| m.sig.is_some() && m.created_at.timestamp_millis() > oldest)
        .take(BACKFILL_PER_CHANNEL)
        .collect();
    recent
        .into_iter()
        .rev()
        .filter(|m| !seen.contains(&m.id))
        .map(|m| WirePayload::Event(ServerEvent::MessageCreate { message: m.clone() }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::channel_key::community_key_task;
    use commy_core::test_support::{fixture, identity_for, join, post, post_at};
    use commy_core::{Permissions, Role, RoleId};

    fn settle(state: &mut ServerState, me: UserId) -> Keyring {
        loop {
            let mut ring = Keyring::unlock_community(state, me, &identity_for(me));
            let Some(task) = community_key_task(state, me, &ring) else { return ring };
            for event in ring.task_events(state, KeyScope::Community, task, chrono::Utc::now().timestamp_millis()) {
                state.apply(event).unwrap();
            }
        }
    }

    fn now() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    #[test]
    fn members_of_one_server_meet_on_the_same_streams_and_others_do_not() {
        let (mut state, channel, owner) = fixture();
        let member = join(&mut state, "member");
        settle(&mut state, owner);
        let ring = |user| Keyring::unlock_community(&state, user, &identity_for(user));
        let none = |_| Keyring::default();
        let ours = wanted(&state, owner, &ring(owner), none, now());
        let theirs = wanted(&state, member, &ring(member), none, now());
        assert!(!ours.is_empty());
        assert_eq!(ours.iter().map(|w| w.keys.id).collect::<Vec<_>>(), theirs.iter().map(|w| w.keys.id).collect::<Vec<_>>());
        assert!(ours.iter().any(|w| w.lane == Lane::State));
        assert!(ours.iter().any(|w| w.lane == Lane::Channel(channel)));
        assert!(ours.iter().all(|w| w.writes));
        let voice = state.server.channels[1].id;
        assert!(!ours.iter().any(|w| w.lane == Lane::Channel(voice)), "voice channels hold no text");
        let stranger = UserId::new();
        assert!(wanted(&state, stranger, &ring(stranger), none, now()).is_empty());

        let (mut other, _, other_owner) = fixture();
        settle(&mut other, other_owner);
        let elsewhere = wanted(&other, other_owner, &Keyring::unlock_community(&other, other_owner, &identity_for(other_owner)), none, now());
        assert!(elsewhere.iter().all(|w| !ours.iter().any(|o| o.keys.id == w.keys.id)));
    }

    #[test]
    fn a_rotation_moves_writes_to_the_new_key_and_keeps_reading_the_last_one() {
        let (mut state, _, owner) = fixture();
        let server = state.server.id;
        let banned = join(&mut state, "banned");
        settle(&mut state, owner);
        let before: Vec<Want> = wanted(&state, owner, &Keyring::unlock_community(&state, owner, &identity_for(owner)), |_| Keyring::default(), now());
        state.apply(ServerEvent::MemberBan { server, user: banned }).unwrap();
        let ring = settle(&mut state, owner);
        let after = wanted(&state, owner, &ring, |_| Keyring::default(), now());
        let writes: Vec<_> = after.iter().filter(|w| w.writes).map(|w| w.keys.id).collect();
        let reads: Vec<_> = after.iter().filter(|w| !w.writes).map(|w| w.keys.id).collect();
        assert_eq!(reads, before.iter().map(|w| w.keys.id).collect::<Vec<_>>());
        assert!(writes.iter().all(|id| !reads.contains(id)));
        let later = wanted(&state, owner, &ring, |_| Keyring::default(), now() + OLDER_EPOCH_MS + 1);
        assert!(later.iter().all(|w| w.writes), "the older key is dropped after two weeks");
        assert!(wanted(&state, banned, &Keyring::unlock_community(&state, banned, &identity_for(banned)), |_| Keyring::default(), now()).is_empty());
    }

    #[test]
    fn locked_channels_get_streams_only_for_their_readers() {
        let (mut state, channel, owner) = fixture();
        let server = state.server.id;
        let member = join(&mut state, "member");
        settle(&mut state, owner);
        let role = Role {
            id: RoleId::new(),
            name: "crew".into(),
            color: None,
            permissions: Permissions::NONE,
            position: 1,
            hoist: false,
            self_assignable: false,
            mentionable: false,
            tag: None,
            banner: None,
        };
        let crew = role.id;
        state.apply(ServerEvent::RoleCreate { server, role }).unwrap();
        state.apply(ServerEvent::ChannelAccessSet { server, channel, view: vec![crew], send: vec![] }).unwrap();
        let mut locked = Keyring::default();
        let record = locked.rotate(server, channel, &[], now());
        state.apply(ServerEvent::ChannelKeyAdd { server, channel, key: record }).unwrap();
        let community = Keyring::unlock_community(&state, owner, &identity_for(owner));
        let ours = wanted(&state, owner, &community, |_| locked.clone(), now());
        let lane = ours.iter().find(|w| w.lane == Lane::Channel(channel)).unwrap();
        assert_eq!(lane.scope, KeyScope::Channel(channel));
        let theirs = wanted(&state, member, &Keyring::unlock_community(&state, member, &identity_for(member)), |_| locked.clone(), now());
        assert!(!theirs.iter().any(|w| w.lane == Lane::Channel(channel)), "members outside the role get no stream for it");
    }

    #[test]
    fn stream_ids_depend_on_the_key_the_server_and_the_lane() {
        let secret = ChannelSecret::random();
        let (server, channel) = (ServerId::new(), ChannelId::new());
        let state = stream_for(&secret, server, KeyScope::Community, Lane::State).id;
        let text = stream_for(&secret, server, KeyScope::Community, Lane::Channel(channel)).id;
        let locked = stream_for(&secret, server, KeyScope::Channel(channel), Lane::Channel(channel)).id;
        assert!(state != text && text != locked && state != locked);
        assert_ne!(stream_for(&secret, ServerId::new(), KeyScope::Community, Lane::State).id, state);
        assert_ne!(stream_for(&ChannelSecret::random(), server, KeyScope::Community, Lane::State).id, state);
        assert_eq!(stream_for(&ChannelSecret::from_bytes(*secret.bytes()), server, KeyScope::Community, Lane::State).id, state);
    }

    #[test]
    fn only_lasting_events_are_stored_and_they_come_back_on_their_lane() {
        let (state, channel, owner) = fixture();
        let server = state.server.id;
        let message = post(&state, channel, owner, "hello");
        let payload = WirePayload::Event(ServerEvent::MessageCreate { message: message.clone() });
        let plain = encode_entry(&payload).unwrap();
        assert_eq!(decode_entry(&plain, Lane::Channel(channel)), Some((ServerEvent::MessageCreate { message }, None)));
        assert_eq!(decode_entry(&plain, Lane::State), None);
        assert_eq!(decode_entry(&plain, Lane::Channel(ChannelId::new())), None);
        assert_eq!(decode_entry(b"junk", Lane::State), None);

        let rename = ServerEvent::ServerRename { server, name: "new".into() };
        let signed = commy_core::action::SignedEvent::sign(server, owner, 5, rename.clone(), &identity_for(owner));
        let plain = encode_entry(&WirePayload::Signed(signed.clone())).unwrap();
        assert_eq!(decode_entry(&plain, Lane::State), Some((rename, Some(signed.action))));

        let typing = WirePayload::Event(ServerEvent::TypingStart { channel, user: owner });
        assert_eq!(encode_entry(&typing), None);
        assert_eq!(encode_entry(&WirePayload::SyncRequest), None);
        let presence = ServerEvent::PresenceUpdate { user: owner, presence: commy_core::Presence::Online };
        assert_eq!(lane_of(&presence), None);
        assert!(is_key_event(&ServerEvent::CommunityKeyGrant {
            server,
            user: owner,
            grant: commy_core::channel_key::SealedKey::seal(server, KeyScope::Community, owner, &identity_for(owner).public(), KeyId::random(), &ChannelSecret::random()).unwrap(),
        }));
    }

    #[test]
    fn backfill_takes_recent_signed_messages_the_stream_lacks() {
        let (mut state, channel, owner) = fixture();
        let old = post_at(&state, channel, owner, "old", chrono::Utc::now() - chrono::Duration::days(120));
        state.apply(ServerEvent::MessageCreate { message: old }).unwrap();
        let mut ids = Vec::new();
        for n in 0..(BACKFILL_PER_CHANNEL + 5) {
            let message = post(&state, channel, owner, &format!("m{n}"));
            ids.push(message.id);
            state.apply(ServerEvent::MessageCreate { message }).unwrap();
        }
        let seen: HashSet<MessageId> = ids[ids.len() - 3..].iter().copied().collect();
        let picked: Vec<MessageId> = backfill_messages(&state, channel, &seen, now())
            .into_iter()
            .map(|p| match p {
                WirePayload::Event(ServerEvent::MessageCreate { message }) => message.id,
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(picked.len(), BACKFILL_PER_CHANNEL - 3);
        assert_eq!(picked, ids[5..ids.len() - 3].to_vec(), "oldest first, without the ones already there");
    }
}
