//! P2P transport for Commy.
//!
//! Architecture:
//! - Every running app is an iroh endpoint identified by a public key.
//! - A Commy server maps to a gossip topic. All server events
//!   (`commy_core::ServerEvent`) are broadcast to the topic and applied
//!   to each member's local `ServerState`.
//! - Invites are self-contained tickets: topic id plus the addresses of
//!   one or more members to bootstrap from. No central registry. Invite
//!   links store tickets sealed with a key that only travels in the link.
//! - Files, avatars, emojis and soundboard clips transfer via
//!   iroh-blobs, addressed by blake3 hash. Messages only carry hashes.
//! - Voice and screen share use direct QUIC connections negotiated over
//!   the gossip topic (signaling), falling back to iroh relays when
//!   hole punching fails.

pub mod dm;
pub mod media;
pub mod node;
pub mod protocol;
pub mod relays;
pub mod sealed;
pub mod ticket;

pub use dm::{DmAck, DmEnvelope, DM_ALPN};
pub use media::{MediaHello, MediaPacket, MEDIA_ALPN};
pub use node::{CommyNode, MediaChannel, NetEvent};
pub use protocol::{Envelope, WirePayload, PROTOCOL_VERSION};
pub use sealed::{SealError, SealKey};
pub use ticket::{ServerTicket, TicketError};
