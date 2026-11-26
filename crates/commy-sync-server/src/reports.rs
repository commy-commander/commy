use commy_core::lock_or_recover::{LockOrRecover, ReadOrRecover};
use std::collections::HashSet;
use std::sync::RwLock;

use commy_sync::report::{HostReport, Receipt};
use commy_sync::wire::{ErrorCode, ServerFrame};
use commy_sync::StreamId;
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::config::{Config, Contact};
use crate::limits::{AddressKey, Refusal};
use crate::mail::Mailer;
use crate::store::{Store, StoreError};
use crate::Service;

pub const MAX_OPEN_REPORTS: usize = 5_000;
pub const DEFAULT_OPERATOR: &str = "this sync server";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportState {
    Open,
    Dismissed,
    Dropped,
}

impl ReportState {
    pub fn as_str(self) -> &'static str {
        match self {
            ReportState::Open => "open",
            ReportState::Dismissed => "dismissed",
            ReportState::Dropped => "dropped",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "dismissed" => ReportState::Dismissed,
            "dropped" => ReportState::Dropped,
            _ => ReportState::Open,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredReport {
    pub id: String,
    pub received: i64,
    pub state: ReportState,
    pub report: HostReport,
}

#[derive(Default)]
pub struct MemoryReports {
    reports: Vec<StoredReport>,
    dropped: HashSet<StreamId>,
}

pub struct Desk {
    pub operator: String,
    pub contact: Option<Contact>,
    mailer: Option<Mailer>,
    dropped: RwLock<HashSet<StreamId>>,
}

impl Desk {
    pub fn new(config: &Config) -> Self {
        let mailer = match (&config.abuse_contact, &config.smtp) {
            (Some(Contact::Email(to)), Some(smtp)) => match Mailer::new(smtp, to) {
                Ok(mailer) => Some(mailer),
                Err(e) => {
                    tracing::warn!("sync: reports are kept but not emailed, the SMTP settings do not work: {e}");
                    None
                }
            },
            _ => None,
        };
        Self {
            operator: config.operator.clone().unwrap_or_else(|| DEFAULT_OPERATOR.to_string()),
            contact: config.abuse_contact.clone(),
            mailer,
            dropped: RwLock::new(HashSet::new()),
        }
    }

    pub fn is_dropped(&self, stream: &StreamId) -> bool {
        self.dropped.read_or_recover().contains(stream)
    }

    pub fn learn_dropped(&self, streams: impl IntoIterator<Item = StreamId>) -> Vec<StreamId> {
        let fresh: HashSet<StreamId> = streams.into_iter().collect();
        let mut held = self.dropped.write_or_recover();
        let newly: Vec<StreamId> = fresh.difference(&held).copied().collect();
        *held = fresh;
        newly
    }

    pub fn emails_reports(&self) -> bool {
        self.mailer.is_some()
    }

    pub fn about(&self) -> serde_json::Value {
        serde_json::json!({
            "operator": self.operator,
            "abuse_contact": self.contact.as_ref().map(Contact::shown),
            "takes_reports": true,
        })
    }

    fn receipt(&self, id: String) -> Receipt {
        Receipt { id, operator: self.operator.clone(), contact: self.contact.as_ref().map(|c| c.shown().to_string()) }
    }
}

pub fn new_report_id() -> String {
    data_encoding::BASE32_NOPAD.encode(&rand::random::<[u8; 10]>()).to_lowercase()
}

pub(crate) async fn take(service: &Service, key: AddressKey, report: HostReport) -> ServerFrame {
    if report.check().is_err() {
        return ServerFrame::error(ErrorCode::BadFrame);
    }
    if let Err(refusal) = service.limiter.report(key) {
        let retry_after_ms = match refusal {
            Refusal::Wait(wait) => Some(wait.as_millis().max(100) as u64),
            Refusal::Full => None,
        };
        return ServerFrame::Error { code: ErrorCode::RateLimited, stream: None, nonce: None, retry_after_ms };
    }
    let stored = StoredReport { id: new_report_id(), received: crate::now_ms(), state: ReportState::Open, report };
    match service.store.add_report(&stored).await {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!("sync: refused a report because {MAX_OPEN_REPORTS} reports are still open");
            return ServerFrame::Error { code: ErrorCode::Unavailable, stream: None, nonce: None, retry_after_ms: None };
        }
        Err(e) => {
            tracing::warn!("sync: storing a report failed: {e}");
            return ServerFrame::Error { code: ErrorCode::Unavailable, stream: None, nonce: None, retry_after_ms: Some(5_000) };
        }
    }
    tracing::info!("sync: report {} received about community {}", stored.id, stored.report.community);
    if let Some(mailer) = &service.desk.mailer {
        let mailer = mailer.clone();
        let emailed = stored.clone();
        tokio::spawn(async move {
            if let Err(e) = mailer.send(&emailed).await {
                tracing::warn!("sync: emailing report {} failed, it is still kept: {e}", emailed.id);
            }
        });
    }
    ServerFrame::Reported { receipt: service.desk.receipt(stored.id) }
}

pub fn describe(stored: &StoredReport) -> String {
    let r = &stored.report;
    let mut lines = vec![
        format!("Report {} ({})", stored.id, stored.state.as_str()),
        format!("Community: {} ({})", r.community_name, r.community),
        format!("Owner: {} ({})", r.owner.name, r.owner.id),
        format!("Reported: {} ({})", r.reported.name, r.reported.id),
        format!("Reporter: {} ({})", r.reporter.name, r.reporter.id),
        format!("Reason: {}", r.reason),
        format!("Sent: {}", r.sent_at),
    ];
    if !r.note.is_empty() {
        lines.push(format!("Note: {}", r.note));
    }
    lines.push(format!("Messages picked by the reporter ({}), as the reporter's app showed them:", r.messages.len()));
    for m in &r.messages {
        lines.push(format!("  [{}] #{} {}: {}", m.at, m.channel, m.author.name, m.text));
    }
    lines.push(format!("Streams the reporter's app listed for this community: {}", r.streams.len()));
    if let Some(command) = suspend_command(&r.reported.id) {
        lines.push(format!("If the reported account lives on your own account server, suspend it with: {command}"));
    }
    lines.join("\n")
}

const HYPHENS_AT: [usize; 4] = [8, 13, 18, 23];

fn is_commy_user_id(text: &str) -> bool {
    text.len() == 36 && text.char_indices().all(|(at, c)| if HYPHENS_AT.contains(&at) { c == '-' } else { c.is_ascii_hexdigit() })
}

pub fn suspend_command(commy_user: &str) -> Option<String> {
    is_commy_user_id(commy_user).then(|| format!("docker compose exec auth node src/suspension-cli.js suspend {} \"<reason>\"", commy_user.to_ascii_lowercase()))
}

impl Store {
    pub async fn add_report(&self, stored: &StoredReport) -> Result<bool, StoreError> {
        match self {
            Store::Memory(m) => {
                let mut books = m.reports.lock_or_recover();
                if books.reports.iter().filter(|r| r.state == ReportState::Open).count() >= MAX_OPEN_REPORTS {
                    return Ok(false);
                }
                books.reports.push(stored.clone());
                Ok(true)
            }
            Store::Postgres(p) => {
                let open: i64 = sqlx::query("SELECT COUNT(*) FROM sync_reports WHERE state = 'open'").fetch_one(&p.pool).await?.get(0);
                if open as usize >= MAX_OPEN_REPORTS {
                    return Ok(false);
                }
                let body = serde_json::to_string(&stored.report).unwrap_or_default();
                sqlx::query("INSERT INTO sync_reports (id, received, state, body) VALUES ($1, $2, $3, $4)")
                    .bind(&stored.id)
                    .bind(stored.received)
                    .bind(stored.state.as_str())
                    .bind(body)
                    .execute(&p.pool)
                    .await?;
                Ok(true)
            }
        }
    }

