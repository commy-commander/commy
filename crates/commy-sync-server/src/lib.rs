pub mod config;
pub mod fcm;
pub mod hub;
pub mod limits;
mod session;
pub mod store;
pub mod wake;

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::DefaultBodyLimit;
use axum::extract::{ConnectInfo, State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use commy_sync::wire::{MAX_CLIENT_FRAME_BYTES, PROTOCOL};

use crate::config::Config;
use crate::hub::Hub;
use crate::limits::Limiter;
use crate::store::Store;
use crate::wake::{Pusher, Wakes};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const PRUNE_EVERY: Duration = Duration::from_secs(60 * 60);
const REPORT_EVERY: Duration = Duration::from_secs(10 * 60);
const FORGET_EVERY: Duration = Duration::from_secs(60);
const WAKE_BODY_LIMIT: usize = 8 * 1024;

#[derive(Default)]
pub struct Stats {
    connections: AtomicUsize,
    opens: AtomicU64,
    appends: AtomicU64,
    pushes: AtomicU64,
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

    fn pushed(&self) {
        self.pushes.fetch_add(1, Ordering::Relaxed);
    }

    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::Relaxed)
    }

    fn take_counts(&self) -> (u64, u64, u64) {
        (self.opens.swap(0, Ordering::Relaxed), self.appends.swap(0, Ordering::Relaxed), self.pushes.swap(0, Ordering::Relaxed))
    }
}

pub struct Service {
    pub store: Store,
    pub hub: Hub,
    pub limiter: Limiter,
    pub stats: Stats,
    pub wakes: Wakes,
    pub trust_proxy: bool,
    pub retention_days: u32,
}

impl Service {
    pub fn new(store: Store, config: &Config) -> Arc<Self> {
        Self::with_pusher(store, config, Pusher::Off)
    }

    pub fn with_pusher(store: Store, config: &Config, pusher: Pusher) -> Arc<Self> {
        Arc::new(Self {
            store,
            hub: Hub::default(),
            limiter: Limiter::new(config.limits.clone()),
            stats: Stats::default(),
            wakes: Wakes::new(pusher),
            trust_proxy: config.trust_proxy,
            retention_days: config.retention_days,
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
        .route("/sync/wake", get(wake::info))
        .route("/sync/wake/register", post(wake::register).layer(DefaultBodyLimit::max(WAKE_BODY_LIMIT)))
        .route("/sync/wake/forget", post(wake::forget).layer(DefaultBodyLimit::max(WAKE_BODY_LIMIT)))
        .route("/sync/wake/ring", post(wake::ring).layer(DefaultBodyLimit::max(WAKE_BODY_LIMIT)))
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
    if service.limiter.connect(key).is_err() {
        return (StatusCode::TOO_MANY_REQUESTS, "too many connections").into_response();
    }
    ws.max_message_size(MAX_CLIENT_FRAME_BYTES)
        .max_frame_size(MAX_CLIENT_FRAME_BYTES)
        .on_failed_upgrade(move |_| {})
        .on_upgrade(move |socket| session::run(service, key, socket))
}

async fn health(State(service): State<Arc<Service>>) -> Response {
    let ok = service.store.ping().await;
    let body = serde_json::json!({ "ok": ok, "protocol": PROTOCOL });
    let status = if ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (status, Json(body)).into_response()
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
            if pruning.retention_days == 0 {
                continue;
            }
            let cutoff = now_ms() - i64::from(pruning.retention_days) * DAY_MS;
            match pruning.store.prune(cutoff).await {
                Ok(0) => {}
                Ok(removed) => tracing::info!("sync: removed {removed} entries older than {} days", pruning.retention_days),
                Err(e) => tracing::warn!("sync: pruning failed: {e}"),
            }
        }
    });
    let forgetting = service.clone();
    tokio::spawn(async move {
        let mut every = tokio::time::interval(FORGET_EVERY);
        loop {
            every.tick().await;
            forgetting.limiter.forget_idle();
            forgetting.wakes.forget_idle();
        }
    });
    tokio::spawn(async move {
        let mut every = tokio::time::interval(REPORT_EVERY);
        every.tick().await;
        loop {
            every.tick().await;
            let (opens, appends, pushes) = service.stats.take_counts();
            tracing::info!(
                "sync: {} connections, {} live streams, {opens} opens, {appends} appends and {pushes} wake-ups in the last 10 minutes",
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
