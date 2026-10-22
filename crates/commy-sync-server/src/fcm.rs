use std::sync::Mutex;
use std::time::{Duration, Instant};

use data_encoding::{BASE64, BASE64URL_NOPAD};
use ring::rand::SystemRandom;
use ring::signature::{RsaKeyPair, RSA_PKCS1_SHA256};
use serde::Deserialize;

const SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
const ASSERTION_SECS: u64 = 3600;
const RENEW_BEFORE: Duration = Duration::from_secs(300);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const WAKE_TTL: &str = "3600s";
const WAKE_COLLAPSE_KEY: &str = "wake";

#[derive(Deserialize)]
struct ServiceAccount {
    project_id: String,
    client_email: String,
    private_key: String,
    token_uri: String,
}

#[derive(Deserialize)]
struct Grant {
    access_token: String,
    expires_in: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    Sent,
    Gone,
    Failed(String),
}

pub struct Fcm {
    http: reqwest::Client,
    send_url: String,
    email: String,
    token_uri: String,
    key: RsaKeyPair,
    access: Mutex<Option<(String, Instant)>>,
}

impl Fcm {
    pub fn from_config(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let json = if text.starts_with('{') {
            text.to_string()
        } else {
            let decoded = BASE64.decode(text.as_bytes()).map_err(|_| "FCM_SERVICE_ACCOUNT must be the service account JSON or its base64".to_string())?;
            String::from_utf8(decoded).map_err(|_| "FCM_SERVICE_ACCOUNT is not text".to_string())?
        };
        let account: ServiceAccount = serde_json::from_str(&json).map_err(|e| format!("FCM_SERVICE_ACCOUNT is not a service account key: {e}"))?;
        let der = pem_body(&account.private_key).ok_or("the service account's private_key is not a PEM key")?;
        let key = RsaKeyPair::from_pkcs8(&der).map_err(|e| format!("the service account's private_key is not usable: {e}"))?;
        if !account.token_uri.starts_with("https://") {
            return Err("the service account's token_uri must be https".into());
        }
        let http = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build().map_err(|e| e.to_string())?;
        Ok(Self {
            http,
            send_url: format!("https://fcm.googleapis.com/v1/projects/{}/messages:send", account.project_id),
            email: account.client_email,
            token_uri: account.token_uri,
            key,
            access: Mutex::new(None),
        })
    }

    fn assertion(&self, now: u64) -> Result<String, String> {
        let header = BASE64URL_NOPAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let claims = serde_json::json!({
            "iss": self.email,
            "scope": SCOPE,
            "aud": self.token_uri,
            "iat": now,
            "exp": now + ASSERTION_SECS,
        });
        let body = BASE64URL_NOPAD.encode(claims.to_string().as_bytes());
        let signing_input = format!("{header}.{body}");
        let mut signature = vec![0u8; self.key.public().modulus_len()];
        self.key
            .sign(&RSA_PKCS1_SHA256, &SystemRandom::new(), signing_input.as_bytes(), &mut signature)
            .map_err(|_| "could not sign the FCM sign-in".to_string())?;
        Ok(format!("{signing_input}.{}", BASE64URL_NOPAD.encode(&signature)))
    }