    pub async fn reports(&self, only_open: bool) -> Result<Vec<StoredReport>, StoreError> {
        match self {
            Store::Memory(m) => {
                let books = m.reports.lock_or_recover();
                Ok(books.reports.iter().filter(|r| !only_open || r.state == ReportState::Open).cloned().collect())
            }
            Store::Postgres(p) => {
                let rows = sqlx::query("SELECT id, received, state, body FROM sync_reports WHERE ($1 = FALSE OR state = 'open') ORDER BY received")
                    .bind(only_open)
                    .fetch_all(&p.pool)
                    .await?;
                Ok(rows.iter().filter_map(stored_from_row).collect())
            }
        }
    }

    pub async fn report(&self, id: &str) -> Result<Option<StoredReport>, StoreError> {
        match self {
            Store::Memory(m) => Ok(m.reports.lock_or_recover().reports.iter().find(|r| r.id == id).cloned()),
            Store::Postgres(p) => {
                let row = sqlx::query("SELECT id, received, state, body FROM sync_reports WHERE id = $1").bind(id).fetch_optional(&p.pool).await?;
                Ok(row.as_ref().and_then(stored_from_row))
            }
        }
    }

    pub async fn settle_report(&self, id: &str, state: ReportState) -> Result<bool, StoreError> {
        match self {
            Store::Memory(m) => {
                let mut books = m.reports.lock_or_recover();
                let Some(found) = books.reports.iter_mut().find(|r| r.id == id) else { return Ok(false) };
                found.state = state;
                Ok(true)
            }
            Store::Postgres(p) => {
                let done = sqlx::query("UPDATE sync_reports SET state = $2 WHERE id = $1").bind(id).bind(state.as_str()).execute(&p.pool).await?;
                Ok(done.rows_affected() > 0)
            }
        }
    }

