use std::collections::{HashMap, HashSet};
use std::hash::Hash;

pub use n0_future::time::Instant;
use n0_future::time::{self, Duration, SystemTime};

use crate::client::socket::{Incoming, Socket};
use crate::stream::{Eraser, StreamId, StreamKeys};
use crate::wire::{ClientFrame, ErrorCode, ServerFrame, ERASE_SINCE};

const HELLO_WAIT: Duration = Duration::from_secs(10);
const ANSWER_WAIT: Duration = Duration::from_secs(15);
const TRIES: usize = 5;
const LONGEST_PAUSE_MS: u64 = 5_000;
pub const TAKEN_WAIT: Duration = Duration::from_secs(5 * 60);
pub const CLAIMS_PER_ROUND: usize = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Errand {
    Claim,
    Erase { through: u64 },
    Release,
}

#[derive(Debug, Clone)]
pub struct OwnerWork {
    pub keys: StreamKeys,
    pub eraser: Eraser,
    pub errand: Errand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Theirs,
    Refused(ErrorCode),
    Unanswered,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ErrandError {
    #[error("the sync service is unreachable: {0}")]
    Unreachable(String),
    #[error("the sync service is too old to erase streams")]
    TooOld,
}

pub struct Round<K> {
    pub given_up: Vec<K>,
    pub claims: Vec<OwnerWork>,
}

pub struct ClaimBook<K> {
    url: String,
    root: [u8; 32],
    owned: HashSet<K>,
    done: HashSet<StreamId>,
    taken: HashMap<StreamId, Instant>,
    busy: bool,
}

impl<K> Default for ClaimBook<K> {
    fn default() -> Self {
        Self { url: String::new(), root: [0; 32], owned: HashSet::new(), done: HashSet::new(), taken: HashMap::new(), busy: false }
    }
}

impl<K: Hash + Eq + Copy> ClaimBook<K> {
    pub fn round(&mut self, url: &str, root: [u8; 32], owned: HashSet<K>, followed: &HashMap<K, Vec<StreamKeys>>, now: Instant) -> Round<K> {
        if self.url != url || self.root != root {
            *self = ClaimBook { url: url.to_string(), root, ..ClaimBook::default() };
        }
        let given_up: Vec<K> = self.owned.difference(&owned).copied().collect();
        self.owned = owned;
        let mine = followed.iter().filter(|(server, _)| self.owned.contains(*server)).flat_map(|(_, streams)| streams);
        let claims = mine
            .filter(|keys| self.due(keys.id, now))
            .take(CLAIMS_PER_ROUND)
            .map(|keys| OwnerWork { keys: keys.clone(), eraser: Eraser::derive(&root, &keys.id), errand: Errand::Claim })
            .collect();
        Round { given_up, claims }
    }

    pub fn letting_go(&mut self, streams: impl IntoIterator<Item = StreamId>) {
        for stream in streams {
            self.done.remove(&stream);
        }
    }

    pub fn start(&mut self) -> bool {
        !std::mem::replace(&mut self.busy, true)
    }

    pub fn finish(&mut self, asked: &[(StreamId, Errand)], answered: &[(StreamId, Outcome)], now: Instant) {
        self.busy = false;
        for ((stream, errand), (_, outcome)) in asked.iter().zip(answered) {
            match (errand, outcome) {
                (Errand::Claim, Outcome::Done) => {
                    self.done.insert(*stream);
                    self.taken.remove(stream);
                }
                (Errand::Claim, Outcome::Theirs) => {
                    tracing::warn!("another key holds the eraser of a server stream you own; trying again later");
                    self.taken.insert(*stream, now + TAKEN_WAIT);
                }
                _ => {}
            }
        }
    }

