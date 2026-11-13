use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use commy_sync::wake::{
    preview_ok, push_target, quiet_until_ok, register_until_ok, wake_day, PushService, WakeForget, WakeQuiet, WakeRegister, WakeRing, WakeSeat,
    MAX_RING_TAGS, PUSH_GAP_SECS, SEAT_HOLD_SECS, URGENT_GAP_SECS, URGENT_PER_HOUR,
};
use tokio::sync::mpsc;

use crate::apns::{Apns, Environment};
use crate::fcm::{Delivery, Fcm};
use crate::Service;

pub enum Pusher {
    Off,
    Live { fcm: Option<Box<Fcm>>, apns: Option<Box<Apns>> },
    Recording(mpsc::UnboundedSender<String>),
}

impl Pusher {
    pub fn live(fcm: Option<Fcm>, apns: Option<Apns>) -> Self {
        if fcm.is_none() && apns.is_none() {
            return Pusher::Off;
        }
        Pusher::Live { fcm: fcm.map(Box::new), apns: apns.map(Box::new) }
    }

    fn offers(&self, service: PushService) -> bool {
        match self {
            Pusher::Off => false,
            Pusher::Live { fcm, apns } => match service {
                PushService::Fcm => fcm.is_some(),
                PushService::Apns | PushService::ApnsSandbox => apns.is_some(),
            },
            Pusher::Recording(_) => true,
        }
    }

    fn services(&self) -> Vec<&'static str> {
        [PushService::Fcm, PushService::Apns, PushService::ApnsSandbox].into_iter().filter(|s| self.offers(*s)).map(PushService::prefix).collect()
    }
}

const PUSH_GAP: Duration = Duration::from_secs(PUSH_GAP_SECS);
const SEAT_HOLD: Duration = Duration::from_secs(SEAT_HOLD_SECS);
const RETRY_WAITS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(4)];
const URGENT_GAP: Duration = Duration::from_secs(URGENT_GAP_SECS);
const URGENT_WINDOW: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, PartialEq, Eq)]
enum Plan {
    Now,
    After(Duration),
    AlreadyQueued,
}

const MAX_HELD_PREVIEWS: usize = 4;

