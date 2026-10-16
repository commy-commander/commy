//! Catch-up reconciliation. Two peers compare per-day digests of the
//! recent history they hold, list the items of the days that differ, and
//! then swap only what one side lacks. Pure logic; the wire protocol
//! lives in `commy-net`.

use crate::ids::{ChannelId, MessageId};
use crate::message::{Message, TombstoneProof};
use crate::store::ServerState;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::RangeInclusive;

/// UTC days of history covered, counting today.
pub const CATCHUP_DAYS: u32 = 30;
/// Most items sent each way in one session.
pub const CATCHUP_MAX_ITEMS: usize = 5000;

const DAY_MS: i64 = 86_400_000;
const DOMAIN: &[u8] = b"commy-catchup/v1";
const PROOF_DOMAIN: &[u8] = b"commy-catchup/v1/proof";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum BucketKey {
    /// Messages of one channel created on `day`.
    Messages { channel: ChannelId, day: u32 },
    /// Delete proofs made on `day`.
    Tombstones { day: u32 },
}

impl BucketKey {
    pub fn day(&self) -> u32 {
        match self {
            BucketKey::Messages { day, .. } | BucketKey::Tombstones { day } => *day,
        }
    }
}

/// Summary of one bucket: item count and XOR of the item hashes. A
/// bucket that is not listed counts as empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BucketDigest {
    pub key: BucketKey,
    pub count: u32,
    pub hash: [u8; 16],
}

/// A bucket's items as (id, version) pairs: a message's edit time, see
/// [`message_version`], or a delete proof's signer, channel and time, see
/// [`proof_version`].
pub type BucketItems = (BucketKey, Vec<(MessageId, i64)>);

/// Days since the epoch, UTC. Times before the epoch count as day 0.
pub fn day_number(t: DateTime<Utc>) -> u32 {
    t.timestamp_millis().div_euclid(DAY_MS).clamp(0, u32::MAX as i64) as u32
}

/// First day inside the window that ends today.
pub fn first_day(now: DateTime<Utc>) -> u32 {
    day_number(now).saturating_sub(CATCHUP_DAYS - 1)
}

/// Last day inside the window: tomorrow, for a peer whose clock is
/// already past midnight. Anything dated later stays out of catch-up.
pub fn last_day(now: DateTime<Utc>) -> u32 {
    day_number(now).saturating_add(1)
}

/// Days of the window at `now`, from [`first_day`] to [`last_day`].
pub fn window(now: DateTime<Utc>) -> RangeInclusive<u32> {
    first_day(now)..=last_day(now)
}

/// The window as of now, by this device's clock.
pub fn current_window() -> RangeInclusive<u32> {
    window(Utc::now())
}

/// Whether `day` lies in the window at `now`.
pub fn in_window(day: u32, now: DateTime<Utc>) -> bool {
    window(now).contains(&day)
}

fn day_start(day: u32) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(day as i64 * DAY_MS).unwrap_or(DateTime::<Utc>::MIN_UTC)
}

/// Edit time in ms, or 0 for a message never edited.
pub fn message_version(message: &Message) -> i64 {
    message.edited_at.map_or(0, |t| t.timestamp_millis())
}

/// A delete proof's signer, channel and time folded into 64 bits, so
/// each proof kept for a message is an item of its own.
pub fn proof_version(proof: &TombstoneProof) -> i64 {
    let digest = Sha256::new()
        .chain_update(PROOF_DOMAIN)
        .chain_update(proof.by.0.as_bytes())
        .chain_update(proof.channel.0.as_bytes())
        .chain_update(proof.at.timestamp_millis().to_be_bytes())
        .finalize();
    let mut out = [0u8; 8];
    out.copy_from_slice(&digest[..8]);
    i64::from_be_bytes(out)
}

pub fn item_hash(id: MessageId, version: i64) -> [u8; 16] {
    let digest = Sha256::new()
        .chain_update(DOMAIN)
        .chain_update(id.0.as_bytes())
        .chain_update(version.to_be_bytes())
        .finalize();
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest[..16]);
    out
}

/// Digest of one bucket's items.
pub fn bucket_digest(key: BucketKey, list: &[(MessageId, i64)]) -> BucketDigest {
    let mut hash = [0u8; 16];
    for &(id, version) in list {
        xor_into(&mut hash, item_hash(id, version));
    }
    BucketDigest { key, count: list.len() as u32, hash }
}

fn xor_into(hash: &mut [u8; 16], item: [u8; 16]) {
    for (h, b) in hash.iter_mut().zip(item) {
        *h ^= b;
    }
}

/// Messages of a channel that catch-up carries: signed, with an id bound
/// to their author (peers would refuse those saved by older versions,
/// edited since or not), inside retention and dated inside the window.
fn covered(state: &ServerState, channel: ChannelId, now: DateTime<Utc>) -> impl Iterator<Item = &Message> {
    let from = day_start(first_day(now));
    let since = state.retention_cutoff(channel, now).map_or(from, |cutoff| cutoff.max(from));
    let until = day_start(last_day(now).saturating_add(1));
    state.messages(channel).iter().filter(move |m| {
        m.sig.is_some() && m.id.is_bound_to(m.author) && m.created_at >= since && m.created_at < until
    })
}

/// Delete proofs that catch-up carries: valid here (see
/// [`ServerState::tombstones_since`]), made inside the window, for a
/// channel that still exists.
fn covered_tombstones(state: &ServerState, now: DateTime<Utc>) -> impl Iterator<Item = &TombstoneProof> {
    let until = day_start(last_day(now).saturating_add(1));
    state
        .tombstones_since(day_start(first_day(now)))
        .filter(move |p| p.at < until && state.server.channel(p.channel).is_some())
}

