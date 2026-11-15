use std::collections::HashMap;

use sqlx::Row;

use crate::ramp::{CutOff, Day, ServerRecord, Tier};
use crate::store::{Store, StoreError};

pub const GATEWAY_SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS sync_gateway_servers (origin TEXT PRIMARY KEY, first_seen BIGINT NOT NULL, clean_since BIGINT NOT NULL, cut_at BIGINT, cut_reason TEXT, cut_auto BOOLEAN NOT NULL DEFAULT FALSE, pinned TEXT)",
    "CREATE TABLE IF NOT EXISTS sync_gateway_days (origin TEXT NOT NULL, day BIGINT NOT NULL, pushes BIGINT NOT NULL, refused BIGINT NOT NULL, new_phones BIGINT NOT NULL, PRIMARY KEY (origin, day))",
    "CREATE INDEX IF NOT EXISTS sync_gateway_days_day ON sync_gateway_days (day)",
    "CREATE TABLE IF NOT EXISTS sync_gateway_phones (origin TEXT NOT NULL, phone BYTEA NOT NULL, last_day BIGINT NOT NULL, PRIMARY KEY (origin, phone))",
];

#[derive(Default)]
pub struct MemoryGateway {
    servers: HashMap<String, ServerRecord>,
    days: HashMap<(String, i64), Day>,
    phones: HashMap<(String, [u8; 32]), i64>,
}

fn record_from_row(row: &sqlx::postgres::PgRow) -> ServerRecord {
    let cut_at: Option<i64> = row.get("cut_at");
    let pinned: Option<String> = row.get("pinned");
    ServerRecord {
        origin: row.get("origin"),
        first_seen: row.get("first_seen"),
        clean_since: row.get("clean_since"),
        cut_off: cut_at.map(|at| CutOff { at, reason: row.get::<Option<String>, _>("cut_reason").unwrap_or_default(), automatic: row.get("cut_auto") }),
        pinned: pinned.as_deref().and_then(Tier::parse),
    }
}

fn day_from_row(row: &sqlx::postgres::PgRow) -> Day {
    let count = |name: &str| row.get::<i64, _>(name).max(0) as u64;
    Day { pushes: count("pushes"), refused: count("refused"), new_phones: count("new_phones") }
}

const SERVER_COLUMNS: &str = "origin, first_seen, clean_since, cut_at, cut_reason, cut_auto, pinned";

impl Store {
    pub async fn gateway_server(&self, origin: &str) -> Result<Option<ServerRecord>, StoreError> {
        match self {
            Store::Memory(m) => Ok(m.gateway.lock().unwrap().servers.get(origin).cloned()),
            Store::Postgres(p) => {
                let row = sqlx::query(&format!("SELECT {SERVER_COLUMNS} FROM sync_gateway_servers WHERE origin = $1")).bind(origin).fetch_optional(&p.pool).await?;
                Ok(row.as_ref().map(record_from_row))
            }
        }
    }

    pub async fn gateway_servers(&self) -> Result<Vec<ServerRecord>, StoreError> {
        match self {
            Store::Memory(m) => {
                let mut all: Vec<ServerRecord> = m.gateway.lock().unwrap().servers.values().cloned().collect();
                all.sort_by(|a, b| a.origin.cmp(&b.origin));
                Ok(all)
            }
            Store::Postgres(p) => {
                let rows = sqlx::query(&format!("SELECT {SERVER_COLUMNS} FROM sync_gateway_servers ORDER BY origin")).fetch_all(&p.pool).await?;
                Ok(rows.iter().map(record_from_row).collect())
            }
        }
    }

