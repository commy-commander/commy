use commy_core::sync_host::clean_sync_host;
use commy_core::{ServerEvent, ServerState, UserId};

use crate::act::Act;

pub const SYNC_HOST_INVALID: &str = "Use the address of a Commy sync server on a public domain, like sync.yourcommunity.org";
pub const SYNC_HOST_OWNER_ONLY: &str = "Only the server's owner can change where it syncs";
pub const SYNC_HOST_UNCHANGED: &str = "This server already syncs there";
pub const HOST_UNREACHABLE: &str = "Commy could not reach a sync server there. Check the address and that the server is running.";
pub const HOST_TOO_OLD: &str = "That sync server needs updating before it can sync this server.";
pub const HOST_TAKEN: &str = "Someone else already holds this server's erase keys on that sync server, so Commy won't move it there.";
pub const HOST_SLOW: &str = "That sync server did not answer in time. Try again in a moment.";
pub const KEYS_NOT_READY: &str = "This server's keys are not ready yet. Try again in a moment.";

pub fn set_sync_host(state: &ServerState, me: UserId, address: Option<&str>) -> Act<ServerEvent> {
    if state.server.owner != me || state.server.group {
        return Err(SYNC_HOST_OWNER_ONLY.into());
    }
    let url = match address.map(str::trim).filter(|a| !a.is_empty()) {
        None => None,
        Some(address) => Some(clean_sync_host(address).ok_or(SYNC_HOST_INVALID)?),
    };
    if url == state.server.sync_host {
        return Err(SYNC_HOST_UNCHANGED.into());
    }
    Ok(ServerEvent::SyncHostSet { server: state.server.id, url })
}

pub fn shown_host(state: &ServerState) -> Option<String> {
    state.server.sync_host.as_deref().and_then(commy_core::sync_host::display_host).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Server, UserProfile};

    #[test]
    fn only_the_owner_picks_a_clean_sync_host() {
        let owner = UserId::new();
        let mut state = ServerState::new(Server::new("s", owner));
        let friend = UserProfile::new("friend");
        let sid = state.server.id;
        let friend_id = friend.id;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: friend }).unwrap();
        let friend = friend_id;
        let event = set_sync_host(&state, owner, Some(" Sync.Example.org ")).unwrap();
        assert!(matches!(event, ServerEvent::SyncHostSet { url: Some(ref u), .. } if u == "https://sync.example.org"));
        assert_eq!(set_sync_host(&state, friend, Some("sync.example.org")).unwrap_err(), SYNC_HOST_OWNER_ONLY);
        assert_eq!(set_sync_host(&state, owner, Some("http://sync.example.org")).unwrap_err(), SYNC_HOST_INVALID);
        assert_eq!(set_sync_host(&state, owner, Some("  ")).unwrap_err(), SYNC_HOST_UNCHANGED, "blank means the default it already has");
        state.apply(event).unwrap();
        assert_eq!(shown_host(&state).as_deref(), Some("sync.example.org"));
        assert_eq!(set_sync_host(&state, owner, Some("https://sync.example.org/")).unwrap_err(), SYNC_HOST_UNCHANGED);
        assert!(matches!(set_sync_host(&state, owner, None), Ok(ServerEvent::SyncHostSet { url: None, .. })));
    }
}
