use commy_core::identity::{Identity, IdentityKey, Signature};
use commy_core::{ServerEvent, ServerId, ServerState, SignedEvent, UserId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Bump on breaking wire changes; peers drop mismatched envelopes.
pub const PROTOCOL_VERSION: u16 = 4;

const SNAPSHOT_DOMAIN: &str = "commy-snapshot/v1";

/// Hard cap for a single gossip frame. Large payloads (files, images,
/// soundboard clips) must go through iroh-blobs instead.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

#[derive(Debug, Error, PartialEq)]
pub enum ProtocolError {
    #[error("peer speaks protocol v{0}, we speak v{PROTOCOL_VERSION}")]
    VersionMismatch(u16),
    #[error("frame exceeds {MAX_FRAME_BYTES} bytes")]
    FrameTooLarge,
    #[error("malformed frame")]
    Malformed,
}

/// Payloads that travel over a server's gossip topic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WirePayload {
    /// A state-mutating event, applied by every member.
    Event(ServerEvent),
    /// New member asking any peer for the current server state.
    SyncRequest,
    /// Snapshot of server metadata for a newcomer, serialized
    /// separately because ServerState holds derived indexes. The members
    /// travel in `members`, not in `server`, see [`build_sync_response`].
    SyncResponse {
        server: commy_core::Server,
        members: Vec<SyncMember>,
        profiles: Vec<commy_core::UserProfile>,
        recent_messages: Vec<commy_core::Message>,
        /// Who is in voice right now, so a newcomer sees the call too.
        /// Older snapshots leave it out.
        #[serde(default)]
        voice: Vec<SyncVoice>,
        by: UserId,
        at: i64,
        sig: Signature,
    },
    /// Voice signaling: ask a peer to open a direct media connection.
    VoiceDial {
        channel: commy_core::ChannelId,
        from_user: commy_core::UserId,
    },
    Signed(SignedEvent),
}

/// A member as a snapshot carries them: half the size of a
/// [`commy_core::Member`] on the wire, as the join time is in ms since
/// the epoch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncMember {
    pub user: UserId,
    pub nickname: Option<String>,
    pub roles: Vec<commy_core::RoleId>,
    pub joined_ms: i64,
}

/// A member in a voice channel, as a snapshot carries them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncVoice {
    pub user: UserId,
    pub channel: commy_core::ChannelId,
    pub muted: bool,
    pub deafened: bool,
    /// When they joined, unix seconds.
    pub since: Option<i64>,
}

impl SyncVoice {
    /// The event that puts this member back in voice.
    pub fn into_event(self) -> ServerEvent {
        ServerEvent::VoiceState {
            user: self.user,
            channel: Some(self.channel),
            muted: self.muted,
            deafened: self.deafened,
            since: self.since,
        }
    }
}

impl From<&commy_core::Member> for SyncMember {
    fn from(m: &commy_core::Member) -> Self {
        let joined_ms = m.joined_at.timestamp_millis();
        Self { user: m.user, nickname: m.nickname.clone(), roles: m.roles.clone(), joined_ms }
    }
}

#[derive(Serialize)]
struct SnapshotBytes<'a> {
    domain: &'static str,
    server_id: ServerId,
    by: UserId,
    at: i64,
    server: &'a commy_core::Server,
    members: &'a [SyncMember],
    profiles: &'a [commy_core::UserProfile],
    recent_messages: &'a [commy_core::Message],
    voice: &'a [SyncVoice],
}

fn snapshot_bytes(payload: &WirePayload) -> Option<Vec<u8>> {
    let WirePayload::SyncResponse { server, members, profiles, recent_messages, voice, by, at, .. } = payload else {
        return None;
    };
    let bytes = SnapshotBytes {
        domain: SNAPSHOT_DOMAIN,
        server_id: server.id,
        by: *by,
        at: *at,
        server,
        members,
        profiles,
        recent_messages,
        voice,
    };
    postcard::to_stdvec(&bytes).ok()
}

