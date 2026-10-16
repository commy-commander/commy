//! Catch-up sessions over a direct stream (ALPN `commy/catchup/1`).
//!
//! A member that was offline dials a gossip neighbor and the two swap
//! the recent history one of them lacks: signed messages, edits and
//! delete proofs. The reconciliation itself is `commy_core::catchup`;
//! this module carries it over the wire and is generic over the stream,
//! so it runs the same over QUIC and over an in-memory pipe in tests.
//!
//! A session, initiator first:
//! 1. `Hello` both ways, each signed over both node ids.
//! 2. The initiator sends `Digests`; the responder answers `Items` for
//!    the buckets that differ.
//! 3. The initiator sends `Want`, then `Push` pages of what the
//!    responder lacks, then `Done`.
//! 4. The responder sends `Push` pages of what was wanted, then `Done`.

use crate::media::{read_frame, write_frame, MediaError, MAX_MEDIA_FRAME};
use commy_core::catchup::{cap, day_number, diff, mismatched, BucketItems, CATCHUP_MAX_ITEMS};
use commy_core::{
    BucketDigest, BucketKey, Identity, IdentityKey, Message, MessageId, ServerEvent, ServerId,
    ServerState, Signature, TombstoneProof, UserId,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

pub const CATCHUP_ALPN: &[u8] = b"commy/catchup/1";

/// Most items (messages plus delete proofs) in one `Push` frame.
pub const PAGE_SIZE: usize = 200;

/// Largest clock difference a `Hello` may show, in ms.
pub const MAX_CLOCK_SKEW_MS: u64 = 10 * 60 * 1000;

/// Delay suggested to a peer that may succeed soon, in ms.
pub const RETRY_AFTER_MS: u64 = 3000;

/// Longest wait for the peer's next frame.
pub const FRAME_TIMEOUT: Duration = Duration::from_secs(30);

const HELLO_DOMAIN: &str = "commy-catchup/v1";

/// Bytes a frame may fill, leaving room for its own framing.
const FRAME_BUDGET: usize = MAX_MEDIA_FRAME as usize - 1024;

/// Frames of a catch-up session, postcard encoded. Variants are only
/// ever appended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CatchupFrame {
    /// Who is speaking. `sig` covers [`hello_bytes`].
    Hello {
        server: ServerId,
        user: UserId,
        identity: IdentityKey,
        ts_ms: i64,
        sig: Signature,
    },
    /// The session is turned down. A retry delay is given when trying
    /// again later may work.
    Refused {
        reason: String,
        retry_after_ms: Option<u64>,
    },
    /// The initiator's bucket digests.
    Digests(Vec<BucketDigest>),
    /// The responder's items of the buckets that differ.
    Items(Vec<BucketItems>),
    /// Ids the initiator lacks, newest first.
    Want(Vec<MessageId>),
    /// One page of messages and delete proofs.
    Push {
        messages: Vec<Message>,
        tombstones: Vec<TombstoneProof>,
    },
    /// No more pages from this side.
    Done,
}

/// Bytes a `Hello` signs. Both sides list the initiator's node first,
/// so a Hello only counts between those two nodes.
pub fn hello_bytes(
    server: ServerId,
    user: UserId,
    initiator: [u8; 32],
    responder: [u8; 32],
    ts_ms: i64,
) -> Vec<u8> {
    postcard::to_stdvec(&(HELLO_DOMAIN, server, user, initiator, responder, ts_ms))
        .expect("postcard encodes plain tuples")
}

impl CatchupFrame {
    /// A `Hello` from `user` for a session between two nodes.
    pub fn hello(
        identity: &Identity,
        server: ServerId,
        user: UserId,
        initiator: [u8; 32],
        responder: [u8; 32],
        ts_ms: i64,
    ) -> Self {
        let sig = identity.sign(&hello_bytes(server, user, initiator, responder, ts_ms));
        CatchupFrame::Hello {
            server,
            user,
            identity: identity.public(),
            ts_ms,
            sig,
        }
    }
}

/// Whether a peer may catch up with us, judged from our copy of the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerCheck {
    /// A member in good standing whose pinned key matches.
    Allowed,
    /// Not known yet, e.g. their join has not reached us. Worth a retry.
    Unknown,
    /// Banned, or holding a different key than the pinned one.
    Denied,
}

/// Outcome of applying one page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyReport {
    /// Ids whose message or delete changed our state.
    pub applied: Vec<MessageId>,
    /// Items set aside until their signer's key is known.
    pub held: usize,
}

/// What a session moved, from one side's point of view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionReport {
    /// Ids we sent that the peer lacked.
    pub pushed: Vec<MessageId>,
    /// Ids of ours the peer already held at the same or a newer
    /// version. With `pushed`, everything of ours the peer now holds.
    pub peer_had: Vec<MessageId>,
    /// Ids the peer sent that changed our state.
    pub received: Vec<MessageId>,
    /// Items from the peer set aside until their signer's key is known.
    pub held: usize,
}

/// Why a peer was turned down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Not a member we know yet.
    UnknownMember,
    /// Banned, or the key does not match the pinned one.
    Denied,
    /// We are not in that server.
    NotInServer,
    /// The signature is invalid or was made for other node ids.
    BadSignature,
    /// The timestamp is too far from our clock.
    ClockSkew,
    /// Too many sessions at once, or not ready yet.
    Busy,
}

impl Refusal {
    pub fn retry_after_ms(self) -> Option<u64> {
        match self {
            Refusal::UnknownMember | Refusal::Busy => Some(RETRY_AFTER_MS),
            _ => None,
        }
    }

    /// The `Refused` frame telling a peer about this refusal. Not being
    /// in the server reads as an unknown member, so a stranger cannot
    /// probe which servers we are in.
    pub fn frame(self) -> CatchupFrame {
        let shown = match self {
            Refusal::NotInServer => Refusal::UnknownMember,
            other => other,
        };
        CatchupFrame::Refused {
            reason: shown.to_string(),
            retry_after_ms: shown.retry_after_ms(),
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Refusal::UnknownMember => "unknown member",
            Refusal::Denied => "not allowed",
            Refusal::NotInServer => "not in that server",
            Refusal::BadSignature => "bad signature",
            Refusal::ClockSkew => "clock too far off",
            Refusal::Busy => "busy",
        })
    }
}

