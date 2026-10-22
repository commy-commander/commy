use std::collections::HashMap;
use std::sync::Mutex;

use commy_sync::stream::{StreamId, APPEND_NONCE_BYTES};
use commy_sync::wire::{Blob, WireEntry};
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};
use sqlx::{ConnectOptions, Row};

const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS sync_streams (id BYTEA PRIMARY KEY, head BIGINT NOT NULL, last_at BIGINT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS sync_entries (stream BYTEA NOT NULL, seq BIGINT NOT NULL, nonce BYTEA NOT NULL, at BIGINT NOT NULL, body BYTEA NOT NULL, PRIMARY KEY (stream, seq))",
    "CREATE UNIQUE INDEX IF NOT EXISTS sync_entries_nonce ON sync_entries (stream, nonce)",
    "CREATE INDEX IF NOT EXISTS sync_entries_at ON sync_entries (at)",
    "CREATE TABLE IF NOT EXISTS sync_wakes (tag BYTEA NOT NULL, push TEXT NOT NULL, until_day BIGINT NOT NULL, PRIMARY KEY (tag, push))",
    "CREATE INDEX IF NOT EXISTS sync_wakes_until ON sync_wakes (until_day)",
];

pub const MAX_PUSHES_PER_TAG: usize = 16;

const UNIQUE_VIOLATION: &str = "23505";
const INVALID_CATALOG_NAME: &str = "3D000";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Appended {
    pub seq: u64,
    pub at: i64,
    pub fresh: bool,
}

pub enum Store {
    Memory(MemoryStore),
    Postgres(PgStore),
}

impl Store {
    pub fn memory() -> Self {
        Store::Memory(MemoryStore::default())
    }

    pub async fn postgres(url: &str) -> Result<Self, StoreError> {
        Ok(Store::Postgres(PgStore::connect(url).await?))
    }

    pub async fn append(&self, stream: &StreamId, nonce: &[u8; APPEND_NONCE_BYTES], body: &[u8], at: i64) -> Result<Appended, StoreError> {
        match self {
            Store::Memory(m) => Ok(m.append(stream, nonce, body, at)),
            Store::Postgres(p) => p.append(stream, nonce, body, at).await,
        }
    }

    pub async fn page(&self, stream: &StreamId, after: u64, limit: usize) -> Result<Vec<WireEntry>, StoreError> {
        match self {
            Store::Memory(m) => Ok(m.page(stream, after, limit)),
            Store::Postgres(p) => p.page(stream, after, limit).await,
        }
    }

    pub async fn head(&self, stream: &StreamId) -> Result<u64, StoreError> {
        match self {
            Store::Memory(m) => Ok(m.head(stream)),
            Store::Postgres(p) => p.head(stream).await,
        }
    }

    pub async fn prune(&self, before: i64) -> Result<u64, StoreError> {
        match self {
            Store::Memory(m) => Ok(m.prune(before)),
            Store::Postgres(p) => p.prune(before).await,
        }
    }

    pub async fn register_wake(&self, tag: &[u8; 32], push: &str, until: i64, today: i64) -> Result<bool, StoreError> {
        match self {
            Store::Memory(m) => Ok(m.register_wake(tag, push, until, today)),
            Store::Postgres(p) => p.register_wake(tag, push, until, today).await,
        }
    }

    pub async fn wake_targets(&self, tags: &[[u8; 32]], today: i64) -> Result<Vec<String>, StoreError> {
        match self {
            Store::Memory(m) => Ok(m.wake_targets(tags, today)),
            Store::Postgres(p) => p.wake_targets(tags, today).await,
        }
    }

    pub async fn forget_wake(&self, tag: &[u8; 32], push: &str) -> Result<(), StoreError> {
        match self {
            Store::Memory(m) => {
                m.forget_wake(tag, push);
                Ok(())
            }
            Store::Postgres(p) => p.forget_wake(tag, push).await,
        }
    }

    pub async fn forget_push(&self, push: &str) -> Result<(), StoreError> {
        match self {
            Store::Memory(m) => {
                m.forget_push(push);
                Ok(())
            }
            Store::Postgres(p) => p.forget_push(push).await,
        }
    }