    pub async fn put_gateway_server(&self, record: &ServerRecord) -> Result<(), StoreError> {
        match self {
            Store::Memory(m) => {
                m.gateway.lock().unwrap().servers.insert(record.origin.clone(), record.clone());
                Ok(())
            }
            Store::Postgres(p) => {
                let cut = record.cut_off.as_ref();
                sqlx::query(
                    "INSERT INTO sync_gateway_servers (origin, first_seen, clean_since, cut_at, cut_reason, cut_auto, pinned) VALUES ($1, $2, $3, $4, $5, $6, $7) \
                     ON CONFLICT (origin) DO UPDATE SET clean_since = EXCLUDED.clean_since, cut_at = EXCLUDED.cut_at, cut_reason = EXCLUDED.cut_reason, cut_auto = EXCLUDED.cut_auto, pinned = EXCLUDED.pinned",
                )
                .bind(&record.origin)
                .bind(record.first_seen)
                .bind(record.clean_since)
                .bind(cut.map(|c| c.at))
                .bind(cut.map(|c| c.reason.clone()))
                .bind(cut.is_some_and(|c| c.automatic))
                .bind(record.pinned.map(Tier::as_str))
                .execute(&p.pool)
                .await?;
                Ok(())
            }
        }
    }

    pub async fn gateway_day(&self, origin: &str, day: i64) -> Result<Day, StoreError> {
        match self {
            Store::Memory(m) => Ok(m.gateway.lock().unwrap().days.get(&(origin.to_string(), day)).copied().unwrap_or_default()),
            Store::Postgres(p) => {
                let row = sqlx::query("SELECT pushes, refused, new_phones FROM sync_gateway_days WHERE origin = $1 AND day = $2")
                    .bind(origin)
                    .bind(day)
                    .fetch_optional(&p.pool)
                    .await?;
                Ok(row.as_ref().map(day_from_row).unwrap_or_default())
            }
        }
    }

    pub async fn gateway_days(&self, day: i64) -> Result<Vec<(String, Day)>, StoreError> {
        match self {
            Store::Memory(m) => {
                let gateway = m.gateway.lock().unwrap();
                let mut all: Vec<(String, Day)> = gateway.days.iter().filter(|((_, d), _)| *d == day).map(|((origin, _), day)| (origin.clone(), *day)).collect();
                all.sort_by(|a, b| b.1.pushes.cmp(&a.1.pushes).then_with(|| a.0.cmp(&b.0)));
                Ok(all)
            }
            Store::Postgres(p) => {
                let rows = sqlx::query("SELECT origin, pushes, refused, new_phones FROM sync_gateway_days WHERE day = $1 ORDER BY pushes DESC, origin")
                    .bind(day)
                    .fetch_all(&p.pool)
                    .await?;
                Ok(rows.iter().map(|row| (row.get("origin"), day_from_row(row))).collect())
            }
        }
    }

    pub async fn count_gateway(&self, origin: &str, day: i64, add: Day) -> Result<(), StoreError> {
        match self {
            Store::Memory(m) => {
                let mut gateway = m.gateway.lock().unwrap();
                let kept = gateway.days.entry((origin.to_string(), day)).or_default();
                kept.pushes += add.pushes;
                kept.refused += add.refused;
                kept.new_phones += add.new_phones;
                Ok(())
            }
            Store::Postgres(p) => {
                sqlx::query(
                    "INSERT INTO sync_gateway_days (origin, day, pushes, refused, new_phones) VALUES ($1, $2, $3, $4, $5) \
                     ON CONFLICT (origin, day) DO UPDATE SET pushes = sync_gateway_days.pushes + EXCLUDED.pushes, refused = sync_gateway_days.refused + EXCLUDED.refused, new_phones = sync_gateway_days.new_phones + EXCLUDED.new_phones",
                )
                .bind(origin)
                .bind(day)
                .bind(add.pushes as i64)
                .bind(add.refused as i64)
                .bind(add.new_phones as i64)
                .execute(&p.pool)
                .await?;
                Ok(())
            }
        }
    }

    pub async fn known_phone(&self, origin: &str, phone: &[u8; 32], day: i64) -> Result<bool, StoreError> {
        match self {
            Store::Memory(m) => {
                let mut gateway = m.gateway.lock().unwrap();
                Ok(gateway.phones.get_mut(&(origin.to_string(), *phone)).map(|last| *last = day).is_some())
            }
            Store::Postgres(p) => {
                let touched = sqlx::query("UPDATE sync_gateway_phones SET last_day = $3 WHERE origin = $1 AND phone = $2")
                    .bind(origin)
                    .bind(phone.as_slice())
                    .bind(day)
                    .execute(&p.pool)
                    .await?;
                Ok(touched.rows_affected() > 0)
            }
        }
    }

