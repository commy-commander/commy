use std::collections::{BTreeMap, HashMap};

use commy_sync::stream::{StreamId, APPEND_NONCE_BYTES};
use sqlx::postgres::PgPool;
use sqlx::Row;
use tokio::sync::{mpsc, oneshot};

use crate::store::{code_of, Appended, StoreError, UNIQUE_VIOLATION};

pub const WRITER_LANES: usize = 4;
pub const MAX_BATCH_ENTRIES: usize = 128;
const MAX_BATCH_BYTES: usize = 2 * 1024 * 1024;
const LANE_QUEUE: usize = 1024;

type Nonce = [u8; APPEND_NONCE_BYTES];
type Reply = oneshot::Sender<Result<Appended, StoreError>>;

pub struct NewEntry {
    pub stream: StreamId,
    pub nonce: Nonce,
    pub body: Vec<u8>,
    pub at: i64,
}

struct Job {
    entry: NewEntry,
    reply: Reply,
}

pub struct Writers {
    lanes: Vec<mpsc::Sender<Job>>,
}

impl Writers {
    pub fn spawn(pool: PgPool) -> Self {
        let lanes = (0..WRITER_LANES)
            .map(|_| {
                let (lane, jobs) = mpsc::channel(LANE_QUEUE);
                tokio::spawn(write_lane(pool.clone(), jobs));
                lane
            })
            .collect();
        Self { lanes }
    }

    pub async fn append(&self, entry: NewEntry) -> Result<Appended, StoreError> {
        let lane = &self.lanes[lane_of(&entry.stream, self.lanes.len())];
        let (reply, answer) = oneshot::channel();
        lane.send(Job { entry, reply }).await.map_err(|_| StoreError::Database(sqlx::Error::PoolClosed))?;
        answer.await.map_err(|_| StoreError::Database(sqlx::Error::PoolClosed))?
    }
}

fn lane_of(stream: &StreamId, lanes: usize) -> usize {
    usize::from(stream.0[0]) % lanes
}

async fn write_lane(pool: PgPool, mut jobs: mpsc::Receiver<Job>) {
    while let Some(first) = jobs.recv().await {
        let mut bytes = first.entry.body.len();
        let mut batch = vec![first];
        while batch.len() < MAX_BATCH_ENTRIES && bytes < MAX_BATCH_BYTES {
            let Ok(next) = jobs.try_recv() else { break };
            bytes += next.entry.body.len();
            batch.push(next);
        }
        write_batch(&pool, batch).await;
    }
}

async fn write_batch(pool: &PgPool, batch: Vec<Job>) {
    let (entries, replies): (Vec<NewEntry>, Vec<Reply>) = batch.into_iter().map(|job| (job.entry, job.reply)).unzip();
    match append_together(pool, &entries).await {
        Ok(done) => {
            for (reply, done) in replies.into_iter().zip(done) {
                let _ = reply.send(Ok(done));
            }
        }
        Err(e) => {
            tracing::debug!("sync: a batch of {} appends goes one by one: {e}", entries.len());
            for (reply, entry) in replies.into_iter().zip(&entries) {
                let _ = reply.send(append_one(pool, entry).await);
            }
        }
    }
}

type Key = (StreamId, Nonce);

struct Plan {
    first_of: HashMap<Key, usize>,
    fresh: Vec<usize>,
    per_stream: BTreeMap<StreamId, (i64, i64)>,
}

fn plan(entries: &[NewEntry], stored: &HashMap<Key, Appended>) -> Plan {
    let mut first_of = HashMap::new();
    let mut fresh = Vec::new();
    let mut per_stream: BTreeMap<StreamId, (i64, i64)> = BTreeMap::new();
    for (index, entry) in entries.iter().enumerate() {
        let key = (entry.stream, entry.nonce);
        if stored.contains_key(&key) || first_of.contains_key(&key) {
            continue;
        }
        first_of.insert(key, index);
        fresh.push(index);
        let (count, last_at) = per_stream.entry(entry.stream).or_insert((0, entry.at));
        *count += 1;
        *last_at = (*last_at).max(entry.at);
    }
    Plan { first_of, fresh, per_stream }
}

fn number(entries: &[NewEntry], plan: &Plan, heads: &HashMap<StreamId, i64>) -> HashMap<usize, Appended> {
    let mut next: HashMap<StreamId, i64> =
        plan.per_stream.iter().map(|(stream, (count, _))| (*stream, heads.get(stream).copied().unwrap_or(*count) - count + 1)).collect();
    plan.fresh
        .iter()
        .map(|&index| {
            let entry = &entries[index];
            let seq = next.get_mut(&entry.stream).expect("every fresh stream is counted");
            let done = Appended { seq: *seq as u64, at: entry.at, fresh: true };
            *seq += 1;
            (index, done)
        })
        .collect()
}