#[derive(Debug, Error)]
pub enum CatchupError {
    /// The peer turned us down.
    #[error("peer refused catch-up: {reason}")]
    Refused {
        reason: String,
        retry_after_ms: Option<u64>,
    },
    /// We turned the peer down.
    #[error("refused peer: {0}")]
    Rejected(Refusal),
    #[error("unexpected catch-up frame")]
    Unexpected,
    #[error("peer closed the stream early")]
    Closed,
    #[error("peer stopped responding")]
    Timeout,
    #[error("peer sent more than one session allows")]
    TooMuch,
    #[error(transparent)]
    Frame(#[from] MediaError),
}

impl CatchupError {
    /// Delay after which the peer suggested trying again, if any.
    pub fn retry_after_ms(&self) -> Option<u64> {
        match self {
            CatchupError::Refused { retry_after_ms, .. } => *retry_after_ms,
            _ => None,
        }
    }
}

/// The app's side of a session. Calls are short and synchronous;
/// implementations lock their state briefly inside each one.
pub trait CatchupStore {
    /// Our user id and identity for `server`, or None when we are not a
    /// member. Asked at the start of every session, so a swapped
    /// identity applies from the next one.
    fn local_identity(&self, server: ServerId) -> Option<(UserId, Identity)>;
    /// Whether `user` holding `key` may catch up with us, see [`check_peer`].
    fn verify_peer(&self, server: ServerId, user: UserId, key: &IdentityKey) -> PeerCheck;
    /// Our bucket digests, see `commy_core::catchup::digests`.
    fn digests(&self, server: ServerId) -> Vec<BucketDigest>;
    /// Our items of these buckets, see `commy_core::catchup::items`.
    fn items(&self, server: ServerId, keys: &[BucketKey]) -> Vec<BucketItems>;
    /// The messages and delete proofs we hold among `ids`, see [`collect`].
    fn get(&self, server: ServerId, ids: &[MessageId]) -> (Vec<Message>, Vec<TombstoneProof>);
    /// Applies one page from the peer, delete proofs first, see [`apply_items`].
    fn apply(
        &self,
        server: ServerId,
        messages: Vec<Message>,
        tombstones: Vec<TombstoneProof>,
    ) -> ApplyReport;
}

/// [`CatchupStore::verify_peer`] over one server's state.
pub fn check_peer(state: &ServerState, user: UserId, key: &IdentityKey) -> PeerCheck {
    if state.server.banned.contains(&user) {
        return PeerCheck::Denied;
    }
    if !state.server.is_member(user) {
        return PeerCheck::Unknown;
    }
    match state.identity_of(user) {
        Some(pinned) if pinned == *key => PeerCheck::Allowed,
        Some(_) => PeerCheck::Denied,
        None => PeerCheck::Unknown,
    }
}

/// [`CatchupStore::get`] over one server's state, in the order of `ids`.
pub fn collect(state: &ServerState, ids: &[MessageId]) -> (Vec<Message>, Vec<TombstoneProof>) {
    let mut order: HashMap<MessageId, usize> = HashMap::with_capacity(ids.len());
    for id in ids {
        let next = order.len();
        order.entry(*id).or_insert(next);
    }
    let mut messages: Vec<&Message> = state
        .server
        .channels
        .iter()
        .flat_map(|c| state.messages(c.id))
        .filter(|m| order.contains_key(&m.id))
        .collect();
    messages.sort_by_key(|m| order[&m.id]);
    let mut tombstones: Vec<&TombstoneProof> =
        order.keys().filter_map(|id| state.tombstone(*id)).collect();
    tombstones.sort_by_key(|p| order[&p.message]);
    (
        messages.into_iter().cloned().collect(),
        tombstones.into_iter().cloned().collect(),
    )
}

/// [`CatchupStore::apply`] over one server's state. Items whose signer
/// has no known key yet are returned as events for the app to hold and
/// retry once that member's profile arrives.
pub fn apply_items(
    state: &mut ServerState,
    messages: Vec<Message>,
    tombstones: Vec<TombstoneProof>,
) -> (ApplyReport, Vec<ServerEvent>) {
    let mut report = ApplyReport::default();
    let mut held = Vec::new();
    // Proofs first, so a message deleted elsewhere is never taken in.
    let events = tombstones
        .into_iter()
        .map(|p| (p.message, p.by, p.into_event()))
        .chain(
            messages
                .into_iter()
                .map(|m| (m.id, m.author, ServerEvent::MessageCreate { message: m })),
        );
    for (id, signer, event) in events {
        if state.identity_of(signer).is_none() {
            held.push(event);
            continue;
        }
        match state.apply(event) {
            Ok(true) => report.applied.push(id),
            Ok(false) => {}
            Err(e) => tracing::debug!("catch-up item {id} rejected: {e}"),
        }
    }
    report.held = held.len();
    (report, held)
}

/// Runs a session as the side that dialed. `local_node` and
/// `remote_node` are the two ends' node ids as the transport sees them.
pub async fn run_initiator<IO, S>(
    mut io: IO,
    store: &S,
    identity: &Identity,
    server: ServerId,
    me: UserId,
    local_node: [u8; 32],
    remote_node: [u8; 32],
) -> Result<SessionReport, CatchupError>
where
    IO: AsyncRead + AsyncWrite + Unpin,
    S: CatchupStore + ?Sized,
{
    let hello = CatchupFrame::hello(identity, server, me, local_node, remote_node, now_ms());
    send(&mut io, &hello).await?;
    match read(&mut io).await? {
        reply @ CatchupFrame::Hello { .. } => {
            if !matches!(reply, CatchupFrame::Hello { server: s, .. } if s == server) {
                return Err(CatchupError::Unexpected);
            }
            if let Err(refusal) = check_hello(store, &reply, local_node, remote_node) {
                return Err(refuse(&mut io, refusal).await);
            }
        }
        other => return Err(unexpected(other)),
    }

    let ours = store.digests(server);
    let mut keys: Vec<BucketKey> = ours.iter().map(|d| d.key).collect();
    send(&mut io, &CatchupFrame::Digests(ours)).await?;
    let theirs = match read(&mut io).await? {
        CatchupFrame::Items(list) => list,
        other => return Err(unexpected(other)),
    };

    // Buckets the responder did not list matched ours, unless it had to
    // leave some out to fit the frame (see `fit_items`).
    let answered: HashSet<BucketKey> = theirs.iter().map(|(key, _)| *key).collect();
    keys.extend(answered.iter().copied());
    keys.sort_unstable();
    keys.dedup();
    let (mine, settled): (Vec<BucketItems>, Vec<BucketItems>) = store
        .items(server, &keys)
        .into_iter()
        .partition(|(key, _)| answered.contains(key));
    let (mut they_lack, mut we_want) = diff(&mine, &theirs);
    log_capped("send", cap(&mut they_lack));
    log_capped("fetch", cap(&mut we_want));

    send(&mut io, &CatchupFrame::Want(we_want.clone())).await?;
    let (messages, tombstones) = store.get(server, &they_lack);
    let pushed = send_pages(&mut io, messages, tombstones).await?;
    send(&mut io, &CatchupFrame::Done).await?;

    let wanted: HashSet<MessageId> = we_want.into_iter().collect();
    let (received, held) =
        receive_pages(&mut io, store, server, |id, _| wanted.contains(&id)).await?;

    let pushed_set: HashSet<MessageId> = pushed.iter().copied().collect();
    let peer_had = initiator_peer_had(&settled, &mine, &theirs)
        .into_iter()
        .filter(|id| !pushed_set.contains(id))
        .collect();
    Ok(SessionReport {
        pushed,
        peer_had,
        received,
        held,
    })
}

/// Runs a session as the side that was dialed. The server comes from
/// the initiator's Hello and is returned with the report.
pub async fn run_responder<IO, S>(
    mut io: IO,
    store: &S,
    local_node: [u8; 32],
    remote_node: [u8; 32],
) -> Result<(ServerId, SessionReport), CatchupError>
where
    IO: AsyncRead + AsyncWrite + Unpin,
    S: CatchupStore + ?Sized,
{
    let hello = read(&mut io).await?;
    let CatchupFrame::Hello { server, .. } = hello else {
        return Err(unexpected(hello));
    };
    let Some((me, identity)) = store.local_identity(server) else {
        return Err(refuse(&mut io, Refusal::NotInServer).await);
    };
    if let Err(refusal) = check_hello(store, &hello, remote_node, local_node) {
        return Err(refuse(&mut io, refusal).await);
    }
    let reply = CatchupFrame::hello(&identity, server, me, remote_node, local_node, now_ms());
    send(&mut io, &reply).await?;

    let theirs = match read(&mut io).await? {
        CatchupFrame::Digests(list) => list,
        other => return Err(unexpected(other)),
    };
    let ours = store.digests(server);
    let differ: HashSet<BucketKey> = mismatched(&ours, &theirs).into_iter().collect();
    let mut keys: Vec<BucketKey> = ours
        .iter()
        .map(|d| d.key)
        .chain(differ.iter().copied())
        .collect();
    keys.sort_unstable();
    keys.dedup();
    let (answer, settled): (Vec<BucketItems>, Vec<BucketItems>) = store
        .items(server, &keys)
        .into_iter()
        .partition(|(key, _)| differ.contains(key));
    let answer = fit_items(answer);
    let answered: HashSet<BucketKey> = answer.iter().map(|(key, _)| *key).collect();
    let answered_ids: Vec<MessageId> = answer
        .iter()
        .flat_map(|(_, list)| list.iter().map(|(id, _)| *id))
        .collect();
    send(&mut io, &CatchupFrame::Items(answer)).await?;

    let want = match read(&mut io).await? {
        CatchupFrame::Want(ids) => ids,
        other => return Err(unexpected(other)),
    };
    // A full list may have been cut short, so it says nothing about the rest.
    let want_complete = want.len() < CATCHUP_MAX_ITEMS;
    let want_all: HashSet<MessageId> = want.iter().copied().collect();
    let offered: HashSet<MessageId> = answered_ids.iter().copied().collect();
    let mut want: Vec<MessageId> = want.into_iter().filter(|id| offered.contains(id)).collect();
    log_capped("send", cap(&mut want));

    let (received, held) =
        receive_pages(&mut io, store, server, |_, key| answered.contains(&key)).await?;

    let (messages, tombstones) = store.get(server, &want);
    let pushed = send_pages(&mut io, messages, tombstones).await?;
    send(&mut io, &CatchupFrame::Done).await?;

    let mut known: Vec<MessageId> = settled
        .iter()
        .flat_map(|(_, list)| list.iter().map(|(id, _)| *id))
        .collect();
    if want_complete {
        known.extend(answered_ids);
    }
    let pushed_set: HashSet<MessageId> = pushed.iter().copied().collect();
    let peer_had = dedup(
        known
            .into_iter()
            .filter(|id| !want_all.contains(id) && !pushed_set.contains(id)),
    );
    Ok((
        server,
        SessionReport {
            pushed,
            peer_had,
            received,
            held,
        },
    ))
}

/// Checks a peer's Hello: signature over this session's node ids, clock,
/// then standing in our copy of the server.
fn check_hello<S: CatchupStore + ?Sized>(
    store: &S,
    hello: &CatchupFrame,
    initiator: [u8; 32],
    responder: [u8; 32],
) -> Result<(), Refusal> {
    let CatchupFrame::Hello {
        server,
        user,
        identity,
        ts_ms,
        sig,
    } = hello
    else {
        return Err(Refusal::BadSignature);
    };
    if !identity.verify(
        &hello_bytes(*server, *user, initiator, responder, *ts_ms),
        sig,
    ) {
        return Err(Refusal::BadSignature);
    }
    if ts_ms.abs_diff(now_ms()) > MAX_CLOCK_SKEW_MS {
        return Err(Refusal::ClockSkew);
    }
    match store.verify_peer(*server, *user, identity) {
        PeerCheck::Allowed => Ok(()),
        PeerCheck::Unknown => Err(Refusal::UnknownMember),
        PeerCheck::Denied => Err(Refusal::Denied),
    }
}

/// Ids of ours the responder holds at no lower version: all of the
/// settled buckets, and whatever its item lists show for the rest.
fn initiator_peer_had(
    settled: &[BucketItems],
    mine: &[BucketItems],
    theirs: &[BucketItems],
) -> Vec<MessageId> {
    let mut index: HashMap<(BucketKey, MessageId), i64> = HashMap::new();
    for (key, list) in theirs {
        for &(id, version) in list {
            let v = index.entry((*key, id)).or_insert(version);
            *v = (*v).max(version);
        }
    }
    let settled_ids = settled
        .iter()
        .flat_map(|(_, list)| list.iter().map(|(id, _)| *id));
    let confirmed = mine.iter().flat_map(|(key, list)| {
        list.iter()
            .filter(|(id, version)| index.get(&(*key, *id)).is_some_and(|v| v >= version))
            .map(|(id, _)| *id)
    });
    dedup(settled_ids.chain(confirmed))
}

/// Reads `Push` pages until `Done`, applying the items `accept` lets
/// through. Returns the ids applied and how many items were held.
async fn receive_pages<IO, S>(
    io: &mut IO,
    store: &S,
    server: ServerId,
    accept: impl Fn(MessageId, BucketKey) -> bool,
) -> Result<(Vec<MessageId>, usize), CatchupError>
where
    IO: AsyncRead + Unpin,
    S: CatchupStore + ?Sized,
{
    // A message and its delete proof per id at most; empty pages count too.
    let mut budget = 2 * CATCHUP_MAX_ITEMS;
    let mut received = Vec::new();
    let mut held = 0;
    let mut ignored = 0;
    loop {
        match read(io).await? {
            CatchupFrame::Push {
                mut messages,
                mut tombstones,
            } => {
                let size = messages.len() + tombstones.len();
                budget = budget
                    .checked_sub(size.max(1))
                    .ok_or(CatchupError::TooMuch)?;
                messages.retain(|m| {
                    accept(
                        m.id,
                        BucketKey::Messages {
                            channel: m.channel,
                            day: day_number(m.created_at),
                        },
                    )
                });
                tombstones.retain(|p| {
                    accept(
                        p.message,
                        BucketKey::Tombstones {
                            day: day_number(p.at),
                        },
                    )
                });
                ignored += size - messages.len() - tombstones.len();
                if messages.is_empty() && tombstones.is_empty() {
                    continue;
                }
                let report = store.apply(server, messages, tombstones);
                received.extend(report.applied);
                held += report.held;
            }
            CatchupFrame::Done => break,
            other => return Err(unexpected(other)),
        }
    }
    if ignored > 0 {
        tracing::debug!("catch-up ignored {ignored} items nobody asked for");
    }
    Ok((dedup(received), held))
}

/// Sends items as `Push` pages and returns the ids sent.
async fn send_pages<W: AsyncWrite + Unpin>(
    io: &mut W,
    messages: Vec<Message>,
    tombstones: Vec<TombstoneProof>,
) -> Result<Vec<MessageId>, CatchupError> {
    let mut sent = Vec::new();
    for page in pages(messages, tombstones) {
        if let CatchupFrame::Push {
            messages,
            tombstones,
        } = &page
        {
            sent.extend(tombstones.iter().map(|p| p.message));
            sent.extend(messages.iter().map(|m| m.id));
        }
        write_frame(io, &page).await?;
    }
    io.flush().await.map_err(MediaError::from)?;
    Ok(dedup(sent))
}

/// Splits items into `Push` frames of at most [`PAGE_SIZE`] items that
/// each fit in a frame, delete proofs first. An item too large for any
/// frame is left out.
fn pages(messages: Vec<Message>, tombstones: Vec<TombstoneProof>) -> Vec<CatchupFrame> {
    enum Item {
        Message(Message),
        Tombstone(TombstoneProof),
    }
    let items = tombstones
        .into_iter()
        .map(Item::Tombstone)
        .chain(messages.into_iter().map(Item::Message));
    let mut out = Vec::new();
    let (mut page_messages, mut page_tombstones, mut bytes) = (Vec::new(), Vec::new(), 0usize);
    for item in items {
        let size = match &item {
            Item::Message(m) => postcard::to_stdvec(m).map_or(usize::MAX, |b| b.len()),
            Item::Tombstone(p) => postcard::to_stdvec(p).map_or(usize::MAX, |b| b.len()),
        };
        if size > FRAME_BUDGET {
            tracing::warn!("catch-up item of {size} bytes is too large to send");
            continue;
        }
        let count = page_messages.len() + page_tombstones.len();
        if count == PAGE_SIZE || bytes + size > FRAME_BUDGET {
            out.push(CatchupFrame::Push {
                messages: std::mem::take(&mut page_messages),
                tombstones: std::mem::take(&mut page_tombstones),
            });
            bytes = 0;
        }
        bytes += size;
        match item {
            Item::Message(m) => page_messages.push(m),
            Item::Tombstone(p) => page_tombstones.push(p),
        }
    }
    if !page_messages.is_empty() || !page_tombstones.is_empty() {
        out.push(CatchupFrame::Push {
            messages: page_messages,
            tombstones: page_tombstones,
        });
    }
    out
}

/// Upper bound of one bucket's encoded size in an `Items` frame.
fn items_bound(entry: &BucketItems) -> usize {
    32 + entry.1.len() * 32
}

/// Drops the oldest buckets until the list fits one frame. Only a vast
/// difference needs this; the dropped buckets wait for a later session.
fn fit_items(mut list: Vec<BucketItems>) -> Vec<BucketItems> {
    let mut total: usize = list.iter().map(items_bound).sum();
    if total <= FRAME_BUDGET {
        return list;
    }
    list.sort_by_key(|(key, _)| std::cmp::Reverse(key.day()));
    let before = list.len();
    while total > FRAME_BUDGET {
        let Some(entry) = list.pop() else { break };
        total -= items_bound(&entry);
    }
    tracing::warn!(
        "catch-up left {} buckets for a later session",
        before - list.len()
    );
    list
}

fn log_capped(what: &str, dropped: usize) {
    if dropped > 0 {
        tracing::info!("catch-up over the cap: {dropped} items left to {what} in a later session");
    }
}

async fn send<W: AsyncWrite + Unpin>(io: &mut W, frame: &CatchupFrame) -> Result<(), CatchupError> {
    write_frame(io, frame).await?;
    io.flush().await.map_err(MediaError::from)?;
    Ok(())
}

async fn read<R: AsyncRead + Unpin>(io: &mut R) -> Result<CatchupFrame, CatchupError> {
    match tokio::time::timeout(FRAME_TIMEOUT, read_frame(io)).await {
        Err(_) => Err(CatchupError::Timeout),
        Ok(Ok(Some(frame))) => Ok(frame),
        Ok(Ok(None)) => Err(CatchupError::Closed),
        Ok(Err(e)) => Err(e.into()),
    }
}

/// Tells the peer why we stop. Best effort: it may be gone already.
async fn refuse<W: AsyncWrite + Unpin>(io: &mut W, refusal: Refusal) -> CatchupError {
    let _ = send(io, &refusal.frame()).await;
    CatchupError::Rejected(refusal)
}

fn unexpected(frame: CatchupFrame) -> CatchupError {
    match frame {
        CatchupFrame::Refused {
            reason,
            retry_after_ms,
        } => CatchupError::Refused {
            reason,
            retry_after_ms,
        },
        _ => CatchupError::Unexpected,
    }
}

fn dedup(ids: impl IntoIterator<Item = MessageId>) -> Vec<MessageId> {
    let mut seen = HashSet::new();
    ids.into_iter().filter(|id| seen.insert(*id)).collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        fixture, identity_for, join, keyed_profile, post, post_at, MemStore,
    };
    use chrono::{DateTime, Utc};
    use commy_core::{ChannelId, Snapshot};

