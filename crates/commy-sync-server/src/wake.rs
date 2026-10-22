use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use commy_sync::wake::{push_target, register_until_ok, wake_day, WakeRegister, WakeRing, MAX_RING_TAGS, PUSH_GAP_SECS};
use tokio::sync::mpsc;

use crate::fcm::{Delivery, Fcm};
use crate::Service;

pub enum Pusher {
    Off,
    Fcm(Box<Fcm>),
    Recording(mpsc::UnboundedSender<String>),
}

pub struct Wakes {
    pusher: Pusher,
    last: Mutex<HashMap<String, Instant>>,
}

impl Wakes {
    pub fn new(pusher: Pusher) -> Self {
        Self { pusher, last: Mutex::new(HashMap::new()) }
    }

    pub fn enabled(&self) -> bool {
        !matches!(self.pusher, Pusher::Off)
    }

    fn due(&self, push: &str, now: Instant) -> bool {
        let mut last = self.last.lock().unwrap();
        match last.get(push) {
            Some(at) if now.saturating_duration_since(*at) < Duration::from_secs(PUSH_GAP_SECS) => false,
            _ => {
                last.insert(push.to_string(), now);
                true
            }
        }
    }

    pub fn forget_idle(&self) {
        let now = Instant::now();
        self.last.lock().unwrap().retain(|_, at| now.saturating_duration_since(*at) < Duration::from_secs(PUSH_GAP_SECS));
    }
}

async fn send(service: Arc<Service>, push: String) {
    let Some((_, token)) = push_target(&push) else {
        return;
    };
    let delivery = match &service.wakes.pusher {
        Pusher::Off => return,
        Pusher::Fcm(fcm) => fcm.wake(token).await,
        Pusher::Recording(sent) => {
            let _ = sent.send(push.clone());
            Delivery::Sent
        }
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
    for push in pushes.into_iter().filter(|push| service.wakes.due(push, now)) {
        tokio::spawn(send(service.clone(), push));
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
        assert!(wakes.due("fcm:a", start));
        assert!(!wakes.due("fcm:a", start + Duration::from_secs(PUSH_GAP_SECS - 1)));
        assert!(wakes.due("fcm:b", start), "other phones are not held back");
        assert!(wakes.due("fcm:a", start + Duration::from_secs(PUSH_GAP_SECS)));
    }
}
