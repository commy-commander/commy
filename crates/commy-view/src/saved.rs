use serde::{Deserialize, Serialize};

pub const MAX_SAVED: usize = 500;
pub const MAX_REMINDERS: usize = 200;
pub const MAX_TEXT: usize = 400;

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
        self.content = crate::truncate(&self.content, MAX_TEXT);
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

    pub fn apply_ops(&mut self, ops: Vec<SavedOp>) {
        for op in ops {
            self.saved.retain(|s| s.entry.message != op.message);
            if let Some(mut kept) = op.saved {
                kept.entry = kept.entry.trimmed();
                self.saved.push(kept);
            }
        }
        self.saved.sort_by(|a, b| b.saved_at.cmp(&a.saved_at).then_with(|| b.entry.message.cmp(&a.entry.message)));
        self.saved.truncate(MAX_SAVED);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedOp {
    pub message: String,
    pub at: i64,
    #[serde(default)]
    pub saved: Option<SavedMessage>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(message: &str) -> Entry {
        Entry { message: message.into(), content: "hello".into(), ..Entry::default() }
    }

    #[test]
    fn a_change_from_another_device_replaces_the_local_copy() {
        let mut store = SavedStore::default();
        store.set_saved(entry("a"), true, 1);
        store.set_saved(entry("b"), true, 2);
        store.apply_ops(vec![
            SavedOp { message: "a".into(), at: 3, saved: Some(SavedMessage { saved_at: 3, entry: entry("a") }) },
            SavedOp { message: "b".into(), at: 4, saved: None },
        ]);
        assert_eq!(store.saved.iter().map(|s| s.entry.message.as_str()).collect::<Vec<_>>(), ["a"]);
        assert_eq!(store.saved[0].saved_at, 3);
    }
}
