use std::collections::HashSet;

use commy_core::action::Action;
use commy_core::channel_key::{ChannelSecret, KeyId, KeyScope, Keyring};
use commy_core::day_keys::{DayKeyring, DaySealed, DaySealer};
use commy_core::{ChannelId, MessageId, ServerEvent, ServerId, ServerState, UserId};
use commy_sync::wire::ERASE_EVERYTHING;
use commy_sync::{StreamId, StreamKeys, StreamSecret};
use serde::{Deserialize, Serialize};

use crate::protocol::{WirePayload, PROTOCOL_VERSION};

const COMMUNITY_STATE: &str = "commy-community-state/v1";
const COMMUNITY_CHANNEL: &str = "commy-community-channel/v1";
const LOCKED_CHANNEL: &str = "commy-locked-channel/v1";
const DAY_CHANNEL: &str = "commy-community-day-channel/v1";
pub const MAX_DAY_EPOCHS_READ: usize = 5;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

pub const SERVICE_SENDER: [u8; 32] = [0; 32];
pub const OLDER_EPOCH_MS: i64 = 14 * DAY_MS;
pub const RETENTION_MS: i64 = 90 * DAY_MS;
pub const FRESH_TAIL: u32 = 200;
pub const STATE_TAIL: u32 = 1_000;
pub const SNAPSHOT_BYTES: usize = 48 * 1024;
pub const BACKFILL_PER_CHANNEL: usize = 100;
pub const RINGS_FOR_SECS: i64 = 10 * 60;

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
    pub days: bool,
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
            out.push(Want { lane: *lane, scope, key, keys: stream_for(&secret, server, scope, *lane), writes: at == 0, days: false });
        }
    }
    out
}

pub fn day_stream_for(addressing: &ChannelSecret, server: ServerId, channel: ChannelId) -> StreamKeys {
    StreamSecret::derive(addressing.bytes(), DAY_CHANNEL, &place(server, Some(channel))).keys()
}

pub fn day_epochs(state: &ServerState, days: &DayKeyring, now_ms: i64) -> Vec<(KeyId, ChannelSecret)> {
    if !commy_core::day_chain::is_on(&state.server) {
        return Vec::new();
    }
    let chain = state.day_chain();
    let window_opens = now_ms - i64::from(commy_core::day_chain::history_days(&state.server)) * DAY_MS;
    days.held(state)
        .into_iter()
        .enumerate()
        .filter(|(at, (epoch, _))| *at == 0 || chain.epochs.iter().filter(|e| (e.at, e.id) > (epoch.at, epoch.id)).map(|e| e.at).min().is_none_or(|ended| ended > window_opens))
        .take(MAX_DAY_EPOCHS_READ)
        .map(|(_, (epoch, share))| (epoch.id, share.addressing().clone()))
        .collect()
}

fn writes_newest(state: &ServerState, days: &DayKeyring, key: KeyId) -> bool {
    state.day_chain().newest().is_some_and(|newest| newest.id == key && days.holds(key))
}

pub fn wanted(state: &ServerState, me: UserId, community: &Keyring, locked: impl Fn(ChannelId) -> Keyring, now_ms: i64) -> Vec<Want> {
    wanted_with_days(state, me, community, &DayKeyring::default(), locked, now_ms)
}

