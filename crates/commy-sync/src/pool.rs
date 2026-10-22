use std::collections::HashMap;

use tokio::sync::mpsc;

use crate::client::{AppendError, SyncClient, SyncEvent};
use crate::stream::{StreamId, StreamKeys};
use crate::wire::MAX_STREAMS_PER_CONNECTION;

struct Slot {
    client: SyncClient,
    streams: usize,
}

pub struct Pool {
    url: String,
    slots: Vec<Slot>,
    home: HashMap<StreamId, usize>,
    events: mpsc::UnboundedSender<SyncEvent>,
}

impl Pool {
    pub fn new(url: &str) -> (Pool, mpsc::UnboundedReceiver<SyncEvent>) {
        let (events, received) = mpsc::unbounded_channel();
        (Pool::with_events(url, events), received)
    }

    pub fn with_events(url: &str, events: mpsc::UnboundedSender<SyncEvent>) -> Pool {
        Pool { url: url.to_string(), slots: Vec::new(), home: HashMap::new(), events }
    }

    pub fn follows(&self, stream: StreamId) -> bool {
        self.home.contains_key(&stream)
    }

    fn slot_with_room(&mut self, needed: usize) -> usize {
        if let Some(at) = self.slots.iter().position(|s| s.streams + needed <= MAX_STREAMS_PER_CONNECTION) {
            return at;
        }
        let (client, mut received) = SyncClient::spawn(self.url.clone());
        let events = self.events.clone();
        n0_future::task::spawn(async move {
            while let Some(event) = received.recv().await {
                if events.send(event).is_err() {
                    break;
                }
            }
        });
        self.slots.push(Slot { client, streams: 0 });
        self.slots.len() - 1
    }

    pub fn follow_together(&mut self, group: Vec<(StreamKeys, u64)>) {
        self.follow_together_tail(group, None);
    }

    pub fn follow_together_tail(&mut self, group: Vec<(StreamKeys, u64)>, tail: Option<u32>) {
        let fresh: Vec<(StreamKeys, u64)> = group.into_iter().filter(|(keys, _)| !self.home.contains_key(&keys.id)).collect();
        if fresh.is_empty() {
            return;
        }
        let at = self.slot_with_room(fresh.len().min(MAX_STREAMS_PER_CONNECTION));
        for (keys, after) in fresh {
            let at = if self.slots[at].streams < MAX_STREAMS_PER_CONNECTION { at } else { self.slot_with_room(1) };
            self.home.insert(keys.id, at);
            self.slots[at].streams += 1;
            self.slots[at].client.follow_tail(keys, after, tail);
        }
    }

    pub fn follow(&mut self, keys: StreamKeys, after: u64) {
        self.follow_together(vec![(keys, after)]);
    }

    pub fn unfollow(&mut self, stream: StreamId) {
        if let Some(at) = self.home.remove(&stream) {
            self.slots[at].streams -= 1;
            self.slots[at].client.unfollow(stream);
        }
    }

    pub fn nudge(&self) {
        for slot in &self.slots {
            slot.client.nudge();
        }
    }

    pub fn client_for(&self, stream: StreamId) -> Option<SyncClient> {
        self.home.get(&stream).map(|at| self.slots[*at].client.clone())
    }

    pub async fn append(&self, stream: StreamId, plain: Vec<u8>) -> Result<u64, AppendError> {
        self.client_for(stream).ok_or(AppendError::NotFollowed)?.append(stream, plain).await
    }
}
