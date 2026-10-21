use crate::compat::{self, Embed};
use crate::handler::{Act, BotEvent, Ctx, Handler};
use crate::{Bot, BotStore};
use chrono::Utc;
use commy_core::{BotCommand, ChannelKind, Message, MessageId, ServerId, ServerState, UserId};
use commy_net::WirePayload;
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinSet;

pub const PROTOCOL_VERSION: u32 = 1;
pub const DEFAULT_PORT: u16 = 47620;
pub const MAX_LINE_BYTES: u64 = 64 * 1024;
pub const MAX_CONNECTIONS: usize = 8;
pub const HELLO_WAIT: Duration = Duration::from_secs(5);
pub const EVENT_BACKLOG: usize = 1024;
const ACCEPT_RETRY: Duration = Duration::from_millis(250);

pub fn loopback(port: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, port))
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Hello {
        token: String,
    },
    Send {
        #[serde(default)]
        nonce: Value,
        server: String,
        channel: String,
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        embeds: Vec<Embed>,
        #[serde(default)]
        reply_to: Option<String>,
    },
    Edit {
        #[serde(default)]
        nonce: Value,
        server: String,
        message: String,
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        embeds: Vec<Embed>,
    },
    Delete {
        #[serde(default)]
        nonce: Value,
        server: String,
        message: String,
    },
    React {
        #[serde(default)]
        nonce: Value,
        server: String,
        message: String,
        emoji: String,
    },
    Unreact {
        #[serde(default)]
        nonce: Value,
        server: String,
        message: String,
        emoji: String,
    },
    Kick {
        #[serde(default)]
        nonce: Value,
        server: String,
        user: String,
    },
    Ban {
        #[serde(default)]
        nonce: Value,
        server: String,
        user: String,
    },
    AddRole {
        #[serde(default)]
        nonce: Value,
        server: String,
        user: String,
        role: String,
    },
    RemoveRole {
        #[serde(default)]
        nonce: Value,
        server: String,
        user: String,
        role: String,
    },
    Server {
        #[serde(default)]
        nonce: Value,
        server: String,
    },
    SetCommands {
        #[serde(default)]
        nonce: Value,
        server: String,
        #[serde(default)]
        commands: Vec<BotCommand>,
    },
}

impl Request {
    fn nonce(&self) -> Value {
        match self {
            Request::Hello { .. } => Value::Null,
            Request::Send { nonce, .. }
            | Request::Edit { nonce, .. }
            | Request::Delete { nonce, .. }
            | Request::React { nonce, .. }
            | Request::Unreact { nonce, .. }
            | Request::Kick { nonce, .. }
            | Request::Ban { nonce, .. }
            | Request::AddRole { nonce, .. }
            | Request::RemoveRole { nonce, .. }
            | Request::Server { nonce, .. }
            | Request::SetCommands { nonce, .. } => nonce.clone(),
        }
    }
}

pub struct Tap {
    events: broadcast::Sender<Arc<str>>,
}

impl Tap {
    pub fn new(events: broadcast::Sender<Arc<str>>) -> Self {
        Self { events }
    }
}

impl Handler for Tap {
    fn handle(&mut self, ctx: &Ctx, event: &BotEvent) -> Vec<Act> {
        if self.events.receiver_count() > 0 {
            let _ = self.events.send(line(&event_json(ctx, event)));
        }
        Vec::new()
    }
}