    const NODE_A: [u8; 32] = [0xa; 32];
    const NODE_B: [u8; 32] = [0xb; 32];
    const MINUTE_MS: i64 = 60_000;

    /// Two replicas of one server: `a` is the owner's device, `b` a friend's.
    fn pair() -> (MemStore, MemStore, ChannelId) {
        let (mut state, channel, owner) = fixture();
        let friend = join(&mut state, "friend");
        (
            MemStore::new(state.clone(), owner),
            MemStore::new(state, friend),
            channel,
        )
    }

    type Outcome = (
        Result<SessionReport, CatchupError>,
        Result<(ServerId, SessionReport), CatchupError>,
    );

    /// Runs one session with `a` dialing `b` over an in-memory pipe.
    async fn session(a: &MemStore, b: &MemStore) -> Outcome {
        let (x, y) = tokio::io::duplex(8 * 1024);
        tokio::join!(
            run_initiator(x, a, &a.identity, a.server(), a.me, NODE_A, NODE_B),
            run_responder(y, b, NODE_B, NODE_A),
        )
    }

    async fn ok_session(a: &MemStore, b: &MemStore) -> (SessionReport, SessionReport) {
        let (ra, rb) = session(a, b).await;
        let (server, rb) = rb.expect("responder");
        assert_eq!(server, a.server());
        (ra.expect("initiator"), rb)
    }

