use commy_core::user::{clean_pronouns, is_emoji_like, rich_text_fits, valid_status_emoji, MAX_ABOUT_CHARS, MAX_PRONOUNS_CHARS, MAX_PROFILE_EMOJIS, MAX_STATUS_CHARS};
use commy_core::{Avatar, Presence, UserProfile};

use crate::act::Act;
use crate::pictures::is_jpeg_or_png;

const MAX_DISPLAY_NAME_BYTES: usize = 64;

pub struct ProfileEdit {
    pub display_name: Option<String>,
    pub custom_status: Option<String>,
    pub presence: Presence,
    pub status_emoji: Option<Option<String>>,
}

pub fn presence(raw: &str) -> Act<Presence> {
    serde_json::from_value(serde_json::json!(raw)).map_err(|_| "invalid presence".to_string())
}

pub fn status_emoji(raw: Option<String>) -> Act<Option<Option<String>>> {
    match raw.map(|e| e.trim().to_string()) {
        None => Ok(None),
        Some(e) if e.is_empty() => Ok(Some(None)),
        Some(e) if valid_status_emoji(&e) => Ok(Some(Some(e))),
        Some(_) => Err("status emoji must be a single emoji".into()),
    }
}

pub fn status_text(raw: Option<String>) -> Option<String> {
    raw.map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && rich_text_fits(s, MAX_STATUS_CHARS))
}

pub fn display_name(raw: Option<String>) -> Option<String> {
    status_text(raw).filter(|name| name.len() <= MAX_DISPLAY_NAME_BYTES)
}

pub fn about(raw: &str) -> Act<String> {
    let about = raw.trim().to_string();
    if !rich_text_fits(&about, MAX_ABOUT_CHARS) {
        return Err(format!("About Me can be up to {MAX_ABOUT_CHARS} characters and {MAX_PROFILE_EMOJIS} custom emojis"));
    }
    Ok(about)
}

pub fn apply_edit(profile: &mut UserProfile, edit: ProfileEdit) {
    profile.display_name = edit.display_name;
    profile.custom_status = edit.custom_status;
    if edit.presence != profile.presence {
        profile.presence_until = None;
    }
    profile.presence = edit.presence;
    if let Some(emoji) = edit.status_emoji {
        profile.status_emoji = emoji;
    }
}

pub fn pronouns(raw: Option<&str>) -> Act<Option<String>> {
    match raw.map(str::trim).filter(|p| !p.is_empty()) {
        None => Ok(None),
        Some(text) => clean_pronouns(text).map(Some).ok_or_else(|| format!("pronouns can be up to {MAX_PRONOUNS_CHARS} characters")),
    }
}

pub fn future(at: Option<i64>, now: i64) -> Option<i64> {
    at.filter(|t| *t > now)
}

pub fn expired(presence_until: Option<i64>, status_until: Option<i64>, now: i64) -> (bool, bool) {
    (presence_until.is_some_and(|t| t <= now), status_until.is_some_and(|t| t <= now))
}

pub fn expire(profile: &mut UserProfile, presence: bool, status: bool) {
    if presence {
        profile.presence = Presence::Online;
        profile.presence_until = None;
    }
    if status {
        profile.custom_status = None;
        profile.status_emoji = None;
    }
}

pub fn emoji_avatar(raw: Option<String>) -> Act<Avatar> {
    let emoji = raw.unwrap_or_default().trim().to_string();
    if !is_emoji_like(&emoji) {
        return Err("pick a single emoji".into());
    }
    Ok(Avatar::Emoji(emoji))
}

pub fn image_bytes(data: &str, max: usize, too_large: &str, not_image: &str) -> Act<Vec<u8>> {
    let bytes = data_encoding::BASE64.decode(data.as_bytes()).map_err(|_| "image data is not valid base64".to_string())?;
    if bytes.len() > max {
        return Err(too_large.into());
    }
    if !is_jpeg_or_png(&bytes) {
        return Err(not_image.into());
    }
    Ok(bytes)
}

pub fn shared_fields_match(a: &UserProfile, b: &UserProfile) -> bool {
    a.id == b.id && a.username == b.username && a.display_name == b.display_name && a.avatar == b.avatar
}

pub fn take_shared_fields(profile: &mut UserProfile, shared: &UserProfile) -> bool {
    if profile.id != shared.id || shared_fields_match(profile, shared) {
        return false;
    }
    profile.username = shared.username.clone();
    profile.display_name = shared.display_name.clone();
    profile.avatar = shared.avatar.clone();
    true
}

