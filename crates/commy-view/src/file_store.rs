use commy_core::file_store::clean_file_store;
use commy_core::{Permissions, ServerEvent, ServerState};

use crate::act::Act;
use crate::manage::need;

pub const FILE_STORE_INVALID: &str = "Use the https address of your file storage, like https://commy-files.yourname.workers.dev";

pub fn set_file_store(state: &ServerState, perms: Permissions, address: Option<&str>) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_FILE_STORAGE, "change where this server keeps files")?;
    let url = match address.map(str::trim).filter(|a| !a.is_empty()) {
        None => None,
        Some(address) => Some(clean_file_store(address).ok_or(FILE_STORE_INVALID)?),
    };
    Ok(ServerEvent::FileStoreSet { server: state.server.id, url })
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Server, UserId};

    #[test]
    fn managers_point_the_server_at_their_own_storage() {
        let state = ServerState::new(Server::new("Rockets", UserId::new()));
        let event = set_file_store(&state, Permissions::all(), Some(" https://commy-files.rockets.workers.dev/ ")).unwrap();
        assert!(matches!(event, ServerEvent::FileStoreSet { url: Some(ref u), .. } if u == "https://commy-files.rockets.workers.dev"));
        assert!(matches!(set_file_store(&state, Permissions::all(), Some("  ")), Ok(ServerEvent::FileStoreSet { url: None, .. })));
        assert_eq!(set_file_store(&state, Permissions::all(), Some("http://files.example.org")).unwrap_err(), FILE_STORE_INVALID);
        assert!(set_file_store(&state, Permissions::default_member(), Some("https://files.example.org")).is_err());
        assert!(set_file_store(&state, Permissions::MANAGE_SERVER, None).is_err(), "Manage Server alone does not reach storage");
        assert!(set_file_store(&state, Permissions::MANAGE_FILE_STORAGE, None).is_ok());
    }
}
