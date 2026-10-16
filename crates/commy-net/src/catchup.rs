//! Catch-up sessions over a direct stream (ALPN `commy/catchup/1`).
//!
//! A member that was offline dials a gossip neighbor and the two swap
//! the recent history one of them lacks: signed messages, edits and
//! delete proofs, with the signers' profiles. The reconciliation itself
//! is `commy_core::catchup`; this module carries it over the wire and is
//! generic over the stream, so it runs the same over QUIC and over an
//! in-memory pipe in tests.
//!
//! A session, initiator first:
//! 1. `Hello` both ways, each signed over both node ids and its role.
//! 2. The initiator sends `Digests`; the responder answers `Items` pages
//!    for the buckets that differ, then `ItemsEnd` naming any bucket of
//!    the initiator's it left out.
//! 3. The initiator sends `Want`, then `Push` pages of what the
//!    responder lacks, each after a `Profiles` frame with the signers it
//!    names first, then `Done`.
//! 4. The responder applies them and answers `Applied`, then sends what
//!    was wanted the same way, then `Done`.
//! 5. The initiator applies them and answers `Applied`.

use crate::media::{read_frame, write_frame, MediaError, MAX_MEDIA_FRAME};
use commy_core::catchup::{
    bucket_digest, cap, current_window, day_number, diff, mismatched, BucketItems, Listing,
    CATCHUP_MAX_ITEMS,
};
use commy_core::{
    BucketDigest, BucketKey, Identity, IdentityKey, Message, MessageId, Presence, ServerEvent,
    ServerId, ServerState, Signature, TombstoneProof, UserId, UserProfile,
};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::hash::Hash;
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const CATCHUP_ALPN: &[u8] = b"commy/catchup/1";

/// Most items (messages plus delete proofs) in one `Push` frame, and
/// most profiles in one `Profiles` frame.
pub const PAGE_SIZE: usize = 200;

/// Largest clock difference a `Hello` may show, in ms.
pub const MAX_CLOCK_SKEW_MS: u64 = 10 * 60 * 1000;

/// Delay suggested to a peer that may succeed soon, in ms.
pub const RETRY_AFTER_MS: u64 = 3000;

/// Longest wait for the peer's next frame.
pub const FRAME_TIMEOUT: Duration = Duration::from_secs(30);

/// Longest a responder waits for the initiator's `Hello`.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Longest a responder waits for the initiator's `Applied` after its
/// own `Done`. Without it, nothing it sent counts as delivered. The
/// initiator may still be applying pages sent earlier, so no less than
/// [`FRAME_TIMEOUT`].
pub const APPLIED_TIMEOUT: Duration = FRAME_TIMEOUT;

/// Most bytes of profiles a peer may have waiting for the page they
/// precede.
pub const MAX_PROFILE_BYTES: usize = 2 * 1024 * 1024;

/// Profiles a peer may send beyond the signers of the pages it sent so
/// far: those of the one page they precede.
pub const PROFILE_MARGIN: usize = PAGE_SIZE;

/// Most items (messages plus delete proofs) one side takes in a session,
/// each page counting at least one. A sender stops adding ids before
/// its items pass this; the rest wait for a later session.
pub const MAX_PUSHED_ITEMS: usize = 2 * CATCHUP_MAX_ITEMS;

/// Largest profile sent along or taken: what one gossip `ProfileUpdate`
/// carries, the same as for a message (see the protocol tests). A full
/// page of signers so stays well within [`MAX_PROFILE_BYTES`].
pub const MAX_RELAYED_PROFILE_BYTES: usize = commy_core::MAX_MESSAGE_BYTES;

/// Most bucket digests a peer may send.
pub const MAX_DIGESTS: usize = 50_000;

/// Most entries the responder's `Items` may list in all, each bucket
/// counting as one more than its items.
pub const MAX_LISTED: usize = 500_000;

/// Largest `Hello` read from a peer not checked yet.
const MAX_HELLO_BYTES: u32 = 4096;

/// Bytes a frame may fill, leaving room for its own framing.
const FRAME_BUDGET: usize = MAX_MEDIA_FRAME as usize - 1024;

/// Upper bounds of a bucket key and of one item in an `Items` frame.
const KEY_BOUND: usize = 32;
const ITEM_BOUND: usize = 32;

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
    /// One page of the responder's items of the buckets that differ. A
    /// large bucket continues on the next page.
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
    /// End of the responder's `Items`. `omitted` names buckets of the
    /// initiator's digests it did not compare, which so prove nothing.
    ItemsEnd { omitted: Vec<BucketKey> },
    /// Profiles of signers named by the next `Push` page and by no
    /// earlier one, so a peer that has not met them yet can check it.
    Profiles(Vec<UserProfile>),
    /// What became of the other side's pages: ids set aside until their
    /// signer's key is known, and ids refused or ignored. The peer holds
    /// the rest now.
    Applied {
        held: Vec<MessageId>,
        rejected: Vec<MessageId>,
    },
}

/// Which side of a session a `Hello` speaks for. It is signed too, so a
/// Hello sent back to the side that made it does not pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelloRole {
    Initiator,
    Responder,
}

impl HelloRole {
    fn domain(self) -> &'static str {
        match self {
            HelloRole::Initiator => "commy-catchup/v1/initiator",
            HelloRole::Responder => "commy-catchup/v1/responder",
        }
    }
}

/// Bytes a `Hello` signs. Both sides list the initiator's node first,
/// so a Hello only counts between those two nodes and in its role.
pub fn hello_bytes(
    role: HelloRole,
    server: ServerId,
    user: UserId,
    initiator: [u8; 32],
    responder: [u8; 32],
    ts_ms: i64,
) -> Vec<u8> {
    postcard::to_stdvec(&(role.domain(), server, user, initiator, responder, ts_ms))
        .expect("postcard encodes plain tuples")
}

impl CatchupFrame {
    /// A `Hello` from `user` in `role` for a session between two nodes.
    pub fn hello(
        identity: &Identity,
        role: HelloRole,
        server: ServerId,
        user: UserId,
        initiator: [u8; 32],
        responder: [u8; 32],
        ts_ms: i64,
    ) -> Self {
        let sig = identity.sign(&hello_bytes(
            role, server, user, initiator, responder, ts_ms,
        ));
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
    /// Ids set aside until their signer's key is known.
    pub held: Vec<MessageId>,
    /// Ids refused, e.g. for a bad signature or an unknown channel.
    pub rejected: Vec<MessageId>,
}

/// What a session moved, from one side's point of view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionReport {
    /// Ids of ours the peer now holds: what it confirmed taking from us
    /// in this session, plus `peer_had`. Pending sends among them are done.
    pub delivered: Vec<MessageId>,
    /// Ids of ours the peer already held at the same or a newer version.
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
    /// The signature is invalid or was made for other node ids or role.
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

    /// The `Refused` frame telling a peer about this refusal. Every
    /// reason that depends on who the peer is or which servers we are in
    /// reads as an unknown member, so a refusal tells a stranger nothing.
    /// The precise reason stays in our own logs.
    pub fn frame(self) -> CatchupFrame {
        let shown = match self {
            Refusal::NotInServer | Refusal::Denied | Refusal::BadSignature => {
                Refusal::UnknownMember
            }
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
    /// Keyed profiles of current members among `users`, see [`profiles_of`].
    fn profiles(&self, server: ServerId, users: &[UserId]) -> Vec<UserProfile>;
    /// Takes the profiles a peer sent along with a page, before that
    /// page is applied, see [`apply_profiles`]. Held items whose signer
    /// became known may be retried afterwards.
    fn apply_profiles(&self, server: ServerId, profiles: Vec<UserProfile>);
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

/// [`CatchupStore::get`] over one server's state, in the order of `ids`,
/// with every delete proof held for each id. Proofs for channels that no
/// longer exist are left out.
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
    let mut tombstones: Vec<&TombstoneProof> = order
        .keys()
        .flat_map(|id| state.tombstones_for(*id))
        .filter(|p| state.server.channel(p.channel).is_some())
        .collect();
    // Stable, so each id's proofs stay best first.
    tombstones.sort_by_key(|p| order[&p.message]);
    (
        messages.into_iter().cloned().collect(),
        tombstones.into_iter().cloned().collect(),
    )
}

/// [`CatchupStore::profiles`] over one server's state: the profiles of
/// `users` that are members now and carry a key, once each, in the order
/// asked.
pub fn profiles_of(state: &ServerState, users: &[UserId]) -> Vec<UserProfile> {
    dedup(users.iter().copied())
        .into_iter()
        .filter(|user| state.server.is_member(*user))
        .filter_map(|user| state.profiles.get(&user))
        .filter(|p| p.identity.is_some())
        .cloned()
        .collect()
}

/// [`CatchupStore::apply_profiles`] over one server's state. Each
/// profile whose key we lack pins that key and is stored, unless the user
/// is banned. Membership is left alone: a relayed profile never makes
/// anyone a member, nor brings back one who left or was kicked. Profiles
/// of users whose key we know already are left alone too, so a relayed
/// copy never replaces a fresher one. Returns the users whose key we
/// learned.
pub fn apply_profiles(state: &mut ServerState, profiles: Vec<UserProfile>) -> Vec<UserId> {
    let mut learned = Vec::new();
    for mut profile in profiles {
        let user = profile.id;
        if profile.identity.is_none()
            || state.identity_of(user).is_some()
            || state.server.banned.contains(&user)
        {
            continue;
        }
        // Presence comes live from the user, not from a relayed copy.
        profile.presence = state
            .profiles
            .get(&user)
            .map_or(Presence::Offline, |p| p.presence);
        state.profiles.insert(user, profile);
        learned.push(user);
    }
    learned
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
            report.held.push(id);
            held.push(event);
            continue;
        }
        match state.apply(event) {
            Ok(true) => report.applied.push(id),
            Ok(false) => {}
            Err(e) => {
                tracing::debug!("catch-up item {id} rejected: {e}");
                report.rejected.push(id);
            }
        }
    }
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
    let hello = CatchupFrame::hello(
        identity,
        HelloRole::Initiator,
        server,
        me,
        local_node,
        remote_node,
        now_ms(),
    );
    send(&mut io, &hello).await?;
    let reply = read(&mut io).await?;
    let CatchupFrame::Hello { server: s, .. } = reply else {
        return Err(unexpected(reply));
    };
    if s != server {
        return Err(CatchupError::Unexpected);
    }
    let checked = check_signed(&reply, HelloRole::Responder, local_node, remote_node)
        .and_then(|(server, user, key)| check_standing(store, server, user, &key));
    if let Err(refusal) = checked {
        return Err(refuse(&mut io, refusal).await);
    }

    let ours = fit_digests(store.digests(server));
    let sent: HashMap<BucketKey, BucketDigest> = ours.iter().map(|d| (d.key, *d)).collect();
    send(&mut io, &CatchupFrame::Digests(ours)).await?;
    let (theirs, omitted) = read_items(&mut io).await?;

    // A bucket of ours matched only if the responder neither listed it
    // nor left it out.
    let window = current_window();
    let answered: HashSet<BucketKey> = theirs.iter().map(|(key, _)| *key).collect();
    let theirs: Vec<BucketItems> = theirs
        .into_iter()
        .filter(|(key, _)| window.contains(&key.day()))
        .collect();
    let listed: HashSet<BucketKey> = theirs.iter().map(|(key, _)| *key).collect();
    let matched = sent
        .keys()
        .filter(|key| !answered.contains(key) && !omitted.contains(key));
    let mut keys: Vec<BucketKey> = matched.chain(&listed).copied().collect();
    keys.sort_unstable();
    keys.dedup();
    let (mine, settled): (Vec<BucketItems>, Vec<BucketItems>) = store
        .items(server, &keys)
        .into_iter()
        .partition(|(key, _)| listed.contains(key));
    let (mut they_lack, mut we_want) = diff(&mine, &theirs);
    log_capped("send", cap(&mut they_lack));
    log_capped("fetch", cap(&mut we_want));

