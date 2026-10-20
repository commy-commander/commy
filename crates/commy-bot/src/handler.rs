use chrono::{DateTime, Duration, Utc};
use commy_core::{ChannelId, Message, MessageId, RoleId, ServerId, ServerState, UserId};

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
}

impl BotEvent {
    pub fn message(&self) -> Option<&Message> {
        match self {
            BotEvent::MessageCreate(m) | BotEvent::MessageUpdate(m) => Some(m),
            _ => None,
        }
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
