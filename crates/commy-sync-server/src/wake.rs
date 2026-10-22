use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use commy_sync::wake::{push_target, register_until_ok, wake_day, WakeForget, WakeRegister, WakeRing, MAX_RING_TAGS, PUSH_GAP_SECS};
use tokio::sync::mpsc;

use crate::fcm::{Delivery, Fcm};
use crate::Service;

pub enum Pusher {
    Off,
    Fcm(Box<Fcm>),
    Recording(mpsc::UnboundedSender<String>),
}

const PUSH_GAP: Duration = Duration::from_secs(PUSH_GAP_SECS);
const RETRY_WAITS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(4)];

#[derive(Debug, PartialEq, Eq)]
enum Plan {
    Now,
    After(Duration),
    AlreadyQueued,
}

pub struct Wakes {
    pusher: Pusher,
    last: Mutex<HashMap<String, Instant>>,
    queued: Mutex<HashSet<String>>,
}

impl Wakes {
    pub fn new(pusher: Pusher) -> Self {
        Self { pusher, last: Mutex::new(HashMap::new()), queued: Mutex::new(HashSet::new()) }
    }

    pub fn enabled(&self) -> bool {
        !matches!(self.pusher, Pusher::Off)
    }

    fn plan(&self, push: &str, now: Instant) -> Plan {
        let mut last = self.last.lock().unwrap();
        match last.get(push).map(|at| now.saturating_duration_since(*at)) {
            Some(since) if since < PUSH_GAP => {
                if self.queued.lock().unwrap().insert(push.to_string()) {
                    Plan::After(PUSH_GAP - since)
                } else {
                    Plan::AlreadyQueued
                }
            }
            _ => {
                last.insert(push.to_string(), now);
                Plan::Now
            }
        }
    }

    fn release_queued(&self, push: &str, now: Instant) {
        self.queued.lock().unwrap().remove(push);
        self.last.lock().unwrap().insert(push.to_string(), now);
    }

    pub fn forget_idle(&self) {
        let now = Instant::now();
        self.last.lock().unwrap().retain(|_, at| now.saturating_duration_since(*at) < PUSH_GAP);
    }
}

async fn deliver(service: &Service, push: &str, token: &str) -> Option<Delivery> {
    match &service.wakes.pusher {
        Pusher::Off => None,
        Pusher::Fcm(fcm) => Some(fcm.wake(token).await),
        Pusher::Recording(sent) => {
            let _ = sent.send(push.to_string());
            Some(Delivery::Sent)
        }
    }
}

async fn deliver_with_retries(service: &Service, push: &str, token: &str) -> Option<Delivery> {
    let mut delivery = deliver(service, push, token).await?;
    for wait in RETRY_WAITS {
        if !matches!(delivery, Delivery::Failed(_)) {
            break;
        }
        tokio::time::sleep(wait).await;
        delivery = deliver(service, push, token).await?;
    }
    Some(delivery)
}

async fn send_after(service: Arc<Service>, push: String, wait: Duration) {
    tokio::time::sleep(wait).await;
    service.wakes.release_queued(&push, Instant::now());
    send(service, push).await;
}

async fn send(service: Arc<Service>, push: String) {
    let Some((_, token)) = push_target(&push) else {
        return;
    };
    let Some(delivery) = deliver_with_retries(&service, &push, token).await else {
        return;
    };
    match delivery {
        Delivery::Sent => service.stats.pushed(),
        Delivery::Gone => {
            if let Err(e) = service.store.forget_push(&push).await {
                tracing::warn!("sync: could not forget a stale push registration: {e}");
            }
        }
        Delivery::Failed(e) => tracing::warn!("sync: push failed: {e}"),
    }
}

fn today() -> i64 {
    wake_day(crate::now_ms())
}

fn limited(service: &Service, peer: SocketAddr, headers: &HeaderMap) -> bool {
    let key = service.limiter.key(service.address(peer, headers));
    service.limiter.wake(key).is_err()
}

pub async fn info(State(service): State<Arc<Service>>) -> Response {
    if !service.wakes.enabled() {
        return (StatusCode::NOT_FOUND, "wake-ups are off").into_response();
    }
    Json(serde_json::json!({ "push": ["fcm"], "days_ahead": commy_sync::wake::MAX_DAYS_AHEAD })).into_response()
}

