pub mod moderator;
pub mod replica;
pub mod runner;
pub mod storage;

pub use moderator::{Act, Moderator, Policy};
pub use replica::Replica;

use chrono::{DateTime, Utc};
use commy_core::{Action, Identity, Message, Presence, ServerEvent, ServerId, TombstoneProof, UserProfile};
use commy_net::WirePayload;
use std::collections::BTreeMap;
use std::time::Instant;

pub const KEEP_HOURS: i64 = 24;

pub struct Bot {
    identity: Identity,
    profile: UserProfile,
    replicas: BTreeMap<ServerId, Replica>,
    moderator: Moderator,
}

impl Bot {
    pub fn new(identity: Identity, mut profile: UserProfile, moderator: Moderator) -> Self {
        profile.presence = Presence::Online;
        profile.identity = Some(identity.public());
        Self { identity, profile, replicas: BTreeMap::new(), moderator }
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

    pub fn on_snapshot(&mut self, server: ServerId, payload: WirePayload, now_ms: i64) -> Option<Vec<WirePayload>> {
        let me = self.profile.as_seen_by_others();
        let accepted = self.replicas.get_mut(&server)?.accept_snapshot(payload, &me, now_ms);
        accepted.then(|| self.announcements(server, now_ms))
    }

    pub fn on_event(
        &mut self,
        server: ServerId,
        event: ServerEvent,
        action: Option<Action>,
        now: Instant,
        wall: DateTime<Utc>,
    ) -> Vec<WirePayload> {
        let me = self.profile.clone();
        let Some(replica) = self.replicas.get_mut(&server) else { return Vec::new() };
        let fresh = replica.apply(event, action, now);
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for id in fresh.into_iter().map(|m| m.id).filter(|id| seen.insert(*id)) {
            let acts = match replica.state.as_ref().and_then(|s| Some((s, s.message(id)?))) {
                Some((state, message)) => self.moderator.review(state, me.id, message, wall),
                None => continue,
            };
            for act in acts {
                let name = replica.name().to_string();
                let payload = match act {
                    Act::Remove { message, hold } => {
                        tracing::info!("removed a message in {name}: {}", hold.describe());
                        let proof = TombstoneProof::sign(server, message.channel, message.id, me.id, wall, &self.identity);
                        WirePayload::Event(proof.into_event())
                    }
                    Act::Ban { user } => {
                        tracing::info!("banned {user} from {name}");
                        replica.signed(&me, &self.identity, ServerEvent::MemberBan { server, user }, wall.timestamp_millis())
                    }
                    Act::Note { channel, text } => {
                        let mut note = Message::new(channel, me.id, text);
                        note.sign(server, &self.identity);
                        WirePayload::Event(ServerEvent::MessageCreate { message: note })
                    }
                };
                replica.apply_own(&payload);
                out.push(payload);
            }
        }
        out
    }

    pub fn tidy(&mut self, wall: DateTime<Utc>) {
        self.moderator.forget(wall);
        let floor = wall - chrono::Duration::hours(KEEP_HOURS);
        for state in self.replicas.values_mut().filter_map(|r| r.state.as_mut()) {
            state.prune_expired(wall);
            state.roll_off(Some(floor));
        }
    }
}
