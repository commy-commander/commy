pub mod admin;
mod any_site;
pub mod apns;
pub mod certificate;
pub mod check;
pub mod config;
pub mod fcm;
pub mod gateway;
pub mod gateway_blocklist;
pub mod gateway_store;
pub mod hub;
pub mod limits;
pub mod mail;
pub mod operator;
pub mod push_via;
pub mod ramp;
mod pictures;
pub mod reports;
pub mod server_key;
pub mod server_proof;
mod session;
pub mod store;
pub mod wake;

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::DefaultBodyLimit;
use axum::middleware::map_response;
use axum::extract::{ConnectInfo, State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use commy_sync::picture::MAX_PICTURE_SEALED;
use commy_sync::wire::{Greeting, MAX_CLIENT_FRAME_BYTES, PROTOCOL};

use crate::config::Config;
use crate::hub::Hub;
use crate::limits::Limiter;
use crate::reports::Desk;
use crate::store::Store;
use crate::wake::{Pusher, Wakes};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const PRUNE_EVERY: Duration = Duration::from_secs(60 * 60);
const REPORT_EVERY: Duration = Duration::from_secs(10 * 60);
const FORGET_EVERY: Duration = Duration::from_secs(60);
const SETTLED_REPORTS_KEPT_DAYS: i64 = 180;
const WAKE_BODY_LIMIT: usize = 8 * 1024;
const SOCKET_READ_BUFFER: usize = 8 * 1024;
const SOCKET_WRITE_BUFFER: usize = 8 * 1024;
const SOCKET_MAX_WRITE_BUFFER: usize = 4 * commy_sync::wire::MAX_FRAME_BYTES;

#[derive(Default)]
pub struct Stats {
    connections: AtomicUsize,
    opens: AtomicU64,
    appends: AtomicU64,
    pushes: AtomicU64,
    pictures: AtomicU64,
    signals: AtomicU64,
}

impl Stats {
    fn connected(&self) {
        self.connections.fetch_add(1, Ordering::Relaxed);
    }

    fn disconnected(&self) {
        self.connections.fetch_sub(1, Ordering::Relaxed);
    }

    fn opened(&self) {
        self.opens.fetch_add(1, Ordering::Relaxed);
    }

    fn appended(&self) {
        self.appends.fetch_add(1, Ordering::Relaxed);
    }

    fn signalled(&self, heard: usize) {
        self.signals.fetch_add(heard as u64, Ordering::Relaxed);
    }

    fn pushed(&self) {
        self.pushes.fetch_add(1, Ordering::Relaxed);
    }

    fn stored_picture(&self) {
        self.pictures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::Relaxed)
    }

    fn take_counts(&self) -> (u64, u64, u64, u64, u64) {
        (
            self.opens.swap(0, Ordering::Relaxed),
            self.appends.swap(0, Ordering::Relaxed),
            self.pushes.swap(0, Ordering::Relaxed),
            self.pictures.swap(0, Ordering::Relaxed),
            self.signals.swap(0, Ordering::Relaxed),
        )
    }
}

pub struct Service {
    pub store: Store,
    pub hub: Hub,
    pub limiter: Limiter,
    pub stats: Stats,
    pub wakes: Wakes,
    pub desk: Desk,
    pub gateway: gateway::Gateway,
    pub trust_proxy: bool,
    pub retention_days: u32,
    pub greeting: Greeting,
    pub server_key: commy_core::Identity,
}

impl Service {
    pub fn new(store: Store, config: &Config) -> Arc<Self> {
        Self::with_pusher(store, config, Pusher::Off)
    }

    pub fn with_pusher(store: Store, config: &Config, pusher: Pusher) -> Arc<Self> {
        Self::with_keys(store, config, pusher, server_proof::KeySource::Web)
    }

