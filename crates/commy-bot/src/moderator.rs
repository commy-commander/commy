use crate::handler::{written, Act, BotEvent, Ctx, Handler};
use chrono::{DateTime, Duration, Utc};
use commy_core::automod::moderates;
use commy_core::{ChannelId, ChannelKind, Message, MessageId, Permissions, ServerId, ServerState, UserId};
use std::collections::HashMap;

pub const REVIEW_FOR_HOURS: i64 = crate::KEEP_HOURS;
pub const STRIKE_WINDOW_MINUTES: i64 = 60;
pub const MAX_BAN_AFTER: u32 = 50;
const NAME_CHARS: usize = 40;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    pub ban_after: Option<u32>,
    pub log_channel: Option<String>,
}

type Strikes = Vec<(DateTime<Utc>, MessageId)>;

#[derive(Debug, Default)]
pub struct Moderator {
    policy: Policy,
    strikes: HashMap<(ServerId, UserId), Strikes>,
}

impl Moderator {
    pub fn new(policy: Policy) -> Self {
        Self { policy, strikes: HashMap::new() }
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    pub fn review(&mut self, state: &ServerState, me: UserId, message: &Message, now: DateTime<Utc>) -> Vec<Act> {
        let written = written(message);
        let Some(joined) = state.server.member(me).map(|m| m.joined_at) else { return Vec::new() };
        if message.author == me || written < joined || now - written > Duration::hours(REVIEW_FOR_HOURS) {
            return Vec::new();
        }
        let Some(hold) = state.hold(message) else { return Vec::new() };
        if !can_remove(state, me) {
            return Vec::new();
        }
        let author = message.author;
        let name = display_name(state, author);
        tracing::info!("removing a message in {}: {}", state.server.name, hold.describe());
        let mut acts = vec![Act::Remove { message: message.id }];
        let mut note = format!("Removed a message from {name}: {}", hold.describe());
        if let Some(limit) = self.policy.ban_after {
            let key = (state.server.id, author);
            let list = self.strikes.entry(key).or_default();
            if !list.iter().any(|(_, id)| *id == message.id) {
                let at = list.partition_point(|(t, _)| *t <= written);
                list.insert(at, (written, message.id));
            }
            let count = busiest_hour(list);
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
            acts.push(Act::Send { channel, content: note, reply_to: None });
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
        let keep = Duration::hours(REVIEW_FOR_HOURS) + Duration::minutes(STRIKE_WINDOW_MINUTES);
        self.strikes.retain(|_, list| {
            list.retain(|(t, _)| now - *t <= keep);
            !list.is_empty()
        });
    }
}

impl Handler for Moderator {
    fn handle(&mut self, ctx: &Ctx, event: &BotEvent) -> Vec<Act> {
        match event.message() {
            Some(message) => self.review(ctx.state, ctx.me, message, ctx.now),
            None => Vec::new(),
        }
    }

    fn tidy(&mut self, now: DateTime<Utc>) {
        self.forget(now);
    }
}

fn busiest_hour(sorted: &[(DateTime<Utc>, MessageId)]) -> usize {
    let window = Duration::minutes(STRIKE_WINDOW_MINUTES);
    let mut start = 0;
    let mut most = 0;
    for (end, (t, _)) in sorted.iter().enumerate() {
        while *t - sorted[start].0 > window {
            start += 1;
        }
        most = most.max(end + 1 - start);
    }
    most
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
