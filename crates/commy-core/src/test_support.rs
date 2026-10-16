//! Test helpers: members with identity keys and signed messages.

use crate::event::ServerEvent;
use crate::identity::Identity;
use crate::ids::{ChannelId, RoleId, UserId};
use crate::message::{Message, TombstoneProof};
use crate::permissions::Permissions;
use crate::server::{Role, Server};
use crate::store::ServerState;
use crate::user::UserProfile;
use chrono::{DateTime, Utc};

/// Deterministic identity per user, so tests can sign as anyone.
pub fn identity_for(user: UserId) -> Identity {
    let mut secret = [0u8; 32];
    secret[..16].copy_from_slice(user.0.as_bytes());
    secret[16..].copy_from_slice(user.0.as_bytes());
    Identity::from_bytes(&secret)
}

/// New profile carrying its test identity key.
pub fn keyed_profile(username: &str) -> UserProfile {
    let mut profile = UserProfile::new(username);
    profile.identity = Some(identity_for(profile.id).public());
    profile
}

/// Server whose owner has a keyed profile, plus its text channel.
pub fn fixture() -> (ServerState, ChannelId, UserId) {
    let owner = keyed_profile("owner");
    let owner_id = owner.id;
    let server = Server::new("test", owner_id);
    let channel = server.channels[0].id;
    let mut state = ServerState::new(server);
    state.profiles.insert(owner_id, owner);
    (state, channel, owner_id)
}

/// Adds a keyed member and returns their id.
pub fn join(state: &mut ServerState, username: &str) -> UserId {
    let profile = keyed_profile(username);
    let id = profile.id;
    let server = state.server.id;
    state.apply(ServerEvent::MemberJoin { server, profile }).unwrap();
    id
}

/// Adds a keyed member holding a role with Manage Messages.
pub fn join_moderator(state: &mut ServerState, username: &str) -> UserId {
    let server = state.server.id;
    let user = join(state, username);
    let role = Role {
        id: RoleId::new(),
        name: "mods".into(),
        color: None,
        permissions: Permissions::MANAGE_MESSAGES,
        position: 1,
        hoist: false,
        self_assignable: false,
        tag: None,
        banner: None,
        updated_at: 0,
    };
    let id = role.id;
    state.apply(ServerEvent::RoleCreate { server, role }).unwrap();
    state.apply(ServerEvent::RoleAssign { server, user, role: id, at: Utc::now().timestamp_millis() }).unwrap();
    user
}

/// Signs `message` as its author.
pub fn signed(state: &ServerState, mut message: Message) -> Message {
    message.sign(state.server.id, &identity_for(message.author));
    message
}

/// New message from `author`, signed.
pub fn post(state: &ServerState, channel: ChannelId, author: UserId, content: &str) -> Message {
    signed(state, Message::new(channel, author, content))
}

/// Edit of `message` signed by its author.
pub fn edit(state: &ServerState, message: &Message, content: &str, edited_at: DateTime<Utc>) -> ServerEvent {
    message.edit_event(state.server.id, content, edited_at, &identity_for(message.author))
}

/// Delete of `message` signed by `by`, made now.
pub fn delete(state: &ServerState, message: &Message, by: UserId) -> ServerEvent {
    proof(state, message, by, Utc::now()).into_event()
}

pub fn proof(state: &ServerState, message: &Message, by: UserId, at: DateTime<Utc>) -> TombstoneProof {
    TombstoneProof::sign(state.server.id, message.channel, message.id, by, at, &identity_for(by))
}
