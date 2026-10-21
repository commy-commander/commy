use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::state::App;

const FILE: &str = "saved.json";
pub const MAX_SAVED: usize = 500;
pub const MAX_REMINDERS: usize = 200;
const MAX_TEXT: usize = 400;

static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub message: String,
    #[serde(default)]
    pub server: Option<String>,
    #[serde(default)]
    pub server_name: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub channel_name: Option<String>,
    #[serde(default)]
    pub dm: Option<String>,
    #[serde(default)]
    pub dm_name: Option<String>,
    #[serde(default)]
    pub author_id: String,
    #[serde(default)]
    pub author_name: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub attachment: Option<String>,
    #[serde(default)]
    pub created_at: String,
}

impl Entry {
    fn trimmed(mut self) -> Self {
        self.content = crate::safety::truncate(&self.content, MAX_TEXT);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedMessage {
    pub saved_at: i64,
    #[serde(flatten)]
    pub entry: Entry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reminder {
    pub id: String,
    pub due: i64,
    pub entry: Entry,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedStore {
    #[serde(default)]
    pub saved: Vec<SavedMessage>,
    #[serde(default)]
    pub reminders: Vec<Reminder>,
}

impl SavedStore {
    pub fn load(dir: &Path) -> Self {
        crate::persist::read_json(&dir.join(FILE)).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        crate::persist::write_json(&dir.join(FILE), self).map_err(|e| format!("could not save messages: {e}"))
    }

    pub fn set_saved(&mut self, entry: Entry, saved: bool, now_ms: i64) -> bool {
        let before = self.saved.len();
        self.saved.retain(|s| s.entry.message != entry.message);
        if !saved {
            return self.saved.len() != before;
        }
        self.saved.push(SavedMessage { saved_at: now_ms, entry: entry.trimmed() });
        self.saved.sort_by(|a, b| b.saved_at.cmp(&a.saved_at).then_with(|| b.entry.message.cmp(&a.entry.message)));
        self.saved.truncate(MAX_SAVED);
        true
    }

    pub fn set_reminder(&mut self, entry: Entry, due: i64, now_ms: i64) -> Result<String, String> {
        if due <= now_ms {
            return Err("pick a time that is still ahead".to_string());
        }
        self.reminders.retain(|r| r.entry.message != entry.message);
        if self.reminders.len() >= MAX_REMINDERS {
            return Err(format!("you can keep up to {MAX_REMINDERS} reminders"));
        }
        let id = format!("{}-{}", due, entry.message);
        self.reminders.push(Reminder { id: id.clone(), due, entry: entry.trimmed() });
        self.reminders.sort_by(|a, b| a.due.cmp(&b.due).then_with(|| a.id.cmp(&b.id)));
        Ok(id)
    }

    pub fn cancel_reminder(&mut self, id: &str) -> bool {
        let before = self.reminders.len();
        self.reminders.retain(|r| r.id != id);
        self.reminders.len() != before
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn dir_of(app: &App) -> Result<std::path::PathBuf, String> {
    app.data_dir().cloned().ok_or_else(|| "storage is not ready yet".to_string())
}

fn update(app: &App, change: impl FnOnce(&mut SavedStore) -> Result<(), String>) -> Result<SavedStore, String> {
    let dir = dir_of(app)?;
    let _guard = LOCK.lock().unwrap();
    let mut store = SavedStore::load(&dir);
    change(&mut store)?;
    store.save(&dir)?;
    Ok(store)
}

#[tauri::command]
pub fn saved_list(app: tauri::State<'_, App>) -> Result<SavedStore, String> {
    let dir = dir_of(&app)?;
    let _guard = LOCK.lock().unwrap();
    Ok(SavedStore::load(&dir))
}

#[tauri::command]
pub fn saved_set(app: tauri::State<'_, App>, entry: Entry, saved: bool) -> Result<SavedStore, String> {
    update(&app, |store| {
        store.set_saved(entry, saved, now_ms());
        Ok(())
    })
}

#[tauri::command]
pub fn reminder_set(app: tauri::State<'_, App>, entry: Entry, due: i64) -> Result<SavedStore, String> {
    update(&app, |store| store.set_reminder(entry, due, now_ms()).map(|_| ()))
}

#[tauri::command]
pub fn reminder_cancel(app: tauri::State<'_, App>, id: String) -> Result<SavedStore, String> {
    update(&app, |store| {
        store.cancel_reminder(&id);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(message: &str) -> Entry {
        Entry {
            message: message.into(),
            server: Some("s".into()),
            server_name: Some("Rust Hangout".into()),
            channel: Some("c".into()),
            channel_name: Some("general".into()),
            author_id: "m1".into(),
            author_name: "ferris".into(),
            content: "hello".into(),
            created_at: "2026-10-20T10:00:00Z".into(),
            ..Entry::default()
        }
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("commy-saved-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn saving_is_a_toggle_that_lists_newest_first_and_round_trips() {
        let dir = temp_dir("toggle");
        let mut store = SavedStore::default();
        assert!(store.set_saved(entry("a"), true, 1));
        assert!(store.set_saved(entry("b"), true, 2));
        assert!(store.set_saved(entry("a"), true, 3), "saving again moves it to the top");
        assert_eq!(store.saved.iter().map(|s| s.entry.message.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert!(store.set_saved(entry("b"), false, 4));
        assert!(!store.set_saved(entry("b"), false, 5), "removing twice changes nothing");
        store.save(&dir).unwrap();
        assert_eq!(SavedStore::load(&dir), store);
        assert_eq!(SavedStore::load(&dir.join("missing")), SavedStore::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_saved_list_is_capped_and_long_text_is_trimmed() {
        let mut store = SavedStore::default();
        for i in 0..(MAX_SAVED + 5) {
            store.set_saved(entry(&format!("m{i}")), true, i as i64);
        }
        assert_eq!(store.saved.len(), MAX_SAVED);
        assert_eq!(store.saved[0].entry.message, format!("m{}", MAX_SAVED + 4));
        let mut long = entry("long");
        long.content = "x".repeat(MAX_TEXT * 2);
        store.set_saved(long, true, 999_999);
        assert!(store.saved[0].entry.content.chars().count() <= MAX_TEXT);
    }

    #[test]
    fn reminders_need_a_future_time_and_one_per_message() {
        let mut store = SavedStore::default();
        assert!(store.set_reminder(entry("a"), 100, 100).is_err());
        let first = store.set_reminder(entry("a"), 500, 100).unwrap();
        let later = store.set_reminder(entry("b"), 300, 100).unwrap();
        assert_eq!(store.reminders.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), [later.as_str(), first.as_str()], "soonest first");
        let replaced = store.set_reminder(entry("a"), 200, 100).unwrap();
        assert_eq!(store.reminders.len(), 2);
        assert_eq!(store.reminders[0].id, replaced);
        assert!(store.cancel_reminder(&replaced));
        assert!(!store.cancel_reminder(&replaced));
        assert_eq!(store.reminders.len(), 1);
    }

    #[test]
    fn a_saved_file_from_before_new_fields_still_loads() {
        let dir = temp_dir("compat");
        std::fs::write(dir.join(FILE), br#"{"saved":[{"saved_at":5,"message":"x","author_name":"ferris"}]}"#).unwrap();
        let store = SavedStore::load(&dir);
        assert_eq!(store.saved.len(), 1);
        assert_eq!(store.saved[0].entry.author_name, "ferris");
        assert!(store.reminders.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