pub fn event_json(ctx: &Ctx, event: &BotEvent) -> Value {
    let server = ctx.server.to_string();
    match event {
        BotEvent::Ready => json!({"type": "server_ready", "server": server_json(ctx.state, true)}),
        BotEvent::MessageCreate(m) => json!({"type": "message_create", "server": server, "message": message_json(ctx, m)}),
        BotEvent::MessageUpdate(m) => json!({"type": "message_update", "server": server, "message": message_json(ctx, m)}),
        BotEvent::MessageDelete { channel, message } => {
            json!({"type": "message_delete", "server": server, "channel": channel.to_string(), "message": message.to_string()})
        }
        BotEvent::ReactionAdd { channel, message, user, emoji } | BotEvent::ReactionRemove { channel, message, user, emoji } => {
            let kind = if matches!(event, BotEvent::ReactionAdd { .. }) { "reaction_add" } else { "reaction_remove" };
            json!({
                "type": kind,
                "server": server,
                "channel": channel.to_string(),
                "message": message.to_string(),
                "user": member_json(ctx.state, *user),
                "emoji": emoji,
            })
        }
        BotEvent::MemberJoin(user) => json!({"type": "member_join", "server": server, "member": member_json(ctx.state, *user)}),
        BotEvent::MemberLeave(user) => json!({"type": "member_leave", "server": server, "user": user.to_string()}),
        BotEvent::CommandInvoked(invoked) => json!({
            "type": "command_invoked",
            "server": server,
            "command": invoked.command,
            "options": invoked.options,
            "message": message_json(ctx, &invoked.message),
        }),
    }
}

pub fn message_json(ctx: &Ctx, m: &Message) -> Value {
    let channel = ctx.state.server.channel(m.channel);
    json!({
        "id": m.id.to_string(),
        "server": ctx.server.to_string(),
        "channel": m.channel.to_string(),
        "channel_name": channel.map(|c| c.name.as_str()),
        "author": member_json(ctx.state, m.author),
        "content": m.content,
        "created_at": m.created_at.to_rfc3339(),
        "edited_at": m.edited_at.map(|t| t.to_rfc3339()),
        "reply_to": m.reply_to.map(|id| id.to_string()),
        "attachments": m.attachments.iter().map(|a| json!({"filename": a.filename, "size": a.size_bytes, "mime_type": a.mime_type})).collect::<Vec<_>>(),
        "reactions": m.reactions.iter().map(|r| json!({"emoji": r.emoji, "count": r.users.len(), "me": r.users.contains(&ctx.me)})).collect::<Vec<_>>(),
        "mentions": compat::mentions(&m.content, ctx.state).iter().map(UserId::to_string).collect::<Vec<_>>(),
        "late": ctx.is_late(m),
    })
}

pub fn member_json(state: &ServerState, user: UserId) -> Value {
    let profile = state.profiles.get(&user);
    let member = state.server.member(user);
    json!({
        "id": user.to_string(),
        "name": profile.map_or("someone", |p| p.visible_name()),
        "username": profile.map(|p| p.username.as_str()),
        "bot": state.is_bot(user),
        "owner": state.server.owner == user,
        "roles": member.map(|m| m.roles.iter().map(|r| r.to_string()).collect::<Vec<_>>()).unwrap_or_default(),
    })
}

pub fn server_json(state: &ServerState, with_members: bool) -> Value {
    let server = &state.server;
    let kind = |k: ChannelKind| match k {
        ChannelKind::Text => "text",
        ChannelKind::Voice => "voice",
        ChannelKind::Announcement => "announcement",
        ChannelKind::Category => "category",
        ChannelKind::Forum => "forum",
        ChannelKind::Stage => "stage",
    };
    let mut value = json!({
        "id": server.id.to_string(),
        "name": server.name,
        "ready": true,
        "owner": server.owner.to_string(),
        "channels": server.channels.iter().map(|c| json!({"id": c.id.to_string(), "name": c.name, "kind": kind(c.kind)})).collect::<Vec<_>>(),
        "roles": server.roles.iter().map(|r| json!({"id": r.id.to_string(), "name": r.name, "position": r.position})).collect::<Vec<_>>(),
    });
    if with_members {
        let members: Vec<Value> =
            server.members.iter().filter(|m| server.is_current_member(m.user)).map(|m| member_json(state, m.user)).collect();
        value["members"] = Value::Array(members);
    }
    value
}

pub fn hello_json(bot: &Bot) -> Value {
    let servers: Vec<Value> = bot
        .replicas()
        .map(|r| match &r.state {
            Some(state) => server_json(state, true),
            None => json!({"id": r.server().to_string(), "name": r.name(), "ready": false}),
        })
        .collect();
    let me = bot.profile();
    json!({
        "type": "hello",
        "version": PROTOCOL_VERSION,
        "me": {"id": me.id.to_string(), "name": me.visible_name()},
        "servers": servers,
    })
}

