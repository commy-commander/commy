use chrono::{DateTime, Duration, Utc};
use commy_core::automod::moderates;
use commy_core::{ChannelId, ChannelKind, Hold, Message, Permissions, ServerId, ServerState, UserId};
use std::collections::{HashMap, VecDeque};

pub const FRESH_FOR_MINUTES: i64 = 10;
pub const STRIKE_WINDOW_MINUTES: i64 = 60;
pub const MAX_BAN_AFTER: u32 = 50;
const NAME_CHARS: usize = 40;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    pub ban_after: Option<u32>,
    pub log_channel: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    Remove { message: Message, hold: Hold },
    Ban { user: UserId },
    Note { channel: ChannelId, text: String },
}

#[derive(Debug, Default)]
pub struct Moderator {
    policy: Policy,
    strikes: HashMap<(ServerId, UserId), VecDeque<DateTime<Utc>>>,
}

impl Moderator {
    pub fn new(policy: Policy) -> Self {
        Self { policy, strikes: HashMap::new() }
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    pub fn review(&mut self, state: &ServerState, me: UserId, message: &Message, now: DateTime<Utc>) -> Vec<Act> {
        let written = message.edited_at.unwrap_or(message.created_at);
        if message.author == me || now - written > Duration::minutes(FRESH_FOR_MINUTES) {
            return Vec::new();
        }
        let Some(hold) = state.hold(message) else { return Vec::new() };
        if !can_remove(state, me) {
            return Vec::new();
        }
        let author = message.author;
        let name = display_name(state, author);
        let mut acts = vec![Act::Remove { message: message.clone(), hold }];
        let mut note = format!("Removed a message from {name}: {}", hold.describe());
        if let Some(limit) = self.policy.ban_after {
            let window = Duration::minutes(STRIKE_WINDOW_MINUTES);
            let key = (state.server.id, author);
            let list = self.strikes.entry(key).or_default();
            list.push_back(now);
            while list.front().is_some_and(|t| now - *t > window) {
                list.pop_front();
            }
            let count = list.len();
            if count >= limit as usize && can_ban(state, me, author) {
                self.strikes.remove(&key);
                acts.push(Act::Ban { user: author });
                note = format!(
                    "Removed a message from {name} ({}) and banned them after {count} removed messages in an hour",
                    hold.describe()
                );
            }
        }
        if let Some(channel) = self.log_channel(state, me) {
            acts.push(Act::Note { channel, text: note });
        }
        acts
    }

    pub fn log_channel(&self, state: &ServerState, me: UserId) -> Option<ChannelId> {
        let wanted = self.policy.log_channel.as_deref()?.trim().trim_start_matches('#');
        state
            .server
            .channels
            .iter()
            .filter(|c| matches!(c.kind, ChannelKind::Text | ChannelKind::Announcement))
            .find(|c| c.name.eq_ignore_ascii_case(wanted))
            .map(|c| c.id)
            .filter(|id| state.server.can_post_in(*id, me))
    }

    pub fn forget(&mut self, now: DateTime<Utc>) {
        let window = Duration::minutes(STRIKE_WINDOW_MINUTES);
        self.strikes.retain(|_, list| list.back().is_some_and(|t| now - *t <= window));
    }
}

pub fn can_remove(state: &ServerState, me: UserId) -> bool {
    state.server.is_current_member(me) && state.server.permissions_for(me).allows(Permissions::MANAGE_MESSAGES)
}

pub fn can_ban(state: &ServerState, me: UserId, user: UserId) -> bool {
    let server = &state.server;
    server.permissions_for(me).allows(Permissions::BAN_MEMBERS) && user != me && !moderates(server, user)
}

fn display_name(state: &ServerState, user: UserId) -> String {
    let name = state.profiles.get(&user).map_or("someone", |p| p.visible_name());
    name.chars().filter(|c| !c.is_control()).take(NAME_CHARS).collect()
}
