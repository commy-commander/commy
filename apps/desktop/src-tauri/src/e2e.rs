//! End-to-end checks of private invite links, the message doorbell,
//! offline delivery and waking a resting device, with the app's own code on real iroh nodes and a real
//! account server. Each device is an `App` without a webview; its network
//! events are handled the way `run_net_loop` handles them.
//!
//! Run them with `scripts/e2e.sh`, which starts the account server on a
//! throwaway Postgres and sets `COMMY_AUTH_URL` and `COMMY_E2E_DATABASE_URL`.

use crate::catchup::{self, AppAccess, AppStore, TestHandle};
use crate::doorbell;
use crate::invite_link::{self, Expiry};
use crate::outbox::{self, Outbox};
use crate::state::App;
use crate::wake;
use commy_core::doorbell as bell;
use commy_core::{
    ChannelId, ChannelKind, Identity, IdentityKey, Message, MessageId, Presence, ServerEvent, ServerId, StoreError,
    TombstoneProof, UserId, UserProfile,
};
use commy_net::local_relay::LocalRelay;
use commy_net::sealed::SealKey;
use commy_net::{CommyNode, NetEvent, ServerTicket, WirePayload};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;

const WAIT: Duration = Duration::from_secs(90);

fn auth_url() -> String {
    std::env::var("COMMY_AUTH_URL").expect("run these with scripts/e2e.sh, which sets COMMY_AUTH_URL")
}

