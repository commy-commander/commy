use commy_core::lock_or_recover::{LockOrRecover, ReadOrRecover};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use commy_sync::pass::{epoch_of, Pass, PassKey, PASS_TOKEN_BYTES};
use data_encoding::BASE64URL_NOPAD;
use serde::Deserialize;

use crate::config::PassSettings;
use crate::store::{Store, StoreError};
use crate::Service;

pub const PASS_SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS sync_spent_passes (token BYTEA PRIMARY KEY, epoch BIGINT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS sync_spent_passes_epoch ON sync_spent_passes (epoch)",
];

const REFRESH_EVERY: Duration = Duration::from_secs(60 * 60);
const FIRST_RETRY: Duration = Duration::from_secs(1);
const LONGEST_RETRY: Duration = Duration::from_secs(60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_KEYS_BYTES: usize = 64 * 1024;
const MAX_KEYS: usize = 8;

#[derive(Deserialize)]
struct Published {
    keys: Vec<PublishedKey>,
}

#[derive(Deserialize)]
struct PublishedKey {
    epoch: u32,
    der: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spent {
    Now,
    Unusable,
}

pub struct Passes {
    settings: Option<PassSettings>,
    keys: RwLock<HashMap<u32, PassKey>>,
}

impl Passes {
    pub fn new(settings: Option<PassSettings>) -> Self {
        Self { settings, keys: RwLock::default() }
    }

    pub fn issuer(&self) -> Option<&str> {
        self.settings.as_ref().map(|s| s.issuer.as_str())
    }

    fn keys_url(&self) -> &str {
        self.settings.as_ref().map_or("", |s| s.keys_url.as_str())
    }

    pub fn required(&self) -> bool {
        self.settings.as_ref().is_some_and(|s| s.required)
    }

    pub fn learn(&self, published: &str) -> Result<usize, String> {
        let published: Published = serde_json::from_str(published).map_err(|e| format!("not a key list: {e}"))?;
        let mut keys = HashMap::new();
        for key in published.keys.into_iter().take(MAX_KEYS) {
            let der = BASE64URL_NOPAD.decode(key.der.as_bytes()).map_err(|_| format!("the key for week {} is not base64url", key.epoch))?;
            let parsed = PassKey::from_der(key.epoch, &der).map_err(|e| format!("week {}: {e}", key.epoch))?;
            keys.insert(key.epoch, parsed);
        }
        let count = keys.len();
        *self.keys.write_or_recover() = keys;
        Ok(count)
    }

    pub fn holds(&self, pass: &Pass, now_ms: i64) -> bool {
        let now = epoch_of(now_ms);
        if pass.epoch != now && pass.epoch.checked_add(1) != Some(now) {
            return false;
        }
        self.keys.read_or_recover().get(&pass.epoch).is_some_and(|key| key.verify(pass))
    }

    pub async fn spend(&self, store: &Store, pass: &Pass) -> Result<Spent, StoreError> {
        Ok(if store.spend_pass(pass.epoch, &pass.token.0).await? { Spent::Now } else { Spent::Unusable })
    }

    pub async fn refresh(&self) -> Result<usize, String> {
        let Some(settings) = &self.settings else { return Ok(0) };
        let client = reqwest::Client::builder().timeout(FETCH_TIMEOUT).build().map_err(|e| e.to_string())?;
        let response = client.get(&settings.keys_url).send().await.map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("answered {}", response.status()));
        }
        let body = response.bytes().await.map_err(|e| e.to_string())?;
        if body.len() > MAX_KEYS_BYTES {
            return Err("the key list is too large".into());
        }
        self.learn(std::str::from_utf8(&body).map_err(|_| "the key list is not text".to_string())?)
    }
}

pub struct Backoff {
    first: Duration,
    longest: Duration,
    next: Duration,
}

impl Backoff {
    pub fn new(first: Duration, longest: Duration) -> Self {
        Self { first, longest, next: first }
    }

    pub fn next_wait(&mut self) -> Duration {
        let wait = self.next;
        self.next = (wait * 2).min(self.longest);
        wait
    }

    pub fn reset(&mut self) {
        self.next = self.first;
    }
}

pub async fn learn_keys_until_answered(passes: &Passes, backoff: &mut Backoff) -> usize {
    let mut failures = 0u32;
    loop {
        match passes.refresh().await {
            Ok(count) => {
                backoff.reset();
                if failures > 0 {
                    tracing::info!("sync: {count} stream pass keys from {} after {failures} failed tries", passes.keys_url());
                }
                return count;
            }
            Err(e) => {
                failures += 1;
                let wait = backoff.next_wait();
                tracing::warn!("sync: fetching stream pass keys from {} failed: {e}; trying again in {}s", passes.keys_url(), wait.as_secs_f32());
                tokio::time::sleep(wait).await;
            }
        }
    }
}