    async fn access_token(&self) -> Result<String, String> {
        if let Some((token, until)) = self.access.lock().unwrap().as_ref() {
            if Instant::now() + RENEW_BEFORE < *until {
                return Ok(token.clone());
            }
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_secs();
        let assertion = self.assertion(now)?;
        let res = self
            .http
            .post(&self.token_uri)
            .form(&[("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"), ("assertion", assertion.as_str())])
            .send()
            .await
            .map_err(|e| format!("FCM sign-in failed: {e}"))?;
        if !res.status().is_success() {
            return Err(format!("FCM sign-in answered {}", res.status()));
        }
        let grant: Grant = res.json().await.map_err(|e| format!("FCM sign-in sent an unreadable answer: {e}"))?;
        let until = Instant::now() + Duration::from_secs(grant.expires_in);
        *self.access.lock().unwrap() = Some((grant.access_token.clone(), until));
        Ok(grant.access_token)
    }

    pub async fn wake(&self, token: &str) -> Delivery {
        let access = match self.access_token().await {
            Ok(access) => access,
            Err(e) => return Delivery::Failed(e),
        };
        let sent = self.http.post(&self.send_url).bearer_auth(access).json(&message(token)).send().await;
        match sent {
            Ok(res) if res.status().is_success() => Delivery::Sent,
            Ok(res) => match res.status().as_u16() {
                400 | 404 => Delivery::Gone,
                401 => {
                    *self.access.lock().unwrap() = None;
                    Delivery::Failed("FCM refused the sign-in".into())
                }
                code => Delivery::Failed(format!("FCM answered {code}")),
            },
            Err(e) => Delivery::Failed(format!("FCM unreachable: {e}")),
        }
    }
}

pub fn message(token: &str) -> serde_json::Value {
    serde_json::json!({
        "message": {
            "token": token,
            "data": { "wake": "1" },
            "android": { "priority": "high", "ttl": WAKE_TTL, "collapse_key": WAKE_COLLAPSE_KEY },
        }
    })
}

fn pem_body(pem: &str) -> Option<Vec<u8>> {
    let pem = pem.replace("\\n", "\n");
    let body: String =
        pem.lines().map(str::trim).skip_while(|line| !line.starts_with("-----BEGIN")).skip(1).take_while(|line| !line.starts_with("-----END")).collect();
    BASE64.decode(body.as_bytes()).ok().filter(|der| !der.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_push_carries_nothing_but_a_wake_marker() {
        let m = message("tok");
        assert_eq!(m["message"]["token"], "tok");
        assert_eq!(m["message"]["data"], serde_json::json!({ "wake": "1" }));
        assert_eq!(m["message"]["android"]["priority"], "high");
        assert!(m["message"].get("notification").is_none(), "no visible text ever comes from the push");
    }

    #[test]
    fn a_phone_that_is_offline_for_a_while_still_gets_one_wake_when_it_returns() {
        let m = message("tok");
        assert_eq!(m["message"]["android"]["ttl"], "3600s");
        assert_eq!(m["message"]["android"]["collapse_key"], "wake", "queued wakes collapse into one");
    }

    #[test]
    fn pem_bodies_are_read_with_real_or_escaped_newlines() {
        let real = "-----BEGIN PRIVATE KEY-----\nAAEC\nAwQ=\n-----END PRIVATE KEY-----\n";
        assert_eq!(pem_body(real), Some(vec![0, 1, 2, 3, 4]));
        assert_eq!(pem_body(&real.replace('\n', "\\n")), Some(vec![0, 1, 2, 3, 4]));
        assert_eq!(pem_body("not a key"), None);
    }

    #[test]
    fn the_sign_in_is_a_jwt_signed_with_the_service_account_key() {
        let Ok(path) = std::env::var("FCM_TEST_SERVICE_ACCOUNT") else {
            eprintln!("FCM_TEST_SERVICE_ACCOUNT is not set, skipping the signing test");
            return;
        };
        let Ok(fcm) = Fcm::from_config(&std::fs::read_to_string(path).unwrap()) else { panic!("the test service account loads") };
        let jwt = fcm.assertion(1_000).unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let claims: serde_json::Value = serde_json::from_slice(&BASE64URL_NOPAD.decode(parts[1].as_bytes()).unwrap()).unwrap();
        assert_eq!(claims["scope"], SCOPE);
        assert_eq!(claims["exp"], 1_000 + ASSERTION_SECS);
        let public = ring::signature::UnparsedPublicKey::new(&ring::signature::RSA_PKCS1_2048_8192_SHA256, fcm.key.public().as_ref());
        let signed = format!("{}.{}", parts[0], parts[1]);
        public.verify(signed.as_bytes(), &BASE64URL_NOPAD.decode(parts[2].as_bytes()).unwrap()).unwrap();
    }

    #[test]
    fn bad_service_accounts_are_refused_with_a_reason() {
        assert!(Fcm::from_config("{}").err().unwrap().contains("service account"));
        assert!(Fcm::from_config("!!").err().unwrap().contains("base64"));
        let no_key = r#"{"project_id":"p","client_email":"e","private_key":"x","token_uri":"https://t"}"#;
        assert!(Fcm::from_config(no_key).err().unwrap().contains("PEM"));
    }
}
