use std::sync::Mutex;
use std::time::{Duration, Instant};

use data_encoding::{BASE64, BASE64URL_NOPAD};
use ring::rand::SystemRandom;
use ring::signature::{EcdsaKeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
use serde::Deserialize;

use crate::fcm::{pem_body, Delivery};

pub const DEFAULT_TOPIC: &str = "chat.commy.mobile";
const PRODUCTION_HOST: &str = "https://api.push.apple.com";
const SANDBOX_HOST: &str = "https://api.sandbox.push.apple.com";
const SIGN_IN_LIFETIME: Duration = Duration::from_secs(45 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const WAKE_TTL_SECS: u64 = 3600;
const WAKE_COLLAPSE_ID: &str = "wake";
pub const ALERT_TITLE: &str = "Commy";
pub const ALERT_BODY: &str = "New messages in Commy";
pub const ALERT_SOUND: &str = "commy_message.wav";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Production,
    Sandbox,
}

impl Environment {
    fn host(self) -> &'static str {
        match self {
            Environment::Production => PRODUCTION_HOST,
            Environment::Sandbox => SANDBOX_HOST,
        }
    }
}

pub struct ApnsConfig<'a> {
    pub key: &'a str,
    pub key_id: &'a str,
    pub team_id: &'a str,
    pub topic: &'a str,
}

pub struct Apns {
    http: reqwest::Client,
    key: EcdsaKeyPair,
    key_id: String,
    team_id: String,
    topic: String,
    sign_in: Mutex<Option<(String, Instant)>>,
}

#[derive(Deserialize)]
struct Refusal {
    #[serde(default)]
    reason: String,
}

impl Apns {
    pub fn from_config(config: ApnsConfig) -> Result<Self, String> {
        let pem = key_text(config.key)?;
        let der = pem_body(&pem).ok_or("APNS_KEY must be the .p8 key Apple gave you, or its base64")?;
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der, &SystemRandom::new())
            .map_err(|e| format!("APNS_KEY is not a usable Apple push key: {e}"))?;
        let key_id = plain_id(config.key_id, "APNS_KEY_ID")?;
        let team_id = plain_id(config.team_id, "APNS_TEAM_ID")?;
        let topic = config.topic.trim();
        if topic.is_empty() || !topic.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-')) {
            return Err("APNS_TOPIC must be the app's bundle id".into());
        }
        let http = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).http2_prior_knowledge().build().map_err(|e| e.to_string())?;
        Ok(Self { http, key, key_id, team_id, topic: topic.to_string(), sign_in: Mutex::new(None) })
    }

    fn signed_sign_in(&self, now: u64) -> Result<String, String> {
        let header = serde_json::json!({ "alg": "ES256", "kid": self.key_id });
        let claims = serde_json::json!({ "iss": self.team_id, "iat": now });
        let signing_input =
            format!("{}.{}", BASE64URL_NOPAD.encode(header.to_string().as_bytes()), BASE64URL_NOPAD.encode(claims.to_string().as_bytes()));
        let signature = self.key.sign(&SystemRandom::new(), signing_input.as_bytes()).map_err(|_| "could not sign the APNs sign-in".to_string())?;
        Ok(format!("{signing_input}.{}", BASE64URL_NOPAD.encode(signature.as_ref())))
    }

    fn sign_in(&self) -> Result<String, String> {
        let mut held = self.sign_in.lock().unwrap();
        if let Some((token, made)) = held.as_ref() {
            if made.elapsed() < SIGN_IN_LIFETIME {
                return Ok(token.clone());
            }
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_secs();
        let token = self.signed_sign_in(now)?;
        *held = Some((token.clone(), Instant::now()));
        Ok(token)
    }

    pub async fn wake(&self, environment: Environment, token: &str, previews: &[String]) -> Delivery {
        let sign_in = match self.sign_in() {
            Ok(sign_in) => sign_in,
            Err(e) => return Delivery::Failed(e),
        };
        let expires = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() + WAKE_TTL_SECS).unwrap_or(0);
        let sent = self
            .http
            .post(format!("{}/3/device/{token}", environment.host()))
            .bearer_auth(sign_in)
            .header("apns-topic", &self.topic)
            .header("apns-push-type", "alert")
            .header("apns-priority", "10")
            .header("apns-collapse-id", WAKE_COLLAPSE_ID)
            .header("apns-expiration", expires.to_string())
            .json(&payload(previews))
            .send()
            .await;
        let res = match sent {
            Ok(res) => res,
            Err(e) => return Delivery::Failed(format!("APNs unreachable: {e}")),
        };
        let status = res.status().as_u16();
        if status == 200 {
            return Delivery::Sent;
        }
        let reason = res.json::<Refusal>().await.map(|r| r.reason).unwrap_or_default();
        self.judge(status, &reason)
    }

    fn judge(&self, status: u16, reason: &str) -> Delivery {
        match (status, reason) {
            (410, _) | (400, "BadDeviceToken" | "DeviceTokenNotForTopic") => Delivery::Gone,
            (403, "ExpiredProviderToken" | "InvalidProviderToken") => {
                *self.sign_in.lock().unwrap() = None;
                Delivery::Failed(format!("APNs refused the sign-in: {reason}"))
            }
            (code, "") => Delivery::Failed(format!("APNs answered {code}")),
            (code, reason) => Delivery::Failed(format!("APNs answered {code}: {reason}")),
        }
    }
}

pub fn payload(previews: &[String]) -> serde_json::Value {
    let mut body = serde_json::json!({
        "aps": {
            "alert": { "title": ALERT_TITLE, "body": ALERT_BODY },
            "sound": ALERT_SOUND,
            "mutable-content": 1,
        },
        "wake": "1",
    });
    if let Some(newest) = previews.last() {
        body["preview"] = newest.as_str().into();
    }
    if previews.len() > 1 {
        body["previews"] = previews.join(",").into();
    }
    body
}