pub fn spawn_refresh(service: Arc<Service>) {
    let Some(settings) = service.passes.settings.clone() else { return };
    tokio::spawn(async move {
        let mut backoff = Backoff::new(FIRST_RETRY, LONGEST_RETRY);
        loop {
            let count = learn_keys_until_answered(&service.passes, &mut backoff).await;
            tracing::debug!("sync: {count} new stream pass keys from {}", settings.keys_url);
            let spent_before = epoch_of(crate::now_ms()).saturating_sub(1);
            if let Err(e) = service.store.prune_passes(spent_before).await {
                tracing::warn!("sync: pruning spent passes failed: {e}");
            }
            tokio::time::sleep(REFRESH_EVERY).await;
        }
    });
}

impl Store {
    pub async fn spend_pass(&self, epoch: u32, token: &[u8; PASS_TOKEN_BYTES]) -> Result<bool, StoreError> {
        match self {
            Store::Memory(m) => Ok(m.spent_passes.lock_or_recover().insert(*token, epoch).is_none()),
            Store::Postgres(p) => {
                let done = sqlx::query("INSERT INTO sync_spent_passes (token, epoch) VALUES ($1, $2) ON CONFLICT (token) DO NOTHING")
                    .bind(&token[..])
                    .bind(i64::from(epoch))
                    .execute(&p.pool)
                    .await?;
                Ok(done.rows_affected() == 1)
            }
        }
    }

