use std::collections::HashMap;

use n0_future::time::{self, Duration, Instant};
use tokio::sync::{mpsc, oneshot};

use crate::stream::{SealError, StreamId, StreamKeys, APPEND_NONCE_BYTES};
use crate::wire::{ClientFrame, ErrorCode, ServerFrame, KEEPALIVE_SECS, SIGNALS_SINCE};

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
#[path = "client/native.rs"]
mod socket;
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
#[path = "client/browser.rs"]
mod socket;

use socket::{Incoming, Socket};

const RECONNECT_STEPS_MS: [u64; 6] = [500, 2_000, 5_000, 10_000, 30_000, 60_000];
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const SILENCE_LIMIT: Duration = Duration::from_secs(KEEPALIVE_SECS * 2 + 10);
const RETRY_AFTER_DEFAULT_MS: u64 = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncEvent {
    Connected,
    Disconnected,
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
    Nudge,
}

#[derive(Clone)]
pub struct SyncClient {
    commands: mpsc::UnboundedSender<Command>,
}

impl SyncClient {
    pub fn spawn(url: impl Into<String>) -> (SyncClient, mpsc::UnboundedReceiver<SyncEvent>) {
        let (commands, inbox) = mpsc::unbounded_channel();
        let (events, received) = mpsc::unbounded_channel();
        n0_future::task::spawn(Link::new(url.into(), events).run(inbox));
        (SyncClient { commands }, received)
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
}

impl Followed {
    fn open(&self) -> ClientFrame {
        ClientFrame::open_tail(&self.keys, self.after, self.tail)
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
}

enum Ended {
    Lost,
    Stopped,
}

impl Link {
    fn new(url: String, events: mpsc::UnboundedSender<SyncEvent>) -> Self {
        Self { url, events, streams: HashMap::new(), pending: HashMap::new(), reopen: HashMap::new(), protocol: 0 }
    }

    async fn run(mut self, mut inbox: mpsc::UnboundedReceiver<Command>) {
        let mut failures = 0usize;
        loop {
            if self.streams.is_empty() && self.pending.is_empty() {
                match inbox.recv().await {
                    Some(command) => self.take(command, None).await,
                    None => return,
                }
                continue;
            }
            match Socket::connect(&self.url).await {
                Ok(socket) => {
                    failures = 0;
                    let _ = self.events.send(SyncEvent::Connected);
                    let ended = self.serve(socket, &mut inbox).await;
                    let _ = self.events.send(SyncEvent::Disconnected);
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
        for followed in self.streams.values() {
            if send(&mut socket, &followed.open()).await.is_err() {
                return Ended::Lost;
            }
        }
        self.reopen.clear();
        self.protocol = 0;
        if self.flush_pending(&mut socket).await.is_err() {
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
                            Ok(frame) => self.handle(frame),
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
                let after = self.streams.get(&keys.id).map_or(after, |f| f.after.max(after));
                let followed = Followed { keys: *keys, after, tail };
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
        for stream in due {
            self.reopen.remove(&stream);
            if let Some(followed) = self.streams.get(&stream) {
                send(socket, &followed.open()).await?;
            }
        }
        for pending in self.pending.values_mut().filter(|p| p.not_before <= now) {
            send(socket, &pending.frame).await?;
            pending.not_before = now + Duration::from_secs(30);
        }
        Ok(())
    }

    fn handle(&mut self, frame: ServerFrame) {
        match frame {
            ServerFrame::Hello { protocol } => self.protocol = protocol,
            ServerFrame::Pong => {}
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