    fn sorted(mut ids: Vec<MessageId>) -> Vec<MessageId> {
        ids.sort();
        ids
    }

    fn day_ago(days: i64) -> DateTime<Utc> {
        Utc::now() - chrono::Duration::days(days)
    }

    #[tokio::test]
    async fn missing_messages_flow_both_ways() {
        let (a, b, channel) = pair();
        let shared = a.post(channel, "shared");
        b.add(&shared);
        let only_a = a.post(channel, "only a");
        let only_b = b.post(channel, "only b");

        let (ra, rb) = ok_session(&a, &b).await;
        for store in [&a, &b] {
            assert!(store.has(shared.id) && store.has(only_a.id) && store.has(only_b.id));
        }
        assert_eq!(a.digests(), b.digests());

        assert_eq!(ra.pushed, vec![only_a.id]);
        assert_eq!(ra.received, vec![only_b.id]);
        assert_eq!(sorted(ra.peer_had), sorted(vec![shared.id]));
        assert_eq!(rb.pushed, vec![only_b.id]);
        assert_eq!(rb.received, vec![only_a.id]);
        assert_eq!(sorted(rb.peer_had), sorted(vec![shared.id]));
        assert_eq!((ra.held, rb.held), (0, 0));
    }

    #[tokio::test]
    async fn in_sync_peers_move_nothing() {
        let (a, b, channel) = pair();
        let old = post_at(
            &a.with(|s| s.clone()),
            channel,
            a.me,
            "last week",
            day_ago(7),
        );
        let new = a.post(channel, "today");
        a.add(&old);
        b.add(&old);
        b.add(&new);

        let (ra, rb) = ok_session(&a, &b).await;
        assert!(ra.pushed.is_empty() && ra.received.is_empty());
        assert!(rb.pushed.is_empty() && rb.received.is_empty());
        // Both sides know the other holds everything, without listing items.
        assert_eq!(sorted(ra.peer_had), sorted(vec![old.id, new.id]));
        assert_eq!(sorted(rb.peer_had), sorted(vec![old.id, new.id]));
    }

    #[tokio::test]
    async fn empty_replicas_finish_cleanly() {
        let (a, b, _) = pair();
        let (ra, rb) = ok_session(&a, &b).await;
        assert_eq!(ra, SessionReport::default());
        assert_eq!(rb, SessionReport::default());
    }

    #[tokio::test]
    async fn edits_flow_both_ways() {
        let (a, b, channel) = pair();
        let mine = a.post(channel, "draft by owner");
        let theirs = b.post(channel, "draft by friend");
        a.add(&theirs);
        b.add(&mine);
        a.with(|s| {
            let edit = mine.edit_event(
                s.server.id,
                "final by owner",
                Utc::now(),
                &identity_for(mine.author),
            );
            s.apply(edit).unwrap()
        });
        b.with(|s| {
            let edit = theirs.edit_event(
                s.server.id,
                "final by friend",
                Utc::now(),
                &identity_for(theirs.author),
            );
            s.apply(edit).unwrap()
        });

        let (ra, rb) = ok_session(&a, &b).await;
        for store in [&a, &b] {
            assert_eq!(store.content(mine.id).as_deref(), Some("final by owner"));
            assert_eq!(store.content(theirs.id).as_deref(), Some("final by friend"));
        }
        assert_eq!(a.digests(), b.digests());
        assert_eq!(ra.pushed, vec![mine.id]);
        assert_eq!(ra.received, vec![theirs.id]);
        assert_eq!(rb.received, vec![mine.id]);
        // Each side's older copy was superseded by the peer's newer one,
        // so the peer counts as holding it; nothing else was shared.
        assert_eq!(ra.peer_had, vec![theirs.id]);
        assert_eq!(rb.peer_had, vec![mine.id]);
    }

