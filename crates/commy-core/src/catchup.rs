//! Catch-up reconciliation. Two peers compare per-day digests of the
//! recent history they hold, list the items of the days that differ, and
//! then swap only what one side lacks. Pure logic; the wire protocol
//! lives in `commy-net`.

use crate::ids::{ChannelId, MessageId};
use crate::message::Message;
use crate::store::ServerState;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// UTC days of history covered, counting today.
pub const CATCHUP_DAYS: u32 = 30;
/// Most items sent each way in one session.
pub const CATCHUP_MAX_ITEMS: usize = 5000;

const DAY_MS: i64 = 86_400_000;
const DOMAIN: &[u8] = b"commy-catchup/v1";

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

/// A bucket's items as (id, version) pairs.
pub type BucketItems = (BucketKey, Vec<(MessageId, i64)>);

/// Days since the epoch, UTC. Times before the epoch count as day 0.
pub fn day_number(t: DateTime<Utc>) -> u32 {
    t.timestamp_millis().div_euclid(DAY_MS).clamp(0, u32::MAX as i64) as u32
}

/// First day inside the window that ends today.
pub fn first_day(now: DateTime<Utc>) -> u32 {
    day_number(now).saturating_sub(CATCHUP_DAYS - 1)
}

fn day_start(day: u32) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(day as i64 * DAY_MS).unwrap_or(DateTime::<Utc>::MIN_UTC)
}

