use crate::event::ServerEvent;
use crate::identity::Identity;
use crate::ids::{ChannelId, RoleId, UserId};
use crate::message::{Message, TombstoneProof};
use crate::permissions::Permissions;
use crate::server::{Role, Server};
use crate::store::ServerState;
use crate::user::UserProfile;
use chrono::{DateTime, Utc};

pub fn identity_for(user: UserId) -> Identity {
    let mut secret = [0u8; 32];
    secret[..16].copy_from_slice(user.0.as_bytes());
    secret[16..].copy_from_slice(user.0.as_bytes());
    Identity::from_bytes(&secret)
}

pub fn keyed_profile(username: &str) -> UserProfile {
    let mut profile = UserProfile::new(username);
    profile.identity = Some(identity_for(profile.id).public());
    profile
}

pub fn fixture() -> (ServerState, ChannelId, UserId) {
    let owner = keyed_profile("owner");
    let owner_id = owner.id;
    let server = Server::new("test", owner_id);
    let channel = server.channels[0].id;
    let mut state = ServerState::new(server);
    state.profiles.insert(owner_id, owner);
    (state, channel, owner_id)
}

pub fn join(state: &mut ServerState, username: &str) -> UserId {
    let profile = keyed_profile(username);
    let id = profile.id;
    let server = state.server.id;
    state.apply(ServerEvent::MemberJoin { server, profile }).unwrap();
    id
}

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
    };
    let id = role.id;
    state.apply(ServerEvent::RoleCreate { server, role }).unwrap();
    state.apply(ServerEvent::RoleAssign { server, user, role: id }).unwrap();
    user
}

pub fn signed(state: &ServerState, mut message: Message) -> Message {
    message.sign(state.server.id, &identity_for(message.author));
    message
}

pub fn post(state: &ServerState, channel: ChannelId, author: UserId, content: &str) -> Message {
    signed(state, Message::new(channel, author, content))
}

pub fn post_at(state: &ServerState, channel: ChannelId, author: UserId, content: &str, at: DateTime<Utc>) -> Message {
    signed(state, Message { created_at: at, ..Message::new(channel, author, content) })
}

pub fn edit(state: &ServerState, message: &Message, content: &str, edited_at: DateTime<Utc>) -> ServerEvent {
    message.edit_event(state.server.id, content, edited_at, &identity_for(message.author))
}

pub fn delete(state: &ServerState, message: &Message, by: UserId) -> ServerEvent {
    proof(state, message, by, Utc::now()).into_event()
}

pub fn proof(state: &ServerState, message: &Message, by: UserId, at: DateTime<Utc>) -> TombstoneProof {
    TombstoneProof::sign(state.server.id, message.channel, message.id, by, at, &identity_for(by))
}
