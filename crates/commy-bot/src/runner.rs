use crate::bridge::{self, Bridge, Tap};
use crate::moderator::{can_ban, can_remove};
use crate::storage::DataDir;
use crate::webhook::{self, Webhooks};
use crate::{Bot, BotStore, Moderator, Outgoing, Policy, Replica};
use anyhow::{bail, Context, Result};
use chrono::Utc;
use commy_core::{Permissions, Presence, ServerEvent, ServerId, ServerState, UserId, UserProfile};
use commy_net::{CommyNode, NetEvent, WirePayload};
use std::collections::BTreeSet;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const BEAT: Duration = Duration::from_secs(45);
pub const ASK_AGAIN: Duration = Duration::from_secs(30);
pub const SAVE_EVERY: Duration = Duration::from_secs(30);
pub const HIGHER_NODE_WAIT: Duration = Duration::from_secs(15);
pub const RECENT_SESSION: Duration = Duration::from_secs(60);
const FAREWELL_LINGER: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Setup {
    pub policy: Policy,
    pub bridge: Option<u16>,
    pub webhooks: Option<SocketAddr>,
}

impl From<Policy> for Setup {
    fn from(policy: Policy) -> Self {
        Self { policy, ..Self::default() }
    }
}

pub fn require_profile(data: &DataDir) -> Result<UserProfile> {
    data.profile()?.context("no bot in this folder yet; start with: commy-bot init NAME")
}

pub fn status_line(state: &ServerState, me: Option<UserId>) -> String {
    let Some(me) = me.filter(|me| state.server.is_current_member(*me)) else {
        return "not a member (removed or banned)".into();
    };
    match (can_remove(state, me), state.server.permissions_for(me).allows(Permissions::BAN_MEMBERS)) {
        (false, _) => "joined; give it a role with Manage Messages so it can remove held messages".into(),
        (true, false) => "removes held messages".into(),
        (true, true) => "removes held messages and can ban".into(),
    }
}

pub async fn run(data: &DataDir, setup: Setup, relays: Option<iroh::RelayMap>, stop: impl Future<Output = ()>) -> Result<()> {
    let profile = require_profile(data)?;
    let identity = data.identity()?;
    if profile.identity != Some(identity.public()) {
        bail!("identity.key in {} does not match this bot's profile; restore it from a backup", data.root().display());
    }
    let secret = data.node_secret()?;
    let servers = data.servers()?;
    if servers.is_empty() {
        bail!("no servers yet; add one with: commy-bot join INVITE");
    }
    let mut bot = Bot::new(identity, profile, Moderator::new(setup.policy));
    let (taps, _) = tokio::sync::broadcast::channel(bridge::EVENT_BACKLOG);
    if setup.bridge.is_some() {
        bot = bot.with(Tap::new(taps.clone()));
    }
    for (ticket, state) in servers {
        bot.add(Replica::new(ticket, state));
    }

    let spawned = match relays {
        Some(map) => CommyNode::spawn_with(Some(secret), Some(map)).await,
        None => CommyNode::spawn(Some(secret)).await,
    };
    let (mut node, mut events, mut media, mut dms) = spawned?;
    tokio::spawn(async move { while media.recv().await.is_some() {} });
    tokio::spawn(async move { while dms.recv().await.is_some() {} });
    let tickets: Vec<_> = bot.replicas().map(|r| r.ticket.clone()).collect();
    let (out, mut outgoing) = tokio::sync::mpsc::unbounded_channel();
    let store = Arc::new(BotStore::new(Arc::new(Mutex::new(bot)), out));
    node.set_catchup_store(store.clone());
    if let Some(port) = setup.bridge {
        let token = data.bridge_token(false)?;
        let listener = tokio::net::TcpListener::bind(bridge::loopback(port))
            .await
            .with_context(|| format!("could not open the bridge on 127.0.0.1:{port}"))?;
        tracing::info!("bridge listening on 127.0.0.1:{port}; the token is in {}", data.root().join(crate::storage::TOKEN_FILE).display());
        tokio::spawn(bridge::listen(listener, Arc::new(Bridge::new(store.clone(), token, taps.clone()))));
    }
    if let Some(addr) = setup.webhooks {
        let listener = tokio::net::TcpListener::bind(addr).await.with_context(|| format!("could not open webhooks on {addr}"))?;
        if !addr.ip().is_loopback() {
            tracing::warn!("webhooks listen on {addr}, reachable beyond this machine; anyone with a webhook URL can post");
        }
        tracing::info!("webhooks listening on http://{addr}");
        tokio::spawn(webhook::listen(listener, Arc::new(Webhooks::new(store.clone(), DataDir::new(data.root())))));
    }
    for ticket in &tickets {
        node.rejoin_server(ticket).await.with_context(|| format!("could not join {}", ticket.server_name))?;
        tracing::info!("connecting to {}", ticket.server_name);
    }

    let mut beat = tokio::time::interval(BEAT);
    let mut ask = tokio::time::interval(ASK_AGAIN);
    let mut saving = tokio::time::interval(SAVE_EVERY);
    let mut dirty = BTreeSet::new();
    tokio::pin!(stop);
    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { break };
                if let Some(server) = handle(&store, &node, event).await {
                    dirty.insert(server);
                }
            }
            Some(item) = outgoing.recv() => match item {
                Outgoing::Send(server, payload) => send(&node, server, *payload).await,
                Outgoing::Changed(server) => {
                    dirty.insert(server);
                }
            },
            _ = beat.tick() => {
                let beats: Vec<_> = {
                    let mut bot = store.bot();
                    bot.tidy(Utc::now());
                    joined(&bot, &node).into_iter().filter_map(|s| Some((s, bot.presence(s, Presence::Online, now_ms())?))).collect()
                };
                for (server, payload) in beats {
                    send(&node, server, payload).await;
                }
            }
            _ = ask.tick() => {
                let waiting: Vec<ServerId> = store.bot().replicas().filter(|r| r.state.is_none()).map(|r| r.server()).collect();
                for server in waiting.into_iter().filter(|s| node.neighbor_count(*s) > 0) {
                    send(&node, server, WirePayload::SyncRequest).await;
                }
            }
            _ = saving.tick() => {
                for server in std::mem::take(&mut dirty) {
                    save(data, &store.bot(), server);
                }
            }
            _ = &mut stop => break,
        }
    }
    let farewells: Vec<_> = {
        let mut bot = store.bot();
        joined(&bot, &node).into_iter().filter_map(|s| Some((s, bot.presence(s, Presence::Offline, now_ms())?))).collect()
    };
    for (server, payload) in farewells {
        send(&node, server, payload).await;
    }
    tokio::time::sleep(FAREWELL_LINGER).await;
    {
        let bot = store.bot();
        for server in bot.replicas().map(|r| r.server()).collect::<Vec<_>>() {
            save(data, &bot, server);
        }
    }
    node.shutdown().await
}

