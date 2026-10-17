use std::collections::HashMap;

use chrono::{DateTime, TimeDelta, Utc};
use commy_core::ServerId;
use serde::{Deserialize, Serialize};

pub const KEEP_DAY_CHOICES: [u32; 4] = [30, 90, 180, 365];
pub const DEFAULT_KEEP: KeepWindow = KeepWindow::Days(90);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum KeepWindow {
    Days(u32),
    Forever,
}

impl KeepWindow {
    pub fn parse(raw: &str) -> Option<Self> {
        if raw == "forever" {
            return Some(Self::Forever);
        }
        raw.parse().ok().filter(|days| KEEP_DAY_CHOICES.contains(days)).map(Self::Days)
    }

    pub fn floor(self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Self::Days(days) => now.checked_sub_signed(TimeDelta::try_days(days.into())?),
            Self::Forever => None,
        }
    }
}

impl From<KeepWindow> for String {
    fn from(window: KeepWindow) -> Self {
        match window {
            KeepWindow::Days(days) => days.to_string(),
            KeepWindow::Forever => "forever".into(),
        }
    }
}

impl TryFrom<String> for KeepWindow {
    type Error = String;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::parse(&raw).ok_or_else(|| format!("unknown history window {raw}"))
    }
}

fn default_keep() -> KeepWindow {
    DEFAULT_KEEP
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StorageSettings {
    #[serde(default = "default_keep")]
    pub keep: KeepWindow,
    #[serde(default)]
    pub servers: HashMap<ServerId, KeepWindow>,
}

impl Default for StorageSettings {
    fn default() -> Self {
        Self { keep: DEFAULT_KEEP, servers: HashMap::new() }
    }
}

impl StorageSettings {
    pub fn window(&self, server: ServerId) -> KeepWindow {
        self.servers.get(&server).copied().unwrap_or(self.keep)
    }

    pub fn floor(&self, server: ServerId, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.window(server).floor(now)
    }

    pub fn set(&mut self, server: Option<ServerId>, window: Option<KeepWindow>) -> Result<(), &'static str> {
        match (server, window) {
            (None, Some(window)) => self.keep = window,
            (None, None) => return Err("pick how long to keep messages"),
            (Some(server), Some(window)) => {
                self.servers.insert(server, window);
            }
            (Some(server), None) => {
                self.servers.remove(&server);
            }
        }
        Ok(())
    }

    pub fn forget(&mut self, server: ServerId) -> bool {
        self.servers.remove(&server).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ninety_days_is_the_default_everywhere() {
        let settings = StorageSettings::default();
        assert_eq!(settings.window(ServerId::new()), KeepWindow::Days(90));
        let now = Utc::now();
        assert_eq!(settings.floor(ServerId::new(), now), Some(now - TimeDelta::days(90)));
    }

    #[test]
    fn only_the_offered_windows_parse() {
        assert_eq!(KeepWindow::parse("30"), Some(KeepWindow::Days(30)));
        assert_eq!(KeepWindow::parse("365"), Some(KeepWindow::Days(365)));
        assert_eq!(KeepWindow::parse("forever"), Some(KeepWindow::Forever));
        for raw in ["0", "7", "29", "-1", "", "90 ", "never", "4294967295"] {
            assert_eq!(KeepWindow::parse(raw), None, "{raw:?}");
        }
    }

    #[test]
    fn a_server_override_wins_until_it_is_cleared() {
        let mut settings = StorageSettings::default();
        let busy = ServerId::new();
        let friends = ServerId::new();
        settings.set(Some(busy), Some(KeepWindow::Days(30))).unwrap();
        settings.set(Some(friends), Some(KeepWindow::Forever)).unwrap();
        settings.set(None, Some(KeepWindow::Days(180))).unwrap();
        assert_eq!(settings.window(busy), KeepWindow::Days(30));
        assert_eq!(settings.window(friends), KeepWindow::Forever);
        assert_eq!(settings.floor(friends, Utc::now()), None);
        assert_eq!(settings.window(ServerId::new()), KeepWindow::Days(180));
        settings.set(Some(busy), None).unwrap();
        assert_eq!(settings.window(busy), KeepWindow::Days(180));
        assert!(settings.forget(friends));
        assert!(!settings.forget(friends));
        assert!(settings.set(None, None).is_err());
    }

    #[test]
    fn settings_round_trip_and_bad_files_fall_back() {
        let mut settings = StorageSettings::default();
        let server = ServerId::new();
        settings.set(Some(server), Some(KeepWindow::Forever)).unwrap();
        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains("\"forever\""));
        assert_eq!(serde_json::from_str::<StorageSettings>(&json).unwrap(), settings);
        assert_eq!(serde_json::from_str::<StorageSettings>("{}").unwrap(), StorageSettings::default());
        assert!(serde_json::from_str::<StorageSettings>(r#"{"keep":"7"}"#).is_err());
    }
}
