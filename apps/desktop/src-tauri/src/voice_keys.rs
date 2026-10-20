use serde::{Deserialize, Serialize};

pub const DEFAULT_TALK_KEY: &str = "Ctrl+Shift+Space";
pub const DEFAULT_TALK_RELEASE_MS: u32 = 200;
pub const MAX_TALK_RELEASE_MS: u32 = 2000;
pub const DEFAULT_SENSITIVITY_DB: f32 = -45.0;
pub const MIN_SENSITIVITY_DB: f32 = -100.0;
pub const MAX_SENSITIVITY_DB: f32 = 0.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    #[default]
    VoiceActivity,
    PushToTalk,
}

impl InputMode {
    pub fn parse(mode: &str) -> Option<Self> {
        match mode {
            "voice_activity" => Some(Self::VoiceActivity),
            "push_to_talk" => Some(Self::PushToTalk),
            _ => None,
        }
    }
}

pub fn default_talk_key() -> String {
    DEFAULT_TALK_KEY.to_string()
}

pub fn default_talk_release_ms() -> u32 {
    DEFAULT_TALK_RELEASE_MS
}

pub fn default_sensitivity_db() -> f32 {
    DEFAULT_SENSITIVITY_DB
}

pub fn clamp_release_ms(ms: u32) -> u32 {
    ms.min(MAX_TALK_RELEASE_MS)
}

pub fn clamp_sensitivity_db(db: f32) -> f32 {
    if db.is_finite() {
        db.clamp(MIN_SENSITIVITY_DB, MAX_SENSITIVITY_DB)
    } else {
        DEFAULT_SENSITIVITY_DB
    }
}

pub fn sensitivity(auto: bool, threshold_db: f32) -> commy_audio::Sensitivity {
    if auto {
        commy_audio::Sensitivity::Automatic
    } else {
        commy_audio::Sensitivity::Threshold(clamp_sensitivity_db(threshold_db))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceKeys {
    pub talk: Option<String>,
    pub mute: Option<String>,
    pub deafen: Option<String>,
}

fn present(key: &str) -> Option<String> {
    let key = key.trim();
    (!key.is_empty()).then(|| key.to_string())
}

impl VoiceKeys {
    pub fn from_settings(
        mode: InputMode,
        talk_key: &str,
        mute_hotkey: &str,
        deafen_hotkey: &str,
    ) -> Self {
        Self {
            talk: (mode == InputMode::PushToTalk)
                .then(|| present(talk_key))
                .flatten(),
            mute: present(mute_hotkey),
            deafen: present(deafen_hotkey),
        }
    }

    pub fn registered(&self) -> Vec<&str> {
        [&self.talk, &self.mute, &self.deafen]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect()
    }
}

pub fn validate_optional_hotkey(key: &str, what: &str) -> Result<String, String> {
    let key = key.trim().to_string();
    if key.is_empty() || crate::overlay::valid_hotkey(&key) {
        Ok(key)
    } else {
        Err(format!("{key} is not a hotkey Commy can use for {what}"))
    }
}

pub fn clashing<S: AsRef<str>>(keys: &[S]) -> Option<String> {
    let mut seen: Vec<String> = Vec::new();
    for key in keys.iter().map(AsRef::as_ref).filter(|k| !k.is_empty()) {
        let lower = key.to_ascii_lowercase();
        if seen.contains(&lower) {
            return Some(key.to_string());
        }
        seen.push(lower);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_round_trip_through_their_wire_names() {
        assert_eq!(
            InputMode::parse("push_to_talk"),
            Some(InputMode::PushToTalk)
        );
        assert_eq!(
            InputMode::parse("voice_activity"),
            Some(InputMode::VoiceActivity)
        );
        assert_eq!(InputMode::parse("yell"), None);
        assert_eq!(
            serde_json::to_string(&InputMode::PushToTalk).unwrap(),
            "\"push_to_talk\""
        );
        assert_eq!(InputMode::default(), InputMode::VoiceActivity);
    }

    #[test]
    fn the_talk_key_registers_only_in_push_to_talk() {
        let keys = VoiceKeys::from_settings(InputMode::VoiceActivity, "Ctrl+Space", "", "Ctrl+D");
        assert_eq!(keys.registered(), vec!["Ctrl+D"]);
        let keys = VoiceKeys::from_settings(InputMode::PushToTalk, "Ctrl+Space", " Ctrl+M ", "");
        assert_eq!(keys.registered(), vec!["Ctrl+Space", "Ctrl+M"]);
    }

    #[test]
    fn hotkeys_may_be_empty_but_not_nonsense() {
        assert_eq!(validate_optional_hotkey("  ", "mute"), Ok(String::new()));
        assert_eq!(
            validate_optional_hotkey("Ctrl+Shift+M", "mute"),
            Ok("Ctrl+Shift+M".into())
        );
        assert!(validate_optional_hotkey("Banana+Q", "mute")
            .unwrap_err()
            .contains("mute"));
    }

    #[test]
    fn clashes_ignore_case_and_blanks() {
        assert_eq!(
            clashing(&["Ctrl+M", "", "", "ctrl+m"]),
            Some("ctrl+m".into())
        );
        assert_eq!(clashing(&["Ctrl+M", "Ctrl+D", ""]), None);
    }

    #[test]
    fn numbers_stay_in_range() {
        assert_eq!(clamp_release_ms(5000), MAX_TALK_RELEASE_MS);
        assert_eq!(clamp_release_ms(150), 150);
        assert_eq!(clamp_sensitivity_db(-200.0), MIN_SENSITIVITY_DB);
        assert_eq!(clamp_sensitivity_db(5.0), MAX_SENSITIVITY_DB);
        assert_eq!(clamp_sensitivity_db(f32::NAN), DEFAULT_SENSITIVITY_DB);
        assert_eq!(
            sensitivity(true, -10.0),
            commy_audio::Sensitivity::Automatic
        );
        assert_eq!(
            sensitivity(false, -10.0),
            commy_audio::Sensitivity::Threshold(-10.0)
        );
    }
}
