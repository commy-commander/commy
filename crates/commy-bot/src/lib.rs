pub mod actions;
pub mod bridge;
pub mod compat;
pub mod handler;
pub mod moderator;
pub mod replica;
pub mod runner;
pub mod storage;
pub mod store;
pub mod webhook;

pub use actions::ActError;
pub use handler::{Act, BotEvent, CommandInvoked, Ctx, Handler};
pub use moderator::{Moderator, Policy};
pub use replica::Replica;
pub use store::{BotStore, Outgoing};

use chrono::{DateTime, Utc};
use commy_core::channel_key::{self, KeyScope, Keyring};
use commy_core::{action, Action, BotCommand, Identity, Message, MessageId, Presence, ServerEvent, ServerId, ServerState, SignedEvent, TombstoneProof, UserId, UserProfile};
use commy_net::catchup::ApplyReport;
use commy_net::WirePayload;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

pub const KEEP_HOURS: i64 = 24;
pub const MAX_ARRIVALS: usize = 50_000;
const RECENT_ARRIVAL_MINUTES: i64 = 60;

pub struct Bot {
    identity: Identity,
    profile: UserProfile,
    replicas: BTreeMap<ServerId, Replica>,
    moderator: Moderator,
    handlers: Vec<Box<dyn Handler>>,
    commands: BTreeMap<ServerId, Vec<BotCommand>>,
    arrivals: HashMap<MessageId, DateTime<Utc>>,
}

impl Bot {
    pub fn new(identity: Identity, mut profile: UserProfile, moderator: Moderator) -> Self {
        profile.presence = Presence::Online;
        profile.identity = Some(identity.public());
        Self { identity, profile, replicas: BTreeMap::new(), moderator, handlers: Vec::new(), commands: BTreeMap::new(), arrivals: HashMap::new() }
    }

    pub fn with(mut self, handler: impl Handler + 'static) -> Self {
        self.handlers.push(Box::new(handler));
        self
    }

    pub fn add(&mut self, replica: Replica) {
        self.replicas.insert(replica.server(), replica);
    }

    pub fn profile(&self) -> &UserProfile {
        &self.profile
    }

    pub fn replica(&self, server: ServerId) -> Option<&Replica> {
        self.replicas.get(&server)
    }

