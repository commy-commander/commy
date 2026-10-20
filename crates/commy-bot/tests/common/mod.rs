#![allow(dead_code)]

use chrono::Utc;
use commy_bot::{Bot, Moderator, Policy, Replica};
use commy_core::test_support::{fixture, identity_for, join};
use commy_core::{
    AutoMod, ChannelId, Identity, Message, Permissions, Role, RoleId, ServerEvent, ServerId, ServerState, SignedEvent,
    UserId, UserProfile,
};
use commy_net::protocol::{build_sync_response, sign_sync_response};
use commy_net::{ServerTicket, WirePayload};
use std::time::Instant;

pub struct World {
    pub owner_state: ServerState,
    pub owner: UserId,
    pub channel: ChannelId,
    pub member: UserId,
    pub bot: Bot,
    pub bot_id: UserId,
    pub clock: i64,
}

impl World {
    pub fn new(policy: Policy) -> Self {
        Self::new_with(policy, |bot| bot)
    }

    pub fn new_with(policy: Policy, build: impl FnOnce(Bot) -> Bot) -> Self {
        let (mut owner_state, channel, owner) = fixture();
        let member = join(&mut owner_state, "member");
        let identity = Identity::generate();
        let profile = UserProfile::new("Tidy");
        let bot_id = profile.id;
        let mut bot = build(Bot::new(identity, profile, Moderator::new(policy)));
        bot.add(Replica::new(ticket(&owner_state, owner), None));
        Self { owner_state, owner, channel, member, bot, bot_id, clock: Utc::now().timestamp_millis() - 60_000 }
    }

    pub fn sid(&self) -> ServerId {
        self.owner_state.server.id
    }

    pub fn snapshot_from(&self, by: UserId) -> WirePayload {
        let mut payload = build_sync_response(&self.owner_state, 50, by);
        sign_sync_response(&mut payload, Utc::now().timestamp_millis(), &identity_for(by));
        payload
    }

    pub fn deliver(&mut self, payloads: Vec<WirePayload>) {
        for payload in payloads {
            match payload {
                WirePayload::Signed(signed) => {
                    self.owner_state.apply_signed(&signed).unwrap();
                }
                WirePayload::Event(event) => {
                    self.owner_state.apply_unsigned(event).unwrap();
                }
                _ => {}
            }
        }
    }

    pub fn by_owner(&mut self, event: ServerEvent) -> Vec<WirePayload> {
        self.by(self.owner, event)
    }

    pub fn by(&mut self, user: UserId, event: ServerEvent) -> Vec<WirePayload> {
        self.clock += 1;
        let signed = SignedEvent::sign(self.sid(), user, self.clock, event, &identity_for(user));
        self.owner_state.apply_signed(&signed).unwrap();
        let sid = self.sid();
        self.bot.on_event(sid, signed.event, Some(signed.action), Instant::now(), Utc::now())
    }

    pub fn posted(&mut self, message: Message) -> Vec<WirePayload> {
        self.owner_state.apply_unsigned(ServerEvent::MessageCreate { message: message.clone() }).unwrap();
        let sid = self.sid();
        self.bot.on_event(sid, ServerEvent::MessageCreate { message }, None, Instant::now(), Utc::now())
    }

    pub fn joined(policy: Policy) -> Self {
        Self::joined_with(policy, |bot| bot)
    }

    pub fn joined_with(policy: Policy, build: impl FnOnce(Bot) -> Bot) -> Self {
        let mut world = Self::new_with(policy, build);
        let sid = world.sid();
        let hello = world.bot.announcements(sid, Utc::now().timestamp_millis());
        world.deliver(hello);
        let snapshot = world.snapshot_from(world.owner);
        let out = world.bot.on_snapshot(sid, snapshot, Instant::now(), Utc::now()).expect("owner snapshot");
        world.deliver(out);
        world
    }

    pub fn promote(&mut self, permissions: Permissions) {
        let role = Role {
            id: RoleId::new(),
            name: "bots".into(),
            color: None,
            permissions,
            position: 1,
            hoist: false,
            self_assignable: false,
            tag: None,
            banner: None,
        };
        let id = role.id;
        let sid = self.sid();
        self.by_owner(ServerEvent::RoleCreate { server: sid, role });
        self.by_owner(ServerEvent::RoleAssign { server: sid, user: self.bot_id, role: id });
    }

    pub fn block(&mut self, term: &str) {
        let rules = AutoMod { blocked_terms: vec![term.into()], ..AutoMod::default() };
        let sid = self.sid();
        self.by_owner(ServerEvent::AutoModSet { server: sid, rules });
    }
}

pub fn ticket(state: &ServerState, owner: UserId) -> ServerTicket {
    let mut ticket = ServerTicket::new(state.server.id, &state.server.name, [7; 32], vec![]);
    ticket.owner = Some((owner, identity_for(owner).public()));
    ticket
}

pub fn deletes(out: &[WirePayload]) -> Vec<commy_core::MessageId> {
    out.iter()
        .filter_map(|p| match p {
            WirePayload::Event(ServerEvent::MessageDelete { message, .. }) => Some(*message),
            _ => None,
        })
        .collect()
}

pub fn notes(out: &[WirePayload]) -> Vec<String> {
    out.iter()
        .filter_map(|p| match p {
            WirePayload::Event(ServerEvent::MessageCreate { message }) => Some(message.content.clone()),
            _ => None,
        })
        .collect()
}