    #[tokio::test]
    async fn deletes_flow_both_ways() {
        let (a, b, channel) = pair();
        let by_owner = a.post(channel, "owner regrets this");
        let by_friend = b.post(channel, "friend crosses a line");
        a.add(&by_friend);
        b.add(&by_owner);
        // The owner deletes their own message on a; on b the owner, a
        // moderator, deletes the friend's message from that device.
        a.with(|s| {
            let proof = TombstoneProof::sign(
                s.server.id,
                channel,
                by_owner.id,
                a.me,
                Utc::now(),
                &identity_for(a.me),
            );
            s.apply(proof.into_event()).unwrap()
        });
        b.with(|s| {
            let proof = TombstoneProof::sign(
                s.server.id,
                channel,
                by_friend.id,
                a.me,
                Utc::now(),
                &identity_for(a.me),
            );
            s.apply(proof.into_event()).unwrap()
        });

        let (ra, rb) = ok_session(&a, &b).await;
        for store in [&a, &b] {
            assert!(!store.has(by_owner.id) && !store.has(by_friend.id));
            assert!(store.with(
                |s| s.tombstone(by_owner.id).is_some() && s.tombstone(by_friend.id).is_some()
            ));
        }
        assert_eq!(a.digests(), b.digests());
        assert!(ra.pushed.contains(&by_owner.id) && ra.received.contains(&by_friend.id));
        assert!(rb.pushed.contains(&by_friend.id) && rb.received.contains(&by_owner.id));
    }

    #[tokio::test]
    async fn late_copy_loses_to_a_delete_in_either_direction() {
        for a_dials in [true, false] {
            let (a, b, channel) = pair();
            let doomed = b.post(channel, "gone soon");
            a.add(&doomed);
            // b deletes it; a still holds the message.
            b.with(|s| {
                let proof = TombstoneProof::sign(
                    s.server.id,
                    channel,
                    doomed.id,
                    b.me,
                    Utc::now(),
                    &identity_for(b.me),
                );
                s.apply(proof.into_event()).unwrap()
            });
            if a_dials {
                ok_session(&a, &b).await;
            } else {
                ok_session(&b, &a).await;
            }
            assert!(
                !a.has(doomed.id) && !b.has(doomed.id),
                "a_dials = {a_dials}"
            );
            assert_eq!(a.digests(), b.digests());
        }
    }

    #[tokio::test]
    async fn unknown_author_is_held_not_applied() {
        let (a, b, channel) = pair();
        // A newcomer joined on a only; b has not seen their join yet.
        let newcomer = a.with(|s| join(s, "newcomer"));
        let early = a.post_as(newcomer, channel, "hi all");

        let (ra, rb) = ok_session(&a, &b).await;
        assert_eq!(ra.pushed, vec![early.id]);
        assert_eq!(rb.held, 1);
        assert!(rb.received.is_empty());
        assert!(!b.has(early.id));
        let held = b.held.lock().unwrap().clone();
        assert_eq!(
            held,
            vec![ServerEvent::MessageCreate {
                message: early.clone()
            }]
        );

        // Once the join arrives, the held event applies.
        let profile = a.with(|s| s.profiles[&newcomer].clone());
        b.with(|s| {
            s.apply(ServerEvent::MemberJoin {
                server: s.server.id,
                profile,
            })
            .unwrap();
            for event in held {
                assert_eq!(s.apply(event), Ok(true));
            }
        });
        assert!(b.has(early.id));
    }

    #[tokio::test]
    async fn many_messages_travel_in_pages() {
        let (a, b, channel) = pair();
        let count = 2 * PAGE_SIZE + 50;
        let base = a.with(|s| s.clone());
        let list: Vec<Message> = (0..count)
            .map(|i| {
                post_at(
                    &base,
                    channel,
                    a.me,
                    &format!("m{i}"),
                    Utc::now() - chrono::Duration::seconds(i as i64),
                )
            })
            .collect();
        let mut sorted_list = list.clone();
        sorted_list.sort_by_key(|m| m.created_at);
        a.with(|s| *s = with_messages(s, channel, sorted_list));

        let (ra, rb) = ok_session(&a, &b).await;
        assert_eq!(ra.pushed.len(), count);
        assert_eq!(rb.received.len(), count);
        assert!(list.iter().all(|m| b.has(m.id)));
        assert_eq!(a.digests(), b.digests());
    }

    /// `state` with its channel's messages replaced, loaded the way a
    /// saved server is (fast, no per-message checks).
    fn with_messages(state: &ServerState, channel: ChannelId, list: Vec<Message>) -> ServerState {
        let mut snapshot: Snapshot = state.snapshot();
        snapshot.messages.retain(|(c, _)| *c != channel);
        snapshot.messages.push((channel, list));
        ServerState::from_snapshot(snapshot)
    }

    /// A store holding `recent` signed messages from today plus `old`
    /// ones from three weeks ago, and an empty peer.
    fn overfull(recent: usize, old: usize) -> (MemStore, MemStore, Vec<Message>, Vec<Message>) {
        let (a, b, channel) = pair();
        let base = a.with(|s| s.clone());
        let old_list: Vec<Message> = (0..old)
            .map(|i| post_at(&base, channel, a.me, &format!("old {i}"), day_ago(21)))
            .collect();
        let recent_list: Vec<Message> = (0..recent)
            .map(|i| {
                post_at(
                    &base,
                    channel,
                    a.me,
                    &format!("new {i}"),
                    Utc::now() - chrono::Duration::milliseconds(i as i64),
                )
            })
            .collect();
        let mut all: Vec<Message> = old_list.iter().chain(&recent_list).cloned().collect();
        all.sort_by_key(|m| m.created_at);
        a.with(|s| *s = with_messages(s, channel, all));
        (a, b, recent_list, old_list)
    }

    #[tokio::test]
    async fn pushes_are_capped_newest_first() {
        let (a, b, recent, old) = overfull(CATCHUP_MAX_ITEMS, 3);
        let (ra, rb) = ok_session(&a, &b).await;
        assert_eq!(ra.pushed.len(), CATCHUP_MAX_ITEMS);
        assert_eq!(rb.received.len(), CATCHUP_MAX_ITEMS);
        assert!(recent.iter().all(|m| b.has(m.id)));
        assert!(
            old.iter().all(|m| !b.has(m.id)),
            "the oldest wait for the next session"
        );
        // The next session moves the rest.
        let (ra, _) = ok_session(&a, &b).await;
        assert_eq!(
            sorted(ra.pushed),
            sorted(old.iter().map(|m| m.id).collect())
        );
        assert_eq!(a.digests(), b.digests());
    }

    #[tokio::test]
    async fn wants_are_capped_newest_first() {
        let (a, b, recent, old) = overfull(CATCHUP_MAX_ITEMS, 3);
        // The empty side dials and pulls.
        let (rb, ra) = ok_session(&b, &a).await;
        assert_eq!(rb.received.len(), CATCHUP_MAX_ITEMS);
        assert_eq!(ra.pushed.len(), CATCHUP_MAX_ITEMS);
        assert!(recent.iter().all(|m| b.has(m.id)));
        assert!(old.iter().all(|m| !b.has(m.id)));
        // A cut-short want list proves nothing about what the peer holds.
        assert!(ra.peer_had.is_empty());
    }

