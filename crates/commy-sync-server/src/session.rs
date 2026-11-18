use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use commy_sync::stream::{
    verify_append, verify_claim, verify_erase, verify_release, verify_signal, StreamId, MAX_SEALED, MAX_SIGNAL_SEALED, NONCE_BYTES, SIZE_BUCKETS,
    TAG_BYTES,
};
use commy_sync::wire::{
    AppendNonce, Blob, ClientFrame, ErrorCode, OpenRequest, ServerFrame, WireEntry, ERASE_WINDOW_MS, FRAME_BODY_BUDGET, MAX_STREAMS_PER_CONNECTION, PAGE_SIZE,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::hub::Live;
use crate::limits::{Caller, Refusal};
use crate::Service;

const IDLE_LIMIT: Duration = Duration::from_secs(180);
const OUTGOING_BUFFER: usize = 64;

struct OpenStream {
    write: [u8; 32],
    follower: JoinHandle<()>,
    will: Option<Blob>,
    unwritten: bool,
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

pub(crate) async fn run(service: Arc<Service>, caller: Caller, socket: WebSocket) {
    service.stats.connected();
    let (mut sink, mut source) = socket.split();
    let (out, mut outgoing) = mpsc::channel::<ServerFrame>(OUTGOING_BUFFER);
    let mut open: HashMap<StreamId, OpenStream> = HashMap::new();
    let mut watching = crate::watch::Watching::default();
    let session: u64 = rand::random();
    let _ = out.send(service.greeting.frame()).await;
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
                    Ok(frame) => handle(&service, caller, session, frame, &mut open, &mut watching, &out).await,
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
    for (stream, opened) in &open {
        if let Some(body) = opened.will.clone() {
            let heard = service.hub.signal(stream, session, body);
            service.stats.signalled(heard.saturating_sub(1));
        }
    }
    for (stream, opened) in open.drain() {
        opened.follower.abort();
        let _ = opened.follower.await;
        service.hub.release(&stream);
    }
    watching.stop(&service).await;
    service.limiter.disconnect(caller);
    service.stats.disconnected();
}

async fn handle(
    service: &Arc<Service>,
    caller: Caller,
    session: u64,
    frame: ClientFrame,
    open: &mut HashMap<StreamId, OpenStream>,
    watching: &mut crate::watch::Watching,
    out: &mpsc::Sender<ServerFrame>,
) {
    if let Some(stream) = stream_of(&frame).filter(|stream| service.desk.is_dropped(stream)) {
        let _ = out.send(ServerFrame::stream_error(ErrorCode::Removed, stream)).await;
        return;
    }
    match frame {
        ClientFrame::Ping => {
            let _ = out.send(ServerFrame::Pong).await;
        }
        ClientFrame::Report { report } => {
            let _ = out.send(crate::reports::take(service, caller.address, *report).await).await;
        }
        ClientFrame::Open { stream, write, read, after, tail } => {
            open_streams(service, caller, session, vec![OpenRequest { stream, write, read, after, tail }], open, out).await;
        }
        ClientFrame::OpenMany { opens } => {
            open_streams(service, caller, session, opens, open, out).await;
        }
        ClientFrame::Watch { streams } => {
            watching.watch(service, caller, streams, out).await;
        }
        ClientFrame::Unwatch { streams } => {
            watching.unwatch(service, &streams).await;
        }
        ClientFrame::Claim { stream, write, read, eraser, sig } => {
            let _ = out.send(claim(service, caller, stream, &write.0, &read.0, &eraser.0, &sig.0).await).await;
        }
        ClientFrame::Erase { stream, through, at, sig } => {
            let _ = out.send(erase(service, caller, stream, through, at, &sig.0).await).await;
        }
        ClientFrame::Release { stream, at, sig } => {
            let _ = out.send(release(service, caller, stream, at, &sig.0).await).await;
        }
        ClientFrame::Close { stream } => {
            if let Some(opened) = open.remove(&stream) {
                opened.follower.abort();
                let _ = opened.follower.await;
                service.hub.release(&stream);
            }
        }
        ClientFrame::Signal { stream, nonce, body, sig } => {
            let Some(opened) = open.get(&stream) else {
                let _ = out.send(refused(ErrorCode::NotOpen, stream, nonce, None)).await;
                return;
            };
            if let Err(code) = check_signal(service, caller, &opened.write, stream, &nonce, &body, &sig) {
                if let Some(code) = code {
                    let _ = out.send(refused(code, stream, nonce, None)).await;
                }
                return;
            }
            let heard = service.hub.signal(&stream, session, body);
            service.stats.signalled(heard.saturating_sub(1));
        }
        ClientFrame::Will { stream, nonce, body, sig } => {
            let Some(opened) = open.get_mut(&stream) else {
                let _ = out.send(refused(ErrorCode::NotOpen, stream, nonce, None)).await;
                return;
            };
            if let Err(code) = check_will(service, caller, &opened.write, stream, &nonce, &body, &sig) {
                if let Some(code) = code {
                    let _ = out.send(refused(code, stream, nonce, None)).await;
                }
                return;
            }
            opened.will = Some(body);
        }
        ClientFrame::Append { stream, nonce, body, sig } => {
            let Some(opened) = open.get_mut(&stream) else {
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
            if opened.unwritten {
                if let Err(refusal) = service.limiter.new_stream(caller.address) {
                    let _ = out.send(refused(ErrorCode::RateLimited, stream, nonce, Some(refusal))).await;
                    return;
                }
                opened.unwritten = false;
            }
            if let Err(refusal) = service.limiter.append_from(caller, &stream.0, body.0.len()) {
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

pub(crate) fn rate_limited(stream: StreamId, refusal: Refusal) -> ServerFrame {
    let retry_after_ms = match refusal {
        Refusal::Wait(wait) => Some(wait.as_millis().max(100) as u64),
        Refusal::Full => None,
    };
    ServerFrame::Error { code: ErrorCode::RateLimited, stream: Some(stream), nonce: None, retry_after_ms }
}

fn checked_opens(service: &Service, requests: Vec<OpenRequest>) -> (Vec<OpenRequest>, Vec<ServerFrame>) {
    let mut taking: Vec<OpenRequest> = Vec::new();
    let mut refusals = Vec::new();
    for request in requests {
        if service.desk.is_dropped(&request.stream) {
            refusals.push(ServerFrame::stream_error(ErrorCode::Removed, request.stream));
        } else if StreamId::of(&request.write.0, &request.read.0) != request.stream {
            refusals.push(ServerFrame::stream_error(ErrorCode::BadStream, request.stream));
        } else if !taking.iter().any(|taken| taken.stream == request.stream) {
            taking.push(request);
        }
    }
    (taking, refusals)
}

fn fit_to_room(taking: &mut Vec<OpenRequest>, open: &HashMap<StreamId, OpenStream>) -> Vec<ServerFrame> {
    let mut room = MAX_STREAMS_PER_CONNECTION.saturating_sub(open.len());
    let mut refusals = Vec::new();
    taking.retain(|request| {
        if open.contains_key(&request.stream) {
            return true;
        }
        if room == 0 {
            refusals.push(ServerFrame::stream_error(ErrorCode::TooManyStreams, request.stream));
            return false;
        }
        room -= 1;
        true
    });
    refusals
}

async fn open_streams(
    service: &Arc<Service>,
    caller: Caller,
    session: u64,
    requests: Vec<OpenRequest>,
    open: &mut HashMap<StreamId, OpenStream>,
    out: &mpsc::Sender<ServerFrame>,
) {
    let (mut taking, mut refusals) = checked_opens(service, requests);
    refusals.extend(fit_to_room(&mut taking, open));
    for refusal in refusals {
        let _ = out.send(refusal).await;
    }
    if taking.is_empty() {
        return;
    }
    if let Err(refusal) = service.limiter.open(caller) {
        for request in &taking {
            let _ = out.send(rate_limited(request.stream, refusal)).await;
        }
        return;
    }
    let mut wills = HashMap::new();
    for request in &taking {
        if let Some(previous) = open.remove(&request.stream) {
            previous.follower.abort();
            let _ = previous.follower.await;
            wills.insert(request.stream, previous.will.filter(|_| previous.write == request.write.0));
        }
    }
    let lives: Vec<_> = taking.iter().map(|request| service.hub.subscribe(request.stream)).collect();
    let streams: Vec<StreamId> = taking.iter().map(|request| request.stream).collect();
    let heads = match service.store.heads(&streams).await {
        Ok(heads) => heads,
        Err(e) => {
            tracing::warn!("sync store heads failed: {e}");
            drop(lives);
            for stream in streams {
                service.hub.release(&stream);
                let _ = out.send(ServerFrame::stream_error(ErrorCode::Unavailable, stream)).await;
            }
            return;
        }
    };
    for (request, live) in taking.into_iter().zip(lives) {
        service.stats.opened();
        let head = heads.get(&request.stream).copied().unwrap_or(0);
        let start = start_after(request.after, request.tail, head);
        let follower = tokio::spawn(follow(service.clone(), session, request.stream, live, head, start, out.clone()));
        let will = wills.remove(&request.stream).flatten();
        open.insert(request.stream, OpenStream { write: request.write.0, follower, will, unwritten: head == 0 });
    }
}

fn stream_of(frame: &ClientFrame) -> Option<StreamId> {
    match frame {
        ClientFrame::Open { stream, .. }
        | ClientFrame::Append { stream, .. }
        | ClientFrame::Signal { stream, .. }
        | ClientFrame::Will { stream, .. }
        | ClientFrame::Claim { stream, .. } => Some(*stream),
        ClientFrame::OpenMany { .. }
        | ClientFrame::Watch { .. }
        | ClientFrame::Unwatch { .. }
        | ClientFrame::Close { .. } | ClientFrame::Erase { .. } | ClientFrame::Release { .. } | ClientFrame::Report { .. } | ClientFrame::Ping => None,
    }
}

fn owner_limited(service: &Service, caller: Caller, stream: StreamId) -> Result<(), ServerFrame> {
    service.limiter.open(caller).map_err(|refusal| rate_limited(stream, refusal))
}

fn unavailable(stream: StreamId, e: crate::store::StoreError) -> ServerFrame {
    tracing::warn!("sync store eraser work failed: {e}");
    ServerFrame::Error { code: ErrorCode::Unavailable, stream: Some(stream), nonce: None, retry_after_ms: Some(2_000) }
}

pub(crate) fn is_fresh(at: i64, now: i64) -> bool {
    at.abs_diff(now) <= ERASE_WINDOW_MS as u64
}

async fn claim(service: &Service, caller: Caller, stream: StreamId, write: &[u8; 32], read: &[u8; 32], eraser: &[u8; 32], sig: &[u8; 64]) -> ServerFrame {
    if StreamId::of(write, read) != stream {
        return ServerFrame::stream_error(ErrorCode::BadStream, stream);
    }
    if !verify_claim(eraser, &stream, sig) {
        return ServerFrame::stream_error(ErrorCode::BadSignature, stream);
    }
    if let Err(refused) = owner_limited(service, caller, stream) {
        return refused;
    }
    match service.store.claim(&stream, eraser, now_ms()).await {
        Ok(mine) => ServerFrame::Claimed { stream, mine },
        Err(e) => unavailable(stream, e),
    }
}

async fn holder(service: &Service, stream: StreamId, at: i64) -> Result<Option<[u8; 32]>, ServerFrame> {
    if !is_fresh(at, now_ms()) {
        return Err(ServerFrame::stream_error(ErrorCode::Expired, stream));
    }
    service.store.eraser(&stream).await.map_err(|e| unavailable(stream, e))
}

async fn erase(service: &Service, caller: Caller, stream: StreamId, through: u64, at: i64, sig: &[u8; 64]) -> ServerFrame {
    if let Err(refused) = owner_limited(service, caller, stream) {
        return refused;
    }
    match holder(service, stream, at).await {
        Ok(Some(eraser)) if verify_erase(&eraser, &stream, through, at, sig) => {}
        Ok(_) => return ServerFrame::stream_error(ErrorCode::NotYours, stream),
        Err(refused) => return refused,
    }
    match service.store.erase(&stream, through).await {
        Ok(removed) => {
            tracing::info!("sync: erased {removed} entries of a stream at its owner's request");
            ServerFrame::Erased { stream, through }
        }
        Err(e) => unavailable(stream, e),
    }
}

async fn release(service: &Service, caller: Caller, stream: StreamId, at: i64, sig: &[u8; 64]) -> ServerFrame {
    if let Err(refused) = owner_limited(service, caller, stream) {
        return refused;
    }
    let eraser = match holder(service, stream, at).await {
        Ok(None) => return ServerFrame::Released { stream },
        Ok(Some(eraser)) if verify_release(&eraser, &stream, at, sig) => eraser,
        Ok(Some(_)) => return ServerFrame::stream_error(ErrorCode::NotYours, stream),
        Err(refused) => return refused,
    };
    match service.store.release(&stream, &eraser).await {
        Ok(()) => ServerFrame::Released { stream },
        Err(e) => unavailable(stream, e),
    }
}

fn verify_signed_word(write: &[u8; 32], stream: StreamId, nonce: &AppendNonce, body: &Blob, sig: &commy_sync::wire::Bytes<64>) -> Result<(), ErrorCode> {
    if !is_padded_size(body.0.len()) || body.0.len() > MAX_SIGNAL_SEALED {
        return Err(ErrorCode::TooLarge);
    }
    if !verify_signal(write, &stream, &nonce.0, &body.0, &sig.0) {
        return Err(ErrorCode::BadSignature);
    }
    Ok(())
}

fn check_signal(
    service: &Service,
    caller: Caller,
    write: &[u8; 32],
    stream: StreamId,
    nonce: &AppendNonce,
    body: &Blob,
    sig: &commy_sync::wire::Bytes<64>,
) -> Result<(), Option<ErrorCode>> {
    verify_signed_word(write, stream, nonce, body, sig).map_err(Some)?;
    service.limiter.signal(caller, body.0.len()).map_err(|_| None)
}

fn check_will(
    service: &Service,
    caller: Caller,
    write: &[u8; 32],
    stream: StreamId,
    nonce: &AppendNonce,
    body: &Blob,
    sig: &commy_sync::wire::Bytes<64>,
) -> Result<(), Option<ErrorCode>> {
    verify_signed_word(write, stream, nonce, body, sig).map_err(Some)?;
    service.limiter.will(caller).map_err(|_| None)
}

pub(crate) fn start_after(after: u64, tail: Option<u32>, head: u64) -> u64 {
    match tail {
        Some(tail) if after == 0 => head.saturating_sub(u64::from(tail)),
        _ if after > head => 0,
        _ => after,
    }
}

async fn follow(
    service: Arc<Service>,
    session: u64,
    stream: StreamId,
    mut live: broadcast::Receiver<Arc<Live>>,
    head: u64,
    mut sent: u64,
    out: mpsc::Sender<ServerFrame>,
) {
    if out.send(ServerFrame::Opened { stream, head }).await.is_err() {
        return;
    }
    let Some(caught) = catch_up(&service, stream, sent, &out).await else { return };
    sent = caught;
    loop {
        let heard = live.recv().await;
        let entry = match heard.as_deref() {
            Ok(Live::Signal { from, body }) => {
                if *from != session && out.try_send(ServerFrame::Signal { stream, body: body.clone() }).is_err() {
                    tracing::debug!("sync: dropped a live signal for a busy connection");
                }
                continue;
            }
            Ok(Live::Removed) => {
                let _ = out.send(ServerFrame::stream_error(ErrorCode::Removed, stream)).await;
                return;
            }
            Ok(Live::Entry(entry)) => Ok(entry),
            Err(e) => Err(e.clone()),
        };
        match entry {
            Ok(entry) if entry.seq <= sent => {}
            Ok(entry) if entry.seq == sent + 1 => {
                sent = entry.seq;
                let frame = ServerFrame::Entries { stream, entries: vec![entry.clone()], caught_up: true };
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

    #[test]
    fn a_tail_starts_a_fresh_reader_near_the_head() {
        assert_eq!(start_after(0, None, 500), 0);
        assert_eq!(start_after(0, Some(200), 500), 300);
        assert_eq!(start_after(0, Some(200), 50), 0);
        assert_eq!(start_after(420, Some(200), 500), 420, "a cursor wins over a tail");
        assert_eq!(start_after(900, None, 500), 0, "a stream that went back is read again");
    }

    #[test]
    fn owner_requests_are_only_taken_while_fresh() {
        let now = 1_760_000_000_000;
        assert!(is_fresh(now, now));
        assert!(is_fresh(now - ERASE_WINDOW_MS, now));
        assert!(is_fresh(now + ERASE_WINDOW_MS, now), "a phone a little ahead still works");
        assert!(!is_fresh(now - ERASE_WINDOW_MS - 1, now), "an old request cannot be replayed");
        assert!(!is_fresh(i64::MIN, now));
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
