use std::path::Path;
use std::sync::Mutex;

use crate::state::App;

pub use commy_view::saved::{Entry, SavedOp, SavedStore};
#[cfg(test)]
use commy_view::saved::{SavedMessage, MAX_SAVED, MAX_TEXT};

const FILE: &str = "saved.json";

static LOCK: Mutex<()> = Mutex::new(());

pub fn load(dir: &Path) -> SavedStore {
    crate::persist::read_json(&dir.join(FILE)).unwrap_or_default()
}

fn store(saved: &SavedStore, dir: &Path) -> Result<(), String> {
    crate::persist::write_json(&dir.join(FILE), saved).map_err(|e| format!("could not save messages: {e}"))
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
    let mut saved = load(&dir);
    change(&mut saved)?;
    store(&saved, &dir)?;
    Ok(saved)
}

#[tauri::command]
pub fn saved_list(app: tauri::State<'_, App>) -> Result<SavedStore, String> {
    let dir = dir_of(&app)?;
    let _guard = LOCK.lock().unwrap();
    Ok(load(&dir))
}

#[tauri::command]
pub fn saved_set(app: tauri::State<'_, App>, entry: Entry, saved: bool) -> Result<SavedStore, String> {
    let at = now_ms();
    let message = entry.message.clone();
    let store = update(&app, |store| {
        store.set_saved(entry, saved, at);
        Ok(())
    })?;
    let kept = store.saved.iter().find(|s| s.entry.message == message).cloned();
    let op = SavedOp { message, at, saved: kept.filter(|_| saved) };
    crate::account_log::queue(&app, crate::account_log::Record::Saved { ops: vec![op] });
    Ok(store)
}

pub fn apply_remote(handle: &tauri::AppHandle, ops: Vec<SavedOp>) {
    use tauri::{Emitter, Manager};
    let app = handle.state::<App>();
    match update(&app, |store| {
        store.apply_ops(ops);
        Ok(())
    }) {
        Ok(store) => {
            let _ = handle.emit("saved-changed", store);
        }
        Err(e) => tracing::debug!("saved messages from another device: {e}"),
    }
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
        super::store(&store, &dir).unwrap();
        assert_eq!(load(&dir), store);
        assert_eq!(load(&dir.join("missing")), SavedStore::default());
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
    fn changes_from_another_device_add_move_and_remove() {
        let mut store = SavedStore::default();
        store.set_saved(entry("a"), true, 1);
        let mut long = entry("b");
        long.content = "x".repeat(MAX_TEXT * 2);
        store.apply_ops(vec![
            SavedOp { message: "b".into(), at: 5, saved: Some(SavedMessage { saved_at: 5, entry: long }) },
            SavedOp { message: "a".into(), at: 6, saved: None },
        ]);
        assert_eq!(store.saved.iter().map(|s| s.entry.message.as_str()).collect::<Vec<_>>(), ["b"]);
        assert!(store.saved[0].entry.content.chars().count() <= MAX_TEXT);
    }

    #[test]
    fn a_saved_file_from_before_new_fields_still_loads() {
        let dir = temp_dir("compat");
        std::fs::write(dir.join(FILE), br#"{"saved":[{"saved_at":5,"message":"x","author_name":"ferris"}]}"#).unwrap();
        let store = load(&dir);
        assert_eq!(store.saved.len(), 1);
        assert_eq!(store.saved[0].entry.author_name, "ferris");
        assert!(store.reminders.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
