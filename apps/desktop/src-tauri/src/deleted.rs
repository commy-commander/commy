use std::collections::HashMap;
use std::path::Path;

use commy_core::{ServerEvent, ServerId, SignedEvent};
use serde::{Deserialize, Serialize};

pub const RELAY_DAYS: i64 = 30;
const DAY_MS: i64 = 86_400_000;
const FILE: &str = "deleted-servers.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeletedServer {
    pub name: String,
    pub notice: SignedEvent,
    #[serde(default)]
    pub ticket: Option<String>,
}

impl DeletedServer {
    fn relaying(&self, now_ms: i64) -> bool {
        now_ms.saturating_sub(self.notice.action.at) < RELAY_DAYS * DAY_MS
    }
}

#[derive(Debug, Default)]
pub struct DeletedServers {
    servers: HashMap<ServerId, DeletedServer>,
}

pub fn is_notice(event: &ServerEvent) -> bool {
    matches!(event, ServerEvent::ServerDelete { .. })
}

impl DeletedServers {
    pub fn load(dir: &Path, now_ms: i64) -> Self {
        let saved: HashMap<ServerId, DeletedServer> = crate::persist::read_json(&dir.join(FILE)).unwrap_or_default();
        let mut deleted = Self { servers: saved };
        deleted.drop_expired(now_ms);
        deleted
    }

    pub fn save(&self, dir: &Path) {
        if let Err(e) = crate::persist::write_json(&dir.join(FILE), &self.servers) {
            tracing::warn!("could not save deleted servers: {e}");
        }
    }

    fn drop_expired(&mut self, now_ms: i64) -> bool {
        let before = self.servers.len();
        self.servers.retain(|_, d| d.relaying(now_ms));
        self.servers.len() != before
    }

    pub fn insert(&mut self, server: ServerId, deleted: DeletedServer) {
        self.servers.insert(server, deleted);
    }

    pub fn contains(&self, server: ServerId) -> bool {
        self.servers.contains_key(&server)
    }

    pub fn notice(&self, server: ServerId) -> Option<SignedEvent> {
        self.servers.get(&server).map(|d| d.notice.clone())
    }

    pub fn to_rejoin(&self) -> Vec<(ServerId, DeletedServer)> {
        self.servers.iter().map(|(id, d)| (*id, d.clone())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Identity, UserId};

    fn notice(server: ServerId, at: i64) -> SignedEvent {
        SignedEvent::sign(server, UserId::new(), at, ServerEvent::ServerDelete { server }, &Identity::generate())
    }

    #[test]
    fn deleted_servers_are_kept_for_relaying_then_forgotten() {
        let dir = std::env::temp_dir().join(format!("commy-deleted-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let now = 100 * DAY_MS;
        let (fresh, stale) = (ServerId::new(), ServerId::new());
        let mut deleted = DeletedServers::default();
        deleted.insert(fresh, DeletedServer { name: "fresh".into(), notice: notice(fresh, now - DAY_MS), ticket: Some("t".into()) });
        deleted.insert(stale, DeletedServer { name: "stale".into(), notice: notice(stale, now - RELAY_DAYS * DAY_MS), ticket: None });
        deleted.save(&dir);

        let loaded = DeletedServers::load(&dir, now);
        assert!(loaded.contains(fresh));
        assert!(!loaded.contains(stale), "past the relay window");
        assert_eq!(loaded.notice(fresh), Some(notice_of(&deleted, fresh)));
        assert_eq!(loaded.to_rejoin().len(), 1);
        assert!(DeletedServers::load(&dir.join("missing"), now).to_rejoin().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn notice_of(deleted: &DeletedServers, server: ServerId) -> SignedEvent {
        deleted.notice(server).unwrap()
    }

    #[test]
    fn only_a_deletion_is_a_notice() {
        let server = ServerId::new();
        assert!(is_notice(&ServerEvent::ServerDelete { server }));
        assert!(!is_notice(&ServerEvent::PublicSet { server, public: true }));
    }
}
