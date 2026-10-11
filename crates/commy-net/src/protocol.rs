use commy_core::{ServerEvent, ServerState};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Bump on breaking wire changes; peers drop mismatched envelopes.
pub const PROTOCOL_VERSION: u16 = 2;

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
    /// separately because ServerState holds derived indexes.
    SyncResponse {
        server: commy_core::Server,
        profiles: Vec<commy_core::UserProfile>,
        recent_messages: Vec<commy_core::Message>,
    },
    /// Voice signaling: ask a peer to open a direct media connection.
    VoiceDial {
        channel: commy_core::ChannelId,
        from_user: commy_core::UserId,
    },
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

/// Builds the snapshot a newcomer receives, capped to the most recent
/// messages per channel so the frame stays small.
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

/// Snapshot for a newcomer, trimmed to fit one gossip frame: the newest
/// messages are kept and older ones dropped until it fits. Emojis, sounds,
/// rules and calendar events are left out (they alone can exceed a frame);
/// members re-announce them one event at a time.
pub fn build_sync_response(state: &ServerState, per_channel: usize) -> WirePayload {
    let mut server = state.server.clone();
    server.emojis.clear();
    server.soundboard.clear();
    server.rules = None;
    server.events.clear();
    server.deleted_events.clear();
    let mut recent = Vec::new();
    for channel in &state.server.channels {
        recent.extend(state.page(channel.id, None, per_channel).iter().cloned());
    }
    // Newest last, so trimming from the front drops the oldest.
    recent.sort_by_key(|m| m.created_at);
    let build = |messages: Vec<commy_core::Message>| WirePayload::SyncResponse {
        server: server.clone(),
        profiles: state.profiles.values().cloned().collect(),
        recent_messages: messages,
    };
    let mut payload = build(recent.clone());
    while gossip_size(&payload) > MAX_GOSSIP_BYTES && !recent.is_empty() {
        let drop = (recent.len() / 4).max(1);
        recent.drain(..drop);
        payload = build(recent.clone());
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Message, Server, ServerState, UserId, UserProfile};

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

    #[test]
    fn sync_response_caps_messages() {
        let owner = UserId::new();
        let server = Server::new("s", owner);
        let channel = server.channels[0].id;
        let mut state = ServerState::new(server);
        state.profiles.insert(owner, UserProfile::new("owner"));
        for i in 0..100 {
            state
                .apply(ServerEvent::MessageCreate {
                    message: Message::new(channel, owner, format!("m{i}")),
                })
                .unwrap();
        }
        let WirePayload::SyncResponse {
            recent_messages, ..
        } = build_sync_response(&state, 50)
        else {
            panic!("wrong payload kind");
        };
        // At most 50 per channel, fewer if needed to fit one gossip frame.
        assert!(!recent_messages.is_empty() && recent_messages.len() <= 50);
        assert_eq!(recent_messages.last().unwrap().content, "m99");
    }

    #[test]
    fn sync_response_fits_one_gossip_frame() {
        let owner = UserId::new();
        let server = Server::new("busy", owner);
        let channel = server.channels[0].id;
        let mut state = ServerState::new(server);
        let mut profile = UserProfile::new("owner");
        profile.id = owner;
        state.profiles.insert(owner, profile);
        for i in 0..200 {
            let msg = Message::new(channel, owner, format!("message number {i} with some padding text"));
            state.apply(ServerEvent::MessageCreate { message: msg }).unwrap();
        }
        let payload = build_sync_response(&state, 50);
        assert!(gossip_size(&payload) <= MAX_GOSSIP_BYTES);
        let WirePayload::SyncResponse { recent_messages, .. } = payload else { panic!("wrong payload") };
        assert!(!recent_messages.is_empty(), "keeps as many recent messages as fit");
        assert!(recent_messages.last().unwrap().content.contains("199"), "newest message survives");
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
        let payload = build_sync_response(&state, 50);
        assert!(gossip_size(&payload) <= MAX_GOSSIP_BYTES);
        let WirePayload::SyncResponse { server, .. } = payload else { panic!("wrong payload kind") };
        assert!(server.emojis.is_empty() && server.soundboard.is_empty());
    }
}
