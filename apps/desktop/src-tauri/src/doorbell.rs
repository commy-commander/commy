//! Message doorbell client, see [`commy_core::doorbell`].
//!
//! Each device listens on its own account server for rings addressed to
//! its daily token in every server it is in, and announces the key and
//! account server to that server's members. When a message of ours goes
//! out while a member's device looks offline, we ring that device with
//! the message's blinded id. A device that takes ids it lacks shows them
//! as on their way and reaches out to its peers until they arrive.
//! Requests carry one token each and never a sign-in, and neither tokens
//! nor ids reveal the server, account or message to the account server.

use crate::catchup::AppAccess;
use crate::state::App;
use commy_core::doorbell::{self as bell, DoorbellKey};
use commy_core::{MessageId, ServerEvent, ServerId, ServerState, UserId};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};
use tauri::Manager;

pub const DOORBELL_FILE: &str = "doorbell.json";
/// Ids for one device are gathered this long into one ring.
const RING_DEBOUNCE: Duration = Duration::from_secs(2);
/// How long an account server's doorbell support is trusted.
const ORIGIN_TTL: Duration = Duration::from_secs(60 * 60);
/// Long-poll length of a take, in seconds; the server allows up to 25.
const TAKE_WAIT_SECS: u64 = 25;
/// Hellos to one server's members at most this often.
const HELLO_EVERY: Duration = Duration::from_secs(30 * 60);
/// While messages are on their way, peers are asked again this often.
const NUDGE_EVERY: Duration = Duration::from_secs(60);
/// Awaited ids that never arrive are dropped after the server would
/// have deleted the ring anyway.
const AWAIT_SECS: i64 = bell::KEEP_DAYS as i64 * 86_400;
/// Most awaited ids kept per server.
const MAX_AWAITED: usize = 1000;

/// Where one member's device listens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Peer {
    pub user: UserId,
    pub url: String,
    pub key: DoorbellKey,
}

/// What survives restarts, by server id.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Saved {
    /// Our key in each server.
    #[serde(default)]
    own: HashMap<ServerId, DoorbellKey>,
    /// Members' devices by node id (hex).
    #[serde(default)]
    peers: HashMap<ServerId, HashMap<String, Peer>>,
    /// Blinded ids we were rung for and do not hold yet, with the Unix
    /// time they arrived.
    #[serde(default)]
    awaited: HashMap<ServerId, HashMap<String, i64>>,
    /// Last day whose token we took from, per server.
    #[serde(default)]
    last_take: HashMap<ServerId, u32>,
}

/// Doorbell endpoints an account server offers.
#[derive(Debug, Clone, PartialEq)]
struct Endpoints {
    ring: String,
    take: String,
}

#[derive(Debug, Default)]
pub struct Doorbell {
    saved: Saved,
    /// Ids waiting for the debounce, by (account server origin, token).
    ringing: HashMap<(String, String), Vec<String>>,
    /// Account servers checked for the doorbell, with when.
    origins: HashMap<String, (Instant, Option<Endpoints>)>,
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

    /// Forgets servers we are no longer in.
    pub fn retain_servers(&mut self, keep: impl Fn(&ServerId) -> bool) -> bool {
        let before = self.saved.own.len() + self.saved.peers.len() + self.saved.awaited.len() + self.saved.last_take.len();
        self.saved.own.retain(|s, _| keep(s));
        self.saved.peers.retain(|s, _| keep(s));
        self.saved.awaited.retain(|s, _| keep(s));
        self.saved.last_take.retain(|s, _| keep(s));
        before != self.saved.own.len() + self.saved.peers.len() + self.saved.awaited.len() + self.saved.last_take.len()
    }

    /// Our key in a server, made on first use. True when it is new.
    fn own_key(&mut self, server: ServerId) -> (DoorbellKey, bool) {
        if let Some(key) = self.saved.own.get(&server) {
            return (*key, false);
        }
        let key: DoorbellKey = commy_net::node::CommyNode::generate_secret();
        self.saved.own.insert(server, key);
        (key, true)
    }

    /// Records a member's announcement. True when it changed anything.
    fn set_peer(&mut self, server: ServerId, node: &str, peer: Option<Peer>) -> bool {
        let peers = self.saved.peers.entry(server).or_default();
        match peer {
            Some(peer) => peers.insert(node.to_string(), peer.clone()) != Some(peer),
            None => peers.remove(node).is_some(),
        }
    }