    pub async fn drop_streams(&self, streams: &[StreamId], report: &str, at: i64) -> Result<u64, StoreError> {
        match self {
            Store::Memory(m) => {
                m.reports.lock_or_recover().dropped.extend(streams.iter().copied());
                let mut removed = 0;
                for stream in streams {
                    removed += m.forget_stream(stream);
                }
                Ok(removed)
            }
            Store::Postgres(p) => {
                let ids: Vec<Vec<u8>> = streams.iter().map(|s| s.0.to_vec()).collect();
                let mut tx = p.pool.begin().await?;
                sqlx::query("INSERT INTO sync_dropped (stream, dropped, report) SELECT s, $2, $3 FROM UNNEST($1::bytea[]) AS s ON CONFLICT (stream) DO NOTHING")
                    .bind(&ids)
                    .bind(at)
                    .bind(report)
                    .execute(&mut *tx)
                    .await?;
                let removed = sqlx::query("DELETE FROM sync_entries WHERE stream = ANY($1)").bind(&ids).execute(&mut *tx).await?.rows_affected();
                sqlx::query("DELETE FROM sync_streams WHERE id = ANY($1)").bind(&ids).execute(&mut *tx).await?;
                sqlx::query("DELETE FROM sync_erasers WHERE stream = ANY($1)").bind(&ids).execute(&mut *tx).await?;
                tx.commit().await?;
                Ok(removed)
            }
        }
    }

    pub async fn dropped_streams(&self) -> Result<Vec<StreamId>, StoreError> {
        match self {
            Store::Memory(m) => Ok(m.reports.lock_or_recover().dropped.iter().copied().collect()),
            Store::Postgres(p) => {
                let rows = sqlx::query("SELECT stream FROM sync_dropped").fetch_all(&p.pool).await?;
                Ok(rows.iter().filter_map(|row| row.get::<Vec<u8>, _>(0).try_into().ok().map(StreamId)).collect())
            }
        }
    }

