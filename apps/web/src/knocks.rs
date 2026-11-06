use std::collections::HashMap;

use commy_core::channel_key::{ChannelSecret, KeyId};
use commy_core::ServerId;
use commy_net::Lobby;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const KNOCKS_KEY: &str = "commy.web.knocks";
pub const NOT_WAITING: &str = "You are not waiting to join that server";

pub fn knocks_key(account: &str) -> String {
    format!("{KNOCKS_KEY}.{account}")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Knocking {
    pub ticket: String,
    key_id: String,
    secret: String,
    pub name: String,
    pub rules: Option<String>,
    pub members: u32,
    pub note: String,
    pub asked_at: i64,
    #[serde(default)]
    pub turned_away: bool,
}

impl Knocking {
    pub fn key(&self) -> Option<(KeyId, ChannelSecret)> {
        let id: [u8; 16] = data_encoding::HEXLOWER.decode(self.key_id.as_bytes()).ok()?.try_into().ok()?;
        let secret: [u8; 32] = data_encoding::HEXLOWER.decode(self.secret.as_bytes()).ok()?.try_into().ok()?;
        Some((KeyId(id), ChannelSecret::from_bytes(secret)))
    }

    fn learn(&mut self, lobby: Lobby) {
        self.name = lobby.name;
        self.rules = lobby.rules;
        self.members = lobby.members;
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Knocks {
    servers: HashMap<ServerId, Knocking>,
}

impl Knocks {
    pub fn parse(raw: Option<&str>) -> Self {
        raw.and_then(|raw| serde_json::from_str(raw).ok()).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn ask(&mut self, server: ServerId, ticket: &str, key: &(KeyId, ChannelSecret), lobby: Lobby, note: &str, asked_at: i64) {
        let mut knock = Knocking {
            ticket: ticket.to_string(),
            key_id: data_encoding::HEXLOWER.encode(&key.0 .0),
            secret: data_encoding::HEXLOWER.encode(key.1.bytes()),
            name: String::new(),
            rules: None,
            members: 0,
            note: note.trim().to_string(),
            asked_at,
            turned_away: false,
        };
        knock.learn(lobby);
        self.servers.insert(server, knock);
    }

    pub fn get(&self, server: ServerId) -> Option<&Knocking> {
        self.servers.get(&server)
    }

    pub fn learn(&mut self, server: ServerId, lobby: Lobby) {
        if let Some(knock) = self.servers.get_mut(&server) {
            knock.learn(lobby);
        }
    }

    pub fn turn_away(&mut self, server: ServerId) -> bool {
        match self.servers.get_mut(&server) {
            Some(knock) if !knock.turned_away => {
                knock.turned_away = true;
                true
            }
            _ => false,
        }
    }

    pub fn remove(&mut self, server: ServerId) -> Option<Knocking> {
        self.servers.remove(&server)
    }

    pub fn set_note(&mut self, server: ServerId, note: &str) -> Result<Knocking, String> {
        let note = commy_core::join_request::clean_note(note).ok_or(commy_view::join_requests::NOTE_TOO_LONG)?;
        let knock = self.servers.get_mut(&server).filter(|k| !k.turned_away).ok_or(NOT_WAITING)?;
        knock.note = note;
        Ok(knock.clone())
    }

    pub fn waiting(&self) -> Vec<(ServerId, Knocking)> {
        let mut list: Vec<(ServerId, Knocking)> = self.servers.iter().filter(|(_, k)| !k.turned_away).map(|(id, k)| (*id, k.clone())).collect();
        list.sort_by_key(|(_, k)| k.asked_at);
        list
    }

    pub fn view(&self, server: ServerId) -> Option<Value> {
        self.servers.get(&server).map(|knock| view_of(server, knock))
    }

    pub fn views(&self) -> Value {
        let mut list: Vec<(&ServerId, &Knocking)> = self.servers.iter().collect();
        list.sort_by_key(|(_, k)| k.asked_at);
        Value::Array(list.into_iter().map(|(id, k)| view_of(*id, k)).collect())
    }
}

fn view_of(server: ServerId, knock: &Knocking) -> Value {
    json!({
        "id": server.to_string(),
        "name": knock.name,
        "rules": knock.rules,
        "members": knock.members,
        "note": knock.note,
        "asked_at": knock.asked_at,
        "turned_away": knock.turned_away,
        "waiting": true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lobby(name: &str) -> Lobby {
        Lobby { name: name.to_string(), rules: Some("Be kind".to_string()), members: 12 }
    }

    fn key() -> (KeyId, ChannelSecret) {
        (KeyId([7; 16]), ChannelSecret::from_bytes([9; 32]))
    }

    #[test]
    fn a_waiting_request_keeps_its_invite_key_and_shows_like_the_desktop() {
        let server = ServerId::new();
        let mut knocks = Knocks::default();
        knocks.ask(server, "commyticket", &key(), lobby("Rust Hangout"), "  hi, I know Sam  ", 50);
        let saved = Knocks::parse(Some(&knocks.to_json()));
        let knock = saved.get(server).unwrap();
        assert_eq!(knock.key().unwrap().0, KeyId([7; 16]));
        assert_eq!(knock.key().unwrap().1.bytes(), &[9; 32]);
        assert_eq!(
            saved.views(),
            json!([{ "id": server.to_string(), "name": "Rust Hangout", "rules": "Be kind", "members": 12, "note": "hi, I know Sam", "asked_at": 50, "turned_away": false, "waiting": true }])
        );
    }

    #[test]
    fn a_note_can_change_while_waiting_but_not_after_being_turned_away() {
        let server = ServerId::new();
        let mut knocks = Knocks::default();
        knocks.ask(server, "t", &key(), lobby("A"), "", 1);
        assert_eq!(knocks.set_note(server, "please").unwrap().note, "please");
        assert!(knocks.set_note(server, &"x".repeat(5000)).is_err());
        assert!(knocks.turn_away(server));
        assert!(!knocks.turn_away(server));
        assert_eq!(knocks.set_note(server, "again").unwrap_err(), NOT_WAITING);
        assert!(knocks.waiting().is_empty());
        assert_eq!(knocks.views()[0]["turned_away"], true);
        assert!(knocks.remove(server).is_some());
        assert_eq!(knocks.views(), json!([]));
    }

    #[test]
    fn broken_saved_requests_read_as_none() {
        assert!(Knocks::parse(Some("not json")).waiting().is_empty());
        assert!(Knocks::parse(None).waiting().is_empty());
        assert_eq!(knocks_key("acct"), "commy.web.knocks.acct");
    }
}