    /// Adds ids we were rung for, minus those we hold. True when the
    /// awaited set changed.
    fn add_awaited(&mut self, server: ServerId, ids: Vec<String>, held: &HashSet<String>, now: i64) -> bool {
        let awaited = self.saved.awaited.entry(server).or_default();
        let mut changed = false;
        for id in ids {
            if held.contains(&id) || awaited.contains_key(&id) {
                continue;
            }
            if awaited.len() >= MAX_AWAITED {
                // The oldest goes; it is the likeliest to have been deleted.
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

    /// Drops awaited ids we now hold or that expired. True when it changed.
    fn settle_awaited(&mut self, server: ServerId, held: &HashSet<String>, now: i64) -> bool {
        let Some(awaited) = self.saved.awaited.get_mut(&server) else { return false };
        let before = awaited.len();
        awaited.retain(|id, at| !held.contains(id) && now - *at < AWAIT_SECS);
        before != awaited.len()
    }

    fn awaited_count(&self, server: ServerId) -> usize {
        self.saved.awaited.get(&server).map_or(0, HashMap::len)
    }
}

/// `url` as a doorbell origin: exactly `scheme://host[:port]`, https, or
/// plain http on this machine only when it is our own account server.
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

/// The doorbell endpoints in a discovery document, when it offers them on
/// its own origin.
fn endpoints_of(doc: Discovery, origin: &str) -> Option<Endpoints> {
    if !doc.features.is_some_and(|f| f.doorbell) {
        return None;
    }
    let endpoints = doc.endpoints?;
    let (ring, take) = (endpoints.ring?, endpoints.take?);
    (on_origin(&ring, origin) && on_origin(&take, origin)).then_some(Endpoints { ring, take })
}

/// The doorbell endpoints of an account server, checked at most hourly.
async fn endpoints(app: &App, origin: &str) -> Option<Endpoints> {
    if let Some((at, found)) = app.doorbell.lock().unwrap().origins.get(origin) {
        if at.elapsed() < ORIGIN_TTL {
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
    // A network failure is not remembered, so the next try checks again.
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

/// Blinded ids of every message we hold in a server, under our key.
fn held_ids(state: &ServerState, key: &DoorbellKey) -> HashSet<String> {
    state
        .server
        .channels
        .iter()
        .flat_map(|c| state.messages(c.id))
        .map(|m| bell::blinded_id(key, m.id))
        .collect()
}

fn member_ok(state: &ServerState, user: UserId) -> bool {
    state.server.is_member(user) && !state.server.banned.contains(&user)
}

// ---------- Announcing ----------

/// Our signed announcement for a server: our account server and key when
/// it offers the doorbell, else none so members stop ringing us there.
/// None while our key may still change or the network is not up.
async fn announcement(app: &App, server: ServerId) -> Option<ServerEvent> {
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

/// Tells a server's members where we listen. Runs apart from the caller
/// since it may ask the account server first.
pub fn announce(handle: &tauri::AppHandle, server: ServerId) {
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        let app = handle.state::<App>();
        if let Some(event) = announcement(&app, server).await {
            let net = app.net.lock().await;
            if let Some(node) = net.as_ref() {
                if let Err(e) = node.broadcast(server, commy_net::WirePayload::Event(event)).await {
                    tracing::debug!("doorbell announcement failed: {e}");
                }
            }
        }
    });
}

/// A member's announcement: kept when it is signed by their pinned key,
/// they are still in the server and its origin is acceptable.
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
    let changed = app.doorbell.lock().unwrap().set_peer(server, &node.to_lowercase(), peer);
    if changed {
        persist(app);
    }
}

// ---------- Ringing ----------

/// Rings members' devices for a message of ours. With `everyone`, the
/// message reached nobody, so every device is rung; otherwise only those
/// of members who do not look online.
pub fn ring(handle: &tauri::AppHandle, server: ServerId, message: MessageId, everyone: bool) {
    let app = handle.state::<App>();
    let our_node = app.node_hex.lock().unwrap().clone();
    let day = bell::day_of(chrono::Utc::now());
    let targets: Vec<(String, String, String)> = {
        let servers = app.servers.lock().unwrap();
        let Some(state) = servers.get(&server) else { return };
        let doorbell = app.doorbell.lock().unwrap();
        let Some(peers) = doorbell.saved.peers.get(&server) else { return };
        peers
            .iter()
            .filter(|(node, _)| Some(node.as_str()) != our_node.as_deref())
            .filter(|(_, p)| member_ok(state, p.user))
            .filter(|(_, p)| everyone || !state.profiles.get(&p.user).is_some_and(|u| u.presence.is_visible_online()))
            .map(|(_, p)| (p.url.clone(), bell::token(&p.key, day), bell::blinded_id(&p.key, message)))
            .collect()
    };
    for (url, token, id) in targets {
        queue_ring(handle, url, token, Some(id));
    }
}

/// Adds an id (or, with None, a hello) to a device's next ring, starting
/// the debounce for that device when none is running.
fn queue_ring(handle: &tauri::AppHandle, url: String, token: String, id: Option<String>) {
    let app = handle.state::<App>();
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
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(RING_DEBOUNCE).await;
        let app = handle.state::<App>();
        let Some(ids) = app.doorbell.lock().unwrap().ringing.remove(&key) else { return };
        let (origin, token) = key;
        let Some(endpoints) = endpoints(&app, &origin).await else {
            tracing::debug!("a member's account server does not offer the doorbell");
            return;
        };
        let mut chunks: Vec<&[String]> = ids.chunks(bell::MAX_RING_IDS).collect();
        if chunks.is_empty() {
            chunks.push(&[]);
        }
        for chunk in chunks {
            let body = serde_json::json!({ "token": token, "ids": chunk });
            let sent = app.http.post(&endpoints.ring).json(&body).timeout(Duration::from_secs(15)).send().await;
            if let Err(e) = sent.and_then(|r| r.error_for_status()) {
                tracing::debug!("doorbell ring failed: {e}");
                return;
            }
        }
    });
}

/// Wakes every member device of a server we know, once per
/// [`HELLO_EVERY`]: they then reach out to us.
fn hello(handle: &tauri::AppHandle, server: ServerId) {
    let app = handle.state::<App>();
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
        queue_ring(handle, url, token, None);
    }
}

// ---------- Listening ----------

/// Starts listening in every server we are in, and says hello where we
/// have messages waiting and nobody to give them to. Runs for the life
/// of the app.
pub async fn run(handle: tauri::AppHandle) {
    tokio::time::sleep(Duration::from_secs(30)).await;
    loop {
        let app = handle.state::<App>();
        let servers: Vec<ServerId> = app.servers.lock().unwrap().keys().copied().collect();
        for server in &servers {
            let start = app.doorbell.lock().unwrap().listening.insert(*server);
            if start {
                tauri::async_runtime::spawn(listen(handle.clone(), *server));
            }
        }
        let lonely: Vec<ServerId> = {
            let net = app.net.lock().await;
            match net.as_ref() {
                Some(node) => servers.iter().copied().filter(|s| node.neighbor_count(*s) == 0).collect(),
                None => vec![],
            }
        };
        for server in lonely {
            if app.outbox.lock().unwrap().has_pending(server) {
                hello(&handle, server);
            }
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

#[derive(Deserialize)]
struct Taken {
    #[serde(default)]
    ids: Vec<String>,
    #[serde(default)]
    hello: bool,
}

async fn take(app: &App, endpoints: &Endpoints, token: &str, wait: u64) -> anyhow::Result<Taken> {
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

/// Takes rings for one server from our account server: days missed since
/// the last take, then today's token in a long poll, until we leave it.
async fn listen(handle: tauri::AppHandle, server: ServerId) {
    let mut failures: u32 = 0;
    loop {
        let app = handle.state::<App>();
        if !app.servers.lock().unwrap().contains_key(&server) {
            app.doorbell.lock().unwrap().listening.remove(&server);
            return;
        }
        let origin = crate::account::effective_server();
        let Some(endpoints) = endpoints(&app, &origin).await else {
            tokio::time::sleep(Duration::from_secs(5 * 60)).await;
            continue;
        };
        let (key, new) = app.doorbell.lock().unwrap().own_key(server);
        if new {
            persist(&app);
            announce(&handle, server);
        }
        let today = bell::day_of(chrono::Utc::now());
        let first = app.doorbell.lock().unwrap().saved.last_take.get(&server).copied().unwrap_or(today);
        let first = first.max(today.saturating_sub(bell::KEEP_DAYS - 1));
        let mut result = Ok(());
        for day in first..=today {
            let wait = if day == today { TAKE_WAIT_SECS } else { 0 };
            match take(&app, &endpoints, &bell::token(&key, day), wait).await {
                Ok(taken) => {
                    let changed = app.doorbell.lock().unwrap().saved.last_take.insert(server, day) != Some(day);
                    if changed {
                        persist(&app);
                    }
                    on_taken(&handle, server, &key, taken);
                }
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        if app.doorbell.lock().unwrap().awaited_count(server) > 0 {
            nudge(&handle, server).await;
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

/// Rings we took: ids we lack are awaited and we reach out to peers; a
/// hello means someone is back, so we reach out too.
fn on_taken(handle: &tauri::AppHandle, server: ServerId, key: &DoorbellKey, taken: Taken) {
    let app = handle.state::<App>();
    let ids: Vec<String> = taken
        .ids
        .into_iter()
        .filter(|id| id.len() == 24 && id.bytes().all(|b| b.is_ascii_hexdigit()))
        .collect();
    let changed = !ids.is_empty() && {
        let held = {
            let servers = app.servers.lock().unwrap();
            let Some(state) = servers.get(&server) else { return };
            held_ids(state, key)
        };
        app.doorbell.lock().unwrap().add_awaited(server, ids, &held, now_secs())
    };
    if changed {
        persist(&app);
        let _ = tauri::Emitter::emit(handle, "server-event", &server.to_string());
    }
    if changed || taken.hello {
        app.doorbell.lock().unwrap().nudged_at.remove(&server);
        let handle = handle.clone();
        tauri::async_runtime::spawn(async move { nudge(&handle, server).await });
    }
}

/// Re-dials a server's members and catches up with current neighbors, at
/// most once per [`NUDGE_EVERY`].
async fn nudge(handle: &tauri::AppHandle, server: ServerId) {
    let app = handle.state::<App>();
    {
        let mut doorbell = app.doorbell.lock().unwrap();
        if doorbell.nudged_at.get(&server).is_some_and(|at| at.elapsed() < NUDGE_EVERY) {
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
    let neighbors = {
        let net = app.net.lock().await;
        let Some(node) = net.as_ref() else { return };
        if !members.is_empty() {
            // Never hold the network lock for long: sends wait on it.
            let _ = tokio::time::timeout(Duration::from_secs(2), node.join_peers(server, &members)).await;
        }
        node.neighbors(server)
    };
    for peer in neighbors {
        crate::catchup::spawn_session(handle.clone(), server, peer, None, false);
    }
}

// ---------- Arrivals ----------

/// Drops awaited ids for messages we now hold, after a catch-up or when
/// one arrives live.
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

/// A message arrived: it is no longer on its way.
pub fn arrived(app: &App, handle: &tauri::AppHandle, server: ServerId, message: MessageId) {
    let changed = {
        let mut doorbell = app.doorbell.lock().unwrap();
        let Some(key) = doorbell.saved.own.get(&server).copied() else { return };
        let id = bell::blinded_id(&key, message);
        doorbell.saved.awaited.get_mut(&server).is_some_and(|a| a.remove(&id).is_some())
    };
    if changed {
        persist(app);
        let _ = tauri::Emitter::emit(handle, "server-event", &server.to_string());
    }
}

/// How many messages we were told about in a server and do not hold yet.
#[tauri::command]
pub fn get_awaited(app: tauri::State<'_, App>, server_id: String) -> Result<u32, String> {
    let server: ServerId = server_id.parse().map_err(|e| format!("{e}"))?;
    let mut doorbell = app.doorbell.lock().unwrap();
    let expired = doorbell.settle_awaited(server, &HashSet::new(), now_secs());
    let count = doorbell.awaited_count(server) as u32;
    drop(doorbell);
    if expired {
        persist(&app);
    }
    Ok(count)
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
        let peer = Peer { user: UserId::new(), url: "https://commy.chat".into(), key: [1; 32] };
        assert!(d.set_peer(server, "ab", Some(peer.clone())));
        assert!(!d.set_peer(server, "ab", Some(peer.clone())));
        assert!(d.set_peer(server, "ab", Some(Peer { key: [2; 32], ..peer })));
        assert!(d.set_peer(server, "ab", None));
        assert!(!d.set_peer(server, "ab", None));
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