    send(&mut io, &CatchupFrame::Want(we_want.clone())).await?;
    // The responder takes only items of the buckets it listed.
    let pushed = send_items(&mut io, store, server, &they_lack, |key| listed.contains(&key)).await?;
    let verdict = read_applied(&mut io).await?;

    let wanted: HashSet<MessageId> = we_want.into_iter().collect();
    let got = receive_pages(&mut io, store, server, |id, _| wanted.contains(&id)).await?;
    // Our pages are in; the peer hanging up first loses only its count.
    if let Err(e) = send(&mut io, &got.verdict()).await {
        tracing::debug!("catch-up peer left before our Applied: {e}");
    }

    let pushed_set: HashSet<MessageId> = pushed.iter().copied().collect();
    let peer_had: Vec<MessageId> = initiator_peer_had(&settled, &sent, &mine, &theirs)
        .into_iter()
        .filter(|id| !pushed_set.contains(id))
        .collect();
    Ok(SessionReport {
        delivered: delivered(&pushed, Some(&verdict), &peer_had),
        peer_had,
        received: got.applied,
        held: got.held.len(),
    })
}

/// Runs a session as the side that was dialed: [`admit`] within
/// [`HANDSHAKE_TIMEOUT`], then [`respond`]. The server comes from the
/// initiator's Hello and is returned with the report.
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
    let handshake = admit(&mut io, store, local_node, remote_node);
    let admitted = tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake)
        .await
        .map_err(|_| CatchupError::Timeout)??;
    respond(io, store, admitted, local_node, remote_node).await
}

/// An initiator whose Hello passed [`admit`].
pub struct Admitted {
    /// The server the session is about.
    pub server: ServerId,
    me: UserId,
    identity: Identity,
}

/// Reads the initiator's Hello and checks it: the signature and the
/// clock first, which need nothing but the Hello, then our place in that
/// server and theirs. A Hello that does not pass is refused. Not timed;
/// callers bound it, see [`HANDSHAKE_TIMEOUT`].
pub async fn admit<IO, S>(
    io: &mut IO,
    store: &S,
    local_node: [u8; 32],
    remote_node: [u8; 32],
) -> Result<Admitted, CatchupError>
where
    IO: AsyncRead + AsyncWrite + Unpin,
    S: CatchupStore + ?Sized,
{
    let hello = read_hello(io).await?;
    if !matches!(hello, CatchupFrame::Hello { .. }) {
        return Err(unexpected(hello));
    }
    let checked = check_signed(&hello, HelloRole::Initiator, remote_node, local_node).and_then(
        |(server, user, key)| {
            let local = store.local_identity(server).ok_or(Refusal::NotInServer)?;
            check_standing(store, server, user, &key)?;
            Ok((server, local))
        },
    );
    match checked {
        Ok((server, (me, identity))) => Ok(Admitted {
            server,
            me,
            identity,
        }),
        Err(refusal) => Err(refuse(io, refusal).await),
    }
}

/// Runs the rest of a responder session for an initiator that
/// [`admit`] let in.
pub async fn respond<IO, S>(
    mut io: IO,
    store: &S,
    admitted: Admitted,
    local_node: [u8; 32],
    remote_node: [u8; 32],
) -> Result<(ServerId, SessionReport), CatchupError>
where
    IO: AsyncRead + AsyncWrite + Unpin,
    S: CatchupStore + ?Sized,
{
    let Admitted {
        server,
        me,
        identity,
    } = admitted;
    let reply = CatchupFrame::hello(
        &identity,
        HelloRole::Responder,
        server,
        me,
        remote_node,
        local_node,
        now_ms(),
    );
    send(&mut io, &reply).await?;

    let theirs = match read(&mut io).await? {
        CatchupFrame::Digests(list) => list,
        other => return Err(unexpected(other)),
    };
    if theirs.len() > MAX_DIGESTS {
        return Err(CatchupError::TooMuch);
    }
    let window = current_window();
    let (theirs, outside): (Vec<BucketDigest>, Vec<BucketDigest>) = theirs
        .into_iter()
        .partition(|d| window.contains(&d.key.day()));
    let their_keys: HashSet<BucketKey> = theirs.iter().map(|d| d.key).collect();
    let ours = store.digests(server);
    let differ: HashSet<BucketKey> = mismatched(&ours, &theirs).into_iter().collect();
    let matched: HashMap<BucketKey, BucketDigest> = ours
        .iter()
        .filter(|d| !differ.contains(&d.key))
        .map(|d| (d.key, *d))
        .collect();
    let mut keys: Vec<BucketKey> = differ.iter().chain(matched.keys()).copied().collect();
    keys.sort_unstable();
    let (answer, settled): (Vec<BucketItems>, Vec<BucketItems>) = store
        .items(server, &keys)
        .into_iter()
        .partition(|(key, _)| differ.contains(key));
    let (answer, dropped) = fit_listing(answer, MAX_LISTED);
    let omitted: Vec<BucketKey> = outside
        .iter()
        .map(|d| d.key)
        .chain(dropped.into_iter().filter(|key| their_keys.contains(key)))
        .collect();
    let answered: HashSet<BucketKey> = answer.iter().map(|(key, _)| *key).collect();
    let offered: HashSet<MessageId> = ids_of(&answer).collect();
    // Not wanting an item proves holding it only in a bucket the
    // initiator has a digest for, so it surely compared that bucket.
    let shown: Vec<MessageId> = answer
        .iter()
        .filter(|(key, _)| their_keys.contains(key))
        .flat_map(|(_, list)| list.iter().map(|(id, _)| *id))
        .collect();
    for page in item_pages(answer) {
        write_frame(&mut io, &CatchupFrame::Items(page)).await?;
    }
    send(&mut io, &CatchupFrame::ItemsEnd { omitted }).await?;

    let want = match read(&mut io).await? {
        CatchupFrame::Want(ids) => ids,
        other => return Err(unexpected(other)),
    };
    // A full list may have been cut short, so it says nothing about the rest.
    let want_complete = want.len() < CATCHUP_MAX_ITEMS;
    let want_all: HashSet<MessageId> = want.iter().copied().collect();
    let mut want: Vec<MessageId> = want.into_iter().filter(|id| offered.contains(id)).collect();
    log_capped("send", cap(&mut want));

    let got = receive_pages(&mut io, store, server, |_, key| answered.contains(&key)).await?;
    send(&mut io, &got.verdict()).await?;
    let pushed = send_items(&mut io, store, server, &want, |_| true).await?;
    let verdict = match tokio::time::timeout(APPLIED_TIMEOUT, read_applied(&mut io)).await {
        Ok(Ok(verdict)) => Some(verdict),
        Ok(Err(e)) => {
            tracing::debug!("catch-up peer did not confirm our pages: {e}");
            None
        }
        Err(_) => {
            tracing::debug!("catch-up peer did not confirm our pages in time");
            None
        }
    };

    let mut known = verified(&settled, &matched);
    if want_complete {
        known.extend(shown);
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
            delivered: delivered(&pushed, verdict.as_ref(), &peer_had),
            peer_had,
            received: got.applied,
            held: got.held.len(),
        },
    ))
}

/// Checks what a Hello proves on its own: a signature in `role` over
/// this session's node ids, made recently. Returns who it claims to be.
fn check_signed(
    hello: &CatchupFrame,
    role: HelloRole,
    initiator: [u8; 32],
    responder: [u8; 32],
) -> Result<(ServerId, UserId, IdentityKey), Refusal> {
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
    let signed = hello_bytes(role, *server, *user, initiator, responder, *ts_ms);
    if !identity.verify(&signed, sig) {
        return Err(Refusal::BadSignature);
    }
    if ts_ms.abs_diff(now_ms()) > MAX_CLOCK_SKEW_MS {
        return Err(Refusal::ClockSkew);
    }
    Ok((*server, *user, *identity))
}

/// Checks a peer's standing in our copy of the server.
fn check_standing<S: CatchupStore + ?Sized>(
    store: &S,
    server: ServerId,
    user: UserId,
    key: &IdentityKey,
) -> Result<(), Refusal> {
    match store.verify_peer(server, user, key) {
        PeerCheck::Allowed => Ok(()),
        PeerCheck::Unknown => Err(Refusal::UnknownMember),
        PeerCheck::Denied => Err(Refusal::Denied),
    }
}

/// Ids of buckets whose items, read after the digests were made, still
/// match those digests. A bucket that changed in between proves nothing.
fn verified(settled: &[BucketItems], digests: &HashMap<BucketKey, BucketDigest>) -> Vec<MessageId> {
    settled
        .iter()
        .filter(|(key, list)| digests.get(key) == Some(&bucket_digest(*key, list)))
        .flat_map(|(_, list)| list.iter().map(|(id, _)| *id))
        .collect()
}

/// Ids of ours the responder holds at no lower version: the buckets that
/// matched, and whatever its item lists show for the rest. An id with a
/// delete proof the responder lacks does not count.
fn initiator_peer_had(
    settled: &[BucketItems],
    sent: &HashMap<BucketKey, BucketDigest>,
    mine: &[BucketItems],
    theirs: &[BucketItems],
) -> Vec<MessageId> {
    let listing = Listing::new(theirs);
    let (mut confirmed, mut lacked) = (Vec::new(), HashSet::new());
    for (key, list) in mine {
        for &(id, version) in list {
            if listing.holds(*key, id, version) {
                confirmed.push(id);
            } else {
                lacked.insert(id);
            }
        }
    }
    dedup(
        verified(settled, sent)
            .into_iter()
            .chain(confirmed)
            .filter(|id| !lacked.contains(id)),
    )
}

/// What the peer holds of ours: what we pushed minus what its `Applied`
/// says it held back or refused, plus what it had already. Nothing
/// pushed counts without an `Applied`.
fn delivered(
    pushed: &[MessageId],
    verdict: Option<&(Vec<MessageId>, Vec<MessageId>)>,
    peer_had: &[MessageId],
) -> Vec<MessageId> {
    let taken = verdict.map(|(held, rejected)| {
        let missing: HashSet<&MessageId> = held.iter().chain(rejected).collect();
        pushed.iter().filter(move |id| !missing.contains(id))
    });
    dedup(taken.into_iter().flatten().chain(peer_had).copied())
}

/// What became of the peer's pages.
#[derive(Debug, Default)]
struct Received {
    applied: Vec<MessageId>,
    held: Vec<MessageId>,
    rejected: Vec<MessageId>,
}

impl Received {
    /// The `Applied` frame telling the peer what became of its pages.
    fn verdict(&self) -> CatchupFrame {
        CatchupFrame::Applied {
            held: self.held.clone(),
            rejected: self.rejected.clone(),
        }
    }
}