    pub async fn prune_passes(&self, before_epoch: u32) -> Result<u64, StoreError> {
        match self {
            Store::Memory(m) => {
                let mut spent = m.spent_passes.lock_or_recover();
                let had = spent.len();
                spent.retain(|_, epoch| *epoch >= before_epoch);
                Ok((had - spent.len()) as u64)
            }
            Store::Postgres(p) => {
                let done = sqlx::query("DELETE FROM sync_spent_passes WHERE epoch < $1").bind(i64::from(before_epoch)).execute(&p.pool).await?;
                Ok(done.rows_affected())
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use commy_sync::pass::PASS_KEY_BITS;
    use rsa::pkcs8::EncodePublicKey;
    use rsa::traits::{PrivateKeyParts, PublicKeyParts};
    use rsa::{BigUint, RsaPrivateKey};
    use std::sync::OnceLock;

    pub(crate) const WEEK_MS: i64 = 7 * 24 * 60 * 60 * 1000;

    pub(crate) fn issuer() -> &'static RsaPrivateKey {
        static KEY: OnceLock<RsaPrivateKey> = OnceLock::new();
        KEY.get_or_init(|| RsaPrivateKey::new(&mut rand::thread_rng(), PASS_KEY_BITS).unwrap())
    }

    pub(crate) fn published(epochs: &[u32]) -> String {
        let der = issuer().to_public_key().to_public_key_der().unwrap();
        let keys: Vec<serde_json::Value> =
            epochs.iter().map(|epoch| serde_json::json!({ "epoch": epoch, "der": BASE64URL_NOPAD.encode(der.as_bytes()) })).collect();
        serde_json::json!({ "keys": keys }).to_string()
    }

    pub(crate) fn pass_for(epoch: u32) -> Pass {
        let der = issuer().to_public_key().to_public_key_der().unwrap();
        let key = PassKey::from_der(epoch, der.as_bytes()).unwrap();
        let (blinded, pending) = key.blind(&mut rand::thread_rng()).unwrap();
        let private = issuer();
        let signed = BigUint::from_bytes_be(&blinded).modpow(private.d(), private.n()).to_bytes_be();
        let mut blind_sig = vec![0u8; private.size() - signed.len()];
        blind_sig.extend_from_slice(&signed);
        key.finish(pending, &blind_sig).unwrap()
    }

    fn settings() -> Option<PassSettings> {
        settings_at("http://127.0.0.1:9/keys")
    }

    fn settings_at(keys_url: &str) -> Option<PassSettings> {
        Some(PassSettings { issuer: "https://accounts.example.org".into(), keys_url: keys_url.into(), required: false })
    }

    async fn serve_keys(listener: tokio::net::TcpListener, body: String) {
        let router = axum::Router::new().route("/keys", axum::routing::get(move || async move { body }));
        axum::serve(listener, router).await.unwrap();
    }

    #[test]
    fn retries_wait_twice_as_long_each_time_up_to_a_limit_and_start_over_after_an_answer() {
        let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(5));
        let waits: Vec<u64> = (0..5).map(|_| backoff.next_wait().as_secs()).collect();
        assert_eq!(waits, [1, 2, 4, 5, 5]);
        backoff.reset();
        assert_eq!(backoff.next_wait(), Duration::from_secs(1));
    }

    #[tokio::test]
    async fn keys_are_learned_once_an_auth_server_that_started_late_answers() {
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = free.local_addr().unwrap();
        drop(free);
        let passes = Arc::new(Passes::new(settings_at(&format!("http://{address}/keys"))));
        let learning = tokio::spawn({
            let passes = passes.clone();
            async move {
                let mut backoff = Backoff::new(Duration::from_millis(10), Duration::from_millis(50));
                learn_keys_until_answered(&passes, &mut backoff).await
            }
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!learning.is_finished(), "it keeps trying while nothing answers");
        assert!(!passes.holds(&pass_for(7), 7 * WEEK_MS));
        let auth = tokio::net::TcpListener::bind(address).await.unwrap();
        tokio::spawn(serve_keys(auth, published(&[7])));
        let learned = tokio::time::timeout(Duration::from_secs(5), learning).await.expect("keys arrive soon after auth answers").unwrap();
        assert_eq!(learned, 1);
        assert!(passes.holds(&pass_for(7), 7 * WEEK_MS));
    }

    #[test]
    fn passes_count_in_their_week_and_the_one_after() {
        let passes = Passes::new(settings());
        let week = 2_900u32;
        let now = i64::from(week) * WEEK_MS + 1000;
        assert_eq!(passes.learn(&published(&[week - 1, week, week + 1])), Ok(3));
        assert!(passes.holds(&pass_for(week), now));
        assert!(passes.holds(&pass_for(week - 1), now), "a pass from last week still counts");
        assert!(!passes.holds(&pass_for(week - 2), now), "older passes do not");
        assert!(!passes.holds(&pass_for(week + 1), now), "nor passes for a week that has not started");
        let mut bent = pass_for(week);
        bent.token.0[0] ^= 1;
        assert!(!passes.holds(&bent, now));
        assert_eq!(passes.learn(&published(&[week + 5])), Ok(1));
        assert!(!passes.holds(&pass_for(week), now), "a key the issuer stopped publishing is dropped");
    }

    #[test]
    fn a_bad_key_list_keeps_the_keys_already_known() {
        let passes = Passes::new(settings());
        let now = 7 * WEEK_MS;
        passes.learn(&published(&[7])).unwrap();
        assert!(passes.learn("junk").is_err());
        assert!(passes.learn(r#"{"keys":[{"epoch":7,"der":"AAAA"}]}"#).is_err());
        assert!(passes.holds(&pass_for(7), now));
    }

    #[tokio::test]
    async fn a_pass_is_spent_once_and_forgotten_after_its_weeks() {
        let store = Store::memory();
        let passes = Passes::new(settings());
        let pass = pass_for(7);
        assert_eq!(passes.spend(&store, &pass).await.unwrap(), Spent::Now);
        assert_eq!(passes.spend(&store, &pass).await.unwrap(), Spent::Unusable);
        assert_eq!(store.prune_passes(7).await.unwrap(), 0);
        assert_eq!(store.prune_passes(8).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn spent_passes_are_kept_in_postgres() {
        let Ok(url) = std::env::var("SYNC_POSTGRES_TEST_URL") else { return };
        let url = format!("{}/commy_sync_passes_{}", url.trim_end_matches('/'), rand::random::<u32>());
        let store = Store::postgres(&url).await.unwrap();
        let token: [u8; PASS_TOKEN_BYTES] = rand::random();
        assert!(store.spend_pass(3, &token).await.unwrap());
        assert!(!store.spend_pass(3, &token).await.unwrap());
        assert_eq!(store.prune_passes(3).await.unwrap(), 0);
        assert_eq!(store.prune_passes(4).await.unwrap(), 1);
        assert!(store.spend_pass(3, &token).await.unwrap(), "a pruned week is past using anyway");
    }

    #[test]
    fn with_no_issuer_nothing_is_taken() {
        let passes = Passes::new(None);
        assert_eq!(passes.issuer(), None);
        assert!(!passes.required());
        assert!(!passes.holds(&pass_for(7), 7 * WEEK_MS));
    }
}