fn answers(entries: &[NewEntry], plan: &Plan, stored: &HashMap<Key, Appended>, numbered: &HashMap<usize, Appended>) -> Vec<Appended> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let key = (entry.stream, entry.nonce);
            if let Some(done) = stored.get(&key) {
                return done.clone();
            }
            let first = plan.first_of[&key];
            let done = numbered[&first].clone();
            if first == index {
                done
            } else {
                Appended { fresh: false, ..done }
            }
        })
        .collect()
}

async fn already_stored(pool: &PgPool, entries: &[NewEntry]) -> Result<HashMap<Key, Appended>, sqlx::Error> {
    let streams: Vec<Vec<u8>> = entries.iter().map(|e| e.stream.0.to_vec()).collect();
    let nonces: Vec<Vec<u8>> = entries.iter().map(|e| e.nonce.to_vec()).collect();
    let rows = sqlx::query(
        "SELECT e.stream, e.nonce, e.seq, e.at FROM sync_entries e \
         JOIN unnest($1::bytea[], $2::bytea[]) AS q(stream, nonce) ON e.stream = q.stream AND e.nonce = q.nonce",
    )
    .bind(&streams)
    .bind(&nonces)
    .fetch_all(pool)
    .await?;
    let mut stored = HashMap::new();
    for row in rows {
        let stream: Vec<u8> = row.get(0);
        let nonce: Vec<u8> = row.get(1);
        let (Ok(stream), Ok(nonce)) = (<[u8; 32]>::try_from(stream), Nonce::try_from(nonce)) else { continue };
        stored.insert((StreamId(stream), nonce), Appended { seq: row.get::<i64, _>(2) as u64, at: row.get(3), fresh: false });
    }
    Ok(stored)
}

pub async fn append_together(pool: &PgPool, entries: &[NewEntry]) -> Result<Vec<Appended>, sqlx::Error> {
    let stored = already_stored(pool, entries).await?;
    let plan = plan(entries, &stored);
    if plan.fresh.is_empty() {
        return Ok(answers(entries, &plan, &stored, &HashMap::new()));
    }
    let mut tx = pool.begin().await?;
    let ids: Vec<Vec<u8>> = plan.per_stream.keys().map(|s| s.0.to_vec()).collect();
    let counts: Vec<i64> = plan.per_stream.values().map(|(count, _)| *count).collect();
    let last: Vec<i64> = plan.per_stream.values().map(|(_, at)| *at).collect();
    let rows = sqlx::query(
        "INSERT INTO sync_streams (id, head, last_at) SELECT * FROM unnest($1::bytea[], $2::bigint[], $3::bigint[]) \
         ON CONFLICT (id) DO UPDATE SET head = sync_streams.head + EXCLUDED.head, last_at = GREATEST(sync_streams.last_at, EXCLUDED.last_at) \
         RETURNING id, head",
    )
    .bind(&ids)
    .bind(&counts)
    .bind(&last)
    .fetch_all(&mut *tx)
    .await?;
    let mut heads = HashMap::new();
    for row in rows {
        let id: Vec<u8> = row.get(0);
        if let Ok(id) = <[u8; 32]>::try_from(id) {
            heads.insert(StreamId(id), row.get::<i64, _>(1));
        }
    }
    let numbered = number(entries, &plan, &heads);
    let fresh: Vec<&NewEntry> = plan.fresh.iter().map(|&i| &entries[i]).collect();
    sqlx::query("INSERT INTO sync_entries (stream, seq, nonce, at, body) SELECT * FROM unnest($1::bytea[], $2::bigint[], $3::bytea[], $4::bigint[], $5::bytea[])")
        .bind(fresh.iter().map(|e| e.stream.0.to_vec()).collect::<Vec<_>>())
        .bind(plan.fresh.iter().map(|i| numbered[i].seq as i64).collect::<Vec<_>>())
        .bind(fresh.iter().map(|e| e.nonce.to_vec()).collect::<Vec<_>>())
        .bind(fresh.iter().map(|e| e.at).collect::<Vec<_>>())
        .bind(fresh.iter().map(|e| e.body.clone()).collect::<Vec<_>>())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(answers(entries, &plan, &stored, &numbered))
}

async fn existing(pool: &PgPool, stream: &StreamId, nonce: &[u8]) -> Result<Option<Appended>, StoreError> {
    let row = sqlx::query("SELECT seq, at FROM sync_entries WHERE stream = $1 AND nonce = $2").bind(&stream.0[..]).bind(nonce).fetch_optional(pool).await?;
    Ok(row.map(|row| Appended { seq: row.get::<i64, _>(0) as u64, at: row.get(1), fresh: false }))
}

