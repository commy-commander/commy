use commy_core::lock_or_recover::LockOrRecover;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use commy_sync::wake::{preview_ok, push_target, wake_day};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{Config, Contact, GatewaySettings};
use crate::gateway_blocklist::Blocklist;
use crate::mail::Mailer;
use crate::ramp::{floods, judge, left, Caps, Day, ServerRecord, Tier, Verdict};
use crate::server_proof::{clean_origin, push_payload, KeyBook, KeySource};
use crate::wake::{dispatch, offers_push, MAX_HELD_PREVIEWS};
use crate::Service;

pub const GATEWAY_PATH: &str = "/sync/gateway/push";
pub const MAX_GATEWAY_PUSHES: usize = 50;
pub const MAX_GATEWAY_BODY: usize = 64 * 1024;
pub const CLOCK_SKEW_MS: i64 = 5 * 60 * 1000;
pub const ORIGIN_HEADER: &str = "x-commy-origin";
pub const AT_HEADER: &str = "x-commy-at";
pub const SIGNATURE_HEADER: &str = "x-commy-signature";
pub const KEPT_DAYS: i64 = 90;
const FLOOD_REASON: &str = "sent far more pings than its daily cap allows";

#[derive(Debug, Deserialize)]
pub struct PushRequest {
    pub pushes: Vec<GatewayPush>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayPush {
    pub to: String,
    #[serde(default)]
    pub previews: Vec<String>,
    #[serde(default)]
    pub urgent: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PushAnswer {
    pub tier: Tier,
    pub sent: u64,
    pub refused: u64,
    pub left: Caps,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerRow {
    pub record: ServerRecord,
    pub tier: Tier,
    pub caps: Caps,
    pub today: Day,
}

#[derive(Debug, Clone, Serialize)]
pub struct Overview {
    pub enabled: bool,
    pub budget: u64,
    pub alert_percent: u64,
    pub used: u64,
    pub refused: u64,
    pub servers: Vec<ServerRow>,
    pub blocklist_from: Option<String>,
    pub blocked: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum BudgetLevel {
    Fine,
    Warn,
    Spent,
}

pub struct Alerts {
    webhook: Option<String>,
    mailer: Option<Mailer>,
}

impl Alerts {
    fn new(config: &Config) -> Self {
        let mailer = match (&config.abuse_contact, &config.smtp) {
            (Some(Contact::Email(to)), Some(smtp)) => Mailer::new(smtp, to).ok(),
            _ => None,
        };
        let webhook = config.gateway.as_ref().and_then(|g| g.alert_webhook.clone());
        Self { webhook, mailer }
    }

    fn raise(&self, subject: String, text: String) {
        tracing::warn!("push gateway: {text}");
        let webhook = self.webhook.clone();
        let mailer = self.mailer.clone();
        tokio::spawn(async move {
            if let Some(url) = webhook {
                let body = serde_json::json!({ "content": text, "text": text });
                if let Err(e) = reqwest::Client::new().post(url).json(&body).timeout(std::time::Duration::from_secs(10)).send().await {
                    tracing::warn!("push gateway: alert webhook failed: {e}");
                }
            }
            if let Some(mailer) = mailer {
                if let Err(e) = mailer.send_text(&subject, &text).await {
                    tracing::warn!("push gateway: alert email failed: {e}");
                }
            }
        });
    }
}

pub struct Gateway {
    settings: Option<GatewaySettings>,
    keys: KeyBook,
    seen: Mutex<HashMap<String, i64>>,
    counting: tokio::sync::Mutex<()>,
    alerted: Mutex<(i64, BudgetLevel)>,
    alerts: Alerts,
    pub blocklist: Blocklist,
}

impl Gateway {
    pub fn new(config: &Config, source: KeySource) -> Self {
        Self {
            settings: config.gateway.clone(),
            keys: KeyBook::new(source),
            seen: Mutex::new(HashMap::new()),
            counting: tokio::sync::Mutex::new(()),
            alerted: Mutex::new((0, BudgetLevel::Fine)),
            alerts: Alerts::new(config),
            blocklist: Blocklist::new(config.gateway.as_ref().and_then(|g| g.blocklist.clone())),
        }
    }

    pub fn enabled(&self) -> bool {
        self.settings.is_some()
    }

    fn first_use_of(&self, signature: &str, now_ms: i64) -> bool {
        let mut seen = self.seen.lock_or_recover();
        seen.retain(|_, at| now_ms.saturating_sub(*at) < 2 * CLOCK_SKEW_MS);
        seen.insert(signature.to_ascii_lowercase(), now_ms).is_none()
    }

    pub fn forget_old(&self, now_ms: i64) {
        self.keys.forget_old(now_ms);
        self.seen.lock_or_recover().retain(|_, at| now_ms.saturating_sub(*at) < 2 * CLOCK_SKEW_MS);
    }

    fn budget_level(budget: u64, alert_percent: u64, used: u64) -> BudgetLevel {
        if used >= budget {
            BudgetLevel::Spent
        } else if used.saturating_mul(100) >= budget.saturating_mul(alert_percent) {
            BudgetLevel::Warn
        } else {
            BudgetLevel::Fine
        }
    }

    fn watch_budget(&self, day: i64, used: u64) {
        let Some(settings) = &self.settings else { return };
        let level = Self::budget_level(settings.daily_budget, settings.alert_percent, used);
        let mut alerted = self.alerted.lock_or_recover();
        if alerted.0 != day {
            *alerted = (day, BudgetLevel::Fine);
        }
        if level <= alerted.1 {
            return;
        }
        alerted.1 = level;
        let budget = settings.daily_budget;
        let (subject, text) = match level {
            BudgetLevel::Spent => (
                "Commy push gateway: daily budget used up".to_string(),
                format!("The push gateway sent {used} of its {budget} pings for today and refuses the rest until midnight UTC."),
            ),
            _ => (
                "Commy push gateway: budget getting close".to_string(),
                format!("The push gateway sent {used} of its {budget} pings for today ({}%).", used * 100 / budget.max(1)),
            ),
        };
        self.alerts.raise(subject, text);
    }
}

fn answer(status: StatusCode, error: &str) -> Response {
    (status, Json(serde_json::json!({ "error": error }))).into_response()
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim)
}

pub fn phone_key(push: &str) -> [u8; 32] {
    Sha256::digest(push.as_bytes()).into()
}

fn valid(push: &GatewayPush) -> bool {
    push_target(&push.to).is_some() && push.previews.len() <= MAX_HELD_PREVIEWS && push.previews.iter().all(|p| preview_ok(p))
}

pub async fn push(State(service): State<Arc<Service>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, body: Bytes) -> Response {
    let gateway = &service.gateway;
    if !gateway.enabled() {
        return answer(StatusCode::NOT_FOUND, "the push gateway is off");
    }
    let key = service.limiter.key(service.address(peer, &headers));
    if service.limiter.wake(key).is_err() {
        return answer(StatusCode::TOO_MANY_REQUESTS, "slow_down");
    }
    let Some(origin) = header(&headers, ORIGIN_HEADER).and_then(clean_origin) else {
        return answer(StatusCode::BAD_REQUEST, "bad_origin");
    };
    if gateway.blocklist.blocks(&origin) {
        return answer(StatusCode::FORBIDDEN, "blocked");
    }
    let now = crate::now_ms();
    let Some(at) = header(&headers, AT_HEADER).and_then(|v| v.parse::<i64>().ok()).filter(|at| (now - at).abs() <= CLOCK_SKEW_MS) else {
        return answer(StatusCode::UNAUTHORIZED, "stale");
    };
    let Some(signature) = header(&headers, SIGNATURE_HEADER).filter(|s| s.len() == 128) else {
        return answer(StatusCode::UNAUTHORIZED, "bad_signature");
    };
    let record = match service.store.gateway_server(&origin).await {
        Ok(record) => record,
        Err(e) => {
            tracing::warn!("push gateway: reading {origin} failed: {e}");
            return answer(StatusCode::SERVICE_UNAVAILABLE, "try_later");
        }
    };
    if let Some(cut) = record.as_ref().and_then(|r| r.cut_off.as_ref()) {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "cut_off", "reason": cut.reason }))).into_response();
    }
    if let Err(e) = gateway.keys.check(&origin, &push_payload(at, &body), signature, now).await {
        tracing::info!("push gateway: refused {origin}: {e}");
        return answer(StatusCode::UNAUTHORIZED, "bad_signature");
    }
    if !gateway.first_use_of(signature, now) {
        return answer(StatusCode::CONFLICT, "replayed");
    }
    let Ok(request) = serde_json::from_slice::<PushRequest>(&body) else {
        return answer(StatusCode::BAD_REQUEST, "bad_body");
    };
    if request.pushes.is_empty() || request.pushes.len() > MAX_GATEWAY_PUSHES || !request.pushes.iter().all(valid) {
        return answer(StatusCode::BAD_REQUEST, "bad_pushes");
    }
    let mut record = record.unwrap_or_else(|| ServerRecord::first(&origin, now));
    match count(&service, &mut record, request.pushes, now).await {
        Ok((answer, accepted)) => {
            for push in accepted {
                dispatch(&service, vec![push.to], &push.previews, push.urgent).await;
            }
            let status = if answer.sent == 0 && answer.refused > 0 { StatusCode::TOO_MANY_REQUESTS } else { StatusCode::OK };
            (status, Json(answer)).into_response()
        }
        Err(e) => {
            tracing::warn!("push gateway: counting for {origin} failed: {e}");
            answer(StatusCode::SERVICE_UNAVAILABLE, "try_later")
        }
    }
}