/// Digests of every non-empty bucket in the window, sorted by key.
pub fn digests(state: &ServerState, now: DateTime<Utc>) -> Vec<BucketDigest> {
    let mut buckets: BTreeMap<BucketKey, (u32, [u8; 16])> = BTreeMap::new();
    let mut add = |key: BucketKey, id: MessageId, version: i64| {
        let (count, hash) = buckets.entry(key).or_insert((0, [0; 16]));
        *count += 1;
        xor_into(hash, item_hash(id, version));
    };
    for channel in &state.server.channels {
        for m in covered(state, channel.id, now) {
            let key = BucketKey::Messages { channel: channel.id, day: day_number(m.created_at) };
            add(key, m.id, message_version(m));
        }
    }
    for proof in covered_tombstones(state, now) {
        add(BucketKey::Tombstones { day: day_number(proof.at) }, proof.message, proof_version(proof));
    }
    buckets
        .into_iter()
        .map(|(key, (count, hash))| BucketDigest { key, count, hash })
        .collect()
}

/// Keys whose digests differ, a missing bucket counting as empty.
pub fn mismatched(ours: &[BucketDigest], theirs: &[BucketDigest]) -> Vec<BucketKey> {
    let index = |list: &[BucketDigest]| -> HashMap<BucketKey, (u32, [u8; 16])> {
        list.iter().map(|d| (d.key, (d.count, d.hash))).collect()
    };
    let (a, b) = (index(ours), index(theirs));
    let empty = (0, [0u8; 16]);
    let keys: BTreeSet<BucketKey> = a.keys().chain(b.keys()).copied().collect();
    keys.into_iter()
        .filter(|k| a.get(k).unwrap_or(&empty) != b.get(k).unwrap_or(&empty))
        .collect()
}

/// Items of the requested buckets inside the window, one entry per
/// distinct key (empty when we hold nothing there), sorted by key. Keys
/// outside the window get no entry.
pub fn items(state: &ServerState, keys: &[BucketKey], now: DateTime<Utc>) -> Vec<BucketItems> {
    let mut out: BTreeMap<BucketKey, Vec<(MessageId, i64)>> = BTreeMap::new();
    let mut channel_days: HashMap<ChannelId, HashSet<u32>> = HashMap::new();
    let mut tombstone_days: HashSet<u32> = HashSet::new();
    for key in keys.iter().filter(|k| in_window(k.day(), now)) {
        out.entry(*key).or_default();
        match *key {
            BucketKey::Messages { channel, day } => {
                channel_days.entry(channel).or_default().insert(day);
            }
            BucketKey::Tombstones { day } => {
                tombstone_days.insert(day);
            }
        }
    }
    for (channel, days) in &channel_days {
        for m in covered(state, *channel, now) {
            let day = day_number(m.created_at);
            if days.contains(&day) {
                let key = BucketKey::Messages { channel: *channel, day };
                out.entry(key).or_default().push((m.id, message_version(m)));
            }
        }
    }
    if !tombstone_days.is_empty() {
        for proof in covered_tombstones(state, now) {
            let day = day_number(proof.at);
            if tombstone_days.contains(&day) {
                out.entry(BucketKey::Tombstones { day }).or_default().push((proof.message, proof_version(proof)));
            }
        }
    }
    for list in out.values_mut() {
        list.sort_unstable();
    }
    out.into_iter().collect()
}

/// Compares item lists of the same buckets. Returns the ids the peer
/// lacks (see [`Listing::holds`]) and the ids we lack, each without
/// duplicates: messages first, so delete proofs never crowd them out of
/// a session, then proofs, each newest first. Not capped, see [`cap`].
pub fn diff(ours: &[BucketItems], theirs: &[BucketItems]) -> (Vec<MessageId>, Vec<MessageId>) {
    (lacking(ours, theirs), lacking(theirs, ours))
}

/// One side's item lists, for asking what it holds.
#[derive(Debug, Default)]
pub struct Listing {
    messages: HashMap<(BucketKey, MessageId), i64>,
    proofs: HashSet<(BucketKey, MessageId, i64)>,
}

impl Listing {
    pub fn new(list: &[BucketItems]) -> Self {
        let mut out = Self::default();
        for (key, items) in list {
            for &(id, version) in items {
                if matches!(key, BucketKey::Tombstones { .. }) {
                    out.proofs.insert((*key, id, version));
                } else {
                    let v = out.messages.entry((*key, id)).or_insert(version);
                    *v = (*v).max(version);
                }
            }
        }
        out
    }

    /// Whether the item is listed in bucket `key`: a message at the same
    /// or a newer version, or the very same delete proof.
    pub fn holds(&self, key: BucketKey, id: MessageId, version: i64) -> bool {
        match key {
            BucketKey::Tombstones { .. } => self.proofs.contains(&(key, id, version)),
            BucketKey::Messages { .. } => self.messages.get(&(key, id)).is_some_and(|v| *v >= version),
        }
    }
}

/// Ids of items in `have` that `other` does not hold.
fn lacking(have: &[BucketItems], other: &[BucketItems]) -> Vec<MessageId> {
    let other = Listing::new(other);
    let mut found: Vec<(bool, u32, i64, MessageId)> = Vec::new();
    for (key, list) in have {
        let message = matches!(key, BucketKey::Messages { .. });
        for &(id, version) in list {
            if !other.holds(*key, id, version) {
                // A proof's version orders nothing.
                found.push((message, key.day(), if message { version } else { 0 }, id));
            }
        }
    }
    found.sort_unstable_by(|a, b| b.cmp(a));
    let mut seen = HashSet::new();
    found
        .into_iter()
        .filter_map(|(_, _, _, id)| seen.insert(id).then_some(id))
        .collect()
}