    pub fn with_keys(store: Store, config: &Config, pusher: Pusher, keys: server_proof::KeySource) -> Arc<Self> {
        Arc::new(Self {
            store,
            hub: Hub::default(),
            limiter: Limiter::new(config.limits.clone()),
            stats: Stats::default(),
            wakes: Wakes::new(pusher),
            desk: Desk::new(config),
            gateway: gateway::Gateway::new(config, keys),
            trust_proxy: config.trust_proxy,
            retention_days: config.retention_days,
            greeting: Greeting {
                version: config.version.clone(),
                relay: config.relay.clone(),
                public_fallback: config.public_relay_fallback,
                operator: config.operator.clone(),
                ..Greeting::of(PROTOCOL)
            },
            server_key: config.server_key.as_ref().map_or_else(commy_core::Identity::generate, |seed| commy_core::Identity::from_bytes(&seed.0)),
        })
    }

    fn address(&self, peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
        if self.trust_proxy {
            let forwarded = headers
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit(',').next())
                .and_then(|v| v.trim().parse::<IpAddr>().ok());
            if let Some(address) = forwarded {
                return address;
            }
        }
        peer.ip()
    }
}

pub fn router(service: Arc<Service>) -> Router {
    Router::new()
        .route("/sync", get(upgrade))
        .route("/sync/healthz", get(health))
        .route("/sync/about", get(about))
        .route("/sync/wake", get(wake::info))
        .route("/sync/wake/register", post(wake::register).layer(DefaultBodyLimit::max(WAKE_BODY_LIMIT)))
        .route("/sync/wake/forget", post(wake::forget).layer(DefaultBodyLimit::max(WAKE_BODY_LIMIT)))
        .route(
            "/sync/wake/ring",
            post(wake::ring).layer(DefaultBodyLimit::max(WAKE_BODY_LIMIT)).options(any_site::preflight).layer(map_response(any_site::open_to_any_site)),
        )
        .route("/sync/wake/seat", post(wake::seat).layer(DefaultBodyLimit::max(WAKE_BODY_LIMIT)))
        .route("/sync/wake/quiet", post(wake::quiet).layer(DefaultBodyLimit::max(WAKE_BODY_LIMIT)))
        .route(gateway::GATEWAY_PATH, post(gateway::push).layer(DefaultBodyLimit::max(gateway::MAX_GATEWAY_BODY)))
        .route(
            "/sync/pictures/{id}",
            get(pictures::fetch)
                .layer(map_response(any_site::open_to_any_site))
                .merge(put(pictures::store).layer(DefaultBodyLimit::max(MAX_PICTURE_SEALED))),
        )
        .route("/healthz", get(health))
        .with_state(service)
}

async fn upgrade(
    State(service): State<Arc<Service>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let key = service.limiter.key(service.address(peer, &headers));
    let Ok(caller) = service.limiter.connect(key) else {
        return (StatusCode::TOO_MANY_REQUESTS, "too many connections").into_response();
    };
    let releasing = service.clone();
    ws.max_message_size(MAX_CLIENT_FRAME_BYTES)
        .max_frame_size(MAX_CLIENT_FRAME_BYTES)
        .read_buffer_size(SOCKET_READ_BUFFER)
        .write_buffer_size(SOCKET_WRITE_BUFFER)
        .max_write_buffer_size(SOCKET_MAX_WRITE_BUFFER)
        .on_failed_upgrade(move |_| releasing.limiter.disconnect(caller))
        .on_upgrade(move |socket| session::run(service, caller, socket))
}

async fn health(State(service): State<Arc<Service>>) -> Response {
    let ok = service.store.ping().await;
    let body = serde_json::json!({ "ok": ok, "protocol": PROTOCOL });
    let status = if ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (status, Json(body)).into_response()
}

async fn about(State(service): State<Arc<Service>>) -> Response {
    let mut body = service.desk.about();
    body["protocol"] = PROTOCOL.into();
    body["oldest_protocol"] = service.greeting.oldest_protocol.into();
    body["version"] = service.greeting.version.clone().into();
    body["relay"] = service.greeting.relay.clone().into();
    body["relay_fallback"] = if service.greeting.public_fallback { "public" } else { "pause" }.into();
    body["serverKey"] = serde_json::json!(server_key::published(&service.server_key));
    Json(body).into_response()
}