    pub async fn prune_wakes(&self, today: i64) -> Result<u64, StoreError> {
        match self {
            Store::Memory(m) => Ok(m.prune_wakes(today)),
            Store::Postgres(p) => p.prune_wakes(today).await,
        }
    }

    pub async fn ping(&self) -> bool {
        match self {
            Store::Memory(_) => true,
            Store::Postgres(p) => sqlx::query("SELECT 1").execute(&p.pool).await.is_ok(),
        }
    }
}

#[derive(Default)]
struct MemoryStream {
    head: u64,
    last_at: i64,
    entries: Vec<(u64, [u8; APPEND_NONCE_BYTES], i64, Vec<u8>)>,
}

#[derive(Default)]
pub struct MemoryStore {
    streams: Mutex<HashMap<StreamId, MemoryStream>>,
    wakes: Mutex<HashMap<[u8; 32], HashMap<String, i64>>>,
}

impl MemoryStore {
    fn register_wake(&self, tag: &[u8; 32], push: &str, until: i64, today: i64) -> bool {
        let mut wakes = self.wakes.lock().unwrap();
        let pushes = wakes.entry(*tag).or_default();
        pushes.retain(|_, kept| *kept >= today);
        if !pushes.contains_key(push) && pushes.len() >= MAX_PUSHES_PER_TAG {
            return false;
        }
        let kept = pushes.entry(push.to_string()).or_insert(until);
        *kept = (*kept).max(until);
        true
    }

    fn wake_targets(&self, tags: &[[u8; 32]], today: i64) -> Vec<String> {
        let wakes = self.wakes.lock().unwrap();
        let mut out: Vec<String> = tags
            .iter()
            .filter_map(|tag| wakes.get(tag))
            .flat_map(|pushes| pushes.iter().filter(|(_, until)| **until >= today).map(|(push, _)| push.clone()))
            .collect();
        out.sort();
        out.dedup();
        out
    }

    fn forget_wake(&self, tag: &[u8; 32], push: &str) {
        let mut wakes = self.wakes.lock().unwrap();
        if let Some(pushes) = wakes.get_mut(tag) {
            pushes.remove(push);
            if pushes.is_empty() {
                wakes.remove(tag);
            }
        }
    }

    fn forget_push(&self, push: &str) {
        let mut wakes = self.wakes.lock().unwrap();
        for pushes in wakes.values_mut() {
            pushes.remove(push);
        }
        wakes.retain(|_, pushes| !pushes.is_empty());
    }

    fn prune_wakes(&self, today: i64) -> u64 {
        let mut wakes = self.wakes.lock().unwrap();
        let mut removed = 0u64;
        for pushes in wakes.values_mut() {
            let had = pushes.len();
            pushes.retain(|_, until| *until >= today);
            removed += (had - pushes.len()) as u64;
        }
        wakes.retain(|_, pushes| !pushes.is_empty());
        removed
    }

    fn append(&self, stream: &StreamId, nonce: &[u8; APPEND_NONCE_BYTES], body: &[u8], at: i64) -> Appended {
        let mut streams = self.streams.lock().unwrap();
        let s = streams.entry(*stream).or_default();
        if let Some((seq, _, at, _)) = s.entries.iter().find(|(_, n, _, _)| n == nonce) {
            return Appended { seq: *seq, at: *at, fresh: false };
        }
        s.head += 1;
        s.last_at = at;
        s.entries.push((s.head, *nonce, at, body.to_vec()));
        Appended { seq: s.head, at, fresh: true }
    }

    fn page(&self, stream: &StreamId, after: u64, limit: usize) -> Vec<WireEntry> {
        let streams = self.streams.lock().unwrap();
        let Some(s) = streams.get(stream) else { return Vec::new() };
        s.entries
            .iter()
            .filter(|(seq, ..)| *seq > after)
            .take(limit)
            .map(|(seq, _, at, body)| WireEntry { seq: *seq, at: *at, body: Blob(body.clone()) })
            .collect()
    }

    fn head(&self, stream: &StreamId) -> u64 {
        self.streams.lock().unwrap().get(stream).map_or(0, |s| s.head)
    }

