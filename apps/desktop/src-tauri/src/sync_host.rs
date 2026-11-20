use commy_core::{ServerEvent, ServerId};
use commy_sync::errands::{self, ClaimGap, ErrandError};
use commy_view::sync_host::{HOST_ON_BLOCKLIST, HOST_SLOW, HOST_TAKEN, HOST_TOO_OLD, HOST_UNREACHABLE, KEYS_NOT_READY, SYNC_HOST_INVALID};

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

pub const NOT_CONNECTED: &str = "Commy is not connected to its sync service yet";

pub fn target(event: &ServerEvent, account_url: &str, blocked: impl Fn(&str) -> bool) -> Result<String, &'static str> {
    let ServerEvent::SyncHostSet { url, .. } = event else { return Err(SYNC_HOST_INVALID) };
    let socket = commy_core::sync_host::chosen_socket(url.as_deref(), Some(account_url)).ok_or(SYNC_HOST_INVALID)?;
    if blocked(&socket) {
        return Err(HOST_ON_BLOCKLIST);
    }
    Ok(socket)
}

pub fn pin_origin(account_url: Option<&str>, blocked: impl Fn(&str) -> bool) -> Result<Option<String>, &'static str> {
    let here = account_url.and_then(commy_core::sync_host::clean_sync_host).ok_or(NOT_CONNECTED)?;
    Ok((!blocked(&here)).then_some(here))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_move_targets_the_chosen_host_or_the_account_servers_sync() {
        let server = ServerId::new();
        let away = ServerEvent::SyncHostSet { server, url: Some("https://sync.example.org".into()) };
        let home = ServerEvent::SyncHostSet { server, url: None };
        let open = |_: &str| false;
        assert_eq!(target(&away, "wss://commy.chat/sync", open).as_deref(), Ok("wss://sync.example.org/sync"));
        assert_eq!(target(&home, "wss://commy.chat/sync", open).as_deref(), Ok("wss://commy.chat/sync"));
        assert_eq!(target(&ServerEvent::ServerDelete { server }, "wss://commy.chat/sync", open), Err(SYNC_HOST_INVALID));
    }

    #[test]
    fn a_move_to_a_blocked_sync_server_says_so() {
        let server = ServerId::new();
        let away = ServerEvent::SyncHostSet { server, url: Some("https://sync.example.org".into()) };
        let blocks_example = |socket: &str| socket.contains("example.org");
        assert_eq!(target(&away, "wss://commy.chat/sync", blocks_example), Err(HOST_ON_BLOCKLIST));
        assert_eq!(target(&ServerEvent::SyncHostSet { server, url: None }, "wss://commy.chat/sync", blocks_example).as_deref(), Ok("wss://commy.chat/sync"));
    }

    #[test]
    fn a_server_on_a_blocked_home_is_not_pinned_there() {
        let open = |_: &str| false;
        let blocks_dev = |origin: &str| origin == "https://dev.example.org";
        assert_eq!(pin_origin(Some("wss://dev.example.org/sync"), open), Ok(Some("https://dev.example.org".into())));
        assert_eq!(pin_origin(Some("wss://dev.example.org/sync"), blocks_dev), Ok(None), "it follows its owner to the new home instead");
        assert_eq!(pin_origin(None, open), Err(NOT_CONNECTED));
    }

    #[test]
    fn every_refusal_reads_as_a_plain_sentence() {
        assert_eq!(refusal(ErrandError::TooOld), HOST_TOO_OLD);
        assert_eq!(refusal(ErrandError::Unreachable("dns".into())), HOST_UNREACHABLE);
        assert_eq!(gap(ClaimGap::Taken), HOST_TAKEN);
        assert_eq!(gap(ClaimGap::Unanswered), HOST_SLOW);
    }
}
