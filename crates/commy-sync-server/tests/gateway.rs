use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use commy_sync_server::config::Config;
use commy_sync_server::gateway::{self, AT_HEADER, GATEWAY_PATH, ORIGIN_HEADER, SIGNATURE_HEADER};
use commy_sync_server::limits::Rate;
use commy_sync_server::ramp::{Tier, FLOOD_FACTOR};
use commy_sync_server::server_proof::{key_id, push_payload, signed_bytes, KeySource, PublishedKey, PUSH_PURPOSE};
use commy_sync_server::store::Store;
use commy_sync_server::wake::Pusher;
use commy_sync_server::{serve, Service};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedReceiver;

const HOME: &str = "https://home.example.org";
const STRANGER: &str = "https://stranger.example.org";

struct Rig {
    base: String,
    service: Arc<Service>,
    pushed: UnboundedReceiver<String>,
    pair: Ed25519KeyPair,
    http: reqwest::Client,
    last_sent_at: AtomicI64,
}

fn pair(seed: u8) -> (Ed25519KeyPair, PublishedKey) {
    let pair = Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap();
    let public: [u8; 32] = pair.public_key().as_ref().try_into().unwrap();
    (pair, PublishedKey { id: key_id(&public), public })
}

async fn rig(pairs: &[(&str, &str)]) -> Rig {
    let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let mut config = Config::from_env(|key| map.get(key).cloned()).unwrap();
    config.limits.wakes = Rate { burst: 1_000_000.0, per_second: 1_000_000.0 };
    let (home_pair, home_key) = pair(1);
    let (_, stranger_key) = pair(2);
    let keys = KeySource::Fixed(HashMap::from([(HOME.to_string(), home_key), (STRANGER.to_string(), stranger_key)]));
    let (sender, pushed) = tokio::sync::mpsc::unbounded_channel();
    let service = Service::with_keys(Store::memory(), &config, Pusher::Recording(sender), keys);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(serve(listener, service.clone()));
    Rig { base, service, pushed, pair: home_pair, http: reqwest::Client::new(), last_sent_at: AtomicI64::new(0) }
}

async fn gateway_on() -> Rig {
    rig(&[("SYNC_PUSH_GATEWAY", "on")]).await
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

fn body_for(phones: impl IntoIterator<Item = String>) -> Vec<u8> {
    let pushes: Vec<Value> = phones.into_iter().map(|to| json!({ "to": to })).collect();
    serde_json::to_vec(&json!({ "pushes": pushes })).unwrap()
}

impl Rig {
    async fn send_as(&self, origin: &str, pair: &Ed25519KeyPair, at: i64, body: &[u8]) -> (u16, Value) {
        let signature = data_encoding::HEXLOWER.encode(pair.sign(&signed_bytes(PUSH_PURPOSE, origin, &push_payload(at, body))).as_ref());
        self.send_signed(origin, at, &signature, body).await
    }

    async fn send_signed(&self, origin: &str, at: i64, signature: &str, body: &[u8]) -> (u16, Value) {
        let response = self
            .http
            .post(format!("{}{GATEWAY_PATH}", self.base))
            .header(ORIGIN_HEADER, origin)
            .header(AT_HEADER, at.to_string())
            .header(SIGNATURE_HEADER, signature)
            .header("content-type", "application/json")
            .body(body.to_vec())
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        (status, response.json().await.unwrap_or(Value::Null))
    }

    fn next_send_at(&self) -> i64 {
        let mut next = 0;
        self.last_sent_at
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |last| {
                next = now_ms().max(last + 1);
                Some(next)
            })
            .unwrap();
        next
    }

    async fn send(&self, body: &[u8]) -> (u16, Value) {
        self.send_as(HOME, &self.pair, self.next_send_at(), body).await
    }

    fn delivered(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(push) = self.pushed.try_recv() {
            out.push(push);
        }
        out
    }
}