fn line(value: &Value) -> Arc<str> {
    let mut text = value.to_string();
    text.push('\n');
    text.into()
}

fn ok(nonce: Value, extra: Value) -> Value {
    let mut value = json!({"type": "result", "nonce": nonce, "ok": true});
    if let (Value::Object(target), Value::Object(more)) = (&mut value, extra) {
        target.extend(more);
    }
    value
}

fn failed(nonce: Value, error: impl ToString) -> Value {
    json!({"type": "result", "nonce": nonce, "ok": false, "error": error.to_string()})
}

pub fn answer(store: &BotStore, request: Request) -> Value {
    let nonce = request.nonce();
    let mut bot = store.bot();
    let (server, act) = match resolve(&bot, request) {
        Ok(Resolved::Act(server, act)) => (server, act),
        Ok(Resolved::Info(value)) => return ok(nonce, json!({"server": value})),
        Err(e) => return failed(nonce, e),
    };
    let payload = match bot.perform(server, act, Utc::now()) {
        Ok(payload) => payload,
        Err(e) => return failed(nonce, e),
    };
    let extra = match &payload {
        WirePayload::Event(commy_core::ServerEvent::MessageCreate { message }) => bot
            .replica(server)
            .and_then(|r| r.state.as_ref())
            .map(|state| {
                let me = bot.profile().id;
                let ctx = Ctx { server, state, me, now: Utc::now() };
                let opened = bot.readable(state, message).unwrap_or_else(|| message.clone());
                json!({"message": message_json(&ctx, &opened)})
            })
            .unwrap_or(Value::Null),
        _ => Value::Null,
    };
    drop(bot);
    store.publish(server, payload);
    ok(nonce, extra)
}

enum Resolved {
    Act(ServerId, Act),
    Info(Value),
}

fn resolve(bot: &Bot, request: Request) -> Result<Resolved, String> {
    let find = |text: &str| -> Result<(ServerId, &ServerState), String> {
        let text = text.trim();
        let replica = bot
            .replicas()
            .find(|r| r.server().to_string() == text)
            .or_else(|| bot.replicas().find(|r| r.name().eq_ignore_ascii_case(text)))
            .ok_or("the bot is not in that server")?;
        let state = replica.state.as_ref().ok_or("the bot has no copy of that server yet")?;
        Ok((replica.server(), state))
    };
    let message = |text: &str| text.trim().parse::<MessageId>().map_err(|_| "that is not a message id".to_string());
    let user = |state: &ServerState, text: &str| compat::member(state, text).ok_or_else(|| "no such member".to_string());
    let role = |state: &ServerState, text: &str| compat::role(state, text).ok_or_else(|| "no such role".to_string());
    let act = match request {
        Request::Hello { .. } => return Err("already signed in".into()),
        Request::Server { server, .. } => return Ok(Resolved::Info(server_json(find(&server)?.1, true))),
        Request::Send { server, channel, content, embeds, reply_to, .. } => {
            let (id, state) = find(&server)?;
            let channel = compat::channel(state, &channel).ok_or("no such text channel")?;
            let content = compat::compose(content.as_deref(), &embeds, state);
            let reply_to = reply_to.as_deref().map(message).transpose()?;
            (id, Act::Send { channel, content, reply_to })
        }
        Request::Edit { server, message: target, content, embeds, .. } => {
            let (id, state) = find(&server)?;
            let content = compat::compose(content.as_deref(), &embeds, state);
            (id, Act::Edit { message: message(&target)?, content })
        }
        Request::Delete { server, message: target, .. } => (find(&server)?.0, Act::Remove { message: message(&target)? }),
        Request::React { server, message: target, emoji, .. } => {
            (find(&server)?.0, Act::React { message: message(&target)?, emoji, active: true })
        }
        Request::Unreact { server, message: target, emoji, .. } => {
            (find(&server)?.0, Act::React { message: message(&target)?, emoji, active: false })
        }
        Request::Kick { server, user: who, .. } => {
            let (id, state) = find(&server)?;
            (id, Act::Kick { user: user(state, &who)? })
        }
        Request::Ban { server, user: who, .. } => {
            let (id, state) = find(&server)?;
            (id, Act::Ban { user: user(state, &who)? })
        }
        Request::AddRole { server, user: who, role: which, .. } => {
            let (id, state) = find(&server)?;
            (id, Act::AddRole { user: user(state, &who)?, role: role(state, &which)? })
        }
        Request::RemoveRole { server, user: who, role: which, .. } => {
            let (id, state) = find(&server)?;
            (id, Act::RemoveRole { user: user(state, &who)?, role: role(state, &which)? })
        }
        Request::SetCommands { server, commands, .. } => (find(&server)?.0, Act::SetCommands { commands }),
    };
    Ok(Resolved::Act(act.0, act.1))
}