    #[tokio::test]
    async fn unknown_member_is_asked_to_retry() {
        let (a, b, _) = pair();
        // Someone who joined through a, before b heard of them.
        let newcomer = a.with(|s| join(s, "newcomer"));
        let c = MemStore::new(a.with(|s| s.clone()), newcomer);
        let (rc, rb) = session(&c, &b).await;
        let err = rc.unwrap_err();
        assert_eq!(err.retry_after_ms(), Some(RETRY_AFTER_MS));
        assert!(
            matches!(err, CatchupError::Refused { ref reason, .. } if reason == "unknown member")
        );
        assert!(matches!(
            rb.unwrap_err(),
            CatchupError::Rejected(Refusal::UnknownMember)
        ));
    }

    #[tokio::test]
    async fn banned_member_is_refused_for_good() {
        let (a, b, _) = pair();
        b.with(|s| s.server.banned.push(a.me));
        let (ra, rb) = session(&a, &b).await;
        let err = ra.unwrap_err();
        assert_eq!(err.retry_after_ms(), None);
        assert!(matches!(err, CatchupError::Refused { .. }));
        assert!(matches!(
            rb.unwrap_err(),
            CatchupError::Rejected(Refusal::Denied)
        ));
    }

    #[tokio::test]
    async fn key_other_than_the_pinned_one_is_refused() {
        let (a, b, _) = pair();
        let mut imposter = MemStore::new(a.with(|s| s.clone()), a.me);
        imposter.identity = Identity::generate();
        let (ra, rb) = session(&imposter, &b).await;
        assert!(matches!(
            ra.unwrap_err(),
            CatchupError::Refused {
                retry_after_ms: None,
                ..
            }
        ));
        assert!(matches!(
            rb.unwrap_err(),
            CatchupError::Rejected(Refusal::Denied)
        ));
    }

    #[tokio::test]
    async fn responder_outside_the_server_refuses() {
        let (a, _, _) = pair();
        let (other, _, _) = fixture();
        let stranger_id = other.server.owner;
        let stranger = MemStore::new(other, stranger_id);
        let (ra, rb) = session(&a, &stranger).await;
        // Told apart only locally; the peer hears what a stranger would.
        let err = ra.unwrap_err();
        assert_eq!(err.to_string(), "peer refused catch-up: unknown member");
        assert_eq!(err.retry_after_ms(), Some(RETRY_AFTER_MS));
        assert!(matches!(
            rb.unwrap_err(),
            CatchupError::Rejected(Refusal::NotInServer)
        ));
    }

    #[tokio::test]
    async fn initiator_turns_down_a_responder_it_does_not_know() {
        let (mut state, channel, owner) = fixture();
        let a = MemStore::new(state.clone(), owner);
        let friend = join(&mut state, "friend");
        let b = MemStore::new(state, friend);
        let only_b = b.post(channel, "from b");
        let (ra, rb) = session(&a, &b).await;
        assert!(matches!(
            ra.unwrap_err(),
            CatchupError::Rejected(Refusal::UnknownMember)
        ));
        let err = rb.unwrap_err();
        assert_eq!(err.retry_after_ms(), Some(RETRY_AFTER_MS));
        assert!(!a.has(only_b.id));
    }

    #[tokio::test]
    async fn hello_made_for_other_nodes_is_refused() {
        let (a, b, _) = pair();
        // A Hello signed for a session with some other node, replayed to b.
        let (x, y) = tokio::io::duplex(8 * 1024);
        let (ra, rb) = tokio::join!(
            run_initiator(x, &a, &a.identity, a.server(), a.me, NODE_A, [0xc; 32]),
            run_responder(y, &b, NODE_B, NODE_A),
        );
        assert!(matches!(
            ra.unwrap_err(),
            CatchupError::Refused {
                retry_after_ms: None,
                ..
            }
        ));
        assert!(matches!(
            rb.unwrap_err(),
            CatchupError::Rejected(Refusal::BadSignature)
        ));

        // Same Hello, but b sees it arrive from a different node.
        let (x, y) = tokio::io::duplex(8 * 1024);
        let (_, rb) = tokio::join!(
            run_initiator(x, &a, &a.identity, a.server(), a.me, NODE_A, NODE_B),
            run_responder(y, &b, NODE_B, [0xc; 32]),
        );
        assert!(matches!(
            rb.unwrap_err(),
            CatchupError::Rejected(Refusal::BadSignature)
        ));
    }

    #[tokio::test]
    async fn responder_hello_for_other_nodes_is_refused() {
        let (a, b, _) = pair();
        let (x, mut y) = tokio::io::duplex(8 * 1024);
        let server = a.server();
        let fake_responder = async {
            let _hello = read(&mut y).await.unwrap();
            // Signed as if a third node had dialed b.
            let reply = CatchupFrame::hello(&b.identity, server, b.me, [0xc; 32], NODE_B, now_ms());
            send(&mut y, &reply).await.unwrap();
            read(&mut y).await.unwrap()
        };
        let (ra, last) = tokio::join!(
            run_initiator(x, &a, &a.identity, server, a.me, NODE_A, NODE_B),
            fake_responder,
        );
        assert!(matches!(
            ra.unwrap_err(),
            CatchupError::Rejected(Refusal::BadSignature)
        ));
        assert_eq!(last, Refusal::BadSignature.frame());
    }

    /// Sends a hand-made Hello to a responder for `b` and returns both outcomes.
    async fn hello_to(
        b: &MemStore,
        hello: CatchupFrame,
    ) -> (
        CatchupFrame,
        Result<(ServerId, SessionReport), CatchupError>,
    ) {
        let (mut x, y) = tokio::io::duplex(8 * 1024);
        // Owns its end, so hanging up after the reply ends the responder too.
        let initiator = async move {
            send(&mut x, &hello).await.unwrap();
            read(&mut x).await.unwrap()
        };
        tokio::join!(initiator, run_responder(y, b, NODE_B, NODE_A))
    }

    #[tokio::test]
    async fn stale_or_future_hello_is_refused() {
        let (a, b, _) = pair();
        for offset in [-11 * MINUTE_MS, 11 * MINUTE_MS] {
            let hello = CatchupFrame::hello(
                &a.identity,
                a.server(),
                a.me,
                NODE_A,
                NODE_B,
                now_ms() + offset,
            );
            let (reply, rb) = hello_to(&b, hello).await;
            assert_eq!(reply, Refusal::ClockSkew.frame());
            assert!(matches!(
                rb.unwrap_err(),
                CatchupError::Rejected(Refusal::ClockSkew)
            ));
        }
    }

    #[tokio::test]
    async fn hello_within_the_skew_is_answered() {
        let (a, b, _) = pair();
        for offset in [-9 * MINUTE_MS, 9 * MINUTE_MS] {
            let hello = CatchupFrame::hello(
                &a.identity,
                a.server(),
                a.me,
                NODE_A,
                NODE_B,
                now_ms() + offset,
            );
            let (reply, rb) = hello_to(&b, hello).await;
            let CatchupFrame::Hello {
                user,
                identity,
                server,
                ..
            } = reply
            else {
                panic!("expected a Hello, got {reply:?}");
            };
            assert_eq!(
                (server, user, identity),
                (a.server(), b.me, b.identity.public())
            );
            assert!(matches!(rb.unwrap_err(), CatchupError::Closed));
        }
    }

    #[tokio::test]
    async fn tampered_hello_is_refused() {
        let (a, b, _) = pair();
        let mut hello =
            CatchupFrame::hello(&a.identity, a.server(), a.me, NODE_A, NODE_B, now_ms());
        if let CatchupFrame::Hello { ts_ms, .. } = &mut hello {
            *ts_ms += 1;
        }
        let (reply, _) = hello_to(&b, hello).await;
        assert_eq!(reply, Refusal::BadSignature.frame());
    }