pub async fn append_one(pool: &PgPool, entry: &NewEntry) -> Result<Appended, StoreError> {
    let NewEntry { stream, nonce, body, at } = entry;
    if let Some(done) = existing(pool, stream, nonce).await? {
        return Ok(done);
    }
    let mut tx = pool.begin().await?;
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
            Ok(Appended { seq: head as u64, at: *at, fresh: true })
        }
        Err(e) if code_of(&e).as_deref() == Some(UNIQUE_VIOLATION) => {
            drop(tx);
            Ok(existing(pool, stream, nonce).await?.ok_or(e)?)
        }
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(stream: u8, nonce: u8, at: i64) -> NewEntry {
        NewEntry { stream: StreamId([stream; 32]), nonce: [nonce; APPEND_NONCE_BYTES], body: vec![nonce], at }
    }

    #[test]
    fn a_batch_numbers_each_stream_in_arrival_order() {
        let entries = vec![entry(1, 1, 10), entry(2, 1, 11), entry(1, 2, 12), entry(1, 1, 13), entry(1, 3, 14)];
        let stored = HashMap::new();
        let plan = plan(&entries, &stored);
        assert_eq!(plan.fresh, vec![0, 1, 2, 4], "a nonce sent twice in one batch is stored once");
        assert_eq!(plan.per_stream[&StreamId([1; 32])], (3, 14));
        assert_eq!(plan.per_stream[&StreamId([2; 32])], (1, 11));
        let heads = HashMap::from([(StreamId([1; 32]), 7), (StreamId([2; 32]), 1)]);
        let numbered = number(&entries, &plan, &heads);
        let done = answers(&entries, &plan, &stored, &numbered);
        let seqs: Vec<(u64, bool)> = done.iter().map(|d| (d.seq, d.fresh)).collect();
        assert_eq!(seqs, vec![(5, true), (1, true), (6, true), (5, false), (7, true)]);
    }

    #[test]
    fn entries_already_stored_keep_their_numbers() {
        let entries = vec![entry(1, 1, 10), entry(1, 2, 20)];
        let stored = HashMap::from([((StreamId([1; 32]), [1; APPEND_NONCE_BYTES]), Appended { seq: 4, at: 3, fresh: false })]);
        let plan = plan(&entries, &stored);
        assert_eq!(plan.fresh, vec![1]);
        let numbered = number(&entries, &plan, &HashMap::from([(StreamId([1; 32]), 5)]));
        let done = answers(&entries, &plan, &stored, &numbered);
        assert_eq!(done, vec![Appended { seq: 4, at: 3, fresh: false }, Appended { seq: 5, at: 20, fresh: true }]);
    }

    async fn test_pool() -> Option<PgPool> {
        let url = std::env::var("SYNC_POSTGRES_TEST_URL").ok()?;
        let url = format!("{}/commy_sync_together_{}", url.trim_end_matches('/'), rand::random::<u32>());
        let crate::store::Store::Postgres(store) = crate::store::Store::postgres(&url).await.unwrap() else { unreachable!() };
        Some(store.pool.clone())
    }

    #[tokio::test]
    async fn postgres_takes_a_whole_batch_in_one_go() {
        let Some(pool) = test_pool().await else { return };
        let a = commy_sync::StreamSecret::random().keys().id;
        let b = commy_sync::StreamSecret::random().keys().id;
        let at = |stream: StreamId, nonce: u8| NewEntry { stream, nonce: [nonce; APPEND_NONCE_BYTES], body: vec![nonce; 3], at: i64::from(nonce) };
        let first = append_together(&pool, &[at(a, 1), at(b, 1), at(a, 2), at(a, 1)]).await.expect("the batch is taken whole");
        let seqs: Vec<(u64, bool)> = first.iter().map(|d| (d.seq, d.fresh)).collect();
        assert_eq!(seqs, vec![(1, true), (1, true), (2, true), (1, false)]);
        let second = append_together(&pool, &[at(a, 2), at(a, 3), at(b, 2)]).await.expect("the next batch too");
        let seqs: Vec<(u64, bool)> = second.iter().map(|d| (d.seq, d.fresh)).collect();
        assert_eq!(seqs, vec![(2, false), (3, true), (2, true)]);
        let retried = append_together(&pool, &[at(b, 2)]).await.unwrap();
        assert_eq!(retried, vec![Appended { seq: 2, at: 2, fresh: false }]);
        let one = append_one(&pool, &at(a, 9)).await.unwrap();
        assert_eq!(one, Appended { seq: 4, at: 9, fresh: true }, "one at a time carries on the same numbering");
    }

    #[test]
    fn a_stream_always_writes_through_the_same_lane() {
        let stream = StreamId([9; 32]);
        assert_eq!(lane_of(&stream, WRITER_LANES), lane_of(&stream, WRITER_LANES));
        assert!((0..=255u8).all(|b| lane_of(&StreamId([b; 32]), WRITER_LANES) < WRITER_LANES));
    }
}
