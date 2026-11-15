use commy_core::{ServerEvent, ServerId};
use commy_sync::errands::{self, ClaimGap, ErrandError};
use commy_view::sync_host::{HOST_SLOW, HOST_TAKEN, HOST_TOO_OLD, HOST_UNREACHABLE, KEYS_NOT_READY, SYNC_HOST_INVALID};

use crate::state::App;

fn refusal(error: ErrandError) -> String {
    match error {
        ErrandError::Unreachable(_) => HOST_UNREACHABLE.into(),
        ErrandError::TooOld => HOST_TOO_OLD.into(),
    }
}

fn gap(gap: ClaimGap) -> String {
    match gap {
        ClaimGap::Taken => HOST_TAKEN.into(),
        ClaimGap::Unanswered => HOST_SLOW.into(),
    }
}

pub async fn check(origin: &str) -> Result<commy_sync::wire::Greeting, String> {
    let url = commy_core::sync_host::socket_url(origin).ok_or(SYNC_HOST_INVALID)?;
    errands::greet(&url).await.map_err(refusal)
}

pub async fn claim_everything(app: &App, sid: ServerId, url: &str) -> Result<(), String> {
    let work = crate::sync_eraser::plan(app, sid).ok_or(KEYS_NOT_READY)?.claims();
    let outcomes = errands::run(url, work).await.map_err(refusal)?;
    errands::claimed_all(&outcomes).map_err(gap)
}

pub fn target(event: &ServerEvent, account_url: &str) -> Option<String> {
    match event {
        ServerEvent::SyncHostSet { url, .. } => crate::community_sync::community_url(url.as_deref(), account_url),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_move_targets_the_chosen_host_or_the_account_servers_sync() {
        let server = ServerId::new();
        let away = ServerEvent::SyncHostSet { server, url: Some("https://sync.example.org".into()) };
        let home = ServerEvent::SyncHostSet { server, url: None };
        assert_eq!(target(&away, "wss://commy.chat/sync").as_deref(), Some("wss://sync.example.org/sync"));
        assert_eq!(target(&home, "wss://commy.chat/sync").as_deref(), Some("wss://commy.chat/sync"));
        assert_eq!(target(&ServerEvent::ServerDelete { server }, "wss://commy.chat/sync"), None);
    }

    #[test]
    fn every_refusal_reads_as_a_plain_sentence() {
        assert_eq!(refusal(ErrandError::TooOld), HOST_TOO_OLD);
        assert_eq!(refusal(ErrandError::Unreachable("dns".into())), HOST_UNREACHABLE);
        assert_eq!(gap(ClaimGap::Taken), HOST_TAKEN);
        assert_eq!(gap(ClaimGap::Unanswered), HOST_SLOW);
    }
}
