use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use chrono::Utc;
use commy_core::ServerId;
use commy_net::checkpoint::{self, Assembler};
use commy_net::community_streams::{self as streams, Lane};
use commy_sync::errands::{self, owner_work, Errand};
use commy_sync::{StreamId, StreamKeys, SyncClient};
use wasm_bindgen_futures::spawn_local;

use super::sync::Live;
use super::Session;

const PATIENCE_MS: i64 = 20_000;
const WRITE_WAIT: std::time::Duration = std::time::Duration::from_secs(8);

struct Reading {
    server: ServerId,
    state: StreamKeys,
    url: String,
    assembler: Assembler,
    looked_back: bool,
    started_ms: i64,
}

#[derive(Default)]
pub(super) struct Checkpoints {
    reading: HashMap<StreamId, Reading>,
    tried: HashSet<StreamId>,
}

impl Checkpoints {
    pub(super) fn reads_first(&self, lane: Lane, state: StreamId, missing: bool) -> bool {
        missing && lane == Lane::State && !self.tried.contains(&state)
    }
}

pub(super) fn start_reading(live: &mut Live, server: ServerId, state: StreamKeys, url: &str) {
    let keys = checkpoint::keys_for(&state);
    live.pool.follow_together_tail_at(url, vec![(keys.clone(), 0)], Some(checkpoint::FIRST_LOOK));
    let reading = Reading { server, state, url: url.to_string(), assembler: Assembler::default(), looked_back: false, started_ms: Utc::now().timestamp_millis() };
    live.checkpoints.reading.insert(keys.id, reading);
}

impl Session {
    pub(super) fn on_checkpoint_entry(&self, stream: StreamId, seq: u64, plain: &[u8]) -> bool {
        let mut guard = self.sync.borrow_mut();
        let Some(read) = guard.as_mut().and_then(|live| live.checkpoints.reading.get_mut(&stream)) else { return false };
        read.assembler.take(seq, plain);
        true
    }

    pub(super) fn on_checkpoint_caught_up(self: &Rc<Self>, stream: StreamId) -> bool {
        {
            let mut guard = self.sync.borrow_mut();
            let Some(live) = guard.as_mut() else { return false };
            let Some(read) = live.checkpoints.reading.get_mut(&stream) else { return false };
            if let (false, Some(from)) = (read.looked_back, read.assembler.read_again_from()) {
                read.looked_back = true;
                let (keys, url) = (checkpoint::keys_for(&read.state), read.url.clone());
                live.pool.unfollow(stream);
                live.pool.follow_together_tail_at(&url, vec![(keys, from)], None);
                return true;
            }
        }
        self.end_checkpoint_read(stream);
        true
    }

    pub(super) fn on_checkpoint_refused(self: &Rc<Self>, stream: StreamId) -> bool {
        let reading = self.sync.borrow().as_ref().is_some_and(|live| live.checkpoints.reading.contains_key(&stream));
        if reading {
            self.end_checkpoint_read(stream);
        }
        reading
    }

    pub(super) fn end_slow_checkpoint_reads(self: &Rc<Self>) {
        let now = Utc::now().timestamp_millis();
        let slow: Vec<StreamId> = match self.sync.borrow().as_ref() {
            Some(live) => live.checkpoints.reading.iter().filter(|(_, read)| now - read.started_ms >= PATIENCE_MS).map(|(stream, _)| *stream).collect(),
            None => return,
        };
        for stream in slow {
            self.end_checkpoint_read(stream);
        }
    }

    fn end_checkpoint_read(self: &Rc<Self>, stream: StreamId) {
        let Some(read) = self.sync.borrow_mut().as_mut().and_then(|live| live.checkpoints.reading.remove(&stream)) else { return };
        let start = self.adopt_checkpoint(&read);
        {
            let mut guard = self.sync.borrow_mut();
            let Some(live) = guard.as_mut() else { return };
            live.checkpoints.tried.insert(read.state.id);
            live.pool.unfollow(stream);
            if live.book.route(read.state.id).is_none() {
                return;
            }
            match start {
                Some(after) => {
                    live.book.start_at(read.state.id, after);
                    live.pool.follow_together_tail_at(&read.url, vec![(read.state, after)], None);
                }
                None => {
                    let after = live.book.after(read.state.id);
                    live.pool.follow_together_tail_at(&read.url, vec![(read.state, after)], Some(streams::STATE_TAIL));
                }
            }
        }
        if start.is_some() {
            self.adopted(read.server);
        }
    }

    fn adopt_checkpoint(&self, read: &Reading) -> Option<u64> {
        let owner = self.client.borrow().ticket_owner_key(read.server)?;
        let found = read.assembler.newest(read.server, &owner)?;
        let after = found.state_after();
        let joined = self.sync.borrow().as_ref().and_then(|live| live.book.joined_ms(read.server));
        (self.client.borrow().install_service_snapshot(read.server, found.snapshot, joined) == Ok(true)).then_some(after)
    }

    pub(super) fn write_checkpoints(self: &Rc<Self>, server: ServerId) {
        let Some(root) = self.client.borrow().identity_secret() else { return };
        let writes: Vec<(StreamKeys, u64, SyncClient, String)> = {
            let mut guard = self.sync.borrow_mut();
            let Some(live) = guard.as_mut() else { return };
            let url = live.socket_of(server);
            let states: Vec<(StreamKeys, u64)> = live
                .book
                .writers(server, Lane::State)
                .into_iter()
                .filter(|stream| live.book.is_live(*stream))
                .filter_map(|stream| Some((checkpoint::keys_for(&live.book.route(stream)?.1.keys), live.book.after(stream))))
                .collect();
            states
                .into_iter()
                .filter_map(|(keys, covers)| {
                    live.pool.follow_together_tail_at(&url, vec![(keys.clone(), 0)], Some(0));
                    let client = live.pool.client_for(keys.id)?;
                    Some((keys, covers, client, url.clone()))
                })
                .collect()
        };
        let covers: Vec<u64> = writes.iter().map(|(_, covers, _, _)| *covers).collect();
        let records = self.client.borrow().owner_checkpoints(server, &covers);
        for ((keys, _, client, url), records) in writes.into_iter().zip(records) {
            let this = self.clone();
            spawn_local(async move {
                let first = write_records(&client, keys.id, records).await;
                if let Some(live) = this.sync.borrow_mut().as_mut() {
                    live.pool.unfollow(keys.id);
                }
                let Some(first) = first.filter(|first| *first > 1) else { return };
                let work = owner_work(&root, &[keys], |_| Errand::Erase { through: first - 1 });
                let _ = errands::run(&url, work).await;
            });
        }
    }
}

async fn write_records(client: &SyncClient, stream: StreamId, records: Vec<Vec<u8>>) -> Option<u64> {
    if records.is_empty() {
        return None;
    }
    let mut first = None;
    for plain in records {
        let seq = n0_future::time::timeout(WRITE_WAIT, client.append(stream, plain)).await.ok()?.ok()?;
        first.get_or_insert(seq);
    }
    first
}
