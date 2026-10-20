use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::Utc;
use commy_core::{ChannelId, Message, ServerId, UserId};
use commy_net::sealed::{Purpose, SealKey};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::entry_list::{self, Fetched, Put, Session};
use crate::state::App;

const LOCAL_FILE: &str = "read-state.json";
const VAULT_ITEM: &str = "read-state";
const PUSH_DELAY: std::time::Duration = std::time::Duration::from_secs(3);
const PULL_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadMark {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub at: i64,
    #[serde(default)]
    pub set: i64,
    #[serde(default)]
    pub back: bool,
}

impl ReadMark {
    fn newer_than(&self, other: &ReadMark) -> bool {
        if self.back || other.back {
            (self.set, self.at, &self.id) > (other.set, other.at, &other.id)
        } else {
            (self.at, &self.id, self.set) > (other.at, &other.id, other.set)
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadState {
    #[serde(default)]
    pub channels: BTreeMap<String, ReadMark>,
}

impl ReadState {
    pub fn merge(a: &ReadState, b: &ReadState) -> ReadState {
        let mut channels = a.channels.clone();
        for (channel, mark) in &b.channels {
            let take = channels.get(channel).map_or(true, |mine| mark.newer_than(mine));
            if take {
                channels.insert(channel.clone(), mark.clone());
            }
        }
        ReadState { channels }
    }

    fn changed_from(&self, before: &ReadState) -> Vec<String> {
        self.channels
            .iter()
            .filter(|(channel, mark)| before.channels.get(*channel) != Some(*mark))
            .map(|(channel, _)| channel.clone())
            .collect()
    }

    fn seal(&self, key: &SealKey) -> String {
        key.seal(Purpose::ReadState, &serde_json::to_vec(self).expect("read state serialization cannot fail"))
    }

    fn open(key: &SealKey, sealed: &str) -> Result<ReadState, String> {
        let bytes = key.open(Purpose::ReadState, sealed).map_err(|_| "The read positions were sealed with another key".to_string())?;
        serde_json::from_slice(&bytes).map_err(|_| "The read positions are damaged".to_string())
    }
}

pub fn count_after(messages: &[Message], mark: &ReadMark, me: Option<UserId>, blocked: &HashSet<UserId>) -> usize {
    let anchored = mark.id.as_deref().and_then(|id| messages.iter().position(|m| m.id.to_string() == id));
    let unread: Box<dyn Iterator<Item = &Message>> = match anchored {
        Some(index) => Box::new(messages[index + 1..].iter()),
        None => Box::new(messages.iter().filter(|m| m.created_at.timestamp_millis() > mark.at)),
    };
    unread.filter(|m| Some(m.author) != me && !blocked.contains(&m.author)).count()
}

pub fn latest_mark(messages: &[Message], now: i64) -> Option<ReadMark> {
    let last = messages.last()?;
    Some(ReadMark { id: Some(last.id.to_string()), at: last.created_at.timestamp_millis(), set: now, back: false })
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChannelRead {
    pub mark: ReadMark,
    pub server: Option<String>,
    pub unread: usize,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ReadView {
    pub channels: BTreeMap<String, ChannelRead>,
}

fn local_path(app: &App) -> Option<std::path::PathBuf> {
    app.data_dir().map(|dir| dir.join(LOCAL_FILE))
}

fn local(app: &App) -> ReadState {
    local_path(app).and_then(|path| entry_list::read(&path)).unwrap_or_default()
}

fn save_local(app: &App, state: &ReadState) {
    if let Some(path) = local_path(app) {
        entry_list::write(&path, state);
    }
}

fn view_of(app: &App, state: &ReadState, only: Option<&[String]>) -> ReadView {
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let blocked: HashSet<UserId> = app.safety.lock().unwrap().blocked.iter().copied().collect();
    let servers = app.servers.lock().unwrap();
    let channels = state
        .channels
        .iter()
        .filter(|(channel, _)| only.map_or(true, |list| list.contains(channel)))
        .map(|(channel, mark)| {
            let cid: Option<ChannelId> = channel.parse().ok();
            let found = cid.and_then(|cid| servers.values().find(|s| s.server.channel(cid).is_some()).map(|s| (s, cid)));
            let (server, unread) = match found {
                Some((state, cid)) if me.is_none_or(|me| state.server.can_view(cid, me)) => {
                    (Some(state.server.id.to_string()), count_after(state.messages(cid), mark, me, &blocked))
                }
                Some((state, _)) => (Some(state.server.id.to_string()), 0),
                None => (None, 0),
            };
            (channel.clone(), ChannelRead { mark: mark.clone(), server, unread })
        })
        .collect();
    ReadView { channels }
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn absorb(app: &App, marks: BTreeMap<String, ReadMark>) -> (ReadState, Vec<String>) {
    let before = local(app);
    let merged = ReadState::merge(&before, &ReadState { channels: marks });
    let changed = merged.changed_from(&before);
    if !changed.is_empty() {
        save_local(app, &merged);
    }
    (merged, changed)
}

#[tauri::command]
pub fn read_state_load(app: tauri::State<'_, App>) -> ReadView {
    view_of(&app, &local(&app), None)
}

#[tauri::command]
pub fn read_state_set(handle: AppHandle, app: tauri::State<'_, App>, channels: BTreeMap<String, ReadMark>, flush: bool) {
    let (_, changed) = absorb(&app, channels);
    if changed.is_empty() {
        return;
    }
    if flush {
        schedule(&handle);
    } else {
        schedule_push(&handle);
    }
}

#[tauri::command]
pub fn read_state_mark_read(
    handle: AppHandle,
    app: tauri::State<'_, App>,
    server_id: String,
    channel_ids: Option<Vec<String>>,
) -> Result<ReadView, String> {
    let sid: ServerId = server_id.parse().map_err(|_| "unknown server".to_string())?;
    let now = now_ms();
    let marks: BTreeMap<String, ReadMark> = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        state
            .server
            .channels
            .iter()
            .filter(|c| channel_ids.as_ref().map_or(true, |ids| ids.contains(&c.id.to_string())))
            .filter_map(|c| latest_mark(state.messages(c.id), now).map(|mark| (c.id.to_string(), mark)))
            .collect()
    };
    let touched: Vec<String> = marks.keys().cloned().collect();
    let (merged, changed) = absorb(&app, marks);
    if !changed.is_empty() {
        schedule(&handle);
    }
    Ok(view_of(&app, &merged, Some(&touched)))
}

#[tauri::command]
pub async fn read_state_sync(handle: AppHandle) -> Result<ReadView, String> {
    sync(&handle).await?;
    let app = handle.state::<App>();
    Ok(view_of(&app, &local(&app), None))
}

static SYNC: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static PUSH_PENDING: AtomicBool = AtomicBool::new(false);

pub fn schedule(handle: &AppHandle) {
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = sync(&handle).await {
            tracing::debug!("read state sync: {e}");
        }
    });
}

fn schedule_push(handle: &AppHandle) {
    if PUSH_PENDING.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(PUSH_DELAY).await;
        PUSH_PENDING.store(false, Ordering::SeqCst);
        schedule(&handle);
    });
}

pub async fn run(handle: AppHandle) {
    loop {
        tokio::time::sleep(PULL_EVERY).await;
        if let Err(e) = sync(&handle).await {
            tracing::debug!("read state pull: {e}");
        }
    }
}

fn key_for(app: &App, s: &Session) -> Option<SealKey> {
    let dir = app.data_dir()?;
    entry_list::load_key(dir, &s.server, &s.account)
}

async fn sync(handle: &AppHandle) -> Result<(), String> {
    let _guard = SYNC.lock().await;
    let app = handle.state::<App>();
    let Some(s) = entry_list::session(&app) else { return Ok(()) };
    if app.account.lock().unwrap().features.is_some_and(|f| !f.vault) {
        return Ok(());
    }
    let Some(key) = key_for(&app, &s) else { return Ok(()) };
    for _ in 0..4 {
        let item = match entry_list::fetch(&app, &s, VAULT_ITEM).await? {
            Fetched::Item(item) => item,
            Fetched::Unsupported => return Ok(()),
        };
        let shared = match item.data.as_deref() {
            Some(data) => ReadState::open(&key, data)?,
            None => ReadState::default(),
        };
        let before = local(&app);
        let merged = ReadState::merge(&before, &shared);
        let changed = merged.changed_from(&before);
        if !changed.is_empty() {
            save_local(&app, &merged);
            let _ = handle.emit("read-state-changed", view_of(&app, &merged, Some(&changed)));
        }
        if merged == shared && item.data.is_some() {
            return Ok(());
        }
        match entry_list::put(&app, &s, VAULT_ITEM, merged.seal(&key), item.version).await? {
            Put::Done => return Ok(()),
            Put::Conflict => continue,
        }
    }
    Err("Your devices kept changing the read positions at the same time; they will sync again soon".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use commy_core::MessageId;

    fn mark(id: &str, at: i64, set: i64) -> ReadMark {
        ReadMark { id: Some(id.into()), at, set, back: false }
    }

    fn back(id: &str, at: i64, set: i64) -> ReadMark {
        ReadMark { back: true, ..mark(id, at, set) }
    }

    fn message(channel: ChannelId, author: UserId, minute: i64) -> Message {
        Message {
            id: MessageId::new(),
            channel,
            author,
            content: "hi".into(),
            created_at: Utc.timestamp_opt(1_760_000_000 + minute * 60, 0).unwrap(),
            edited_at: None,
            attachments: vec![],
            reactions: vec![],
            reply_to: None,
            sig: None,
        }
    }

    fn one(mark: ReadMark) -> ReadState {
        ReadState { channels: BTreeMap::from([("c".to_string(), mark)]) }
    }

    #[test]
    fn plain_reads_merge_to_the_furthest_position() {
        let far = one(mark("z", 500, 100));
        let stale = one(mark("b", 200, 150));
        assert_eq!(ReadState::merge(&far, &stale).channels["c"], mark("z", 500, 100));
        assert_eq!(ReadState::merge(&stale, &far).channels["c"], mark("z", 500, 100));
        let rewritten = one(mark("z", 500, 120));
        assert_eq!(ReadState::merge(&far, &rewritten).channels["c"], mark("z", 500, 120), "same position: the later write");
    }

    #[test]
    fn marking_unread_wins_by_write_time_and_a_later_read_wins_it_back() {
        let read = one(mark("z", 500, 100));
        let unread = one(back("b", 200, 150));
        assert_eq!(ReadState::merge(&read, &unread).channels["c"], back("b", 200, 150));
        assert_eq!(ReadState::merge(&unread, &read).channels["c"], back("b", 200, 150));
        let read_again = one(mark("z", 500, 300));
        assert_eq!(ReadState::merge(&unread, &read_again).channels["c"], mark("z", 500, 300));
        assert_eq!(ReadState::merge(&read_again, &unread).channels["c"], mark("z", 500, 300));
    }

    #[test]
    fn merging_keeps_channels_only_one_side_knows_and_reports_what_moved() {
        let mine = ReadState { channels: BTreeMap::from([("a".to_string(), mark("1", 1, 1)), ("b".to_string(), mark("2", 2, 2))]) };
        let theirs = ReadState { channels: BTreeMap::from([("b".to_string(), mark("3", 3, 3)), ("c".to_string(), mark("4", 4, 4))]) };
        let merged = ReadState::merge(&mine, &theirs);
        assert_eq!(merged.channels.len(), 3);
        assert_eq!(merged.changed_from(&mine), vec!["b".to_string(), "c".to_string()]);
        assert_eq!(merged.changed_from(&merged), Vec::<String>::new());
    }

    #[test]
    fn unread_counts_skip_my_messages_and_blocked_people_and_fall_back_to_time() {
        let channel = ChannelId::new();
        let (me, friend, pest) = (UserId::new(), UserId::new(), UserId::new());
        let messages = vec![
            message(channel, friend, 0),
            message(channel, friend, 1),
            message(channel, me, 2),
            message(channel, pest, 3),
            message(channel, friend, 4),
        ];
        let blocked = HashSet::from([pest]);
        let at_first = ReadMark { id: Some(messages[0].id.to_string()), at: messages[0].created_at.timestamp_millis(), set: 1, back: false };
        assert_eq!(count_after(&messages, &at_first, Some(me), &blocked), 2);
        let gone = ReadMark { id: Some("missing".into()), at: messages[1].created_at.timestamp_millis(), set: 1, back: false };
        assert_eq!(count_after(&messages, &gone, Some(me), &blocked), 1, "by time when the marked message is gone");
        let before_all = ReadMark { id: None, at: 0, set: 1, back: true };
        assert_eq!(count_after(&messages, &before_all, Some(me), &HashSet::new()), 4);
        let latest = latest_mark(&messages, 9).unwrap();
        assert_eq!(latest.id.as_deref(), Some(messages[4].id.to_string().as_str()));
        assert_eq!(count_after(&messages, &latest, Some(me), &blocked), 0);
        assert_eq!(latest_mark(&[], 9), None);
    }

    #[test]
    fn sealed_read_positions_round_trip_and_reject_other_keys() {
        let state = ReadState { channels: BTreeMap::from([("c".to_string(), mark("m", 5, 6))]) };
        let key = SealKey::generate();
        let sealed = state.seal(&key);
        assert_eq!(ReadState::open(&key, &sealed).unwrap(), state);
        assert!(ReadState::open(&SealKey::generate(), &sealed).is_err());
        let older_build: ReadState = serde_json::from_str(r#"{"channels":{"c":{"at":5}}}"#).unwrap();
        assert_eq!(older_build.channels["c"], ReadMark { id: None, at: 5, set: 0, back: false });
    }
}
