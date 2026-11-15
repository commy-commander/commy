use std::sync::Mutex;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use commy_core::Identity;
use serde_json::Value;

use crate::fcm::Delivery;
use crate::gateway::{AT_HEADER, GATEWAY_PATH, ORIGIN_HEADER, SIGNATURE_HEADER};
use crate::server_proof::{push_payload, PUSH_PURPOSE};

pub const REFUSED_PAUSE: Duration = Duration::from_secs(10 * 60);
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

pub struct ViaGateway {
    http: reqwest::Client,
    endpoint: String,
    origin: String,
    key: Identity,
    paused_until: Mutex<Option<Instant>>,
}

impl ViaGateway {
    pub fn new(gateway: &str, origin: &str, key: Identity) -> Self {
        let http = reqwest::Client::builder().timeout(SEND_TIMEOUT).redirect(reqwest::redirect::Policy::none()).build().unwrap_or_default();
        Self { http, endpoint: format!("{}{GATEWAY_PATH}", gateway.trim_end_matches('/')), origin: origin.to_string(), key, paused_until: Mutex::new(None) }
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    fn paused(&self, now: Instant) -> bool {
        self.paused_until.lock().unwrap().is_some_and(|until| now < until)
    }

    fn pause(&self, now: Instant) {
        *self.paused_until.lock().unwrap() = Some(now + REFUSED_PAUSE);
    }

    pub async fn wake(&self, push: &str, previews: &[String]) -> Delivery {
        if self.paused(Instant::now()) {
            return Delivery::Refused("the push gateway refused this server a moment ago".into());
        }
        let body = match serde_json::to_vec(&serde_json::json!({ "pushes": [{ "to": push, "previews": previews }] })) {
            Ok(body) => body,
            Err(e) => return Delivery::Failed(e.to_string()),
        };
        let at = crate::now_ms();
        let signature = commy_core::server_key::sign(&self.key, PUSH_PURPOSE, &self.origin, push_payload(at, &body).as_bytes());
        let sent = self
            .http
            .post(&self.endpoint)
            .header(ORIGIN_HEADER, &self.origin)
            .header(AT_HEADER, at.to_string())
            .header(SIGNATURE_HEADER, signature.to_hex())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await;
        let response = match sent {
            Ok(response) => response,
            Err(e) => return Delivery::Failed(format!("the push gateway did not answer: {e}")),
        };
        let status = response.status();
        let answer: Value = response.json().await.unwrap_or(Value::Null);
        let error = answer["error"].as_str().unwrap_or("").to_string();
        match status {
            StatusCode::OK if answer["sent"].as_u64().unwrap_or(0) > 0 => Delivery::Sent,
            StatusCode::OK => Delivery::Refused("the push gateway carried nothing".into()),
            StatusCode::CONFLICT => Delivery::Failed("the push gateway saw this request twice".into()),
            s if s.is_server_error() => Delivery::Failed(format!("the push gateway answered {s}")),
            StatusCode::TOO_MANY_REQUESTS | StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED | StatusCode::NOT_FOUND => {
                self.pause(Instant::now());
                let reason = answer["reason"].as_str().map(|r| format!(": {r}")).unwrap_or_default();
                Delivery::Refused(format!("the push gateway refused this server ({status} {error}{reason})"))
            }
            s => Delivery::Refused(format!("the push gateway answered {s} {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_refusal_pauses_the_gateway_for_a_while() {
        let key = Identity::from_bytes(&[3; 32]);
        let via = ViaGateway::new("http://127.0.0.1:9/", "https://sync.example.org", key);
        assert_eq!(via.endpoint, format!("http://127.0.0.1:9{GATEWAY_PATH}"));
        let now = Instant::now();
        assert!(!via.paused(now));
        via.pause(now);
        assert!(via.paused(now + REFUSED_PAUSE / 2));
        assert!(!via.paused(now + REFUSED_PAUSE));
        assert!(matches!(via.wake("fcm:a", &[]).await, Delivery::Refused(_)), "no request while paused");
    }
}
