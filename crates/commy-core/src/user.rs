use crate::identity::IdentityKey;
use crate::ids::UserId;
use serde::{Deserialize, Serialize};

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
    pub fn is_visible_online(self) -> bool {
        matches!(self, Presence::Online | Presence::Idle | Presence::DoNotDisturb)
    }

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
    pub avatar: Option<String>,
    pub presence: Presence,
    pub custom_status: Option<String>,
    #[serde(default)]
    pub node: Option<String>,
    #[serde(default)]
    pub status_emoji: Option<String>,
    #[serde(default)]
    pub activity: Option<String>,
    #[serde(default)]
    pub activity_cover: Option<String>,
    #[serde(default)]
    pub activity_since: Option<i64>,
    #[serde(default)]
    pub about: Option<String>,
    #[serde(default)]
    pub identity: Option<IdentityKey>,
    #[serde(default)]
    pub presence_until: Option<i64>,
    #[serde(default)]
    pub banner: Option<String>,
    #[serde(default)]
    pub server_tag: Option<crate::server_tag::WornTag>,
    #[serde(default)]
    pub avatar_border: Option<String>,
    #[serde(default)]
    pub card_effect: Option<String>,
}

pub const AVATAR_BORDERS: [&str; 8] = ["ring", "double", "sunset", "ocean", "neon", "stars", "leaves", "orbit"];
pub const CARD_EFFECTS: [&str; 4] = ["sparkles", "snow", "aurora", "bubbles"];

pub fn valid_avatar_border(name: &str) -> bool {
    AVATAR_BORDERS.contains(&name)
}

pub fn valid_card_effect(name: &str) -> bool {
    CARD_EFFECTS.contains(&name)
}

pub const MAX_ABOUT_CHARS: usize = 190;
pub const MAX_STATUS_CHARS: usize = 64;
pub const MAX_PROFILE_EMOJIS: usize = 10;

fn emoji_spans(text: &str) -> Vec<(usize, usize, usize)> {
    crate::server::image_tokens(text)
        .into_iter()
        .filter_map(|(start, end, _)| {
            let (name, _) = crate::server::parse_emoji_token(&text[start..end])?;
            Some((start, end, name.chars().count() + 2))
        })
        .collect()
}

pub fn rich_len(text: &str) -> usize {
    let spans = emoji_spans(text);
    let raw: usize = spans.iter().map(|(start, end, _)| text[*start..*end].chars().count()).sum();
    let shown: usize = spans.iter().map(|(_, _, shown)| shown).sum();
    text.chars().count() - raw + shown
}

pub fn rich_text_fits(text: &str, max_chars: usize) -> bool {
    rich_len(text) <= max_chars && emoji_spans(text).len() <= MAX_PROFILE_EMOJIS
}

pub fn truncate_rich(text: &str, max_chars: usize) -> String {
    let spans = emoji_spans(text);
    let mut out = String::new();
    let mut used = 0;
    let mut pos = 0;
    for (start, end, shown) in spans.iter().copied().chain(std::iter::once((text.len(), text.len(), 0))) {
        for c in text[pos..start].chars() {
            if used == max_chars {
                return out;
            }
            out.push(c);
            used += 1;
        }
        if start == text.len() {
            break;
        }
        if used + shown > max_chars {
            return out;
        }
        out.push_str(&text[start..end]);
        used += shown;
        pos = end;
    }
    out
}

pub fn valid_status_emoji(raw: &str) -> bool {
    is_emoji_like(raw) || crate::server::parse_emoji_token(raw).is_some()
}
pub const MAX_PRONOUNS_CHARS: usize = 24;

