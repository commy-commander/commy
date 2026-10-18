use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_PAIRED_DECKS: usize = 8;
pub const MAX_DECK_NAME_CHARS: usize = 40;
pub const DEFAULT_DECK_NAME: &str = "Stream Deck";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Hello {
        version: u32,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        token: Option<String>,
    },
    List,
    Play {
        id: u64,
        #[serde(default)]
        server: Option<String>,
        sound: String,
    },
}

impl Request {
    pub fn parse(line: &str) -> Option<Self> {
        serde_json::from_str(line.trim_end_matches(['\r', '\n'])).ok()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyReason {
    Busy,
    Declined,
    Removed,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    Waiting,
    Paired { token: String },
    Ready { version: u32 },
    Denied { reason: DenyReason },
    Soundboards(SoundboardList),
    Played { id: u64 },
    Failed { id: u64, message: String },
}

impl Reply {
    pub fn to_line(&self) -> String {
        let mut line = serde_json::to_string(self).unwrap_or_default();
        line.push('\n');
        line
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SoundboardList {
    pub voice_server: Option<String>,
    pub defaults: Vec<SoundChoice>,
    pub servers: Vec<SoundboardServer>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SoundboardServer {
    pub id: String,
    pub name: String,
    pub can_play: bool,
    pub sounds: Vec<SoundChoice>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SoundChoice {
    pub name: String,
    pub label: String,
    pub emoji: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub decks: Vec<PairedDeck>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairedDeck {
    pub id: String,
    pub name: String,
    pub token: String,
    pub paired_at: DateTime<Utc>,
}

impl Settings {
    pub fn deck_for(&self, token: &str) -> Option<&PairedDeck> {
        self.decks.iter().find(|deck| same_secret(&deck.token, token))
    }

    pub fn pair(&mut self, name: &str, now: DateTime<Utc>) -> PairedDeck {
        let deck = PairedDeck { id: random_hex(8), name: deck_name(name), token: random_token(), paired_at: now };
        self.decks.push(deck.clone());
        let overflow = self.decks.len().saturating_sub(MAX_PAIRED_DECKS);
        self.decks.drain(..overflow);
        deck
    }

    pub fn forget(&mut self, id: &str) -> bool {
        let before = self.decks.len();
        self.decks.retain(|deck| deck.id != id);
        self.decks.len() != before
    }
}

pub fn deck_name(raw: &str) -> String {
    let name: String = raw.chars().filter(|c| !c.is_control()).take(MAX_DECK_NAME_CHARS).collect();
    let name = name.trim();
    if name.is_empty() {
        DEFAULT_DECK_NAME.to_string()
    } else {
        name.to_string()
    }
}

pub fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn random_token() -> String {
    data_encoding::BASE64URL_NOPAD.encode(&commy_net::CommyNode::generate_secret())
}

fn random_hex(bytes: usize) -> String {
    data_encoding::HEXLOWER.encode(&commy_net::CommyNode::generate_secret()[..bytes])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_list_and_play_parse() {
        assert_eq!(
            Request::parse(r#"{"type":"hello","version":1,"name":"Deck","token":"abc"}"#),
            Some(Request::Hello { version: 1, name: Some("Deck".into()), token: Some("abc".into()) })
        );
        assert_eq!(
            Request::parse("{\"type\":\"hello\",\"version\":1}\r\n"),
            Some(Request::Hello { version: 1, name: None, token: None })
        );
        assert_eq!(Request::parse(r#"{"type":"list"}"#), Some(Request::List));
        assert_eq!(
            Request::parse(r#"{"type":"play","id":7,"sound":"default:airhorn"}"#),
            Some(Request::Play { id: 7, server: None, sound: "default:airhorn".into() })
        );
    }

    #[test]
    fn browser_requests_and_junk_never_parse() {
        assert_eq!(Request::parse("GET / HTTP/1.1"), None);
        assert_eq!(Request::parse("POST /play HTTP/1.1"), None);
        assert_eq!(Request::parse(r#"{"type":"shutdown"}"#), None);
        assert_eq!(Request::parse(r#"{"type":"play","sound":"x"}"#), None);
        assert_eq!(Request::parse(""), None);
    }

    #[test]
    fn replies_are_one_json_line_each() {
        assert_eq!(Reply::Ready { version: 1 }.to_line(), "{\"type\":\"ready\",\"version\":1}\n");
        assert_eq!(Reply::Denied { reason: DenyReason::Declined }.to_line(), "{\"type\":\"denied\",\"reason\":\"declined\"}\n");
        let list = Reply::Soundboards(SoundboardList::default()).to_line();
        assert!(list.starts_with("{\"type\":\"soundboards\""), "{list}");
        assert_eq!(list.matches('\n').count(), 1);
    }

    #[test]
    fn pairing_issues_long_unique_tokens() {
        let mut settings = Settings::default();
        let a = settings.pair("Deck", Utc::now());
        let b = settings.pair("Deck", Utc::now());
        assert_eq!(a.token.len(), 43);
        assert_ne!(a.token, b.token);
        assert_ne!(a.id, b.id);
        assert_eq!(settings.deck_for(&a.token).map(|d| d.id.as_str()), Some(a.id.as_str()));
        assert!(settings.deck_for("").is_none());
        assert!(settings.deck_for(&a.token[..42]).is_none());
    }

    #[test]
    fn forgetting_a_deck_revokes_its_token() {
        let mut settings = Settings::default();
        let deck = settings.pair("Deck", Utc::now());
        assert!(settings.forget(&deck.id));
        assert!(!settings.forget(&deck.id));
        assert!(settings.deck_for(&deck.token).is_none());
    }

    #[test]
    fn oldest_decks_drop_off_past_the_limit() {
        let mut settings = Settings::default();
        let first = settings.pair("first", Utc::now());
        for _ in 0..MAX_PAIRED_DECKS {
            settings.pair("next", Utc::now());
        }
        assert_eq!(settings.decks.len(), MAX_PAIRED_DECKS);
        assert!(settings.deck_for(&first.token).is_none());
    }

    #[test]
    fn deck_names_are_trimmed_and_bounded() {
        assert_eq!(deck_name("  Desk Deck \u{7}"), "Desk Deck");
        assert_eq!(deck_name(""), DEFAULT_DECK_NAME);
        assert_eq!(deck_name("\n\t"), DEFAULT_DECK_NAME);
        assert_eq!(deck_name(&"x".repeat(100)).chars().count(), MAX_DECK_NAME_CHARS);
    }

    #[test]
    fn secrets_compare_by_value() {
        assert!(same_secret("abc", "abc"));
        assert!(!same_secret("abc", "abd"));
        assert!(!same_secret("abc", "abcd"));
    }

    #[test]
    fn settings_default_to_off_with_no_decks() {
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert!(!settings.enabled);
        assert!(settings.decks.is_empty());
    }
}