    #[tokio::test]
    async fn responder_wants_a_hello_first() {
        let (_, b, _) = pair();
        let (reply, rb) = {
            let (mut x, y) = tokio::io::duplex(1024);
            let initiator = async {
                send(&mut x, &CatchupFrame::Done).await.unwrap();
                read(&mut x).await
            };
            tokio::join!(initiator, run_responder(y, &b, NODE_B, NODE_A))
        };
        assert!(matches!(rb.unwrap_err(), CatchupError::Unexpected));
        assert!(matches!(reply.unwrap_err(), CatchupError::Closed));
    }

    #[tokio::test]
    async fn hang_up_before_hello_is_closed() {
        let (_, b, _) = pair();
        let (x, y) = tokio::io::duplex(1024);
        drop(x);
        assert!(matches!(
            run_responder(y, &b, NODE_B, NODE_A).await.unwrap_err(),
            CatchupError::Closed
        ));
    }

    #[tokio::test]
    async fn initiator_ignores_items_it_did_not_want() {
        let (a, b, channel) = pair();
        let shared = a.post(channel, "shared");
        b.add(&shared);
        let unasked = b.with(|s| post(s, channel, b.me, "never offered"));
        let server = a.server();
        let (x, mut y) = tokio::io::duplex(8 * 1024);
        let fake_responder = async {
            let _hello = read(&mut y).await.unwrap();
            send(
                &mut y,
                &CatchupFrame::hello(&b.identity, server, b.me, NODE_A, NODE_B, now_ms()),
            )
            .await
            .unwrap();
            let _digests = read(&mut y).await.unwrap();
            send(&mut y, &CatchupFrame::Items(vec![])).await.unwrap();
            assert_eq!(read(&mut y).await.unwrap(), CatchupFrame::Want(vec![]));
            assert_eq!(read(&mut y).await.unwrap(), CatchupFrame::Done);
            let push = CatchupFrame::Push {
                messages: vec![unasked.clone()],
                tombstones: vec![],
            };
            send(&mut y, &push).await.unwrap();
            send(&mut y, &CatchupFrame::Done).await.unwrap();
        };
        let (ra, ()) = tokio::join!(
            run_initiator(x, &a, &a.identity, server, a.me, NODE_A, NODE_B),
            fake_responder
        );
        let ra = ra.unwrap();
        assert!(ra.received.is_empty());
        assert!(!a.has(unasked.id));
        // Nothing was answered, so every bucket counts as in sync.
        assert_eq!(ra.peer_had, vec![shared.id]);
    }

    #[tokio::test]
    async fn responder_ignores_items_outside_the_buckets_that_differ() {
        let (a, b, channel) = pair();
        let shared = a.post(channel, "shared");
        b.add(&shared);
        let old = a.with(|s| post_at(s, channel, a.me, "from another day", day_ago(3)));
        let server = a.server();
        let ours = a.digests();
        let (mut x, y) = tokio::io::duplex(8 * 1024);
        let fake_initiator = async {
            send(
                &mut x,
                &CatchupFrame::hello(&a.identity, server, a.me, NODE_A, NODE_B, now_ms()),
            )
            .await
            .unwrap();
            assert!(matches!(
                read(&mut x).await.unwrap(),
                CatchupFrame::Hello { .. }
            ));
            send(&mut x, &CatchupFrame::Digests(ours)).await.unwrap();
            assert_eq!(read(&mut x).await.unwrap(), CatchupFrame::Items(vec![]));
            send(&mut x, &CatchupFrame::Want(vec![shared.id]))
                .await
                .unwrap();
            send(
                &mut x,
                &CatchupFrame::Push {
                    messages: vec![old.clone()],
                    tombstones: vec![],
                },
            )
            .await
            .unwrap();
            send(&mut x, &CatchupFrame::Done).await.unwrap();
            read(&mut x).await.unwrap()
        };
        let (last, rb) = tokio::join!(fake_initiator, run_responder(y, &b, NODE_B, NODE_A));
        let (_, rb) = rb.unwrap();
        assert!(rb.received.is_empty());
        assert!(!b.has(old.id));
        // A want outside the offered items is not served.
        assert_eq!(last, CatchupFrame::Done);
        assert!(rb.pushed.is_empty());
    }

    #[tokio::test]
    async fn endless_pushes_end_the_session() {
        let (_, b, _) = pair();
        let (mut x, mut y) = tokio::io::duplex(64 * 1024);
        let flood = async {
            for _ in 0..=2 * CATCHUP_MAX_ITEMS {
                if send(
                    &mut x,
                    &CatchupFrame::Push {
                        messages: vec![],
                        tombstones: vec![],
                    },
                )
                .await
                .is_err()
                {
                    break;
                }
            }
        };
        let accept_all = |_: MessageId, _: BucketKey| true;
        let (_, result) = tokio::join!(flood, receive_pages(&mut y, &b, b.server(), accept_all));
        assert!(matches!(result.unwrap_err(), CatchupError::TooMuch));
    }

    #[tokio::test]
    async fn refusal_mid_session_surfaces_with_its_retry_hint() {
        let (_, b, _) = pair();
        let (mut x, mut y) = tokio::io::duplex(1024);
        send(&mut x, &Refusal::Busy.frame()).await.unwrap();
        let err = receive_pages(&mut y, &b, b.server(), |_, _| true)
            .await
            .unwrap_err();
        assert_eq!(err.retry_after_ms(), Some(RETRY_AFTER_MS));
        assert_eq!(err.to_string(), "peer refused catch-up: busy");
    }

    #[test]
    fn hello_bytes_follow_the_spec() {
        let (server, user) = (ServerId::new(), UserId::new());
        let expected =
            postcard::to_stdvec(&("commy-catchup/v1", server, user, NODE_A, NODE_B, 1234i64))
                .unwrap();
        assert_eq!(hello_bytes(server, user, NODE_A, NODE_B, 1234), expected);
        assert_ne!(
            hello_bytes(server, user, NODE_B, NODE_A, 1234),
            expected,
            "node order matters"
        );
    }

    #[test]
    fn frames_roundtrip_through_postcard() {
        let (state, channel, owner) = fixture();
        let m = post(&state, channel, owner, "hi");
        let proof = TombstoneProof::sign(
            state.server.id,
            channel,
            m.id,
            owner,
            Utc::now(),
            &identity_for(owner),
        );
        let key = BucketKey::Messages {
            channel,
            day: 20_000,
        };
        let frames = vec![
            CatchupFrame::hello(
                &identity_for(owner),
                state.server.id,
                owner,
                NODE_A,
                NODE_B,
                now_ms(),
            ),
            Refusal::UnknownMember.frame(),
            CatchupFrame::Digests(vec![BucketDigest {
                key,
                count: 1,
                hash: [7; 16],
            }]),
            CatchupFrame::Items(vec![(key, vec![(m.id, 5)])]),
            CatchupFrame::Want(vec![m.id]),
            CatchupFrame::Push {
                messages: vec![m],
                tombstones: vec![proof],
            },
            CatchupFrame::Done,
        ];
        for frame in frames {
            let bytes = postcard::to_stdvec(&frame).unwrap();
            assert_eq!(postcard::from_bytes::<CatchupFrame>(&bytes).unwrap(), frame);
        }
    }

    #[test]
    fn variant_tags_are_stable() {
        let tag = |frame: &CatchupFrame| postcard::to_stdvec(frame).unwrap()[0];
        assert_eq!(tag(&Refusal::Busy.frame()), 1);
        assert_eq!(tag(&CatchupFrame::Digests(vec![])), 2);
        assert_eq!(tag(&CatchupFrame::Items(vec![])), 3);
        assert_eq!(tag(&CatchupFrame::Want(vec![])), 4);
        assert_eq!(
            tag(&CatchupFrame::Push {
                messages: vec![],
                tombstones: vec![]
            }),
            5
        );
        assert_eq!(tag(&CatchupFrame::Done), 6);
    }

