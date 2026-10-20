use chrono::Utc;
use commy_bot::bridge::{self, Bridge, Tap};
use commy_bot::storage::DataDir;
use commy_bot::webhook::{Webhook, Webhooks};
use commy_bot::{Act, ActError, BotEvent, BotStore, Ctx, Outgoing, Policy};
use commy_core::test_support::post;
use commy_core::{Permissions, ServerEvent, UserId};
use commy_net::WirePayload;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::UnboundedReceiver;

mod common;
use common::World;

fn pong(ctx: &Ctx, event: &BotEvent) -> Vec<Act> {
    match event {
        BotEvent::MessageCreate(m) if m.content == "!ping" && !ctx.is_late(m) => {
            vec![Act::Send { channel: m.channel, content: format!("pong, {}", ctx.name_of(m.author)), reply_to: Some(m.id) }]
        }
        BotEvent::MemberJoin(user) => {
            let general = ctx.state.server.channels[0].id;
            vec![Act::Send { channel: general, content: format!("welcome {}", ctx.name_of(*user)), reply_to: None }]
        }
        _ => Vec::new(),
    }
}

fn sent(out: &[WirePayload]) -> Vec<commy_core::Message> {
    out.iter()
        .filter_map(|p| match p {
            WirePayload::Event(ServerEvent::MessageCreate { message }) => Some(message.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_handler_answers_commands_and_greets_newcomers() {
    let mut world = World::joined_with(Policy::default(), |bot| bot.with(pong));
    let ping = post(&world.owner_state, world.channel, world.member, "!ping");
    let out = world.posted(ping.clone());
    let replies = sent(&out);
    assert_eq!(replies.len(), 1);
    assert_eq!((replies[0].content.as_str(), replies[0].reply_to), ("pong, member", Some(ping.id)));
    world.deliver(out);
    assert!(world.owner_state.has_message(replies[0].id), "members accept the bot's signed reply");

    let sid = world.sid();
    let state = world.bot.replica(sid).unwrap().state.as_ref().unwrap();
    let later = Ctx { server: sid, state, me: world.bot_id, now: Utc::now() + chrono::Duration::minutes(10) };
    assert!(later.is_late(&ping) && pong(&later, &BotEvent::MessageCreate(ping.clone())).is_empty(), "no answers to stale commands");

    let newcomer = commy_core::test_support::keyed_profile("newcomer");
    let out = world.by(newcomer.id, ServerEvent::MemberJoin { server: sid, profile: newcomer.clone() });
    assert_eq!(sent(&out).iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), vec!["welcome newcomer"]);
}

#[test]
fn reactions_and_departures_reach_handlers() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let mut world = World::joined_with(Policy::default(), move |bot| {
        bot.with(move |_: &Ctx, event: &BotEvent| {
            log.lock().unwrap().push(event.clone());
            Vec::new()
        })
    });
    let hello = post(&world.owner_state, world.channel, world.member, "hello");
    world.posted(hello.clone());
    let sid = world.sid();
    let react = |active| ServerEvent::ReactionSet { channel: world.channel, message: hello.id, emoji: "👍".into(), user: world.member, active };
    let (on, off) = (react(true), react(false));
    world.by(world.member, on);
    world.by(world.member, off);
    let member = world.member;
    world.by(world.owner, ServerEvent::MemberLeave { server: sid, user: member });
    let seen = seen.lock().unwrap();
    let reactions: Vec<_> = seen.iter().filter(|e| matches!(e, BotEvent::ReactionAdd { .. } | BotEvent::ReactionRemove { .. })).collect();
    assert_eq!(reactions.len(), 2);
    assert!(matches!(reactions[0], BotEvent::ReactionAdd { emoji, user, .. } if emoji == "👍" && *user == member));
    assert!(matches!(reactions[1], BotEvent::ReactionRemove { .. }));
    assert_eq!(seen.last(), Some(&BotEvent::MemberLeave(member)));
    assert!(seen.iter().any(|e| matches!(e, BotEvent::MessageCreate(m) if m.id == hello.id)));
}

#[test]
fn actions_are_checked_against_the_bots_roles() {
    let mut world = World::joined(Policy::default());
    let sid = world.sid();
    let now = Utc::now();
    let theirs = post(&world.owner_state, world.channel, world.member, "hi");
    world.posted(theirs.clone());

    let remove = world.bot.perform(sid, Act::Remove { message: theirs.id }, now);
    assert_eq!(remove, Err(ActError::Missing("Manage Messages")));
    assert_eq!(world.bot.perform(sid, Act::Ban { user: world.member }, now), Err(ActError::Missing("Ban Members")));
    assert_eq!(world.bot.perform(sid, Act::Kick { user: world.owner }, now), Err(ActError::Owner));
    assert_eq!(world.bot.perform(sid, Act::Kick { user: world.bot_id }, now), Err(ActError::Itself));
    assert_eq!(world.bot.perform(sid, Act::Edit { message: theirs.id, content: "x".into() }, now), Err(ActError::NotOwn));
    assert_eq!(world.bot.perform(sid, Act::Send { channel: world.channel, content: "  ".into(), reply_to: None }, now), Err(ActError::Empty));
    let nowhere = commy_core::ChannelId::new();
    assert_eq!(world.bot.perform(sid, Act::Send { channel: nowhere, content: "hi".into(), reply_to: None }, now), Err(ActError::UnknownChannel));
    assert_eq!(world.bot.perform(sid, Act::React { message: theirs.id, emoji: "nope".into(), active: true }, now), Err(ActError::BadEmoji));

    let react = world.bot.perform(sid, Act::React { message: theirs.id, emoji: "🎉".into(), active: true }, now).unwrap();
    let own = world.bot.perform(sid, Act::Send { channel: world.channel, content: "first".into(), reply_to: None }, now).unwrap();
    let own_id = sent(std::slice::from_ref(&own))[0].id;
    let edit = world.bot.perform(sid, Act::Edit { message: own_id, content: "second".into() }, now).unwrap();
    world.deliver(vec![react, own, edit]);
    assert_eq!(world.owner_state.message(own_id).unwrap().content, "second");
    assert!(world.owner_state.message(theirs.id).unwrap().reactions.iter().any(|r| r.emoji == "🎉" && r.users.contains(&world.bot_id)));

    world.promote(Permissions::MANAGE_MESSAGES | Permissions::KICK_MEMBERS | Permissions::MANAGE_ROLES | Permissions::SEND_MESSAGES);
    let role = world.owner_state.server.roles.iter().find(|r| r.name == "bots").unwrap().id;
    let give = world.bot.perform(sid, Act::AddRole { user: world.member, role }, Utc::now()).unwrap();
    let removed = world.bot.perform(sid, Act::Remove { message: theirs.id }, Utc::now()).unwrap();
    let kick = world.bot.perform(sid, Act::Kick { user: world.member }, Utc::now()).unwrap();
    world.deliver(vec![give, removed, kick]);
    assert!(!world.owner_state.has_message(theirs.id));
    assert!(!world.owner_state.server.is_current_member(world.member));
}

#[test]
fn auto_moderation_applies_to_what_the_bot_sends() {
    let mut world = World::joined(Policy::default());
    world.block("gold");
    let sid = world.sid();
    let held = world.bot.perform(sid, Act::Send { channel: world.channel, content: "buy gold".into(), reply_to: None }, Utc::now());
    assert_eq!(held, Err(ActError::Held("blocked word".into())));
}

fn drain(outgoing: &mut UnboundedReceiver<Outgoing>) -> Vec<WirePayload> {
    let mut sent = Vec::new();
    while let Ok(item) = outgoing.try_recv() {
        if let Outgoing::Send(_, payload) = item {
            sent.push(*payload);
        }
    }
    sent
}

async fn next(reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>) -> Value {
    let mut line = String::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), reader.read_line(&mut line)).await.unwrap().unwrap();
    serde_json::from_str(&line).unwrap()
}