/// Reads `Profiles` and `Push` pages until `Done`, applying the items
/// inside the window that `accept` lets through, each page after the
/// profiles of its signers. Items left out count as rejected.
///
/// Profiles apply to the page right after them and are dropped once it
/// is in; one larger than [`MAX_RELAYED_PROFILE_BYTES`] is ignored. A
/// peer that sends more than [`PROFILE_MARGIN`] profiles beyond the
/// signers its pages named so far, or has more than [`MAX_PROFILE_BYTES`]
/// of them waiting, ends the session.
async fn receive_pages<IO, S>(
    io: &mut IO,
    store: &S,
    server: ServerId,
    accept: impl Fn(MessageId, BucketKey) -> bool,
) -> Result<Received, CatchupError>
where
    IO: AsyncRead + Unpin,
    S: CatchupStore + ?Sized,
{
    // Empty pages count too.
    let mut budget = MAX_PUSHED_ITEMS;
    let mut waiting: HashMap<UserId, (UserProfile, usize)> = HashMap::new();
    let mut waiting_bytes = 0usize;
    let mut profiles_sent = 0usize;
    let mut signers: HashSet<UserId> = HashSet::new();
    let mut out = Received::default();
    let window = current_window();
    let fits = |id: MessageId, key: BucketKey| window.contains(&key.day()) && accept(id, key);
    loop {
        match read(io).await? {
            CatchupFrame::Profiles(list) => {
                profiles_sent += list.len().max(1);
                if profiles_sent > signers.len() + PROFILE_MARGIN {
                    return Err(CatchupError::TooMuch);
                }
                for profile in list.into_iter().filter(|p| p.identity.is_some()) {
                    let size = encoded_len(&profile);
                    if size > MAX_RELAYED_PROFILE_BYTES {
                        tracing::debug!("catch-up profile of {} is too large to take", profile.id);
                        continue;
                    }
                    if let Some((_, old)) = waiting.insert(profile.id, (profile, size)) {
                        waiting_bytes -= old;
                    }
                    waiting_bytes += size;
                }
                if waiting_bytes > MAX_PROFILE_BYTES {
                    return Err(CatchupError::TooMuch);
                }
            }
            CatchupFrame::Push {
                messages,
                tombstones,
            } => {
                let size = messages.len() + tombstones.len();
                budget = budget
                    .checked_sub(size.max(1))
                    .ok_or(CatchupError::TooMuch)?;
                signers.extend(
                    tombstones
                        .iter()
                        .map(|p| p.by)
                        .chain(messages.iter().map(|m| m.author)),
                );
                let (messages, refused): (Vec<Message>, Vec<Message>) = messages
                    .into_iter()
                    .partition(|m| fits(m.id, message_key(m)));
                out.rejected.extend(refused.iter().map(|m| m.id));
                let (tombstones, refused): (Vec<TombstoneProof>, Vec<TombstoneProof>) = tombstones
                    .into_iter()
                    .partition(|p| fits(p.message, tombstone_key(p)));
                out.rejected.extend(refused.iter().map(|p| p.message));
                let profiles: Vec<UserProfile> = dedup(
                    tombstones
                        .iter()
                        .map(|p| p.by)
                        .chain(messages.iter().map(|m| m.author)),
                )
                .into_iter()
                .filter_map(|user| waiting.remove(&user).map(|(p, _)| p))
                .collect();
                // What this page did not use goes.
                waiting.clear();
                waiting_bytes = 0;
                if messages.is_empty() && tombstones.is_empty() {
                    continue;
                }
                if !profiles.is_empty() {
                    store.apply_profiles(server, profiles);
                }
                let report = store.apply(server, messages, tombstones);
                out.applied.extend(report.applied);
                out.held.extend(report.held);
                out.rejected.extend(report.rejected);
            }
            CatchupFrame::Done => break,
            other => return Err(unexpected(other)),
        }
    }
    if !out.rejected.is_empty() {
        tracing::debug!(
            "catch-up left out {} items nobody asked for or we refused",
            out.rejected.len()
        );
    }
    Ok(Received {
        applied: dedup(out.applied),
        held: dedup(out.held),
        rejected: dedup(out.rejected),
    })
}

fn encoded_len<T: Serialize>(value: &T) -> usize {
    postcard::to_stdvec(value).map_or(usize::MAX, |b| b.len())
}

fn message_key(m: &Message) -> BucketKey {
    BucketKey::Messages {
        channel: m.channel,
        day: day_number(m.created_at),
    }
}

fn tombstone_key(p: &TombstoneProof) -> BucketKey {
    BucketKey::Tombstones {
        day: day_number(p.at),
    }
}

/// Reads the peer's `Applied` as (held, rejected).
async fn read_applied<R: AsyncRead + Unpin>(
    io: &mut R,
) -> Result<(Vec<MessageId>, Vec<MessageId>), CatchupError> {
    match read(io).await? {
        CatchupFrame::Applied { held, rejected } => Ok((held, rejected)),
        other => Err(unexpected(other)),
    }
}

/// Reads the responder's `Items` pages up to `ItemsEnd`. Returns its
/// listing, one entry per bucket, and the buckets it left out.
async fn read_items<R: AsyncRead + Unpin>(
    io: &mut R,
) -> Result<(Vec<BucketItems>, HashSet<BucketKey>), CatchupError> {
    let mut budget = MAX_LISTED;
    let mut listed: BTreeMap<BucketKey, Vec<(MessageId, i64)>> = BTreeMap::new();
    loop {
        match read(io).await? {
            CatchupFrame::Items(page) => {
                let mut cost = 0;
                for (key, list) in page {
                    if !listed.contains_key(&key) {
                        cost += 1;
                    }
                    cost += list.len();
                    listed.entry(key).or_default().extend(list);
                }
                budget = budget
                    .checked_sub(cost.max(1))
                    .ok_or(CatchupError::TooMuch)?;
            }
            CatchupFrame::ItemsEnd { omitted } => {
                return Ok((listed.into_iter().collect(), omitted.into_iter().collect()));
            }
            other => return Err(unexpected(other)),
        }
    }
}

/// Sends what we hold among `ids` in the buckets `keep` lets through as
/// `Push` pages, each after a `Profiles` frame with the keyed profiles
/// of the signers it names first, then `Done`. Stays within
/// [`MAX_PUSHED_ITEMS`], see [`fit_items`]. Returns the ids sent.
async fn send_items<IO, S>(
    io: &mut IO,
    store: &S,
    server: ServerId,
    ids: &[MessageId],
    keep: impl Fn(BucketKey) -> bool,
) -> Result<Vec<MessageId>, CatchupError>
where
    IO: AsyncWrite + Unpin,
    S: CatchupStore + ?Sized,
{
    let (mut messages, mut tombstones) = if ids.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        store.get(server, ids)
    };
    messages.retain(|m| keep(message_key(m)));
    tombstones.retain(|p| keep(tombstone_key(p)));
    let (messages, tombstones) = fit_items(ids, messages, tombstones, MAX_PUSHED_ITEMS);
    let signers = dedup(
        tombstones
            .iter()
            .map(|p| p.by)
            .chain(messages.iter().map(|m| m.author)),
    );
    let mut profiles: HashMap<UserId, UserProfile> = if signers.is_empty() {
        HashMap::new()
    } else {
        store
            .profiles(server, &signers)
            .into_iter()
            .filter(|p| p.identity.is_some())
            .filter(|p| {
                let fits = encoded_len(p) <= MAX_RELAYED_PROFILE_BYTES;
                if !fits {
                    tracing::debug!("catch-up profile of {} is too large to send", p.id);
                }
                fits
            })
            .map(|p| (p.id, p))
            .collect()
    };
    let mut sent = Vec::new();
    for page in pages(messages, tombstones) {
        let CatchupFrame::Push {
            messages,
            tombstones,
        } = &page
        else {
            continue;
        };
        let ahead: Vec<UserProfile> = dedup(
            tombstones
                .iter()
                .map(|p| p.by)
                .chain(messages.iter().map(|m| m.author)),
        )
        .into_iter()
        .filter_map(|user| profiles.remove(&user))
        .collect();
        for chunk in chunks(ahead) {
            write_frame(io, &CatchupFrame::Profiles(chunk)).await?;
        }
        sent.extend(tombstones.iter().map(|p| p.message));
        sent.extend(messages.iter().map(|m| m.id));
        write_frame(io, &page).await?;
    }
    send(io, &CatchupFrame::Done).await?;
    Ok(dedup(sent))
}

/// Keeps whole ids, in the order of `ids` (newest first), while their
/// messages and delete proofs together stay within `max`. The first id
/// that does not fit and all after it wait for a later session.
fn fit_items(
    ids: &[MessageId],
    mut messages: Vec<Message>,
    mut tombstones: Vec<TombstoneProof>,
    max: usize,
) -> (Vec<Message>, Vec<TombstoneProof>) {
    if messages.len() + tombstones.len() <= max {
        return (messages, tombstones);
    }
    let mut cost: HashMap<MessageId, usize> = HashMap::new();
    for id in messages
        .iter()
        .map(|m| m.id)
        .chain(tombstones.iter().map(|p| p.message))
    {
        *cost.entry(id).or_default() += 1;
    }
    let (mut kept, mut used) = (HashSet::new(), 0usize);
    for id in dedup(ids.iter().copied()) {
        let Some(&n) = cost.get(&id) else { continue };
        if used + n > max {
            break;
        }
        used += n;
        kept.insert(id);
    }
    log_capped("send", cost.len() - kept.len());
    messages.retain(|m| kept.contains(&m.id));
    tombstones.retain(|p| kept.contains(&p.message));
    (messages, tombstones)
}

/// Splits items into `Push` frames of at most [`PAGE_SIZE`] items that
/// each fit in a frame, delete proofs first. An item too large for any
/// frame is left out.
fn pages(messages: Vec<Message>, tombstones: Vec<TombstoneProof>) -> Vec<CatchupFrame> {
    #[derive(Serialize)]
    enum Item {
        Message(Message),
        Tombstone(TombstoneProof),
    }
    let items = tombstones
        .into_iter()
        .map(Item::Tombstone)
        .chain(messages.into_iter().map(Item::Message));
    chunks(items)
        .into_iter()
        .map(|chunk| {
            let (mut messages, mut tombstones) = (Vec::new(), Vec::new());
            for item in chunk {
                match item {
                    Item::Message(m) => messages.push(m),
                    Item::Tombstone(p) => tombstones.push(p),
                }
            }
            CatchupFrame::Push {
                messages,
                tombstones,
            }
        })
        .collect()
}

/// Splits values into runs of at most [`PAGE_SIZE`] that each fit in a
/// frame. A value too large for any frame is left out.
fn chunks<T: Serialize>(values: impl IntoIterator<Item = T>) -> Vec<Vec<T>> {
    let mut out = Vec::new();
    let (mut run, mut bytes) = (Vec::new(), 0usize);
    for value in values {
        let size = postcard::to_stdvec(&value).map_or(usize::MAX, |b| b.len());
        if size > FRAME_BUDGET {
            tracing::warn!("catch-up item of {size} bytes is too large to send");
            continue;
        }
        if run.len() == PAGE_SIZE || bytes + size > FRAME_BUDGET {
            out.push(std::mem::take(&mut run));
            bytes = 0;
        }
        bytes += size;
        run.push(value);
    }
    if !run.is_empty() {
        out.push(run);
    }
    out
}

/// Keeps at most [`MAX_DIGESTS`] digests, the newest days first. The
/// rest wait for a later session.
fn fit_digests(mut list: Vec<BucketDigest>) -> Vec<BucketDigest> {
    if list.len() <= MAX_DIGESTS {
        return list;
    }
    tracing::warn!(
        "catch-up left {} buckets for a later session",
        list.len() - MAX_DIGESTS
    );
    list.sort_by_key(|d| Reverse(d.key.day()));
    list.truncate(MAX_DIGESTS);
    list.sort_unstable_by_key(|d| d.key);
    list
}

/// Keeps a listing within `max` entries, each bucket counting one more
/// than its items. Takes the newest buckets first and leaves out only
/// those that no longer fit, so one oversized bucket holds back no other.
/// Returns what is kept and the keys left out, which wait for a later
/// session.
fn fit_listing(mut list: Vec<BucketItems>, max: usize) -> (Vec<BucketItems>, Vec<BucketKey>) {
    let cost = |entry: &BucketItems| 1 + entry.1.len();
    if list.iter().map(cost).sum::<usize>() <= max {
        return (list, Vec::new());
    }
    list.sort_by_key(|(key, _)| Reverse(key.day()));
    let (mut kept, mut dropped, mut used) = (Vec::new(), Vec::new(), 0usize);
    for entry in list {
        let n = cost(&entry);
        if used + n <= max {
            used += n;
            kept.push(entry);
        } else {
            dropped.push(entry.0);
        }
    }
    tracing::warn!(
        "catch-up left {} buckets for a later session",
        dropped.len()
    );
    kept.sort_unstable_by_key(|(key, _)| *key);
    dropped.sort_unstable();
    (kept, dropped)
}

