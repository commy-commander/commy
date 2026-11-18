use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use commy_core::ServerId;
use commy_net::sealed::SealKey;
use commy_net::sync_streams;
use commy_sync::StreamId;
use serde_json::{json, Value};
use wasm_bindgen_futures::spawn_local;

use super::Session;
use crate::browser_copy::{self, stream_cursors, AccountLog, ServerCopy, Writes, SERVERS_PER_SAVE};

const KEEP_EVERY: Duration = Duration::from_secs(15);

#[derive(Default)]
pub(super) struct Resume {
    pub(super) home: Option<String>,
    pub(super) log: AccountLog,
    pub(super) cursors: Vec<(ServerId, String, HashMap<StreamId, u64>)>,
    pub(super) opened: HashSet<ServerId>,
    pub(super) waiting: Vec<ServerCopy>,
}

impl Session {
    pub(super) async fn resume_from_copy(&self, key: &SealKey) -> Resume {
        let account = sync_streams::account_stream(key).id;
        let records: HashMap<String, String> = match self.host.copy_load().await {
            Ok(value) => value.as_string().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default(),
            Err(_) => HashMap::new(),
        };
        if records.is_empty() {
            return Resume::default();
        }
        let restored = browser_copy::restore(key, account, &records);
        if restored.unreadable {
            self.forget_copy().await;
            return Resume::default();
        }
        let stale: Writes = restored.outdated.into_iter().map(|name| (name, None)).collect();
        let mut resume = Resume { home: restored.home, log: restored.log, ..Default::default() };
        for copy in restored.servers {
            if !self.client.borrow().tickets.contains_key(&copy.server) {
                resume.waiting.push(copy);
                continue;
            }
            let ServerCopy { server, socket, cursors, snapshot } = copy;
            if self.client.borrow().install_copy(snapshot) == Ok(true) {
                resume.opened.insert(server);
                resume.cursors.push((server, socket, stream_cursors(&cursors)));
            }
        }
        if !stale.is_empty() {
            self.write_copy(stale).await;
        }
        if !resume.opened.is_empty() {
            self.emit("servers-changed", Value::Null);
            for server in &resume.opened {
                if self.client.borrow().is_group(*server) {
                    self.emit("dm-changed", json!(server.to_string()));
                }
            }
            self.announce_if_loaded();
        }
        resume
    }

    pub(super) fn restore_cursors(&self, cursors: Vec<(ServerId, String, HashMap<StreamId, u64>)>) {
        let mut guard = self.sync.borrow_mut();
        let Some(live) = guard.as_mut() else { return };
        let home = live.home().to_string();
        for (server, socket, kept) in cursors {
            let socket = (socket != home).then_some(socket);
            live.book.restore(server, socket, kept);
        }
    }

    pub(super) fn open_waiting(self: &Rc<Self>, waiting: Vec<ServerCopy>, cursors_hold: bool) {
        if waiting.is_empty() {
            return;
        }
        let Some(listed) = self.sync.borrow().as_ref().map(|live| live.book.tickets()) else { return };
        let mut stale = Writes::new();
        let mut added = Vec::new();
        let mut cursors = Vec::new();
        for ServerCopy { server, socket, cursors: kept, snapshot } in waiting {
            let Some(ticket) = listed.get(&server).cloned() else {
                stale.insert(browser_copy::server_name(server), None);
                continue;
            };
            let opened = {
                let mut client = self.client.borrow_mut();
                if client.tickets.contains_key(&server) {
                    continue;
                }
                client.tickets.insert(server, ticket.clone());
                client.install_copy(snapshot) == Ok(true)
            };
            added.push(ticket);
            if opened {
                if let Some(live) = self.sync.borrow_mut().as_mut() {
                    live.keeping.saved.insert(server);
                }
                cursors.push((server, socket, stream_cursors(&kept)));
            }
        }
        if cursors_hold {
            self.restore_cursors(cursors);
        }
        if !stale.is_empty() {
            let this = self.clone();
            spawn_local(async move {
                this.write_copy(stale).await;
            });
        }
        if !added.is_empty() {
            self.emit("servers-changed", Value::Null);
            spawn_local(self.clone().load_servers(added));
        }
    }

    pub(super) fn keep_copy(self: &Rc<Self>) {
        let Some(key) = self.status.borrow().key.clone() else { return };
        let (writes, written) = {
            let mut guard = self.sync.borrow_mut();
            let Some(live) = guard.as_mut() else { return };
            if live.keeping.writing || live.keeping.given_up() {
                return;
            }
            let (due, later) = browser_copy::pick_due(live.book.take_dirty(), SERVERS_PER_SAVE);
            for server in later {
                live.book.touch(server);
            }
            let account = live.account;
            let client = self.client.borrow();
            let servers = client.servers.lock().unwrap();
            let mut writes = Writes::new();
            let mut written = Vec::new();
            for server in due {
                let Some(state) = servers.get(&server) else { continue };
                let copy = ServerCopy {
                    server,
                    socket: live.socket_of(server),
                    cursors: browser_copy::cursor_names(&live.book.cursors_of(server)),
                    snapshot: state.snapshot(),
                };
                if let Some(sealed) = browser_copy::seal_server(&key, account, &copy) {
                    writes.insert(browser_copy::server_name(server), Some(sealed));
                    live.keeping.saved.insert(server);
                    written.push(server);
                }
            }
            writes.extend(live.keeping.removals(|server| client.tickets.contains_key(&server) && servers.contains_key(&server)));
            let home = live.home().to_string();
            writes.extend(live.keeping.log.writes(&key, account, &home));
            if writes.is_empty() {
                return;
            }
            live.keeping.writing = true;
            (writes, written)
        };
        let this = self.clone();
        spawn_local(async move {
            let kept = this.write_copy(writes).await;
            let mut guard = this.sync.borrow_mut();
            let Some(live) = guard.as_mut() else { return };
            live.keeping.writing = false;
            if kept {
                live.keeping.failures = 0;
                return;
            }
            live.keeping.failures += 1;
            live.keeping.log.rewind();
            for server in written {
                live.book.touch(server);
            }
        });
    }

    pub(super) fn touch_copy(&self, server: ServerId) {
        if let Some(live) = self.sync.borrow_mut().as_mut() {
            live.book.touch(server);
        }
    }

    pub(super) async fn tend_copy(self: Rc<Self>) {
        loop {
            n0_future::time::sleep(KEEP_EVERY).await;
            if self.sync.borrow().is_none() {
                return;
            }
            self.keep_copy();
        }
    }

    async fn write_copy(&self, writes: Writes) -> bool {
        let Ok(text) = serde_json::to_string(&writes) else { return false };
        matches!(self.host.copy_write(&text).await, Ok(answer) if answer.as_bool() == Some(true))
    }

    pub(super) async fn forget_copy(&self) {
        let _ = self.host.copy_clear().await;
    }
}