#[tokio::test]
async fn a_signed_ping_from_a_server_reaches_the_phone() {
    let mut rig = gateway_on().await;
    let (status, answer) = rig.send(&body_for(["fcm:phone-a".to_string()])).await;
    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer["tier"], "new");
    assert_eq!(answer["sent"], 1);
    assert_eq!(answer["left"]["pushes"], Tier::New.caps().pushes - 1);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(rig.delivered(), vec!["fcm:phone-a".to_string()]);
}

#[tokio::test]
async fn the_gateway_is_off_unless_turned_on() {
    let rig = rig(&[]).await;
    assert_eq!(rig.send(&body_for(["fcm:a".to_string()])).await.0, 404);
}

#[tokio::test]
async fn only_the_server_that_holds_the_key_can_send() {
    let rig = gateway_on().await;
    let body = body_for(["fcm:a".to_string()]);
    let (forger, _) = pair(9);
    assert_eq!(rig.send_as(HOME, &forger, now_ms(), &body).await.1["error"], "bad_signature");
    assert_eq!(rig.send_as(STRANGER, &rig.pair, now_ms(), &body).await.1["error"], "bad_signature", "another server's key");
    assert_eq!(rig.send_as("https://unknown.example.org", &rig.pair, now_ms(), &body).await.0, 401);
    assert_eq!(rig.send_as("http://home.example.org", &rig.pair, now_ms(), &body).await.1["error"], "bad_origin");
    assert_eq!(rig.send_as(HOME, &rig.pair, now_ms() - gateway::CLOCK_SKEW_MS - 1_000, &body).await.1["error"], "stale");
    let at = now_ms();
    assert_eq!(rig.send_as(HOME, &rig.pair, at, &body).await.0, 200);
    assert_eq!(rig.send_as(HOME, &rig.pair, at, &body).await.1["error"], "replayed");
}

#[tokio::test]
async fn only_sealed_previews_and_real_phones_are_carried() {
    let rig = gateway_on().await;
    let plain = serde_json::to_vec(&json!({ "pushes": [{ "to": "fcm:a", "previews": ["Hello, this is readable text"] }] })).unwrap();
    assert_eq!(rig.send(&plain).await.1["error"], "bad_pushes");
    assert_eq!(rig.send(&body_for(["sms:+15550100".to_string()])).await.1["error"], "bad_pushes");
    let too_many = body_for((0..=gateway::MAX_GATEWAY_PUSHES).map(|n| format!("fcm:{n}")));
    assert_eq!(rig.send(&too_many).await.1["error"], "bad_pushes");
    assert_eq!(rig.send(b"{}").await.1["error"], "bad_body");
}

#[tokio::test]
async fn abuse_drill_a_flooding_server_is_capped_then_cut_off() {
    let mut rig = gateway_on().await;
    let caps = Tier::New.caps();
    let batch = gateway::MAX_GATEWAY_PUSHES;
    let mut sent = 0u64;
    let mut requests = 0u64;
    let mut cut_off = false;
    for round in 0u64.. {
        let phones = (0..batch).map(|n| format!("fcm:victim-{}", (round * batch as u64 + n as u64) % 40));
        let (status, answer) = rig.send(&body_for(phones)).await;
        requests += 1;
        if status == 403 {
            assert_eq!(answer["error"], "cut_off");
            cut_off = true;
            break;
        }
        assert!(status == 200 || status == 429, "{status} {answer}");
        sent += answer["sent"].as_u64().unwrap();
        assert!(requests < 2 * caps.pushes * FLOOD_FACTOR / batch as u64, "the flood never stopped");
    }
    assert!(cut_off, "the server is cut off automatically");
    assert_eq!(sent, caps.pushes, "no more pings than a new server's cap");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let reached: HashSet<String> = rig.delivered().into_iter().collect();
    assert_eq!(reached.len() as u64, caps.new_phones, "no more strangers' phones than the new phone cap");
    let overview = gateway::overview(&rig.service).await.unwrap();
    let row = overview.servers.iter().find(|row| row.record.origin == HOME).unwrap();
    assert!(row.record.cut_off.as_ref().is_some_and(|cut| cut.automatic));
    assert_eq!(row.today.pushes, caps.pushes);
    assert!(row.today.refused >= caps.pushes * FLOOD_FACTOR);
    assert_eq!(rig.send(&body_for(["fcm:victim-0".to_string()])).await.0, 403, "it stays cut off");
    let (status, _) = rig.send_as(STRANGER, &pair(2).0, now_ms(), &body_for(["fcm:friend".to_string()])).await;
    assert_eq!(status, 200, "other servers keep working");
}

