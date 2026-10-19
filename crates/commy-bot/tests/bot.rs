use chrono::{Duration, Utc};
use commy_bot::storage::{ticket_in, DataDir};
use commy_bot::{Bot, Moderator, Policy, Replica};
use commy_core::test_support::{fixture, identity_for, join, post, post_at};
use commy_core::{
    AutoMod, ChannelId, Identity, Message, Permissions, Role, RoleId, ServerEvent, ServerId, ServerState, SignedEvent,
    UserId, UserProfile,
};
use commy_net::protocol::{build_sync_response, sign_sync_response};
use commy_net::{ServerTicket, WirePayload};
use std::time::Instant;

struct World {
    owner_state: ServerState,
    owner: UserId,
    channel: ChannelId,
    member: UserId,
    bot: Bot,
    bot_id: UserId,
    clock: i64,
}

impl World {
    fn new(policy: Policy) -> Self {
        let (mut owner_state, channel, owner) = fixture();
        let member = join(&mut owner_state, "member");
        let identity = Identity::generate();
        let profile = UserProfile::new("Tidy");
        let bot_id = profile.id;
        let mut bot = Bot::new(identity, profile, Moderator::new(policy));
        bot.add(Replica::new(ticket(&owner_state, owner), None));
        Self { owner_state, owner, channel, member, bot, bot_id, clock: Utc::now().timestamp_millis() - 60_000 }
    }

    fn sid(&self) -> ServerId {
        self.owner_state.server.id
    }

    fn snapshot_from(&self, by: UserId) -> WirePayload {
        let mut payload = build_sync_response(&self.owner_state, 50, by);
        sign_sync_response(&mut payload, Utc::now().timestamp_millis(), &identity_for(by));
        payload
    }

