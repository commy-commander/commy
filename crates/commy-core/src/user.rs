use crate::identity::IdentityKey;
use crate::ids::UserId;
use serde::{Deserialize, Serialize};

/// Presence shown in the member list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    Online,
    Idle,
    DoNotDisturb,
    Invisible,
    #[default]
    Offline,
}

impl Presence {
    /// Whether other members should see this user as reachable.
    pub fn is_visible_online(self) -> bool {
        matches!(self, Presence::Online | Presence::Idle | Presence::DoNotDisturb)
    }

    /// The presence other members get to see. Appearing offline
    /// (`Invisible`) never leaves this device as such: everyone else sees
    /// plain `Offline`, so nothing on the wire tells it apart from a user
    /// who really is offline.
    pub fn as_seen_by_others(self) -> Presence {
        match self {
            Presence::Invisible => Presence::Offline,
            other => other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserProfile {
    pub id: UserId,
    pub username: String,
    pub display_name: Option<String>,
    /// Avatar, see [`Avatar`]: "blob:<iroh hash>" for an uploaded image
    /// served by `node`, or "emoji:<emoji>".
    pub avatar: Option<String>,
    pub presence: Presence,
    pub custom_status: Option<String>,
    /// Hex-encoded iroh node id of the device this user is on, used by
    /// peers to open direct media connections (voice, screen share).
    #[serde(default)]
    pub node: Option<String>,
    /// Emoji shown next to the custom status.
    #[serde(default)]
    pub status_emoji: Option<String>,
    /// Game or app in the foreground ("Playing X"), when sharing is on.
    #[serde(default)]
    pub activity: Option<String>,
    /// IGDB cover image id for the activity, when it is a known game.
    #[serde(default)]
    pub activity_cover: Option<String>,
    /// Short "About Me" shown on profiles.
    #[serde(default)]
    pub about: Option<String>,
    /// Public identity key that signs this user's messages. Peers pin
    /// the first one they see.
    #[serde(default)]
    pub identity: Option<IdentityKey>,
}

/// Longest About Me, in characters.
pub const MAX_ABOUT_CHARS: usize = 190;

/// Parsed form of [`UserProfile::avatar`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Avatar {
    Image { blob: String },
    Emoji(String),
}

/// Upper bound on an emoji avatar or status emoji, in bytes. Long
/// enough for flag and family sequences, short enough to stop text.
pub const MAX_EMOJI_BYTES: usize = 32;

impl Avatar {
    pub fn parse(raw: &str) -> Option<Self> {
        if let Some(blob) = raw.strip_prefix("blob:") {
            let ok = !blob.is_empty() && blob.len() <= 128 && blob.chars().all(|c| c.is_ascii_alphanumeric());
            return ok.then(|| Avatar::Image { blob: blob.to_string() });
        }
        let emoji = raw.strip_prefix("emoji:")?;
        is_emoji_like(emoji).then(|| Avatar::Emoji(emoji.to_string()))
    }

    pub fn encode(&self) -> String {
        match self {
            Avatar::Image { blob } => format!("blob:{blob}"),
            Avatar::Emoji(e) => format!("emoji:{e}"),
        }
    }
}

/// Rough emoji check: short, non-empty, and no plain letters or digits,
/// so names and sentences cannot pose as an emoji.
pub fn is_emoji_like(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_EMOJI_BYTES
        && !s.chars().any(|c| c.is_ascii_alphanumeric() || c.is_whitespace() || c.is_control())
}

impl UserProfile {
    pub fn new(username: impl Into<String>) -> Self {
        Self {
            id: UserId::new(),
            username: username.into(),
            display_name: None,
            avatar: None,
            presence: Presence::Online,
            custom_status: None,
            node: None,
            status_emoji: None,
            activity: None,
            activity_cover: None,
            about: None,
            identity: None,
        }
    }

    /// Name to render in chat: display name if set, otherwise username.
    pub fn visible_name(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.username)
    }

    /// This profile as other members see it. While appearing offline it
    /// shows as `Offline` and shares no activity, like a user who is not
    /// running the app.
    pub fn as_seen_by_others(&self) -> UserProfile {
        let mut shared = self.clone();
        if self.presence == Presence::Invisible {
            shared.presence = Presence::Offline;
            shared.activity = None;
            shared.activity_cover = None;
        }
        shared
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_name_prefers_display_name() {
        let mut user = UserProfile::new("commy");
        assert_eq!(user.visible_name(), "commy");
        user.display_name = Some("Commy C".to_string());
        assert_eq!(user.visible_name(), "Commy C");
    }

    #[test]
    fn presence_visibility() {
        assert!(Presence::Online.is_visible_online());
        assert!(Presence::DoNotDisturb.is_visible_online());
        assert!(!Presence::Invisible.is_visible_online());
        assert_eq!(Presence::Invisible.as_seen_by_others(), Presence::Offline);
        assert_eq!(Presence::DoNotDisturb.as_seen_by_others(), Presence::DoNotDisturb);
        assert!(!Presence::Offline.is_visible_online());
    }

    #[test]
    fn presence_serializes_snake_case() {
        let json = serde_json::to_string(&Presence::DoNotDisturb).unwrap();
        assert_eq!(json, "\"do_not_disturb\"");
    }

    #[test]
    fn avatar_roundtrips() {
        for avatar in [
            Avatar::Image { blob: "abc123def".to_string() },
            Avatar::Emoji("🐸".to_string()),
        ] {
            assert_eq!(Avatar::parse(&avatar.encode()), Some(avatar));
        }
    }

    #[test]
    fn avatar_rejects_bad_input() {
        assert_eq!(Avatar::parse("blob:"), None);
        assert_eq!(Avatar::parse("blob:../../etc"), None);
        assert_eq!(Avatar::parse("emoji:hello"), None);
        assert_eq!(Avatar::parse("emoji:"), None);
        assert_eq!(Avatar::parse("https://example.com/a.png"), None);
        assert_eq!(Avatar::parse(&format!("emoji:{}", "🐸".repeat(20))), None);
    }

    #[test]
    fn emoji_check_allows_sequences() {
        assert!(is_emoji_like("👩‍💻"));
        assert!(is_emoji_like("🇺🇸"));
        assert!(!is_emoji_like("a"));
        assert!(!is_emoji_like("🐸 hi"));
    }

    #[test]
    fn older_profiles_without_status_emoji_still_load() {
        let mut json: serde_json::Value = serde_json::to_value(UserProfile::new("x")).unwrap();
        json.as_object_mut().unwrap().remove("status_emoji");
        let profile: UserProfile = serde_json::from_value(json).unwrap();
        assert_eq!(profile.status_emoji, None);
    }

    #[test]
    fn identity_key_is_optional_and_stored_as_hex() {
        let mut json: serde_json::Value = serde_json::to_value(UserProfile::new("x")).unwrap();
        json.as_object_mut().unwrap().remove("identity");
        let mut profile: UserProfile = serde_json::from_value(json).unwrap();
        assert_eq!(profile.identity, None);
        let key = crate::identity::Identity::generate().public();
        profile.identity = Some(key);
        let json = serde_json::to_value(&profile).unwrap();
        assert_eq!(json["identity"], serde_json::Value::String(key.to_hex()));
        let back: UserProfile = serde_json::from_value(json).unwrap();
        assert_eq!(back.identity, Some(key));
        let bytes = postcard::to_stdvec(&profile).unwrap();
        assert_eq!(postcard::from_bytes::<UserProfile>(&bytes).unwrap(), profile);
    }
}
