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
pub use handler::{Act, BotEvent, Ctx, Handler};
pub use moderator::{Moderator, Policy};
pub use replica::Replica;
pub use store::{BotStore, Outgoing};

use chrono::{DateTime, Utc};
use commy_core::{Action, Identity, Message, Presence, ServerEvent, ServerId, TombstoneProof, UserProfile};
use commy_net::catchup::ApplyReport;
use commy_net::WirePayload;
use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

pub const KEEP_HOURS: i64 = 24;

pub struct Bot {
    identity: Identity,
    profile: UserProfile,
    replicas: BTreeMap<ServerId, Replica>,
    moderator: Moderator,
    handlers: Vec<Box<dyn Handler>>,
}

impl Bot {
    pub fn new(identity: Identity, mut profile: UserProfile, moderator: Moderator) -> Self {
        profile.presence = Presence::Online;
        profile.identity = Some(identity.public());
        Self { identity, profile, replicas: BTreeMap::new(), moderator, handlers: Vec::new() }
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

    pub fn announcements(&mut self, server: ServerId, now_ms: i64) -> Vec<WirePayload> {
        let me = self.profile.as_seen_by_others();
        let Some(replica) = self.replicas.get_mut(&server).filter(|r| !r.banned(me.id)) else { return Vec::new() };
        let events = [
            ServerEvent::MemberJoin { server, profile: me.clone() },
            ServerEvent::BotSet { server, user: me.id, bot: true },
            ServerEvent::PresenceUpdate { user: me.id, presence: Presence::Online },
        ];
        let mut out: Vec<WirePayload> = events
            .into_iter()
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
        let fresh = replica.apply(event, action, now);
        self.review(server, fresh, wall)
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
                let Some(event) = current(state, me, event, &mut seen) else { continue };
                let ctx = Ctx { server, state, me, now: wall };
                let mut acts = self.moderator.handle(&ctx, &event);
                for handler in &mut self.handlers {
                    acts.extend(handler.handle(&ctx, &event));
                }
                acts
            };
            for act in acts {
                match self.perform(server, act, wall) {
                    Ok(payload) => out.push(payload),
                    Err(e) => tracing::warn!("{}: {e}", self.replicas.get(&server).map_or("", |r| r.name())),
                }
            }
        }
        out
    }

    pub fn tidy(&mut self, wall: DateTime<Utc>) {
        self.moderator.tidy(wall);
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

fn current(state: &commy_core::ServerState, me: commy_core::UserId, event: BotEvent, seen: &mut HashSet<commy_core::MessageId>) -> Option<BotEvent> {
    match &event {
        BotEvent::ReactionAdd { user, .. } | BotEvent::ReactionRemove { user, .. } if *user == me => return None,
        _ => {}
    }
    let Some(message) = event.message() else { return Some(event) };
    let joined = state.server.member(me)?.joined_at;
    if !seen.insert(message.id) || message.author == me {
        return None;
    }
    let message = state.message(message.id)?.clone();
    if handler::written(&message) < joined {
        return None;
    }
    Some(match event {
        BotEvent::MessageUpdate(_) => BotEvent::MessageUpdate(message),
        _ => BotEvent::MessageCreate(message),
    })
}