    fn prune(&self, before: i64) -> u64 {
        let mut streams = self.streams.lock().unwrap();
        let mut removed = 0u64;
        for s in streams.values_mut() {
            let had = s.entries.len();
            s.entries.retain(|(_, _, at, _)| *at >= before);
            removed += (had - s.entries.len()) as u64;
        }
        streams.retain(|_, s| !s.entries.is_empty() || s.last_at >= before);
        removed
    }
}

pub struct PgStore {
    pool: PgPool,
}

fn code_of(e: &sqlx::Error) -> Option<String> {
    match e {
        sqlx::Error::Database(db) => db.code().map(|c| c.into_owned()),
        _ => None,
    }
}

impl PgStore {
    pub async fn connect(url: &str) -> Result<Self, StoreError> {
        let options: PgConnectOptions = url.parse()?;
        let options = options.disable_statement_logging();
        let pool = match PgPoolOptions::new().max_connections(8).connect_with(options.clone()).await {
            Ok(pool) => pool,
            Err(e) if code_of(&e).as_deref() == Some(INVALID_CATALOG_NAME) => {
                Self::create_database(&options).await?;
                PgPoolOptions::new().max_connections(8).connect_with(options).await?
            }
            Err(e) => return Err(e.into()),
        };
        for statement in SCHEMA {
            sqlx::query(statement).execute(&pool).await?;
        }
        Ok(Self { pool })
    }