/// Splits a listing into `Items` pages that each fit in a frame. A
/// bucket too large for one page continues on the next.
fn item_pages(list: Vec<BucketItems>) -> Vec<Vec<BucketItems>> {
    let mut out = Vec::new();
    let (mut page, mut bytes): (Vec<BucketItems>, usize) = (Vec::new(), 0);
    for (key, list) in list {
        let mut rest = list.into_iter().peekable();
        loop {
            if !page.is_empty() && bytes + KEY_BOUND + ITEM_BOUND > FRAME_BUDGET {
                out.push(std::mem::take(&mut page));
                bytes = 0;
            }
            let room = (FRAME_BUDGET - bytes - KEY_BOUND) / ITEM_BOUND;
            let part: Vec<(MessageId, i64)> = rest.by_ref().take(room).collect();
            bytes += KEY_BOUND + part.len() * ITEM_BOUND;
            page.push((key, part));
            if rest.peek().is_none() {
                break;
            }
            out.push(std::mem::take(&mut page));
            bytes = 0;
        }
    }
    if !page.is_empty() {
        out.push(page);
    }
    out
}

fn ids_of(list: &[BucketItems]) -> impl Iterator<Item = MessageId> + '_ {
    list.iter()
        .flat_map(|(_, items)| items.iter().map(|(id, _)| *id))
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

/// Reads the first frame of a session, from a peer not checked yet, so
/// at most [`MAX_HELLO_BYTES`]. Not timed.
async fn read_hello<R: AsyncRead + Unpin>(io: &mut R) -> Result<CatchupFrame, CatchupError> {
    let mut len = [0u8; 4];
    match io.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(CatchupError::Closed)
        }
        Err(e) => return Err(MediaError::from(e).into()),
    }
    let len = u32::from_le_bytes(len);
    if len > MAX_HELLO_BYTES {
        return Err(MediaError::FrameTooLarge(len).into());
    }
    let mut buf = vec![0u8; len as usize];
    io.read_exact(&mut buf).await.map_err(MediaError::from)?;
    postcard::from_bytes(&buf).map_err(|_| MediaError::Malformed.into())
}