pub fn for_list(profile: &UserProfile) -> UserProfile {
    let mut kept = profile.clone();
    kept.node = None;
    kept.activity = None;
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0];

    #[test]
    fn presence_names_match_the_ui() {
        assert_eq!(presence("do_not_disturb"), Ok(Presence::DoNotDisturb));
        assert_eq!(presence("invisible"), Ok(Presence::Invisible));
        assert_eq!(presence("away"), Err("invalid presence".into()));
    }

    #[test]
    fn a_status_emoji_can_be_left_cleared_or_set() {
        assert_eq!(status_emoji(None), Ok(None));
        assert_eq!(status_emoji(Some("  ".into())), Ok(Some(None)));
        assert_eq!(status_emoji(Some("🎉".into())), Ok(Some(Some("🎉".into()))));
        assert!(status_emoji(Some("party".into())).is_err());
    }

    #[test]
    fn long_or_empty_names_and_statuses_are_dropped() {
        assert_eq!(display_name(Some("  Sam  ".into())), Some("Sam".into()));
        assert_eq!(display_name(Some("".into())), None);
        assert_eq!(display_name(Some("é".repeat(40))), None);
        assert_eq!(status_text(Some("x".repeat(MAX_STATUS_CHARS + 1))), None);
        assert_eq!(status_text(Some("out for lunch".into())), Some("out for lunch".into()));
    }

    #[test]
    fn about_me_has_a_limit() {
        assert_eq!(about("  hi  "), Ok("hi".into()));
        assert!(about(&"x".repeat(MAX_ABOUT_CHARS + 1)).is_err());
    }

    #[test]
    fn an_edit_keeps_the_status_emoji_unless_told() {
        let mut profile = UserProfile::new("sam");
        profile.status_emoji = Some("🌙".into());
        profile.presence_until = Some(5);
        let edit = ProfileEdit { display_name: Some("Sam".into()), custom_status: None, presence: Presence::Idle, status_emoji: None };
        apply_edit(&mut profile, edit);
        assert_eq!(profile.display_name.as_deref(), Some("Sam"));
        assert_eq!(profile.status_emoji.as_deref(), Some("🌙"));
        assert_eq!(profile.presence, Presence::Idle);
        assert_eq!(profile.presence_until, None);
        let edit = ProfileEdit { display_name: None, custom_status: None, presence: Presence::Idle, status_emoji: Some(None) };
        apply_edit(&mut profile, edit);
        assert_eq!(profile.status_emoji, None);
    }

    #[test]
    fn pronouns_are_short_and_can_be_cleared() {
        assert_eq!(pronouns(Some("  they / them ")), Ok(Some("they / them".into())));
        assert_eq!(pronouns(Some("  ")), Ok(None));
        assert_eq!(pronouns(None), Ok(None));
        assert!(pronouns(Some(&"x".repeat(MAX_PRONOUNS_CHARS + 1))).is_err());
    }

    #[test]
    fn a_timed_presence_or_status_runs_out() {
        assert_eq!(future(Some(5), 10), None);
        assert_eq!(future(Some(15), 10), Some(15));
        assert_eq!(expired(Some(150), Some(200), 100), (false, false));
        assert_eq!(expired(Some(150), Some(200), 150), (true, false));
        assert_eq!(expired(Some(150), None, 200), (true, false));
        let mut profile = UserProfile::new("sam");
        profile.presence = Presence::DoNotDisturb;
        profile.presence_until = Some(150);
        profile.custom_status = Some("busy".into());
        profile.status_emoji = Some("🔕".into());
        expire(&mut profile, true, false);
        assert_eq!((profile.presence, profile.presence_until), (Presence::Online, None));
        assert_eq!(profile.custom_status.as_deref(), Some("busy"));
        expire(&mut profile, false, true);
        assert_eq!((profile.custom_status, profile.status_emoji), (None, None));
    }

    #[test]
    fn avatars_are_one_emoji_or_a_small_picture() {
        assert_eq!(emoji_avatar(Some("🐙".into())), Ok(Avatar::Emoji("🐙".into())));
        assert!(emoji_avatar(Some("ab".into())).is_err());
        let data = data_encoding::BASE64.encode(PNG);
        assert_eq!(image_bytes(&data, 100, "big", "bad"), Ok(PNG.to_vec()));
        assert_eq!(image_bytes(&data, 4, "big", "bad"), Err("big".into()));
        assert_eq!(image_bytes(&data_encoding::BASE64.encode(b"GIF89a"), 100, "big", "bad"), Err("bad".into()));
        assert_eq!(image_bytes("not base64!", 100, "big", "bad"), Err("image data is not valid base64".into()));
    }

    #[test]
    fn another_device_shares_only_the_name_and_avatar() {
        let mut mine = UserProfile::new("sam");
        mine.custom_status = Some("here".into());
        let mut theirs = mine.clone();
        theirs.display_name = Some("Sammy".into());
        theirs.custom_status = None;
        assert!(take_shared_fields(&mut mine, &theirs));
        assert_eq!(mine.display_name.as_deref(), Some("Sammy"));
        assert_eq!(mine.custom_status.as_deref(), Some("here"));
        assert!(!take_shared_fields(&mut mine, &theirs));
        assert!(!take_shared_fields(&mut mine, &UserProfile::new("other")));
    }
}