    async fn create_database(options: &PgConnectOptions) -> Result<(), StoreError> {
        let name = options.get_database().unwrap_or("commy_sync").to_string();
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(sqlx::Error::Configuration("the database name may only use letters, digits and _".into()).into());
        }
        let maintenance = options.clone().database("postgres");
        let mut connection = maintenance.connect().await?;
        let created = sqlx::query(&format!("CREATE DATABASE \"{name}\"")).execute(&mut connection).await;
        match created {
            Ok(_) => Ok(()),
            Err(e) if code_of(&e).as_deref() == Some("42P04") => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    async fn existing(&self, stream: &StreamId, nonce: &[u8]) -> Result<Option<Appended>, StoreError> {
        let row = sqlx::query("SELECT seq, at FROM sync_entries WHERE stream = $1 AND nonce = $2")
            .bind(&stream.0[..])
            .bind(nonce)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| Appended { seq: row.get::<i64, _>(0) as u64, at: row.get(1), fresh: false }))
    }

    async fn append(&self, stream: &StreamId, nonce: &[u8; APPEND_NONCE_BYTES], body: &[u8], at: i64) -> Result<Appended, StoreError> {
        if let Some(done) = self.existing(stream, nonce).await? {
            return Ok(done);
        }
        let mut tx = self.pool.begin().await?;
        let head: i64 = sqlx::query(
            "INSERT INTO sync_streams (id, head, last_at) VALUES ($1, 1, $2) \
             ON CONFLICT (id) DO UPDATE SET head = sync_streams.head + 1, last_at = $2 RETURNING head",
        )
        .bind(&stream.0[..])
        .bind(at)
        .fetch_one(&mut *tx)
        .await?
        .get(0);
        let inserted = sqlx::query("INSERT INTO sync_entries (stream, seq, nonce, at, body) VALUES ($1, $2, $3, $4, $5)")
            .bind(&stream.0[..])
            .bind(head)
            .bind(&nonce[..])
            .bind(at)
            .bind(body)
            .execute(&mut *tx)
            .await;
        match inserted {
            Ok(_) => {
                tx.commit().await?;
                Ok(Appended { seq: head as u64, at, fresh: true })
            }
            Err(e) if code_of(&e).as_deref() == Some(UNIQUE_VIOLATION) => {
                drop(tx);
                Ok(self.existing(stream, nonce).await?.ok_or(e)?)
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn page(&self, stream: &StreamId, after: u64, limit: usize) -> Result<Vec<WireEntry>, StoreError> {
        let rows = sqlx::query("SELECT seq, at, body FROM sync_entries WHERE stream = $1 AND seq > $2 ORDER BY seq LIMIT $3")
            .bind(&stream.0[..])
            .bind(after.min(i64::MAX as u64) as i64)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| WireEntry { seq: row.get::<i64, _>(0) as u64, at: row.get(1), body: Blob(row.get(2)) })
            .collect())
    }

    async fn head(&self, stream: &StreamId) -> Result<u64, StoreError> {
        let row = sqlx::query("SELECT head FROM sync_streams WHERE id = $1").bind(&stream.0[..]).fetch_optional(&self.pool).await?;
        Ok(row.map_or(0, |row| row.get::<i64, _>(0) as u64))
    }

    async fn register_wake(&self, tag: &[u8; 32], push: &str, until: i64, today: i64) -> Result<bool, StoreError> {
        let others: i64 = sqlx::query("SELECT count(*) FROM sync_wakes WHERE tag = $1 AND push <> $2 AND until_day >= $3")
            .bind(&tag[..])
            .bind(push)
            .bind(today)
            .fetch_one(&self.pool)
            .await?
            .get(0);
        if others as usize >= MAX_PUSHES_PER_TAG {
            return Ok(false);
        }
        sqlx::query(
            "INSERT INTO sync_wakes (tag, push, until_day) VALUES ($1, $2, $3) \
             ON CONFLICT (tag, push) DO UPDATE SET until_day = GREATEST(sync_wakes.until_day, EXCLUDED.until_day)",
        )
        .bind(&tag[..])
        .bind(push)
        .bind(until)
        .execute(&self.pool)
        .await?;
        Ok(true)
    }

    async fn wake_targets(&self, tags: &[[u8; 32]], today: i64) -> Result<Vec<String>, StoreError> {
        let tags: Vec<Vec<u8>> = tags.iter().map(|t| t.to_vec()).collect();
        let rows = sqlx::query("SELECT DISTINCT push FROM sync_wakes WHERE tag = ANY($1) AND until_day >= $2 ORDER BY push")
            .bind(&tags)
            .bind(today)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(|row| row.get(0)).collect())
    }

    async fn forget_wake(&self, tag: &[u8; 32], push: &str) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM sync_wakes WHERE tag = $1 AND push = $2").bind(&tag[..]).bind(push).execute(&self.pool).await?;
        Ok(())
    }

    async fn forget_push(&self, push: &str) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM sync_wakes WHERE push = $1").bind(push).execute(&self.pool).await?;
        Ok(())
    }

    async fn prune_wakes(&self, today: i64) -> Result<u64, StoreError> {
        Ok(sqlx::query("DELETE FROM sync_wakes WHERE until_day < $1").bind(today).execute(&self.pool).await?.rows_affected())
    }

    async fn prune(&self, before: i64) -> Result<u64, StoreError> {
        let removed = sqlx::query("DELETE FROM sync_entries WHERE at < $1").bind(before).execute(&self.pool).await?.rows_affected();
        sqlx::query("DELETE FROM sync_streams s WHERE s.last_at < $1 AND NOT EXISTS (SELECT 1 FROM sync_entries e WHERE e.stream = s.id)")
            .bind(before)
            .execute(&self.pool)
            .await?;
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_sync::StreamSecret;

    async fn exercise(store: &Store) {
        let a = StreamSecret::random().keys().id;
        let b = StreamSecret::random().keys().id;
        assert_eq!(store.head(&a).await.unwrap(), 0);
        assert!(store.page(&a, 0, 10).await.unwrap().is_empty());

        let first = store.append(&a, &[1; 16], b"one", 1_000).await.unwrap();
        assert_eq!(first, Appended { seq: 1, at: 1_000, fresh: true });
        let again = store.append(&a, &[1; 16], b"one", 9_999).await.unwrap();
        assert_eq!(again, Appended { seq: 1, at: 1_000, fresh: false }, "a retried append is not stored twice");
        assert_eq!(store.append(&a, &[2; 16], b"two", 2_000).await.unwrap().seq, 2);
        assert_eq!(store.append(&b, &[1; 16], b"other", 2_500).await.unwrap().seq, 1, "each stream counts on its own");
        assert_eq!(store.append(&a, &[3; 16], b"three", 3_000).await.unwrap().seq, 3);
        assert_eq!(store.head(&a).await.unwrap(), 3);

        let all = store.page(&a, 0, 10).await.unwrap();
        assert_eq!(all.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(all[1].body.0, b"two");
        assert_eq!(all[1].at, 2_000);
        let later = store.page(&a, 1, 1).await.unwrap();
        assert_eq!(later.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![2]);

        assert_eq!(store.prune(2_200).await.unwrap(), 2);
        assert_eq!(store.page(&a, 0, 10).await.unwrap().iter().map(|e| e.seq).collect::<Vec<_>>(), vec![3]);
        assert_eq!(store.head(&a).await.unwrap(), 3, "numbering carries on after old entries go");
        assert_eq!(store.append(&a, &[4; 16], b"four", 4_000).await.unwrap().seq, 4);
        assert_eq!(store.prune(10_000).await.unwrap(), 3);
        assert_eq!(store.head(&b).await.unwrap(), 0, "an emptied, idle stream is forgotten");
        assert!(store.ping().await);

        exercise_wakes(store).await;
    }

    async fn exercise_wakes(store: &Store) {
        let (tag, other) = ([1u8; 32], [2u8; 32]);
        assert!(store.wake_targets(&[tag], 100).await.unwrap().is_empty());
        assert!(store.register_wake(&tag, "fcm:a", 101, 100).await.unwrap());
        assert!(store.register_wake(&tag, "fcm:b", 100, 100).await.unwrap());
        assert!(store.register_wake(&other, "fcm:a", 101, 100).await.unwrap());
        assert!(store.register_wake(&tag, "fcm:a", 100, 100).await.unwrap(), "a shorter renewal keeps the longer one");
        assert_eq!(store.wake_targets(&[tag, other], 100).await.unwrap(), vec!["fcm:a", "fcm:b"], "each phone once");
        assert_eq!(store.wake_targets(&[tag], 101).await.unwrap(), vec!["fcm:a"], "expired registrations are not woken");
        for n in 0..MAX_PUSHES_PER_TAG {
            assert!(store.register_wake(&[9; 32], &format!("fcm:p{n}"), 100, 100).await.unwrap());
        }
        assert!(!store.register_wake(&[9; 32], "fcm:extra", 100, 100).await.unwrap(), "a tag takes a bounded number of phones");
        assert!(store.register_wake(&[9; 32], "fcm:p0", 101, 100).await.unwrap(), "renewing is always allowed");
        store.forget_push("fcm:a").await.unwrap();
        assert_eq!(store.wake_targets(&[tag, other], 100).await.unwrap(), vec!["fcm:b"]);
        assert_eq!(store.prune_wakes(101).await.unwrap(), MAX_PUSHES_PER_TAG as u64);
        assert!(store.wake_targets(&[tag], 100).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn memory_store_keeps_ordered_idempotent_streams() {
        exercise(&Store::memory()).await;
    }

    #[tokio::test]
    async fn postgres_store_keeps_ordered_idempotent_streams() {
        let Ok(url) = std::env::var("SYNC_POSTGRES_TEST_URL") else {
            eprintln!("SYNC_POSTGRES_TEST_URL is not set, skipping the Postgres store test");
            return;
        };
        let name = format!("commy_sync_test_{}", rand::random::<u32>());
        let url = format!("{}/{name}", url.trim_end_matches('/'));
        let store = Store::postgres(&url).await.expect("the store creates its own database");
        exercise(&store).await;
        let again = Store::postgres(&url).await.expect("the schema applies twice");
        assert!(again.ping().await);
    }

    #[tokio::test]
    async fn postgres_appends_race_without_doubling() {
        let Ok(url) = std::env::var("SYNC_POSTGRES_TEST_URL") else { return };
        let url = format!("{}/commy_sync_race_{}", url.trim_end_matches('/'), rand::random::<u32>());
        let store = std::sync::Arc::new(Store::postgres(&url).await.unwrap());
        let stream = StreamSecret::random().keys().id;
        let mut tasks = Vec::new();
        for i in 0..40u8 {
            let store = store.clone();
            tasks.push(tokio::spawn(async move { store.append(&stream, &[i % 20; 16], &[i], 1).await.unwrap().seq }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        let seqs: Vec<u64> = store.page(&stream, 0, 100).await.unwrap().iter().map(|e| e.seq).collect();
        assert_eq!(seqs, (1..=20).collect::<Vec<_>>());
    }
}