/// Runs a query on the account server's database and returns its rows,
/// one per line with `|` between columns.
fn sql(query: &str) -> String {
    let url = std::env::var("COMMY_E2E_DATABASE_URL").expect("COMMY_E2E_DATABASE_URL is set by scripts/e2e.sh");
    let out = std::process::Command::new("psql")
        .args([url.as_str(), "-At", "-v", "ON_ERROR_STOP=1", "-c", query])
        .output()
        .expect("psql runs");
    assert!(out.status.success(), "psql failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn columns(table: &str) -> Vec<String> {
    let rows = sql(&format!(
        "select column_name from information_schema.columns where table_name = '{table}' order by column_name"
    ));
    rows.lines().map(str::to_string).collect()
}

async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

/// Things about a server that its invite row must not reveal.
fn secrets_of(app: &App, server: ServerId) -> Vec<String> {
    let servers = app.servers.lock().unwrap();
    let state = &servers[&server];
    let mut out = vec![state.server.name.clone(), server.to_string(), server.0.simple().to_string()];
    out.push(app.invites.lock().unwrap()[&server].clone());
    out.push(hex(&commy_net::node::topic_for(server)));
    for profile in state.profiles.values() {
        out.push(profile.id.to_string());
        out.push(profile.username.clone());
        out.extend(profile.node.clone());
        out.extend(profile.identity.map(|k| k.to_hex()));
    }
    out
}

// ---------- A device ----------

struct Device {
    access: TestHandle,
    secret: [u8; 32],
    pump: Option<JoinHandle<()>>,
    dir: PathBuf,
    /// Relays the node uses instead of the app's (see [`LocalRelay`]).
    relays: Option<commy_net::local_relay::RelayMap>,
}

impl Device {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("commy-e2e-{name}-{}", hex(&CommyNode::generate_secret()[..6])));
        std::fs::create_dir_all(&dir).unwrap();
        let app = App::default();
        app.data_dir.set(dir.clone()).unwrap();
        let identity = Identity::generate();
        let mut profile = UserProfile::new(name);
        profile.identity = Some(identity.public());
        *app.profile.lock().unwrap() = Some(profile);
        *app.identity.lock().unwrap() = Some(identity);
        Self { access: TestHandle::new(Arc::new(app)), secret: CommyNode::generate_secret(), pump: None, dir, relays: None }
    }

    /// A device whose only relay is `relay`.
    fn on_relay(name: &str, relay: &LocalRelay) -> Self {
        let mut device = Self::new(name);
        device.relays = Some(relay.map());
        device
    }

    fn app(&self) -> &App {
        self.access.app()
    }

    fn me(&self) -> UserProfile {
        crate::current_profile(self.app()).unwrap()
    }

    fn key(&self) -> IdentityKey {
        self.app().identity().unwrap().public()
    }

    fn node_hex(&self) -> String {
        self.app().node_hex.lock().unwrap().clone().unwrap()
    }

    /// Starts the node and rejoins every server, as the app does at launch.
    async fn online(&mut self) {
        let app = self.app();
        let (mut node, events, _media, _dms) = match &self.relays {
            Some(relays) => CommyNode::spawn_with(Some(self.secret), Some(relays.clone())).await.unwrap(),
            None => CommyNode::spawn(Some(self.secret)).await.unwrap(),
        };
        let hex = node.node_id_hex();
        *app.node_hex.lock().unwrap() = Some(hex.clone());
        let me = {
            let mut profile = app.profile.lock().unwrap();
            let profile = profile.as_mut().unwrap();
            profile.node = Some(hex);
            profile.clone()
        };
        let servers: Vec<(ServerId, String, bool)> = app
            .servers
            .lock()
            .unwrap()
            .values()
            .map(|s| (s.server.id, s.server.name.clone(), s.server.owner == me.id))
            .collect();
        for (id, name, owned) in servers {
            if owned {
                let ticket = node.host_server(id, &name, app.own_key_for_tickets()).await.unwrap();
                app.invites.lock().unwrap().insert(id, ticket.encode());
            } else {
                let raw = app.invites.lock().unwrap()[&id].clone();
                // Fine when nobody is online yet: NeighborUp follows later.
                node.join_server(&ServerTicket::decode(&raw).unwrap()).await.unwrap();
            }
        }
        for state in app.servers.lock().unwrap().values_mut() {
            let server = state.server.id;
            let _ = state.apply(ServerEvent::MemberJoin { server, profile: me.clone() });
        }
        node.set_catchup_store(Arc::new(AppStore(self.access.clone())));
        *app.net.lock().await = Some(node);
        self.pump = Some(tokio::spawn(pump(self.access.clone(), events)));
    }

    /// Closes the app: the node goes away, what it saved stays.
    async fn offline(&mut self) {
        if let Some(pump) = self.pump.take() {
            pump.abort();
        }
        if let Some(node) = self.app().net.lock().await.take() {
            node.shutdown().await.ok();
        }
    }

    async fn create_server(&self, name: &str) -> ServerId {
        let app = self.app();
        let id = ServerId::new();
        let hosting = async {
            let mut net = app.net.lock().await;
            let ticket = net.as_mut().unwrap().host_server(id, name, app.own_key_for_tickets()).await.map_err(|e| e.to_string())?;
            app.invites.lock().unwrap().insert(id, ticket.encode());
            Ok(())
        };
        let (state, _, signing) = crate::own_server_state(app, id, name, hosting).await.unwrap();
        app.servers.lock().unwrap().insert(id, state);
        drop(signing);
        app.persist_server(id);
        id
    }

    async fn join(&self, ticket: &str) {
        crate::join_ticket(self.app(), &self.access, ServerTicket::decode(ticket).unwrap()).await.unwrap();
    }

    fn text_channel(&self, server: ServerId) -> ChannelId {
        let servers = self.app().servers.lock().unwrap();
        servers[&server].server.channels.iter().find(|c| c.kind == ChannelKind::Text).unwrap().id
    }

    fn message(&self, server: ServerId, id: MessageId) -> Option<Message> {
        self.app().servers.lock().unwrap()[&server].message(id).cloned()
    }

    fn knows_member(&self, server: ServerId, user: UserId) -> bool {
        self.app().servers.lock().unwrap().get(&server).is_some_and(|s| s.server.is_member(user))
    }

    /// Gossip neighbors on a server; None while the node is busy.
    fn neighbors(&self, server: ServerId) -> Option<usize> {
        let net = self.app().net.try_lock().ok()?;
        Some(net.as_ref().map_or(0, |n| n.neighbor_count(server)))
    }

    /// Sends a message the way `commit_message` does: applied here, sent
    /// or kept pending, then the doorbells of members who may have missed
    /// it are rung. Returns it, whether it went out, and the rings.
    async fn send(&self, server: ServerId, content: &str) -> (Message, bool, Vec<(String, String, String)>) {
        let app = self.app();
        let mut message = Message::new(self.text_channel(server), self.me().id, content);
        crate::sign_message(app, server, &mut message).unwrap();
        let event = ServerEvent::MessageCreate { message: message.clone() };
        app.servers.lock().unwrap().get_mut(&server).unwrap().apply(event.clone()).unwrap();
        let delivered = outbox::broadcast_local(app, server, event).await;
        app.persist_server(server);
        let rings = doorbell::ring_targets(app, server, message.id, !delivered);
        for (origin, token, id) in &rings {
            let endpoints = doorbell::endpoints(app, origin).await.expect("the account server offers the doorbell");
            doorbell::send_ring(app, &endpoints.ring, token, std::slice::from_ref(id)).await.unwrap();
        }
        (message, delivered, rings)
    }

    /// Edits or deletes the way `edit_message` and `delete_message` do.
    async fn change(&self, server: ServerId, event: ServerEvent) -> bool {
        let app = self.app();
        app.servers.lock().unwrap().get_mut(&server).unwrap().apply(event.clone()).unwrap();
        let delivered = outbox::broadcast_local(app, server, event).await;
        app.persist_server(server);
        delivered
    }

    async fn edit(&self, server: ServerId, id: MessageId, content: &str) -> bool {
        let original = self.message(server, id).unwrap();
        let identity = self.app().identity().unwrap();
        let event = crate::sign_edit(server, &original, content.to_string(), chrono::Utc::now(), &identity).unwrap();
        self.change(server, event).await
    }

    async fn delete(&self, server: ServerId, id: MessageId) -> bool {
        let channel = self.message(server, id).unwrap().channel;
        let identity = self.app().identity().unwrap();
        let proof = TombstoneProof::sign(server, channel, id, self.me().id, chrono::Utc::now(), &identity);
        self.change(server, proof.into_event()).await
    }

    /// Takes today's rings for a server, as the listener does, and returns
    /// the token and the ids taken.
    async fn take_doorbell(&self, server: ServerId) -> (String, Vec<String>) {
        let app = self.app();
        let endpoints = doorbell::endpoints(app, &auth_url()).await.unwrap();
        let (key, _) = app.doorbell.lock().unwrap().own_key(server);
        let token = bell::token(&key, bell::day_of(chrono::Utc::now()));
        let taken = doorbell::take(app, &endpoints, &token, 0).await.unwrap();
        doorbell::note_taken(app, server, &key, taken.ids.clone());
        (token, taken.ids)
    }

    /// "N messages are on their way".
    fn awaited(&self, server: ServerId) -> u32 {
        doorbell::awaited(self.app(), server)
    }

    fn outbox_on_disk(&self) -> Outbox {
        Outbox::load(&self.dir)
    }

    /// Away, the way AFK sets it: Idle for every member to see.
    async fn go_away(&self, server: ServerId) {
        let app = self.app();
        let profile = {
            let mut profile = app.profile.lock().unwrap();
            let profile = profile.as_mut().unwrap();
            profile.presence = Presence::Idle;
            profile.clone()
        };
        app.auto_idle.store(true, std::sync::atomic::Ordering::Relaxed);
        let event = ServerEvent::ProfileUpdate { profile: app.announced(profile) };
        app.servers.lock().unwrap().get_mut(&server).unwrap().apply(event.clone()).unwrap();
        assert!(outbox::broadcast_local(app, server, event).await);
    }

    fn presence_of(&self, server: ServerId, user: UserId) -> Option<Presence> {
        self.app().servers.lock().unwrap()[&server].profiles.get(&user).map(|p| p.presence)
    }

    /// In the server's mesh; None while the node is busy.
    fn joined(&self, server: ServerId) -> Option<bool> {
        let net = self.app().net.try_lock().ok()?;
        Some(net.as_ref().is_some_and(|n| n.is_joined(server)))
    }

    fn home_relay(&self) -> Option<String> {
        let net = self.app().net.try_lock().ok()?;
        net.as_ref()?.home_relay()
    }

    fn node_bytes(&self) -> [u8; 32] {
        data_encoding::HEXLOWER.decode(self.node_hex().as_bytes()).unwrap().try_into().unwrap()
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

async fn broadcast(app: &App, server: ServerId, payload: WirePayload) {
    if let Some(node) = app.net.lock().await.as_ref() {
        node.broadcast(server, payload).await.ok();
    }
}

/// A catch-up session with `peer`, as `catchup::run_session` runs it.
async fn session(access: TestHandle, server: ServerId, peer: [u8; 32]) {
    for _ in 0..3 {
        let app = access.app();
        if !app.catchup.lock().unwrap().begin((server, peer)) {
            return;
        }
        let run = {
            let net = app.net.lock().await;
            net.as_ref().map(|node| node.catch_up(server, peer, Arc::new(AppStore(access.clone()))))
        };
        let result = match run {
            Some(run) => run.await,
            None => Err(anyhow::anyhow!("offline")),
        };
        app.catchup.lock().unwrap().finish((server, peer), Instant::now(), result.is_ok());
        catchup::after_session(&access, server, result.as_ref().ok());
        match result {
            Ok(_) => return,
            Err(e) => {
                tracing::info!("catch-up failed, trying again: {e:#}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

/// What `run_net_loop` does with each network event, minus voice.
async fn pump(access: TestHandle, mut events: UnboundedReceiver<NetEvent>) {
    while let Some(event) = events.recv().await {
        let app = access.app();
        match event {
            NetEvent::Event { server, event, action, .. } => {
                if matches!(event, ServerEvent::DoorbellSet { .. }) {
                    doorbell::on_announce(app, server, &event);
                    continue;
                }
                let holdable = catchup::holdable(&event).then(|| event.clone());
                let held_action = action.map(|action| commy_core::SignedEvent { action, event: event.clone() });
                let (applied, new_message, pinned) = {
                    let mut servers = app.servers.lock().unwrap();
                    let Some(state) = servers.get_mut(&server) else { continue };
                    catchup::apply_live(state, event, action)
                };
                if applied.is_ok() {
                    if let Some(user) = pinned {
                        catchup::release_held(&access, server, &[user]);
                    }
                }
                match applied {
                    Ok(true) => {
                        app.persist_server(server);
                        if let Some(message) = new_message {
                            doorbell::arrived(&access, server, message.id);
                        }
                    }
                    Ok(false) => {}
                    Err(StoreError::UnknownAuthorKey(signer)) if holdable.is_some() => {
                        catchup::hold(app, server, signer, holdable.unwrap())
                    }
                    Err(StoreError::UnknownAuthorKey(_)) if held_action.is_some() => {
                        app.catchup.lock().unwrap().hold_action(server, held_action.unwrap(), Instant::now())
                    }
                    Err(e) => tracing::warn!("rejected remote event: {e}"),
                }
            }
            NetEvent::SyncRequested { server, .. } => {
                for payload in catchup::sync_replies(app, server) {
                    broadcast(app, server, payload).await;
                }
            }
            NetEvent::SyncReceived { server, payload } => {
                catchup::apply_snapshot(&access, server, payload);
            }
            NetEvent::VoiceDial { .. } => {}
            NetEvent::NeighborUp { server, peer } => {
                let Some(me) = app.profile.lock().unwrap().clone() else { continue };
                let join = ServerEvent::MemberJoin { server, profile: app.announced(me) };
                broadcast(app, server, app.wire_for(server, join)).await;
                if app.awaits_snapshot(server) {
                    broadcast(app, server, WirePayload::SyncRequest).await;
                }
                outbox::flush_queued(app, server).await;
                let ours: [u8; 32] = data_encoding::HEXLOWER
                    .decode(app.node_hex.lock().unwrap().as_deref().unwrap_or_default().as_bytes())
                    .ok()
                    .and_then(|b| b.try_into().ok())
                    .unwrap_or_default();
                let pending = app.outbox.lock().unwrap().has_pending(server);
                let delay = catchup::start_delay(ours, peer, pending);
                let session_access = access.clone();
                tokio::spawn(async move {
                    if let Some(delay) = delay {
                        tokio::time::sleep(delay).await;
                    }
                    session(session_access, server, peer).await;
                });
                if let Some(event) = doorbell::announcement(app, server).await {
                    broadcast(app, server, app.wire_for(server, event)).await;
                }
            }
            NetEvent::CaughtUp { server, peer, report } => {
                app.catchup.lock().unwrap().record_completed((server, peer), Instant::now());
                catchup::after_session(&access, server, Some(&report));
            }
            // As `wake::handle_wake`: back into the mesh, then a catch-up
            // with whoever woke us.
            NetEvent::Wake { from } => {
                let servers = wake::on_wake(app, from);
                if servers.is_empty() {
                    continue;
                }
                let access = access.clone();
                tokio::spawn(async move {
                    wake::unrest(access.app()).await;
                    for server in servers {
                        session(access.clone(), server, from).await;
                    }
                });
            }
        }
    }
}

/// An owner with a server, and members who joined it from its ticket and
/// know each other's doorbells.
async fn community(names: &[&str]) -> (Vec<Device>, ServerId) {
    gather(names.iter().map(|n| Device::new(n)).collect()).await
}

async fn gather(mut devices: Vec<Device>) -> (Vec<Device>, ServerId) {
    for d in devices.iter_mut() {
        d.online().await;
    }
    let server = devices[0].create_server("Moonbase Book Club").await;
    let ticket = devices[0].app().invites.lock().unwrap()[&server].clone();
    for d in &devices[1..] {
        d.join(&ticket).await;
    }
    for a in &devices {
        for b in &devices {
            let user = b.me().id;
            wait_until("every member to know every other", || a.knows_member(server, user)).await;
        }
    }
    // Each device rings every other one
    let others = devices.len() - 1;
    for d in &devices {
        wait_until("doorbell announcements", || {
            doorbell::ring_targets(d.app(), server, MessageId::new(), true).len() == others
        })
        .await;
    }
    (devices, server)
}

fn init_logs() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "error".into()))
        .with_test_writer()
        .try_init();
}

// ---------- 1. Private invite links ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn private_invite_links_join_and_reveal_nothing() {
    init_logs();
    let origin = auth_url();
    let mut owner = Device::new("ada");
    let mut joiner = Device::new("grace");
    let mut stranger = Device::new("linus");
    owner.online().await;
    joiner.online().await;
    stranger.online().await;
    let server = owner.create_server("Moonbase Book Club").await;

    let link = invite_link::create(owner.app(), &server.to_string(), Expiry::Week).await.unwrap().url;
    let parsed = invite_link::parse(&link).unwrap();
    assert_eq!(parsed.origin, origin);
    let key = parsed.key.clone().expect("the link carries its key");
    assert_eq!(link, format!("{origin}/i/{}#{key}", parsed.code));

    // The stored row: a code, a sealed blob and an expiry, nothing else
    assert_eq!(columns("invite"), ["code", "createdAt", "createdBy", "expiresAt", "id", "serverName", "ticket"]);
    let row = sql(&format!(
        "select ticket, \"serverName\", coalesce(\"createdBy\", '<null>') from invite where code = '{}'",
        parsed.code
    ));
    let mut fields = row.split('|');
    let (ticket, name, by) = (fields.next().unwrap(), fields.next().unwrap(), fields.next().unwrap());
    eprintln!("stored invite row: code={} serverName={name:?} createdBy={by} ticket={}…", parsed.code, &ticket[..24]);
    assert!(ticket.starts_with("sealed1."), "only a sealed ticket is stored");
    assert_eq!(name, "", "no server name is stored");
    assert_eq!(by, "<null>", "an anonymous link names no account");
    assert!(ServerTicket::decode(ticket).is_err(), "the stored blob is not a usable ticket");
    assert!(!ticket.contains(&key), "the key never reaches the server");
    for secret in secrets_of(owner.app(), server) {
        assert!(!ticket.contains(&secret), "the stored invite reveals {secret:?}");
    }

    // A second device joins through the link
    let resolved = invite_link::resolve(joiner.app(), &link).await.unwrap();
    assert_eq!(resolved.server_name, "Moonbase Book Club");
    assert!(resolved.trusted, "the link points at our own account server");
    joiner.join(&resolved.ticket).await;
    let (owner_id, joiner_id) = (owner.me().id, joiner.me().id);
    wait_until("the joiner to receive the server", || joiner.knows_member(server, owner_id)).await;
    wait_until("the owner to see the joiner", || owner.knows_member(server, joiner_id)).await;
    assert_eq!(joiner.app().servers.lock().unwrap()[&server].server.name, "Moonbase Book Club");

    // A link is good for anyone until it expires
    assert!(invite_link::resolve(stranger.app(), &link).await.is_ok());

    // Another key, a missing key or another code opens nothing
    let wrong = format!("{origin}/i/{}#{}", parsed.code, SealKey::generate().to_text());
    let err = invite_link::resolve(stranger.app(), &wrong).await.unwrap_err();
    assert_eq!(err, "This invite link is damaged or was changed");
    let keyless = format!("{origin}/i/{}", parsed.code);
    let err = invite_link::resolve(stranger.app(), &keyless).await.unwrap_err();
    assert_eq!(err, "This invite link is missing its key. Ask for a new link.");
    let other_code = if parsed.code == "ABCDEFGH" { "HGFEDCBA" } else { "ABCDEFGH" };
    let err = invite_link::resolve(stranger.app(), &format!("{origin}/i/{other_code}#{key}")).await.unwrap_err();
    assert_eq!(err, "This invite is invalid or has expired");

    // An expired link is refused, and the next link made sweeps it away
    sql(&format!("update invite set \"expiresAt\" = now() - interval '1 minute' where code = '{}'", parsed.code));
    let err = invite_link::resolve(stranger.app(), &link).await.unwrap_err();
    assert_eq!(err, "This invite is invalid or has expired");
    let other = joiner.create_server("Second Server").await;
    invite_link::create(joiner.app(), &other.to_string(), Expiry::Week).await.unwrap();
    assert_eq!(sql(&format!("select count(*) from invite where code = '{}'", parsed.code)), "0");

    // The expiry sealed in the link caps a row the server kept longer
    let hour = invite_link::create(owner.app(), &server.to_string(), Expiry::Hour).await.unwrap();
    let hour_code = invite_link::parse(&hour.url).unwrap().code;
    sql(&format!("update invite set \"expiresAt\" = now() + interval '1 year' where code = '{hour_code}'"));
    let resolved = invite_link::resolve(stranger.app(), &hour.url).await.unwrap();
    let until = chrono::DateTime::parse_from_rfc3339(resolved.expires_at.as_deref().unwrap()).unwrap();
    let left = until.with_timezone(&chrono::Utc) - chrono::Utc::now();
    assert!(left <= chrono::Duration::hours(1) && left > chrono::Duration::minutes(55), "{left}");

    // Links that never expire: only for a server marked public
    let err = invite_link::create(owner.app(), &server.to_string(), Expiry::Never).await.unwrap_err();
    assert_eq!(err, "Only links to a public server can last forever");
    owner.app().servers.lock().unwrap().get_mut(&server).unwrap()
        .apply(commy_core::ServerEvent::PublicSet { server, public: true })
        .unwrap();
    let forever = invite_link::create(owner.app(), &server.to_string(), Expiry::Never).await.unwrap();
    assert_eq!(forever.expires_at, None);
    let forever_code = invite_link::parse(&forever.url).unwrap().code;
    let row = sql(&format!(
        "select extract(year from \"expiresAt\"), \"serverName\", coalesce(\"createdBy\", '<null>'), ticket from invite where code = '{forever_code}'"
    ));
    let fields: Vec<&str> = row.split('|').collect();
    eprintln!("never-expiring row: year={} serverName={:?} createdBy={}", fields[0], fields[1], fields[2]);
    assert!(fields[0].starts_with("9999"), "stored as never expiring");
    assert_eq!((fields[1], fields[2]), ("", "<null>"), "no name and no account");
    for secret in secrets_of(owner.app(), server) {
        assert!(!fields[3].contains(&secret), "the stored invite reveals {secret:?}");
    }
    let resolved = invite_link::resolve(stranger.app(), &forever.url).await.unwrap();
    assert_eq!(resolved.expires_at, None);
    assert_eq!(resolved.server_name, "Moonbase Book Club");

    // A plain ticket, which names the server, is never accepted
    let raw = owner.app().invites.lock().unwrap()[&server].clone();
    let res = owner
        .app()
        .http
        .post(format!("{origin}/api/auth/commy/invites/create"))
        .json(&serde_json::json!({ "ticket": raw }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);

    for d in [&mut owner, &mut joiner, &mut stranger] {
        d.offline().await;
    }
}

// ---------- 2. Doorbell and catch-up ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn doorbell_wakes_offline_members_and_catch_up_delivers() {
    init_logs();
    let (mut devices, server) = community(&["ada", "grace", "linus"]).await;
    let [sender, first, second] = &mut devices[..] else { unreachable!() };
    let sender_key = sender.key();
    let user_ids = [sender.me().id, first.me().id, second.me().id];
    let node_ids = [sender.node_hex(), first.node_hex(), second.node_hex()];

    first.offline().await;
    second.offline().await;
    wait_until("the sender to have no neighbors", || sender.neighbors(server) == Some(0)).await;

    // Sent with nobody online: it waits on the sender and both are rung
    let (message, delivered, rings) = sender.send(server, "Chapter 3 tonight at eight").await;
    assert!(!delivered);
    assert!(sender.app().outbox.lock().unwrap().is_pending(server, message.id));
    assert!(sender.outbox_on_disk().is_pending(server, message.id), "the pending send is saved");
    assert_eq!(rings.len(), 2, "one ring per offline device");

    // What the account server holds: a daily token and a blinded id per ring
    assert_eq!(columns("doorbellRing"), ["blindedId", "createdAt", "id", "token"]);
    let stored = sql("select token, \"blindedId\" from \"doorbellRing\" order by token");
    let mut plain = vec![message.id.to_string(), message.id.0.simple().to_string(), server.to_string(), message.content.clone()];
    plain.extend(user_ids.iter().map(|u| u.to_string()));
    plain.extend(node_ids.iter().cloned());
    for (_, token, id) in &rings {
        assert!(stored.contains(&format!("{token}|{id}")));
        assert_eq!(sql(&format!("select count(*) from \"doorbellRing\" where token = '{token}'")), "1");
        assert!(token.len() == 32 && id.len() == 24);
        for p in &plain {
            assert!(!token.contains(p.as_str()) && !id.contains(p.as_str()));
        }
    }
    let (tokens, ids): (Vec<_>, Vec<_>) = rings.iter().map(|(_, t, i)| (t.clone(), i.clone())).unzip();
    assert_ne!(tokens[0], tokens[1], "each device has its own token");
    assert_ne!(ids[0], ids[1], "one message looks different to each device");
    eprintln!("stored rings: {}", rings.iter().map(|(_, t, i)| format!("{t}|{i}")).collect::<Vec<_>>().join(", "));

    // The first member's app starts: it takes its ring before the network is up
    let (token, taken) = first.take_doorbell(server).await;
    assert_eq!(taken.len(), 1);
    assert!(ids.contains(&taken[0]));
    assert_eq!(first.awaited(server), 1, "1 message is on its way");
    assert_eq!(sql(&format!("select count(*) from \"doorbellRing\" where token = '{token}'")), "0", "taking deletes the ring");
    let (_, again) = first.take_doorbell(server).await;
    assert!(again.is_empty(), "a ring is taken once");

    // Online again: catch-up brings the message, signed by its author
    first.online().await;
    wait_until("the message to arrive by catch-up", || first.message(server, message.id).is_some()).await;
    let got = first.message(server, message.id).unwrap();
    assert_eq!(got.content, message.content);
    assert!(got.verify(server, &sender_key), "the author's signature verifies");
    assert_eq!(first.awaited(server), 0, "nothing is on its way any more");
    wait_until("the sender's outbox to clear", || !sender.app().outbox.lock().unwrap().has_pending(server)).await;

    // The second member's ring was still waiting for it
    let (_, taken_second) = second.take_doorbell(server).await;
    assert_eq!(taken_second.len(), 1);
    assert!(ids.contains(&taken_second[0]) && taken_second[0] != taken[0]);
    assert_eq!(second.awaited(server), 1);
    second.online().await;
    wait_until("the second member to catch up", || second.message(server, message.id).is_some()).await;
    assert!(second.message(server, message.id).unwrap().verify(server, &sender_key));
    assert_eq!(second.awaited(server), 0);

    for d in devices.iter_mut() {
        d.offline().await;
    }
}

/// Linked devices are kept as an id, node id, owner and two times: no
/// computer name or anything else that describes them.
#[test]
fn linked_devices_keep_nothing_descriptive() {
    assert_eq!(columns("linkedDevice"), ["createdAt", "id", "lastSeenAt", "nodeId", "userId"]);
}

/// The bench case: the sender closes, the member comes back alone and
/// sees "1 message is on its way" from the app's own listener, before any
/// peer is reachable.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn doorbell_banner_shows_before_any_peer_is_back() {
    init_logs();
    let (mut devices, server) = community(&["ada", "grace"]).await;
    let [sender, member] = &mut devices[..] else { unreachable!() };

    member.offline().await;
    wait_until("the sender to have no neighbors", || sender.neighbors(server) == Some(0)).await;
    let (message, delivered, rings) = sender.send(server, "Snacks are on me").await;
    assert!(!delivered && rings.len() == 1);
    sender.offline().await;

    // The member's app starts as it does at launch: network up, listener running
    member.online().await;
    member.access.take_events();
    let launched = Instant::now();
    let listener = tokio::spawn(doorbell::run(member.access.clone()));
    wait_until("the banner to show", || member.awaited(server) == 1).await;
    let took = launched.elapsed();
    eprintln!("rings taken {took:?} after launch");
    assert!(took < Duration::from_secs(10), "rings are taken right after launch, not after a delay ({took:?})");
    assert_eq!(member.neighbors(server), Some(0), "no peer is back yet");
    let refreshed = member.access.take_events().into_iter().any(|(event, payload)| {
        event == "server-event" && payload == serde_json::json!(server.to_string())
    });
    assert!(refreshed, "the window is told to refresh the banner");

    // The sender comes back and dials the member, as the app's mesh healer
    // does: the message arrives and the banner goes
    sender.online().await;
    let member_node = member.node_hex();
    {
        let net = sender.app().net.lock().await;
        net.as_ref().unwrap().join_peers(server, &[member_node]).await.unwrap();
    }
    wait_until("the message to arrive", || member.message(server, message.id).is_some()).await;
    assert_eq!(member.awaited(server), 0);

    listener.abort();
    for d in devices.iter_mut() {
        d.offline().await;
    }
}

#[tokio::test]
async fn doorbell_limits_hold() {
    let origin = auth_url();
    let http = reqwest::Client::new();
    let ring = format!("{origin}/api/auth/commy/doorbell/ring");
    let take = format!("{origin}/api/auth/commy/doorbell/take");
    let random = |n: usize| hex(&CommyNode::generate_secret()[..n]);
    let post = |url: &str, ip: &str, body: serde_json::Value| {
        http.post(url).header("x-forwarded-for", ip).json(&body).send()
    };

    // Per token: at most 1000 rings are kept, the oldest go first
    let token = random(16);
    for _ in 0..21 {
        let ids: Vec<String> = (0..bell::MAX_RING_IDS).map(|_| random(12)).collect();
        let res = post(&ring, "198.51.100.1", serde_json::json!({ "token": token, "ids": ids })).await.unwrap();
        assert_eq!(res.status(), 200);
    }
    assert_eq!(sql(&format!("select count(*) from \"doorbellRing\" where token = '{token}'")), "1000");
    let res = post(&take, "198.51.100.1", serde_json::json!({ "token": token })).await.unwrap();
    let taken: serde_json::Value = res.json().await.unwrap();
    assert_eq!(taken["ids"].as_array().unwrap().len(), 1000);
    assert_eq!(sql(&format!("select count(*) from \"doorbellRing\" where token = '{token}'")), "0");

    // Malformed rings are refused
    let bad = [
        serde_json::json!({ "token": "nothex", "ids": [] }),
        serde_json::json!({ "token": random(16), "ids": ["short"] }),
        serde_json::json!({ "token": random(16), "ids": (0..51).map(|_| random(12)).collect::<Vec<_>>() }),
    ];
    for body in bad {
        assert_eq!(post(&ring, "198.51.100.2", body).await.unwrap().status(), 400);
    }
    let big = serde_json::json!({ "token": random(16), "ids": [], "pad": "x".repeat(9000) });
    assert_eq!(post(&ring, "198.51.100.2", big).await.unwrap().status(), 413);

    // Per address: 300 rings and 600 takes a minute, each address on its own
    for (url, max) in [(&ring, 300), (&take, 600)] {
        let ip = format!("203.0.113.{}", if max == 300 { 10 } else { 11 });
        for i in 0..max {
            let body = serde_json::json!({ "token": random(16), "ids": [] });
            let status = post(url, &ip, body).await.unwrap().status();
            assert_eq!(status, 200, "request {i} from one address");
        }
        let body = serde_json::json!({ "token": random(16), "ids": [] });
        assert_eq!(post(url, &ip, body.clone()).await.unwrap().status(), 429, "over the limit");
        assert_eq!(post(url, "203.0.113.99", body).await.unwrap().status(), 200, "another address is not held up");
    }
}

// ---------- 3. Edits and deletes made offline ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn offline_edits_and_deletes_arrive_in_order_signed() {
    init_logs();
    let (mut devices, server) = community(&["ada", "grace"]).await;
    let [author, reader] = &mut devices[..] else { unreachable!() };
    let author_key = author.key();

    reader.offline().await;
    wait_until("the author to have no neighbors", || author.neighbors(server) == Some(0)).await;

    let (kept, _, _) = author.send(server, "first draft").await;
    let (gone, _, _) = author.send(server, "posted by mistake").await;
    assert!(!author.edit(server, kept.id, "second draft").await);
    assert!(!author.edit(server, kept.id, "final words").await);
    assert!(!author.delete(server, gone.id).await);

    // Waiting on the author: the kept message, edited in place, and the
    // delete. Edits of a message still pending travel with it, so none can
    // reach the reader before the message it changes.
    let mut saved = author.outbox_on_disk();
    assert!(saved.is_pending(server, kept.id));
    assert!(!saved.is_pending(server, gone.id), "a deleted message is not sent");
    let queued = saved.take_queued(server);
    let kinds: Vec<String> = queued
        .iter()
        .map(|e| match e {
            ServerEvent::MessageEdit { message, content, .. } => format!("edit {message} {content}"),
            ServerEvent::MessageDelete { message, .. } => format!("delete {message}"),
            other => format!("{other:?}"),
        })
        .collect();
    eprintln!("queued while offline: {kinds:?}");
    assert_eq!(kinds.last().unwrap(), &format!("delete {}", gone.id));
    assert!(!kinds.iter().any(|k| k.starts_with("edit ")), "edits of a pending message are not queued");

    // Edited once it went out: that edit is queued and arrives after it
    reader.online().await;
    wait_until("the kept message to arrive", || reader.message(server, kept.id).is_some()).await;
    assert_eq!(reader.message(server, kept.id).unwrap().content, "final words");
    reader.offline().await;
    wait_until("the author to have no neighbors", || author.neighbors(server) == Some(0)).await;
    wait_until("the author's pending send to clear", || !author.app().outbox.lock().unwrap().has_pending(server)).await;
    assert!(!author.edit(server, kept.id, "final words, really").await);
    let queued = author.outbox_on_disk().take_queued(server);
    assert!(
        queued.iter().any(|e| matches!(e, ServerEvent::MessageEdit { message, .. } if *message == kept.id)),
        "an edit of a delivered message waits in the queue"
    );

    reader.online().await;
    wait_until("the final edit to arrive", || {
        reader.message(server, kept.id).is_some_and(|m| m.content == "final words, really")
    })
    .await;
    let want = author.message(server, kept.id).unwrap();
    let got = reader.message(server, kept.id).unwrap();
    assert_eq!(got.edited_at, want.edited_at);
    assert!(got.verify(server, &author_key), "the edited message carries the author's signature");
    wait_until("the delete to arrive", || {
        !reader.app().servers.lock().unwrap()[&server].tombstones_for(gone.id).is_empty()
    })
    .await;
    assert!(reader.message(server, gone.id).is_none());
    let proofs = reader.app().servers.lock().unwrap()[&server].tombstones_for(gone.id).into_iter().cloned().collect::<Vec<_>>();
    assert!(proofs.iter().all(|p| p.by == author.me().id && p.verify(server, &author_key)));

    // A message deleted before it went out never reaches anyone
    let channel = reader.text_channel(server);
    let reader_view: Vec<(MessageId, String)> =
        reader.app().servers.lock().unwrap()[&server].messages(channel).iter().map(|m| (m.id, m.content.clone())).collect();
    let author_view: Vec<(MessageId, String)> =
        author.app().servers.lock().unwrap()[&server].messages(channel).iter().map(|m| (m.id, m.content.clone())).collect();
    assert_eq!(reader_view, author_view);
    wait_until("the author's outbox to empty", || {
        let mut outbox = author.app().outbox.lock().unwrap().clone();
        !outbox.has_pending(server) && outbox.take_queued(server).is_empty()
    })
    .await;

    for d in devices.iter_mut() {
        d.offline().await;
    }
}