    fn to_owner(&mut self, payloads: Vec<WirePayload>) {
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

    fn by_owner(&mut self, event: ServerEvent) -> Vec<WirePayload> {
        self.clock += 1;
        let signed = SignedEvent::sign(self.sid(), self.owner, self.clock, event, &identity_for(self.owner));
        self.owner_state.apply_signed(&signed).unwrap();
        let sid = self.sid();
        self.bot.on_event(sid, signed.event, Some(signed.action), Instant::now(), Utc::now())
    }

    fn posted(&mut self, message: Message) -> Vec<WirePayload> {
        self.owner_state.apply_unsigned(ServerEvent::MessageCreate { message: message.clone() }).unwrap();
        let sid = self.sid();
        self.bot.on_event(sid, ServerEvent::MessageCreate { message }, None, Instant::now(), Utc::now())
    }

    fn joined(policy: Policy) -> Self {
        let mut world = Self::new(policy);
        let sid = world.sid();
        let hello = world.bot.announcements(sid, Utc::now().timestamp_millis());
        world.to_owner(hello);
        let snapshot = world.snapshot_from(world.owner);
        let out = world.bot.on_snapshot(sid, snapshot, Utc::now().timestamp_millis()).expect("owner snapshot");
        world.to_owner(out);
        world
    }

    fn promote(&mut self, permissions: Permissions) {
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

    fn block(&mut self, term: &str) {
        let rules = AutoMod { blocked_terms: vec![term.into()], ..AutoMod::default() };
        let sid = self.sid();
        self.by_owner(ServerEvent::AutoModSet { server: sid, rules });
    }
}

fn ticket(state: &ServerState, owner: UserId) -> ServerTicket {
    let mut ticket = ServerTicket::new(state.server.id, &state.server.name, [7; 32], vec![]);
    ticket.owner = Some((owner, identity_for(owner).public()));
    ticket
}

fn deletes(out: &[WirePayload]) -> Vec<commy_core::MessageId> {
    out.iter()
        .filter_map(|p| match p {
            WirePayload::Event(ServerEvent::MessageDelete { message, .. }) => Some(*message),
            _ => None,
        })
        .collect()
}

fn notes(out: &[WirePayload]) -> Vec<String> {
    out.iter()
        .filter_map(|p| match p {
            WirePayload::Event(ServerEvent::MessageCreate { message }) => Some(message.content.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_bot_joins_only_from_the_owners_snapshot_and_wears_the_badge() {
    let mut world = World::new(Policy::default());
    let sid = world.sid();
    let hello = world.bot.announcements(sid, Utc::now().timestamp_millis());
    assert!(hello.contains(&WirePayload::SyncRequest), "asks for a snapshot until it has one");
    world.to_owner(hello);
    assert!(world.owner_state.server.is_member(world.bot_id));
    assert!(world.owner_state.is_bot(world.bot_id), "members see the badge");

    let forged = world.snapshot_from(world.member);
    assert_eq!(world.bot.on_snapshot(sid, forged, Utc::now().timestamp_millis()), None);
    let fresh = world.snapshot_from(world.owner);
    let much_later = Utc::now().timestamp_millis() + 3_600_000;
    assert_eq!(world.bot.on_snapshot(sid, fresh.clone(), much_later), None, "stale snapshots are refused");
    let out = world.bot.on_snapshot(sid, fresh.clone(), Utc::now().timestamp_millis()).expect("the owner's snapshot");
    assert!(!out.contains(&WirePayload::SyncRequest));
    assert_eq!(world.bot.on_snapshot(sid, fresh, Utc::now().timestamp_millis()), None, "only the first one");

    let state = world.bot.replica(sid).unwrap().state.as_ref().unwrap();
    assert!(state.server.is_member(world.member) && state.server.is_member(world.bot_id));
    assert!(state.is_bot(world.bot_id));
    assert_eq!(state.identity_of(world.member), world.owner_state.identity_of(world.member));
}

#[test]
fn held_messages_are_removed_only_once_the_bot_may_remove_them() {
    let mut world = World::joined(Policy { log_channel: Some("#general".into()), ban_after: None });
    world.block("gold");
    let early = post(&world.owner_state, world.channel, world.member, "cheap gold");
    assert!(world.posted(early.clone()).is_empty(), "no Manage Messages yet, so it only watches");

    world.promote(Permissions::MANAGE_MESSAGES | Permissions::SEND_MESSAGES);
    let spam = post(&world.owner_state, world.channel, world.member, "more gold here");
    let out = world.posted(spam.clone());
    assert_eq!(deletes(&out), vec![spam.id]);
    assert_eq!(notes(&out), vec!["Removed a message from member: blocked word".to_string()]);
    world.to_owner(out);
    assert!(!world.owner_state.has_message(spam.id), "gone for everyone");
    assert!(world.owner_state.has_message(early.id), "it does not sweep older messages");
    let note = world.owner_state.messages(world.channel).last().unwrap();
    assert_eq!((note.author, note.content.as_str()), (world.bot_id, "Removed a message from member: blocked word"));

    let fine = post(&world.owner_state, world.channel, world.member, "hello there");
    assert!(world.posted(fine).is_empty());
    let from_owner = post(&world.owner_state, world.channel, world.owner, "gold is exempt for the owner");
    assert!(world.posted(from_owner).is_empty());
    let replayed = post_at(&world.owner_state, world.channel, world.member, "old gold", Utc::now() - Duration::hours(1));
    assert!(world.posted(replayed).is_empty(), "old messages arriving late are left to moderators");

    world.bot.tidy(Utc::now() + Duration::hours(commy_bot::KEEP_HOURS + 1));
    let kept = world.bot.replica(world.sid()).unwrap().state.as_ref().unwrap();
    assert!(kept.messages(world.channel).is_empty(), "only the last day stays in memory");
}

#[test]
fn repeat_offenders_are_banned_when_the_bot_may_ban() {
    let mut world = World::joined(Policy { log_channel: None, ban_after: Some(2) });
    world.block("gold");
    world.promote(Permissions::MANAGE_MESSAGES | Permissions::BAN_MEMBERS);
    let first = post(&world.owner_state, world.channel, world.member, "gold");
    let out = world.posted(first);
    assert!(out.iter().all(|p| !matches!(p, WirePayload::Signed(s) if matches!(s.event, ServerEvent::MemberBan { .. }))));
    world.to_owner(out);
    let second = post(&world.owner_state, world.channel, world.member, "gold again");
    let out = world.posted(second);
    assert!(out.iter().any(|p| matches!(p, WirePayload::Signed(s) if s.event == ServerEvent::MemberBan { server: world.owner_state.server.id, user: world.member })));
    world.to_owner(out);
    assert!(world.owner_state.server.banned.contains(&world.member));
}

#[test]
fn messages_from_members_it_has_not_met_yet_are_reviewed_once_their_key_arrives() {
    let mut world = World::joined(Policy::default());
    world.block("gold");
    world.promote(Permissions::MANAGE_MESSAGES);
    let newcomer = commy_core::test_support::keyed_profile("newcomer");
    let message = post(&world.owner_state, world.channel, newcomer.id, "gold for all");
    let sid = world.sid();
    let early = world.bot.on_event(sid, ServerEvent::MessageCreate { message: message.clone() }, None, Instant::now(), Utc::now());
    assert!(early.is_empty());
    assert_eq!(world.bot.replica(sid).unwrap().pending(), 1);
    let at = Utc::now().timestamp_millis();
    let hello = SignedEvent::sign(sid, newcomer.id, at, ServerEvent::MemberJoin { server: sid, profile: newcomer.clone() }, &identity_for(newcomer.id));
    let out = world.bot.on_event(sid, hello.event, Some(hello.action), Instant::now(), Utc::now());
    assert_eq!(deletes(&out), vec![message.id]);
    assert_eq!(world.bot.replica(sid).unwrap().pending(), 0);
}

#[test]
fn the_data_folder_keeps_the_bot_and_its_servers() {
    let dir = std::env::temp_dir().join(format!("commy-bot-test-{}", UserId::new()));
    let data = DataDir::new(&dir);
    assert!(data.create_profile("bad/name").is_err());
    let profile = data.create_profile("Tidy").unwrap();
    assert!(data.create_profile("Again").is_err(), "one bot per folder");
    assert_eq!(data.profile().unwrap().unwrap().id, profile.id);
    assert_eq!(data.identity().unwrap().public(), profile.identity.unwrap());
    assert_eq!(data.node_secret().unwrap(), data.node_secret().unwrap());

    let (mut state, _, owner) = fixture();
    state.server.name = "Rust Hangout".into();
    let ticket = ticket(&state, owner);
    let text = format!("Come join me in Rust Hangout! {}", ticket.encode());
    assert_eq!(ticket_in(&text).unwrap(), ticket);
    assert!(ticket_in("https://commy.chat/i/ABCDEFGH").is_err());
    let mut ownerless = ticket.clone();
    ownerless.owner = None;
    assert!(ticket_in(&ownerless.encode()).is_err());

    data.save_server(&ticket, None).unwrap();
    assert!(data.has_server(ticket.server_id));
    let saved = data.servers().unwrap();
    assert_eq!(saved.len(), 1);
    assert!(saved[0].1.is_none());
    data.save_server(&ticket, Some(&state)).unwrap();
    let saved = data.servers().unwrap();
    assert_eq!(saved[0].1.as_ref().unwrap().server.name, "Rust Hangout");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rules_relayed_before_the_snapshot_still_apply_once_it_arrives() {
    let mut world = World::new(Policy::default());
    let sid = world.sid();
    let hello = world.bot.announcements(sid, Utc::now().timestamp_millis());
    world.to_owner(hello);
    world.block("gold");
    world.promote(Permissions::MANAGE_MESSAGES);
    assert_eq!(world.bot.replica(sid).unwrap().pending(), 3, "kept until the bot has a copy of the server");
    let snapshot = world.snapshot_from(world.owner);
    world.bot.on_snapshot(sid, snapshot, Utc::now().timestamp_millis()).unwrap();
    assert_eq!(world.bot.replica(sid).unwrap().pending(), 0);
    let spam = post(&world.owner_state, world.channel, world.member, "gold");
    assert_eq!(deletes(&world.posted(spam.clone())), vec![spam.id]);
}

#[test]
fn edits_into_spam_are_removed_and_held_edits_replay_in_order() {
    let mut world = World::joined(Policy::default());
    world.block("gold");
    world.promote(Permissions::MANAGE_MESSAGES);
    let hello = post(&world.owner_state, world.channel, world.member, "hello");
    assert!(world.posted(hello.clone()).is_empty());
    let edit = commy_core::test_support::edit(&world.owner_state, &hello, "now with gold", Utc::now());
    let sid = world.sid();
    let out = world.bot.on_event(sid, edit, None, Instant::now(), Utc::now());
    assert_eq!(deletes(&out), vec![hello.id]);

    let newcomer = commy_core::test_support::keyed_profile("newcomer");
    let first = post(&world.owner_state, world.channel, newcomer.id, "cheap gold");
    let fixed = commy_core::test_support::edit(&world.owner_state, &first, "never mind", Utc::now());
    for event in [ServerEvent::MessageCreate { message: first.clone() }, fixed] {
        assert!(world.bot.on_event(sid, event, None, Instant::now(), Utc::now()).is_empty());
    }
    let at = Utc::now().timestamp_millis();
    let join = SignedEvent::sign(sid, newcomer.id, at, ServerEvent::MemberJoin { server: sid, profile: newcomer.clone() }, &identity_for(newcomer.id));
    let out = world.bot.on_event(sid, join.event, Some(join.action), Instant::now(), Utc::now());
    assert!(deletes(&out).is_empty(), "the edit lands after the create, so the fixed text is what gets judged");
    let kept = world.bot.replica(sid).unwrap().state.as_ref().unwrap();
    assert_eq!(kept.message(first.id).unwrap().content, "never mind");
}
