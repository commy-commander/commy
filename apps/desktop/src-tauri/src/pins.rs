use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::state::App;

const FILE: &str = "pins.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedChannel {
    pub server: String,
    pub channel: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pins {
    #[serde(default)]
    pub dms: Vec<String>,
    #[serde(default)]
    pub channels: Vec<PinnedChannel>,
}

impl Pins {
    pub fn load(dir: &Path) -> Pins {
        crate::persist::read_json(&dir.join(FILE)).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        crate::persist::write_json(&dir.join(FILE), self).map_err(|e| format!("Could not save your pins: {e}"))
    }

    pub fn set_dm(&mut self, user: &str, pinned: bool) -> bool {
        let had = self.dms.iter().position(|u| u == user);
        match (had, pinned) {
            (None, true) => self.dms.push(user.to_string()),
            (Some(index), false) => {
                self.dms.remove(index);
            }
            _ => return false,
        }
        true
    }

    pub fn set_channel(&mut self, server: &str, channel: &str, pinned: bool) -> bool {
        let had = self.channels.iter().position(|c| c.server == server && c.channel == channel);
        match (had, pinned) {
            (None, true) => self.channels.push(PinnedChannel { server: server.to_string(), channel: channel.to_string() }),
            (Some(index), false) => {
                self.channels.remove(index);
            }
            _ => return false,
        }
        true
    }

}

fn dir_of(app: &App) -> Result<std::path::PathBuf, String> {
    app.data_dir().cloned().ok_or_else(|| "No data folder".to_string())
}

fn change(app: &App, edit: impl FnOnce(&mut Pins) -> bool) -> Result<Pins, String> {
    let dir = dir_of(app)?;
    let mut pins = Pins::load(&dir);
    if edit(&mut pins) {
        pins.save(&dir)?;
    }
    Ok(pins)
}

#[tauri::command]
pub fn pins_load(app: tauri::State<'_, App>) -> Pins {
    dir_of(&app).map(|dir| Pins::load(&dir)).unwrap_or_default()
}

#[tauri::command]
pub fn pin_dm(app: tauri::State<'_, App>, user_id: String, pinned: bool) -> Result<Pins, String> {
    change(&app, |pins| pins.set_dm(&user_id, pinned))
}

#[tauri::command]
pub fn pin_channel(app: tauri::State<'_, App>, server_id: String, channel_id: String, pinned: bool) -> Result<Pins, String> {
    change(&app, |pins| pins.set_channel(&server_id, &channel_id, pinned))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_toggle_once_and_keep_their_order() {
        let mut pins = Pins::default();
        assert!(pins.set_dm("b", true));
        assert!(pins.set_dm("a", true));
        assert!(!pins.set_dm("a", true), "pinning twice changes nothing");
        assert_eq!(pins.dms, vec!["b".to_string(), "a".to_string()]);
        assert!(pins.set_dm("b", false));
        assert!(!pins.set_dm("b", false));
        assert_eq!(pins.dms, vec!["a".to_string()]);

        assert!(pins.set_channel("s1", "c1", true));
        assert!(pins.set_channel("s2", "c1", true));
        assert!(!pins.set_channel("s1", "c1", true));
        assert!(pins.set_channel("s1", "c1", false));
        assert_eq!(pins.channels, vec![PinnedChannel { server: "s2".into(), channel: "c1".into() }]);
    }

    #[test]
    fn pins_round_trip_through_the_file_and_tolerate_an_older_build() {
        let dir = std::env::temp_dir().join(format!("commy-pins-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut pins = Pins::default();
        pins.set_dm("friend", true);
        pins.set_channel("s", "c", true);
        pins.save(&dir).unwrap();
        assert_eq!(Pins::load(&dir), pins);
        assert_eq!(Pins::load(&dir.join("missing")), Pins::default());
        let older: Pins = serde_json::from_str(r#"{"dms":["x"]}"#).unwrap();
        assert_eq!(older.dms, vec!["x".to_string()]);
        assert!(older.channels.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
