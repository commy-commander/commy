use chrono::{DateTime, Duration, Utc};
use commy_core::{BotCommand, ChannelId, Message, MessageId, RoleId, ServerId, ServerState, UserId};
use std::collections::BTreeMap;

pub const LATE_AFTER_SECONDS: i64 = 120;

#[derive(Debug, Clone, PartialEq)]
pub enum BotEvent {
    Ready,
    MessageCreate(Message),
    MessageUpdate(Message),
    MessageDelete { channel: ChannelId, message: MessageId },
    ReactionAdd { channel: ChannelId, message: MessageId, user: UserId, emoji: String },
    ReactionRemove { channel: ChannelId, message: MessageId, user: UserId, emoji: String },
    MemberJoin(UserId),
    MemberLeave(UserId),
    CommandInvoked(CommandInvoked),
}

impl BotEvent {
    pub fn message(&self) -> Option<&Message> {
        match self {
            BotEvent::MessageCreate(m) | BotEvent::MessageUpdate(m) => Some(m),
            BotEvent::CommandInvoked(c) => Some(&c.message),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommandInvoked {
    pub message: Message,
    pub command: String,
    pub options: BTreeMap<String, String>,
}

impl CommandInvoked {
    pub fn option(&self, name: &str) -> Option<&str> {
        self.options.get(name).map(String::as_str)
    }

    pub fn reply(&self, text: impl Into<String>) -> Act {
        Act::Send { channel: self.message.channel, content: text.into(), reply_to: Some(self.message.id) }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    Send { channel: ChannelId, content: String, reply_to: Option<MessageId> },
    Edit { message: MessageId, content: String },
    Remove { message: MessageId },
    React { message: MessageId, emoji: String, active: bool },
    Kick { user: UserId },
    Ban { user: UserId },
    AddRole { user: UserId, role: RoleId },
    RemoveRole { user: UserId, role: RoleId },
    SetCommands { commands: Vec<BotCommand> },
}

pub struct Ctx<'a> {
    pub server: ServerId,
    pub state: &'a ServerState,
    pub me: UserId,
    pub now: DateTime<Utc>,
}

impl Ctx<'_> {
    pub fn is_late(&self, message: &Message) -> bool {
        self.now - written(message) > Duration::seconds(LATE_AFTER_SECONDS)
    }

    pub fn name_of(&self, user: UserId) -> &str {
        self.state.profiles.get(&user).map_or("someone", |p| p.visible_name())
    }
}

pub fn written(message: &Message) -> DateTime<Utc> {
    message.edited_at.unwrap_or(message.created_at)
}

pub trait Handler: Send {
    fn handle(&mut self, ctx: &Ctx, event: &BotEvent) -> Vec<Act>;

    fn tidy(&mut self, _now: DateTime<Utc>) {}
}

impl<F> Handler for F
where
    F: FnMut(&Ctx, &BotEvent) -> Vec<Act> + Send,
{
    fn handle(&mut self, ctx: &Ctx, event: &BotEvent) -> Vec<Act> {
        self(ctx, event)
    }
}