    fn due(&self, stream: StreamId, now: Instant) -> bool {
        !self.done.contains(&stream) && self.taken.get(&stream).is_none_or(|until| *until <= now)
    }
}

pub fn owner_work(root: &[u8; 32], streams: &[StreamKeys], errand: impl Fn(StreamId) -> Errand) -> Vec<OwnerWork> {
    streams.iter().map(|keys| OwnerWork { keys: keys.clone(), eraser: Eraser::derive(root, &keys.id), errand: errand(keys.id) }).collect()
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

pub const ERASE_WAITS: [Duration; 5] =
    [Duration::ZERO, Duration::from_secs(10), Duration::from_secs(60), Duration::from_secs(5 * 60), Duration::from_secs(20 * 60)];

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Finished {
    pub done: usize,
    pub refused: Vec<(StreamId, Outcome)>,
    pub left: usize,
    pub too_old: bool,
}

pub async fn keep_at(url: &str, mut work: Vec<OwnerWork>, waits: &[Duration]) -> Finished {
    let mut finished = Finished::default();
    for wait in waits {
        if work.is_empty() {
            break;
        }
        time::sleep(*wait).await;
        match run(url, work.clone()).await {
            Ok(outcomes) => {
                let unanswered: HashSet<StreamId> = outcomes.iter().filter(|(_, o)| *o == Outcome::Unanswered).map(|(s, _)| *s).collect();
                finished.done += outcomes.iter().filter(|(_, o)| *o == Outcome::Done).count();
                finished.refused.extend(outcomes.into_iter().filter(|(_, o)| matches!(o, Outcome::Theirs | Outcome::Refused(_))));
                work.retain(|w| unanswered.contains(&w.keys.id));
            }
            Err(ErrandError::TooOld) => {
                finished.too_old = true;
                break;
            }
            Err(e) => tracing::debug!("owner work waits for the sync service: {e}"),
        }
    }
    finished.left = work.len();
    finished
}

pub async fn run(url: &str, work: Vec<OwnerWork>) -> Result<Vec<(StreamId, Outcome)>, ErrandError> {
    let mut socket = Socket::connect(url).await.map_err(ErrandError::Unreachable)?;
    let outcome = serve(&mut socket, work).await;
    socket.close().await;
    outcome
}

async fn serve(socket: &mut Socket, work: Vec<OwnerWork>) -> Result<Vec<(StreamId, Outcome)>, ErrandError> {
    let protocol = match time::timeout(HELLO_WAIT, next_frame(socket)).await {
        Ok(Some(ServerFrame::Hello { protocol })) => protocol,
        _ => return Err(ErrandError::Unreachable("no greeting".into())),
    };
    if protocol < ERASE_SINCE {
        return Err(ErrandError::TooOld);
    }
    let mut out = Vec::with_capacity(work.len());
    let mut lost = false;
    for item in work {
        let stream = item.keys.id;
        let outcome = if lost { Outcome::Unanswered } else { carry_out(socket, &item).await };
        lost |= outcome == Outcome::Unanswered;
        out.push((stream, outcome));
    }
    Ok(out)
}

async fn carry_out(socket: &mut Socket, item: &OwnerWork) -> Outcome {
    let stream = item.keys.id;
    match item.errand {
        Errand::Claim => claim(socket, item).await,
        Errand::Erase { through } => match claim(socket, item).await {
            Outcome::Done => ask(socket, stream, || ClientFrame::erase(stream, &item.eraser, through, now_ms())).await,
            other => other,
        },
        Errand::Release => ask(socket, stream, || ClientFrame::release(stream, &item.eraser, now_ms())).await,
    }
}

async fn claim(socket: &mut Socket, item: &OwnerWork) -> Outcome {
    ask(socket, item.keys.id, || ClientFrame::claim(&item.keys, &item.eraser)).await
}

async fn ask(socket: &mut Socket, stream: StreamId, frame: impl Fn() -> ClientFrame) -> Outcome {
    for _ in 0..TRIES {
        let Ok(text) = serde_json::to_string(&frame()) else { return Outcome::Refused(ErrorCode::BadFrame) };
        if socket.send(text).await.is_err() {
            return Outcome::Unanswered;
        }
        let answer = match time::timeout(ANSWER_WAIT, answer_for(socket, stream)).await {
            Ok(Some(answer)) => answer,
            _ => return Outcome::Unanswered,
        };
        match outcome_of(answer) {
            Retry(wait) => time::sleep(wait).await,
            Settled(outcome) => return outcome,
        }
    }
    Outcome::Unanswered
}

enum Next {
    Retry(Duration),
    Settled(Outcome),
}
use Next::{Retry, Settled};

fn outcome_of(answer: ServerFrame) -> Next {
    match answer {
        ServerFrame::Claimed { mine: true, .. } | ServerFrame::Erased { .. } | ServerFrame::Released { .. } => Settled(Outcome::Done),
        ServerFrame::Claimed { mine: false, .. } => Settled(Outcome::Theirs),
        ServerFrame::Error { code: ErrorCode::NotYours, .. } => Settled(Outcome::Theirs),
        ServerFrame::Error { code, retry_after_ms, .. } if code.is_retryable() => {
            Retry(Duration::from_millis(retry_after_ms.unwrap_or(1_000).min(LONGEST_PAUSE_MS)))
        }
        ServerFrame::Error { code, .. } => Settled(Outcome::Refused(code)),
        _ => Settled(Outcome::Unanswered),
    }
}

fn answers(frame: &ServerFrame, stream: StreamId) -> bool {
    match frame {
        ServerFrame::Claimed { stream: s, .. } | ServerFrame::Erased { stream: s, .. } | ServerFrame::Released { stream: s } => *s == stream,
        ServerFrame::Error { stream: Some(s), nonce: None, .. } => *s == stream,
        ServerFrame::Error { stream: None, nonce: None, .. } => true,
        _ => false,
    }
}

async fn answer_for(socket: &mut Socket, stream: StreamId) -> Option<ServerFrame> {
    loop {
        let frame = next_frame(socket).await?;
        if answers(&frame, stream) {
            return Some(frame);
        }
    }
}

async fn next_frame(socket: &mut Socket) -> Option<ServerFrame> {
    loop {
        match socket.next().await {
            Incoming::Text(text) => {
                if let Ok(frame) = serde_json::from_str::<ServerFrame>(&text) {
                    return Some(frame);
                }
            }
            Incoming::Other => {}
            Incoming::Closed => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StreamSecret;

    fn book_with(owned: &[u8], followed: &HashMap<u8, Vec<StreamKeys>>, book: &mut ClaimBook<u8>, now: Instant) -> Round<u8> {
        book.round("wss://a/sync", [1; 32], owned.iter().copied().collect(), followed, now)
    }

    #[test]
    fn an_owner_claims_each_stream_once_and_waits_on_one_held_by_another_key() {
        let (mine, taken, failed) = (StreamSecret::random().keys(), StreamSecret::random().keys(), StreamSecret::random().keys());
        let joined = StreamSecret::random().keys();
        let followed = HashMap::from([(1u8, vec![mine.clone(), taken.clone(), failed.clone()]), (2u8, vec![joined.clone()])]);
        let mut book = ClaimBook::default();
        let now = Instant::now();
        let round = book_with(&[1], &followed, &mut book, now);
        let asked: Vec<(StreamId, Errand)> = round.claims.iter().map(|w| (w.keys.id, w.errand)).collect();
        assert_eq!(asked.len(), 3, "only streams of servers we own are claimed");
        assert!(round.claims.iter().all(|w| w.eraser == Eraser::derive(&[1; 32], &w.keys.id)));
        assert!(book.start());
        assert!(!book.start(), "one round at a time");
        book.finish(&asked, &[(mine.id, Outcome::Done), (taken.id, Outcome::Theirs), (failed.id, Outcome::Unanswered)], now);
        let again: Vec<StreamId> = book_with(&[1], &followed, &mut book, now).claims.iter().map(|w| w.keys.id).collect();
        assert_eq!(again, vec![failed.id], "a held claim is not sent again, and one held by another key waits");
        let later: HashSet<StreamId> = book_with(&[1], &followed, &mut book, now + TAKEN_WAIT).claims.iter().map(|w| w.keys.id).collect();
        assert_eq!(later, HashSet::from([taken.id, failed.id]), "the previous owner may have let go by now");
    }

    #[test]
    fn a_big_owner_claims_a_few_streams_at_a_time() {
        let many: Vec<StreamKeys> = (0..CLAIMS_PER_ROUND * 2 + 3).map(|_| StreamSecret::random().keys()).collect();
        let followed = HashMap::from([(1u8, many)]);
        let mut book = ClaimBook::default();
        let now = Instant::now();
        let round = book_with(&[1], &followed, &mut book, now);
        assert_eq!(round.claims.len(), CLAIMS_PER_ROUND, "claims leave room for the app's own streams to open");
        let asked: Vec<(StreamId, Errand)> = round.claims.iter().map(|w| (w.keys.id, w.errand)).collect();
        let answered: Vec<(StreamId, Outcome)> = asked.iter().map(|(id, _)| (*id, Outcome::Done)).collect();
        book.finish(&asked, &answered, now);
        let next = book_with(&[1], &followed, &mut book, now);
        assert_eq!(next.claims.len(), CLAIMS_PER_ROUND);
        assert!(next.claims.iter().all(|w| !asked.iter().any(|(id, _)| *id == w.keys.id)));
    }

    #[test]
    fn handing_a_server_on_lets_go_of_it_once() {
        let keys = StreamSecret::random().keys();
        let followed = HashMap::from([(1u8, vec![keys.clone()])]);
        let mut book = ClaimBook::default();
        let now = Instant::now();
        book_with(&[1], &followed, &mut book, now);
        book.finish(&[(keys.id, Errand::Claim)], &[(keys.id, Outcome::Done)], now);
        let handed = book_with(&[], &followed, &mut book, now);
        assert_eq!(handed.given_up, vec![1]);
        assert!(handed.claims.is_empty());
        assert!(book_with(&[], &followed, &mut book, now).given_up.is_empty());
        book.letting_go([keys.id]);
        assert_eq!(book_with(&[1], &followed, &mut book, now).claims.len(), 1, "owning it again claims it again");
    }

    #[test]
    fn another_account_or_service_starts_the_claims_over() {
        let keys = StreamSecret::random().keys();
        let followed = HashMap::from([(1u8, vec![keys.clone()])]);
        let mut book = ClaimBook::default();
        let now = Instant::now();
        book_with(&[1], &followed, &mut book, now);
        book.finish(&[(keys.id, Errand::Claim)], &[(keys.id, Outcome::Done)], now);
        assert!(book.round("wss://b/sync", [1; 32], HashSet::from([1]), &followed, now).claims.len() == 1);
        book.finish(&[(keys.id, Errand::Claim)], &[(keys.id, Outcome::Done)], now);
        let other = book.round("wss://b/sync", [2; 32], HashSet::from([1]), &followed, now);
        assert_eq!(other.claims.len(), 1);
        assert!(other.given_up.is_empty(), "a new account has nothing to let go of");
    }

    #[test]
    fn answers_are_matched_to_their_stream() {
        let (a, b) = (StreamSecret::random().keys().id, StreamSecret::random().keys().id);
        assert!(answers(&ServerFrame::Claimed { stream: a, mine: true }, a));
        assert!(!answers(&ServerFrame::Claimed { stream: b, mine: true }, a));
        assert!(answers(&ServerFrame::stream_error(ErrorCode::NotYours, a), a));
        assert!(answers(&ServerFrame::error(ErrorCode::BadFrame), a), "a frame the service could not read answers whatever was asked");
        assert!(!answers(&ServerFrame::Pong, a));
    }

    #[test]
    fn only_passing_troubles_are_tried_again() {
        let a = StreamSecret::random().keys().id;
        assert!(matches!(outcome_of(ServerFrame::Erased { stream: a, through: 3 }), Settled(Outcome::Done)));
        assert!(matches!(outcome_of(ServerFrame::Claimed { stream: a, mine: false }), Settled(Outcome::Theirs)));
        assert!(matches!(outcome_of(ServerFrame::stream_error(ErrorCode::NotYours, a)), Settled(Outcome::Theirs)));
        assert!(matches!(outcome_of(ServerFrame::stream_error(ErrorCode::Expired, a)), Settled(Outcome::Refused(ErrorCode::Expired))));
        let limited = ServerFrame::Error { code: ErrorCode::RateLimited, stream: Some(a), nonce: None, retry_after_ms: Some(60_000) };
        assert!(matches!(outcome_of(limited), Retry(wait) if wait == Duration::from_millis(LONGEST_PAUSE_MS)));
    }
}