pub fn wanted_with_days(state: &ServerState, me: UserId, community: &Keyring, days: &DayKeyring, locked: impl Fn(ChannelId) -> Keyring, now_ms: i64) -> Vec<Want> {
    let server = &state.server;
    if !server.is_current_member(me) || state.is_deleted() {
        return Vec::new();
    }
    let texts = server.channels.iter().filter(|c| c.kind.holds_messages());
    let open: Vec<ChannelId> = texts.clone().filter(|c| !c.is_private()).map(|c| c.id).collect();
    let lanes: Vec<Lane> = std::iter::once(Lane::State).chain(open.iter().map(|c| Lane::Channel(*c))).collect();
    let mut out = wants_of(state, KeyScope::Community, &lanes, community, now_ms);
    let epochs = day_epochs(state, days, now_ms);
    let day_writer = epochs.iter().any(|(key, _)| writes_newest(state, days, *key));
    for want in out.iter_mut().filter(|w| day_writer && matches!(w.lane, Lane::Channel(_))) {
        want.writes = false;
    }
    for (key, addressing) in &epochs {
        let writes = writes_newest(state, days, *key);
        for channel in &open {
            let keys = day_stream_for(addressing, server.id, *channel);
            out.push(Want { lane: Lane::Channel(*channel), scope: KeyScope::Community, key: *key, keys, writes, days: true });
        }
    }
    for channel in texts.filter(|c| c.is_private() && server.can_view(c.id, me)) {
        let scope = KeyScope::Channel(channel.id);
        out.extend(wants_of(state, scope, &[Lane::Channel(channel.id)], &locked(channel.id), now_ms));
    }
    out
}

pub fn every_stream(state: &ServerState, community: &Keyring, locked: impl Fn(ChannelId) -> Keyring) -> Vec<(Lane, StreamKeys)> {
    every_stream_with_days(state, community, &DayKeyring::default(), locked)
}

pub fn every_stream_with_days(state: &ServerState, community: &Keyring, days: &DayKeyring, locked: impl Fn(ChannelId) -> Keyring) -> Vec<(Lane, StreamKeys)> {
    let server = state.server.id;
    let texts: Vec<&commy_core::Channel> = state.server.channels.iter().filter(|c| c.kind.holds_messages()).collect();
    let lanes: Vec<Lane> = std::iter::once(Lane::State).chain(texts.iter().map(|c| Lane::Channel(c.id))).collect();
    let mut out: Vec<(Lane, StreamKeys)> = Vec::new();
    let mut add = |lane: Lane, keys: StreamKeys| {
        if !out.iter().any(|(_, known)| known.id == keys.id) {
            out.push((lane, keys));
        }
    };
    for (_, secret) in community.held_records(state, KeyScope::Community) {
        for lane in &lanes {
            let keys = stream_for(secret, server, KeyScope::Community, *lane);
            let checkpoint = (*lane == Lane::State).then(|| crate::checkpoint::keys_for(&keys));
            add(*lane, keys);
            if let Some(checkpoint) = checkpoint {
                add(Lane::State, checkpoint);
            }
        }
    }
    for (_, share) in days.held(state) {
        for channel in texts.iter().filter(|c| !c.is_private()) {
            add(Lane::Channel(channel.id), day_stream_for(share.addressing(), server, channel.id));
        }
    }
    for channel in texts.iter().filter(|c| c.is_private()) {
        let scope = KeyScope::Channel(channel.id);
        let ring = locked(channel.id);
        for (_, secret) in ring.held_records(state, scope) {
            add(Lane::Channel(channel.id), stream_for(secret, server, scope, Lane::Channel(channel.id)));
        }
    }
    out
}