    pub fn replicas(&self) -> impl Iterator<Item = &Replica> {
        self.replicas.values()
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    pub fn moderator(&self) -> &Moderator {
        &self.moderator
    }

    pub fn commands(&self, server: ServerId) -> &[BotCommand] {
        self.commands.get(&server).map_or(&[], Vec::as_slice)
    }

    pub fn register_commands(&mut self, server: ServerId, commands: Vec<BotCommand>, wall: DateTime<Utc>) -> Result<WirePayload, ActError> {
        self.perform(server, Act::SetCommands { commands }, wall)
    }

    pub fn announcements(&mut self, server: ServerId, now_ms: i64) -> Vec<WirePayload> {
        let me = self.profile.as_seen_by_others();
        let Some(replica) = self.replicas.get_mut(&server).filter(|r| !r.banned(me.id)) else { return Vec::new() };
        let commands = self.commands.get(&server).filter(|c| !c.is_empty()).cloned();
        let events = [
            Some(ServerEvent::MemberJoin { server, profile: me.clone() }),
            Some(ServerEvent::BotSet { server, user: me.id, bot: true }),
            commands.map(|commands| ServerEvent::BotCommandsSet { server, bot: me.id, commands, by: me.id }),
            Some(ServerEvent::PresenceUpdate { user: me.id, presence: Presence::Online }),
        ];
        let mut out: Vec<WirePayload> = events
            .into_iter()
            .flatten()
            .map(|event| {
                let payload = replica.signed(&me, &self.identity, event, now_ms);
                replica.apply_own(&payload);
                payload
            })
            .collect();
        match &replica.state {
            Some(state) => {
                let relayed = state.signed_actions().into_iter().filter(|s| s.action.by != me.id);
                out.extend(relayed.cloned().map(WirePayload::Signed));
            }
            None => out.push(WirePayload::SyncRequest),
        }
        out
    }

    pub fn presence(&mut self, server: ServerId, presence: Presence, now_ms: i64) -> Option<WirePayload> {
        let me = self.profile.clone();
        let replica = self.replicas.get_mut(&server).filter(|r| !r.banned(me.id))?;
        let payload = replica.signed(&me, &self.identity, ServerEvent::PresenceUpdate { user: me.id, presence }, now_ms);
        replica.apply_own(&payload);
        Some(payload)
    }

    pub fn on_snapshot(
        &mut self,
        server: ServerId,
        payload: WirePayload,
        now: Instant,
        wall: DateTime<Utc>,
    ) -> Option<Vec<WirePayload>> {
        let me = self.profile.as_seen_by_others();
        let fresh = self.replicas.get_mut(&server)?.accept_snapshot(payload, &me, now, wall.timestamp_millis())?;
        let mut out = self.announcements(server, wall.timestamp_millis());
        out.extend(self.review(server, fresh, wall));
        out.extend(self.tend_community_key(server, wall));
        out.extend(self.say_day_keys_ready(server, wall));
        Some(out)
    }

    pub fn on_event(
        &mut self,
        server: ServerId,
        event: ServerEvent,
        action: Option<Action>,
        now: Instant,
        wall: DateTime<Utc>,
    ) -> Vec<WirePayload> {
        let Some(replica) = self.replicas.get_mut(&server) else { return Vec::new() };
        let banned = matches!(event, ServerEvent::MemberBan { .. });
        if let Some(message) = arriving(&event) {
            note_arrival(&mut self.arrivals, message, wall);
        }
        let fresh = replica.apply(event, action, now);
        let mut out = self.review(server, fresh, wall);
        if banned {
            out.extend(self.tend_community_key(server, wall));
        }
        out
    }

    pub fn gossip_keys(&self, server: ServerId) -> Option<commy_net::gossip_seal::ServerKeys> {
        let state = self.replicas.get(&server)?.state.as_ref()?;
        let ring = Keyring::unlock_community(state, self.profile.id, &self.identity);
        let days = commy_core::day_keys::DayKeyring::unlock(state, self.profile.id, &self.identity);
        Some(commy_net::gossip_seal::server_keys_with_days(state, &ring, &days))
    }

    pub fn say_day_keys_ready(&mut self, server: ServerId, wall: DateTime<Utc>) -> Vec<WirePayload> {
        let me = self.profile.id;
        let Some(state) = self.replicas.get_mut(&server).and_then(|r| r.state.as_mut()) else { return Vec::new() };
        let Some(ready) = commy_core::day_chain::ready_event(&state.server, me) else { return Vec::new() };
        let at = state.signing_time(&ready, wall.timestamp_millis());
        let signed = SignedEvent::sign(server, me, at, ready, &self.identity);
        if !action::authorized(&state.server, me, &signed.event) || state.apply_signed(&signed).is_err() {
            return Vec::new();
        }
        vec![WirePayload::Signed(signed)]
    }

    pub fn tend_community_key(&mut self, server: ServerId, wall: DateTime<Utc>) -> Vec<WirePayload> {
        let me = self.profile.id;
        let Some(state) = self.replicas.get_mut(&server).and_then(|r| r.state.as_mut()) else { return Vec::new() };
        let mut ring = Keyring::unlock_community(state, me, &self.identity);
        let Some(task) = channel_key::community_key_task(state, me, &ring) else { return Vec::new() };
        let at = wall.timestamp_millis();
        let mut out = Vec::new();
        for event in ring.task_events(state, KeyScope::Community, task, at) {
            let signed = SignedEvent::sign(server, me, state.signing_time(&event, at), event, &self.identity);
            if !action::authorized(&state.server, me, &signed.event) || state.apply_signed(&signed).is_err() {
                break;
            }
            out.push(WirePayload::Signed(signed));
        }
        out
    }

    pub fn arrived_at(&self, message: MessageId) -> Option<DateTime<Utc>> {
        self.arrivals.get(&message).copied()
    }

    pub fn on_caught_up(
        &mut self,
        server: ServerId,
        messages: Vec<Message>,
        tombstones: Vec<TombstoneProof>,
        now: Instant,
        wall: DateTime<Utc>,
    ) -> (ApplyReport, Vec<WirePayload>) {
        let Some(replica) = self.replicas.get_mut(&server) else {
            let rejected = messages.iter().map(|m| m.id).chain(tombstones.iter().map(|p| p.message)).collect();
            return (ApplyReport { rejected, ..ApplyReport::default() }, Vec::new());
        };
        let (report, fresh) = replica.apply_caught_up(messages, tombstones, now);
        (report, self.review(server, fresh, wall))
    }

    pub fn on_profiles(
        &mut self,
        server: ServerId,
        profiles: Vec<UserProfile>,
        now: Instant,
        wall: DateTime<Utc>,
    ) -> Vec<WirePayload> {
        let Some(replica) = self.replicas.get_mut(&server) else { return Vec::new() };
        let fresh = replica.learn_profiles(profiles, now);
        self.review(server, fresh, wall)
    }

    fn review(&mut self, server: ServerId, events: Vec<BotEvent>, wall: DateTime<Utc>) -> Vec<WirePayload> {
        let me = self.profile.id;
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for event in events {
            let acts = {
                let Some(state) = self.replicas.get(&server).and_then(|r| r.state.as_ref()) else { return out };
                let Some(event) = current(state, me, &self.identity, event, &self.arrivals, &mut seen) else { continue };
                let ctx = Ctx { server, state, me, now: wall };
                let mut acts = match event.message() {
                    Some(message) => self.moderator.review(state, me, message, wall, self.arrivals.get(&message.id).copied()),
                    None => Vec::new(),
                };
                let invoked = invocation(state, me, &event).map(BotEvent::CommandInvoked);
                for handler in &mut self.handlers {
                    acts.extend(handler.handle(&ctx, &event));
                    if let Some(invoked) = &invoked {
                        acts.extend(handler.handle(&ctx, invoked));
                    }
                }
                acts
            };
            for act in acts {
                let bans = matches!(act, Act::Ban { .. });
                match self.perform(server, act, wall) {
                    Ok(payload) => {
                        out.push(payload);
                        if bans {
                            out.extend(self.tend_community_key(server, wall));
                        }
                    }
                    Err(e) => tracing::warn!("{}: {e}", self.replicas.get(&server).map_or("", |r| r.name())),
                }
            }
        }
        out
    }

    pub fn readable(&self, state: &ServerState, message: &Message) -> Option<Message> {
        readable(state, self.profile.id, &self.identity, message)
    }

    pub fn tidy(&mut self, wall: DateTime<Utc>) {
        self.moderator.tidy(wall);
        let kept_since = wall - chrono::Duration::hours(KEEP_HOURS);
        self.arrivals.retain(|_, at| *at >= kept_since);
        for handler in &mut self.handlers {
            handler.tidy(wall);
        }
        let floor = wall - chrono::Duration::hours(KEEP_HOURS);
        for state in self.replicas.values_mut().filter_map(|r| r.state.as_mut()) {
            state.prune_expired(wall);
            state.roll_off(Some(floor));
        }
    }
}

fn arriving(event: &ServerEvent) -> Option<MessageId> {
    match event {
        ServerEvent::MessageCreate { message } => Some(message.id),
        ServerEvent::MessageEdit { message, .. } => Some(*message),
        _ => None,
    }
}

fn note_arrival(arrivals: &mut HashMap<MessageId, DateTime<Utc>>, message: MessageId, wall: DateTime<Utc>) {
    if arrivals.len() >= MAX_ARRIVALS {
        let recent = wall - chrono::Duration::minutes(RECENT_ARRIVAL_MINUTES);
        arrivals.retain(|_, at| *at >= recent);
    }
    if arrivals.len() >= MAX_ARRIVALS {
        arrivals.clear();
    }
    arrivals.insert(message, wall);
}

fn current(
    state: &ServerState,
    me: UserId,
    identity: &Identity,
    event: BotEvent,
    arrivals: &HashMap<MessageId, DateTime<Utc>>,
    seen: &mut HashSet<MessageId>,
) -> Option<BotEvent> {
    match &event {
        BotEvent::ReactionAdd { user, .. } | BotEvent::ReactionRemove { user, .. } if *user == me => return None,
        _ => {}
    }
    let Some(message) = event.message() else { return Some(event) };
    let joined = state.server.member(me)?.joined_at;
    if !seen.insert(message.id) || message.author == me {
        return None;
    }
    let message = readable(state, me, identity, state.message(message.id)?)?;
    if !arrivals.contains_key(&message.id) && handler::written(&message) < joined {
        return None;
    }
    Some(match event {
        BotEvent::MessageUpdate(_) => BotEvent::MessageUpdate(message),
        _ => BotEvent::MessageCreate(message),
    })
}

fn invocation(state: &ServerState, me: UserId, event: &BotEvent) -> Option<CommandInvoked> {
    let BotEvent::MessageCreate(message) = event else { return None };
    let used = message.command_use().filter(|u| u.bot == me)?;
    let command = state.command_of(&used)?;
    if !state.server.can_post_in(message.channel, message.author) {
        return None;
    }
    Some(CommandInvoked { message: message.clone(), command: command.name.clone(), options: used.options })
}

pub(crate) fn readable(state: &ServerState, me: UserId, identity: &Identity, message: &Message) -> Option<Message> {
    if !channel_key::is_sealed(&message.content) {
        return Some(message.clone());
    }
    let ring = Keyring::unlock(state, message.channel, me, identity);
    channel_key::read_message(state.server.id, message, &ring)
}
