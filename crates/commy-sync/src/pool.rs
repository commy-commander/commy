use std::collections::HashMap;

use tokio::sync::mpsc;

use crate::client::{AppendError, SyncClient, SyncEvent};
use crate::stream::{StreamId, StreamKeys};
use crate::wire::MAX_STREAMS_PER_CONNECTION;

struct Slot {
    url: String,
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

    fn slot_with_room(&mut self, url: &str, needed: usize) -> usize {
        if let Some(at) = room_at(self.slots.iter().map(|s| (s.url.as_str(), s.streams)), url, needed) {
            return at;
        }
        let (client, mut received) = SyncClient::spawn(url.to_string());
        let events = self.events.clone();
        n0_future::task::spawn(async move {
            while let Some(event) = received.recv().await {
                if events.send(event).is_err() {
                    break;
                }
            }
        });
        self.slots.push(Slot { url: url.to_string(), client, streams: 0 });
        self.slots.len() - 1
    }

    pub fn follow_together(&mut self, group: Vec<(StreamKeys, u64)>) {
        self.follow_together_tail(group, None);
    }

    pub fn follow_together_tail(&mut self, group: Vec<(StreamKeys, u64)>, tail: Option<u32>) {
        let url = self.url.clone();
        self.follow_together_tail_at(&url, group, tail);
    }

    pub fn follow_together_tail_at(&mut self, url: &str, group: Vec<(StreamKeys, u64)>, tail: Option<u32>) {
        let fresh: Vec<(StreamKeys, u64)> = group.into_iter().filter(|(keys, _)| !self.home.contains_key(&keys.id)).collect();
        if fresh.is_empty() {
            return;
        }
        let at = self.slot_with_room(url, fresh.len().min(MAX_STREAMS_PER_CONNECTION));
        for (keys, after) in fresh {
            let at = if self.slots[at].streams < MAX_STREAMS_PER_CONNECTION { at } else { self.slot_with_room(url, 1) };
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

    pub fn host_of(&self, stream: StreamId) -> Option<&str> {
        self.home.get(&stream).map(|at| self.slots[*at].url.as_str())
    }

    pub fn client_for(&self, stream: StreamId) -> Option<SyncClient> {
        self.home.get(&stream).map(|at| self.slots[*at].client.clone())
    }

    pub async fn append(&self, stream: StreamId, plain: Vec<u8>) -> Result<u64, AppendError> {
        self.client_for(stream).ok_or(AppendError::NotFollowed)?.append(stream, plain).await
    }
}

fn room_at<'a>(slots: impl Iterator<Item = (&'a str, usize)>, url: &str, needed: usize) -> Option<usize> {
    slots.enumerate().find(|(_, (at, streams))| *at == url && streams + needed <= MAX_STREAMS_PER_CONNECTION).map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_share_a_connection_only_with_streams_on_the_same_host() {
        let slots = [("wss://commy.chat/sync", 3), ("wss://sync.example.org/sync", 1), ("wss://commy.chat/sync", 0)];
        assert_eq!(room_at(slots.into_iter(), "wss://commy.chat/sync", 2), Some(0));
        assert_eq!(room_at(slots.into_iter(), "wss://sync.example.org/sync", 2), Some(1));
        assert_eq!(room_at(slots.into_iter(), "wss://other.example.org/sync", 1), None, "a new host gets its own connection");
        let full = [("wss://commy.chat/sync", MAX_STREAMS_PER_CONNECTION), ("wss://commy.chat/sync", MAX_STREAMS_PER_CONNECTION - 1)];
        assert_eq!(room_at(full.into_iter(), "wss://commy.chat/sync", 1), Some(1));
        assert_eq!(room_at(full.into_iter(), "wss://commy.chat/sync", 2), None);
    }
}