/// Edit time in ms, or 0 for a message never edited.
pub fn message_version(message: &Message) -> i64 {
    message.edited_at.map_or(0, |t| t.timestamp_millis())
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

/// Messages of a channel that catch-up carries: signed (unsigned ones
/// from older versions would be refused anyway), inside retention, and
/// created at or after `from` when given.
fn covered(
    state: &ServerState,
    channel: ChannelId,
    from: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> impl Iterator<Item = &Message> {
    let since = match (state.retention_cutoff(channel, now), from) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    state
        .messages(channel)
        .iter()
        .filter(move |m| m.sig.is_some() && !since.is_some_and(|s| m.created_at < s))
}

/// Digests of every non-empty bucket in the window, sorted by key.
pub fn digests(state: &ServerState, now: DateTime<Utc>) -> Vec<BucketDigest> {
    let from = day_start(first_day(now));
    let mut buckets: BTreeMap<BucketKey, (u32, [u8; 16])> = BTreeMap::new();
    let mut add = |key: BucketKey, id: MessageId, version: i64| {
        let (count, hash) = buckets.entry(key).or_insert((0, [0; 16]));
        *count += 1;
        for (h, b) in hash.iter_mut().zip(item_hash(id, version)) {
            *h ^= b;
        }
    };
    for channel in &state.server.channels {
        for m in covered(state, channel.id, Some(from), now) {
            let key = BucketKey::Messages { channel: channel.id, day: day_number(m.created_at) };
            add(key, m.id, message_version(m));
        }
    }
    for proof in state.tombstones_since(from) {
        add(BucketKey::Tombstones { day: day_number(proof.at) }, proof.message, 0);
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

/// Items of the requested buckets, one entry per distinct key (empty
/// when we hold nothing there), sorted by key. Keys outside our window
/// are answered too, so peers whose clocks straddle midnight agree.
pub fn items(state: &ServerState, keys: &[BucketKey], now: DateTime<Utc>) -> Vec<BucketItems> {
    let mut out: BTreeMap<BucketKey, Vec<(MessageId, i64)>> = BTreeMap::new();
    let mut channel_days: HashMap<ChannelId, HashSet<u32>> = HashMap::new();
    let mut tombstone_days: HashSet<u32> = HashSet::new();
    for key in keys {
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
        for m in covered(state, *channel, None, now) {
            let day = day_number(m.created_at);
            if days.contains(&day) {
                let key = BucketKey::Messages { channel: *channel, day };
                out.entry(key).or_default().push((m.id, message_version(m)));
            }
        }
    }
    if !tombstone_days.is_empty() {
        for proof in state.tombstones_since(DateTime::<Utc>::MIN_UTC) {
            let day = day_number(proof.at);
            if tombstone_days.contains(&day) {
                out.entry(BucketKey::Tombstones { day }).or_default().push((proof.message, 0));
            }
        }
    }
    for list in out.values_mut() {
        list.sort_unstable();
    }
    out.into_iter().collect()
}

/// Compares item lists of the same buckets. Returns the ids the peer
/// lacks (absent there or at a lower version) and the ids we lack, each
/// newest first and without duplicates. Not capped, see [`cap`].
pub fn diff(ours: &[BucketItems], theirs: &[BucketItems]) -> (Vec<MessageId>, Vec<MessageId>) {
    (lacking(ours, theirs), lacking(theirs, ours))
}

/// Ids in `have` that `other` lacks or holds at a lower version.
fn lacking(have: &[BucketItems], other: &[BucketItems]) -> Vec<MessageId> {
    let mut index: HashMap<BucketKey, HashMap<MessageId, i64>> = HashMap::new();
    for (key, list) in other {
        let bucket = index.entry(*key).or_default();
        for &(id, version) in list {
            let v = bucket.entry(id).or_insert(version);
            *v = (*v).max(version);
        }
    }
    let mut found: Vec<(u32, i64, MessageId)> = Vec::new();
    for (key, list) in have {
        let bucket = index.get(key);
        for &(id, version) in list {
            if bucket.and_then(|b| b.get(&id)).is_none_or(|v| *v < version) {
                found.push((key.day(), version, id));
            }
        }
    }
    found.sort_unstable_by(|a, b| b.cmp(a));
    let mut seen = HashSet::new();
    found
        .into_iter()
        .filter_map(|(_, _, id)| seen.insert(id).then_some(id))
        .collect()
}

/// Keeps the first [`CATCHUP_MAX_ITEMS`] ids (the newest, for lists
/// from [`diff`]) and returns how many were dropped.
pub fn cap(ids: &mut Vec<MessageId>) -> usize {
    let dropped = ids.len().saturating_sub(CATCHUP_MAX_ITEMS);
    ids.truncate(CATCHUP_MAX_ITEMS);
    dropped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ServerEvent;
    use crate::test_support::{delete, edit, fixture, join, post, proof, signed};
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
            if let Some(p) = from.tombstone(*id) {
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
        let m = post(&state, channel, owner, "bye");
        state.apply(ServerEvent::MessageCreate { message: m.clone() }).unwrap();
        let p = proof(&state, &m, owner, now - Duration::days(1));
        state.apply(p.into_event()).unwrap();
        let list = digests(&state, now);
        assert_eq!(list.len(), 1);
        let key = BucketKey::Tombstones { day: day_number(now) - 1 };
        assert_eq!(list[0], BucketDigest { key, count: 1, hash: item_hash(m.id, 0) });
        assert_eq!(items(&state, &[key], now), vec![(key, vec![(m.id, 0)])]);
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
        let other = BucketKey::Messages { channel: ChannelId::new(), day: 4 };
        let out = items(&state, &[today, other, today], now);
        assert_eq!(out.len(), 2, "duplicates collapse");
        assert!(out.contains(&(today, vec![(m.id, 0)])));
        assert!(out.contains(&(other, vec![])));

        // Days before our window are answered on request.
        let mut old = Message::new(channel, owner, "old");
        old.created_at = now - Duration::days(45);
        let old = signed(&state, old);
        state.apply(ServerEvent::MessageCreate { message: old.clone() }).unwrap();
        let old_key = BucketKey::Messages { channel, day: day_number(old.created_at) };
        assert_eq!(items(&state, &[old_key], now), vec![(old_key, vec![(old.id, 0)])]);
    }

    #[test]
    fn diff_finds_missing_and_older_items() {
        let k1 = BucketKey::Tombstones { day: 10 };
        let k2 = BucketKey::Tombstones { day: 12 };
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
}
