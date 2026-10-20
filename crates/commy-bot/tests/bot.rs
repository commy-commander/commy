use chrono::{Duration, Utc};
use commy_bot::storage::{ticket_in, DataDir};
use commy_bot::{Bot, BotStore, Moderator, Outgoing, Policy, Replica};
use commy_core::test_support::{fixture, identity_for, join, post, post_at};
use commy_core::{
    AutoMod, ChannelId, Identity, Message, Permissions, Role, RoleId, ServerEvent, ServerId, ServerState, SignedEvent,
    UserId, UserProfile,
};
use commy_net::catchup::CatchupStore;
use commy_net::protocol::{build_sync_response, sign_sync_response};
use commy_net::{ServerTicket, WirePayload};
use std::sync::{Arc, Mutex};
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

    fn deliver(&mut self, payloads: Vec<WirePayload>) {
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
        world.deliver(hello);
        let snapshot = world.snapshot_from(world.owner);
        let out = world.bot.on_snapshot(sid, snapshot, Instant::now(), Utc::now()).expect("owner snapshot");
        world.deliver(out);
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
    world.deliver(hello);
    assert!(world.owner_state.server.is_member(world.bot_id));
    assert!(world.owner_state.is_bot(world.bot_id), "members see the badge");

    let forged = world.snapshot_from(world.member);
    assert_eq!(world.bot.on_snapshot(sid, forged, Instant::now(), Utc::now()), None);
    let fresh = world.snapshot_from(world.owner);
    assert_eq!(world.bot.on_snapshot(sid, fresh.clone(), Instant::now(), Utc::now() + Duration::hours(1)), None, "stale snapshots are refused");
    let out = world.bot.on_snapshot(sid, fresh.clone(), Instant::now(), Utc::now()).expect("the owner's snapshot");
    assert!(!out.contains(&WirePayload::SyncRequest));
    assert_eq!(world.bot.on_snapshot(sid, fresh, Instant::now(), Utc::now()), None, "only the first one");

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
    world.deliver(out);
    assert!(!world.owner_state.has_message(spam.id), "gone for everyone");
    assert!(world.owner_state.has_message(early.id), "it does not sweep older messages");
    let note = world.owner_state.messages(world.channel).last().unwrap();
    assert_eq!((note.author, note.content.as_str()), (world.bot_id, "Removed a message from member: blocked word"));

    let fine = post(&world.owner_state, world.channel, world.member, "hello there");
    assert!(world.posted(fine).is_empty());
    let from_owner = post(&world.owner_state, world.channel, world.owner, "gold is exempt for the owner");
    assert!(world.posted(from_owner).is_empty());
    let replayed = post_at(&world.owner_state, world.channel, world.member, "old gold", Utc::now() - Duration::hours(1));
    assert!(world.posted(replayed).is_empty(), "messages from before the bot joined are left to moderators");

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
    world.deliver(out);
    let second = post(&world.owner_state, world.channel, world.member, "gold again");
    let out = world.posted(second);
    assert!(out.iter().any(|p| matches!(p, WirePayload::Signed(s) if s.event == ServerEvent::MemberBan { server: world.owner_state.server.id, user: world.member })));
    world.deliver(out);
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
    world.deliver(hello);
    world.block("gold");
    world.promote(Permissions::MANAGE_MESSAGES);
    assert_eq!(world.bot.replica(sid).unwrap().pending(), 3, "kept until the bot has a copy of the server");
    let snapshot = world.snapshot_from(world.owner);
    world.bot.on_snapshot(sid, snapshot, Instant::now(), Utc::now()).unwrap();
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

fn is_ban(payload: &WirePayload, user: UserId) -> bool {
    matches!(payload, WirePayload::Signed(s) if matches!(s.event, ServerEvent::MemberBan { user: banned, .. } if banned == user))
}

fn drain(outgoing: &mut tokio::sync::mpsc::UnboundedReceiver<Outgoing>) -> (Vec<WirePayload>, bool) {
    let mut sent = Vec::new();
    let mut changed = false;
    while let Ok(item) = outgoing.try_recv() {
        match item {
            Outgoing::Send(_, payload) => sent.push(*payload),
            Outgoing::Changed(_) => changed = true,
        }
    }
    (sent, changed)
}

#[test]
fn messages_learned_through_catch_up_are_removed_and_count_toward_bans() {
    let mut world = World::joined(Policy { log_channel: Some("general".into()), ban_after: Some(2) });
    world.block("gold");
    world.promote(Permissions::MANAGE_MESSAGES | Permissions::BAN_MEMBERS | Permissions::SEND_MESSAGES);
    let sid = world.sid();
    let older = post_at(&world.owner_state, world.channel, world.member, "old gold", Utc::now() - Duration::hours(1));
    let first = post(&world.owner_state, world.channel, world.member, "gold");
    let second = post(&world.owner_state, world.channel, world.member, "more gold");
    let World { bot, member, .. } = world;
    let (out, mut outgoing) = tokio::sync::mpsc::unbounded_channel();
    let store = BotStore::new(Arc::new(Mutex::new(bot)), out);

    let report = store.apply(sid, vec![older.clone(), first.clone()], vec![]);
    assert_eq!(report.applied, vec![older.id, first.id]);
    let (sent, changed) = drain(&mut outgoing);
    assert!(changed);
    assert_eq!(deletes(&sent), vec![first.id], "only messages written since the bot joined");
    assert!(!sent.iter().any(|p| is_ban(p, member)));
    assert_eq!(notes(&sent), vec!["Removed a message from member: blocked word".to_string()]);

    let report = store.apply(sid, vec![first.clone(), second.clone()], vec![]);
    assert_eq!(report.applied, vec![second.id], "a removed message stays removed");
    let (sent, _) = drain(&mut outgoing);
    assert_eq!(deletes(&sent), vec![second.id]);
    assert!(sent.iter().any(|p| is_ban(p, member)), "both removals count toward --ban-after");

    let bot = store.bot();
    let state = bot.replica(sid).unwrap().state.as_ref().unwrap();
    assert!(state.server.banned.contains(&member));
    assert!(!state.has_message(first.id) && !state.has_message(second.id));
    assert!(state.has_message(older.id));
}

#[test]
fn caught_up_messages_from_unknown_keys_are_reviewed_once_the_profile_arrives() {
    let mut world = World::joined(Policy::default());
    world.block("gold");
    world.promote(Permissions::MANAGE_MESSAGES);
    let sid = world.sid();
    let newcomer = commy_core::test_support::keyed_profile("newcomer");
    let message = post(&world.owner_state, world.channel, newcomer.id, "gold for all");
    let World { bot, .. } = world;
    let (out, mut outgoing) = tokio::sync::mpsc::unbounded_channel();
    let store = BotStore::new(Arc::new(Mutex::new(bot)), out);

    let report = store.apply(sid, vec![message.clone()], vec![]);
    assert_eq!(report.held, vec![message.id]);
    assert!(deletes(&drain(&mut outgoing).0).is_empty());
    store.apply_profiles(sid, vec![newcomer]);
    assert_eq!(deletes(&drain(&mut outgoing).0), vec![message.id]);
    assert_eq!(store.bot().replica(sid).unwrap().pending(), 0);
}

#[test]
fn blocked_messages_in_the_first_snapshot_are_removed_if_written_after_the_bot_joined() {
    let mut world = World::new(Policy::default());
    let sid = world.sid();
    let hello = world.bot.announcements(sid, Utc::now().timestamp_millis());
    world.deliver(hello);
    world.block("gold");
    world.promote(Permissions::MANAGE_MESSAGES);
    let before = post_at(&world.owner_state, world.channel, world.member, "gold before", Utc::now() - Duration::hours(1));
    world.owner_state.apply_unsigned(ServerEvent::MessageCreate { message: before.clone() }).unwrap();
    let spam = post(&world.owner_state, world.channel, world.member, "gold while it waited");
    world.owner_state.apply_unsigned(ServerEvent::MessageCreate { message: spam.clone() }).unwrap();
    let snapshot = world.snapshot_from(world.owner);
    let out = world.bot.on_snapshot(sid, snapshot, Instant::now(), Utc::now()).unwrap();
    assert_eq!(deletes(&out), vec![spam.id]);
    world.deliver(out);
    assert!(!world.owner_state.has_message(spam.id));
    assert!(world.owner_state.has_message(before.id));
}

#[test]
fn private_channels_are_reviewed_through_the_bots_own_key() {
    use commy_core::channel_key::{self, KeyTask, Keyring};
    let mut world = World::joined(Policy { log_channel: Some("#general".into()), ban_after: None });
    world.block("gold");
    world.promote(Permissions::MANAGE_MESSAGES | Permissions::SEND_MESSAGES);
    let sid = world.sid();
    let (channel, member) = (world.channel, world.member);
    let role = Role {
        id: RoleId::new(),
        name: "crew".into(),
        color: None,
        permissions: Permissions::NONE,
        position: 2,
        hoist: false,
        self_assignable: false,
        tag: None,
        banner: None,
    };
    let crew = role.id;
    world.by_owner(ServerEvent::RoleCreate { server: sid, role });
    for user in [member, world.bot_id] {
        world.by_owner(ServerEvent::RoleAssign { server: sid, user, role: crew });
    }
    world.by_owner(ServerEvent::ChannelAccessSet { server: sid, channel, view: vec![crew], send: vec![] });

    let mut ring = Keyring::unlock(&world.owner_state, channel, world.owner, &identity_for(world.owner));
    let Some(KeyTask::Rotate { links }) = channel_key::key_task(&world.owner_state, channel, world.owner, &ring) else {
        panic!("a fresh private channel needs its first key");
    };
    let record = ring.rotate(sid, channel, &links, Utc::now().timestamp_millis());
    let key = record.id;
    world.by_owner(ServerEvent::ChannelKeyAdd { server: sid, channel, key: record });
    let readers = world.owner_state.server.readers_of(channel);
    assert!(readers.contains(&world.bot_id) && readers.contains(&member));
    for grant in ring.grant_events(&world.owner_state, channel, key, &readers) {
        world.by_owner(grant);
    }

    let member_ring = Keyring::unlock(&world.owner_state, channel, member, &identity_for(member));
    let (key, secret) = member_ring.sending_key(&world.owner_state, channel).expect("the member holds the key");
    let mut spam = Message::new(channel, member, "sealed gold inside");
    channel_key::seal_message(sid, &mut spam, key, secret);
    spam.sign(sid, &identity_for(member));
    let out = world.posted(spam.clone());
    assert_eq!(deletes(&out), vec![spam.id]);
    let written = notes(&out);
    assert_eq!(written.len(), 1);
    assert!(channel_key::is_sealed(&written[0]), "the note in a private channel is sealed too");
    world.deliver(out);
    let note = world.owner_state.messages(channel).last().unwrap().clone();
    let opened = channel_key::read_message(sid, &note, &member_ring).expect("readers open the note");
    assert_eq!(opened.content, "Removed a message from member: blocked word");
}
