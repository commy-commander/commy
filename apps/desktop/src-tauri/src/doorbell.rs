use crate::catchup::AppAccess;
use crate::state::App;
use commy_core::doorbell::{self as bell, DoorbellKey};
use commy_core::{MessageId, ServerEvent, ServerId, ServerState, UserId};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

pub const DOORBELL_FILE: &str = "doorbell.json";
const RING_DEBOUNCE: Duration = Duration::from_secs(2);
const DOORBELL_SUPPORT_TTL: Duration = Duration::from_secs(60 * 60);
const SERVER_MAX_TAKE_WAIT_SECS: u64 = 25;
const HELLO_EVERY: Duration = Duration::from_secs(30 * 60);
const NUDGE_EVERY: Duration = Duration::from_secs(60);
const NUDGE_SLOW: Duration = Duration::from_secs(10 * 60);
const HELLO_AFTER_START: Duration = Duration::from_secs(30);
const LISTEN_CHECK_EVERY: Duration = Duration::from_secs(5);
const RING_SPACING: Duration = Duration::from_secs(30);
const RINGS_PER_MINUTE: usize = 250;
const SERVER_RINGS_PER_MINUTE_LIMIT: usize = 300;
const _: () = assert!(RINGS_PER_MINUTE < SERVER_RINGS_PER_MINUTE_LIMIT);
const YESTERDAY_TOKEN_GRACE_SECS: i64 = 60 * 60;
const MAX_PEERS_PER_USER: usize = 8;
const AWAIT_SECS: i64 = bell::KEEP_DAYS as i64 * 86_400;
const MAX_AWAITED: usize = 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Peer {
    pub user: UserId,
    pub url: String,
    pub key: DoorbellKey,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    own: HashMap<ServerId, DoorbellKey>,
    #[serde(default)]
    peers: HashMap<ServerId, HashMap<String, Peer>>,
    #[serde(default)]
    awaited: HashMap<ServerId, HashMap<String, i64>>,
    #[serde(default)]
    last_take: HashMap<ServerId, u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Endpoints {
    pub(crate) ring: String,
    pub(crate) take: String,
}

#[derive(Debug, Default)]
pub struct Doorbell {
    saved: Saved,
    ringing: HashMap<(String, String), Vec<String>>,
    origins: HashMap<String, (Instant, Option<Endpoints>)>,
    rung_at: HashMap<(String, String), Instant>,
    budget: HashMap<String, (Instant, usize)>,
    hello_at: HashMap<ServerId, Instant>,
    nudged_at: HashMap<ServerId, Instant>,
    listening: HashSet<ServerId>,
}

impl Doorbell {
    pub fn load(dir: &Path) -> Self {
        let saved = crate::persist::read_json(&dir.join(DOORBELL_FILE)).unwrap_or_default();
        Self { saved, ..Default::default() }
    }

    fn save(&self, dir: &Path) -> anyhow::Result<()> {
        crate::persist::write_json(&dir.join(DOORBELL_FILE), &self.saved)
    }

    pub fn retain_servers(&mut self, keep: impl Fn(&ServerId) -> bool) -> bool {
        let before = self.saved.own.len() + self.saved.peers.len() + self.saved.awaited.len() + self.saved.last_take.len();
        self.saved.own.retain(|s, _| keep(s));
        self.saved.peers.retain(|s, _| keep(s));
        self.saved.awaited.retain(|s, _| keep(s));
        self.saved.last_take.retain(|s, _| keep(s));
        before != self.saved.own.len() + self.saved.peers.len() + self.saved.awaited.len() + self.saved.last_take.len()
    }

    pub(crate) fn own_key(&mut self, server: ServerId) -> (DoorbellKey, bool) {
        if let Some(key) = self.saved.own.get(&server) {
            return (*key, false);
        }
        let key: DoorbellKey = commy_net::node::CommyNode::generate_secret();
        self.saved.own.insert(server, key);
        (key, true)
    }

    fn set_peer(&mut self, server: ServerId, user: UserId, node: &str, peer: Option<Peer>) -> bool {
        let peers = self.saved.peers.entry(server).or_default();
        if peers.get(node).is_some_and(|p| p.user != user) {
            return false;
        }
        match peer {
            Some(peer) => {
                if !peers.contains_key(node) {
                    let theirs: Vec<String> = peers.iter().filter(|(_, p)| p.user == user).map(|(n, _)| n.clone()).collect();
                    if theirs.len() >= MAX_PEERS_PER_USER {
                        if let Some(drop) = theirs.iter().min() {
                            peers.remove(drop);
                        }
                    }
                }
                peers.insert(node.to_string(), peer.clone()) != Some(peer)
            }
            None => peers.remove(node).is_some(),
        }
    }

    fn retain_members(&mut self, server: ServerId, keep: impl Fn(UserId) -> bool) -> bool {
        let Some(peers) = self.saved.peers.get_mut(&server) else { return false };
        let before = peers.len();
        peers.retain(|_, p| keep(p.user));
        before != peers.len()
    }

    fn spend(&mut self, origin: &str, now: Instant) -> Option<Duration> {
        let minute = Duration::from_secs(60);
        let (start, used) = self.budget.entry(origin.to_string()).or_insert((now, 0));
        if now.duration_since(*start) >= minute {
            (*start, *used) = (now, 0);
        }
        if *used >= RINGS_PER_MINUTE {
            return Some(minute - now.duration_since(*start));
        }
        *used += 1;
        None
    }

    fn newest_awaited(&self, server: ServerId) -> Option<i64> {
        self.saved.awaited.get(&server)?.values().copied().max()
    }

    fn add_awaited(&mut self, server: ServerId, ids: Vec<String>, held: &HashSet<String>, now: i64) -> bool {
        let awaited = self.saved.awaited.entry(server).or_default();
        let mut changed = false;
        for id in ids {
            if held.contains(&id) || awaited.contains_key(&id) {
                continue;
            }
            if awaited.len() >= MAX_AWAITED {
                let oldest = awaited.iter().min_by_key(|(id, at)| (**at, (*id).clone())).map(|(id, _)| id.clone());
                if let Some(oldest) = oldest {
                    awaited.remove(&oldest);
                }
            }
            awaited.insert(id, now);
            changed = true;
        }
        changed
    }

    fn settle_awaited(&mut self, server: ServerId, held: &HashSet<String>, now: i64) -> bool {
        let Some(awaited) = self.saved.awaited.get_mut(&server) else { return false };
        let before = awaited.len();
        awaited.retain(|id, at| !held.contains(id) && now - *at < AWAIT_SECS);
        before != awaited.len()
    }

    pub(crate) fn devices(&self, server: ServerId) -> Vec<(String, UserId)> {
        self.saved.peers.get(&server).map_or_else(Vec::new, |peers| peers.iter().map(|(n, p)| (n.clone(), p.user)).collect())
    }

    pub(crate) fn awaited_count(&self, server: ServerId) -> usize {
        self.saved.awaited.get(&server).map_or(0, HashMap::len)
    }
}

fn peer_origin_ok(url: &str, own_origin: &str) -> bool {
    crate::invite_link::normalize_origin(url).as_deref() == Some(url)
        && (url.starts_with("https://") || url == own_origin)
}

fn on_origin(url: &str, origin: &str) -> bool {
    url.strip_prefix(origin).is_some_and(|rest| rest.starts_with('/'))
}

#[derive(Deserialize, Default)]
struct DiscoveryFeatures {
    #[serde(default)]
    doorbell: bool,
}

#[derive(Deserialize, Default)]
struct DiscoveryEndpoints {
    #[serde(default, rename = "doorbellRing")]
    ring: Option<String>,
    #[serde(default, rename = "doorbellTake")]
    take: Option<String>,
}

#[derive(Deserialize)]
struct Discovery {
    #[serde(default)]
    features: Option<DiscoveryFeatures>,
    #[serde(default)]
    endpoints: Option<DiscoveryEndpoints>,
}

fn endpoints_of(doc: Discovery, origin: &str) -> Option<Endpoints> {
    if !doc.features.is_some_and(|f| f.doorbell) {
        return None;
    }
    let endpoints = doc.endpoints?;
    let (ring, take) = (endpoints.ring?, endpoints.take?);
    (on_origin(&ring, origin) && on_origin(&take, origin)).then_some(Endpoints { ring, take })
}

pub(crate) async fn endpoints(app: &App, origin: &str) -> Option<Endpoints> {
    if let Some((at, found)) = app.doorbell.lock().unwrap().origins.get(origin) {
        if at.elapsed() < DOORBELL_SUPPORT_TTL {
            return found.clone();
        }
    }
    let fetched = async {
        let res = app
            .http
            .get(format!("{origin}/.well-known/commy-auth.json"))
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .ok()?;
        if !res.status().is_success() {
            return None;
        }
        Some(res.json::<Discovery>().await.ok())
    }
    .await;
    let doc = fetched?;
    let found = doc.and_then(|doc| endpoints_of(doc, origin));
    app.doorbell.lock().unwrap().origins.insert(origin.to_string(), (Instant::now(), found.clone()));
    found
}

fn persist(app: &App) {
    let Some(dir) = app.data_dir() else { return };
    let doorbell = app.doorbell.lock().unwrap();
    if let Err(e) = doorbell.save(dir) {
        tracing::warn!("failed to persist doorbell: {e}");
    }
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

fn held_ids(state: &ServerState, key: &DoorbellKey) -> HashSet<String> {
    state
        .server
        .channels
        .iter()
        .flat_map(|c| state.messages(c.id))
        .map(|m| m.id)
        .chain(state.deleted_ids())
        .map(|id| bell::blinded_id(key, id))
        .collect()
}

pub(crate) fn member_ok(state: &ServerState, user: UserId) -> bool {
    state.server.is_member(user) && !state.server.banned.contains(&user)
}

pub(crate) async fn announcement(app: &App, server: ServerId) -> Option<ServerEvent> {
    if !app.key_settled() {
        return None;
    }
    let node = app.node_hex.lock().unwrap().clone()?;
    let user = app.profile.lock().unwrap().as_ref()?.id;
    let identity = app.identity()?;
    let origin = crate::account::effective_server();
    let (url, key) = match endpoints(app, &origin).await {
        Some(_) => {
            let (key, new) = app.doorbell.lock().unwrap().own_key(server);
            if new {
                persist(app);
            }
            (origin, Some(key))
        }
        None => (String::new(), None),
    };
    let sig = identity.sign(&bell::set_signing_bytes(server, user, &node, &url, key.as_ref()));
    Some(ServerEvent::DoorbellSet { server, user, node, url, key, sig })
}

pub fn announce<A: AppAccess + Clone>(access: &A, server: ServerId) {
    let access = access.clone();
    spawn(async move {
        let app = access.app();
        if let Some(event) = announcement(app, server).await {
            let net = app.net.lock().await;
            if let Some(node) = net.as_ref() {
                if let Err(e) = node.broadcast(server, commy_net::WirePayload::Event(event)).await {
                    tracing::debug!("doorbell announcement failed: {e}");
                }
            }
        }
    });
}

pub fn on_announce(app: &App, server: ServerId, event: &ServerEvent) {
    let ServerEvent::DoorbellSet { server: named, user, node, url, key, sig } = event else { return };
    if *named != server || node.len() != 64 || !node.bytes().all(|b| b.is_ascii_hexdigit()) {
        return;
    }
    if app.node_hex.lock().unwrap().as_deref() == Some(node.as_str()) {
        return;
    }
    let signed = {
        let servers = app.servers.lock().unwrap();
        let Some(state) = servers.get(&server) else { return };
        let signer = state.profiles.get(user).and_then(|p| p.identity);
        member_ok(state, *user)
            && signer.is_some_and(|k| bell::verify_set(&k, server, *user, node, url, key.as_ref(), sig))
    };
    if !signed {
        tracing::debug!("dropped a doorbell announcement that does not verify");
        return;
    }
    let own_origin = crate::account::effective_server();
    let peer = key.filter(|_| peer_origin_ok(url, &own_origin)).map(|key| Peer { user: *user, url: url.clone(), key });
    let changed = {
        let servers = app.servers.lock().unwrap();
        let Some(state) = servers.get(&server) else { return };
        let mut doorbell = app.doorbell.lock().unwrap();
        let pruned = doorbell.retain_members(server, |u| member_ok(state, u));
        doorbell.set_peer(server, *user, &node.to_lowercase(), peer) | pruned
    };
    if changed {
        persist(app);
    }
}

pub fn ring<A: AppAccess + Clone>(access: &A, server: ServerId, message: MessageId, everyone: bool) {
    let targets = ring_targets(access.app(), server, message, everyone);
    for (url, token, id) in targets {
        queue_ring(access, url, token, Some(id));
    }
}

pub(crate) fn ring_targets(app: &App, server: ServerId, message: MessageId, everyone: bool) -> Vec<(String, String, String)> {
    let our_node = app.node_hex.lock().unwrap().clone();
    let day = bell::day_of(chrono::Utc::now());
    let servers = app.servers.lock().unwrap();
    let Some(state) = servers.get(&server) else { return Vec::new() };
    let doorbell = app.doorbell.lock().unwrap();
    let Some(peers) = doorbell.saved.peers.get(&server) else { return Vec::new() };
    peers
        .iter()
        .filter(|(node, _)| Some(node.as_str()) != our_node.as_deref())
        .filter(|(_, p)| member_ok(state, p.user))
        .filter(|(_, p)| everyone || !state.profiles.get(&p.user).is_some_and(|u| u.presence.is_visible_online()))
        .map(|(_, p)| (p.url.clone(), bell::token(&p.key, day), bell::blinded_id(&p.key, message)))
        .collect()
}

fn queue_ring<A: AppAccess + Clone>(access: &A, url: String, token: String, id: Option<String>) {
    let app = access.app();
    let key = (url, token);
    let start = {
        let mut doorbell = app.doorbell.lock().unwrap();
        let start = !doorbell.ringing.contains_key(&key);
        let ids = doorbell.ringing.entry(key.clone()).or_default();
        ids.extend(id);
        start
    };
    if !start {
        return;
    }
    let access = access.clone();
    spawn(async move {
        let app = access.app();
        let wait = app.doorbell.lock().unwrap().rung_at.get(&key).map_or(RING_DEBOUNCE, |at| {
            RING_SPACING.saturating_sub(at.elapsed()).max(RING_DEBOUNCE)
        });
        tokio::time::sleep(wait).await;
        let ids = {
            let mut doorbell = app.doorbell.lock().unwrap();
            doorbell.rung_at.retain(|_, at| at.elapsed() < RING_SPACING);
            doorbell.rung_at.insert(key.clone(), Instant::now());
            doorbell.ringing.remove(&key)
        };
        let Some(ids) = ids else { return };
        let (origin, token) = key;
        let Some(endpoints) = endpoints(app, &origin).await else {
            tracing::debug!("a member's account server does not offer the doorbell");
            return;
        };
        let mut chunks: Vec<&[String]> = ids.chunks(bell::MAX_RING_IDS).collect();
        if chunks.is_empty() {
            chunks.push(&[]);
        }
        let mut retried = false;
        let mut chunks = chunks.into_iter().peekable();
        while let Some(chunk) = chunks.peek() {
            let over = app.doorbell.lock().unwrap().spend(&origin, Instant::now());
            if let Some(wait) = over {
                tokio::time::sleep(wait).await;
                continue;
            }
            match send_ring(app, &endpoints.ring, &token, chunk).await {
                Ok(_) => {
                    chunks.next();
                }
                Err(e) if !retried => {
                    tracing::debug!("doorbell ring failed, trying again: {e}");
                    retried = true;
                    tokio::time::sleep(Duration::from_secs(60)).await;
                }
                Err(e) => {
                    tracing::debug!("doorbell ring failed: {e}");
                    return;
                }
            }
        }
    });
}

pub fn ring_device<A: AppAccess + Clone>(access: &A, server: ServerId, node: &str, message: MessageId) {
    let target = {
        let app = access.app();
        let servers = app.servers.lock().unwrap();
        let Some(state) = servers.get(&server) else { return };
        let doorbell = app.doorbell.lock().unwrap();
        let Some(peer) = doorbell.saved.peers.get(&server).and_then(|p| p.get(node)) else { return };
        if !member_ok(state, peer.user) {
            return;
        }
        let day = bell::day_of(chrono::Utc::now());
        (peer.url.clone(), bell::token(&peer.key, day), bell::blinded_id(&peer.key, message))
    };
    let (url, token, id) = target;
    queue_ring(access, url, token, Some(id));
}

pub(crate) async fn send_ring(app: &App, ring: &str, token: &str, ids: &[String]) -> reqwest::Result<()> {
    let body = serde_json::json!({ "token": token, "ids": ids });
    app.http.post(ring).json(&body).timeout(Duration::from_secs(15)).send().await?.error_for_status()?;
    Ok(())
}

fn hello<A: AppAccess + Clone>(access: &A, server: ServerId) {
    let app = access.app();
    let our_node = app.node_hex.lock().unwrap().clone();
    let day = bell::day_of(chrono::Utc::now());
    let targets: Vec<(String, String)> = {
        let servers = app.servers.lock().unwrap();
        let Some(state) = servers.get(&server) else { return };
        let mut doorbell = app.doorbell.lock().unwrap();
        if doorbell.hello_at.get(&server).is_some_and(|at| at.elapsed() < HELLO_EVERY) {
            return;
        }
        doorbell.hello_at.insert(server, Instant::now());
        let Some(peers) = doorbell.saved.peers.get(&server) else { return };
        peers
            .iter()
            .filter(|(node, p)| Some(node.as_str()) != our_node.as_deref() && member_ok(state, p.user))
            .map(|(_, p)| (p.url.clone(), bell::token(&p.key, day)))
            .collect()
    };
    for (url, token) in targets {
        queue_ring(access, url, token, None);
    }
}

pub async fn run<A: AppAccess + Clone>(access: A) {
    let started = Instant::now();
    let mut hello_at: Option<Instant> = None;
    loop {
        let app = access.app();
        let servers: Vec<ServerId> = app.servers.lock().unwrap().keys().copied().collect();
        for server in &servers {
            let start = app.doorbell.lock().unwrap().listening.insert(*server);
            if start {
                spawn(listen(access.clone(), *server));
            }
        }
        let hello_due = started.elapsed() >= HELLO_AFTER_START && hello_at.is_none_or(|at| at.elapsed() >= HELLO_AFTER_START);
        if hello_due {
            hello_at = Some(Instant::now());
            let lonely: Vec<ServerId> = {
                let net = app.net.lock().await;
                match net.as_ref() {
                    Some(node) => servers.iter().copied().filter(|s| node.neighbor_count(*s) == 0).collect(),
                    None => vec![],
                }
            };
            for server in lonely {
                if app.outbox.lock().unwrap().has_pending(server) {
                    hello(&access, server);
                }
            }
        }
        tokio::time::sleep(LISTEN_CHECK_EVERY).await;
    }
}

fn spawn<F>(future: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(runtime) => {
            runtime.spawn(future);
        }
        Err(_) => {
            tauri::async_runtime::spawn(future);
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct Taken {
    #[serde(default)]
    pub(crate) ids: Vec<String>,
    #[serde(default)]
    pub(crate) hello: bool,
}

pub(crate) async fn take(app: &App, endpoints: &Endpoints, token: &str, wait: u64) -> anyhow::Result<Taken> {
    let body = serde_json::json!({ "token": token, "wait": wait });
    let res = app
        .http
        .post(&endpoints.take)
        .json(&body)
        .timeout(Duration::from_secs(wait + 20))
        .send()
        .await?
        .error_for_status()?;
    Ok(res.json().await?)
}

async fn listen<A: AppAccess + Clone>(access: A, server: ServerId) {
    let mut failures: u32 = 0;
    loop {
        let app = access.app();
        if !app.servers.lock().unwrap().contains_key(&server) {
            app.doorbell.lock().unwrap().listening.remove(&server);
            return;
        }
        let origin = crate::account::effective_server();
        let Some(endpoints) = endpoints(app, &origin).await else {
            tokio::time::sleep(Duration::from_secs(5 * 60)).await;
            continue;
        };
        let (key, new) = app.doorbell.lock().unwrap().own_key(server);
        if new {
            persist(app);
            announce(&access, server);
        }
        let today = bell::day_of(chrono::Utc::now());
        let first = app.doorbell.lock().unwrap().saved.last_take.get(&server).copied().unwrap_or(today);
        let first = first.max(today.saturating_sub(bell::KEEP_DAYS - 1));
        let first = if chrono::Utc::now().timestamp().rem_euclid(86_400) < YESTERDAY_TOKEN_GRACE_SECS {
            first.min(today.saturating_sub(1))
        } else {
            first
        };
        let mut result = Ok(());
        for day in first..=today {
            let wait = if day == today { SERVER_MAX_TAKE_WAIT_SECS } else { 0 };
            match take(app, &endpoints, &bell::token(&key, day), wait).await {
                Ok(taken) => {
                    let changed = app.doorbell.lock().unwrap().saved.last_take.insert(server, day) != Some(day);
                    if changed {
                        persist(app);
                    }
                    on_taken(&access, server, &key, taken);
                }
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        if app.doorbell.lock().unwrap().awaited_count(server) > 0 {
            nudge(&access, server).await;
        }
        match result {
            Ok(()) => failures = 0,
            Err(e) => {
                failures = failures.saturating_add(1);
                tracing::debug!("doorbell take failed: {e}");
                let base = 5u64.saturating_mul(1 << failures.min(6)).min(300);
                let jitter = rand_jitter(base);
                tokio::time::sleep(Duration::from_secs(base + jitter)).await;
            }
        }
    }
}

fn rand_jitter(base: u64) -> u64 {
    let r = u64::from(commy_net::node::CommyNode::generate_secret()[0]);
    r * base / 512
}

fn on_taken<A: AppAccess + Clone>(access: &A, server: ServerId, key: &DoorbellKey, taken: Taken) {
    let app = access.app();
    if !app.servers.lock().unwrap().contains_key(&server) {
        return;
    }
    let changed = note_taken(app, server, key, taken.ids);
    if changed {
        access.emit("server-event", serde_json::json!(server.to_string()));
    }
    if changed || taken.hello {
        app.doorbell.lock().unwrap().nudged_at.remove(&server);
        let access = access.clone();
        spawn(async move { nudge(&access, server).await });
    }
}

pub(crate) fn note_taken(app: &App, server: ServerId, key: &DoorbellKey, ids: Vec<String>) -> bool {
    let ids: Vec<String> = ids.into_iter().filter(|id| id.len() == 24 && id.bytes().all(|b| b.is_ascii_hexdigit())).collect();
    if ids.is_empty() {
        return false;
    }
    let held = {
        let servers = app.servers.lock().unwrap();
        let Some(state) = servers.get(&server) else { return false };
        held_ids(state, key)
    };
    let changed = app.doorbell.lock().unwrap().add_awaited(server, ids, &held, now_secs());
    if changed {
        persist(app);
    }
    changed
}

async fn nudge<A: AppAccess + Clone>(access: &A, server: ServerId) {
    let app = access.app();
    {
        let mut doorbell = app.doorbell.lock().unwrap();
        let fresh = doorbell.newest_awaited(server).is_some_and(|at| now_secs() - at < 3600);
        let every = if fresh { NUDGE_EVERY } else { NUDGE_SLOW };
        if doorbell.nudged_at.get(&server).is_some_and(|at| at.elapsed() < every) {
            return;
        }
        doorbell.nudged_at.insert(server, Instant::now());
    }
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let members: Vec<String> = {
        let servers = app.servers.lock().unwrap();
        let Some(state) = servers.get(&server) else { return };
        state.profiles.values().filter(|p| Some(p.id) != me).filter_map(|p| p.node.clone()).collect()
    };
    if crate::wake::unrest(app).await > 0 {
        tracing::info!("a doorbell ring woke us from resting");
    }
    let neighbors = {
        let net = app.net.lock().await;
        let Some(node) = net.as_ref() else { return };
        if !members.is_empty() {
            let _ = tokio::time::timeout(Duration::from_secs(2), node.join_peers(server, &members)).await;
        }
        node.neighbors(server)
    };
    for peer in neighbors {
        access.catch_up(server, peer);
    }
}

pub fn settle<A: AppAccess>(access: &A, server: ServerId) {
    let app = access.app();
    let changed = {
        let Some(key) = app.doorbell.lock().unwrap().saved.own.get(&server).copied() else { return };
        if app.doorbell.lock().unwrap().awaited_count(server) == 0 {
            return;
        }
        let held = {
            let servers = app.servers.lock().unwrap();
            let Some(state) = servers.get(&server) else { return };
            held_ids(state, &key)
        };
        app.doorbell.lock().unwrap().settle_awaited(server, &held, now_secs())
    };
    if changed {
        persist(app);
        access.emit("server-event", serde_json::json!(server.to_string()));
    }
}

pub fn arrived<A: AppAccess>(access: &A, server: ServerId, message: MessageId) {
    let app = access.app();
    let changed = {
        let mut doorbell = app.doorbell.lock().unwrap();
        let Some(key) = doorbell.saved.own.get(&server).copied() else { return };
        let id = bell::blinded_id(&key, message);
        doorbell.saved.awaited.get_mut(&server).is_some_and(|a| a.remove(&id).is_some())
    };
    if changed {
        persist(app);
        access.emit("server-event", serde_json::json!(server.to_string()));
    }
}

#[tauri::command]
pub fn get_awaited(app: tauri::State<'_, App>, server_id: String) -> Result<u32, String> {
    let server: ServerId = server_id.parse().map_err(|e| format!("{e}"))?;
    Ok(awaited(&app, server))
}

pub(crate) fn awaited(app: &App, server: ServerId) -> u32 {
    let (changed, count) = {
        let servers = app.servers.lock().unwrap();
        let mut doorbell = app.doorbell.lock().unwrap();
        let key = doorbell.saved.own.get(&server).copied();
        let held = match (key, servers.get(&server)) {
            (Some(key), Some(state)) if doorbell.awaited_count(server) > 0 => held_ids(state, &key),
            _ => HashSet::new(),
        };
        let changed = doorbell.settle_awaited(server, &held, now_secs());
        (changed, doorbell.awaited_count(server) as u32)
    };
    if changed {
        persist(app);
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_origins_are_https_origins_or_our_dev_server() {
        let own = "http://localhost:3100";
        assert!(peer_origin_ok("https://commy.chat", own));
        assert!(peer_origin_ok("https://auth.example.org:8443", own));
        assert!(peer_origin_ok("http://localhost:3100", own));
        assert!(!peer_origin_ok("http://localhost:3100", "https://commy.chat"));
        assert!(!peer_origin_ok("http://example.org", own));
        assert!(!peer_origin_ok("https://commy.chat/", own));
        assert!(!peer_origin_ok("https://commy.chat/api", own));
        assert!(!peer_origin_ok("https://user@commy.chat", own));
        assert!(!peer_origin_ok("https://Commy.chat", own));
        assert!(!peer_origin_ok("", own));
    }

    #[test]
    fn discovery_endpoints_must_be_offered_and_on_the_origin() {
        let doc = |features: bool, ring: &str| Discovery {
            features: Some(DiscoveryFeatures { doorbell: features }),
            endpoints: Some(DiscoveryEndpoints {
                ring: Some(ring.to_string()),
                take: Some("https://commy.chat/api/auth/commy/doorbell/take".to_string()),
            }),
        };
        let origin = "https://commy.chat";
        let ok = endpoints_of(doc(true, "https://commy.chat/api/auth/commy/doorbell/ring"), origin).unwrap();
        assert_eq!(ok.ring, "https://commy.chat/api/auth/commy/doorbell/ring");
        assert!(endpoints_of(doc(false, "https://commy.chat/api/auth/commy/doorbell/ring"), origin).is_none());
        assert!(endpoints_of(doc(true, "https://commy.chat.evil.example/ring"), origin).is_none());
        assert!(endpoints_of(doc(true, "https://other.example/ring"), origin).is_none());
        let old: Discovery = serde_json::from_str(r#"{"issuer":"https://commy.chat","endpoints":{}}"#).unwrap();
        assert!(endpoints_of(old, origin).is_none(), "servers without the doorbell are skipped");
    }

    #[test]
    fn awaited_ids_skip_held_ones_expire_and_stay_capped() {
        let mut d = Doorbell::default();
        let server = ServerId::new();
        let held: HashSet<String> = ["aa".repeat(12)].into_iter().collect();
        assert!(d.add_awaited(server, vec!["aa".repeat(12), "bb".repeat(12)], &held, 100));
        assert_eq!(d.awaited_count(server), 1);
        assert!(!d.add_awaited(server, vec!["bb".repeat(12)], &held, 200), "an id is awaited once");
        let now_held: HashSet<String> = ["bb".repeat(12)].into_iter().collect();
        assert!(d.settle_awaited(server, &now_held, 300));
        assert_eq!(d.awaited_count(server), 0);
        d.add_awaited(server, vec!["cc".repeat(12)], &HashSet::new(), 0);
        assert!(!d.settle_awaited(server, &HashSet::new(), AWAIT_SECS - 1));
        assert!(d.settle_awaited(server, &HashSet::new(), AWAIT_SECS));
        let many: Vec<String> = (0..MAX_AWAITED + 5).map(|i| format!("{i:024x}")).collect();
        d.add_awaited(server, many, &HashSet::new(), 1);
        assert_eq!(d.awaited_count(server), MAX_AWAITED);
    }

    #[test]
    fn peers_change_only_when_their_announcement_does() {
        let mut d = Doorbell::default();
        let server = ServerId::new();
        let user = UserId::new();
        let peer = Peer { user, url: "https://commy.chat".into(), key: [1; 32] };
        assert!(d.set_peer(server, user, "ab", Some(peer.clone())));
        assert!(!d.set_peer(server, user, "ab", Some(peer.clone())));
        assert!(d.set_peer(server, user, "ab", Some(Peer { key: [2; 32], ..peer.clone() })));
        assert!(d.set_peer(server, user, "ab", None));
        assert!(!d.set_peer(server, user, "ab", None));
    }

    #[test]
    fn members_cannot_take_over_or_flood_device_entries() {
        let mut d = Doorbell::default();
        let server = ServerId::new();
        let (alice, mallory) = (UserId::new(), UserId::new());
        let entry = |user| Peer { user, url: "https://commy.chat".into(), key: [1; 32] };
        assert!(d.set_peer(server, alice, "a1", Some(entry(alice))));
        assert!(!d.set_peer(server, mallory, "a1", Some(entry(mallory))), "another member's device stays theirs");
        assert!(!d.set_peer(server, mallory, "a1", None));
        assert_eq!(d.saved.peers[&server]["a1"].user, alice);
        for i in 0..100 {
            d.set_peer(server, mallory, &format!("m{i:03}"), Some(entry(mallory)));
        }
        let theirs = d.saved.peers[&server].values().filter(|p| p.user == mallory).count();
        assert_eq!(theirs, MAX_PEERS_PER_USER);
        assert!(d.retain_members(server, |u| u == alice));
        assert_eq!(d.saved.peers[&server].len(), 1, "departed members' devices go");
    }

    #[test]
    fn ring_budget_per_account_server() {
        let mut d = Doorbell::default();
        let now = Instant::now();
        for _ in 0..RINGS_PER_MINUTE {
            assert_eq!(d.spend("https://commy.chat", now), None);
        }
        assert!(d.spend("https://commy.chat", now).is_some());
        assert_eq!(d.spend("https://other.example", now), None, "each server has its own budget");
        assert_eq!(d.spend("https://commy.chat", now + Duration::from_secs(61)), None, "a new minute starts fresh");
    }

    #[test]
    fn saved_state_round_trips() {
        let dir = std::env::temp_dir().join(format!("commy-doorbell-{}", ServerId::new()));
        let mut d = Doorbell::default();
        let server = ServerId::new();
        let (key, new) = d.own_key(server);
        assert!(new);
        assert_eq!(d.own_key(server), (key, false));
        d.add_awaited(server, vec!["dd".repeat(12)], &HashSet::new(), 5);
        d.save(&dir).unwrap();
        let loaded = Doorbell::load(&dir);
        assert_eq!(loaded.saved.own.get(&server), Some(&key));
        assert_eq!(loaded.awaited_count(server), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