/// Tells the peer why we stop. Best effort: it may be gone already.
pub(crate) async fn refuse<W: AsyncWrite + Unpin>(io: &mut W, refusal: Refusal) -> CatchupError {
    tracing::debug!("catch-up peer refused: {refusal}");
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

fn dedup<T: Copy + Eq + Hash>(values: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut seen = HashSet::new();
    values.into_iter().filter(|v| seen.insert(*v)).collect()
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
        fixture, identity_for, join, join_moderator, keyed_profile, post, post_at, proof_at,
        MemStore,
    };
    use chrono::{DateTime, Utc};
    use commy_core::{Channel, ChannelId, ChannelKind, Snapshot};
    use std::sync::Mutex;

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

    /// One session dialed by `a` or by `b`. Returns (a's report, b's).
    async fn either_way(
        a: &MemStore,
        b: &MemStore,
        a_dials: bool,
    ) -> (SessionReport, SessionReport) {
        if a_dials {
            ok_session(a, b).await
        } else {
            let (rb, ra) = ok_session(b, a).await;
            (ra, rb)
        }
    }

    fn sorted(mut ids: Vec<MessageId>) -> Vec<MessageId> {
        ids.sort();
        ids
    }

    fn day_ago(days: i64) -> DateTime<Utc> {
        Utc::now() - chrono::Duration::days(days)
    }

    fn initiator_hello(store: &MemStore, ts_ms: i64) -> CatchupFrame {
        CatchupFrame::hello(
            &store.identity,
            HelloRole::Initiator,
            store.server(),
            store.me,
            NODE_A,
            NODE_B,
            ts_ms,
        )
    }

    fn responder_hello(store: &MemStore) -> CatchupFrame {
        CatchupFrame::hello(
            &store.identity,
            HelloRole::Responder,
            store.server(),
            store.me,
            NODE_A,
            NODE_B,
            now_ms(),
        )
    }

    fn applied_nothing() -> CatchupFrame {
        CatchupFrame::Applied {
            held: vec![],
            rejected: vec![],
        }
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

        assert_eq!(sorted(ra.delivered), sorted(vec![only_a.id, shared.id]));
        assert_eq!(ra.received, vec![only_b.id]);
        assert_eq!(ra.peer_had, vec![shared.id]);
        assert_eq!(sorted(rb.delivered), sorted(vec![only_b.id, shared.id]));
        assert_eq!(rb.received, vec![only_a.id]);
        assert_eq!(rb.peer_had, vec![shared.id]);
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
        assert!(ra.received.is_empty() && rb.received.is_empty());
        // Both sides know the other holds everything, without listing items.
        for report in [ra, rb] {
            assert_eq!(
                sorted(report.peer_had.clone()),
                sorted(vec![old.id, new.id])
            );
            assert_eq!(sorted(report.delivered), sorted(report.peer_had));
        }
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
        assert_eq!(ra.received, vec![theirs.id]);
        assert_eq!(rb.received, vec![mine.id]);
        // Each side's older copy was superseded by the peer's newer one,
        // so the peer counts as holding it; nothing else was shared.
        assert_eq!(ra.peer_had, vec![theirs.id]);
        assert_eq!(rb.peer_had, vec![mine.id]);
        assert_eq!(sorted(ra.delivered), sorted(vec![mine.id, theirs.id]));
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
        assert!(ra.delivered.contains(&by_owner.id) && ra.received.contains(&by_friend.id));
        assert!(rb.delivered.contains(&by_friend.id) && rb.received.contains(&by_owner.id));
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
    async fn every_moderators_delete_reaches_a_replica_that_missed_one() {
        for (days_apart, a_dials) in [(0, true), (0, false), (1, true), (1, false)] {
            let (mut state, channel, owner) = fixture();
            let author = join(&mut state, "author");
            let first = join_moderator(&mut state, "first");
            let second = join_moderator(&mut state, "second");
            let a = MemStore::new(state.clone(), owner);
            let c = MemStore::new(state, author);
            let spam = a.post_as(author, channel, "spam");
            c.add(&spam);
            let now = Utc::now();
            let (p1, p2) = a.with(|s| {
                let p1 = proof_at(s, &spam, first, now - chrono::Duration::days(days_apart));
                (p1, proof_at(s, &spam, second, now))
            });
            a.with(|s| {
                s.apply(p1.clone().into_event()).unwrap();
                s.apply(p2.clone().into_event()).unwrap();
            });
            // c missed the second moderator's delete.
            c.with(|s| s.apply(p1.clone().into_event()).unwrap());
            assert_ne!(a.digests(), c.digests());

            let case = format!("days apart {days_apart}, a dials {a_dials}");
            if a_dials {
                ok_session(&a, &c).await;
            } else {
                ok_session(&c, &a).await;
            }
            let held = |store: &MemStore| {
                let mut by: Vec<UserId> =
                    store.with(|s| s.tombstones_for(spam.id).iter().map(|p| p.by).collect());
                by.sort();
                by
            };
            let mut both = vec![first, second];
            both.sort();
            assert_eq!(held(&c), both, "{case}");
            assert_eq!(held(&a), both, "{case}");
            assert_eq!(a.digests(), c.digests(), "{case}");
            // Nothing is left to move.
            let (ra, rc) = ok_session(&a, &c).await;
            assert!(ra.received.is_empty() && rc.received.is_empty(), "{case}");
            assert!(!a.has(spam.id) && !c.has(spam.id));
        }
    }

    /// A newcomer who posted on `a` and whose key `a` no longer holds, so
    /// nobody can vouch for their messages.
    fn unvouched(a: &MemStore, channel: ChannelId) -> (UserId, Message) {
        let newcomer = a.with(|s| join(s, "newcomer"));
        let early = a.post_as(newcomer, channel, "hi all");
        a.with(|s| s.profiles.get_mut(&newcomer).unwrap().identity = None);
        (newcomer, early)
    }

    #[tokio::test]
    async fn signer_without_a_known_key_is_held_not_applied() {
        let (a, b, channel) = pair();
        let (newcomer, early) = unvouched(&a, channel);

        let (ra, rb) = ok_session(&a, &b).await;
        assert_eq!(rb.held, 1);
        assert!(rb.received.is_empty());
        assert!(ra.delivered.is_empty(), "held is not delivered");
        assert!(!b.has(early.id));
        let held = b.held.lock().unwrap().clone();
        assert_eq!(
            held,
            vec![ServerEvent::MessageCreate {
                message: early.clone()
            }]
        );

        // Once the join arrives, the held event applies.
        let mut profile = a.with(|s| s.profiles[&newcomer].clone());
        profile.identity = Some(identity_for(newcomer).public());
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
    async fn only_what_the_peer_took_counts_as_delivered() {
        let (a, b, channel) = pair();
        let taken = a.post(channel, "taken");
        // A channel whose create has not reached b yet.
        let plans = Channel::new("plans", ChannelKind::Text, 1);
        let plans_id = plans.id;
        a.with(|s| {
            let server = s.server.id;
            s.apply(ServerEvent::ChannelCreate {
                server,
                channel: plans,
            })
            .unwrap()
        });
        let in_plans = a.post(plans_id, "first in plans");
        let (_, early) = unvouched(&a, channel);

        let (ra, rb) = ok_session(&a, &b).await;
        assert_eq!(rb.received, vec![taken.id]);
        assert_eq!(rb.held, 1);
        assert!(!b.has(in_plans.id) && !b.has(early.id));
        assert_eq!(ra.delivered, vec![taken.id]);
        assert!(ra.peer_had.is_empty());
    }

    #[tokio::test]
    async fn a_message_under_another_members_id_never_shuts_out_the_real_one() {
        for a_dials in [true, false] {
            let (mut state, channel, owner) = fixture();
            let friend = join(&mut state, "friend");
            let intruder = join(&mut state, "intruder");
            let a = MemStore::new(state.clone(), owner);
            let b = MemStore::new(state.clone(), friend);
            let real = a.post(channel, "real");
            let mut fake = Message {
                id: real.id,
                created_at: real.created_at,
                ..Message::new(channel, intruder, "fake")
            };
            fake.sign(state.server.id, &identity_for(intruder));
            let case = format!("a dials {a_dials}");

            let event = ServerEvent::MessageCreate {
                message: fake.clone(),
            };
            assert_eq!(
                b.with(|s| s.apply(event)),
                Err(commy_core::StoreError::UnboundMessageId),
                "{case}"
            );
            assert!(!b.has(real.id), "{case}");

            // The intruder's own replica, holding it as if saved, cannot
            // pass it on either.
            let c = MemStore::new(with_messages(&state, channel, vec![fake]), intruder);
            let (rc, rb) = ok_session(&c, &b).await;
            assert!(rc.delivered.is_empty() && rb.received.is_empty(), "{case}");
            assert!(!b.has(real.id), "{case}");

            let (ra, rb) = either_way(&a, &b, a_dials).await;
            assert_eq!(rb.received, vec![real.id], "{case}");
            assert_eq!(ra.delivered, vec![real.id], "{case}");
            assert_eq!(b.content(real.id).as_deref(), Some("real"), "{case}");
            assert_eq!(a.digests(), b.digests(), "{case}");
        }
    }

    #[tokio::test]
    async fn responder_counts_its_pages_only_once_the_initiator_confirms_them() {
        let refused = |id| CatchupFrame::Applied {
            held: vec![],
            rejected: vec![id],
        };
        for (answer, counts) in [
            (None, false),
            (Some(refused as fn(MessageId) -> CatchupFrame), false),
            (
                Some((|_| applied_nothing()) as fn(MessageId) -> CatchupFrame),
                true,
            ),
        ] {
            let (a, b, channel) = pair();
            let only_b = b.post(channel, "only b");
            let server = b.server();
            let (mut x, y) = tokio::io::duplex(64 * 1024);
            let fake_initiator = async {
                send(&mut x, &initiator_hello(&a, now_ms())).await.unwrap();
                assert!(matches!(
                    read(&mut x).await.unwrap(),
                    CatchupFrame::Hello { .. }
                ));
                send(&mut x, &CatchupFrame::Digests(a.digests()))
                    .await
                    .unwrap();
                let (listed, omitted) = read_items(&mut x).await.unwrap();
                assert!(omitted.is_empty());
                let want: Vec<MessageId> = ids_of(&listed).collect();
                assert_eq!(want, vec![only_b.id]);
                send(&mut x, &CatchupFrame::Want(want)).await.unwrap();
                send(&mut x, &CatchupFrame::Done).await.unwrap();
                assert_eq!(read(&mut x).await.unwrap(), applied_nothing());
                let got = receive_pages(&mut x, &a, server, |_, _| true)
                    .await
                    .unwrap();
                assert_eq!(got.applied, vec![only_b.id]);
                // Without an answer, this end just hangs up.
                if let Some(answer) = answer {
                    send(&mut x, &answer(only_b.id)).await.unwrap();
                }
                drop(x);
            };
            let ((), rb) = tokio::join!(fake_initiator, run_responder(y, &b, NODE_B, NODE_A));
            let (_, rb) = rb.unwrap();
            assert_eq!(rb.delivered.contains(&only_b.id), counts, "{answer:?}");
            assert!(rb.peer_had.is_empty());
        }
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
        assert_eq!(ra.delivered.len(), count);
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
        assert_eq!(ra.delivered.len(), CATCHUP_MAX_ITEMS);
        assert_eq!(rb.received.len(), CATCHUP_MAX_ITEMS);
        assert!(recent.iter().all(|m| b.has(m.id)));
        assert!(
            old.iter().all(|m| !b.has(m.id)),
            "the oldest wait for the next session"
        );
        // The next session moves the rest.
        let (ra, rb) = ok_session(&a, &b).await;
        assert_eq!(
            sorted(rb.received),
            sorted(old.iter().map(|m| m.id).collect())
        );
        assert_eq!(ra.delivered.len(), CATCHUP_MAX_ITEMS + old.len());
        assert_eq!(a.digests(), b.digests());
    }

    #[tokio::test]
    async fn wants_are_capped_newest_first() {
        let (a, b, recent, old) = overfull(CATCHUP_MAX_ITEMS, 3);
        // The empty side dials and pulls.
        let (rb, ra) = ok_session(&b, &a).await;
        assert_eq!(rb.received.len(), CATCHUP_MAX_ITEMS);
        assert_eq!(ra.delivered.len(), CATCHUP_MAX_ITEMS);
        assert!(recent.iter().all(|m| b.has(m.id)));
        assert!(old.iter().all(|m| !b.has(m.id)));
        // A cut-short want list proves nothing about what the peer holds.
        assert!(ra.peer_had.is_empty());
    }

    #[tokio::test]
    async fn a_delete_waits_for_the_role_that_allows_it() {
        for a_dials in [true, false] {
            let (mut state, channel, owner) = fixture();
            let author = join(&mut state, "author");
            let moderator = join(&mut state, "moderator");
            // b never got the role that lets the moderator delete.
            let missed = state.clone();
            let server = state.server.id;
            let role = commy_core::Role {
                id: commy_core::RoleId::new(),
                name: "mods".into(),
                color: None,
                permissions: commy_core::Permissions::MANAGE_MESSAGES,
                position: 1,
                hoist: false,
                self_assignable: false,
                tag: None,
                banner: None,
            };
            let rid = role.id;
            let grant = [
                ServerEvent::RoleCreate { server, role },
                ServerEvent::RoleAssign {
                    server,
                    user: moderator,
                    role: rid,
                },
            ];
            for event in &grant {
                assert_eq!(state.apply(event.clone()), Ok(true));
            }
            let a = MemStore::new(state, owner);
            let b = MemStore::new(missed, author);
            let spam = a.post_as(author, channel, "spam");
            b.add(&spam);
            let proof = a.with(|s| proof_at(s, &spam, moderator, Utc::now()));
            a.with(|s| s.apply(proof.clone().into_event()).unwrap());
            let case = format!("a dials {a_dials}");

            let (ra, _) = either_way(&a, &b, a_dials).await;
            // b keeps the proof without applying it, and holds it as far as
            // a can tell.
            assert!(b.has(spam.id), "{case}");
            let kept: Vec<TombstoneProof> =
                b.with(|s| s.tombstones_for(spam.id).into_iter().cloned().collect());
            assert_eq!(kept, vec![proof], "{case}");
            assert!(ra.delivered.contains(&spam.id), "{case}");
            let proofs = |store: &MemStore| -> Vec<BucketDigest> {
                store
                    .digests()
                    .into_iter()
                    .filter(|d| matches!(d.key, BucketKey::Tombstones { .. }))
                    .collect()
            };
            assert_eq!(proofs(&a), proofs(&b), "{case}");
            let (_, rb) = either_way(&a, &b, a_dials).await;
            assert!(rb.received.is_empty(), "{case}");

            // Once the role arrives, the delete applies.
            for event in &grant {
                b.with(|s| s.apply(event.clone()).unwrap());
            }
            assert!(!b.has(spam.id), "{case}");
            assert_eq!(a.digests(), b.digests(), "{case}");
            let (ra, rb) = either_way(&a, &b, a_dials).await;
            assert!(ra.received.is_empty() && rb.received.is_empty(), "{case}");
        }
    }

    /// A store holding `recent` ids deleted today and `old` ids deleted
    /// yesterday, each with `per_id` proofs from the owner and
    /// moderators, and an empty peer. The proofs are spread over enough
    /// signers that none passes `MAX_ORPHAN_PROOFS` on either side.
    fn many_proofs(
        recent: usize,
        old: usize,
        per_id: usize,
    ) -> (MemStore, MemStore, Vec<MessageId>, Vec<MessageId>) {
        let (mut state, channel, owner) = fixture();
        let friend = join(&mut state, "friend");
        let pool = ((recent + old) * per_id)
            .div_ceil(commy_core::store::MAX_ORPHAN_PROOFS)
            .max(per_id);
        let signers: Vec<UserId> = std::iter::once(owner)
            .chain((1..pool).map(|i| join_moderator(&mut state, &format!("mod {i}"))))
            .collect();
        let a = MemStore::new(state.clone(), owner);
        let b = MemStore::new(state, friend);
        let server = a.server();
        let ids = |n: usize| -> Vec<MessageId> { (0..n).map(|_| MessageId::new()).collect() };
        let (recent, old) = (ids(recent), ids(old));
        a.with(|s| {
            let all = recent
                .iter()
                .map(|id| (id, Utc::now()))
                .chain(old.iter().map(|id| (id, day_ago(1))));
            for (k, (id, at)) in all.enumerate() {
                for j in 0..per_id {
                    let by = signers[(k * per_id + j) % pool];
                    let proof = TombstoneProof::sign(server, channel, *id, by, at, &identity_for(by));
                    assert_eq!(s.apply(proof.into_event()), Ok(true));
                }
            }
        });
        (a, b, recent, old)
    }

    #[tokio::test]
    async fn ids_with_several_proofs_stay_within_what_the_peer_takes() {
        const PROOFS: usize = 3;
        // About 3,400 ids, one more page of items than a peer takes.
        let recent = MAX_PUSHED_ITEMS / PROOFS;
        let old = 67;
        assert!((recent + old) * PROOFS > MAX_PUSHED_ITEMS);
        for a_dials in [true, false] {
            let (a, b, recent_ids, old_ids) = many_proofs(recent, old, PROOFS);
            let (ra, rb) = either_way(&a, &b, a_dials).await;
            let case = format!("a dials {a_dials}");
            // The newest ids, each whole, and the report says so.
            assert_eq!(
                sorted(rb.received.clone()),
                sorted(recent_ids.clone()),
                "{case}"
            );
            assert_eq!(sorted(ra.delivered), sorted(recent_ids.clone()), "{case}");
            let proofs_at_b: usize = b.with(|s| {
                recent_ids
                    .iter()
                    .map(|id| s.tombstones_for(*id).len())
                    .sum()
            });
            assert_eq!(proofs_at_b, recent * PROOFS, "{case}");
            assert!(b.with(|s| old_ids.iter().all(|id| s.tombstone(*id).is_none())));

            // The next session sends the rest.
            let (_, rb) = either_way(&a, &b, a_dials).await;
            assert_eq!(sorted(rb.received), sorted(old_ids), "{case}");
            assert_eq!(a.digests(), b.digests(), "{case}");
        }
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
    async fn banned_member_hears_what_a_stranger_would() {
        let (a, b, _) = pair();
        b.with(|s| s.server.banned.push(a.me));
        let (ra, rb) = session(&a, &b).await;
        let err = ra.unwrap_err();
        assert_eq!(err.to_string(), "peer refused catch-up: unknown member");
        assert_eq!(err.retry_after_ms(), Some(RETRY_AFTER_MS));
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
            CatchupError::Refused { ref reason, .. } if reason == "unknown member"
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
    async fn refusals_read_the_same_whoever_asks() {
        let (a, b, _) = pair();
        let server = a.server();
        let (other, _, _) = fixture();
        let other_owner = other.server.owner;
        let outsider = MemStore::new(other, other_owner);
        let banning = MemStore::new(
            b.with(|s| {
                let mut s = s.clone();
                s.server.banned.push(a.me);
                s
            }),
            b.me,
        );
        let hello = |identity: &Identity, user: UserId, ts_ms: i64| {
            CatchupFrame::hello(
                identity,
                HelloRole::Initiator,
                server,
                user,
                NODE_A,
                NODE_B,
                ts_ms,
            )
        };
        let good = hello(&a.identity, a.me, now_ms());
        let mut tampered = good.clone();
        if let CatchupFrame::Hello { ts_ms, .. } = &mut tampered {
            *ts_ms += 1;
        }
        let newcomer = keyed_profile("newcomer");
        let unknown = hello(&identity_for(newcomer.id), newcomer.id, now_ms());
        let imposter = hello(&Identity::generate(), a.me, now_ms());

        let cases = [
            (&b, tampered.clone(), Refusal::BadSignature),
            (&outsider, tampered, Refusal::BadSignature),
            (&b, unknown.clone(), Refusal::UnknownMember),
            (&outsider, unknown, Refusal::NotInServer),
            (&b, imposter.clone(), Refusal::Denied),
            (&outsider, imposter, Refusal::NotInServer),
            (&banning, good.clone(), Refusal::Denied),
            (&outsider, good, Refusal::NotInServer),
        ];
        for (store, hello, why) in cases {
            let (reply, result) = hello_to(store, hello).await;
            // The precise reason stays local.
            assert!(matches!(result, Err(CatchupError::Rejected(r)) if r == why));
            assert_eq!(reply, Refusal::UnknownMember.frame(), "{why}");
        }

        // A clock too far off is told apart, the same in and out of the server.
        let stale = hello(&a.identity, a.me, now_ms() - 11 * MINUTE_MS);
        let (inside, _) = hello_to(&b, stale.clone()).await;
        let (outside, _) = hello_to(&outsider, stale).await;
        assert_eq!(inside, Refusal::ClockSkew.frame());
        assert_eq!(outside, inside);
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
            CatchupError::Refused { ref reason, .. } if reason == "unknown member"
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
            let reply = CatchupFrame::hello(
                &b.identity,
                HelloRole::Responder,
                server,
                b.me,
                [0xc; 32],
                NODE_B,
                now_ms(),
            );
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

    #[tokio::test]
    async fn echoed_hello_is_refused() {
        let (a, _, channel) = pair();
        a.post(channel, "members only");
        let server = a.server();
        let (x, mut y) = tokio::io::duplex(8 * 1024);
        // A peer that knows only the server id sends our own Hello back.
        let reflector = async {
            let hello = read(&mut y).await.unwrap();
            send(&mut y, &hello).await.unwrap();
            let mut seen = Vec::new();
            while let Ok(frame) = read(&mut y).await {
                seen.push(frame);
            }
            seen
        };
        let (ra, seen) = tokio::join!(
            run_initiator(x, &a, &a.identity, server, a.me, NODE_A, NODE_B),
            reflector,
        );
        assert!(matches!(
            ra.unwrap_err(),
            CatchupError::Rejected(Refusal::BadSignature)
        ));
        assert_eq!(
            seen,
            vec![Refusal::BadSignature.frame()],
            "nothing after the refusal"
        );
    }

    #[tokio::test]
    async fn hello_signed_for_the_other_role_is_refused() {
        let (a, b, _) = pair();
        let (reply, rb) = hello_to(&b, responder_hello(&a)).await;
        assert_eq!(reply, Refusal::UnknownMember.frame());
        assert!(matches!(
            rb.unwrap_err(),
            CatchupError::Rejected(Refusal::BadSignature)
        ));
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
            let (reply, rb) = hello_to(&b, initiator_hello(&a, now_ms() + offset)).await;
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
            let (reply, rb) = hello_to(&b, initiator_hello(&a, now_ms() + offset)).await;
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
        let mut hello = initiator_hello(&a, now_ms());
        if let CatchupFrame::Hello { ts_ms, .. } = &mut hello {
            *ts_ms += 1;
        }
        let (reply, rb) = hello_to(&b, hello).await;
        assert_eq!(reply, Refusal::BadSignature.frame());
        assert!(matches!(
            rb.unwrap_err(),
            CatchupError::Rejected(Refusal::BadSignature)
        ));
    }

    #[tokio::test]
    async fn oversized_hello_is_not_read() {
        let (_, b, _) = pair();
        let (mut x, y) = tokio::io::duplex(1024);
        x.write_all(&(MAX_HELLO_BYTES + 1).to_le_bytes())
            .await
            .unwrap();
        let err = run_responder(y, &b, NODE_B, NODE_A).await.unwrap_err();
        assert!(matches!(
            err,
            CatchupError::Frame(MediaError::FrameTooLarge(_))
        ));
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
    async fn initiator_ignores_items_and_profiles_it_did_not_want() {
        let (a, b, channel) = pair();
        let shared = a.post(channel, "shared");
        b.add(&shared);
        let stranger = keyed_profile("stranger");
        let unasked = b.with(|s| post(s, channel, stranger.id, "never offered"));
        let server = a.server();
        let (x, mut y) = tokio::io::duplex(8 * 1024);
        let fake_responder = async {
            let _hello = read(&mut y).await.unwrap();
            send(&mut y, &responder_hello(&b)).await.unwrap();
            let _digests = read(&mut y).await.unwrap();
            send(&mut y, &CatchupFrame::ItemsEnd { omitted: vec![] })
                .await
                .unwrap();
            assert_eq!(read(&mut y).await.unwrap(), CatchupFrame::Want(vec![]));
            assert_eq!(read(&mut y).await.unwrap(), CatchupFrame::Done);
            send(&mut y, &applied_nothing()).await.unwrap();
            send(&mut y, &CatchupFrame::Profiles(vec![stranger.clone()]))
                .await
                .unwrap();
            let push = CatchupFrame::Push {
                messages: vec![unasked.clone()],
                tombstones: vec![],
            };
            send(&mut y, &push).await.unwrap();
            send(&mut y, &CatchupFrame::Done).await.unwrap();
            read(&mut y).await.unwrap()
        };
        let (ra, last) = tokio::join!(
            run_initiator(x, &a, &a.identity, server, a.me, NODE_A, NODE_B),
            fake_responder
        );
        let ra = ra.unwrap();
        assert!(ra.received.is_empty());
        assert!(!a.has(unasked.id));
        assert!(a.with(|s| !s.profiles.contains_key(&stranger.id)));
        // The peer hears that its item was not taken.
        assert_eq!(
            last,
            CatchupFrame::Applied {
                held: vec![],
                rejected: vec![unasked.id]
            }
        );
        // Nothing was listed or left out, so every bucket matched.
        assert_eq!(ra.peer_had, vec![shared.id]);
    }

    #[tokio::test]
    async fn buckets_left_out_prove_nothing() {
        let (a, b, channel) = pair();
        let shared = a.post(channel, "shared");
        b.add(&shared);
        let server = a.server();
        let (x, mut y) = tokio::io::duplex(8 * 1024);
        let fake_responder = async {
            let _hello = read(&mut y).await.unwrap();
            send(&mut y, &responder_hello(&b)).await.unwrap();
            let CatchupFrame::Digests(list) = read(&mut y).await.unwrap() else {
                panic!("expected digests");
            };
            let omitted = list.iter().map(|d| d.key).collect();
            send(&mut y, &CatchupFrame::ItemsEnd { omitted })
                .await
                .unwrap();
            assert_eq!(read(&mut y).await.unwrap(), CatchupFrame::Want(vec![]));
            assert_eq!(read(&mut y).await.unwrap(), CatchupFrame::Done);
            send(&mut y, &applied_nothing()).await.unwrap();
            send(&mut y, &CatchupFrame::Done).await.unwrap();
            read(&mut y).await.unwrap()
        };
        let (ra, last) = tokio::join!(
            run_initiator(x, &a, &a.identity, server, a.me, NODE_A, NODE_B),
            fake_responder
        );
        let ra = ra.unwrap();
        assert!(ra.peer_had.is_empty() && ra.delivered.is_empty());
        assert_eq!(last, applied_nothing());
    }

    #[tokio::test]
    async fn responder_leaves_out_buckets_outside_its_window() {
        let (a, b, channel) = pair();
        let today = day_number(Utc::now());
        let old = BucketKey::Tombstones { day: today - 40 };
        let ahead = BucketKey::Messages {
            channel,
            day: today + 5,
        };
        let digest = |key| BucketDigest {
            key,
            count: 1,
            hash: [9; 16],
        };
        let (mut x, y) = tokio::io::duplex(8 * 1024);
        let fake_initiator = async move {
            send(&mut x, &initiator_hello(&a, now_ms())).await.unwrap();
            assert!(matches!(
                read(&mut x).await.unwrap(),
                CatchupFrame::Hello { .. }
            ));
            let list = vec![digest(old), digest(ahead)];
            send(&mut x, &CatchupFrame::Digests(list)).await.unwrap();
            read(&mut x).await.unwrap()
        };
        let (first, _) = tokio::join!(fake_initiator, run_responder(y, &b, NODE_B, NODE_A));
        assert_eq!(
            first,
            CatchupFrame::ItemsEnd {
                omitted: vec![old, ahead]
            }
        );
    }

    /// A store whose bucket gains a message the first time its items are
    /// read, as when one arrives while a session runs.
    struct Racing {
        inner: MemStore,
        late: Mutex<Option<Message>>,
    }

    impl CatchupStore for Racing {
        fn local_identity(&self, server: ServerId) -> Option<(UserId, Identity)> {
            self.inner.local_identity(server)
        }
        fn verify_peer(&self, server: ServerId, user: UserId, key: &IdentityKey) -> PeerCheck {
            self.inner.verify_peer(server, user, key)
        }
        fn digests(&self, server: ServerId) -> Vec<BucketDigest> {
            CatchupStore::digests(&self.inner, server)
        }
        fn items(&self, server: ServerId, keys: &[BucketKey]) -> Vec<BucketItems> {
            if let Some(m) = self.late.lock().unwrap().take() {
                self.inner.add(&m);
            }
            self.inner.items(server, keys)
        }
        fn get(&self, server: ServerId, ids: &[MessageId]) -> (Vec<Message>, Vec<TombstoneProof>) {
            self.inner.get(server, ids)
        }
        fn profiles(&self, server: ServerId, users: &[UserId]) -> Vec<UserProfile> {
            self.inner.profiles(server, users)
        }
        fn apply_profiles(&self, server: ServerId, profiles: Vec<UserProfile>) {
            self.inner.apply_profiles(server, profiles)
        }
        fn apply(
            &self,
            server: ServerId,
            messages: Vec<Message>,
            tombstones: Vec<TombstoneProof>,
        ) -> ApplyReport {
            self.inner.apply(server, messages, tombstones)
        }
    }

    #[tokio::test]
    async fn bucket_that_changes_during_a_session_proves_nothing() {
        for racing_dials in [true, false] {
            let (a, b, channel) = pair();
            let shared = a.post(channel, "shared");
            b.add(&shared);
            let late = a.with(|s| post(s, channel, a.me, "sent while the session runs"));
            let server = a.server();
            let (me, identity) = (a.me, a.identity.clone());
            let a = Racing {
                inner: a,
                late: Mutex::new(Some(late.clone())),
            };
            let (x, y) = tokio::io::duplex(8 * 1024);
            let report = if racing_dials {
                let (ra, rb) = tokio::join!(
                    run_initiator(x, &a, &identity, server, me, NODE_A, NODE_B),
                    run_responder(y, &b, NODE_B, NODE_A),
                );
                rb.unwrap();
                ra.unwrap()
            } else {
                let (rb, ra) = tokio::join!(
                    run_initiator(x, &b, &b.identity, server, b.me, NODE_B, NODE_A),
                    run_responder(y, &a, NODE_A, NODE_B),
                );
                rb.unwrap();
                ra.unwrap().1
            };
            assert!(a.inner.has(late.id) && !b.has(late.id));
            assert!(
                !report.peer_had.contains(&late.id) && !report.delivered.contains(&late.id),
                "racing_dials = {racing_dials}"
            );
        }
    }

    #[tokio::test]
    async fn history_from_members_never_met_comes_with_their_keys() {
        let (mut base, channel, owner) = fixture();
        let server = base.server.id;
        let c_user = join(&mut base, "c");
        // d is a member c knows only from a profile without a key.
        let d = keyed_profile("d");
        let mut d_without_key = d.clone();
        d_without_key.identity = None;
        let mut c_state = base.clone();
        c_state
            .apply(ServerEvent::MemberJoin {
                server,
                profile: d_without_key,
            })
            .unwrap();
        // b joined while c was away, so c has never seen them at all.
        let mut a_state = base;
        let b_user = join(&mut a_state, "b");
        a_state
            .apply(ServerEvent::MemberJoin {
                server,
                profile: d.clone(),
            })
            .unwrap();
        let a = MemStore::new(a_state, owner);
        let c = MemStore::new(c_state, c_user);
        let from_b = a.post_as(b_user, channel, "while c was away");
        let from_d = a.post_as(d.id, channel, "me too");

        let (rc, ra) = ok_session(&c, &a).await;
        assert_eq!(rc.held, 0);
        assert_eq!(sorted(rc.received), sorted(vec![from_b.id, from_d.id]));
        assert!(c.has(from_b.id) && c.has(from_d.id));
        c.with(|s| {
            // Membership comes from b's own join, not from a relayed profile.
            assert!(!s.server.is_member(b_user) && s.profiles.contains_key(&b_user));
            assert_eq!(s.identity_of(b_user), Some(identity_for(b_user).public()));
            assert_eq!(s.identity_of(d.id), d.identity);
        });
        assert_eq!(sorted(ra.delivered), sorted(vec![from_b.id, from_d.id]));
        assert_eq!(a.digests(), c.digests());
    }

    #[test]
    fn apply_profiles_pins_keys_without_making_members() {
        let (mut state, _, _) = fixture();
        let server = state.server.id;
        let friend = join(&mut state, "friend");
        let newcomer = keyed_profile("newcomer");
        let banned = keyed_profile("banned");
        state.server.banned.push(banned.id);
        let keyless = keyed_profile("keyless");
        let mut stored = keyless.clone();
        stored.identity = None;
        stored.presence = Presence::Idle;
        state
            .apply(ServerEvent::MemberJoin {
                server,
                profile: stored,
            })
            .unwrap();
        let mut forged = state.profiles[&friend].clone();
        forged.identity = Some(Identity::generate().public());
        forged.username = "forged".into();
        let mut no_key = keyed_profile("no key");
        no_key.identity = None;
        let members = state.server.members.clone();

        let learned = apply_profiles(
            &mut state,
            vec![
                newcomer.clone(),
                banned.clone(),
                keyless.clone(),
                forged,
                no_key.clone(),
            ],
        );
        assert_eq!(learned, vec![newcomer.id, keyless.id]);
        assert_eq!(state.server.members, members, "membership is left alone");
        assert_eq!(state.identity_of(newcomer.id), newcomer.identity);
        assert_eq!(state.profiles[&newcomer.id].presence, Presence::Offline);
        assert_eq!(state.identity_of(keyless.id), keyless.identity);
        assert_eq!(state.profiles[&keyless.id].presence, Presence::Idle);
        assert!(!state.profiles.contains_key(&banned.id) && !state.profiles.contains_key(&no_key.id));
        // A known key is never replaced, nor the profile around it.
        assert_eq!(
            state.identity_of(friend),
            Some(identity_for(friend).public())
        );
        assert_eq!(state.profiles[&friend].username, "friend");
    }

    #[test]
    fn profiles_of_lists_keyed_profiles_once() {
        let (mut state, _, owner) = fixture();
        let friend = join(&mut state, "friend");
        let mut keyless = keyed_profile("keyless");
        keyless.identity = None;
        let server = state.server.id;
        state
            .apply(ServerEvent::MemberJoin {
                server,
                profile: keyless.clone(),
            })
            .unwrap();
        let former = join(&mut state, "former");
        state
            .apply(ServerEvent::MemberLeave {
                server,
                user: former,
            })
            .unwrap();
        let asked = [friend, keyless.id, owner, former, friend, UserId::new()];
        let ids: Vec<UserId> = profiles_of(&state, &asked).iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![friend, owner], "keyed members only");
    }

    #[tokio::test]
    async fn relayed_history_never_brings_back_a_kicked_member() {
        let (mut base, channel, owner) = fixture();
        let server = base.server.id;
        let viewer = join(&mut base, "viewer");
        let kicked = keyed_profile("kicked");
        let mut keyless = kicked.clone();
        keyless.identity = None;
        let kick = ServerEvent::MemberLeave {
            server,
            user: kicked.id,
        };
        // c and e know the member only from a profile without a key, and
        // applied the kick.
        let mut c_state = base.clone();
        c_state
            .apply(ServerEvent::MemberJoin {
                server,
                profile: keyless,
            })
            .unwrap();
        c_state.apply(kick.clone()).unwrap();
        let c = MemStore::new(c_state.clone(), viewer);
        let e = MemStore::new(c_state, viewer);
        // a has the member's key and has not heard of the kick yet.
        let mut a_state = base;
        a_state
            .apply(ServerEvent::MemberJoin {
                server,
                profile: kicked.clone(),
            })
            .unwrap();
        let a = MemStore::new(a_state, owner);
        let old = a.post_as(kicked.id, channel, "posted before the kick");

        let (rc, _) = ok_session(&c, &a).await;
        assert_eq!(rc.received, vec![old.id]);
        let key = kicked.identity.unwrap();
        c.with(|s| {
            assert!(!s.server.is_member(kicked.id), "still out");
            assert_eq!(s.identity_of(kicked.id), Some(key));
            assert_eq!(check_peer(s, kicked.id, &key), PeerCheck::Unknown);
        });

        // Once a applies the kick too, it no longer vouches for the key.
        a.with(|s| s.apply(kick).unwrap());
        let (re, _) = ok_session(&e, &a).await;
        assert_eq!((re.held, re.received.len()), (1, 0));
        e.with(|s| {
            assert!(!s.server.is_member(kicked.id) && s.identity_of(kicked.id).is_none());
        });
    }

    /// Feeds `frames` to the receiving side of a session and returns how
    /// it ended.
    async fn receive(store: &MemStore, frames: Vec<CatchupFrame>) -> Result<Received, CatchupError> {
        let (mut x, mut y) = tokio::io::duplex(64 * 1024);
        let feed = async move {
            for frame in frames {
                if write_frame(&mut x, &frame).await.is_err() {
                    break;
                }
            }
        };
        let take = async move {
            let accept_all = |_: MessageId, _: BucketKey| true;
            let result = receive_pages(&mut y, store, store.server(), accept_all).await;
            // Hanging up lets a feed cut short finish.
            drop(y);
            result
        };
        let ((), result) = tokio::join!(feed, take);
        result
    }

    #[tokio::test]
    async fn profiles_beyond_the_pages_signers_end_the_session() {
        let (a, b, channel) = pair();
        let profiles = |n: usize| -> Vec<UserProfile> {
            (0..n).map(|i| keyed_profile(&format!("p{i}"))).collect()
        };
        // A page's worth may come before the page that names them.
        let ahead = profiles(PROFILE_MARGIN);
        let got = receive(&b, vec![CatchupFrame::Profiles(ahead.clone()), CatchupFrame::Done]).await;
        assert!(got.unwrap().applied.is_empty());
        let one_more = vec![
            CatchupFrame::Profiles(ahead.clone()),
            CatchupFrame::Profiles(profiles(1)),
            CatchupFrame::Done,
        ];
        assert!(matches!(receive(&b, one_more).await.unwrap_err(), CatchupError::TooMuch));
        // Signers of pages already sent count on top.
        let base = a.with(|s| s.clone());
        let page: Vec<Message> = ahead.iter().map(|p| post(&base, channel, p.id, "hi")).collect();
        let named = vec![
            CatchupFrame::Profiles(ahead.clone()),
            CatchupFrame::Push {
                messages: page.clone(),
                tombstones: vec![],
            },
            CatchupFrame::Profiles(profiles(PROFILE_MARGIN)),
            CatchupFrame::Done,
        ];
        let got = receive(&b, named).await.unwrap();
        assert_eq!(got.applied.len(), PROFILE_MARGIN);
        let past = vec![
            CatchupFrame::Push {
                messages: page,
                tombstones: vec![],
            },
            CatchupFrame::Profiles(profiles(2 * PROFILE_MARGIN + 1)),
            CatchupFrame::Done,
        ];
        assert!(matches!(receive(&b, past).await.unwrap_err(), CatchupError::TooMuch));
    }

    /// A keyed profile that encodes to `size` bytes.
    fn profile_of_size(name: &str, size: usize) -> UserProfile {
        let mut p = keyed_profile(name);
        p.about = Some("x".repeat(200));
        let fill = 200 + size - encoded_len(&p);
        p.about = Some("x".repeat(fill));
        assert_eq!(encoded_len(&p), size);
        p
    }

    #[tokio::test]
    async fn profiles_larger_than_gossip_carries_are_ignored() {
        let (a, b, channel) = pair();
        let base = a.with(|s| s.clone());
        let fits = profile_of_size("fits", MAX_RELAYED_PROFILE_BYTES);
        let over = profile_of_size("over", MAX_RELAYED_PROFILE_BYTES + 1);
        let frames = vec![
            CatchupFrame::Profiles(vec![fits.clone(), over.clone()]),
            CatchupFrame::Push {
                messages: vec![
                    post(&base, channel, fits.id, "hi"),
                    post(&base, channel, over.id, "hi"),
                ],
                tombstones: vec![],
            },
            CatchupFrame::Done,
        ];
        let got = receive(&b, frames).await.unwrap();
        assert_eq!((got.applied.len(), got.held.len()), (1, 1));
        b.with(|s| {
            assert!(s.identity_of(fits.id).is_some());
            assert!(s.identity_of(over.id).is_none());
        });
    }

    #[tokio::test]
    async fn profiles_past_the_byte_limit_end_the_session() {
        let (a, b, channel) = pair();
        let base = a.with(|s| s.clone());
        let flood = |n: usize| {
            CatchupFrame::Profiles(
                (0..n)
                    .map(|_| profile_of_size("large", MAX_RELAYED_PROFILE_BYTES))
                    .collect(),
            )
        };
        // The most that may wait at once. Pages naming enough signers come
        // first, so the count of profiles is no limit.
        let most = MAX_PROFILE_BYTES / MAX_RELAYED_PROFILE_BYTES;
        let named: Vec<UserProfile> = (0..2 * PAGE_SIZE)
            .map(|i| keyed_profile(&format!("n{i}")))
            .collect();
        assert!(most + 2 <= named.len() + PROFILE_MARGIN);
        let pages: Vec<CatchupFrame> = named
            .chunks(PAGE_SIZE)
            .map(|chunk| CatchupFrame::Push {
                messages: chunk
                    .iter()
                    .map(|p| post(&base, channel, p.id, "hi"))
                    .collect(),
                tombstones: vec![],
            })
            .collect();
        let session = |rest: Vec<CatchupFrame>| {
            let mut frames = pages.clone();
            frames.extend(rest);
            frames.push(CatchupFrame::Done);
            frames
        };
        assert!(receive(&b, session(vec![flood(most)])).await.is_ok());
        assert!(matches!(
            receive(&b, session(vec![flood(most + 1)])).await.unwrap_err(),
            CatchupError::TooMuch
        ));
        // Profiles a page did not use do not add up across pages.
        let empty = CatchupFrame::Push {
            messages: vec![],
            tombstones: vec![],
        };
        let half = most / 2 + 1;
        let spread = vec![flood(half), empty.clone(), flood(half), empty];
        assert!(receive(&b, session(spread)).await.is_ok());
    }

    #[tokio::test]
    async fn many_signers_bring_their_profiles_page_by_page() {
        let (a, b, channel) = pair();
        let count = 2 * PAGE_SIZE + 10;
        let authors: Vec<UserId> = a.with(|s| (0..count).map(|i| join(s, &format!("m{i}"))).collect());
        for author in &authors {
            a.post_as(*author, channel, "first post");
        }
        let (rb, ra) = ok_session(&b, &a).await;
        assert_eq!((rb.received.len(), rb.held), (count, 0));
        assert_eq!(ra.delivered.len(), count);
        b.with(|s| {
            assert!(authors.iter().all(|u| s.identity_of(*u).is_some()));
        });
    }

    #[tokio::test]
    async fn initiator_keeps_its_report_when_the_peer_leaves_before_its_applied() {
        let (a, b, channel) = pair();
        let only_a = a.post(channel, "only a");
        let only_b = b.post(channel, "only b");
        let server = a.server();
        let (x, mut y) = tokio::io::duplex(64 * 1024);
        let fake_responder = async move {
            let _hello = read(&mut y).await.unwrap();
            send(&mut y, &responder_hello(&b)).await.unwrap();
            let CatchupFrame::Digests(theirs) = read(&mut y).await.unwrap() else {
                panic!("expected digests");
            };
            let keys = mismatched(&b.digests(), &theirs);
            let listed = CatchupStore::items(&b, server, &keys);
            send(&mut y, &CatchupFrame::Items(listed)).await.unwrap();
            send(&mut y, &CatchupFrame::ItemsEnd { omitted: vec![] })
                .await
                .unwrap();
            let CatchupFrame::Want(want) = read(&mut y).await.unwrap() else {
                panic!("expected want");
            };
            let got = receive_pages(&mut y, &b, server, |_, _| true)
                .await
                .unwrap();
            send(&mut y, &got.verdict()).await.unwrap();
            send_items(&mut y, &b, server, &want, |_| true).await.unwrap();
            // Gone before the initiator's `Applied`, as when the peer
            // stopped waiting for it.
            drop(y);
            b
        };
        let (ra, b) = tokio::join!(
            run_initiator(x, &a, &a.identity, server, a.me, NODE_A, NODE_B),
            fake_responder
        );
        let ra = ra.expect("the report survives");
        assert_eq!(ra.delivered, vec![only_a.id]);
        assert_eq!(ra.received, vec![only_b.id]);
        assert!(a.has(only_b.id) && b.has(only_a.id));
    }

    #[test]
    fn the_responder_waits_for_applied_as_long_as_for_any_frame() {
        assert!(APPLIED_TIMEOUT >= FRAME_TIMEOUT);
    }

    #[tokio::test]
    async fn future_dated_messages_stay_out_of_catch_up() {
        let (a, b, channel) = pair();
        let base = a.with(|s| s.clone());
        let ahead = |days: i64, text: &str| {
            post_at(
                &base,
                channel,
                a.me,
                text,
                Utc::now() + chrono::Duration::days(days),
            )
        };
        let tomorrow = ahead(1, "tomorrow");
        let later = ahead(3, "in three days");
        a.add(&tomorrow);
        // Refused when it arrives now, but data saved before may hold it.
        a.with(|s| {
            let mut list = s.messages(channel).to_vec();
            list.push(later.clone());
            *s = with_messages(s, channel, list);
        });
        assert!(a.has(later.id));

        let (ra, rb) = ok_session(&a, &b).await;
        assert!(b.has(tomorrow.id) && !b.has(later.id));
        assert_eq!(ra.delivered, vec![tomorrow.id]);
        assert_eq!(rb.received, vec![tomorrow.id]);
        assert_eq!(a.digests(), b.digests());
    }

    #[tokio::test]
    async fn pages_dated_outside_the_window_are_ignored() {
        let (a, b, channel) = pair();
        let ahead = a.with(|s| {
            post_at(
                s,
                channel,
                a.me,
                "from the future",
                Utc::now() + chrono::Duration::days(3),
            )
        });
        let (mut x, mut y) = tokio::io::duplex(8 * 1024);
        let push = CatchupFrame::Push {
            messages: vec![ahead.clone()],
            tombstones: vec![],
        };
        send(&mut x, &push).await.unwrap();
        send(&mut x, &CatchupFrame::Done).await.unwrap();
        let got = receive_pages(&mut y, &b, b.server(), |_, _| true)
            .await
            .unwrap();
        assert!(got.applied.is_empty());
        assert_eq!(got.rejected, vec![ahead.id]);
        assert!(!b.has(ahead.id));
    }

    #[tokio::test]
    async fn too_many_digests_end_the_session() {
        let (a, b, _) = pair();
        let today = day_number(Utc::now());
        let list: Vec<BucketDigest> = (0..=MAX_DIGESTS)
            .map(|_| BucketDigest {
                key: BucketKey::Messages {
                    channel: ChannelId::new(),
                    day: today,
                },
                count: 1,
                hash: [1; 16],
            })
            .collect();
        let (mut x, y) = tokio::io::duplex(64 * 1024);
        let fake_initiator = async move {
            send(&mut x, &initiator_hello(&a, now_ms())).await.unwrap();
            assert!(matches!(
                read(&mut x).await.unwrap(),
                CatchupFrame::Hello { .. }
            ));
            send(&mut x, &CatchupFrame::Digests(list)).await.unwrap();
            read(&mut x).await
        };
        let (after, rb) = tokio::join!(fake_initiator, run_responder(y, &b, NODE_B, NODE_A));
        assert!(matches!(rb.unwrap_err(), CatchupError::TooMuch));
        assert!(matches!(after.unwrap_err(), CatchupError::Closed));
    }

    #[tokio::test]
    async fn endless_pushes_end_the_session() {
        let (_, b, _) = pair();
        let (mut x, mut y) = tokio::io::duplex(64 * 1024);
        let flood = async {
            for _ in 0..=MAX_PUSHED_ITEMS {
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
        let spec = |domain: &str| {
            postcard::to_stdvec(&(domain, server, user, NODE_A, NODE_B, 1234i64)).unwrap()
        };
        let initiator = hello_bytes(HelloRole::Initiator, server, user, NODE_A, NODE_B, 1234);
        let responder = hello_bytes(HelloRole::Responder, server, user, NODE_A, NODE_B, 1234);
        assert_eq!(initiator, spec("commy-catchup/v1/initiator"));
        assert_eq!(responder, spec("commy-catchup/v1/responder"));
        assert_ne!(initiator, responder, "role matters");
        assert_ne!(
            hello_bytes(HelloRole::Initiator, server, user, NODE_B, NODE_A, 1234),
            initiator,
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
                HelloRole::Initiator,
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
                messages: vec![m.clone()],
                tombstones: vec![proof],
            },
            CatchupFrame::Done,
            CatchupFrame::ItemsEnd { omitted: vec![key] },
            CatchupFrame::Profiles(vec![state.profiles[&owner].clone()]),
            CatchupFrame::Applied {
                held: vec![m.id],
                rejected: vec![MessageId::new()],
            },
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
        assert_eq!(tag(&CatchupFrame::ItemsEnd { omitted: vec![] }), 7);
        assert_eq!(tag(&CatchupFrame::Profiles(vec![])), 8);
        assert_eq!(tag(&applied_nothing()), 9);
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
    fn fit_items_keeps_whole_ids_in_order() {
        let (mut state, channel, owner) = fixture();
        let first = post(&state, channel, owner, "first");
        let last = post(&state, channel, owner, "last");
        let gone = post(&state, channel, owner, "gone");
        let proofs: Vec<TombstoneProof> = ["a", "b"]
            .map(|name| join_moderator(&mut state, name))
            .iter()
            .map(|by| proof_at(&state, &gone, *by, Utc::now()))
            .collect();
        let gone = gone.id;
        let ids = [first.id, gone, last.id];
        let messages = vec![first.clone(), last.clone()];
        let fit = |max| fit_items(&ids, messages.clone(), proofs.clone(), max);
        assert_eq!(fit(4), (messages.clone(), proofs.clone()));
        assert_eq!(fit(3), (vec![first.clone()], proofs.clone()));
        // An id that does not fit stops the rest, even ones that would.
        assert_eq!(fit(2), (vec![first], vec![]));
        assert_eq!(fit(0), (vec![], vec![]));
    }

    fn bucket(channel: ChannelId, day: u32, n: usize) -> BucketItems {
        (
            BucketKey::Messages { channel, day },
            (0..n).map(|_| (MessageId::new(), 0)).collect(),
        )
    }

    #[test]
    fn fit_listing_leaves_out_the_oldest_buckets_only_when_needed() {
        let channel = ChannelId::new();
        let small = vec![bucket(channel, 1, 3), bucket(channel, 2, 3)];
        assert_eq!(fit_listing(small.clone(), 8), (small, vec![]));

        let big = vec![
            bucket(channel, 5, 4),
            bucket(channel, 9, 4),
            bucket(channel, 7, 4),
            bucket(channel, 6, 4),
        ];
        let (kept, dropped) = fit_listing(big, 10);
        let days: Vec<u32> = kept.iter().map(|(k, _)| k.day()).collect();
        assert_eq!(days, vec![7, 9]);
        let dropped: Vec<u32> = dropped.iter().map(|k| k.day()).collect();
        assert_eq!(dropped, vec![5, 6]);
    }

    #[test]
    fn an_oversized_bucket_holds_back_only_itself() {
        let channel = ChannelId::new();
        let other = ChannelId::new();
        let list = vec![
            bucket(channel, 3, 2),
            bucket(channel, 5, 2),
            bucket(other, 9, 50),
            bucket(channel, 7, 2),
            bucket(channel, 9, 2),
        ];
        let (kept, dropped) = fit_listing(list, 12);
        let days: Vec<u32> = kept.iter().map(|(k, _)| k.day()).collect();
        assert_eq!(days, vec![3, 5, 7, 9]);
        assert_eq!(dropped, vec![BucketKey::Messages { channel: other, day: 9 }]);
    }

    #[tokio::test]
    async fn large_listings_span_pages_and_arrive_whole() {
        let channel = ChannelId::new();
        let per_page = (FRAME_BUDGET - KEY_BOUND) / ITEM_BOUND;
        let list = vec![
            bucket(channel, 1, 2),
            bucket(channel, 2, per_page + 10),
            bucket(channel, 3, 0),
        ];
        let out = item_pages(list.clone());
        assert_eq!(out.len(), 2);
        assert_eq!(out[1][0].0, list[1].0, "the large bucket goes on");
        let (mut x, mut y) = tokio::io::duplex(64 * 1024);
        let writer = async {
            for page in out {
                let frame = CatchupFrame::Items(page);
                assert!(postcard::to_stdvec(&frame).unwrap().len() <= MAX_MEDIA_FRAME as usize);
                write_frame(&mut x, &frame).await.unwrap();
            }
            send(&mut x, &CatchupFrame::ItemsEnd { omitted: vec![] })
                .await
                .unwrap();
        };
        let ((), got) = tokio::join!(writer, read_items(&mut y));
        let (got, omitted) = got.unwrap();
        assert_eq!(got, list);
        assert!(omitted.is_empty());
    }

    #[test]
    fn fit_digests_keeps_the_newest_days() {
        let channel = ChannelId::new();
        let digest = |day| BucketDigest {
            key: BucketKey::Messages { channel, day },
            count: 1,
            hash: [0; 16],
        };
        let small = vec![digest(1), digest(2)];
        assert_eq!(fit_digests(small.clone()), small);
        let big: Vec<BucketDigest> = (0..MAX_DIGESTS as u32 + 3).map(digest).collect();
        let kept = fit_digests(big);
        assert_eq!(kept.len(), MAX_DIGESTS);
        assert_eq!(kept[0].key.day(), 3);
        assert!(kept.windows(2).all(|w| w[0].key < w[1].key));
        let bytes = postcard::to_stdvec(&CatchupFrame::Digests(kept)).unwrap();
        assert!(bytes.len() <= FRAME_BUDGET);
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

        // Nothing of a channel that is gone, proofs included.
        state.server.channels.retain(|c| c.id != channel);
        assert_eq!(collect(&state, &[first.id, gone.id]), (vec![], vec![]));
    }

    #[test]
    fn apply_items_rejects_deletes_of_unseen_messages_by_others() {
        let (mut state, channel, _) = fixture();
        let author = join(&mut state, "author");
        let member = join(&mut state, "member");
        let theirs = post(&state, channel, author, "theirs");
        let own = post(&state, channel, member, "own");
        let now = Utc::now();
        let proofs = vec![
            proof_at(&state, &theirs, member, now),
            proof_at(&state, &own, member, now),
        ];
        let (report, held) = apply_items(&mut state, vec![], proofs);
        assert_eq!(report.rejected, vec![theirs.id]);
        assert_eq!(report.applied, vec![own.id]);
        assert!(held.is_empty());
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
        // The proof removes `dup` first, so its late copy is a no-op.
        assert_eq!(report.applied, vec![dup.id, fresh.id]);
        assert_eq!(report.held, vec![from_stranger.id]);
        assert_eq!(report.rejected, vec![forged.id]);
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
