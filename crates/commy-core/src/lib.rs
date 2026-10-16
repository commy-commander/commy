//! Core domain model for Commy: users, servers, channels, messages,
//! permissions, and the in-memory state that every peer replicates.
//!
//! This crate is transport-agnostic. Networking lives in `commy-net`,
//! audio in `commy-audio`, and the UI in `apps/desktop`.

pub mod catchup;
pub mod dm;
pub mod event;
pub mod identity;
pub mod ids;
pub mod message;
pub mod permissions;
pub mod server;
pub mod store;
pub mod user;

#[cfg(test)]
mod test_support;

pub use catchup::{BucketDigest, BucketKey};
pub use dm::{DmMessage, DmStore};
pub use event::ServerEvent;
pub use identity::{Identity, IdentityKey, Signature};
pub use ids::{ChannelId, MessageId, RoleId, ServerId, UserId};
pub use message::{Attachment, Message, Reaction, TombstoneProof, MAX_MESSAGE_BYTES};
pub use permissions::Permissions;
pub use server::{Channel, ChannelKind, Member, Role, Server, SoundboardSound};
pub use store::{ChangeKey, MemberMark, Membership, RoleMark, ServerState, Snapshot, StoreError};
pub use user::{Avatar, Presence, UserProfile};