pub async fn register(
    State(service): State<Arc<Service>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<WakeRegister>,
) -> Response {
    if !service.wakes.enabled() {
        return (StatusCode::NOT_FOUND, "wake-ups are off").into_response();
    }
    if limited(&service, peer, &headers) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let today = today();
    if push_target(&request.push).is_none() || !register_until_ok(request.until, today) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match service.store.register_wake(&request.tag.0, &request.push, request.until, today).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => StatusCode::CONFLICT.into_response(),
        Err(e) => {
            tracing::warn!("sync: could not keep a wake registration: {e}");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

pub async fn forget(
    State(service): State<Arc<Service>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<WakeForget>,
) -> Response {
    if !service.wakes.enabled() {
        return (StatusCode::NOT_FOUND, "wake-ups are off").into_response();
    }
    if limited(&service, peer, &headers) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    if push_target(&request.push).is_none() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match service.store.forget_wake(&request.tag.0, &request.push).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::warn!("sync: could not drop a wake registration: {e}");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

pub async fn ring(
    State(service): State<Arc<Service>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<WakeRing>,
) -> Response {
    if request.tags.is_empty() || request.tags.len() > MAX_RING_TAGS {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if !service.wakes.enabled() {
        return StatusCode::NO_CONTENT.into_response();
    }
    if limited(&service, peer, &headers) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let tags: Vec<[u8; 32]> = request.tags.iter().map(|t| t.0).collect();
    let pushes = match service.store.wake_targets(&tags, today()).await {
        Ok(pushes) => pushes,
        Err(e) => {
            tracing::warn!("sync: could not look up wake registrations: {e}");
            return StatusCode::NO_CONTENT.into_response();
        }
    };
    let now = Instant::now();
    for push in pushes {
        match service.wakes.plan(&push, now) {
            Plan::Now => {
                tokio::spawn(send(service.clone(), push));
            }
            Plan::After(wait) => {
                tokio::spawn(send_after(service.clone(), push, wait));
            }
            Plan::AlreadyQueued => {}
        }
    }
    StatusCode::NO_CONTENT.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phone_is_woken_at_most_once_per_gap() {
        let wakes = Wakes::new(Pusher::Off);
        let start = Instant::now();
        assert_eq!(wakes.plan("fcm:a", start), Plan::Now);
        assert_eq!(wakes.plan("fcm:b", start), Plan::Now, "other phones are not held back");
        assert_eq!(wakes.plan("fcm:a", start + PUSH_GAP), Plan::Now);
    }

    #[test]
    fn a_ring_inside_the_gap_is_kept_for_when_the_gap_ends() {
        let wakes = Wakes::new(Pusher::Off);
        let start = Instant::now();
        assert_eq!(wakes.plan("fcm:a", start), Plan::Now);
        let early = PUSH_GAP / 3;
        assert_eq!(wakes.plan("fcm:a", start + early), Plan::After(PUSH_GAP - early));
        assert_eq!(wakes.plan("fcm:a", start + PUSH_GAP / 2), Plan::AlreadyQueued, "one held push covers every ring in the gap");
    }

    #[test]
    fn a_held_push_starts_a_new_gap_when_it_goes_out() {
        let wakes = Wakes::new(Pusher::Off);
        let start = Instant::now();
        assert_eq!(wakes.plan("fcm:a", start), Plan::Now);
        let early = PUSH_GAP / 3;
        assert_eq!(wakes.plan("fcm:a", start + early), Plan::After(PUSH_GAP - early));
        let sent = start + PUSH_GAP;
        wakes.release_queued("fcm:a", sent);
        assert_eq!(wakes.plan("fcm:a", sent + early), Plan::After(PUSH_GAP - early));
    }
    #[tokio::test]
    async fn a_delivered_push_is_sent_once() {
        let (pushed, mut pushes) = tokio::sync::mpsc::unbounded_channel();
        let config = crate::config::Config::from_env(|_| None).unwrap();
        let service = Service::with_pusher(crate::store::Store::memory(), &config, Pusher::Recording(pushed));
        assert_eq!(deliver_with_retries(&service, "fcm:a", "a").await, Some(Delivery::Sent));
        assert_eq!(pushes.recv().await.as_deref(), Some("fcm:a"));
        assert!(pushes.try_recv().is_err());
    }
}