async fn count(service: &Service, record: &mut ServerRecord, pushes: Vec<GatewayPush>, now: i64) -> Result<(PushAnswer, Vec<GatewayPush>), crate::store::StoreError> {
    let gateway = &service.gateway;
    let budget = gateway.settings.as_ref().map_or(0, |s| s.daily_budget);
    let _one_at_a_time = gateway.counting.lock().await;
    let day = wake_day(now);
    let origin = record.origin.clone();
    if service.store.gateway_server(&origin).await?.is_none() {
        service.store.put_gateway_server(record).await?;
    }
    let mut today = service.store.gateway_day(&origin, day).await?;
    let mut used: u64 = service.store.gateway_days(day).await?.iter().map(|(_, d)| d.pushes).sum();
    let tier = record.tier(now);
    let caps = tier.caps();
    let mut added = Day::default();
    let mut accepted = Vec::new();
    for push in pushes {
        let phone = phone_key(&push.to);
        let known = service.store.known_phone(&origin, &phone, day).await?;
        let offered = offers_push(service, &push.to);
        match judge(caps, &today, !known, budget.saturating_sub(used)) {
            Verdict::Send if offered => {
                today.pushes += 1;
                added.pushes += 1;
                used += 1;
                if !known {
                    today.new_phones += 1;
                    added.new_phones += 1;
                    service.store.remember_phone(&origin, &phone, day).await?;
                }
                accepted.push(push);
            }
            _ => {
                today.refused += 1;
                added.refused += 1;
            }
        }
    }
    service.store.count_gateway(&origin, day, added).await?;
    if floods(caps, &today) && record.cut_off.is_none() {
        record.cut(now, FLOOD_REASON, true);
        service.store.put_gateway_server(record).await?;
        gateway.alerts.raise(
            format!("Commy push gateway: cut off {origin}"),
            format!("{origin} was cut off: it {FLOOD_REASON} ({} refused today, cap {}). Restore it with `commy-sync gateway restore {origin}`.", today.refused, caps.pushes),
        );
    }
    gateway.watch_budget(day, used);
    Ok((PushAnswer { tier, sent: added.pushes, refused: added.refused, left: left(caps, &today) }, accepted))
}

