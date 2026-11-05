use serde::{Deserialize, Serialize};

pub const GIF_PROVIDERS: [&str; 2] = ["klipy", "giphy"];
pub const MAX_KEY_CHARS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GifKey {
    pub provider: String,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameDbKey {
    pub client_id: String,
    pub client_secret: String,
}

pub fn clean_key(input: &str) -> Option<String> {
    let key = input.trim();
    let valid = !key.is_empty() && key.len() <= MAX_KEY_CHARS && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.');
    valid.then(|| key.to_string())
}

impl GifKey {
    pub fn new(provider: &str, key: &str) -> Option<Self> {
        let provider = provider.trim().to_ascii_lowercase();
        GIF_PROVIDERS.contains(&provider.as_str()).then_some(())?;
        Some(Self { provider, key: clean_key(key)? })
    }

    pub fn is_valid(&self) -> bool {
        Self::new(&self.provider, &self.key).as_ref() == Some(self)
    }
}

impl GameDbKey {
    pub fn new(client_id: &str, client_secret: &str) -> Option<Self> {
        Some(Self { client_id: clean_key(client_id)?, client_secret: clean_key(client_secret)? })
    }

    pub fn is_valid(&self) -> bool {
        Self::new(&self.client_id, &self.client_secret).as_ref() == Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gif_keys_name_a_known_provider_and_a_plain_key() {
        let key = GifKey::new(" KLIPY ", " abc-123_x ").unwrap();
        assert_eq!(key, GifKey { provider: "klipy".into(), key: "abc-123_x".into() });
        assert!(key.is_valid());
        assert!(GifKey::new("giphy", "k").is_some());
        assert_eq!(GifKey::new("tenor", "k"), None);
        assert_eq!(GifKey::new("klipy", ""), None);
        assert_eq!(GifKey::new("klipy", "a/b"), None);
        assert_eq!(GifKey::new("klipy", "a b"), None);
        assert_eq!(GifKey::new("klipy", &"a".repeat(MAX_KEY_CHARS + 1)), None);
        assert!(!GifKey { provider: "klipy".into(), key: " padded".into() }.is_valid());
    }

    #[test]
    fn a_game_database_login_needs_both_parts() {
        let login = GameDbKey::new(" id1 ", "secret2").unwrap();
        assert_eq!(login.client_id, "id1");
        assert!(login.is_valid());
        assert_eq!(GameDbKey::new("id1", ""), None);
        assert_eq!(GameDbKey::new("", "secret2"), None);
        assert!(!GameDbKey { client_id: "id1".into(), client_secret: "has space".into() }.is_valid());
    }
}