pub fn clean_pronouns(raw: &str) -> Option<String> {
    let text = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let fits = !text.is_empty() && text.chars().count() <= MAX_PRONOUNS_CHARS && !text.chars().any(char::is_control);
    fits.then_some(text)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Avatar {
    Image { blob: String },
    Emoji(String),
}

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

pub fn valid_banner(raw: &str) -> bool {
    matches!(Avatar::parse(raw), Some(Avatar::Image { .. }))
}

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
            activity_since: None,
            about: None,
            banner: None,
            identity: None,
            presence_until: None,
            server_tag: None,
            avatar_border: None,
            card_effect: None,
        }
    }

    pub fn visible_name(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.username)
    }

    pub fn as_seen_by_others(&self) -> UserProfile {
        let mut shared = self.clone();
        shared.server_tag = shared.server_tag.filter(crate::server_tag::WornTag::is_valid);
        shared.avatar_border = shared.avatar_border.filter(|b| valid_avatar_border(b));
        shared.card_effect = shared.card_effect.filter(|e| valid_card_effect(e));
        if self.presence == Presence::Invisible {
            shared.presence = Presence::Offline;
            shared.activity = None;
            shared.activity_cover = None;
            shared.activity_since = None;
            shared.presence_until = None;
        }
        shared
    }

    pub fn without_details(&self) -> UserProfile {
        UserProfile {
            about: None,
            banner: None,
            activity: None,
            activity_cover: None,
            activity_since: None,
            server_tag: None,
            ..self.clone()
        }
    }

    pub fn pin_identity(&mut self, pinned: Option<IdentityKey>) -> bool {
        match (pinned, self.identity) {
            (Some(pinned), Some(key)) => pinned == key,
            (Some(pinned), None) => {
                self.identity = Some(pinned);
                true
            }
            (None, _) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_built_in_borders_and_card_effects_travel() {
        let mut user = UserProfile::new("commy");
        user.avatar_border = Some("stars".into());
        user.card_effect = Some("snow".into());
        let shared = user.as_seen_by_others();
        assert_eq!((shared.avatar_border.as_deref(), shared.card_effect.as_deref()), (Some("stars"), Some("snow")));
        user.avatar_border = Some("https://example.com/frame.png".into());
        user.card_effect = Some("fireworks".into());
        let shared = user.as_seen_by_others();
        assert_eq!((shared.avatar_border, shared.card_effect), (None, None));
        assert!(AVATAR_BORDERS.iter().all(|b| valid_avatar_border(b)));
        assert!(CARD_EFFECTS.iter().all(|e| valid_card_effect(e)));
    }

    #[test]
    fn visible_name_prefers_display_name() {
        let mut user = UserProfile::new("commy");
        assert_eq!(user.visible_name(), "commy");
        user.display_name = Some("Commy C".to_string());
        assert_eq!(user.visible_name(), "Commy C");
    }

    #[test]
    fn banners_are_image_blobs_only() {
        assert!(valid_banner("blob:abc123"));
        assert!(!valid_banner("emoji:\u{1f600}"));
        assert!(!valid_banner("blob:"));
        assert!(!valid_banner("blob:../x"));
    }

    #[test]
    fn presence_others_see_never_reveals_appearing_offline() {
        assert!(Presence::Online.is_visible_online());
        assert!(Presence::DoNotDisturb.is_visible_online());
        assert!(!Presence::Invisible.is_visible_online());
        assert_eq!(Presence::Invisible.as_seen_by_others(), Presence::Offline);
        assert_eq!(Presence::DoNotDisturb.as_seen_by_others(), Presence::DoNotDisturb);
        let mut profile = UserProfile::new("busy");
        profile.presence = Presence::DoNotDisturb;
        profile.presence_until = Some(1_800_000_000);
        assert_eq!(profile.as_seen_by_others().presence_until, Some(1_800_000_000), "others see when you are back");
        profile.presence = Presence::Invisible;
        assert_eq!(profile.as_seen_by_others().presence_until, None, "but not how long you look offline");
        assert!(!Presence::Offline.is_visible_online());
    }

    #[test]
    fn a_profile_without_details_keeps_what_chat_needs() {
        let mut profile = UserProfile::new("quiet");
        profile.display_name = Some("Quiet One".into());
        profile.avatar = Some("emoji:\u{1f60e}".into());
        profile.custom_status = Some("around".into());
        profile.about = Some("long bio".into());
        profile.banner = Some("blob:abc".into());
        profile.activity = Some("Playing Chess".into());
        profile.activity_cover = Some("co1abc".into());
        profile.activity_since = Some(1_700_000_000);
        profile.identity = Some(crate::Identity::generate().public());
        let bare = profile.without_details();
        assert_eq!(bare.visible_name(), "Quiet One");
        assert_eq!(bare.avatar, profile.avatar);
        assert_eq!(bare.custom_status, profile.custom_status);
        assert_eq!(bare.identity, profile.identity);
        assert_eq!((bare.about, bare.banner, bare.activity), (None, None, None));
        assert_eq!((bare.activity_cover, bare.activity_since), (None, None));
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
    fn custom_emojis_count_as_their_short_name() {
        let token = format!("<:wave:{}@{}>", "a".repeat(64), "b".repeat(64));
        let text = format!("hi {token}!");
        assert_eq!(rich_len(&text), "hi :wave:!".chars().count());
        assert!(rich_text_fits(&text, 10));
        assert!(!rich_text_fits(&text, 9));
        assert!(!rich_text_fits(&token.repeat(MAX_PROFILE_EMOJIS + 1), 1000));
        assert_eq!(truncate_rich(&text, 8), "hi ");
        assert_eq!(truncate_rich(&text, 9), format!("hi {token}"));
        assert_eq!(truncate_rich(&text, 10), text);
        assert_eq!(truncate_rich("plain text", 5), "plain");
        assert!(valid_status_emoji(&token));
        assert!(valid_status_emoji("🐸"));
        assert!(!valid_status_emoji(&format!("{token} hi")));
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
