use crate::handler::Act;
use crate::Bot;
use chrono::{DateTime, Duration, Utc};
use commy_core::channel_key::{self, Keyring};
use commy_core::server::valid_reaction;
use commy_core::command::{valid_commands, MAX_COMMANDS, MAX_DESCRIPTION_CHARS, MAX_NAME_CHARS, MAX_OPTIONS};
use commy_core::{
    action, Identity, Message, Permissions, ServerEvent, ServerId, ServerState, SignedEvent, TombstoneProof, UserId,
};
use commy_net::protocol::{gossip_size, MAX_GOSSIP_BYTES};
use commy_net::WirePayload;

pub const MAX_CONTENT_BYTES: usize = 4000;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActError {
    #[error("the bot is not in that server")]
    UnknownServer,
    #[error("the bot has no copy of that server yet; it gets one when the owner's app is online")]
    NotReady,
    #[error("the bot is not a member of that server any more")]
    NotMember,
    #[error("no such channel")]
    UnknownChannel,
    #[error("no such message (the bot keeps one day of messages)")]
    UnknownMessage,
    #[error("no such member")]
    UnknownMember,
    #[error("no such role")]
    UnknownRole,
    #[error("the bot needs a role with {0}")]
    Missing(&'static str),
    #[error("the bot can only edit its own messages")]
    NotOwn,
    #[error("the server owner cannot be removed")]
    Owner,
    #[error("the bot cannot remove itself")]
    Itself,
    #[error("a message needs some text")]
    Empty,
    #[error("that message is too long")]
    TooLong,
    #[error("this server's auto moderation would hold that message ({0})")]
    Held(String),
    #[error("the bot has not been given the key to that private channel yet")]
    KeyPending,
    #[error("a reaction is one emoji, or a custom emoji of this server written as :name:")]
    BadEmoji,
    #[error("the server did not accept that: {0}")]
    Refused(String),
    #[error("commands need unique lowercase names of up to {MAX_NAME_CHARS} letters, digits and dashes, descriptions of up to {MAX_DESCRIPTION_CHARS} characters, at most {MAX_OPTIONS} options with required ones first, and at most {MAX_COMMANDS} commands")]
    BadCommands,
}

impl Act {
    fn needs(&self) -> &'static str {
        match self {
            Act::Send { .. } | Act::Edit { .. } => "Send Messages in that channel",
            Act::React { .. } => "Add Reactions",
            Act::Remove { .. } => "Manage Messages",
            Act::Kick { .. } => "Kick Members",
            Act::Ban { .. } => "Ban Members",
            Act::AddRole { .. } | Act::RemoveRole { .. } => "Manage Roles",
            Act::SetCommands { .. } => "membership",
        }
    }
}

