use std::collections::HashMap;

use n0_future::time::{self, Duration, Instant};
use tokio::sync::{mpsc, oneshot};

use crate::stream::{SealError, StreamId, StreamKeys, APPEND_NONCE_BYTES};
use crate::wire::{ClientFrame, ErrorCode, Greeting, OpenRequest, ServerFrame, KEEPALIVE_SECS, MAX_STREAMS_PER_CONNECTION, OPEN_MANY_SINCE, SIGNALS_SINCE, WILLS_SINCE};

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
#[path = "client/native.rs"]
pub(crate) mod socket;
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
#[path = "client/browser.rs"]
pub(crate) mod socket;

use socket::{Incoming, Socket};

const RECONNECT_STEPS_MS: [u64; 6] = [500, 2_000, 5_000, 10_000, 30_000, 60_000];
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const SILENCE_LIMIT: Duration = Duration::from_secs(KEEPALIVE_SECS * 2 + 10);
const RETRY_AFTER_DEFAULT_MS: u64 = 2_000;
const HELLO_WAIT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncEvent {
    Connected,
    Disconnected { streams: Vec<StreamId> },
    Entry { stream: StreamId, seq: u64, at: i64, plain: Vec<u8> },
    Unreadable { stream: StreamId, seq: u64 },
    CaughtUp { stream: StreamId, seq: u64 },
    Refused { stream: StreamId, code: ErrorCode },
    Signal { stream: StreamId, plain: Vec<u8> },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AppendError {
    #[error(transparent)]
    Seal(#[from] SealError),
    #[error("the stream is not followed")]
    NotFollowed,
    #[error("the sync service refused the entry: {0:?}")]
    Refused(ErrorCode),
    #[error("the sync client stopped")]
    Stopped,
}

type Reply = oneshot::Sender<Result<u64, AppendError>>;

enum Command {
    Follow { keys: Box<StreamKeys>, after: u64, tail: Option<u32> },
    Unfollow { stream: StreamId },
    Append { stream: StreamId, plain: Vec<u8>, reply: Reply },
    Signal { stream: StreamId, plain: Vec<u8> },
    Will { stream: StreamId, plain: Vec<u8> },
    Nudge,
}

type Heard = std::sync::Arc<std::sync::Mutex<Option<Greeting>>>;

#[derive(Clone)]
pub struct SyncClient {
    commands: mpsc::UnboundedSender<Command>,
    heard: Heard,
}

impl SyncClient {
    pub fn spawn(url: impl Into<String>) -> (SyncClient, mpsc::UnboundedReceiver<SyncEvent>) {
        let (commands, inbox) = mpsc::unbounded_channel();
        let (events, received) = mpsc::unbounded_channel();
        let heard = Heard::default();
        n0_future::task::spawn(Link::new(url.into(), events, heard.clone()).run(inbox));
        (SyncClient { commands, heard }, received)
    }

    pub fn greeting(&self) -> Option<Greeting> {
        self.heard.lock().unwrap().clone()
    }

    pub fn follow(&self, keys: StreamKeys, after: u64) {
        self.follow_tail(keys, after, None);
    }

    pub fn follow_tail(&self, keys: StreamKeys, after: u64, tail: Option<u32>) {
        let _ = self.commands.send(Command::Follow { keys: Box::new(keys), after, tail });
    }

    pub fn unfollow(&self, stream: StreamId) {
        let _ = self.commands.send(Command::Unfollow { stream });
    }

    pub async fn append(&self, stream: StreamId, plain: Vec<u8>) -> Result<u64, AppendError> {
        let (reply, answer) = oneshot::channel();
        self.commands.send(Command::Append { stream, plain, reply }).map_err(|_| AppendError::Stopped)?;
        answer.await.map_err(|_| AppendError::Stopped)?
    }

    pub fn signal(&self, stream: StreamId, plain: Vec<u8>) {
        let _ = self.commands.send(Command::Signal { stream, plain });
    }

    pub fn will(&self, stream: StreamId, plain: Vec<u8>) {
        let _ = self.commands.send(Command::Will { stream, plain });
    }

    pub fn nudge(&self) {
        let _ = self.commands.send(Command::Nudge);
    }

    pub fn is_running(&self) -> bool {
        !self.commands.is_closed()
    }
}

struct Followed {
    keys: StreamKeys,
    after: u64,
    tail: Option<u32>,
    will: Option<Vec<u8>>,
}

impl Followed {
    fn open(&self) -> ClientFrame {
        ClientFrame::open_tail(&self.keys, self.after, self.tail)
    }

    fn open_request(&self) -> OpenRequest {
        OpenRequest::of(&self.keys, self.after, self.tail)
    }

    fn last_word(&self) -> Option<ClientFrame> {
        let body = self.keys.seal(self.will.as_ref()?).ok()?;
        Some(ClientFrame::will(&self.keys, rand::random(), body))
    }
}

struct Pending {
    frame: ClientFrame,
    reply: Reply,
    not_before: Instant,
}

struct Link {
    url: String,
    events: mpsc::UnboundedSender<SyncEvent>,
    streams: HashMap<StreamId, Followed>,
    pending: HashMap<[u8; APPEND_NONCE_BYTES], Pending>,
    reopen: HashMap<StreamId, Instant>,
    protocol: u32,
    wills_due: bool,
    heard: Heard,
}

enum Ended {
    Lost,
    Stopped,
}

impl Link {
    fn new(url: String, events: mpsc::UnboundedSender<SyncEvent>, heard: Heard) -> Self {
        Self { url, events, streams: HashMap::new(), pending: HashMap::new(), reopen: HashMap::new(), protocol: 0, wills_due: false, heard }
    }

    async fn run(mut self, mut inbox: mpsc::UnboundedReceiver<Command>) {
        let mut failures = 0usize;
        loop {
            if self.streams.is_empty() && self.pending.is_empty() {
                match inbox.recv().await {
                    Some(command) => self.take(command, None).await,
                    None => return,
                }
                while let Ok(command) = inbox.try_recv() {
                    self.take(command, None).await;
                }
                continue;
            }
            match Socket::connect(&self.url).await {
                Ok(socket) => {
                    failures = 0;
                    let _ = self.events.send(SyncEvent::Connected);
                    let ended = self.serve(socket, &mut inbox).await;
                    let _ = self.events.send(SyncEvent::Disconnected { streams: self.streams.keys().copied().collect() });
                    if matches!(ended, Ended::Stopped) {
                        return;
                    }
                }
                Err(e) => {
                    tracing::debug!("sync service unreachable: {e}");
                    failures += 1;
                }
            }
            let wait = reconnect_delay(failures);
            let deadline = Instant::now() + wait;
            loop {
                tokio::select! {
                    _ = time::sleep_until(deadline) => break,
                    command = inbox.recv() => match command {
                        Some(Command::Nudge) => {
                            failures = 0;
                            break;
                        }
                        Some(command) => self.take(command, None).await,
                        None => return,
                    },
                }
            }
        }
    }

    async fn serve(&mut self, mut socket: Socket, inbox: &mut mpsc::UnboundedReceiver<Command>) -> Ended {
        self.protocol = 0;
        self.wills_due = false;
        if !self.await_hello(&mut socket).await {
            return Ended::Lost;
        }
        let every: Vec<StreamId> = self.streams.keys().copied().collect();
        self.reopen.clear();
        if self.send_opens(&mut socket, &every).await.is_err() || self.flush_pending(&mut socket).await.is_err() {
            return Ended::Lost;
        }
        if self.wills_due && self.send_wills(&mut socket).await.is_err() {
            return Ended::Lost;
        }
        let mut keepalive = time::interval(Duration::from_secs(KEEPALIVE_SECS));
        keepalive.tick().await;
        let mut heard = Instant::now();
        let mut retry = time::interval(Duration::from_millis(500));
        loop {
            tokio::select! {
                command = inbox.recv() => match command {
                    Some(Command::Nudge) => {
                        if gone_quiet(heard.elapsed()) || send(&mut socket, &ClientFrame::Ping).await.is_err() {
                            return Ended::Lost;
                        }
                    }
                    Some(command) => {
                        self.take(command, Some(&mut socket)).await;
                    }
                    None => {
                        socket.close().await;
                        return Ended::Stopped;
                    }
                },
                message = socket.next() => {
                    heard = Instant::now();
                    match message {
                        Incoming::Text(text) => match serde_json::from_str::<ServerFrame>(&text) {
                            Ok(frame) => {
                                self.handle(frame);
                                if self.wills_due && self.send_wills(&mut socket).await.is_err() {
                                    return Ended::Lost;
                                }
                            }
                            Err(e) => tracing::debug!("sync service sent an unknown frame: {e}"),
                        },
                        Incoming::Other => {}
                        Incoming::Closed => return Ended::Lost,
                    }
                }
                _ = keepalive.tick() => {
                    if heard.elapsed() > SILENCE_LIMIT || send(&mut socket, &ClientFrame::Ping).await.is_err() {
                        return Ended::Lost;
                    }
                }
                _ = retry.tick() => {
                    if self.flush_due(&mut socket).await.is_err() {
                        return Ended::Lost;
                    }
                }
            }
        }
    }

    async fn take(&mut self, command: Command, socket: Option<&mut Socket>) {
        match command {
            Command::Nudge => {}
            Command::Follow { keys, after, tail } => {
                let known = self.streams.get(&keys.id);
                let after = known.map_or(after, |f| f.after.max(after));
                let will = known.and_then(|f| f.will.clone());
                let followed = Followed { keys: *keys, after, tail, will };
                let open = followed.open();
                self.streams.insert(followed.keys.id, followed);
                if let Some(socket) = socket {
                    let _ = send(socket, &open).await;
                }
            }
            Command::Unfollow { stream } => {
                if self.streams.remove(&stream).is_some() {
                    if let Some(socket) = socket {
                        let _ = send(socket, &ClientFrame::Close { stream }).await;
                    }
                }
            }
            Command::Signal { stream, plain } => {
                let (Some(followed), Some(socket)) = (self.streams.get(&stream), socket) else { return };
                if self.protocol < SIGNALS_SINCE {
                    return;
                }
                let Ok(body) = followed.keys.seal(&plain) else { return };
                let _ = send(socket, &ClientFrame::signal(&followed.keys, rand::random(), body)).await;
            }
            Command::Will { stream, plain } => {
                let Some(followed) = self.streams.get_mut(&stream) else { return };
                followed.will = Some(plain);
                let (Some(socket), true) = (socket, self.protocol >= WILLS_SINCE) else { return };
                if let Some(frame) = followed.last_word() {
                    let _ = send(socket, &frame).await;
                }
            }
            Command::Append { stream, plain, reply } => {
                let Some(followed) = self.streams.get(&stream) else {
                    let _ = reply.send(Err(AppendError::NotFollowed));
                    return;
                };
                let body = match followed.keys.seal(&plain) {
                    Ok(body) => body,
                    Err(e) => {
                        let _ = reply.send(Err(e.into()));
                        return;
                    }
                };
                let nonce: [u8; APPEND_NONCE_BYTES] = rand::random();
                let frame = ClientFrame::append(&followed.keys, nonce, body);
                if let Some(socket) = socket {
                    let _ = send(socket, &frame).await;
                }
                self.pending.insert(nonce, Pending { frame, reply, not_before: Instant::now() + Duration::from_secs(30) });
            }
        }
    }

    async fn await_hello(&mut self, socket: &mut Socket) -> bool {
        let deadline = Instant::now() + HELLO_WAIT;
        while self.protocol == 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            match time::timeout(left, socket.next()).await {
                Err(_) => return true,
                Ok(Incoming::Text(text)) => match serde_json::from_str::<ServerFrame>(&text) {
                    Ok(frame) => self.handle(frame),
                    Err(e) => tracing::debug!("sync service sent an unknown frame: {e}"),
                },
                Ok(Incoming::Other) => {}
                Ok(Incoming::Closed) => return false,
            }
        }
        true
    }

    async fn send_opens(&self, socket: &mut Socket, streams: &[StreamId]) -> Result<(), ()> {
        for frame in open_frames(self.protocol, streams.iter().filter_map(|stream| self.streams.get(stream))) {
            send(socket, &frame).await?;
        }
        Ok(())
    }

    async fn send_wills(&mut self, socket: &mut Socket) -> Result<(), ()> {
        self.wills_due = false;
        for frame in self.streams.values().filter_map(Followed::last_word) {
            send(socket, &frame).await?;
        }
        Ok(())
    }

    async fn flush_pending(&mut self, socket: &mut Socket) -> Result<(), ()> {
        for pending in self.pending.values_mut() {
            send(socket, &pending.frame).await?;
            pending.not_before = Instant::now() + Duration::from_secs(30);
        }
        Ok(())
    }

    async fn flush_due(&mut self, socket: &mut Socket) -> Result<(), ()> {
        let now = Instant::now();
        let due: Vec<StreamId> = self.reopen.iter().filter(|(_, at)| **at <= now).map(|(id, _)| *id).collect();
        for stream in &due {
            self.reopen.remove(stream);
        }
        self.send_opens(socket, &due).await?;
        for pending in self.pending.values_mut().filter(|p| p.not_before <= now) {
            send(socket, &pending.frame).await?;
            pending.not_before = now + Duration::from_secs(30);
        }
        Ok(())
    }

    fn handle(&mut self, frame: ServerFrame) {
        if let Some(greeting) = frame.greeting() {
            *self.heard.lock().unwrap() = Some(greeting);
        }
        match frame {
            ServerFrame::Hello { protocol, .. } => {
                self.protocol = protocol;
                self.wills_due = protocol >= WILLS_SINCE;
            }
            ServerFrame::Pong | ServerFrame::Claimed { .. } | ServerFrame::Erased { .. } | ServerFrame::Released { .. } | ServerFrame::Reported { .. } => {}
            ServerFrame::Opened { stream, head } => {
                if let Some(followed) = self.streams.get_mut(&stream) {
                    if head < followed.after {
                        followed.after = 0;
                    }
                }
            }
            ServerFrame::Entries { stream, entries, caught_up } => {
                let Some(followed) = self.streams.get_mut(&stream) else { return };
                for entry in entries {
                    if entry.seq <= followed.after {
                        continue;
                    }
                    followed.after = entry.seq;
                    let event = match followed.keys.open(&entry.body.0) {
                        Ok(plain) => SyncEvent::Entry { stream, seq: entry.seq, at: entry.at, plain },
                        Err(_) => SyncEvent::Unreadable { stream, seq: entry.seq },
                    };
                    let _ = self.events.send(event);
                }
                if caught_up {
                    let _ = self.events.send(SyncEvent::CaughtUp { stream, seq: followed.after });
                }
            }
            ServerFrame::Signal { stream, body } => {
                let Some(followed) = self.streams.get(&stream) else { return };
                if let Ok(plain) = followed.keys.open(&body.0) {
                    let _ = self.events.send(SyncEvent::Signal { stream, plain });
                }
            }
            ServerFrame::Appended { nonce, seq, .. } => {
                if let Some(pending) = self.pending.remove(&nonce.0) {
                    let _ = pending.reply.send(Ok(seq));
                }
            }
            ServerFrame::Error { code, stream, nonce, retry_after_ms } => {
                if let Some(nonce) = nonce {
                    if code.is_retryable() {
                        if let Some(pending) = self.pending.get_mut(&nonce.0) {
                            pending.not_before = Instant::now() + Duration::from_millis(retry_after_ms.unwrap_or(RETRY_AFTER_DEFAULT_MS));
                        }
                    } else if let Some(pending) = self.pending.remove(&nonce.0) {
                        let _ = pending.reply.send(Err(AppendError::Refused(code)));
                    }
                    return;
                }
                if let Some(stream) = stream {
                    if code.is_retryable() {
                        let wait = Duration::from_millis(retry_after_ms.unwrap_or(RETRY_AFTER_DEFAULT_MS));
                        self.reopen.insert(stream, Instant::now() + wait);
                    } else if self.streams.remove(&stream).is_some() {
                        let stranded: Vec<_> = self
                            .pending
                            .iter()
                            .filter(|(_, p)| matches!(&p.frame, ClientFrame::Append { stream: s, .. } if *s == stream))
                            .map(|(nonce, _)| *nonce)
                            .collect();
                        for nonce in stranded {
                            if let Some(pending) = self.pending.remove(&nonce) {
                                let _ = pending.reply.send(Err(AppendError::Refused(code)));
                            }
                        }
                        let _ = self.events.send(SyncEvent::Refused { stream, code });
                    }
                }
            }
        }
    }
}

fn open_frames<'a>(protocol: u32, followed: impl Iterator<Item = &'a Followed>) -> Vec<ClientFrame> {
    let followed: Vec<&Followed> = followed.collect();
    if protocol < OPEN_MANY_SINCE || followed.len() < 2 {
        return followed.iter().map(|f| f.open()).collect();
    }
    followed.chunks(MAX_STREAMS_PER_CONNECTION).map(|chunk| ClientFrame::open_many(chunk.iter().map(|f| f.open_request()).collect())).collect()
}