    pub async fn remember_phone(&self, origin: &str, phone: &[u8; 32], day: i64) -> Result<(), StoreError> {
        match self {
            Store::Memory(m) => {
                m.gateway.lock().unwrap().phones.insert((origin.to_string(), *phone), day);
                Ok(())
            }
            Store::Postgres(p) => {
                sqlx::query("INSERT INTO sync_gateway_phones (origin, phone, last_day) VALUES ($1, $2, $3) ON CONFLICT (origin, phone) DO UPDATE SET last_day = EXCLUDED.last_day")
                    .bind(origin)
                    .bind(phone.as_slice())
                    .bind(day)
                    .execute(&p.pool)
                    .await?;
                Ok(())
            }
        }
    }

    pub async fn prune_gateway(&self, before_day: i64) -> Result<(), StoreError> {
        match self {
            Store::Memory(m) => {
                let mut gateway = m.gateway.lock().unwrap();
                gateway.days.retain(|(_, day), _| *day >= before_day);
                gateway.phones.retain(|_, last| *last >= before_day);
                Ok(())
            }
            Store::Postgres(p) => {
                sqlx::query("DELETE FROM sync_gateway_days WHERE day < $1").bind(before_day).execute(&p.pool).await?;
                sqlx::query("DELETE FROM sync_gateway_phones WHERE last_day < $1").bind(before_day).execute(&p.pool).await?;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn exercise(store: &Store) {
        let origin = format!("https://home{}.example.org", rand::random::<u32>());
        assert_eq!(store.gateway_server(&origin).await.unwrap(), None);
        let mut record = ServerRecord::first(&origin, 1_000);
        store.put_gateway_server(&record).await.unwrap();
        assert_eq!(store.gateway_server(&origin).await.unwrap(), Some(record.clone()));
        record.cut(2_000, "flooded", true);
        record.pinned = Some(Tier::Settled);
        store.put_gateway_server(&record).await.unwrap();
        let kept = store.gateway_server(&origin).await.unwrap().unwrap();
        assert_eq!(kept, record);
        assert!(store.gateway_servers().await.unwrap().iter().any(|r| r.origin == origin));
        record.restore(3_000);
        record.pinned = None;
        store.put_gateway_server(&record).await.unwrap();
        assert_eq!(store.gateway_server(&origin).await.unwrap(), Some(record));

        store.count_gateway(&origin, 5, Day { pushes: 2, refused: 1, new_phones: 1 }).await.unwrap();
        store.count_gateway(&origin, 5, Day { pushes: 3, refused: 0, new_phones: 2 }).await.unwrap();
        assert_eq!(store.gateway_day(&origin, 5).await.unwrap(), Day { pushes: 5, refused: 1, new_phones: 3 });
        assert_eq!(store.gateway_day(&origin, 6).await.unwrap(), Day::default());
        assert!(store.gateway_days(5).await.unwrap().iter().any(|(o, d)| *o == origin && d.pushes == 5));

        let phone = [7u8; 32];
        assert!(!store.known_phone(&origin, &phone, 5).await.unwrap());
        store.remember_phone(&origin, &phone, 5).await.unwrap();
        assert!(store.known_phone(&origin, &phone, 6).await.unwrap());
        store.prune_gateway(7).await.unwrap();
        assert_eq!(store.gateway_day(&origin, 5).await.unwrap(), Day::default());
        assert!(!store.known_phone(&origin, &phone, 7).await.unwrap(), "a phone unseen for the kept days is forgotten");
    }

    #[tokio::test]
    async fn the_memory_store_keeps_gateway_records() {
        exercise(&Store::memory()).await;
    }

    #[tokio::test]
    async fn postgres_keeps_gateway_records() {
        let Ok(url) = std::env::var("SYNC_POSTGRES_TEST_URL") else { return };
        let url = format!("{}/commy_sync_gateway_{}", url.trim_end_matches('/'), rand::random::<u32>());
        exercise(&Store::postgres(&url).await.unwrap()).await;
    }
}
