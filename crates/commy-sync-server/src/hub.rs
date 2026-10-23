use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use commy_sync::stream::StreamId;
use commy_sync::wire::{Blob, WireEntry};
use tokio::sync::broadcast;

const LIVE_BUFFER: usize = 256;

#[derive(Debug)]
pub enum Live {
    Entry(WireEntry),
    Signal { from: u64, body: Blob },
}

#[derive(Default)]
pub struct Hub {
    streams: Mutex<HashMap<StreamId, broadcast::Sender<Arc<Live>>>>,
}

impl Hub {
    pub fn subscribe(&self, stream: StreamId) -> broadcast::Receiver<Arc<Live>> {
        let mut streams = self.streams.lock().unwrap();
        streams.entry(stream).or_insert_with(|| broadcast::channel(LIVE_BUFFER).0).subscribe()
    }

    pub fn publish(&self, stream: &StreamId, entry: WireEntry) {
        self.send(stream, Live::Entry(entry));
    }

    pub fn signal(&self, stream: &StreamId, from: u64, body: Blob) -> usize {
        self.send(stream, Live::Signal { from, body })
    }

    fn send(&self, stream: &StreamId, live: Live) -> usize {
        let streams = self.streams.lock().unwrap();
        streams.get(stream).map_or(0, |sender| sender.send(Arc::new(live)).unwrap_or(0))
    }

    pub fn release(&self, stream: &StreamId) {
        let mut streams = self.streams.lock().unwrap();
        if streams.get(stream).is_some_and(|sender| sender.receiver_count() == 0) {
            streams.remove(stream);
        }
    }

    pub fn live_streams(&self) -> usize {
        self.streams.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_sync::StreamSecret;

    fn entry(seq: u64) -> WireEntry {
        WireEntry { seq, at: 0, body: Blob(vec![]) }
    }

    #[tokio::test]
    async fn subscribers_hear_only_their_stream() {
        let hub = Hub::default();
        let a = StreamSecret::random().keys().id;
        let b = StreamSecret::random().keys().id;
        let mut on_a = hub.subscribe(a);
        let mut on_b = hub.subscribe(b);
        hub.publish(&a, entry(1));
        assert!(matches!(&*on_a.recv().await.unwrap(), Live::Entry(e) if e.seq == 1));
        assert_eq!(hub.signal(&a, 7, Blob(vec![1])), 1);
        assert!(matches!(&*on_a.recv().await.unwrap(), Live::Signal { from: 7, .. }));
        assert!(on_b.try_recv().is_err());
        hub.publish(&StreamSecret::random().keys().id, entry(9));
        assert!(on_b.try_recv().is_err());
    }

    #[tokio::test]
    async fn streams_nobody_follows_are_released() {
        let hub = Hub::default();
        let a = StreamSecret::random().keys().id;
        let listener = hub.subscribe(a);
        hub.release(&a);
        assert_eq!(hub.live_streams(), 1, "still followed");
        drop(listener);
        hub.release(&a);
        assert_eq!(hub.live_streams(), 0);
    }
}
