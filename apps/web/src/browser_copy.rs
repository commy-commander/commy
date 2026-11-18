use std::collections::{BTreeMap, HashMap, HashSet};

use commy_core::{ServerId, Snapshot};
use commy_net::sealed::{Purpose, SealKey};
use commy_sync::StreamId;
use data_encoding::BASE64URL_NOPAD;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub const COPY_VERSION: u32 = 1;
pub const ACCOUNT_PAGE: usize = 500;
pub const SERVERS_PER_SAVE: usize = 8;
pub const GIVE_UP_AFTER_FAILURES: u32 = 3;
const ACCOUNT_HEAD: &str = "account";
const ACCOUNT_PAGE_PREFIX: &str = "account-page-";
const SERVER_PREFIX: &str = "server-";

pub type Writes = BTreeMap<String, Option<String>>;

#[derive(Serialize, Deserialize)]
struct Envelope<T> {
    version: u32,
    name: String,
    account: String,
    body: T,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct AccountHead {
    home: String,
    after: u64,
    pages: usize,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct AccountPage {
    entries: Vec<(u64, String)>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServerCopy {
    pub server: ServerId,
    pub socket: String,
    pub cursors: BTreeMap<String, u64>,
    pub snapshot: Snapshot,
}

impl ServerCopy {
    pub fn stream_cursors(&self) -> HashMap<StreamId, u64> {
        stream_cursors(&self.cursors)
    }
}

pub fn stream_cursors(cursors: &BTreeMap<String, u64>) -> HashMap<StreamId, u64> {
    cursors.iter().filter_map(|(stream, seq)| Some((StreamId::from_hex(stream)?, *seq))).collect()
}

pub fn cursor_names(cursors: &HashMap<StreamId, u64>) -> BTreeMap<String, u64> {
    cursors.iter().map(|(stream, seq)| (stream.to_hex(), *seq)).collect()
}

pub fn server_name(server: ServerId) -> String {
    format!("{SERVER_PREFIX}{server}")
}

fn is_account_name(name: &str) -> bool {
    name == ACCOUNT_HEAD || name.starts_with(ACCOUNT_PAGE_PREFIX)
}

fn page_name(index: usize) -> String {
    format!("{ACCOUNT_PAGE_PREFIX}{index}")
}

fn seal<T: Serialize>(key: &SealKey, account: StreamId, name: &str, body: T) -> Option<String> {
    let envelope = Envelope { version: COPY_VERSION, name: name.to_string(), account: account.to_hex(), body };
    let plain = serde_json::to_vec(&envelope).ok()?;
    Some(key.seal(Purpose::BrowserCopy, &plain))
}

enum Opened {
    Kept(serde_json::Value),
    Outdated,
    Unreadable,
}

fn open(key: &SealKey, account: StreamId, name: &str, sealed: &str) -> Opened {
    let Ok(plain) = key.open(Purpose::BrowserCopy, sealed) else { return Opened::Unreadable };
    let Ok(envelope) = serde_json::from_slice::<Envelope<serde_json::Value>>(&plain) else { return Opened::Outdated };
    if envelope.account != account.to_hex() {
        return Opened::Unreadable;
    }
    if envelope.version != COPY_VERSION || envelope.name != name {
        return Opened::Outdated;
    }
    Opened::Kept(envelope.body)
}

fn take<T: DeserializeOwned>(opened: &mut HashMap<String, serde_json::Value>, name: &str) -> Option<T> {
    serde_json::from_value(opened.remove(name)?).ok()
}

pub fn seal_server(key: &SealKey, account: StreamId, copy: &ServerCopy) -> Option<String> {
    seal(key, account, &server_name(copy.server), copy)
}

#[derive(Default, Debug, PartialEq)]
pub struct AccountLog {
    entries: Vec<(u64, Vec<u8>)>,
    after: u64,
    written: usize,
    written_after: u64,
    stored_pages: usize,
}

impl AccountLog {
    pub fn note(&mut self, seq: u64, plain: &[u8]) {
        if seq <= self.after {
            return;
        }
        self.entries.push((seq, plain.to_vec()));
        self.after = seq;
    }

    pub fn skip(&mut self, seq: u64) {
        self.after = self.after.max(seq);
    }

    pub fn after(&self) -> u64 {
        self.after
    }

    pub fn entries(&self) -> impl Iterator<Item = (u64, &[u8])> {
        self.entries.iter().map(|(seq, plain)| (*seq, plain.as_slice()))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn rewind(&mut self) {
        self.written = 0;
        self.written_after = 0;
    }

    pub fn restart(&mut self) {
        let stored_pages = self.stored_pages;
        *self = AccountLog { stored_pages, ..Default::default() };
    }

    fn pages(&self) -> usize {
        self.entries.len().div_ceil(ACCOUNT_PAGE)
    }

    pub fn writes(&mut self, key: &SealKey, account: StreamId, home: &str) -> Writes {
        let mut writes = Writes::new();
        let changed = self.written != self.entries.len() || self.written_after != self.after || self.stored_pages != self.pages();
        if !changed {
            return writes;
        }
        let pages = self.pages();
        let first = if self.written < self.entries.len() { self.written / ACCOUNT_PAGE } else { pages };
        for index in first..pages {
            let entries = self.entries[index * ACCOUNT_PAGE..((index + 1) * ACCOUNT_PAGE).min(self.entries.len())]
                .iter()
                .map(|(seq, plain)| (*seq, BASE64URL_NOPAD.encode(plain)))
                .collect();
            let name = page_name(index);
            writes.insert(name.clone(), seal(key, account, &name, AccountPage { entries }));
        }
        for index in pages..self.stored_pages {
            writes.insert(page_name(index), None);
        }
        let head = AccountHead { home: home.to_string(), after: self.after, pages };
        writes.insert(ACCOUNT_HEAD.to_string(), seal(key, account, ACCOUNT_HEAD, head));
        self.written = self.entries.len();
        self.written_after = self.after;
        self.stored_pages = pages;
        writes
    }
}

#[derive(Default, Debug)]
pub struct Restored {
    pub home: Option<String>,
    pub log: AccountLog,
    pub servers: Vec<ServerCopy>,
    pub unreadable: bool,
    pub outdated: Vec<String>,
}

fn open_account(opened: &mut HashMap<String, serde_json::Value>) -> Option<(String, AccountLog)> {
    let head: AccountHead = take(opened, ACCOUNT_HEAD)?;
    let mut entries = Vec::new();
    for index in 0..head.pages {
        let page: AccountPage = take(opened, &page_name(index))?;
        let full = index + 1 == head.pages || page.entries.len() == ACCOUNT_PAGE;
        if !full {
            return None;
        }
        for (seq, plain) in page.entries {
            entries.push((seq, BASE64URL_NOPAD.decode(plain.as_bytes()).ok()?));
        }
    }
    let ordered = entries.windows(2).all(|pair| pair[0].0 < pair[1].0);
    let within = entries.last().is_none_or(|(seq, _)| *seq <= head.after);
    if !ordered || !within {
        return None;
    }
    let written = entries.len();
    let log = AccountLog { entries, after: head.after, written, written_after: head.after, stored_pages: head.pages };
    Some((head.home, log))
}

pub fn restore(key: &SealKey, account: StreamId, records: &HashMap<String, String>) -> Restored {
    let mut restored = Restored::default();
    let mut opened = HashMap::new();
    for (name, sealed) in records {
        match open(key, account, name, sealed) {
            Opened::Kept(body) => {
                opened.insert(name.clone(), body);
            }
            Opened::Outdated => restored.outdated.push(name.clone()),
            Opened::Unreadable => return Restored { unreadable: true, ..Default::default() },
        }
    }
    let account_names: Vec<String> = opened.keys().filter(|name| is_account_name(name)).cloned().collect();
    match open_account(&mut opened) {
        Some((home, log)) => {
            restored.home = Some(home);
            restored.log = log;
        }
        None => {
            for name in account_names {
                opened.remove(&name);
                restored.outdated.push(name);
            }
        }
    }
    let names: Vec<String> = opened.keys().cloned().collect();
    for name in names {
        let copy = name.starts_with(SERVER_PREFIX).then(|| take::<ServerCopy>(&mut opened, &name)).flatten();
        match copy {
            Some(copy) if server_name(copy.server) == name && copy.snapshot.server.id == copy.server => restored.servers.push(copy),
            _ => restored.outdated.push(name),
        }
    }
    restored.outdated.sort();
    restored
}

pub fn same_socket(saved: &str, now: Option<&str>, home: &str) -> bool {
    saved == now.unwrap_or(home)
}

#[derive(Default)]
pub struct Keeping {
    pub log: AccountLog,
    pub saved: HashSet<ServerId>,
    pub failures: u32,
    pub writing: bool,
}

impl Keeping {
    pub fn given_up(&self) -> bool {
        self.failures >= GIVE_UP_AFTER_FAILURES
    }

    pub fn removals(&mut self, still_held: impl Fn(ServerId) -> bool) -> Writes {
        let gone: Vec<ServerId> = self.saved.iter().copied().filter(|server| !still_held(*server)).collect();
        gone.into_iter()
            .map(|server| {
                self.saved.remove(&server);
                (server_name(server), None)
            })
            .collect()
    }
}

pub fn pick_due(dirty: HashSet<ServerId>, limit: usize) -> (Vec<ServerId>, Vec<ServerId>) {
    let mut now = Vec::new();
    let mut later = Vec::new();
    for server in dirty {
        if now.len() < limit { now.push(server) } else { later.push(server) }
    }
    (now, later)
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Server, ServerState, UserId};

    fn account() -> StreamId {
        StreamId([7; 32])
    }

    fn records(writes: Writes) -> HashMap<String, String> {
        writes.into_iter().filter_map(|(name, value)| Some((name, value?))).collect()
    }

    fn server_copy(name: &str) -> ServerCopy {
        let state = ServerState::new(Server::new(name, UserId::new()));
        let id = state.server.id;
        ServerCopy {
            server: id,
            socket: "https://commy.chat/sync".into(),
            cursors: [(StreamId([1; 32]).to_hex(), 41)].into(),
            snapshot: state.snapshot(),
        }
    }

    #[test]
    fn an_account_log_comes_back_in_order_with_its_position() {
        let key = SealKey::generate();
        let mut log = AccountLog::default();
        for seq in 1..=1_203 {
            log.note(seq, format!("record {seq}").as_bytes());
        }
        log.skip(1_210);
        let kept = records(log.writes(&key, account(), "https://commy.chat/sync"));
        assert_eq!(kept.len(), 4, "three pages and a head");
        let back = restore(&key, account(), &kept);
        assert!(!back.unreadable);
        assert_eq!(back.home.as_deref(), Some("https://commy.chat/sync"));
        assert_eq!(back.log.after(), 1_210);
        assert_eq!(back.log.len(), 1_203);
        assert_eq!(back.log.entries().nth(1_202), Some((1_203, b"record 1203".as_slice())));
    }

    #[test]
    fn only_the_newest_page_is_written_again() {
        let key = SealKey::generate();
        let mut log = AccountLog::default();
        for seq in 1..=ACCOUNT_PAGE as u64 + 3 {
            log.note(seq, b"x");
        }
        assert_eq!(log.writes(&key, account(), "h").len(), 3);
        assert!(log.writes(&key, account(), "h").is_empty(), "nothing new, nothing written");
        log.note(ACCOUNT_PAGE as u64 + 4, b"y");
        let again: Vec<String> = log.writes(&key, account(), "h").into_keys().collect();
        assert_eq!(again, vec!["account".to_string(), "account-page-1".to_string()]);
        log.skip(9_000);
        let moved: Vec<String> = log.writes(&key, account(), "h").into_keys().collect();
        assert_eq!(moved, vec!["account".to_string()], "a skipped entry moves only the position");
    }

    #[test]
    fn a_failed_write_is_sent_again_in_full() {
        let key = SealKey::generate();
        let mut log = AccountLog::default();
        for seq in 1..=ACCOUNT_PAGE as u64 * 2 {
            log.note(seq, b"x");
        }
        log.writes(&key, account(), "h");
        log.rewind();
        assert_eq!(log.writes(&key, account(), "h").len(), 3);
    }

    #[test]
    fn a_restarted_log_removes_the_pages_it_no_longer_has() {
        let key = SealKey::generate();
        let mut log = AccountLog::default();
        for seq in 1..=ACCOUNT_PAGE as u64 * 2 {
            log.note(seq, b"x");
        }
        log.writes(&key, account(), "old");
        log.restart();
        log.note(1, b"fresh");
        let writes = log.writes(&key, account(), "new");
        assert_eq!(writes.get("account-page-1"), Some(&None));
        assert!(writes.get("account-page-0").is_some_and(Option::is_some));
        let back = restore(&key, account(), &records(writes));
        assert_eq!(back.home.as_deref(), Some("new"));
        assert_eq!(back.log.len(), 1);
    }

    #[test]
    fn an_older_entry_is_not_kept_twice() {
        let mut log = AccountLog::default();
        log.note(5, b"a");
        log.note(5, b"a");
        log.note(3, b"b");
        assert_eq!(log.len(), 1);
        assert_eq!(log.after(), 5);
    }

    #[test]
    fn a_missing_page_drops_the_whole_account_copy() {
        let key = SealKey::generate();
        let mut log = AccountLog::default();
        for seq in 1..=ACCOUNT_PAGE as u64 + 1 {
            log.note(seq, b"x");
        }
        let mut kept = records(log.writes(&key, account(), "h"));
        kept.remove("account-page-0");
        let back = restore(&key, account(), &kept);
        assert!(back.log.is_empty());
        assert_eq!(back.log.after(), 0);
        assert_eq!(back.outdated, vec!["account".to_string(), "account-page-1".to_string()]);
    }

    #[test]
    fn a_server_copy_comes_back_with_its_cursors() {
        let key = SealKey::generate();
        let copy = server_copy("Garden");
        let kept: HashMap<String, String> = [(server_name(copy.server), seal_server(&key, account(), &copy).unwrap())].into();
        let back = restore(&key, account(), &kept);
        assert_eq!(back.servers.len(), 1);
        assert_eq!(back.servers[0].snapshot.server.name, "Garden");
        assert_eq!(back.servers[0].stream_cursors(), [(StreamId([1; 32]), 41)].into());
    }

    #[test]
    fn a_copy_for_another_account_or_key_is_unreadable() {
        let key = SealKey::generate();
        let copy = server_copy("Garden");
        let kept: HashMap<String, String> = [(server_name(copy.server), seal_server(&key, account(), &copy).unwrap())].into();
        assert!(restore(&SealKey::generate(), account(), &kept).unreadable);
        assert!(restore(&key, StreamId([9; 32]), &kept).unreadable);
        let plain: HashMap<String, String> = [("server-x".to_string(), "not sealed".to_string())].into();
        assert!(restore(&key, account(), &plain).unreadable);
    }

    #[test]
    fn a_copy_moved_to_another_name_is_dropped() {
        let key = SealKey::generate();
        let copy = server_copy("Garden");
        let other = server_name(ServerId::new());
        let kept: HashMap<String, String> = [(other.clone(), seal_server(&key, account(), &copy).unwrap())].into();
        let back = restore(&key, account(), &kept);
        assert!(back.servers.is_empty());
        assert_eq!(back.outdated, vec![other]);
    }

    #[test]
    fn the_sealed_copy_hides_what_it_holds() {
        let key = SealKey::generate();
        let copy = server_copy("Secret garden");
        let sealed = seal_server(&key, account(), &copy).unwrap();
        assert!(!sealed.contains("Secret"));
        assert!(!sealed.contains(&copy.server.to_string()));
    }

    #[test]
    fn cursors_count_only_on_the_service_that_gave_them() {
        assert!(same_socket("https://commy.chat/sync", None, "https://commy.chat/sync"));
        assert!(same_socket("https://sync.example.org", Some("https://sync.example.org"), "https://commy.chat/sync"));
        assert!(!same_socket("https://commy.chat/sync", Some("https://sync.example.org"), "https://commy.chat/sync"));
        assert!(!same_socket("https://commy.chat/sync", None, "https://staging.commy.chat/sync"));
    }

    #[test]
    fn servers_that_left_the_list_are_removed() {
        let (kept, gone) = (ServerId::new(), ServerId::new());
        let mut keeping = Keeping { saved: [kept, gone].into(), ..Default::default() };
        let writes = keeping.removals(|server| server == kept);
        assert_eq!(writes, [(server_name(gone), None)].into());
        assert_eq!(keeping.saved, [kept].into());
    }

    #[test]
    fn a_busy_moment_saves_a_few_servers_at_a_time() {
        let dirty: HashSet<ServerId> = (0..20).map(|_| ServerId::new()).collect();
        let (now, later) = pick_due(dirty, SERVERS_PER_SAVE);
        assert_eq!(now.len(), SERVERS_PER_SAVE);
        assert_eq!(later.len(), 12);
    }

    #[test]
    fn writing_stops_after_repeated_failures() {
        let mut keeping = Keeping::default();
        for _ in 0..GIVE_UP_AFTER_FAILURES {
            assert!(!keeping.given_up());
            keeping.failures += 1;
        }
        assert!(keeping.given_up());
    }
}