pub struct Wakes {
    pusher: Pusher,
    last: Mutex<HashMap<String, Instant>>,
    queued: Mutex<HashMap<String, Vec<String>>>,
    seated: Mutex<HashMap<String, Instant>>,
    urgent: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl Wakes {
    pub fn new(pusher: Pusher) -> Self {
        Self {
            pusher,
            last: Mutex::new(HashMap::new()),
            queued: Mutex::new(HashMap::new()),
            seated: Mutex::new(HashMap::new()),
            urgent: Mutex::new(HashMap::new()),
        }
    }

    fn take_urgent(&self, push: &str, now: Instant) -> bool {
        let mut urgent = self.urgent.lock().unwrap();
        let sent = urgent.entry(push.to_string()).or_default();
        sent.retain(|at| now.saturating_duration_since(*at) < URGENT_WINDOW);
        let too_soon = sent.back().is_some_and(|at| now.saturating_duration_since(*at) < URGENT_GAP);
        if too_soon || sent.len() >= URGENT_PER_HOUR {
            return false;
        }
        sent.push_back(now);
        true
    }

    fn seat(&self, pushes: impl IntoIterator<Item = String>, now: Instant) {
        let mut seated = self.seated.lock().unwrap();
        for push in pushes {
            seated.insert(push, now + SEAT_HOLD);
        }
    }

    fn seated_elsewhere(&self, push: &str, now: Instant) -> bool {
        self.seated.lock().unwrap().get(push).is_some_and(|until| now < *until)
    }

    pub fn enabled(&self) -> bool {
        !matches!(self.pusher, Pusher::Off)
    }

    fn offers(&self, push: &str) -> bool {
        push_target(push).is_some_and(|(service, _)| self.pusher.offers(service))
    }

    fn plan(&self, push: &str, now: Instant, preview: Option<&str>) -> Plan {
        let mut last = self.last.lock().unwrap();
        match last.get(push).map(|at| now.saturating_duration_since(*at)) {
            Some(since) if since < PUSH_GAP => {
                let mut queued = self.queued.lock().unwrap();
                let fresh = !queued.contains_key(push);
                let held = queued.entry(push.to_string()).or_default();
                if let Some(preview) = preview {
                    held.push(preview.to_string());
                    if held.len() > MAX_HELD_PREVIEWS {
                        held.remove(0);
                    }
                }
                if fresh {
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

    fn release_queued(&self, push: &str, now: Instant) -> Vec<String> {
        let previews = self.queued.lock().unwrap().remove(push).unwrap_or_default();
        self.last.lock().unwrap().insert(push.to_string(), now);
        previews
    }

    pub fn forget_idle(&self) {
        let now = Instant::now();
        self.last.lock().unwrap().retain(|_, at| now.saturating_duration_since(*at) < PUSH_GAP);
        self.seated.lock().unwrap().retain(|_, until| now < *until);
        let mut urgent = self.urgent.lock().unwrap();
        urgent.values_mut().for_each(|sent| sent.retain(|at| now.saturating_duration_since(*at) < URGENT_WINDOW));
        urgent.retain(|_, sent| !sent.is_empty());
    }
}

async fn quiet_now(service: &Service, pushes: &[String]) -> HashSet<String> {
    match service.store.quiet_pushes(pushes, crate::now_ms()).await {
        Ok(quiet) => quiet.into_iter().collect(),
        Err(e) => {
            tracing::warn!("sync: could not look up quiet hours: {e}");
            HashSet::new()
        }
    }
}

async fn deliver(service: &Service, push: &str, previews: &[String]) -> Option<Delivery> {
    let (kind, token) = push_target(push)?;
    match (&service.wakes.pusher, kind) {
        (Pusher::Off, _) => None,
        (Pusher::Live { fcm: Some(fcm), .. }, PushService::Fcm) => Some(fcm.wake(token, previews).await),
        (Pusher::Live { apns: Some(apns), .. }, PushService::Apns) => Some(apns.wake(Environment::Production, token, previews).await),
        (Pusher::Live { apns: Some(apns), .. }, PushService::ApnsSandbox) => Some(apns.wake(Environment::Sandbox, token, previews).await),
        (Pusher::Live { .. }, _) => None,
        (Pusher::Recording(sent), _) => {
            let _ = sent.send(push.to_string());
            Some(Delivery::Sent)
        }
    }
}

async fn deliver_with_retries(service: &Service, push: &str, previews: &[String]) -> Option<Delivery> {
    let mut delivery = deliver(service, push, previews).await?;
    for wait in RETRY_WAITS {
        if !matches!(delivery, Delivery::Failed(_)) {
            break;
        }
        tokio::time::sleep(wait).await;
        delivery = deliver(service, push, previews).await?;
    }
    Some(delivery)
}

async fn send_after(service: Arc<Service>, push: String, wait: Duration) {
    tokio::time::sleep(wait).await;
    let now = Instant::now();
    let previews = service.wakes.release_queued(&push, now);
    if service.wakes.seated_elsewhere(&push, now) || !quiet_now(&service, std::slice::from_ref(&push)).await.is_empty() {
        return;
    }
    send(service, push, previews).await;
}

async fn send(service: Arc<Service>, push: String, previews: Vec<String>) {
    let Some(delivery) = deliver_with_retries(&service, &push, &previews).await else {
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
    Json(serde_json::json!({ "push": service.wakes.pusher.services(), "days_ahead": commy_sync::wake::MAX_DAYS_AHEAD })).into_response()
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
    if !service.wakes.offers(&request.push) {
        return (StatusCode::NOT_FOUND, "wake-ups are off for this kind of phone").into_response();
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

pub async fn seat(
    State(service): State<Arc<Service>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<WakeSeat>,
) -> Response {
    if !service.wakes.enabled() {
        return (StatusCode::NOT_FOUND, "wake-ups are off").into_response();
    }
    if limited(&service, peer, &headers) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    match service.store.wake_targets(&[request.seat.0], today()).await {
        Ok(pushes) => {
            service.wakes.seat(pushes, Instant::now());
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            tracing::warn!("sync: could not look up wake registrations: {e}");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

pub async fn quiet(
    State(service): State<Arc<Service>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<WakeQuiet>,
) -> Response {
    if !service.wakes.enabled() {
        return (StatusCode::NOT_FOUND, "wake-ups are off").into_response();
    }
    if limited(&service, peer, &headers) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let until = quiet_until_ok(request.until_ms, crate::now_ms());
    let kept = match service.store.wake_targets(&[request.seat.0], today()).await {
        Ok(pushes) if pushes.is_empty() => Ok(()),
        Ok(pushes) => service.store.set_quiet(&pushes, until).await,
        Err(e) => Err(e),
    };
    match kept {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::warn!("sync: could not keep quiet hours: {e}");
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
    if request.tags.is_empty() || request.tags.len() > MAX_RING_TAGS || !request.urgent_ok() || request.preview.as_deref().is_some_and(|p| !preview_ok(p)) {
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
    let pushes: Vec<String> = pushes.into_iter().filter(|push| !service.wakes.seated_elsewhere(push, now)).collect();
    let quiet = quiet_now(&service, &pushes).await;
    for push in pushes {
        if quiet.contains(&push) {
            if request.urgent && service.wakes.take_urgent(&push, now) {
                tokio::spawn(send(service.clone(), push, request.preview.iter().cloned().collect()));
            }
            continue;
        }
        match service.wakes.plan(&push, now, request.preview.as_deref()) {
            Plan::Now => {
                tokio::spawn(send(service.clone(), push, request.preview.iter().cloned().collect()));
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
    fn notify_anyway_reaches_a_quiet_phone_a_few_times_an_hour_at_most() {
        let wakes = Wakes::new(Pusher::Off);
        let start = Instant::now();
        assert!(wakes.take_urgent("fcm:a", start));
        assert!(!wakes.take_urgent("fcm:a", start + URGENT_GAP / 2), "one at a time");
        assert!(wakes.take_urgent("fcm:b", start), "each phone counts on its own");
        let mut at = start;
        for _ in 1..URGENT_PER_HOUR {
            at += URGENT_GAP;
            assert!(wakes.take_urgent("fcm:a", at));
        }
        assert!(!wakes.take_urgent("fcm:a", at + URGENT_GAP), "the hour is used up");
        assert!(wakes.take_urgent("fcm:a", start + URGENT_WINDOW + URGENT_GAP), "and refills as the hour moves on");
    }

    #[test]
    fn a_phone_is_woken_at_most_once_per_gap() {
        let wakes = Wakes::new(Pusher::Off);
        let start = Instant::now();
        assert_eq!(wakes.plan("fcm:a", start, None), Plan::Now);
        assert_eq!(wakes.plan("fcm:b", start, None), Plan::Now, "other phones are not held back");
        assert_eq!(wakes.plan("fcm:a", start + PUSH_GAP, None), Plan::Now);
    }

    #[test]
    fn a_ring_inside_the_gap_is_kept_for_when_the_gap_ends() {
        let wakes = Wakes::new(Pusher::Off);
        let start = Instant::now();
        assert_eq!(wakes.plan("fcm:a", start, None), Plan::Now);
        let early = PUSH_GAP / 3;
        assert_eq!(wakes.plan("fcm:a", start + early, None), Plan::After(PUSH_GAP - early));
        assert_eq!(wakes.plan("fcm:a", start + PUSH_GAP / 2, None), Plan::AlreadyQueued, "one held push covers every ring in the gap");
    }

    #[test]
    fn a_held_push_starts_a_new_gap_when_it_goes_out() {
        let wakes = Wakes::new(Pusher::Off);
        let start = Instant::now();
        assert_eq!(wakes.plan("fcm:a", start, None), Plan::Now);
        let early = PUSH_GAP / 3;
        assert_eq!(wakes.plan("fcm:a", start + early, None), Plan::After(PUSH_GAP - early));
        let sent = start + PUSH_GAP;
        wakes.release_queued("fcm:a", sent);
        assert_eq!(wakes.plan("fcm:a", sent + early, None), Plan::After(PUSH_GAP - early));
    }
    #[test]
    fn a_held_push_carries_every_preview_from_its_gap() {
        let wakes = Wakes::new(Pusher::Off);
        let start = Instant::now();
        assert_eq!(wakes.plan("fcm:a", start, Some("first")), Plan::Now);
        let early = PUSH_GAP / 3;
        assert_eq!(wakes.plan("fcm:a", start + early, Some("second")), Plan::After(PUSH_GAP - early));
        assert_eq!(wakes.plan("fcm:a", start + early, Some("third")), Plan::AlreadyQueued);
        assert_eq!(wakes.plan("fcm:a", start + early, None), Plan::AlreadyQueued, "a ring without a preview adds nothing");
        assert_eq!(wakes.release_queued("fcm:a", start + PUSH_GAP), ["second", "third"]);
        assert!(wakes.release_queued("fcm:b", start + PUSH_GAP).is_empty());
    }

    #[test]
    fn a_held_push_keeps_only_the_newest_previews() {
        let wakes = Wakes::new(Pusher::Off);
        let start = Instant::now();
        assert_eq!(wakes.plan("fcm:a", start, None), Plan::Now);
        let early = PUSH_GAP / 3;
        for n in 0..MAX_HELD_PREVIEWS + 2 {
            wakes.plan("fcm:a", start + early, Some(&n.to_string()));
        }
        let held = wakes.release_queued("fcm:a", start + PUSH_GAP);
        let newest: Vec<String> = (2..MAX_HELD_PREVIEWS + 2).map(|n| n.to_string()).collect();
        assert_eq!(held, newest);
    }

    #[test]
    fn a_phone_stays_quiet_while_its_person_sits_at_another_device() {
        let wakes = Wakes::new(Pusher::Off);
        let start = Instant::now();
        assert!(!wakes.seated_elsewhere("fcm:a", start));
        wakes.seat(["fcm:a".to_string()], start);
        assert!(wakes.seated_elsewhere("fcm:a", start + SEAT_HOLD / 2));
        assert!(!wakes.seated_elsewhere("fcm:b", start), "other phones still ring");
        assert!(!wakes.seated_elsewhere("fcm:a", start + SEAT_HOLD), "the phone rings again once the beats stop");
        wakes.seat(["fcm:a".to_string()], start + SEAT_HOLD / 2);
        assert!(wakes.seated_elsewhere("fcm:a", start + SEAT_HOLD), "each beat holds it longer");
    }

    #[test]
    fn a_service_only_takes_the_kinds_of_phones_it_can_reach() {
        assert!(matches!(Pusher::live(None, None), Pusher::Off));
        let (pushed, _pushes) = tokio::sync::mpsc::unbounded_channel();
        let wakes = Wakes::new(Pusher::Recording(pushed));
        assert!(wakes.offers("fcm:a") && wakes.offers("apns:ab") && wakes.offers("apns-dev:ab"));
        assert!(!wakes.offers("apns:not-hex"));
        let fcm_only = Pusher::Live { fcm: None, apns: None };
        assert!(!fcm_only.offers(PushService::Apns));
        assert_eq!(fcm_only.services(), Vec::<&str>::new());
    }

    #[tokio::test]
    async fn a_delivered_push_is_sent_once() {
        let (pushed, mut pushes) = tokio::sync::mpsc::unbounded_channel();
        let config = crate::config::Config::from_env(|_| None).unwrap();
        let service = Service::with_pusher(crate::store::Store::memory(), &config, Pusher::Recording(pushed));
        assert_eq!(deliver_with_retries(&service, "fcm:a", &[]).await, Some(Delivery::Sent));
        assert_eq!(pushes.recv().await.as_deref(), Some("fcm:a"));
        assert!(pushes.try_recv().is_err());
    }
}