// ---------- 4. Waking a resting device ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_wake_through_the_relay_brings_a_resting_device_back() {
    init_logs();
    let relay = LocalRelay::start().await.unwrap();
    let relay_url = relay.url.to_string();
    let devices = ["ada", "grace", "linus"].map(|n| Device::on_relay(n, &relay)).into_iter().collect();
    let (mut devices, server) = gather(devices).await;
    let [sender, sleeper, other] = &mut devices[..] else { unreachable!() };
    for d in [&*sender, &*sleeper, &*other] {
        wait_until("every device to pick the local relay", || d.home_relay().as_deref() == Some(relay_url.as_str())).await;
    }
    let sleeper_id = sleeper.me().id;

    // The sleeper goes away, then rests: out of every mesh, relay link kept
    sleeper.go_away(server).await;
    wait_until("the others to see the sleeper away", || {
        sender.presence_of(server, sleeper_id) == Some(Presence::Idle)
            && other.presence_of(server, sleeper_id) == Some(Presence::Idle)
    })
    .await;
    // Catch-up sessions from joining (retried every 2 s) finish first; a
    // resting device still answers catch-up, it just starts none.
    tokio::time::sleep(Duration::from_secs(8)).await;
    assert_eq!(wake::rest(sleeper.app()).await, 1);
    assert!(wake::is_resting(sleeper.app()));
    assert_eq!(sleeper.joined(server), Some(false));
    assert_eq!(sleeper.home_relay().as_deref(), Some(relay_url.as_str()), "still on the relay");

    // Messages go out to the mesh; the resting device misses them
    let mut sent = Vec::new();
    for text in ["Chapter 3 tonight at eight", "Bring the annotated copy", "Snacks are on me"] {
        let (message, delivered, _) = sender.send(server, text).await;
        assert!(delivered, "the other member is online");
        sent.push(message);
    }
    for m in &sent {
        wait_until("the online member to get the messages", || other.message(server, m.id).is_some()).await;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(sent.iter().all(|m| sleeper.message(server, m.id).is_none()), "nothing reaches a resting device");

    // The sender wakes only devices of members who look away or offline
    assert_eq!(wake::targets(sender.app(), server), vec![sleeper.node_hex()]);
    let woken = wake::wake_members(sender.app(), server).await;
    eprintln!("wakes sent: {woken:?}");
    assert_eq!(woken, vec![(sleeper.node_hex(), true)], "one wake, and it landed");
    assert!(wake::wake_members(sender.app(), server).await.is_empty(), "one wake per device per 30 s");

    // Woken: back in the mesh, and catch-up with the sender brings every message
    for m in &sent {
        wait_until("the woken device to catch up", || sleeper.message(server, m.id).is_some()).await;
        assert!(sleeper.message(server, m.id).unwrap().verify(server, &sender.key()), "signed by its author");
    }
    assert!(!wake::is_resting(sleeper.app()));
    wait_until("the woken device to rejoin the mesh", || sleeper.joined(server) == Some(true)).await;
    wait_until("the woken device to have a neighbor", || sleeper.neighbors(server).is_some_and(|n| n > 0)).await;
    let (live, delivered, _) = sender.send(server, "See you all there").await;
    assert!(delivered);
    wait_until("live messages to flow again", || sleeper.message(server, live.id).is_some()).await;
    assert!(wake::on_wake(sleeper.app(), sender.node_bytes()).is_empty(), "an awake device ignores wakes");

    // A device that is not a member cannot wake anyone
    let mut stranger = Device::on_relay("mallory", &relay);
    stranger.online().await;
    wake::rest(sleeper.app()).await;
    assert!(wake::on_wake(sleeper.app(), stranger.node_bytes()).is_empty());
    let ring = stranger.app().net.lock().await.as_ref().unwrap().wake(&sleeper.node_hex());
    assert!(ring.await.is_ok(), "the wake itself is answered");
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(wake::is_resting(sleeper.app()), "but a stranger's wake changes nothing");
    assert_eq!(sleeper.joined(server), Some(false));

    stranger.offline().await;
    for d in devices.iter_mut() {
        d.offline().await;
    }
    relay.stop().await;
}