#[tokio::test]
async fn our_daily_budget_caps_every_server_together() {
    let rig = rig(&[("SYNC_PUSH_GATEWAY", "on"), ("SYNC_GATEWAY_DAILY_BUDGET", "3")]).await;
    let (_, first) = rig.send(&body_for(["fcm:a".to_string(), "fcm:b".to_string()])).await;
    assert_eq!(first["sent"], 2);
    let (status, second) = rig.send_as(STRANGER, &pair(2).0, now_ms(), &body_for(["fcm:c".to_string(), "fcm:d".to_string()])).await;
    assert_eq!((status, second["sent"].as_u64(), second["refused"].as_u64()), (200, Some(1), Some(1)));
    let (status, third) = rig.send(&body_for(["fcm:a".to_string()])).await;
    assert_eq!((status, third["sent"].as_u64()), (429, Some(0)));
    let overview = gateway::overview(&rig.service).await.unwrap();
    assert_eq!((overview.budget, overview.used, overview.refused), (3, 3, 2));
}

#[tokio::test]
async fn a_self_hosted_server_wakes_phones_through_the_gateway() {
    use commy_sync_server::fcm::Delivery;
    use commy_sync_server::push_via::ViaGateway;
    let mut commy = gateway_on().await;
    let via = ViaGateway::new(&commy.base, HOME, commy_core::Identity::from_bytes(&[1; 32]));
    assert_eq!(via.wake("fcm:phone-b", &[]).await, Delivery::Sent);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(commy.delivered(), vec!["fcm:phone-b".to_string()]);

    let impostor = ViaGateway::new(&commy.base, HOME, commy_core::Identity::from_bytes(&[9; 32]));
    assert!(matches!(impostor.wake("fcm:phone-b", &[]).await, Delivery::Refused(_)));
    let off = rig(&[]).await;
    let unwelcome = ViaGateway::new(&off.base, HOME, commy_core::Identity::from_bytes(&[1; 32]));
    assert!(matches!(unwelcome.wake("fcm:phone-b", &[]).await, Delivery::Refused(_)), "a gateway that is off refuses");
}

#[tokio::test]
async fn a_server_on_the_signed_blocklist_is_refused() {
    use commy_core::server_blocklist::{BlockedServer, Envelope, ServerBlocklist, LIST_PURPOSE};
    let rig = gateway_on().await;
    let list_key = commy_core::Identity::from_bytes(&[8; 32]);
    let list = ServerBlocklist { v: 1, issued: 1, expires: 2, domains: vec![BlockedServer { domain: "example.org".into(), since: 0 }], homes: Vec::new() };
    let text = serde_json::to_string(&list).unwrap();
    let sig = commy_core::server_key::sign(&list_key, LIST_PURPOSE, "https://commy.chat", text.as_bytes());
    let envelope = Envelope { list: text, key: commy_core::server_key::PublishedKey::of(list_key.public()).id, sig };
    rig.service.gateway.blocklist.take(&envelope, &list_key.public()).unwrap();
    let (status, answer) = rig.send(&body_for(["fcm:a".to_string()])).await;
    assert_eq!((status, answer["error"].as_str()), (403, Some("blocked")));
}
