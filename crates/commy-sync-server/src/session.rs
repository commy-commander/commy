use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use commy_sync::stream::{verify_append, StreamId, MAX_SEALED, NONCE_BYTES, SIZE_BUCKETS, TAG_BYTES};
use commy_sync::wire::{
    AppendNonce, ClientFrame, ErrorCode, ServerFrame, WireEntry, FRAME_BODY_BUDGET, MAX_STREAMS_PER_CONNECTION, PAGE_SIZE, PROTOCOL,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::limits::{AddressKey, Refusal};
use crate::Service;

const IDLE_LIMIT: Duration = Duration::from_secs(180);
const OUTGOING_BUFFER: usize = 64;

struct OpenStream {
    write: [u8; 32],
    follower: JoinHandle<()>,
}

pub(crate) fn is_padded_size(len: usize) -> bool {
    len <= MAX_SEALED && SIZE_BUCKETS.iter().any(|bucket| bucket + NONCE_BYTES + TAG_BYTES == len)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn refused(code: ErrorCode, stream: StreamId, nonce: AppendNonce, refusal: Option<Refusal>) -> ServerFrame {
    let retry_after_ms = match refusal {
        Some(Refusal::Wait(wait)) => Some(wait.as_millis().max(100) as u64),
        _ => None,
    };
    ServerFrame::Error { code, stream: Some(stream), nonce: Some(nonce), retry_after_ms }
}

pub(crate) async fn run(service: Arc<Service>, key: AddressKey, socket: WebSocket) {
    service.stats.connected();
    let (mut sink, mut source) = socket.split();
    let (out, mut outgoing) = mpsc::channel::<ServerFrame>(OUTGOING_BUFFER);
    let mut open: HashMap<StreamId, OpenStream> = HashMap::new();
    let _ = out.send(ServerFrame::Hello { protocol: PROTOCOL }).await;
    let mut deadline = Instant::now() + IDLE_LIMIT;
    loop {
        tokio::select! {
            message = source.next() => {
                deadline = Instant::now() + IDLE_LIMIT;
                let text = match message {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    Some(Ok(Message::Binary(_))) => {
                        let _ = out.send(ServerFrame::error(ErrorCode::BadFrame)).await;
                        continue;
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                };
                match serde_json::from_str::<ClientFrame>(&text) {
                    Ok(frame) => handle(&service, key, frame, &mut open, &out).await,
                    Err(_) => {
                        let _ = out.send(ServerFrame::error(ErrorCode::BadFrame)).await;
                    }
                }
            }
            frame = outgoing.recv() => {
                let Some(frame) = frame else { break };
                let Ok(text) = serde_json::to_string(&frame) else { continue };
                if sink.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            _ = tokio::time::sleep_until(deadline) => break,
        }
    }
    for (stream, opened) in open.drain() {
        opened.follower.abort();
        let _ = opened.follower.await;
        service.hub.release(&stream);
    }
    service.limiter.disconnect(key);
    service.stats.disconnected();
}

async fn handle(service: &Arc<Service>, key: AddressKey, frame: ClientFrame, open: &mut HashMap<StreamId, OpenStream>, out: &mpsc::Sender<ServerFrame>) {
    match frame {
        ClientFrame::Ping => {
            let _ = out.send(ServerFrame::Pong).await;
        }
        ClientFrame::Open { stream, write, read, after, tail } => {
            if StreamId::of(&write.0, &read.0) != stream {
                let _ = out.send(ServerFrame::stream_error(ErrorCode::BadStream, stream)).await;
                return;
            }
            if let Some(previous) = open.remove(&stream) {
                previous.follower.abort();
                let _ = previous.follower.await;
            } else if open.len() >= MAX_STREAMS_PER_CONNECTION {
                let _ = out.send(ServerFrame::stream_error(ErrorCode::TooManyStreams, stream)).await;
                return;
            }
            if let Err(refusal) = service.limiter.open(key) {
                let retry_after_ms = match refusal {
                    Refusal::Wait(wait) => Some(wait.as_millis().max(100) as u64),
                    Refusal::Full => None,
                };
                let _ = out.send(ServerFrame::Error { code: ErrorCode::RateLimited, stream: Some(stream), nonce: None, retry_after_ms }).await;
                return;
            }
            service.stats.opened();
            let follower = tokio::spawn(follow(service.clone(), stream, after, tail, out.clone()));
            open.insert(stream, OpenStream { write: write.0, follower });
        }
        ClientFrame::Close { stream } => {
            if let Some(opened) = open.remove(&stream) {
                opened.follower.abort();
                let _ = opened.follower.await;
                service.hub.release(&stream);
            }
        }
        ClientFrame::Append { stream, nonce, body, sig } => {
            let Some(opened) = open.get(&stream) else {
                let _ = out.send(refused(ErrorCode::NotOpen, stream, nonce, None)).await;
                return;
            };
            if !is_padded_size(body.0.len()) {
                let _ = out.send(refused(ErrorCode::TooLarge, stream, nonce, None)).await;
                return;
            }
            if !verify_append(&opened.write, &stream, &nonce.0, &body.0, &sig.0) {
                let _ = out.send(refused(ErrorCode::BadSignature, stream, nonce, None)).await;
                return;
            }
            if let Err(refusal) = service.limiter.append(key, &stream.0, body.0.len()) {
                let _ = out.send(refused(ErrorCode::RateLimited, stream, nonce, Some(refusal))).await;
                return;
            }
            match service.store.append(&stream, &nonce.0, &body.0, now_ms()).await {
                Ok(done) => {
                    if done.fresh {
                        service.stats.appended();
                        service.hub.publish(&stream, WireEntry { seq: done.seq, at: done.at, body });
                    }
                    let _ = out.send(ServerFrame::Appended { stream, nonce, seq: done.seq, at: done.at }).await;
                }
                Err(e) => {
                    tracing::warn!("sync store append failed: {e}");
                    let _ = out.send(refused(ErrorCode::Unavailable, stream, nonce, Some(Refusal::Wait(Duration::from_secs(2))))).await;
                }
            }
        }
    }
}

pub(crate) fn start_after(after: u64, tail: Option<u32>, head: u64) -> u64 {
    match tail {
        Some(tail) if after == 0 => head.saturating_sub(u64::from(tail)),
        _ if after > head => 0,
        _ => after,
    }
}

async fn follow(service: Arc<Service>, stream: StreamId, after: u64, tail: Option<u32>, out: mpsc::Sender<ServerFrame>) {
    let mut live = service.hub.subscribe(stream);
    let head = match service.store.head(&stream).await {
        Ok(head) => head,
        Err(e) => {
            tracing::warn!("sync store head failed: {e}");
            let _ = out.send(ServerFrame::stream_error(ErrorCode::Unavailable, stream)).await;
            return;
        }
    };
    let mut sent = start_after(after, tail, head);
    if out.send(ServerFrame::Opened { stream, head }).await.is_err() {
        return;
    }
    let Some(caught) = catch_up(&service, stream, sent, &out).await else { return };
    sent = caught;
    loop {
        match live.recv().await {
            Ok(entry) if entry.seq <= sent => {}
            Ok(entry) if entry.seq == sent + 1 => {
                sent = entry.seq;
                let frame = ServerFrame::Entries { stream, entries: vec![(*entry).clone()], caught_up: true };
                if out.send(frame).await.is_err() {
                    return;
                }
            }
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {
                let Some(caught) = catch_up(&service, stream, sent, &out).await else { return };
                sent = caught;
            }
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

async fn catch_up(service: &Service, stream: StreamId, mut sent: u64, out: &mpsc::Sender<ServerFrame>) -> Option<u64> {
    loop {
        let page = match service.store.page(&stream, sent, PAGE_SIZE).await {
            Ok(page) => page,
            Err(e) => {
                tracing::warn!("sync store page failed: {e}");
                let _ = out.send(ServerFrame::stream_error(ErrorCode::Unavailable, stream)).await;
                return None;
            }
        };
        let last_page = page.len() < PAGE_SIZE;
        let frames = split_by_size(page);
        let count = frames.len();
        for (i, entries) in frames.into_iter().enumerate() {
            if let Some(last) = entries.last() {
                sent = last.seq;
            }
            let caught_up = last_page && i + 1 == count;
            out.send(ServerFrame::Entries { stream, entries, caught_up }).await.ok()?;
        }
        if last_page {
            return Some(sent);
        }
    }
}

fn split_by_size(page: Vec<WireEntry>) -> Vec<Vec<WireEntry>> {
    let mut frames = vec![Vec::new()];
    let mut bytes = 0usize;
    for entry in page {
        let current = frames.last_mut().expect("never empty");
        if !current.is_empty() && bytes + entry.body.0.len() > FRAME_BODY_BUDGET {
            frames.push(Vec::new());
            bytes = 0;
        }
        bytes += entry.body.0.len();
        frames.last_mut().expect("never empty").push(entry);
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_sync::wire::Blob;

    #[test]
    fn a_tail_starts_a_fresh_reader_near_the_head() {
        assert_eq!(start_after(0, None, 500), 0);
        assert_eq!(start_after(0, Some(200), 500), 300);
        assert_eq!(start_after(0, Some(200), 50), 0);
        assert_eq!(start_after(420, Some(200), 500), 420, "a cursor wins over a tail");
        assert_eq!(start_after(900, None, 500), 0, "a stream that went back is read again");
    }

    #[test]
    fn only_padded_bodies_are_accepted() {
        assert!(is_padded_size(512 + 28));
        assert!(is_padded_size(64 * 1024 + 28));
        assert!(!is_padded_size(513 + 28));
        assert!(!is_padded_size(0));
        assert!(!is_padded_size(MAX_SEALED + 1));
    }

    #[test]
    fn big_pages_are_split_into_frames_under_the_budget() {
        let entry = |seq| WireEntry { seq, at: 0, body: Blob(vec![0; 64 * 1024 + 28]) };
        let frames = split_by_size((1..=10).map(entry).collect());
        assert!(frames.len() >= 3);
        assert!(frames.iter().all(|f| !f.is_empty()));
        assert!(frames.iter().all(|f| f.iter().map(|e| e.body.0.len()).sum::<usize>() <= FRAME_BODY_BUDGET));
        let seqs: Vec<u64> = frames.iter().flatten().map(|e| e.seq).collect();
        assert_eq!(seqs, (1..=10).collect::<Vec<_>>());
        assert_eq!(split_by_size(Vec::new()), vec![Vec::<WireEntry>::new()]);
    }
}