async fn write(stream: &mut tokio::net::tcp::OwnedWriteHalf, value: Value) {
    stream.write_all(format!("{value}\n").as_bytes()).await.unwrap();
}

#[tokio::test]
async fn bridge_clients_need_the_token_then_get_events_and_act() {
    let (taps, _) = tokio::sync::broadcast::channel(16);
    let tap = taps.clone();
    let world = World::joined_with(Policy::default(), move |bot| bot.with(Tap::new(tap)));
    let World { bot, member, channel, owner_state, bot_id, .. } = world;
    let sid = owner_state.server.id;
    let (out, mut outgoing) = tokio::sync::mpsc::unbounded_channel();
    let store = Arc::new(BotStore::new(Arc::new(Mutex::new(bot)), out));
    let listener = TcpListener::bind(bridge::loopback(0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(bridge::listen(listener, Arc::new(Bridge::new(store.clone(), "secret".into(), taps))));

    let (read, mut send) = TcpStream::connect(bridge::loopback(port)).await.unwrap().into_split();
    let mut read = BufReader::new(read);
    write(&mut send, json!({"op": "hello", "token": "guess"})).await;
    assert_eq!(next(&mut read).await["type"], "denied");

    let (read, mut send) = TcpStream::connect(bridge::loopback(port)).await.unwrap().into_split();
    let mut read = BufReader::new(read);
    write(&mut send, json!({"op": "hello", "token": "secret"})).await;
    let hello = next(&mut read).await;
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["me"]["id"], bot_id.to_string());
    let server = &hello["servers"][0];
    assert_eq!((server["id"].as_str(), server["ready"].as_bool()), (Some(sid.to_string().as_str()), Some(true)));
    assert!(server["members"].as_array().unwrap().iter().any(|m| m["id"] == member.to_string() && m["bot"] == false));

    let message = post(&owner_state, channel, member, &format!("hey <@{bot_id}> @Tidy"));
    let delivered = store.bot().on_event(sid, ServerEvent::MessageCreate { message: message.clone() }, None, Instant::now(), Utc::now());
    assert!(delivered.is_empty());
    let event = next(&mut read).await;
    assert_eq!(event["type"], "message_create");
    assert_eq!(event["message"]["author"]["name"], "member");
    assert_eq!(event["message"]["mentions"], json!([bot_id.to_string()]));
    assert_eq!(event["message"]["late"], false);

    let channel_name = owner_state.server.channel(channel).unwrap().name.clone();
    write(&mut send, json!({
        "op": "send", "nonce": 7, "server": owner_state.server.name, "channel": format!("#{channel_name}"),
        "content": format!("hi <@{member}>"), "embeds": [{"title": "Status", "fields": [{"name": "Build", "value": "green"}]}],
        "reply_to": message.id.to_string(),
    })).await;
    let result = next(&mut read).await;
    assert_eq!((result["type"].as_str(), result["nonce"].as_i64(), result["ok"].as_bool()), (Some("result"), Some(7), Some(true)));
    assert_eq!(result["message"]["content"], "hi @member\n\nStatus\nBuild: green");
    let sent = drain(&mut outgoing);
    assert_eq!(sent.len(), 1);

    write(&mut send, json!({"op": "ban", "nonce": "b", "server": sid.to_string(), "user": member.to_string()})).await;
    let result = next(&mut read).await;
    assert_eq!((result["nonce"].as_str(), result["ok"].as_bool()), (Some("b"), Some(false)));
    assert_eq!(result["error"], "the bot needs a role with Ban Members");
    write(&mut send, json!({"op": "fly", "nonce": 3})).await;
    let result = next(&mut read).await;
    assert_eq!((result["nonce"].as_i64(), result["ok"].as_bool()), (Some(3), Some(false)));
    assert!(drain(&mut outgoing).is_empty());
}

#[tokio::test]
async fn webhooks_post_into_their_channel_until_removed() {
    let world = World::joined(Policy::default());
    let World { bot, channel, owner_state, .. } = world;
    let sid = owner_state.server.id;
    let dir = std::env::temp_dir().join(format!("commy-bot-hooks-{}", UserId::new()));
    let data = DataDir::new(&dir);
    let hook = data.add_webhook(Webhook::new("Builds\u{7}", sid, channel)).unwrap();
    assert_eq!(hook.name, "Builds");
    let url = std::fs::read_to_string(data.webhook_url_file(&hook.id)).unwrap();
    assert!(url.trim().ends_with(&hook.path()));
    let (out, mut outgoing) = tokio::sync::mpsc::unbounded_channel();
    let store = Arc::new(BotStore::new(Arc::new(Mutex::new(bot)), out));
    let hooks = Arc::new(Webhooks::new(store.clone(), DataDir::new(&dir)));

    let body = json!({"content": "deploy done", "embeds": [{"title": "v1.2", "url": "https://example.com/r"}]}).to_string();
    assert_eq!(hooks.post(&hook.id, "wrong", body.as_bytes()).0.as_u16(), 404);
    assert_eq!(hooks.post(&hook.id, &hook.secret, b"not json").0.as_u16(), 400);
    assert_eq!(hooks.post(&hook.id, &hook.secret, br#"{"content": " "}"#).0.as_u16(), 422);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(commy_bot::webhook::listen(listener, hooks.clone()));
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        hook.path(),
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 204"), "{response}");
    let posted = sent(&drain(&mut outgoing));
    assert_eq!(posted.len(), 1);
    assert_eq!((posted[0].channel, posted[0].content.as_str()), (channel, "deploy done\n\nv1.2\nhttps://example.com/r"));

    for _ in 2..commy_bot::webhook::PER_MINUTE {
        assert_eq!(hooks.post(&hook.id, &hook.secret, body.as_bytes()).0.as_u16(), 204);
    }
    assert_eq!(hooks.post(&hook.id, &hook.secret, body.as_bytes()).0.as_u16(), 429, "refused posts count too");

    assert!(data.remove_webhook(&hook.id).unwrap());
    assert!(!data.webhook_url_file(&hook.id).exists());
    assert_eq!(hooks.post(&hook.id, &hook.secret, body.as_bytes()).0.as_u16(), 404);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_bridge_token_is_kept_until_reset() {
    let dir = std::env::temp_dir().join(format!("commy-bot-token-{}", UserId::new()));
    let data = DataDir::new(&dir);
    let token = data.bridge_token(false).unwrap();
    assert_eq!(token.len(), 64);
    assert_eq!(data.bridge_token(false).unwrap(), token);
    assert_ne!(data.bridge_token(true).unwrap(), token);
    std::fs::remove_dir_all(dir).unwrap();
}
