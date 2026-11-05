use commy_core::search_keys::{GameDbKey, GifKey};
use commy_core::{Permissions, ServerEvent, ServerState};
use serde::Serialize;

use crate::act::Act;
use crate::manage::need;

pub const GIF_KEY_INVALID: &str = "Choose KLIPY or GIPHY and paste the key exactly as it was given";
pub const GAME_DB_INVALID: &str = "Enter both the client id and the client secret";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GameDbView {
    pub client_id: String,
}

pub fn set_gif_key(state: &ServerState, perms: Permissions, provider: Option<&str>, key: Option<&str>) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_SERVER, "change this server's GIF search")?;
    let gifs = match key.map(str::trim).filter(|k| !k.is_empty()) {
        None => None,
        Some(key) => Some(GifKey::new(provider.unwrap_or("klipy"), key).ok_or(GIF_KEY_INVALID)?),
    };
    Ok(ServerEvent::GifKeySet { server: state.server.id, gifs })
}

pub fn game_db_login(state: &ServerState, client_id: Option<&str>, client_secret: Option<&str>) -> Result<Option<GameDbKey>, String> {
    let Some(client_id) = client_id.map(str::trim).filter(|c| !c.is_empty()) else { return Ok(None) };
    let kept = state.server.game_db.as_ref().map(|g| g.client_secret.as_str());
    let secret = client_secret.map(str::trim).filter(|s| !s.is_empty()).or(kept).unwrap_or("");
    GameDbKey::new(client_id, secret).map(Some).ok_or_else(|| GAME_DB_INVALID.to_string())
}

pub fn set_game_db(state: &ServerState, perms: Permissions, games: Option<GameDbKey>) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_SERVER, "change this server's game database")?;
    Ok(ServerEvent::GameDbSet { server: state.server.id, games })
}

pub fn game_db_view(state: &ServerState, perms: Permissions) -> Option<GameDbView> {
    perms.allows(Permissions::MANAGE_SERVER).then(|| state.server.game_db.as_ref().map(|g| GameDbView { client_id: g.client_id.clone() })).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Server, UserId};

    #[test]
    fn managers_give_the_server_a_gif_key() {
        let state = ServerState::new(Server::new("Rockets", UserId::new()));
        let event = set_gif_key(&state, Permissions::all(), Some("giphy"), Some(" abc ")).unwrap();
        assert!(matches!(event, ServerEvent::GifKeySet { gifs: Some(ref g), .. } if g.provider == "giphy" && g.key == "abc"));
        assert!(matches!(set_gif_key(&state, Permissions::all(), None, Some(" ")), Ok(ServerEvent::GifKeySet { gifs: None, .. })));
        assert_eq!(set_gif_key(&state, Permissions::all(), Some("klipy"), Some("a b")).unwrap_err(), GIF_KEY_INVALID);
        assert!(set_gif_key(&state, Permissions::default_member(), Some("klipy"), Some("abc")).is_err());
    }

    #[test]
    fn a_saved_secret_is_kept_when_only_the_client_id_is_sent() {
        let mut state = ServerState::new(Server::new("Rockets", UserId::new()));
        assert_eq!(game_db_login(&state, Some("id1"), None).unwrap_err(), GAME_DB_INVALID);
        assert_eq!(game_db_login(&state, Some(" "), Some("s")), Ok(None));
        state.server.game_db = GameDbKey::new("id1", "secret2");
        assert_eq!(game_db_login(&state, Some("id9"), None), Ok(GameDbKey::new("id9", "secret2")));
        assert_eq!(game_db_login(&state, Some("id9"), Some("new3")), Ok(GameDbKey::new("id9", "new3")));
        assert!(set_game_db(&state, Permissions::default_member(), None).is_err());
        assert_eq!(game_db_view(&state, Permissions::all()), Some(GameDbView { client_id: "id1".into() }));
        assert_eq!(game_db_view(&state, Permissions::default_member()), None);
    }
}