pub fn sign_sync_response(payload: &mut WirePayload, at: i64, identity: &Identity) {
    if let WirePayload::SyncResponse { at: slot, .. } = payload {
        *slot = at;
    }
    let Some(bytes) = snapshot_bytes(payload) else { return };
    if let WirePayload::SyncResponse { sig, .. } = payload {
        *sig = identity.sign(&bytes);
    }
}

pub fn verify_sync_response(payload: &WirePayload, key: &IdentityKey) -> bool {
    let (Some(bytes), WirePayload::SyncResponse { sig, .. }) = (snapshot_bytes(payload), payload) else {
        return false;
    };
    key.verify(&bytes, sig)
}

pub fn signed_size(server: ServerId, event: &ServerEvent) -> usize {
    let by = UserId::new();
    let signed = SignedEvent::sign(server, by, i64::MAX, event.clone(), &Identity::generate());
    gossip_size(&WirePayload::Signed(signed))
}

/// Versioned, sender-attributed wrapper for every gossip frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub version: u16,
    /// Raw public key bytes of the sending iroh node.
    pub sender: [u8; 32],
    pub payload: WirePayload,
    /// Makes every frame unique. iroh-gossip drops a message whose bytes
    /// match one it saw recently, so re-sending an identical event (joining
    /// the same voice channel again, restarting a share) would vanish.
    pub nonce: u64,
}

fn next_nonce() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    nanos ^ COUNTER.fetch_add(1, Ordering::Relaxed).rotate_left(48)
}

impl Envelope {
    pub fn new(sender: [u8; 32], payload: WirePayload) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            sender,
            payload,
            nonce: next_nonce(),
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        let bytes = postcard::to_stdvec(self).map_err(|_| ProtocolError::Malformed)?;
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(ProtocolError::FrameTooLarge);
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(ProtocolError::FrameTooLarge);
        }
        let envelope: Envelope =
            postcard::from_bytes(bytes).map_err(|_| ProtocolError::Malformed)?;
        if envelope.version != PROTOCOL_VERSION {
            return Err(ProtocolError::VersionMismatch(envelope.version));
        }
        Ok(envelope)
    }
}

/// Largest envelope we put on gossip. iroh-gossip 0.28 closes the whole
/// connection when a frame exceeds its 4096 byte limit (framing included),
/// so every broadcast must stay comfortably under it.
pub const MAX_GOSSIP_BYTES: usize = 3800;

/// Encoded size of a payload as it would go on the wire.
pub fn gossip_size(payload: &WirePayload) -> usize {
    Envelope::new([0u8; 32], payload.clone())
        .encode()
        .map(|b| b.len())
        .unwrap_or(usize::MAX)
}

/// Encoded size of one item of a snapshot.
fn encoded<T: Serialize>(item: &T) -> usize {
    postcard::to_stdvec(item).map_or(0, |b| b.len())
}