pub fn erase_through(stream: StreamId, notices: &[(StreamId, u64)]) -> u64 {
    match notices.iter().find(|(held, _)| *held == stream) {
        Some((_, seq)) => seq.saturating_sub(1),
        None => ERASE_EVERYTHING,
    }
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
        | E::ActivityQueue { .. }
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

pub fn owner_key_records(state: &ServerState) -> Vec<WirePayload> {
    let owner = state.server.owner;
    state.signed_actions().into_iter().filter(|signed| republished_key_record(&signed.event, owner)).map(|signed| WirePayload::Signed(signed.clone())).collect()
}

fn republished_key_record(event: &ServerEvent, owner: UserId) -> bool {
    match event {
        ServerEvent::CommunityKeyAdd { .. } => true,
        ServerEvent::CommunityKeyGrant { user, .. } => *user == owner,
        _ => false,
    }
}

pub fn changes_lanes(event: &ServerEvent) -> bool {
    is_key_event(event)
        || matches!(
            event,
            ServerEvent::ChannelCreate { .. }
                | ServerEvent::ChannelDelete { .. }
                | ServerEvent::ChannelKindSet { .. }
                | ServerEvent::ChannelAccessSet { .. }
        )
}

#[derive(Serialize, Deserialize)]
struct Entry {
    version: u16,
    payload: WirePayload,
}

#[derive(Serialize, Deserialize)]
struct SignalEntry {
    version: u16,
    node: [u8; 32],
    payload: WirePayload,
}

pub fn signals(event: &ServerEvent) -> bool {
    use ServerEvent as E;
    matches!(
        event,
        E::PresenceUpdate { .. }
            | E::TypingStart { .. }
            | E::VoiceState { .. }
            | E::ScreenShare { .. }
            | E::CameraState { .. }
            | E::VoiceStatusSet { .. }
            | E::VoiceModerate { .. }
            | E::VoiceMove { .. }
            | E::StageSet { .. }
            | E::HandRaise { .. }
            | E::ActivitySet { .. }
            | E::ActivityInput { .. }
            | E::ActivityQueue { .. }
            | E::DeviceAnnounce { .. }
    )
}

pub fn signal_lane(event: &ServerEvent) -> Lane {
    match event {
        ServerEvent::TypingStart { channel, .. } => Lane::Channel(*channel),
        _ => Lane::State,
    }
}

pub fn encode_signal(node: [u8; 32], payload: &WirePayload) -> Option<Vec<u8>> {
    let (event, _) = event_of(payload)?;
    if !signals(event) {
        return None;
    }
    commy_core::wire_layout::encode(&SignalEntry { version: PROTOCOL_VERSION, node, payload: payload.clone() }).ok()
}

pub fn decode_signal(plain: &[u8], lane: Lane) -> Option<([u8; 32], WirePayload)> {
    let entry: SignalEntry = commy_core::wire_layout::decode(plain).ok()?;
    let (event, _) = event_of(&entry.payload)?;
    let fits = entry.version == PROTOCOL_VERSION && signals(event) && signal_lane(event) == lane && entry.node != SERVICE_SENDER;
    fits.then_some((entry.node, entry.payload))
}

pub fn event_of(payload: &WirePayload) -> Option<(&ServerEvent, Option<Action>)> {
    match payload {
        WirePayload::Event(event) => Some((event, None)),
        WirePayload::Signed(signed) => Some((&signed.event, Some(signed.action))),
        _ => None,
    }
}

pub fn lane_of_payload(payload: &WirePayload) -> Option<Lane> {
    match payload {
        WirePayload::SyncResponse { .. } => Some(Lane::State),
        _ => lane_of(event_of(payload)?.0),
    }
}

pub fn encode_entry(payload: &WirePayload) -> Option<Vec<u8>> {
    lane_of_payload(payload)?;
    commy_core::wire_layout::encode(&Entry { version: PROTOCOL_VERSION, payload: payload.clone() }).ok()
}

pub fn decode_entry(plain: &[u8], lane: Lane) -> Option<WirePayload> {
    let entry: Entry = commy_core::wire_layout::decode(plain).ok()?;
    if entry.version != PROTOCOL_VERSION {
        return None;
    }
    (lane_of_payload(&entry.payload) == Some(lane)).then_some(entry.payload)
}

#[derive(Serialize, Deserialize)]
struct DayEntry {
    version: u16,
    sealed: DaySealed,
}

pub fn seal_day_entry(sealer: &DaySealer, server: ServerId, lane: Lane, plain: &[u8], now_ms: i64) -> Option<Vec<u8>> {
    let Lane::Channel(channel) = lane else { return None };
    let sealed = sealer.seal(DAY_CHANNEL, &place(server, Some(channel)), plain, now_ms)?;
    commy_core::wire_layout::encode(&DayEntry { version: PROTOCOL_VERSION, sealed }).ok()
}

pub fn open_day_entry(days: &DayKeyring, server: ServerId, lane: Lane, body: &[u8]) -> Option<Vec<u8>> {
    let Lane::Channel(channel) = lane else { return None };
    let entry: DayEntry = commy_core::wire_layout::decode(body).ok()?;
    if entry.version != PROTOCOL_VERSION {
        return None;
    }
    days.open(server, &entry.sealed, DAY_CHANNEL, &place(server, Some(channel)))
}

pub fn rings(live_now: bool, created_at: chrono::DateTime<chrono::Utc>, now: chrono::DateTime<chrono::Utc>) -> bool {
    live_now && (now - created_at).num_seconds() < RINGS_FOR_SECS
}

pub fn wake_point(want: &Want) -> Option<Option<ChannelId>> {
    if !want.writes {
        return None;
    }
    match (want.scope, want.lane) {
        (KeyScope::Community, Lane::State) => Some(None),
        (KeyScope::Channel(channel), _) => Some(Some(channel)),
        _ => None,
    }
}

pub fn bootstrap(server: ServerId, key: KeyId, secret: &ChannelSecret) -> Want {
    let scope = KeyScope::Community;
    Want { lane: Lane::State, scope, key, keys: stream_for(secret, server, scope, Lane::State), writes: false, days: false }
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
    use commy_core::channel_key::{community_key_task, KeyTask};
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
    fn a_member_who_moves_finds_the_owners_copy_despite_a_stray_key_they_made() {
        let (mut state, _, owner) = fixture();
        let server = state.server.id;
        let member = join(&mut state, "ben");
        settle(&mut state, owner);
        let first = state.community_keys().newest().unwrap().id;
        let mut ring = Keyring::unlock_community(&state, owner, &identity_for(owner));
        for event in ring.task_events(&state, KeyScope::Community, KeyTask::Rotate { links: vec![first] }, now()) {
            state.apply(event).unwrap();
        }
        let owner_ring = settle(&mut state, owner);
        let mut empty_copy = Keyring::default();
        let stray = empty_copy.rotate(server, KeyScope::Community, &[], now() + 60_000);
        state.apply(ServerEvent::CommunityKeyAdd { server, key: stray.clone() }).unwrap();
        let mut member_ring = Keyring::unlock_community(&state, member, &identity_for(member));
        member_ring.admit(&state, KeyScope::Community, stray.id, empty_copy.secret(stray.id).unwrap().clone());

        let writes = |ring: &Keyring| epochs(&state, KeyScope::Community, ring, now())[0].0;
        let rotated = writes(&owner_ring);
        assert_ne!(rotated, stray.id);
        assert_eq!(writes(&member_ring), rotated, "the member reads and writes where the owner does");

        let (saved, secret) = member_ring.newest_in(&state, KeyScope::Community).expect("the key the member saves to their account");
        assert_eq!(saved, rotated);
        let after_move = bootstrap(server, saved, secret);
        let owners = wanted(&state, owner, &owner_ring, |_| Keyring::default(), now());
        let owners_state = owners.iter().find(|w| w.lane == Lane::State && w.writes).unwrap();
        assert_eq!(after_move.keys.id, owners_state.keys.id, "a fresh folder after a move opens the stream the owner's copy is on");
    }

    #[test]
    fn live_signals_carry_only_passing_state_on_its_own_lane() {
        let (state, channel, owner) = fixture();
        let server = state.server.id;
        let node = [4; 32];
        let typing = WirePayload::Event(ServerEvent::TypingStart { channel, user: owner });
        let plain = encode_signal(node, &typing).unwrap();
        assert_eq!(decode_signal(&plain, Lane::Channel(channel)), Some((node, typing.clone())));
        assert_eq!(decode_signal(&plain, Lane::State), None, "typing travels on its channel only");
        let online = WirePayload::Event(ServerEvent::PresenceUpdate { user: owner, presence: commy_core::Presence::Online });
        assert!(decode_signal(&encode_signal(node, &online).unwrap(), Lane::State).is_some());
        let rename = WirePayload::Event(ServerEvent::ServerRename { server, name: "x".into() });
        assert_eq!(encode_signal(node, &rename), None, "lasting changes are stored, never signalled");
        let sound = WirePayload::Event(ServerEvent::SoundboardPlay { server, channel, user: owner, name: "x".into() });
        assert!(!signals(streams_event(&sound)), "a sound would play twice when it also arrives from a peer");
        let input = WirePayload::Event(ServerEvent::ActivityInput { server, channel, seq: 3, input: commy_core::ActivityInput::Join, by: owner });
        assert_eq!(decode_signal(&encode_signal(node, &input).unwrap(), Lane::State), Some((node, input.clone())), "numbered activity inputs reach the browser app, which hears no peers");
        let change = commy_core::QueueChange::Skip { run: "run-1".into() };
        let queue = WirePayload::Event(ServerEvent::ActivityQueue { server, channel, seq: 4, change, by: owner });
        assert_eq!(decode_signal(&encode_signal(node, &queue).unwrap(), Lane::State), Some((node, queue.clone())));
        assert_eq!(encode_signal(SERVICE_SENDER, &online).and_then(|p| decode_signal(&p, Lane::State)), None);
        assert_eq!(decode_signal(b"junk", Lane::State), None);
    }

    fn streams_event(payload: &WirePayload) -> &ServerEvent {
        event_of(payload).unwrap().0
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
    fn a_new_channel_changes_the_lanes_to_follow() {
        let (mut state, channel, owner) = fixture();
        let server = state.server.id;
        settle(&mut state, owner);
        let lanes = |state: &ServerState| {
            let ring = Keyring::unlock_community(state, owner, &identity_for(owner));
            wanted(state, owner, &ring, |_| Keyring::default(), now()).into_iter().map(|w| w.lane).collect::<Vec<_>>()
        };
        let forum = commy_core::Channel::new("ideas", commy_core::ChannelKind::Forum, 9);
        let made = ServerEvent::ChannelCreate { server, channel: forum.clone() };
        assert!(changes_lanes(&made));
        assert!(!lanes(&state).contains(&Lane::Channel(forum.id)));
        state.apply(made).unwrap();
        assert!(lanes(&state).contains(&Lane::Channel(forum.id)));
        assert!(changes_lanes(&ServerEvent::ChannelDelete { server, channel: forum.id }));
        let message = post(&state, channel, owner, "hi");
        assert!(!changes_lanes(&ServerEvent::MessageCreate { message }));
        assert!(!changes_lanes(&ServerEvent::ServerRename { server, name: "x".into() }));
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
    fn a_deleted_server_still_names_every_stream_its_owner_can_erase() {
        let (mut state, channel, owner) = fixture();
        let server = state.server.id;
        let banned = join(&mut state, "banned");
        settle(&mut state, owner);
        let first = wanted(&state, owner, &Keyring::unlock_community(&state, owner, &identity_for(owner)), |_| Keyring::default(), now());
        state.apply(ServerEvent::MemberBan { server, user: banned }).unwrap();
        let ring = settle(&mut state, owner);
        let later = now() + OLDER_EPOCH_MS + 1;
        let current = wanted(&state, owner, &ring, |_| Keyring::default(), later);
        state.apply(ServerEvent::ServerDelete { server }).unwrap();
        assert!(wanted(&state, owner, &ring, |_| Keyring::default(), later).is_empty(), "nobody follows a deleted server");

        let every = every_stream(&state, &ring, |_| Keyring::default());
        let ids: Vec<StreamId> = every.iter().map(|(_, keys)| keys.id).collect();
        for want in first.iter().chain(&current) {
            assert!(ids.contains(&want.keys.id), "{:?} of every key ever used is covered", want.lane);
        }
        assert!(every.iter().any(|(lane, _)| *lane == Lane::Channel(channel)));
        for want in current.iter().filter(|w| w.lane == Lane::State) {
            assert!(ids.contains(&crate::checkpoint::keys_for(&want.keys).id), "the checkpoint copy is erased with the server too");
        }
        let voice = state.server.channels[1].id;
        assert!(!every.iter().any(|(lane, _)| *lane == Lane::Channel(voice)));
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn the_deletion_notice_survives_its_own_erase() {
        let (a, b) = (StreamSecret::random().keys().id, StreamSecret::random().keys().id);
        let notices = [(a, 42)];
        assert_eq!(erase_through(a, &notices), 41, "everything before the notice goes");
        assert_eq!(erase_through(b, &notices), ERASE_EVERYTHING);
        assert_eq!(erase_through(a, &[(a, 0)]), 0);
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
    fn a_key_alone_finds_the_server_state_stream() {
        let (mut state, _, owner) = fixture();
        let ring = settle(&mut state, owner);
        let (key, secret) = ring.newest_in(&state, KeyScope::Community).unwrap();
        let start = bootstrap(state.server.id, key, secret);
        let wants = wanted(&state, owner, &ring, |_| Keyring::default(), now());
        let lasting = wants.iter().find(|w| w.lane == Lane::State && w.writes).unwrap();
        assert_eq!(start.keys.id, lasting.keys.id);
        assert!(!start.writes);
    }

    #[test]
    fn only_lasting_events_are_stored_and_they_come_back_on_their_lane() {
        let (state, channel, owner) = fixture();
        let server = state.server.id;
        let message = post(&state, channel, owner, "hello");
        let payload = WirePayload::Event(ServerEvent::MessageCreate { message: message.clone() });
        let plain = encode_entry(&payload).unwrap();
        assert_eq!(decode_entry(&plain, Lane::Channel(channel)), Some(payload.clone()));
        assert_eq!(decode_entry(&plain, Lane::State), None);
        assert_eq!(decode_entry(&plain, Lane::Channel(ChannelId::new())), None);
        assert_eq!(decode_entry(b"junk", Lane::State), None);

        let rename = ServerEvent::ServerRename { server, name: "new".into() };
        let signed = commy_core::action::SignedEvent::sign(server, owner, 5, rename.clone(), &identity_for(owner));
        let plain = encode_entry(&WirePayload::Signed(signed.clone())).unwrap();
        assert_eq!(decode_entry(&plain, Lane::State), Some(WirePayload::Signed(signed.clone())));

        let snapshot = crate::protocol::build_sync_response(&state, 10, owner);
        let plain = encode_entry(&snapshot).unwrap();
        assert_eq!(decode_entry(&plain, Lane::State), Some(snapshot));
        assert_eq!(decode_entry(&plain, Lane::Channel(channel)), None);

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

    #[test]
    fn only_fresh_live_messages_ring() {
        let now = chrono::Utc::now();
        assert!(rings(true, now - chrono::Duration::seconds(5), now));
        assert!(!rings(false, now - chrono::Duration::seconds(5), now));
        assert!(!rings(true, now - chrono::Duration::hours(1), now));
    }

    fn day_settle(state: &mut ServerState, me: UserId) -> DayKeyring {
        for _ in 0..8 {
            let mut held = DayKeyring::unlock(state, me, &identity_for(me));
            let Some(task) = commy_core::day_keys::day_key_task(state, me, &held, now()) else { return held };
            for event in held.task_events(state, task, now()) {
                state.apply(event).unwrap();
            }
        }
        panic!("the day key task never settled");
    }

    fn switch_on(state: &mut ServerState) {
        let server = state.server.id;
        state.apply(ServerEvent::DayKeysSet { server, on: true, since: now(), history_days: 30 }).unwrap();
    }

    #[test]
    fn with_day_keys_messages_move_to_day_streams_and_state_stays_where_it_was() {
        let (mut state, channel, owner) = fixture();
        let member = join(&mut state, "member");
        let community = settle(&mut state, owner);
        let none = |_| Keyring::default();
        let before = wanted(&state, owner, &community, none, now());
        assert_eq!(wanted_with_days(&state, owner, &community, &DayKeyring::default(), none, now()), before);
        switch_on(&mut state);
        let days = day_settle(&mut state, owner);
        let ours = wanted_with_days(&state, owner, &community, &days, none, now());
        let theirs_ring = DayKeyring::unlock(&state, member, &identity_for(member));
        let member_community = Keyring::unlock_community(&state, member, &identity_for(member));
        let theirs = wanted_with_days(&state, member, &member_community, &theirs_ring, none, now());
        assert_eq!(ours.iter().map(|w| w.keys.id).collect::<Vec<_>>(), theirs.iter().map(|w| w.keys.id).collect::<Vec<_>>());

        let writers: Vec<&Want> = ours.iter().filter(|w| w.writes).collect();
        assert!(writers.iter().any(|w| w.lane == Lane::State && !w.days), "state still goes on the community stream");
        let channel_writers: Vec<&Want> = writers.iter().copied().filter(|w| w.lane == Lane::Channel(channel)).collect();
        assert_eq!(channel_writers.len(), 1);
        assert!(channel_writers[0].days, "messages are written only to the day stream");
        let old = before.iter().find(|w| w.lane == Lane::Channel(channel)).unwrap();
        assert!(ours.iter().any(|w| w.keys.id == old.keys.id && !w.writes), "earlier messages stay readable");
        assert!(every_stream_with_days(&state, &community, &days, none).iter().any(|(_, keys)| *keys == channel_writers[0].keys));
    }

    #[test]
    fn until_a_day_key_arrives_messages_keep_going_where_they_went() {
        let (mut state, channel, owner) = fixture();
        let member = join(&mut state, "member");
        let community = settle(&mut state, owner);
        switch_on(&mut state);
        let none = |_| Keyring::default();
        let waiting = wanted_with_days(&state, member, &Keyring::unlock_community(&state, member, &identity_for(member)), &DayKeyring::default(), none, now());
        assert!(waiting.iter().any(|w| w.lane == Lane::Channel(channel) && w.writes && !w.days));
        day_settle(&mut state, owner);
        let ours = wanted_with_days(&state, owner, &community, &DayKeyring::unlock(&state, owner, &identity_for(owner)), none, now());
        assert!(ours.iter().any(|w| w.lane == Lane::Channel(channel) && w.days));
    }

    #[test]
    fn day_entries_open_only_with_the_day_keys_and_on_their_own_lane() {
        let (mut state, channel, owner) = fixture();
        let member = join(&mut state, "member");
        settle(&mut state, owner);
        switch_on(&mut state);
        let days = day_settle(&mut state, owner);
        let server = state.server.id;
        let message = post(&state, channel, owner, "the plan for friday");
        let plain = encode_entry(&WirePayload::Event(ServerEvent::MessageCreate { message })).unwrap();
        let sealer = days.sealer(&state).unwrap();
        let sealed = seal_day_entry(&sealer, server, Lane::Channel(channel), &plain, now()).unwrap();
        assert!(!sealed.windows(6).any(|w| w == b"friday"));
        let theirs = DayKeyring::unlock(&state, member, &identity_for(member));
        assert_eq!(open_day_entry(&theirs, server, Lane::Channel(channel), &sealed), Some(plain.clone()));
        let other = state.server.channels.iter().find(|c| c.id != channel).unwrap().id;
        assert_eq!(open_day_entry(&theirs, server, Lane::Channel(other), &sealed), None);
        assert_eq!(open_day_entry(&DayKeyring::default(), server, Lane::Channel(channel), &sealed), None);
        assert_eq!(seal_day_entry(&sealer, server, Lane::State, &plain, now()), None, "state entries are not day sealed");
    }
}