/// Keeps the first [`CATCHUP_MAX_ITEMS`] ids (messages, then proofs,
/// each the newest first for lists from [`diff`]) and returns how many
/// were dropped.
pub fn cap(ids: &mut Vec<MessageId>) -> usize {
    let dropped = ids.len().saturating_sub(CATCHUP_MAX_ITEMS);
    ids.truncate(CATCHUP_MAX_ITEMS);
    dropped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ServerEvent;
    use crate::test_support::{delete, edit, fixture, join, join_moderator, post, post_at, proof, signed};
    use chrono::Duration;

    /// Runs one reconciliation between two replicas, the way a session
    /// does, and returns (pushed to b, pulled from b).
    fn reconcile(a: &mut ServerState, b: &mut ServerState, now: DateTime<Utc>) -> (Vec<MessageId>, Vec<MessageId>) {
        let keys = mismatched(&digests(a, now), &digests(b, now));
        let (mut b_lacks, mut a_wants) = diff(&items(a, &keys, now), &items(b, &keys, now));
        cap(&mut b_lacks);
        cap(&mut a_wants);
        transfer(a, b, &b_lacks);
        transfer(b, a, &a_wants);
        (b_lacks, a_wants)
    }

    fn transfer(from: &ServerState, to: &mut ServerState, ids: &[MessageId]) {
        for id in ids {
            if let Some(m) = from.message(*id) {
                to.apply(ServerEvent::MessageCreate { message: m.clone() }).unwrap();
            }
            for p in from.valid_tombstones_for(&[*id]) {
                to.apply(p.clone().into_event()).unwrap();
            }
        }
    }

    fn at(day: u32, ms_into_day: i64) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(day as i64 * DAY_MS + ms_into_day).unwrap()
    }

    #[test]
    fn day_numbers_split_at_utc_midnight() {
        assert_eq!(day_number(at(0, 0)), 0);
        assert_eq!(day_number(at(0, DAY_MS - 1)), 0);
        assert_eq!(day_number(at(1, 0)), 1);
        assert_eq!(day_number(at(20_000, 5)), 20_000);
        assert_eq!(day_number(DateTime::from_timestamp_millis(-5).unwrap()), 0);
        assert_eq!(day_start(20_000), at(20_000, 0));
        assert_eq!(first_day(at(20_000, 123)), 20_000 - 29);
        assert_eq!(first_day(at(3, 0)), 0);
        assert_eq!(last_day(at(20_000, 123)), 20_001);
        let now = at(20_000, 5);
        assert!(in_window(20_000 - 29, now) && in_window(20_001, now));
        assert!(!in_window(20_000 - 30, now) && !in_window(20_002, now));
    }

    #[test]
    fn item_hash_matches_spec() {
        let id = MessageId::new();
        let mut h = Sha256::new();
        h.update(b"commy-catchup/v1");
        h.update(id.0.as_bytes());
        h.update(1234i64.to_be_bytes());
        assert_eq!(&item_hash(id, 1234)[..], &h.finalize()[..16]);
        assert_ne!(item_hash(id, 0), item_hash(id, 1));
        assert_ne!(item_hash(id, 0), item_hash(MessageId::new(), 0));
    }

    #[test]
    fn empty_state_has_no_buckets() {
        let (state, _, _) = fixture();
        assert!(digests(&state, Utc::now()).is_empty());
    }

    #[test]
    fn digests_bucket_by_channel_and_day() {
        let (mut state, channel, owner) = fixture();
        let now = Utc::now();
        let today = day_number(now);
        let mut ids = vec![];
        for (age, text) in [(0, "a"), (0, "b"), (2, "c")] {
            let mut m = Message::new(channel, owner, text);
            m.created_at = now - Duration::days(age);
            let m = signed(&state, m);
            ids.push((m.id, age));
            state.apply(ServerEvent::MessageCreate { message: m }).unwrap();
        }
        let list = digests(&state, now);
        assert_eq!(list.len(), 2);
        let today_bucket = list.iter().find(|d| d.key == BucketKey::Messages { channel, day: today }).unwrap();
        assert_eq!(today_bucket.count, 2);
        let mut expected = [0u8; 16];
        for (id, _) in ids.iter().filter(|(_, age)| *age == 0) {
            for (e, b) in expected.iter_mut().zip(item_hash(*id, 0)) {
                *e ^= b;
            }
        }
        assert_eq!(today_bucket.hash, expected);
        assert!(list.iter().any(|d| d.key == BucketKey::Messages { channel, day: today - 2 } && d.count == 1));
        // Sorted by key.
        assert!(list.windows(2).all(|w| w[0].key < w[1].key));
    }

    #[test]
    fn digests_skip_old_unsigned_and_expired_messages() {
        let (mut state, channel, owner) = fixture();
        let now = Utc::now();
        let mut old = Message::new(channel, owner, "old");
        old.created_at = now - Duration::days(CATCHUP_DAYS as i64 + 1);
        state.apply(ServerEvent::MessageCreate { message: signed(&state, old) }).unwrap();
        assert!(digests(&state, now).is_empty(), "outside the window");

        // Unsigned messages from older versions only arrive via snapshots.
        let mut snapshot = state.snapshot();
        let legacy = Message::new(channel, owner, "legacy");
        snapshot.messages.retain(|(c, _)| *c != channel);
        snapshot.messages.push((channel, vec![legacy]));
        let mut state = ServerState::from_snapshot(snapshot);
        assert_eq!(state.messages(channel).len(), 1);
        assert!(digests(&state, now).is_empty(), "unsigned");

        let mut week_old = Message::new(channel, owner, "week");
        week_old.created_at = now - Duration::days(7);
        state.apply(ServerEvent::MessageCreate { message: signed(&state, week_old) }).unwrap();
        assert_eq!(digests(&state, now).len(), 1);
        state.server.channels.iter_mut().find(|c| c.id == channel).unwrap().retention_days = Some(3);
        assert!(digests(&state, now).is_empty(), "past retention");
        let key = BucketKey::Messages { channel, day: day_number(now - Duration::days(7)) };
        assert_eq!(items(&state, &[key], now), vec![(key, vec![])]);
    }

    #[test]
    fn edits_change_the_digest() {
        let (mut state, channel, owner) = fixture();
        let now = Utc::now();
        let m = post(&state, channel, owner, "v1");
        state.apply(ServerEvent::MessageCreate { message: m.clone() }).unwrap();
        let before = digests(&state, now);
        state.apply(edit(&state, &m, "v2", now)).unwrap();
        let after = digests(&state, now);
        assert_eq!(before[0].count, after[0].count);
        assert_ne!(before[0].hash, after[0].hash);
        assert_eq!(mismatched(&before, &after), vec![before[0].key]);
    }

    #[test]
    fn tombstones_bucket_by_delete_day() {
        let (mut state, channel, owner) = fixture();
        let now = Utc::now();
        let m = post_at(&state, channel, owner, "bye", now - Duration::days(2));
        state.apply(ServerEvent::MessageCreate { message: m.clone() }).unwrap();
        let p = proof(&state, &m, owner, now - Duration::days(1));
        state.apply(p.clone().into_event()).unwrap();
        let list = digests(&state, now);
        assert_eq!(list.len(), 1);
        let key = BucketKey::Tombstones { day: day_number(now) - 1 };
        let version = proof_version(&p);
        assert_eq!(list[0], BucketDigest { key, count: 1, hash: item_hash(m.id, version) });
        assert_eq!(items(&state, &[key], now), vec![(key, vec![(m.id, version)])]);
    }

    #[test]
    fn mismatched_treats_missing_buckets_as_empty() {
        let key = |day| BucketKey::Tombstones { day };
        let d = |day, count, byte| BucketDigest { key: key(day), count, hash: [byte; 16] };
        assert!(mismatched(&[d(1, 1, 7)], &[d(1, 1, 7)]).is_empty());
        assert!(mismatched(&[], &[]).is_empty());
        assert_eq!(mismatched(&[d(1, 1, 7)], &[d(1, 1, 8)]), vec![key(1)]);
        assert_eq!(mismatched(&[d(1, 1, 7)], &[d(1, 2, 7)]), vec![key(1)]);
        assert_eq!(mismatched(&[d(3, 1, 7)], &[d(1, 1, 7)]), vec![key(1), key(3)]);
        // An explicitly empty bucket equals a missing one.
        assert!(mismatched(&[d(1, 0, 0)], &[]).is_empty());
    }

    #[test]
    fn items_answer_each_requested_key() {
        let (mut state, channel, owner) = fixture();
        let now = Utc::now();
        let m = post(&state, channel, owner, "hi");
        state.apply(ServerEvent::MessageCreate { message: m.clone() }).unwrap();
        let today = BucketKey::Messages { channel, day: day_number(now) };
        let other = BucketKey::Messages { channel: ChannelId::new(), day: day_number(now) };
        let out = items(&state, &[today, other, today], now);
        assert_eq!(out.len(), 2, "duplicates collapse");
        assert!(out.contains(&(today, vec![(m.id, 0)])));
        assert!(out.contains(&(other, vec![])));

        // Days outside our window get no entry, even when we hold items there.
        let mut old = Message::new(channel, owner, "old");
        old.created_at = now - Duration::days(45);
        let old = signed(&state, old);
        state.apply(ServerEvent::MessageCreate { message: old.clone() }).unwrap();
        let old_key = BucketKey::Messages { channel, day: day_number(old.created_at) };
        let far = BucketKey::Tombstones { day: day_number(now) + 2 };
        assert!(items(&state, &[old_key, far], now).is_empty());
    }

    #[test]
    fn future_dated_items_stay_out_of_the_window() {
        let (mut state, channel, owner) = fixture();
        let now = Utc::now();
        let dated = |state: &ServerState, days: i64, text: &str| {
            let mut m = Message::new(channel, owner, text);
            m.created_at = now + Duration::days(days);
            signed(state, m)
        };
        let tomorrow = dated(&state, 1, "tomorrow");
        state.apply(ServerEvent::MessageCreate { message: tomorrow.clone() }).unwrap();
        let gone = post(&state, channel, owner, "deleted tomorrow");
        state.apply(proof(&state, &gone, owner, now + Duration::days(1)).into_event()).unwrap();
        // Later dates are refused now, but may sit in data saved before.
        let later = dated(&state, 3, "in three days");
        let gone_later = post(&state, channel, owner, "deleted in three days");
        let mut snapshot = state.snapshot();
        for (c, list) in snapshot.messages.iter_mut() {
            if *c == channel {
                list.push(later.clone());
            }
        }
        snapshot.tombstone_proofs.push(proof(&state, &gone_later, owner, now + Duration::days(3)));
        let state = ServerState::from_snapshot(snapshot);
        assert!(state.has_message(later.id) && state.tombstone(gone_later.id).is_some());

        let next = day_number(now) + 1;
        let keys: Vec<BucketKey> = digests(&state, now).iter().map(|d| d.key).collect();
        assert_eq!(keys, vec![BucketKey::Messages { channel, day: next }, BucketKey::Tombstones { day: next }]);
        let beyond = [BucketKey::Messages { channel, day: next + 2 }, BucketKey::Tombstones { day: next + 2 }];
        assert!(items(&state, &beyond, now).is_empty());
        let key = BucketKey::Messages { channel, day: next };
        assert_eq!(items(&state, &[key], now), vec![(key, vec![(tomorrow.id, 0)])]);
    }

    #[test]
    fn proofs_for_channels_that_are_gone_stay_out() {
        let (mut state, channel, owner) = fixture();
        let m = post(&state, channel, owner, "in a channel about to go");
        let now = Utc::now();
        state.apply(ServerEvent::MessageCreate { message: m.clone() }).unwrap();
        let p = proof(&state, &m, owner, now);
        state.apply(p.clone().into_event()).unwrap();
        let key = BucketKey::Tombstones { day: day_number(now) };
        assert_eq!(items(&state, &[key], now), vec![(key, vec![(m.id, proof_version(&p))])]);
        // As left behind by an older version, or held by a peer that has
        // not pruned yet.
        state.server.channels.retain(|c| c.id != channel);
        assert!(state.tombstone(m.id).is_some());
        assert!(digests(&state, now).is_empty());
        assert_eq!(items(&state, &[key], now), vec![(key, vec![])]);
        let (other, _, _) = fixture();
        assert!(mismatched(&digests(&state, now), &digests(&other, now)).is_empty());
    }

    #[test]
    fn bucket_digest_matches_digests() {
        let (mut state, channel, owner) = fixture();
        let now = Utc::now();
        for text in ["a", "b", "c"] {
            let m = post(&state, channel, owner, text);
            state.apply(ServerEvent::MessageCreate { message: m }).unwrap();
        }
        let sent = digests(&state, now);
        let listed = items(&state, &[sent[0].key], now);
        assert_eq!(bucket_digest(listed[0].0, &listed[0].1), sent[0]);
        assert_eq!(bucket_digest(sent[0].key, &[]), BucketDigest { key: sent[0].key, count: 0, hash: [0; 16] });
    }

    #[test]
    fn diff_finds_missing_and_older_items() {
        let channel = ChannelId::new();
        let k1 = BucketKey::Messages { channel, day: 10 };
        let k2 = BucketKey::Messages { channel, day: 12 };
        let (a, b, c, d) = (MessageId::new(), MessageId::new(), MessageId::new(), MessageId::new());
        let ours = vec![(k1, vec![(a, 0), (b, 5)]), (k2, vec![(d, 0)])];
        let theirs = vec![(k1, vec![(b, 3), (c, 0)])];
        let (they_lack, we_want) = diff(&ours, &theirs);
        // Newest day first: d (day 12), then b (version 5), then a.
        assert_eq!(they_lack, vec![d, b, a]);
        assert_eq!(we_want, vec![c]);
        // Same versions on both sides: nothing to do.
        assert_eq!(diff(&ours, &ours), (vec![], vec![]));
        // Newer on their side means we want it.
        let theirs = vec![(k1, vec![(a, 0), (b, 9)]), (k2, vec![(d, 0)])];
        assert_eq!(diff(&ours, &theirs), (vec![], vec![b]));
    }

    #[test]
    fn proofs_differ_by_signer_and_time_not_by_order() {
        let (mut state, channel, owner) = fixture();
        let other = join(&mut state, "other");
        let m = post(&state, channel, owner, "x");
        let now = Utc::now();
        let (p1, p2) = (proof(&state, &m, owner, now), proof(&state, &m, other, now));
        let later = proof(&state, &m, owner, now + Duration::seconds(1));
        let voice = state.server.channels[1].id;
        let elsewhere = TombstoneProof::sign(state.server.id, voice, m.id, owner, now, &crate::test_support::identity_for(owner));
        assert_ne!(proof_version(&p1), proof_version(&p2));
        assert_ne!(proof_version(&p1), proof_version(&later));
        assert_ne!(proof_version(&p1), proof_version(&elsewhere));
        let key = BucketKey::Tombstones { day: day_number(now) };
        let (v1, v2) = (proof_version(&p1), proof_version(&p2));
        // Whichever proof ranks higher, each side lacks the one it has not got.
        let ours = vec![(key, vec![(m.id, v1), (m.id, v2)])];
        assert_eq!(diff(&ours, &[(key, vec![(m.id, v1)])]), (vec![m.id], vec![]));
        assert_eq!(diff(&ours, &[(key, vec![(m.id, v2)])]), (vec![m.id], vec![]));
        assert_eq!(diff(&[(key, vec![(m.id, v1)])], &[(key, vec![(m.id, v2)])]), (vec![m.id], vec![m.id]));
        assert_eq!(diff(&ours, &ours), (vec![], vec![]));
        let listing = Listing::new(&ours);
        assert!(listing.holds(key, m.id, v1) && !listing.holds(key, m.id, v1.wrapping_add(1)));
        let messages = BucketKey::Messages { channel, day: day_number(now) };
        let edits = Listing::new(&[(messages, vec![(m.id, 5)])]);
        assert!(edits.holds(messages, m.id, 3) && edits.holds(messages, m.id, 5) && !edits.holds(messages, m.id, 6));
    }

    #[test]
    fn diff_compares_per_bucket_and_dedups() {
        let id = MessageId::new();
        let messages = BucketKey::Messages { channel: ChannelId::new(), day: 3 };
        let tombstones = BucketKey::Tombstones { day: 3 };
        // A message on our side and its tombstone on theirs are different items.
        let (they_lack, we_want) = diff(&[(messages, vec![(id, 0)])], &[(tombstones, vec![(id, 0)])]);
        assert_eq!((they_lack, we_want), (vec![id], vec![id]));
        let both = vec![(messages, vec![(id, 0)]), (tombstones, vec![(id, 0)])];
        assert_eq!(diff(&both, &[]).0, vec![id], "listed once");
    }

    #[test]
    fn diff_lists_messages_before_proofs() {
        let channel = ChannelId::new();
        let (old, new, gone) = (MessageId::new(), MessageId::new(), MessageId::new());
        let ours = vec![
            (BucketKey::Messages { channel, day: 9 }, vec![(old, 0)]),
            (BucketKey::Messages { channel, day: 10 }, vec![(new, 0)]),
            (BucketKey::Tombstones { day: 11 }, vec![(gone, 7)]),
        ];
        // Even a proof dated later waits behind every message.
        assert_eq!(diff(&ours, &[]).0, vec![new, old, gone]);
        assert_eq!(diff(&[], &ours).1, vec![new, old, gone]);
    }

    #[test]
    fn cap_keeps_the_newest() {
        let mut ids: Vec<MessageId> = (0..CATCHUP_MAX_ITEMS + 3).map(|_| MessageId::new()).collect();
        let first = ids[0];
        let last_kept = ids[CATCHUP_MAX_ITEMS - 1];
        assert_eq!(cap(&mut ids), 3);
        assert_eq!(ids.len(), CATCHUP_MAX_ITEMS);
        assert_eq!((ids[0], ids[CATCHUP_MAX_ITEMS - 1]), (first, last_kept));
        let mut few = vec![first];
        assert_eq!(cap(&mut few), 0);
        assert_eq!(few, vec![first]);
    }

    #[test]
    fn diff_orders_newest_first_so_cap_drops_the_oldest() {
        let channel = ChannelId::new();
        let ours: Vec<BucketItems> = (0..3u32)
            .map(|day| (BucketKey::Messages { channel, day }, vec![(MessageId::new(), 0)]))
            .collect();
        let (they_lack, _) = diff(&ours, &[]);
        let expected: Vec<MessageId> = ours.iter().rev().map(|(_, list)| list[0].0).collect();
        assert_eq!(they_lack, expected);
    }

    #[test]
    fn replicas_converge_after_one_session() {
        let (mut a, channel, owner) = fixture();
        let friend = join(&mut a, "friend");
        let mut b = a.clone();
        let now = Utc::now();

        let shared = post(&a, channel, owner, "shared");
        let edited = post(&a, channel, friend, "draft");
        let doomed = post(&a, channel, friend, "doomed");
        for m in [&shared, &edited, &doomed] {
            a.apply(ServerEvent::MessageCreate { message: m.clone() }).unwrap();
            b.apply(ServerEvent::MessageCreate { message: m.clone() }).unwrap();
        }
        let only_a = post(&a, channel, owner, "only a");
        a.apply(ServerEvent::MessageCreate { message: only_a.clone() }).unwrap();
        let only_b = post(&b, channel, friend, "only b");
        b.apply(ServerEvent::MessageCreate { message: only_b.clone() }).unwrap();
        a.apply(edit(&a, &edited, "final", now)).unwrap();
        b.apply(delete(&b, &doomed, friend)).unwrap();

        assert_ne!(digests(&a, now), digests(&b, now));
        let (pushed, pulled) = reconcile(&mut a, &mut b, now);
        assert!(pushed.contains(&only_a.id) && pushed.contains(&edited.id));
        assert!(pulled.contains(&only_b.id) && pulled.contains(&doomed.id));

        assert_eq!(digests(&a, now), digests(&b, now));
        for state in [&a, &b] {
            assert!(state.has_message(only_a.id) && state.has_message(only_b.id));
            assert_eq!(state.message(edited.id).unwrap().content, "final");
            assert!(!state.has_message(doomed.id));
            assert!(state.tombstone(doomed.id).is_some());
        }
        // A second session finds nothing to do.
        assert_eq!(reconcile(&mut a, &mut b, now), (vec![], vec![]));
    }

    #[test]
    fn late_tombstone_wins_over_a_peer_still_holding_the_message() {
        let (mut a, channel, owner) = fixture();
        let mut b = a.clone();
        let now = Utc::now();
        let m = post(&a, channel, owner, "gone soon");
        a.apply(ServerEvent::MessageCreate { message: m.clone() }).unwrap();
        // b only ever saw the delete.
        b.apply(delete(&b, &m, owner)).unwrap();
        reconcile(&mut a, &mut b, now);
        assert!(!a.has_message(m.id) && !b.has_message(m.id));
        assert_eq!(digests(&a, now), digests(&b, now));
    }

    #[test]
    fn replicas_holding_different_proofs_for_one_message_converge() {
        for days_apart in [0, 1] {
            let (mut a, channel, owner) = fixture();
            let first = join_moderator(&mut a, "first");
            let second = join_moderator(&mut a, "second");
            let mut c = a.clone();
            let now = Utc::now();
            let spam = post_at(&a, channel, owner, "spam", now - Duration::days(2));
            a.apply(ServerEvent::MessageCreate { message: spam.clone() }).unwrap();
            c.apply(ServerEvent::MessageCreate { message: spam.clone() }).unwrap();
            let p1 = proof(&a, &spam, first, now - Duration::days(days_apart));
            let p2 = proof(&a, &spam, second, now);
            for p in [&p1, &p2] {
                a.apply(p.clone().into_event()).unwrap();
            }
            // c missed the second delete.
            c.apply(p1.clone().into_event()).unwrap();
            assert_ne!(digests(&a, now), digests(&c, now));
            let (pushed, pulled) = reconcile(&mut a, &mut c, now);
            assert_eq!((pushed, pulled), (vec![spam.id], vec![]), "days apart: {days_apart}");
            assert_eq!(c.tombstones_for(spam.id).len(), 2);
            assert_eq!(digests(&a, now), digests(&c, now), "days apart: {days_apart}");
            assert_eq!(reconcile(&mut a, &mut c, now), (vec![], vec![]));
        }
    }

    #[test]
    fn two_deletes_naming_different_channels_settle_the_same() {
        for right_first in [true, false] {
            let (mut a, channel, _) = fixture();
            let sid = a.server.id;
            let author = join(&mut a, "author");
            let mut b = a.clone();
            let m = post(&a, channel, author, "regretted");
            let now = Utc::now();
            a.apply(ServerEvent::MessageCreate { message: m.clone() }).unwrap();
            // The wrong one is the earlier, so a replica choosing between
            // the two by time alone would keep it.
            let voice = a.server.channels[1].id;
            let identity = crate::test_support::identity_for(author);
            let wrong = TombstoneProof::sign(sid, voice, m.id, author, now - Duration::seconds(1), &identity);
            let right = proof(&a, &m, author, now);
            let order = if right_first { [&right, &wrong] } else { [&wrong, &right] };
            for p in order {
                let _ = a.apply(p.clone().into_event());
                b.apply(p.clone().into_event()).unwrap();
            }
            assert!(!a.has_message(m.id), "right first: {right_first}");
            reconcile(&mut a, &mut b, now);
            assert_eq!(digests(&a, now), digests(&b, now), "right first: {right_first}");
            assert_eq!(reconcile(&mut a, &mut b, now), (vec![], vec![]));
            // Only the one that removed it counts as accepted, after a
            // restart too.
            let restored = ServerState::from_snapshot(a.snapshot());
            assert_eq!(restored.snapshot().accepted, vec![(m.id, author, channel, author)]);
            // A late copy stays out on both.
            for state in [&mut a, &mut b] {
                state.apply(ServerEvent::MessageCreate { message: m.clone() }).unwrap();
                assert!(!state.has_message(m.id));
            }
        }
    }

    #[test]
    fn a_message_taken_in_drops_the_proofs_naming_another_channel() {
        let (mut a, channel, _) = fixture();
        let author = join(&mut a, "author");
        let moderator = join_moderator(&mut a, "moderator");
        let mut b = a.clone();
        let first = post(&a, channel, author, "first");
        let now = Utc::now();
        a.apply(ServerEvent::MessageCreate { message: first.clone() }).unwrap();
        a.apply(proof(&a, &first, moderator, now).into_event()).unwrap();
        // The author signs another message under the same id elsewhere.
        let voice = a.server.channels[1].id;
        let again = signed(&a, Message { channel: voice, content: "again".into(), ..first.clone() });
        for state in [&mut a, &mut b] {
            assert_eq!(state.apply(ServerEvent::MessageCreate { message: again.clone() }), Ok(true));
        }
        assert!(a.tombstone(first.id).is_none());
        assert_eq!(digests(&a, now), digests(&b, now));
    }

    #[test]
    fn another_deleters_proof_does_not_hide_the_authors_from_catch_up() {
        let (mut n, channel, _) = fixture();
        let author = join(&mut n, "author");
        let moderator = join_moderator(&mut n, "moderator");
        let mut q = n.clone();
        let x = post(&n, channel, author, "mine");
        let now = Utc::now();
        q.apply(ServerEvent::MessageCreate { message: x.clone() }).unwrap();
        // The author deletes on q; n only got a moderator's proof, the same day.
        q.apply(proof(&q, &x, author, now).into_event()).unwrap();
        n.apply(proof(&n, &x, moderator, now).into_event()).unwrap();
        reconcile(&mut n, &mut q, now);
        assert!(n.tombstones_for(x.id).iter().any(|p| p.by == author));
        assert!(q.tombstones_for(x.id).iter().any(|p| p.by == moderator));
        // Even once the moderator's own no longer counts, a stale replica's
        // late copy stays out on n.
        let role = n.server.member(moderator).unwrap().roles[0];
        n.apply(n.role_change(moderator, role, false, now)).unwrap();
        n.apply(ServerEvent::MessageCreate { message: x.clone() }).unwrap();
        assert!(!n.has_message(x.id));
    }

    /// Demotes `user` from `role` at `at`, or has them leave then.
    fn step_down(state: &ServerState, how: &str, user: crate::ids::UserId, role: crate::ids::RoleId, at: DateTime<Utc>) -> ServerEvent {
        match how {
            "demote" => state.role_change(user, role, false, at),
            _ => ServerEvent::MemberLeave { server: state.server.id, user, at: at.timestamp_millis() },
        }
    }

    #[test]
    fn a_moderators_delete_reaches_stale_replicas_after_a_demotion_or_leave() {
        for how in ["demote", "leave"] {
            let (mut a, channel, _) = fixture();
            let author = join(&mut a, "author");
            let moderator = join_moderator(&mut a, "moderator");
            let role = a.server.member(moderator).unwrap().roles[0];
            let mut lacking = a.clone();
            let spam = post(&a, channel, author, "spam");
            a.apply(ServerEvent::MessageCreate { message: spam.clone() }).unwrap();
            let mut holding = a.clone();
            assert_eq!(a.apply(delete(&a, &spam, moderator)), Ok(true));
            // The stale replicas hear of the demotion or leave first.
            let later = Utc::now() + Duration::seconds(1);
            let out = step_down(&a, how, moderator, role, later);
            for state in [&mut a, &mut holding, &mut lacking] {
                assert_eq!(state.apply(out.clone()), Ok(true), "{how}");
            }
            let now = Utc::now();
            for stale in [&mut holding, &mut lacking] {
                assert_eq!(reconcile(&mut a, stale, now).0, vec![spam.id], "{how}");
                assert!(!stale.has_message(spam.id), "{how}");
                assert_eq!(digests(&a, now), digests(stale, now), "{how}");
                assert_eq!(reconcile(&mut a, stale, now), (vec![], vec![]), "{how}");
            }
        }
    }

    #[test]
    fn a_removal_tells_a_replica_that_missed_the_grant_when_it_was_made() {
        for how in ["demote", "leave"] {
            let (mut a, channel, _) = fixture();
            let sid = a.server.id;
            let author = join(&mut a, "author");
            let moderator = join(&mut a, "moderator");
            let mods = crate::server::Role {
                id: crate::ids::RoleId::new(),
                name: "mods".into(),
                color: None,
                permissions: crate::permissions::Permissions::MANAGE_MESSAGES,
                position: 1,
                hoist: false,
                self_assignable: false,
                tag: None,
                banner: None,
                updated_at: 1,
            };
            a.apply(ServerEvent::RoleCreate { server: sid, role: mods.clone() }).unwrap();
            let spam = post(&a, channel, author, "spam");
            a.apply(ServerEvent::MessageCreate { message: spam.clone() }).unwrap();
            // This replica misses the grant.
            let mut unaware = a.clone();
            a.apply(a.role_change(moderator, mods.id, true, Utc::now())).unwrap();
            assert_eq!(a.apply(delete(&a, &spam, moderator)), Ok(true));
            let out = step_down(&a, how, moderator, mods.id, Utc::now() + Duration::seconds(1));
            a.apply(out.clone()).unwrap();
            unaware.apply(out).unwrap();
            // What the removal tells comes with it, or with the marks
            // announced again once the moderator is gone.
            for (_, event) in a.changes_since(0, None) {
                let _ = unaware.apply(event);
            }
            let now = Utc::now();
            reconcile(&mut a, &mut unaware, now);
            assert!(!unaware.has_message(spam.id), "{how}");
            assert_eq!(digests(&a, now), digests(&unaware, now), "{how}");
        }
    }

    #[test]
    fn a_delete_dated_before_its_message_is_refused() {
        let (mut state, channel, _) = fixture();
        let author = join(&mut state, "author");
        let moderator = join_moderator(&mut state, "moderator");
        let role = state.server.member(moderator).unwrap().roles[0];
        let now = Utc::now();
        state.apply(state.role_change(moderator, role, false, now - Duration::minutes(30))).unwrap();
        let mut lacking = state.clone();
        // Written after the demotion, deleted by a proof dated in the
        // moderator's time.
        let msg = post(&state, channel, author, "after");
        state.apply(ServerEvent::MessageCreate { message: msg.clone() }).unwrap();
        let backdated = proof(&state, &msg, moderator, now - Duration::hours(1));
        assert_eq!(state.apply(backdated.clone().into_event()), Err(crate::store::StoreError::NotAllowed));
        assert!(state.has_message(msg.id) && state.tombstone(msg.id).is_none());
        // Without the message it is kept, and goes once the message arrives.
        assert_eq!(lacking.apply(backdated.into_event()), Ok(true));
        reconcile(&mut state, &mut lacking, now);
        assert!(lacking.has_message(msg.id) && lacking.tombstone(msg.id).is_none());
        assert_eq!(digests(&state, now), digests(&lacking, now));
    }

    #[test]
    fn messages_and_deletes_saved_by_older_versions_stay_out() {
        let (mut base, channel, _) = fixture();
        let author = join(&mut base, "author");
        let moderator = join_moderator(&mut base, "moderator");
        let mut other = base.clone();
        // Older versions made ids at random and signed nothing.
        let legacy = |text: &str| Message { id: MessageId::new(), ..Message::new(channel, author, text) };
        let (edited, own, moderated) = (legacy("edited"), legacy("own"), legacy("moderated"));
        let mut snapshot = base.snapshot();
        for (c, list) in snapshot.messages.iter_mut() {
            if *c == channel {
                list.extend([edited.clone(), own.clone(), moderated.clone()]);
            }
        }
        let mut state = ServerState::from_snapshot(snapshot);
        let now = Utc::now();
        assert!(digests(&state, now).is_empty());
        // Once edited it is signed, but its id would still be refused.
        state.apply(edit(&state, &edited, "new", Utc::now())).unwrap();
        assert!(state.message(edited.id).unwrap().sig.is_some());
        // So would its author's delete where the message is missing; a
        // moderator's would not.
        let by_moderator = proof(&state, &moderated, moderator, Utc::now());
        assert_eq!(state.apply(delete(&state, &own, author)), Ok(true));
        assert_eq!(state.apply(by_moderator.clone().into_event()), Ok(true));
        assert!(!state.has_message(own.id) && !state.has_message(moderated.id));
        let key = BucketKey::Tombstones { day: day_number(by_moderator.at) };
        let keys: Vec<BucketKey> = digests(&state, now).iter().map(|d| d.key).collect();
        assert_eq!(keys, vec![key]);
        assert_eq!(items(&state, &[key], now), vec![(key, vec![(moderated.id, proof_version(&by_moderator))])]);
        reconcile(&mut state, &mut other, now);
        assert_eq!(digests(&state, now), digests(&other, now));
        assert_eq!(reconcile(&mut state, &mut other, now), (vec![], vec![]));
    }
}
