use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use commy_sync::stream::StreamId;
use commy_sync::wire::{ErrorCode, Head, ServerFrame, WatchRequest, MAX_WATCHED_PER_CONNECTION, MAX_WATCHES_PER_FRAME};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

use crate::hub::Live;
use crate::limits::Caller;
use crate::Service;

pub(crate) const HEADS_GAP: Duration = Duration::from_millis(500);

#[derive(Default)]
pub(crate) struct Watching {
    watchers: HashMap<StreamId, JoinHandle<()>>,
}

pub(crate) fn checked(service: &Service, requests: Vec<WatchRequest>, watching: &Watching) -> (Vec<StreamId>, Vec<ServerFrame>) {
    let mut taking: Vec<StreamId> = Vec::new();
    let mut refusals = Vec::new();
    let mut room = MAX_WATCHED_PER_CONNECTION.saturating_sub(watching.watchers.len());
    for request in requests.into_iter().take(MAX_WATCHES_PER_FRAME) {
        let stream = request.stream;
        if service.desk.is_dropped(&stream) {
            refusals.push(ServerFrame::stream_error(ErrorCode::Removed, stream));
        } else if StreamId::of(&request.write.0, &request.read.0) != stream {
            refusals.push(ServerFrame::stream_error(ErrorCode::BadStream, stream));
        } else if watching.watchers.contains_key(&stream) || taking.contains(&stream) {
            continue;
        } else if room == 0 {
            refusals.push(ServerFrame::stream_error(ErrorCode::TooManyStreams, stream));
        } else {
            room -= 1;
            taking.push(stream);
        }
    }
    (taking, refusals)
}

impl Watching {
    pub(crate) async fn watch(&mut self, service: &Arc<Service>, caller: Caller, requests: Vec<WatchRequest>, out: &mpsc::Sender<ServerFrame>) {
        let (taking, refusals) = checked(service, requests, self);
        for refusal in refusals {
            let _ = out.send(refusal).await;
        }
        if taking.is_empty() {
            return;
        }
        if let Err(refusal) = service.limiter.open(caller) {
            for stream in taking {
                let _ = out.send(crate::session::rate_limited(stream, refusal)).await;
            }
            return;
        }
        let lives: Vec<_> = taking.iter().map(|stream| service.hub.subscribe(*stream)).collect();
        let heads = match service.store.heads(&taking).await {
            Ok(heads) => heads,
            Err(e) => {
                tracing::warn!("sync store heads failed: {e}");
                drop(lives);
                for stream in taking {
                    service.hub.release(&stream);
                    let _ = out.send(ServerFrame::stream_error(ErrorCode::Unavailable, stream)).await;
                }
                return;
            }
        };
        service.stats.watched(taking.len());
        let now: Vec<Head> = taking.iter().map(|stream| Head { stream: *stream, head: heads.get(stream).copied().unwrap_or(0) }).collect();
        for ((stream, live), head) in taking.into_iter().zip(lives).zip(&now) {
            let watcher = tokio::spawn(watch_one(service.clone(), stream, live, head.head, out.clone()));
            self.watchers.insert(stream, watcher);
        }
        let _ = out.send(ServerFrame::Heads { heads: now }).await;
    }

    pub(crate) async fn unwatch(&mut self, service: &Service, streams: &[StreamId]) {
        for stream in streams {
            if let Some(watcher) = self.watchers.remove(stream) {
                watcher.abort();
                let _ = watcher.await;
                service.hub.release(stream);
            }
        }
    }

    pub(crate) async fn stop(&mut self, service: &Service) {
        let every: Vec<StreamId> = self.watchers.keys().copied().collect();
        self.unwatch(service, &every).await;
    }
}

enum Heard {
    Moved(u64),
    Removed,
    Lagged,
    Quiet,
    Closed,
}

fn heard(live: Result<Arc<Live>, broadcast::error::RecvError>) -> Heard {
    match live.as_deref() {
        Ok(Live::Entry(entry)) => Heard::Moved(entry.seq),
        Ok(Live::Removed) => Heard::Removed,
        Ok(Live::Signal { .. }) => Heard::Quiet,
        Err(broadcast::error::RecvError::Lagged(_)) => Heard::Lagged,
        Err(broadcast::error::RecvError::Closed) => Heard::Closed,
    }
}

fn drained(live: &mut broadcast::Receiver<Arc<Live>>, mut head: u64) -> Result<u64, Heard> {
    loop {
        match live.try_recv() {
            Ok(entry) => match heard(Ok(entry)) {
                Heard::Moved(seq) => head = head.max(seq),
                Heard::Removed => return Err(Heard::Removed),
                _ => {}
            },
            Err(broadcast::error::TryRecvError::Empty) => return Ok(head),
            Err(broadcast::error::TryRecvError::Lagged(_)) => return Err(Heard::Lagged),
            Err(broadcast::error::TryRecvError::Closed) => return Err(Heard::Closed),
        }
    }
}

async fn watch_one(service: Arc<Service>, stream: StreamId, mut live: broadcast::Receiver<Arc<Live>>, mut told: u64, out: mpsc::Sender<ServerFrame>) {
    loop {
        let moved = match heard(live.recv().await) {
            Heard::Moved(seq) => {
                tokio::time::sleep(HEADS_GAP).await;
                drained(&mut live, seq)
            }
            other => Err(other),
        };
        let head = match moved {
            Ok(head) => head,
            Err(Heard::Removed) => {
                let _ = out.send(ServerFrame::stream_error(ErrorCode::Removed, stream)).await;
                return;
            }
            Err(Heard::Lagged) => match service.store.heads(&[stream]).await {
                Ok(heads) => heads.get(&stream).copied().unwrap_or(told),
                Err(_) => continue,
            },
            Err(Heard::Closed) => return,
            Err(Heard::Moved(_) | Heard::Quiet) => continue,
        };
        if head <= told {
            continue;
        }
        told = head;
        if out.send(ServerFrame::Heads { heads: vec![Head { stream, head }] }).await.is_err() {
            return;
        }
    }
}