    pub async fn prune_reports(&self, before: i64) -> Result<u64, StoreError> {
        match self {
            Store::Memory(m) => {
                let mut books = m.reports.lock_or_recover();
                let had = books.reports.len();
                books.reports.retain(|r| r.state == ReportState::Open || r.received >= before);
                Ok((had - books.reports.len()) as u64)
            }
            Store::Postgres(p) => {
                let done = sqlx::query("DELETE FROM sync_reports WHERE state <> 'open' AND received < $1").bind(before).execute(&p.pool).await?;
                Ok(done.rows_affected())
            }
        }
    }
}

fn stored_from_row(row: &sqlx::postgres::PgRow) -> Option<StoredReport> {
    let report: HostReport = serde_json::from_str(&row.get::<String, _>(3)).ok()?;
    Some(StoredReport { id: row.get(0), received: row.get(1), state: ReportState::parse(&row.get::<String, _>(2)), report })
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_sync::report::{Person, PickedMessage};
    use commy_sync::StreamSecret;

    pub(crate) fn sample(streams: Vec<StreamId>) -> HostReport {
        let person = |id: &str, name: &str| Person { id: id.into(), name: name.into() };
        HostReport {
            community: "c1".into(),
            community_name: "Night Owls".into(),
            owner: person("u-owner", "Owl"),
            reporter: person("u-me", "Ferris"),
            reported: person("u-them", "Nova"),
            reason: "spam".into(),
            note: String::new(),
            messages: vec![PickedMessage { id: "m1".into(), channel: "general".into(), author: person("u-them", "Nova"), at: "2026-11-15T04:00:00Z".into(), text: "buy now".into() }],
            streams,
            sent_at: "2026-11-15T04:01:00Z".into(),
        }
    }

    async fn exercise(store: &Store) {
        let stream = StreamSecret::random().keys().id;
        let other = StreamSecret::random().keys().id;
        store.append(&stream, &[1; 16], b"kept for now", 1).await.unwrap();
        store.append(&other, &[1; 16], b"someone else", 1).await.unwrap();
        let stored = StoredReport { id: new_report_id(), received: 10, state: ReportState::Open, report: sample(vec![stream]) };
        assert!(store.add_report(&stored).await.unwrap());
        assert_eq!(store.reports(true).await.unwrap(), vec![stored.clone()]);
        assert_eq!(store.report(&stored.id).await.unwrap(), Some(stored.clone()));
        assert_eq!(store.report("nope").await.unwrap(), None);
        assert_eq!(store.drop_streams(&stored.report.streams, &stored.id, 20).await.unwrap(), 1);
        assert!(store.settle_report(&stored.id, ReportState::Dropped).await.unwrap());
        assert!(!store.settle_report("nope", ReportState::Dismissed).await.unwrap());
        assert!(store.reports(true).await.unwrap().is_empty());
        assert_eq!(store.reports(false).await.unwrap()[0].state, ReportState::Dropped);
        assert_eq!(store.dropped_streams().await.unwrap(), vec![stream]);
        assert_eq!(store.head(&stream).await.unwrap(), 0, "a dropped stream is gone");
        assert_eq!(store.head(&other).await.unwrap(), 1, "other communities stay");
        assert_eq!(store.drop_streams(&[stream], &stored.id, 30).await.unwrap(), 0, "dropping twice is harmless");
        assert_eq!(store.prune_reports(11).await.unwrap(), 1, "settled reports age out");
        assert!(store.reports(false).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn memory_reports_are_kept_settled_and_dropped() {
        exercise(&Store::memory()).await;
    }

    #[tokio::test]
    async fn postgres_reports_are_kept_settled_and_dropped() {
        let Ok(url) = std::env::var("SYNC_POSTGRES_TEST_URL") else { return };
        let url = format!("{}/commy_sync_reports_{}", url.trim_end_matches('/'), rand::random::<u32>());
        exercise(&Store::postgres(&url).await.unwrap()).await;
    }

    #[tokio::test]
    async fn open_reports_are_capped() {
        let store = Store::memory();
        for _ in 0..MAX_OPEN_REPORTS {
            assert!(store.add_report(&StoredReport { id: new_report_id(), received: 1, state: ReportState::Open, report: sample(vec![]) }).await.unwrap());
        }
        assert!(!store.add_report(&StoredReport { id: new_report_id(), received: 1, state: ReportState::Open, report: sample(vec![]) }).await.unwrap());
    }

    #[test]
    fn the_operator_reads_what_the_reporter_picked() {
        let stored = StoredReport { id: "r1".into(), received: 1, state: ReportState::Open, report: sample(vec![]) };
        let text = describe(&stored);
        assert!(text.contains("Report r1 (open)"));
        assert!(text.contains("Community: Night Owls (c1)"));
        assert!(text.contains("#general Nova: buy now"));
        assert!(text.contains("as the reporter's app showed them"));
        assert!(!text.contains("suspension-cli"), "sample ids are not Commy ids, so no command is offered");
    }

    #[test]
    fn the_operator_is_shown_how_to_suspend_a_reported_account() {
        let mut report = sample(vec![]);
        report.reported.id = "6F1C1D6E-2B7A-4C55-9D4E-0D8A1B2C3D4E".into();
        let text = describe(&StoredReport { id: "r2".into(), received: 1, state: ReportState::Open, report });
        assert!(text.contains("docker compose exec auth node src/suspension-cli.js suspend 6f1c1d6e-2b7a-4c55-9d4e-0d8a1b2c3d4e \"<reason>\""));
        assert_eq!(suspend_command("6f1c1d6e-2b7a-4c55-9d4e-0d8a1b2c3d4"), None);
        assert_eq!(suspend_command("6f1c1d6e-2b7a-4c55-9d4e-0d8a1b2c3d4e; rm -rf /"), None);
        assert_eq!(suspend_command("6f1c1d6e2b7a-4c55-9d4e-0d8a1b2c3d4e-"), None);
        assert_eq!(suspend_command("6f1c1d6e-2b7a-4c55-9d4e-0d8a1b2c3d4g"), None);
    }

    #[test]
    fn report_ids_are_short_and_distinct() {
        let (a, b) = (new_report_id(), new_report_id());
        assert_eq!(a.len(), 16);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    }
}
