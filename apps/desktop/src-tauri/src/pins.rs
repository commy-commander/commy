use std::path::Path;

use crate::state::App;

pub use commy_view::pins::Pins;

const FILE: &str = "pins.json";

fn load(dir: &Path) -> Pins {
    crate::persist::read_json(&dir.join(FILE)).unwrap_or_default()
}

fn store(pins: &Pins, dir: &Path) -> Result<(), String> {
    crate::persist::write_json(&dir.join(FILE), pins).map_err(|e| format!("Could not save your pins: {e}"))
}

fn dir_of(app: &App) -> Result<std::path::PathBuf, String> {
    app.data_dir().cloned().ok_or_else(|| "No data folder".to_string())
}

fn change(app: &App, edit: impl FnOnce(&mut Pins) -> bool) -> Result<Pins, String> {
    let dir = dir_of(app)?;
    let mut pins = load(&dir);
    if edit(&mut pins) {
        store(&pins, &dir)?;
    }
    Ok(pins)
}

#[tauri::command]
pub fn pins_load(app: tauri::State<'_, App>) -> Pins {
    dir_of(&app).map(|dir| load(&dir)).unwrap_or_default()
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
    fn pins_round_trip_through_the_file_and_tolerate_an_older_build() {
        let dir = std::env::temp_dir().join(format!("commy-pins-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut pins = Pins::default();
        pins.set_dm("friend", true);
        pins.set_channel("s", "c", true);
        store(&pins, &dir).unwrap();
        assert_eq!(load(&dir), pins);
        assert_eq!(load(&dir.join("missing")), Pins::default());
        let older: Pins = serde_json::from_str(r#"{"dms":["x"]}"#).unwrap();
        assert_eq!(older.dms, vec!["x".to_string()]);
        assert!(older.channels.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