pub async fn learn_dropped(service: &Service) {
    match service.store.dropped_streams().await {
        Ok(streams) => {
            let newly = service.desk.learn_dropped(streams);
            let told: usize = newly.iter().map(|stream| service.hub.removed(stream)).sum();
            if !newly.is_empty() {
                tracing::info!("sync: {} streams are removed from this server now, told {told} open followers", newly.len());
            }
        }
        Err(e) => tracing::warn!("sync: reading removed communities failed: {e}"),
    }
}

pub fn spawn_housekeeping(service: Arc<Service>) {
    let pruning = service.clone();
    tokio::spawn(async move {
        let mut every = tokio::time::interval(PRUNE_EVERY);
        loop {
            every.tick().await;
            if let Err(e) = pruning.store.prune_wakes(commy_sync::wake::wake_day(now_ms())).await {
                tracing::warn!("sync: pruning wake registrations failed: {e}");
            }
            if let Err(e) = pruning.store.prune_quiet(now_ms()).await {
                tracing::warn!("sync: pruning quiet hours failed: {e}");
            }
            if let Err(e) = gateway::prune(&pruning).await {
                tracing::warn!("sync: pruning push gateway counts failed: {e}");
            }
            if let Err(e) = pruning.store.prune_reports(now_ms() - SETTLED_REPORTS_KEPT_DAYS * DAY_MS).await {
                tracing::warn!("sync: pruning settled reports failed: {e}");
            }
            if pruning.retention_days == 0 {
                continue;
            }
            let cutoff = now_ms() - i64::from(pruning.retention_days) * DAY_MS;
            match pruning.store.prune(cutoff).await {
                Ok(0) => {}
                Ok(removed) => tracing::info!("sync: removed {removed} entries older than {} days", pruning.retention_days),
                Err(e) => tracing::warn!("sync: pruning failed: {e}"),
            }
            match pruning.store.prune_pictures(cutoff).await {
                Ok(0) => {}
                Ok(removed) => tracing::info!("sync: removed {removed} pictures nobody used for {} days", pruning.retention_days),
                Err(e) => tracing::warn!("sync: pruning pictures failed: {e}"),
            }
        }
    });
    if service.gateway.enabled() && service.gateway.blocklist.from().is_some() {
        let refreshing = service.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(gateway_blocklist::REFRESH_EVERY);
            loop {
                every.tick().await;
                if let Err(e) = refreshing.gateway.blocklist.refresh().await {
                    tracing::warn!("push gateway: the server blocklist did not refresh, keeping the last one: {e}");
                }
            }
        });
    }
    let forgetting = service.clone();
    tokio::spawn(async move {
        let mut every = tokio::time::interval(FORGET_EVERY);
        loop {
            every.tick().await;
            learn_dropped(&forgetting).await;
            forgetting.limiter.forget_idle();
            forgetting.wakes.forget_idle();
            forgetting.gateway.forget_old(now_ms());
        }
    });
    tokio::spawn(async move {
        let mut every = tokio::time::interval(REPORT_EVERY);
        every.tick().await;
        loop {
            every.tick().await;
            let (opens, appends, pushes, pictures, signals) = service.stats.take_counts();
            tracing::info!(
                "sync: {} connections, {} live streams, {opens} opens, {appends} appends, {pictures} pictures, {signals} live signals sent and {pushes} wake-ups in the last 10 minutes",
                service.stats.connections(),
                service.hub.live_streams()
            );
        }
    });
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

pub async fn serve(listener: tokio::net::TcpListener, service: Arc<Service>) -> std::io::Result<()> {
    axum::serve(listener, router(service).into_make_service_with_connect_info::<SocketAddr>()).await
}