pub struct Bridge {
    store: Arc<BotStore>,
    token: String,
    events: broadcast::Sender<Arc<str>>,
}

impl Bridge {
    pub fn new(store: Arc<BotStore>, token: String, events: broadcast::Sender<Arc<str>>) -> Self {
        Self { store, token, events }
    }

    fn admits(&self, token: &str) -> bool {
        let (a, b) = (self.token.as_bytes(), token.trim().as_bytes());
        a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }
}

pub async fn listen(listener: TcpListener, bridge: Arc<Bridge>) {
    let mut connections = JoinSet::new();
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                tracing::debug!("bridge accept failed: {e}");
                tokio::time::sleep(ACCEPT_RETRY).await;
                continue;
            }
        };
        while connections.try_join_next().is_some() {}
        if !peer.ip().is_loopback() || connections.len() >= MAX_CONNECTIONS {
            continue;
        }
        let _ = stream.set_nodelay(true);
        connections.spawn(serve(stream, bridge.clone()));
    }
}

async fn serve(stream: TcpStream, bridge: Arc<Bridge>) {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let hello = tokio::time::timeout(HELLO_WAIT, read_line(&mut reader)).await.ok().flatten();
    let token = match hello.as_deref().map(serde_json::from_str::<Request>) {
        Some(Ok(Request::Hello { token })) => token,
        _ => {
            let _ = write.write_all(line(&json!({"type": "denied", "error": "send hello with the token first"})).as_bytes()).await;
            return;
        }
    };
    if !bridge.admits(&token) {
        let _ = write.write_all(line(&json!({"type": "denied", "error": "wrong token"})).as_bytes()).await;
        return;
    }
    let mut events = bridge.events.subscribe();
    let hello = line(&hello_json(&bridge.store.bot()));
    if write.write_all(hello.as_bytes()).await.is_err() {
        return;
    }
    let (lines_tx, mut lines) = mpsc::channel::<String>(16);
    let reading = tokio::spawn(async move {
        while let Some(text) = read_line(&mut reader).await {
            if lines_tx.send(text).await.is_err() {
                break;
            }
        }
    });
    loop {
        let out = tokio::select! {
            text = lines.recv() => match text {
                Some(text) => line(&match serde_json::from_str::<Request>(&text) {
                    Ok(request) => answer(&bridge.store, request),
                    Err(e) => failed(nonce_of(&text), format!("could not read that request: {e}")),
                }),
                None => break,
            },
            event = events.recv() => match event {
                Ok(text) => text,
                Err(broadcast::error::RecvError::Lagged(missed)) => line(&json!({"type": "lagged", "missed": missed})),
                Err(broadcast::error::RecvError::Closed) => break,
            },
        };
        if write.write_all(out.as_bytes()).await.is_err() {
            break;
        }
    }
    reading.abort();
}

fn nonce_of(text: &str) -> Value {
    serde_json::from_str::<Value>(text).ok().and_then(|v| v.get("nonce").cloned()).unwrap_or(Value::Null)
}

async fn read_line<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> Option<String> {
    let mut text = String::new();
    let read = reader.take(MAX_LINE_BYTES).read_line(&mut text).await.ok()?;
    (read > 0 && text.ends_with('\n')).then_some(text)
}