    #[test]
    fn pages_hold_at_most_page_size_items() {
        let (state, channel, owner) = fixture();
        let messages: Vec<Message> = (0..2 * PAGE_SIZE + 10)
            .map(|i| post(&state, channel, owner, &format!("m{i}")))
            .collect();
        let proofs: Vec<TombstoneProof> = (0..5)
            .map(|_| {
                TombstoneProof::sign(
                    state.server.id,
                    channel,
                    MessageId::new(),
                    owner,
                    Utc::now(),
                    &identity_for(owner),
                )
            })
            .collect();
        let out = pages(messages.clone(), proofs.clone());
        let sizes: Vec<(usize, usize)> = out
            .iter()
            .map(|p| match p {
                CatchupFrame::Push {
                    messages,
                    tombstones,
                } => (messages.len(), tombstones.len()),
                other => panic!("not a page: {other:?}"),
            })
            .collect();
        assert_eq!(sizes, vec![(PAGE_SIZE - 5, 5), (PAGE_SIZE, 0), (15, 0)]);
        // Order is kept: proofs first, then messages as given.
        let flat: Vec<MessageId> = out
            .iter()
            .flat_map(|p| match p {
                CatchupFrame::Push {
                    messages,
                    tombstones,
                } => tombstones
                    .iter()
                    .map(|t| t.message)
                    .chain(messages.iter().map(|m| m.id))
                    .collect::<Vec<_>>(),
                _ => vec![],
            })
            .collect();
        let expected: Vec<MessageId> = proofs
            .iter()
            .map(|p| p.message)
            .chain(messages.iter().map(|m| m.id))
            .collect();
        assert_eq!(flat, expected);
        assert!(pages(vec![], vec![]).is_empty());
    }

    #[tokio::test]
    async fn pages_split_by_size_and_fit_a_frame() {
        let (state, channel, owner) = fixture();
        let big = "x".repeat(700 * 1024);
        let messages: Vec<Message> = (0..12)
            .map(|_| post(&state, channel, owner, &big))
            .collect();
        let out = pages(messages.clone(), vec![]);
        assert!(out.len() >= 3, "{} pages", out.len());
        let (mut x, mut y) = tokio::io::duplex(64 * 1024);
        let writer = async {
            for page in &out {
                write_frame(&mut x, page).await.unwrap();
            }
            drop(x);
        };
        let reader = async {
            let mut got = Vec::new();
            while let Some(CatchupFrame::Push { messages, .. }) =
                read_frame::<_, CatchupFrame>(&mut y).await.unwrap()
            {
                got.extend(messages);
            }
            got
        };
        let ((), got) = tokio::join!(writer, reader);
        assert_eq!(got, messages);
    }

    #[test]
    fn oversized_item_is_left_out() {
        let (state, channel, owner) = fixture();
        let huge = post(
            &state,
            channel,
            owner,
            &"x".repeat(MAX_MEDIA_FRAME as usize),
        );
        let small = post(&state, channel, owner, "fits");
        let out = pages(vec![huge, small.clone()], vec![]);
        assert_eq!(
            out,
            vec![CatchupFrame::Push {
                messages: vec![small],
                tombstones: vec![]
            }]
        );
    }

    #[test]
    fn fit_items_drops_the_oldest_buckets_only_when_needed() {
        let channel = ChannelId::new();
        let bucket = |day: u32, n: usize| -> BucketItems {
            (
                BucketKey::Messages { channel, day },
                (0..n).map(|_| (MessageId::new(), 0)).collect(),
            )
        };
        let small = vec![bucket(1, 3), bucket(2, 3)];
        assert_eq!(fit_items(small.clone()), small);

        let per_bucket = FRAME_BUDGET / 32 / 3;
        let big = vec![
            bucket(5, per_bucket),
            bucket(9, per_bucket),
            bucket(7, per_bucket),
            bucket(6, per_bucket),
        ];
        let kept = fit_items(big);
        let days: Vec<u32> = kept.iter().map(|(k, _)| k.day()).collect();
        assert_eq!(days, vec![9, 7]);
        assert!(kept.iter().map(items_bound).sum::<usize>() <= FRAME_BUDGET);
        let bytes = postcard::to_stdvec(&CatchupFrame::Items(kept)).unwrap();
        assert!(bytes.len() <= MAX_MEDIA_FRAME as usize);
    }

    #[test]
    fn check_peer_covers_each_standing() {
        let (mut state, _, owner) = fixture();
        let friend = join(&mut state, "friend");
        let key = |user| identity_for(user).public();
        assert_eq!(check_peer(&state, friend, &key(friend)), PeerCheck::Allowed);
        assert_eq!(check_peer(&state, owner, &key(owner)), PeerCheck::Allowed);
        assert_eq!(
            check_peer(&state, friend, &Identity::generate().public()),
            PeerCheck::Denied
        );
        let stranger = keyed_profile("stranger");
        assert_eq!(
            check_peer(&state, stranger.id, &key(stranger.id)),
            PeerCheck::Unknown
        );

        // A member whose profile has no key yet.
        let keyless = UserId::new();
        state.server.members.push(commy_core::Member {
            user: keyless,
            nickname: None,
            roles: vec![],
            joined_at: Utc::now(),
        });
        assert_eq!(
            check_peer(&state, keyless, &key(keyless)),
            PeerCheck::Unknown
        );

        state.server.banned.push(friend);
        assert_eq!(check_peer(&state, friend, &key(friend)), PeerCheck::Denied);
    }

    #[test]
    fn collect_returns_what_we_hold_in_request_order() {
        let (mut state, channel, owner) = fixture();
        let first = post(&state, channel, owner, "first");
        let second = post(&state, channel, owner, "second");
        let gone = post(&state, channel, owner, "gone");
        for m in [&first, &second, &gone] {
            state
                .apply(ServerEvent::MessageCreate { message: m.clone() })
                .unwrap();
        }
        let proof = TombstoneProof::sign(
            state.server.id,
            channel,
            gone.id,
            owner,
            Utc::now(),
            &identity_for(owner),
        );
        state.apply(proof.clone().into_event()).unwrap();
        let unknown = MessageId::new();
        let (messages, tombstones) =
            collect(&state, &[second.id, unknown, gone.id, first.id, second.id]);
        let ids: Vec<MessageId> = messages.iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![second.id, first.id]);
        assert_eq!(tombstones, vec![proof]);
    }

    #[test]
    fn apply_items_reports_applied_duplicates_held_and_rejected() {
        let (mut state, channel, owner) = fixture();
        let source = state.clone();
        let fresh = post(&source, channel, owner, "fresh");
        let dup = post(&source, channel, owner, "dup");
        state
            .apply(ServerEvent::MessageCreate {
                message: dup.clone(),
            })
            .unwrap();
        let mut forged = post(&source, channel, owner, "forged");
        forged.content = "changed after signing".into();
        let stranger = keyed_profile("stranger");
        let from_stranger = post(&source, channel, stranger.id, "who am i");
        let proof = TombstoneProof::sign(
            source.server.id,
            channel,
            dup.id,
            owner,
            Utc::now(),
            &identity_for(owner),
        );

        let (report, held) = apply_items(
            &mut state,
            vec![
                fresh.clone(),
                dup.clone(),
                forged.clone(),
                from_stranger.clone(),
            ],
            vec![proof],
        );
        // The proof removes `dup` first, so its late copy is refused.
        assert_eq!(report.applied, vec![dup.id, fresh.id]);
        assert_eq!(report.held, 1);
        assert_eq!(
            held,
            vec![ServerEvent::MessageCreate {
                message: from_stranger
            }]
        );
        assert!(state.has_message(fresh.id));
        assert!(!state.has_message(dup.id) && !state.has_message(forged.id));
    }
}
