use crate::moderator::{can_ban, can_remove};
use crate::storage::DataDir;
use crate::{Bot, Moderator, Policy, Replica};
use anyhow::{bail, Context, Result};
use chrono::Utc;
use commy_core::{Permissions, Presence, ServerEvent, ServerId, ServerState, UserId, UserProfile};
use commy_net::{CommyNode, NetEvent, WirePayload};
use std::collections::BTreeSet;
use std::future::Future;
use std::time::{Duration, Instant};

pub const BEAT: Duration = Duration::from_secs(45);
pub const ASK_AGAIN: Duration = Duration::from_secs(30);
pub const SAVE_EVERY: Duration = Duration::from_secs(30);
const FAREWELL_LINGER: Duration = Duration::from_millis(300);

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

pub async fn run(data: &DataDir, policy: Policy, relays: Option<iroh::RelayMap>, stop: impl Future<Output = ()>) -> Result<()> {
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
    let mut bot = Bot::new(identity, profile, Moderator::new(policy));
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
                if let Some(server) = handle(&mut bot, &node, event).await {
                    dirty.insert(server);
                }
            }
            _ = beat.tick() => {
                bot.tidy(Utc::now());
                for server in joined(&bot, &node) {
                    if let Some(payload) = bot.presence(server, Presence::Online, now_ms()) {
                        send(&node, server, payload).await;
                    }
                }
            }
            _ = ask.tick() => {
                let waiting: Vec<ServerId> = bot.replicas().filter(|r| r.state.is_none()).map(|r| r.server()).collect();
                for server in waiting.into_iter().filter(|s| node.neighbor_count(*s) > 0) {
                    send(&node, server, WirePayload::SyncRequest).await;
                }
            }
            _ = saving.tick() => {
                for server in std::mem::take(&mut dirty) {
                    save(data, &bot, server);
                }
            }
            _ = &mut stop => break,
        }
    }
    for server in joined(&bot, &node) {
        if let Some(payload) = bot.presence(server, Presence::Offline, now_ms()) {
            send(&node, server, payload).await;
        }
    }
    tokio::time::sleep(FAREWELL_LINGER).await;
    for server in bot.replicas().map(|r| r.server()).collect::<Vec<_>>() {
        save(data, &bot, server);
    }
    node.shutdown().await
}

async fn handle(bot: &mut Bot, node: &CommyNode, event: NetEvent) -> Option<ServerId> {
    match event {
        NetEvent::Event { server, event, action, .. } => {
            let fleeting = matches!(event, ServerEvent::PresenceUpdate { .. } | ServerEvent::TypingStart { .. });
            for payload in bot.on_event(server, event, action, Instant::now(), Utc::now()) {
                send(node, server, payload).await;
            }
            (!fleeting).then_some(server)
        }
        NetEvent::SyncReceived { server, payload } => {
            let out = bot.on_snapshot(server, payload, now_ms())?;
            if let Some(state) = bot.replica(server).and_then(|r| r.state.as_ref()) {
                tracing::info!("joined {}: {}", state.server.name, status_line(state, Some(bot.profile().id)));
                warn_missing_permissions(bot, state);
            }
            for payload in out {
                send(node, server, payload).await;
            }
            Some(server)
        }
        NetEvent::NeighborUp { server, .. } => {
            for payload in bot.announcements(server, now_ms()) {
                send(node, server, payload).await;
            }
            None
        }
        _ => None,
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