fn key_text(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.contains("-----BEGIN") {
        return Ok(text.to_string());
    }
    let decoded = BASE64.decode(text.as_bytes()).map_err(|_| "APNS_KEY must be the .p8 key text or its base64".to_string())?;
    String::from_utf8(decoded).map_err(|_| "APNS_KEY is not text".to_string())
}

fn plain_id(text: &str, name: &str) -> Result<String, String> {
    let id = text.trim();
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(format!("{name} must be the 10 letter id Apple shows"));
    }
    Ok(id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{KeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_FIXED};

    const PEM_LABEL: &str = "PRIVATE KEY";

    fn wrap_pem(body: &str) -> String {
        format!("-----BEGIN {PEM_LABEL}-----\n{body}\n-----END {PEM_LABEL}-----\n")
    }

    fn test_key_pem() -> String {
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new()).unwrap();
        let body = BASE64.encode(pkcs8.as_ref());
        let lines: Vec<&str> = body.as_bytes().chunks(64).map(|c| std::str::from_utf8(c).unwrap()).collect();
        wrap_pem(&lines.join("\n"))
    }

    fn apns(key: &str) -> Result<Apns, String> {
        Apns::from_config(ApnsConfig { key, key_id: "ABC123DEFG", team_id: "TEAM456789", topic: DEFAULT_TOPIC })
    }

    #[test]
    fn the_push_shows_only_a_fixed_line_and_carries_previews_sealed() {
        let plain = payload(&[]);
        assert_eq!(plain["aps"]["alert"]["title"], ALERT_TITLE);
        assert_eq!(plain["aps"]["alert"]["body"], ALERT_BODY);
        assert_eq!(plain["aps"]["mutable-content"], 1);
        assert_eq!(plain["aps"]["sound"], ALERT_SOUND);
        assert!(plain.get("preview").is_none());
        let held = payload(&["b25l".to_string(), "dHdv".to_string()]);
        assert_eq!(held["preview"], "dHdv");
        assert_eq!(held["previews"], "b25l,dHdv");
        assert_eq!(held["aps"]["alert"]["body"], ALERT_BODY, "message text never travels in the clear");
    }

    #[test]
    fn the_push_names_a_sound_the_iphone_app_carries() {
        let bundled = concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/desktop/mobile/sounds/");
        assert!(std::path::Path::new(bundled).join(ALERT_SOUND).is_file(), "{ALERT_SOUND} is not in {bundled}");
    }

    #[test]
    fn the_sign_in_is_an_es256_jwt_for_the_team_and_key() {
        let pem = test_key_pem();
        let apns = apns(&pem).unwrap();
        let jwt = apns.signed_sign_in(1_000).unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let header: serde_json::Value = serde_json::from_slice(&BASE64URL_NOPAD.decode(parts[0].as_bytes()).unwrap()).unwrap();
        assert_eq!(header, serde_json::json!({ "alg": "ES256", "kid": "ABC123DEFG" }));
        let claims: serde_json::Value = serde_json::from_slice(&BASE64URL_NOPAD.decode(parts[1].as_bytes()).unwrap()).unwrap();
        assert_eq!(claims, serde_json::json!({ "iss": "TEAM456789", "iat": 1_000 }));
        let public = UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, apns.key.public_key().as_ref().to_vec());
        let signed = format!("{}.{}", parts[0], parts[1]);
        public.verify(signed.as_bytes(), &BASE64URL_NOPAD.decode(parts[2].as_bytes()).unwrap()).unwrap();
    }

    #[test]
    fn the_key_is_read_as_pem_escaped_pem_or_base64() {
        let pem = test_key_pem();
        assert!(apns(&pem).is_ok());
        assert!(apns(&pem.replace('\n', "\\n")).is_ok());
        assert!(apns(&BASE64.encode(pem.as_bytes())).is_ok());
    }

    #[test]
    fn bad_settings_are_refused_with_a_reason() {
        assert!(apns("!!").err().unwrap().contains("APNS_KEY"));
        assert!(apns(&wrap_pem("AAEC")).err().unwrap().contains("APNS_KEY"));
        let pem = test_key_pem();
        let no_team = Apns::from_config(ApnsConfig { key: &pem, key_id: "ABC123DEFG", team_id: " ", topic: DEFAULT_TOPIC });
        assert!(no_team.err().unwrap().contains("APNS_TEAM_ID"));
        let bad_topic = Apns::from_config(ApnsConfig { key: &pem, key_id: "ABC123DEFG", team_id: "TEAM456789", topic: "a b" });
        assert!(bad_topic.err().unwrap().contains("APNS_TOPIC"));
    }

    #[test]
    fn apple_answers_decide_whether_a_phone_is_gone() {
        let apns = apns(&test_key_pem()).unwrap();
        assert_eq!(apns.judge(410, "Unregistered"), Delivery::Gone);
        assert_eq!(apns.judge(400, "BadDeviceToken"), Delivery::Gone);
        assert_eq!(apns.judge(400, "DeviceTokenNotForTopic"), Delivery::Gone);
        assert!(matches!(apns.judge(429, "TooManyRequests"), Delivery::Failed(_)));
        assert!(matches!(apns.judge(500, ""), Delivery::Failed(_)));
        apns.sign_in().unwrap();
        assert!(matches!(apns.judge(403, "ExpiredProviderToken"), Delivery::Failed(_)));
        assert!(apns.sign_in.lock().unwrap().is_none(), "a refused sign-in is made again");
    }
}