impl Bot {
    pub fn perform(&mut self, server: ServerId, act: Act, wall: DateTime<Utc>) -> Result<WirePayload, ActError> {
        let me = self.profile.id;
        let needs = act.needs();
        let replica = self.replicas.get_mut(&server).ok_or(ActError::UnknownServer)?;
        let state = replica.state.as_mut().ok_or(ActError::NotReady)?;
        if !state.server.is_current_member(me) {
            return Err(ActError::NotMember);
        }
        let identity = &self.identity;
        let signed = |event: ServerEvent| WirePayload::Signed(SignedEvent::sign(server, me, wall.timestamp_millis(), event, identity));
        let payload = match act {
            Act::Send { channel, content, reply_to } => {
                state.server.channel(channel).ok_or(ActError::UnknownChannel)?;
                if !state.server.can_post_in(channel, me) {
                    return Err(ActError::Missing(needs));
                }
                let mut message = Message::new(channel, me, clean(&content)?);
                message.created_at = wall;
                message.reply_to = reply_to.filter(|id| state.has_message(*id));
                if let Some(hold) = state.hold(&message) {
                    return Err(ActError::Held(hold.describe().to_string()));
                }
                if private(state, channel) {
                    let ring = Keyring::unlock(state, channel, me, identity);
                    let (key, secret) = ring.sending_key(state, channel).ok_or(ActError::KeyPending)?;
                    channel_key::seal_message(server, &mut message, key, secret);
                }
                message.sign(server, identity);
                WirePayload::Event(ServerEvent::MessageCreate { message })
            }
            Act::Edit { message, content } => {
                let stored = state.message(message).ok_or(ActError::UnknownMessage)?.clone();
                if stored.author != me {
                    return Err(ActError::NotOwn);
                }
                let original = crate::readable(state, me, identity, &stored).ok_or(ActError::KeyPending)?;
                let content = clean(&content)?;
                let edited_at = match original.edited_at {
                    Some(last) if wall <= last => last + Duration::milliseconds(1),
                    _ => wall,
                };
                let edited = Message { content: content.clone(), edited_at: Some(edited_at), ..original.clone() };
                if let Some(hold) = state.hold(&edited) {
                    return Err(ActError::Held(hold.describe().to_string()));
                }
                let content = seal_edit(state, me, identity, &stored, &original, content)?;
                WirePayload::Event(stored.edit_event(server, content, edited_at, identity))
            }
            Act::Remove { message } => {
                let found = state.message(message).ok_or(ActError::UnknownMessage)?;
                if found.author != me && !state.server.permissions_for(me).allows(Permissions::MANAGE_MESSAGES) {
                    return Err(ActError::Missing(needs));
                }
                let proof = TombstoneProof::sign(server, found.channel, message, me, wall, identity);
                WirePayload::Event(proof.into_event())
            }
            Act::React { message, emoji, active } => {
                let emoji = emoji.trim().to_string();
                if !valid_reaction(&emoji) {
                    return Err(ActError::BadEmoji);
                }
                let channel = state.message(message).ok_or(ActError::UnknownMessage)?.channel;
                signed(ServerEvent::ReactionSet { channel, message, emoji, user: me, active })
            }
            Act::Kick { user } => signed(ServerEvent::MemberLeave { server, user: removable(state, me, user)? }),
            Act::Ban { user } => signed(ServerEvent::MemberBan { server, user: removable(state, me, user)? }),
            Act::AddRole { user, role } => signed(ServerEvent::RoleAssign { server, user: member(state, user)?, role: known_role(state, role)? }),
            Act::RemoveRole { user, role } => {
                signed(ServerEvent::RoleUnassign { server, user: member(state, user)?, role: known_role(state, role)? })
            }
            Act::SetCommands { commands } => {
                if !valid_commands(&commands) {
                    return Err(ActError::BadCommands);
                }
                signed(ServerEvent::BotCommandsSet { server, bot: me, commands, by: me })
            }
        };
        if gossip_size(&payload) > MAX_GOSSIP_BYTES {
            return Err(ActError::TooLong);
        }
        let applied = match &payload {
            WirePayload::Signed(signed) => {
                if !action::authorized(&state.server, me, &signed.event) {
                    return Err(ActError::Missing(needs));
                }
                state.apply_signed(signed)
            }
            WirePayload::Event(event) => state.apply(event.clone()),
            _ => Ok(false),
        };
        applied.map_err(|e| ActError::Refused(e.to_string()))?;
        if let WirePayload::Signed(SignedEvent { event: ServerEvent::BotCommandsSet { commands, .. }, .. }) = &payload {
            self.commands.insert(server, commands.clone());
        }
        Ok(payload)
    }
}

fn private(state: &ServerState, channel: commy_core::ChannelId) -> bool {
    state.server.channel(channel).is_some_and(|c| c.is_private())
}

fn seal_edit(
    state: &ServerState,
    me: UserId,
    identity: &Identity,
    stored: &Message,
    opened: &Message,
    content: String,
) -> Result<String, ActError> {
    if !channel_key::is_sealed(&stored.content) && !private(state, stored.channel) {
        return Ok(content);
    }
    let ring = Keyring::unlock(state, stored.channel, me, identity);
    let (key, secret) = ring.sending_key(state, stored.channel).ok_or(ActError::KeyPending)?;
    Ok(channel_key::seal_body(state.server.id, stored, &content, &opened.attachments, key, secret))
}

fn clean(content: &str) -> Result<String, ActError> {
    let content = content.trim();
    match content.len() {
        0 => Err(ActError::Empty),
        n if n > MAX_CONTENT_BYTES => Err(ActError::TooLong),
        _ => Ok(content.to_string()),
    }
}

fn member(state: &ServerState, user: UserId) -> Result<UserId, ActError> {
    state.server.is_current_member(user).then_some(user).ok_or(ActError::UnknownMember)
}

fn removable(state: &ServerState, me: UserId, user: UserId) -> Result<UserId, ActError> {
    if user == me {
        return Err(ActError::Itself);
    }
    if user == state.server.owner {
        return Err(ActError::Owner);
    }
    member(state, user)
}

fn known_role(state: &ServerState, role: commy_core::RoleId) -> Result<commy_core::RoleId, ActError> {
    state.server.roles.iter().any(|r| r.id == role).then_some(role).ok_or(ActError::UnknownRole)
}