fn gone_quiet(silent: Duration) -> bool {
    silent > Duration::from_secs(KEEPALIVE_SECS)
}

fn reconnect_delay(failures: usize) -> Duration {
    let base = RECONNECT_STEPS_MS[failures.min(RECONNECT_STEPS_MS.len() - 1)];
    let spread = rand::random::<u64>() % (base / 2 + 1);
    Duration::from_millis(base + spread)
}

async fn send(socket: &mut Socket, frame: &ClientFrame) -> Result<(), ()> {
    let text = serde_json::to_string(frame).map_err(|_| ())?;
    socket.send(text).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn followed(count: usize) -> Vec<Followed> {
        (0..count).map(|n| Followed { keys: crate::StreamSecret::random().keys(), after: n as u64, tail: None, will: None }).collect()
    }

    #[test]
    fn services_that_know_open_many_get_streams_in_batches() {
        let streams = followed(MAX_STREAMS_PER_CONNECTION + 3);
        let frames = open_frames(OPEN_MANY_SINCE, streams.iter());
        assert_eq!(frames.len(), 2);
        let ClientFrame::OpenMany { opens } = &frames[0] else { panic!("a batch") };
        assert_eq!(opens.len(), MAX_STREAMS_PER_CONNECTION);
        assert_eq!(opens[0], streams[0].open_request());
        let ClientFrame::OpenMany { opens } = &frames[1] else { panic!("a batch") };
        assert_eq!(opens.len(), 3);
    }

    #[test]
    fn older_services_and_single_streams_get_plain_opens() {
        let streams = followed(4);
        let frames = open_frames(OPEN_MANY_SINCE - 1, streams.iter());
        assert_eq!(frames, streams.iter().map(Followed::open).collect::<Vec<_>>());
        assert_eq!(open_frames(OPEN_MANY_SINCE, streams[..1].iter()), vec![streams[0].open()]);
        assert!(open_frames(OPEN_MANY_SINCE, std::iter::empty()).is_empty());
    }

    #[test]
    fn reconnects_back_off_and_spread_out() {
        assert!(reconnect_delay(0) >= Duration::from_millis(500));
        assert!(reconnect_delay(0) <= Duration::from_millis(750));
        assert!(reconnect_delay(50) >= Duration::from_secs(60));
        assert!(reconnect_delay(50) <= Duration::from_secs(90));
    }

    #[test]
    fn a_nudge_drops_a_link_that_went_quiet_while_the_app_slept() {
        assert!(!gone_quiet(Duration::from_secs(5)));
        assert!(!gone_quiet(Duration::from_secs(KEEPALIVE_SECS)));
        assert!(gone_quiet(Duration::from_secs(KEEPALIVE_SECS + 1)));
        assert!(gone_quiet(Duration::from_secs(600)));
    }
}