async fn handle(store: &Arc<BotStore>, node: &CommyNode, event: NetEvent) -> Option<ServerId> {
    match event {
        NetEvent::Event { server, event, action, .. } => {
            let fleeting = matches!(event, ServerEvent::PresenceUpdate { .. } | ServerEvent::TypingStart { .. });
            let out = store.bot().on_event(server, event, action, Instant::now(), Utc::now());
            for payload in out {
                send(node, server, payload).await;
            }
            (!fleeting).then_some(server)
        }
        NetEvent::SyncReceived { server, payload } => {
            let out = {
                let mut bot = store.bot();
                let out = bot.on_snapshot(server, payload, Instant::now(), Utc::now())?;
                if let Some(state) = bot.replica(server).and_then(|r| r.state.as_ref()) {
                    tracing::info!("joined {}: {}", state.server.name, status_line(state, Some(bot.profile().id)));
                    warn_missing_permissions(&bot, state);
                }
                out
            };
            for payload in out {
                send(node, server, payload).await;
            }
            for peer in node.neighbors(server) {
                catch_up(store, node, server, peer);
            }
            Some(server)
        }
        NetEvent::NeighborUp { server, peer } => {
            let out = store.bot().announcements(server, now_ms());
            for payload in out {
                send(node, server, payload).await;
            }
            catch_up(store, node, server, peer);
            None
        }
        NetEvent::CaughtUp { server, peer, report } => {
            store.finish(server, peer);
            log_session(server, &report);
            None
        }
        _ => None,
    }
}

fn catch_up(store: &Arc<BotStore>, node: &CommyNode, server: ServerId, peer: [u8; 32]) {
    if store.bot().replica(server).and_then(|r| r.state.as_ref()).is_none() {
        return;
    }
    let wait = (node.node_id_bytes() > peer).then_some(HIGHER_NODE_WAIT);
    let session = node.catch_up(server, peer, store.clone());
    let store = store.clone();
    tokio::spawn(async move {
        if let Some(wait) = wait {
            tokio::time::sleep(wait).await;
        }
        if !store.begin(server, peer, wait.map(|_| RECENT_SESSION)) {
            return;
        }
        let result = session.await;
        store.finish(server, peer);
        match result {
            Ok(report) => log_session(server, &report),
            Err(e) => tracing::debug!("catch-up with a peer ended: {e:#}"),
        }
    });
}

fn log_session(server: ServerId, report: &commy_net::catchup::SessionReport) {
    if !report.received.is_empty() || report.held > 0 {
        tracing::info!("caught up on {server}: received {}, held {}", report.received.len(), report.held);
    }
}

fn warn_missing_permissions(bot: &Bot, state: &ServerState) {
    let me = bot.profile().id;
    let policy = bot.moderator().policy();
    if policy.ban_after.is_some() && !can_ban(state, me, UserId::new()) {
        tracing::warn!("{}: --ban-after needs a role with Ban Members", state.server.name);
    }
    if let Some(name) = policy.log_channel.as_deref().filter(|_| bot.moderator().log_channel(state, me).is_none()) {
        tracing::warn!("{}: no text channel named {name} that the bot can post in", state.server.name);
    }
}

fn joined(bot: &Bot, node: &CommyNode) -> Vec<ServerId> {
    bot.replicas().map(|r| r.server()).filter(|s| node.neighbor_count(*s) > 0).collect()
}

async fn send(node: &CommyNode, server: ServerId, payload: WirePayload) {
    if let Err(e) = node.broadcast(server, payload).await {
        tracing::debug!("broadcast failed: {e:#}");
    }
}

fn save(data: &DataDir, bot: &Bot, server: ServerId) {
    let Some(replica) = bot.replica(server) else { return };
    if let Err(e) = data.save_server(&replica.ticket, replica.state.as_ref()) {
        tracing::warn!("{e:#}");
    }
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}