pub async fn overview(service: &Service) -> Result<Overview, crate::store::StoreError> {
    let now = crate::now_ms();
    let day = wake_day(now);
    let days: HashMap<String, Day> = service.store.gateway_days(day).await?.into_iter().collect();
    let mut servers: Vec<ServerRow> = service
        .store
        .gateway_servers()
        .await?
        .into_iter()
        .map(|record| {
            let tier = record.tier(now);
            let today = days.get(&record.origin).copied().unwrap_or_default();
            ServerRow { tier, caps: tier.caps(), today, record }
        })
        .collect();
    servers.sort_by(|a, b| b.today.pushes.cmp(&a.today.pushes).then_with(|| a.record.origin.cmp(&b.record.origin)));
    Ok(Overview {
        enabled: service.gateway.enabled(),
        budget: service.gateway.settings.as_ref().map_or(0, |s| s.daily_budget),
        alert_percent: service.gateway.settings.as_ref().map_or(100, |s| s.alert_percent),
        used: days.values().map(|d| d.pushes).sum(),
        refused: days.values().map(|d| d.refused).sum(),
        servers,
        blocklist_from: service.gateway.blocklist.from().map(str::to_string),
        blocked: service.gateway.blocklist.blocked_count(),
    })
}

pub async fn prune(service: &Service) -> Result<(), crate::store::StoreError> {
    service.store.prune_gateway(wake_day(crate::now_ms()) - KEPT_DAYS).await
}