/// Snapshot for a newcomer, at most `per_channel` of the newest messages
/// of each channel, trimmed to fit one gossip frame. Emojis, sounds,
/// rules and calendar events are always left out (they alone can exceed
/// a frame); members re-announce them one event at a time. What does not
/// fit goes in this order, so what a newcomer needs to take part (the
/// owner, channels, roles and member list) stays longest:
/// 1. profiles, but the owner's and those of the carried messages'
///    authors; role holders' go last;
/// 2. structure members announce anyway: images, game tags, rule
///    acceptances, voice moderation;
/// 3. messages, the oldest first, each author's profile with their last
///    message;
/// 4. members, the latest to join first and role holders last, then the
///    owner's profile.
///
/// Bans and deleted channels always stay: they are small, and keep the
/// banned out and deleted channels from coming back.
///
/// Members left out are learned from their own join, which each member
/// announces to new neighbors and with its answer to a sync request; what
/// they wrote verifies meanwhile, as catch-up relays their keys.
pub fn build_sync_response(state: &ServerState, per_channel: usize, by: UserId) -> WirePayload {
    let mut server = state.server.clone();
    server.emojis.clear();
    server.soundboard.clear();
    server.rules = None;
    server.events.clear();
    server.deleted_events.clear();
    // Members who may pin re-announce pins, like other structure.
    server.pins.clear();
    let mut messages: Vec<commy_core::Message> = Vec::new();
    for channel in &state.server.channels {
        messages.extend(state.page(channel.id, None, per_channel).iter().cloned());
    }
    // Newest first, so trimming from the end drops the oldest.
    messages.sort_by_key(|m| std::cmp::Reverse(m.created_at));
    let owner = server.owner;
    let role_holders = server.members.iter().filter(|m| !m.roles.is_empty()).map(|m| m.user);
    let members = server.members.iter().map(|m| m.user);
    let mut former: Vec<UserId> = state.profiles.keys().copied().collect();
    former.sort();
    let mut seen = std::collections::HashSet::new();
    let order = std::iter::once(owner).chain(messages.iter().map(|m| m.author)).chain(role_holders);
    let order: Vec<UserId> = order.chain(members).chain(former).filter(|u| seen.insert(*u)).collect();
    let mut profiles: Vec<commy_core::UserProfile> =
        order.iter().filter_map(|u| state.profiles.get(u)).cloned().collect();
    let mut members: Vec<SyncMember> = std::mem::take(&mut server.members).iter().map(SyncMember::from).collect();
    let mut voice: Vec<SyncVoice> = state
        .voice_states
        .iter()
        .map(|(user, channel)| {
            let (muted, deafened) = state.voice_flags(*user);
            SyncVoice { user: *user, channel: *channel, muted, deafened, since: state.voice_since(*user) }
        })
        .collect();
    voice.sort_by_key(|v| v.user);
    let build = |server: &commy_core::Server,
                 members: &[SyncMember],
                 profiles: &[commy_core::UserProfile],
                 messages: &[commy_core::Message]| WirePayload::SyncResponse {
        server: server.clone(),
        members: members.to_vec(),
        profiles: profiles.to_vec(),
        // Oldest first, as they are applied.
        recent_messages: messages.iter().rev().cloned().collect(),
        voice: voice.clone(),
        by,
        at: i64::MAX,
        sig: Signature([0xff; 64]),
    };
    // An upper bound kept as items go: a list's length prefix only shrinks.
    let mut size = gossip_size(&build(&server, &members, &profiles, &messages));
    let fits = |size: usize| size <= MAX_GOSSIP_BYTES;
    let needed = |user: UserId, messages: &[commy_core::Message]| {
        user == owner || messages.iter().any(|m| m.author == user)
    };

    // 1. Profiles no carried message needs, the least wanted first.
    while !fits(size) {
        let Some(last) = profiles.last() else { break };
        if needed(last.id, &messages) {
            break;
        }
        size -= encoded(last);
        profiles.pop();
    }
    // 2. Structure announced anyway.
    type Step = fn(&mut commy_core::Server);
    let steps: [Step; 4] = [
        |s| (s.icon, s.banner) = (None, None),
        |s| (s.game_tags, s.game_tag_meta) = (Vec::new(), Vec::new()),
        |s| s.rules_accepted.clear(),
        |s| (s.server_muted, s.server_deafened) = (Vec::new(), Vec::new()),
    ];
    for step in steps {
        if fits(size) {
            break;
        }
        let before = encoded(&server);
        step(&mut server);
        size -= before - encoded(&server);
    }
    // 3. Messages, the oldest first.
    while !fits(size) {
        let Some(message) = messages.pop() else { break };
        size -= encoded(&message);
        if !needed(message.author, &messages) {
            if let Some(pos) = profiles.iter().position(|p| p.id == message.author) {
                size -= encoded(&profiles.remove(pos));
            }
        }
    }
    // 4. Members, the latest to join first and role holders last.
    if !fits(size) {
        members.sort_by_key(|m| (m.user != owner, m.roles.is_empty(), m.joined_ms));
        while !fits(size) && members.len() > 1 {
            size -= members.pop().map_or(0, |m| encoded(&m));
        }
    }
    if !fits(size) {
        profiles.clear();
    }
    build(&server, &members, &profiles, &messages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{fixture, join, post};
    use commy_core::{Message, Server, ServerState, UserId};

    #[test]
    fn identical_events_encode_differently() {
        let payload = WirePayload::Event(ServerEvent::TypingStart { channel: commy_core::ChannelId::new(), user: UserId::new() });
        let a = Envelope::new([1u8; 32], payload.clone()).encode().unwrap();
        let b = Envelope::new([1u8; 32], payload).encode().unwrap();
        assert_ne!(a, b, "gossip would drop the second as a duplicate");
    }

    #[test]
    fn envelope_roundtrip() {
        let user = UserId::new();
        let server = Server::new("s", user);
        let channel = server.channels[0].id;
        let envelope = Envelope::new(
            [3u8; 32],
            WirePayload::Event(ServerEvent::MessageCreate {
                message: Message::new(channel, user, "over the wire"),
            }),
        );
        let bytes = envelope.encode().unwrap();
        assert_eq!(Envelope::decode(&bytes).unwrap(), envelope);
    }

    #[test]
    fn version_mismatch_is_rejected() {
        let mut envelope = Envelope::new([0u8; 32], WirePayload::SyncRequest);
        envelope.version = PROTOCOL_VERSION + 1;
        let bytes = postcard::to_stdvec(&envelope).unwrap();
        assert_eq!(
            Envelope::decode(&bytes),
            Err(ProtocolError::VersionMismatch(PROTOCOL_VERSION + 1))
        );
    }

    #[test]
    fn garbage_is_malformed() {
        assert_eq!(
            Envelope::decode(&[0xff, 0xfe, 0xfd]),
            Err(ProtocolError::Malformed)
        );
    }

    #[test]
    fn oversized_frame_is_rejected() {
        let user = UserId::new();
        let server = Server::new("s", user);
        let channel = server.channels[0].id;
        let big = "x".repeat(MAX_FRAME_BYTES);
        let envelope = Envelope::new(
            [0u8; 32],
            WirePayload::Event(ServerEvent::MessageCreate {
                message: Message::new(channel, user, big),
            }),
        );
        assert_eq!(envelope.encode(), Err(ProtocolError::FrameTooLarge));
    }

    /// Size of `payload` in the smallest envelope: a one byte nonce.
    fn smallest(payload: WirePayload) -> usize {
        let envelope = Envelope { version: PROTOCOL_VERSION, sender: [0; 32], payload, nonce: 0 };
        postcard::to_stdvec(&envelope).unwrap().len()
    }

    #[test]
    fn the_store_takes_every_message_and_profile_gossip_carries() {
        let (mut state, channel, owner) = fixture();
        let base = post(&state, channel, owner, &"x".repeat(200));
        let sized = |size: usize| {
            let fill = 200 + size - base.authored_size();
            let mut m = Message { content: "x".repeat(fill), ..base.clone() };
            m.sign(state.server.id, &crate::test_support::identity_for(owner));
            assert_eq!(m.authored_size(), size);
            m
        };
        let create = |message: Message| WirePayload::Event(ServerEvent::MessageCreate { message });
        let largest = sized(commy_core::MAX_MESSAGE_BYTES);
        let over = sized(commy_core::MAX_MESSAGE_BYTES + 1);
        assert_eq!(smallest(create(largest.clone())), MAX_GOSSIP_BYTES);
        assert_eq!(smallest(create(over.clone())), MAX_GOSSIP_BYTES + 1);
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: over }), Err(commy_core::StoreError::TooLarge));
        assert_eq!(state.apply(ServerEvent::MessageCreate { message: largest }), Ok(true));

        let profile = state.profiles[&owner].clone();
        let around = smallest(WirePayload::Event(ServerEvent::ProfileUpdate { profile: profile.clone() }))
            - postcard::to_stdvec(&profile).unwrap().len();
        assert_eq!(MAX_GOSSIP_BYTES - around, crate::catchup::MAX_RELAYED_PROFILE_BYTES);
    }

    #[test]
    fn sync_response_caps_messages() {
        let (mut state, channel, owner) = fixture();
        for i in 0..100 {
            let message = post(&state, channel, owner, &format!("m{i}"));
            state.apply(ServerEvent::MessageCreate { message }).unwrap();
        }
        let WirePayload::SyncResponse {
            recent_messages, ..
        } = build_sync_response(&state, 50, owner)
        else {
            panic!("wrong payload kind");
        };
        // At most 50 per channel, fewer if needed to fit one gossip frame.
        assert!(!recent_messages.is_empty() && recent_messages.len() <= 50);
        assert_eq!(recent_messages.last().unwrap().content, "m99");
    }

    #[test]
    fn sync_response_fits_one_gossip_frame() {
        let (mut state, channel, owner) = fixture();
        for i in 0..200 {
            let msg = post(
                &state,
                channel,
                owner,
                &format!("message number {i} with some padding text"),
            );
            state
                .apply(ServerEvent::MessageCreate { message: msg })
                .unwrap();
        }
        let payload = build_sync_response(&state, 50, owner);
        assert!(gossip_size(&payload) <= MAX_GOSSIP_BYTES);
        let WirePayload::SyncResponse { recent_messages, .. } = payload else { panic!("wrong payload") };
        assert!(!recent_messages.is_empty(), "keeps as many recent messages as fit");
        assert!(recent_messages.last().unwrap().content.contains("199"), "newest message survives");
    }

    #[test]
    fn sync_response_messages_verify_on_receive() {
        let (mut state, channel, owner) = fixture();
        let friend = join(&mut state, "friend");
        for (i, author) in [owner, friend, owner].into_iter().enumerate() {
            let message = post(&state, channel, author, &format!("hello {i}"));
            state.apply(ServerEvent::MessageCreate { message }).unwrap();
        }
        let WirePayload::SyncResponse {
            server,
            profiles,
            recent_messages,
            ..
        } = build_sync_response(&state, 50, owner)
        else {
            panic!("wrong payload kind");
        };
        assert_eq!(recent_messages.len(), 3);
        assert!(recent_messages.iter().all(|m| m.sig.is_some()));

        // A newcomer inserts profiles first, then applies the messages.
        let mut fresh = ServerState::new(server.clone());
        for profile in profiles.clone() {
            fresh.profiles.insert(profile.id, profile);
        }
        for message in recent_messages.clone() {
            assert_eq!(
                fresh.apply(ServerEvent::MessageCreate { message }),
                Ok(true)
            );
        }
        assert_eq!(fresh.messages(channel).len(), 3);

        // Without the author's key a message is not taken.
        let mut keyless = ServerState::new(server);
        for mut profile in profiles {
            profile.identity = None;
            keyless.profiles.insert(profile.id, profile);
        }
        let first = recent_messages[0].clone();
        let author = first.author;
        assert_eq!(
            keyless.apply(ServerEvent::MessageCreate { message: first }),
            Err(commy_core::StoreError::UnknownAuthorKey(author))
        );
    }

    /// A server as communities run it: `members` members with full
    /// profiles, roles held by some, categories of channels, images, game
    /// tags, a few bans and some talk.
    fn realistic(members: usize) -> ServerState {
        use commy_core::{Channel, ChannelKind, Permissions, Role, RoleId, UserProfile};
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let filled = |mut p: UserProfile, i: usize| {
            p.display_name = Some(format!("Player {i:03}"));
            p.avatar = (i % 10 < 7).then(|| format!("blob:{}", "a1".repeat(32)));
            p.node = Some("f0".repeat(32));
            p.custom_status = i.is_multiple_of(5).then(|| "streaming later".to_string());
            p.activity = i.is_multiple_of(10).then(|| "Some Game".to_string());
            p.about = (i % 10 == 1).then(|| "x".repeat(60));
            p
        };
        let me = state.profiles[&owner].clone();
        state.profiles.insert(owner, filled(me, 0));
        let image = format!("{}@{}", "c3".repeat(32), "d4".repeat(32));
        state.server.icon = Some(image.clone());
        state.server.banner = Some(image);
        state.server.game_tags = vec!["Strategy".into(), "Shooter".into(), "Racing".into()];
        let mut text = vec![channel];
        for (i, name) in ["info", "community"].into_iter().enumerate() {
            let category = Channel::new(name, ChannelKind::Category, 10 + i as u32 * 10);
            let parent = category.id;
            state.apply(ServerEvent::ChannelCreate { server: sid, channel: category }).unwrap();
            for j in 0..3 {
                let mut c = Channel::new(format!("{name}-{j}"), ChannelKind::Text, 11 + (i * 10 + j) as u32);
                c.parent = Some(parent);
                c.topic = Some("what this channel is for".into());
                text.push(c.id);
                state.apply(ServerEvent::ChannelCreate { server: sid, channel: c }).unwrap();
            }
        }
        let roles: Vec<RoleId> = ["Admin", "Moderator", "Regular", "Events", "Streamer"]
            .into_iter()
            .enumerate()
            .map(|(i, name)| {
                let role = Role {
                    id: RoleId::new(),
                    name: name.into(),
                    color: Some("5865f2".into()),
                    permissions: if i < 2 { Permissions::MANAGE_MESSAGES } else { Permissions::NONE },
                    position: 5 - i as u32,
                    hoist: i < 2,
                    self_assignable: i >= 3,
                    tag: None,
                    banner: None,
                };
                let id = role.id;
                state.apply(ServerEvent::RoleCreate { server: sid, role }).unwrap();
                id
            })
            .collect();
        let mut users = vec![owner];
        for i in 1..members {
            let profile = filled(crate::test_support::keyed_profile(&format!("player_{i:03}")), i);
            let user = profile.id;
            state.apply(ServerEvent::MemberJoin { server: sid, profile }).unwrap();
            // Joined one after another, a millisecond apart.
            let joined = chrono::DateTime::from_timestamp_millis(1_000 + i as i64).unwrap();
            state.server.members.last_mut().unwrap().joined_at = joined;
            if i.is_multiple_of(7) {
                let role = roles[i % roles.len()];
                state.apply(ServerEvent::RoleAssign { server: sid, user, role }).unwrap();
            }
            users.push(user);
        }
        for _ in 0..3 {
            state.apply(ServerEvent::MemberBan { server: sid, user: UserId::new() }).unwrap();
        }
        for i in 0..40 {
            let author = users[(i * 3) % users.len()];
            let message = post(&state, text[i % 3], author, &format!("message {i} about the match tonight"));
            state.apply(ServerEvent::MessageCreate { message }).unwrap();
        }
        state
    }

    /// The parts of a snapshot.
    fn parts(payload: WirePayload) -> (Server, Vec<SyncMember>, Vec<commy_core::UserProfile>, Vec<Message>) {
        let WirePayload::SyncResponse { server, members, profiles, recent_messages, .. } = payload else {
            panic!("not a snapshot")
        };
        (server, members, profiles, recent_messages)
    }

    #[test]
    fn large_servers_get_a_snapshot_that_fits() {
        for members in [15, 50, 94, 100, 200] {
            let state = realistic(members);
            let payload = build_sync_response(&state, 50, state.server.owner);
            let size = gossip_size(&payload);
            let (server, kept, profiles, recent) = parts(payload);
            eprintln!("{members} members: {size} bytes, {} members, {} profiles, {} messages", kept.len(), profiles.len(), recent.len());
            assert!(size <= MAX_GOSSIP_BYTES, "{members}");
            // What a newcomer needs to take part.
            assert_eq!(server.owner, state.server.owner);
            assert_eq!((&server.channels, &server.roles), (&state.server.channels, &state.server.roles));
            assert!(server.members.is_empty(), "they travel on their own");
            assert!(kept.iter().any(|m| m.user == state.server.owner));
            assert!(profiles.iter().any(|p| p.id == state.server.owner), "{members}: the owner's key");
            // Every carried message comes with its author's key.
            assert!(recent.iter().all(|m| profiles.iter().any(|p| p.id == m.author)), "{members}");
            if kept.len() == members {
                continue;
            }
            // The role holders stay, then the earliest to join.
            let kept_ids: std::collections::HashSet<UserId> = kept.iter().map(|m| m.user).collect();
            assert!(state.server.members.iter().filter(|m| !m.roles.is_empty()).all(|m| kept_ids.contains(&m.user)));
            let joined = |user: UserId| state.server.member(user).map_or(0, |m| m.joined_at.timestamp_millis());
            let dropped = state.server.members.iter().filter(|m| !kept_ids.contains(&m.user));
            let first_dropped = dropped.map(|m| joined(m.user)).min().unwrap();
            let others = kept.iter().filter(|m| m.roles.is_empty() && m.user != state.server.owner);
            assert!(others.map(|m| joined(m.user)).all(|t| t < first_dropped));
            assert!(recent.is_empty() && profiles.len() == 1, "members go last");
        }
        // With 50 members there is room for keys and talk as well; up to
        // about 90 everyone is taken in, and of 200 more than 80 are.
        let snapshot = |n: usize| {
            let state = realistic(n);
            build_sync_response(&state, 50, state.server.owner)
        };
        let (_, kept, profiles, recent) = parts(snapshot(50));
        assert!(kept.len() == 50 && profiles.len() > 1 && !recent.is_empty());
        assert_eq!(parts(snapshot(90)).1.len(), 90);
        assert!(parts(snapshot(200)).1.len() > 80);
    }

    #[test]
    fn a_snapshot_trims_profiles_then_announced_structure_then_messages() {
        let state = realistic(15);
        let all = gossip_size(&WirePayload::SyncResponse {
            server: state.server.clone(),
            members: vec![],
            profiles: state.profiles.values().cloned().collect(),
            recent_messages: vec![],
            voice: vec![],
            by: state.server.owner,
            at: 0,
            sig: Signature([0; 64]),
        });
        assert!(all > MAX_GOSSIP_BYTES, "its profiles alone are too many");
        let (server, members, profiles, recent) = parts(build_sync_response(&state, 50, state.server.owner));
        assert_eq!(members.len(), 15);
        // Profiles of those who wrote nothing carried went first, before
        // any structure or message.
        let authors: std::collections::HashSet<UserId> = recent.iter().map(|m| m.author).collect();
        assert!(profiles.iter().all(|p| p.id == state.server.owner || authors.contains(&p.id)));
        assert!(server.icon.is_none() && server.banner.is_none() && server.game_tags.is_empty());
        assert_eq!(server.banned, state.server.banned, "bans always stay");
        assert!(!recent.is_empty() && recent.len() < 40, "messages last, the newest kept");
        let newest = state.server.channels.iter().flat_map(|c| state.messages(c.id)).max_by_key(|m| m.created_at).unwrap();
        assert_eq!(recent.last().map(|m| m.id), Some(newest.id));

        // A small server keeps it all but what is always left out.
        let (mut small, channel, owner) = fixture();
        let friend = join(&mut small, "friend");
        small.server.icon = Some("icon".into());
        small.apply(ServerEvent::MemberBan { server: small.server.id, user: UserId::new() }).unwrap();
        let message = post(&small, channel, friend, "hi");
        small.apply(ServerEvent::MessageCreate { message }).unwrap();
        let (server, members, profiles, recent) = parts(build_sync_response(&small, 50, owner));
        assert_eq!((members.len(), profiles.len(), recent.len()), (2, 2, 1));
        assert_eq!((server.icon.as_deref(), server.banned.len()), (Some("icon"), 1));
        assert_eq!(profiles[0].id, owner);
    }

    #[test]
    fn a_snapshot_never_trims_bans_or_deleted_channels() {
        let mut state = realistic(200);
        let sid = state.server.id;
        for _ in 0..4 {
            let channel = commy_core::Channel::new("gone", commy_core::ChannelKind::Text, 50);
            let id = channel.id;
            state.apply(ServerEvent::ChannelCreate { server: sid, channel }).unwrap();
            state.apply(ServerEvent::ChannelDelete { server: sid, channel: id }).unwrap();
        }
        let payload = build_sync_response(&state, 50, state.server.owner);
        assert!(gossip_size(&payload) <= MAX_GOSSIP_BYTES);
        let (server, members, _, recent) = parts(payload);
        // Messages and members went first.
        assert!(recent.is_empty() && members.len() < state.server.members.len());
        assert_eq!(server.banned, state.server.banned);
        assert_eq!(server.deleted_channels, state.server.deleted_channels);
        assert_eq!(server.deleted_channels.len(), 4);
    }

    #[test]
    fn snapshot_members_keep_what_a_member_holds() {
        let (mut state, _, _) = fixture();
        let user = join(&mut state, "member");
        let member = commy_core::Member {
            nickname: Some("nick".into()),
            roles: vec![state.server.roles[0].id],
            ..state.server.member(user).unwrap().clone()
        };
        let carried = SyncMember::from(&member);
        assert_eq!((carried.user, &carried.nickname, &carried.roles), (member.user, &member.nickname, &member.roles));
        assert_eq!(carried.joined_ms, member.joined_at.timestamp_millis());
        assert!(encoded(&carried) < encoded(&member) - 20);
    }

    #[test]
    fn protocol_version_is_bumped_for_signed_actions() {
        assert_eq!(PROTOCOL_VERSION, 4);
    }

    #[test]
    fn a_snapshot_is_signed_by_whoever_serves_it() {
        let (mut state, channel, owner) = fixture();
        let friend = join(&mut state, "friend");
        let message = post(&state, channel, friend, "hi");
        state.apply(ServerEvent::MessageCreate { message }).unwrap();
        let identity = crate::test_support::identity_for(owner);
        let mut payload = build_sync_response(&state, 50, owner);
        assert!(!verify_sync_response(&payload, &identity.public()), "unsigned yet");
        sign_sync_response(&mut payload, 1_234, &identity);
        assert!(verify_sync_response(&payload, &identity.public()));
        assert!(gossip_size(&payload) <= MAX_GOSSIP_BYTES);
        let bytes = Envelope::new([1; 32], payload.clone()).encode().unwrap();
        let back = Envelope::decode(&bytes).unwrap().payload;
        assert!(verify_sync_response(&back, &identity.public()), "verifies as decoded");
        assert!(!verify_sync_response(&back, &crate::test_support::identity_for(friend).public()));
        let WirePayload::SyncResponse { at, by, .. } = &back else { panic!("not a snapshot") };
        assert_eq!((*at, *by), (1_234, owner));
        let mut tampered = back.clone();
        if let WirePayload::SyncResponse { server, .. } = &mut tampered {
            server.owner = friend;
        }
        assert!(!verify_sync_response(&tampered, &identity.public()));
        let mut tampered = back.clone();
        if let WirePayload::SyncResponse { members, .. } = &mut tampered {
            members[0].roles.push(commy_core::RoleId::new());
        }
        assert!(!verify_sync_response(&tampered, &identity.public()));
        let mut tampered = back;
        if let WirePayload::SyncResponse { profiles, .. } = &mut tampered {
            profiles[0].node = Some("00".repeat(32));
        }
        assert!(!verify_sync_response(&tampered, &identity.public()));
    }

    #[test]
    fn signed_events_fit_the_frame_with_room_to_spare() {
        let (state, _, owner) = fixture();
        let sid = state.server.id;
        let rules = ServerEvent::RulesSet { server: sid, rules: Some("r".repeat(commy_core::server::MAX_RULES_BYTES)) };
        assert!(signed_size(sid, &rules) <= MAX_GOSSIP_BYTES);
        let identity = crate::test_support::identity_for(owner);
        let signed = SignedEvent::sign(sid, owner, 1, rules, &identity);
        let bytes = Envelope::new([1; 32], WirePayload::Signed(signed.clone())).encode().unwrap();
        let WirePayload::Signed(back) = Envelope::decode(&bytes).unwrap().payload else { panic!("not signed") };
        assert_eq!(back, signed);
        assert!(back.verify(sid, &identity.public()));
    }

    #[test]
    fn sync_response_leaves_out_expressions() {
        use commy_core::server::{CustomEmoji, SoundboardSound};
        let owner = UserId::new();
        let mut server = Server::new("s", owner);
        for i in 0..50 {
            server.emojis.push(CustomEmoji { name: format!("emoji{i}"), blob: format!("{}@{}", "a".repeat(64), "b".repeat(64)) });
            server.soundboard.push(SoundboardSound { name: format!("s{i}"), blob: "a@b".into(), volume: 1.0 });
        }
        let state = ServerState::new(server);
        let payload = build_sync_response(&state, 50, owner);
        assert!(gossip_size(&payload) <= MAX_GOSSIP_BYTES);
        let WirePayload::SyncResponse { server, .. } = payload else { panic!("wrong payload kind") };
        assert!(server.emojis.is_empty() && server.soundboard.is_empty());
    }
}
