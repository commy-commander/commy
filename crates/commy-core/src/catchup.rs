use crate::event::ServerEvent;
use crate::ids::{ChannelId, MessageId, UserId};
use crate::message::{Message, TombstoneProof};
use crate::store::{ReactionMark, ServerState, MAX_ACTION_AHEAD_MS, TOMBSTONE_KEEP_DAYS};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::RangeInclusive;

pub const CATCHUP_DAYS: u32 = 30;
pub const TOMBSTONE_DAYS: u32 = TOMBSTONE_KEEP_DAYS as u32;
pub const CATCHUP_MAX_ITEMS: usize = 5000;
pub const MAX_REACTION_CHANGES: usize = 2000;

const DAY_MS: i64 = 86_400_000;
const DOMAIN: &[u8] = b"commy-catchup/v1";
const PROOF_DOMAIN: &[u8] = b"commy-catchup/v1/proof";
const SIG_BITS: u32 = 20;
const DAYS_AHEAD_FOR_PEERS_PAST_MIDNIGHT: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum BucketKey {
    Messages { channel: ChannelId, day: u32 },
    Tombstones { day: u32 },
}

impl BucketKey {
    pub fn day(&self) -> u32 {
        match self {
            BucketKey::Messages { day, .. } | BucketKey::Tombstones { day } => *day,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BucketDigest {
    pub key: BucketKey,
    pub count: u32,
    pub hash: [u8; 16],
}

pub type BucketItems = (BucketKey, Vec<(MessageId, i64)>);

pub fn day_number(t: DateTime<Utc>) -> u32 {
    t.timestamp_millis().div_euclid(DAY_MS).clamp(0, u32::MAX as i64) as u32
}

pub fn first_day(now: DateTime<Utc>) -> u32 {
    day_number(now).saturating_sub(CATCHUP_DAYS - 1)
}

pub fn first_tombstone_day(now: DateTime<Utc>) -> u32 {
    day_number(now).saturating_sub(TOMBSTONE_DAYS - 1)
}

pub fn last_day(now: DateTime<Utc>) -> u32 {
    day_number(now).saturating_add(DAYS_AHEAD_FOR_PEERS_PAST_MIDNIGHT)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub messages: RangeInclusive<u32>,
    pub tombstones: RangeInclusive<u32>,
}

impl Window {
    pub fn contains(&self, key: &BucketKey) -> bool {
        match key {
            BucketKey::Messages { day, .. } => self.messages.contains(day),
            BucketKey::Tombstones { day } => self.tombstones.contains(day),
        }
    }
}

pub fn window(now: DateTime<Utc>) -> Window {
    Window { messages: first_day(now)..=last_day(now), tombstones: first_tombstone_day(now)..=last_day(now) }
}

pub fn current_window() -> Window {
    window(Utc::now())
}

pub fn in_window(key: &BucketKey, now: DateTime<Utc>) -> bool {
    window(now).contains(key)
}

fn day_start(day: u32) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(day as i64 * DAY_MS).unwrap_or(DateTime::<Utc>::MIN_UTC)
}

pub fn message_version(message: &Message) -> i64 {
    let edited = message.edited_at.map_or(0, |t| t.timestamp_millis()).clamp(0, i64::MAX >> SIG_BITS);
    let first = message.sig.as_ref().map_or(0, |s| u32::from_be_bytes([0, s.0[0], s.0[1], s.0[2]]) >> (24 - SIG_BITS));
    let mask = (1u32 << SIG_BITS) - 1;
    (edited << SIG_BITS) | i64::from(!first & mask)
}

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

fn covered(state: &ServerState, channel: ChannelId, now: DateTime<Utc>) -> impl Iterator<Item = &Message> {
    let from = day_start(first_day(now));
    let since = state.retention_cutoff(channel, now).map_or(from, |cutoff| cutoff.max(from));
    let until = day_start(last_day(now).saturating_add(1));
    state
        .messages(channel)
        .iter()
        .filter(move |m| m.created_at >= since && m.created_at < until && peers_would_accept(m))
}

fn peers_would_accept(message: &Message) -> bool {
    message.sig.is_some() && message.id.is_bound_to(message.author)
}

fn covered_tombstones(state: &ServerState, now: DateTime<Utc>) -> impl Iterator<Item = &TombstoneProof> {
    let until = day_start(last_day(now).saturating_add(1));
    state
        .tombstones_since(day_start(first_tombstone_day(now)))
        .filter(move |p| p.at < until && state.server.channel(p.channel).is_some())
}

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

pub fn items(state: &ServerState, keys: &[BucketKey], now: DateTime<Utc>) -> Vec<BucketItems> {
    let mut out: BTreeMap<BucketKey, Vec<(MessageId, i64)>> = BTreeMap::new();
    let mut channel_days: HashMap<ChannelId, HashSet<u32>> = HashMap::new();
    let mut tombstone_days: HashSet<u32> = HashSet::new();
    let window = window(now);
    for key in keys.iter().filter(|k| window.contains(k)) {
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

pub fn diff(ours: &[BucketItems], theirs: &[BucketItems]) -> (Vec<MessageId>, Vec<MessageId>) {
    (lacking(ours, theirs), lacking(theirs, ours))
}

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

    pub fn holds(&self, key: BucketKey, id: MessageId, version: i64) -> bool {
        match key {
            BucketKey::Tombstones { .. } => self.proofs.contains(&(key, id, version)),
            BucketKey::Messages { .. } => self.messages.get(&(key, id)).is_some_and(|v| *v >= version),
        }
    }
}

fn lacking(have: &[BucketItems], other: &[BucketItems]) -> Vec<MessageId> {
    lacking_from(have, &Listing::new(other))
}

pub fn lacking_from(have: &[BucketItems], other: &Listing) -> Vec<MessageId> {
    type IsMessageDayVersionId = (bool, u32, i64, MessageId);
    let mut found: Vec<IsMessageDayVersionId> = Vec::new();
    for (key, list) in have {
        let is_message = matches!(key, BucketKey::Messages { .. });
        for &(id, version) in list {
            if !other.holds(*key, id, version) {
                let ordering_version = if is_message { version } else { 0 };
                found.push((is_message, key.day(), ordering_version, id));
            }
        }
    }
    let messages_before_proofs_newest_first = |a: &IsMessageDayVersionId, b: &IsMessageDayVersionId| b.cmp(a);
    found.sort_unstable_by(messages_before_proofs_newest_first);
    let mut seen = HashSet::new();
    found
        .into_iter()
        .filter_map(|(_, _, _, id)| seen.insert(id).then_some(id))
        .collect()
}

pub fn cap(ids: &mut Vec<MessageId>) -> usize {
    let dropped = ids.len().saturating_sub(CATCHUP_MAX_ITEMS);
    ids.truncate(CATCHUP_MAX_ITEMS);
    dropped
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionChange {
    pub message: MessageId,
    pub emoji: String,
    pub at: i64,
    pub active: bool,
}

fn covered_messages(state: &ServerState, now: DateTime<Utc>) -> impl Iterator<Item = &Message> {
    state.server.channels.iter().flat_map(move |c| covered(state, c.id, now))
}

pub fn reaction_changes(state: &ServerState, user: UserId, now: DateTime<Utc>) -> Vec<ReactionChange> {
    let mut out = Vec::new();
    for m in covered_messages(state, now) {
        let marks = state.reaction_marks(m.id).iter().filter(|mark| mark.user == user);
        out.extend(marks.map(|mark| ReactionChange {
            message: m.id,
            emoji: mark.emoji.clone(),
            at: mark.at,
            active: mark.active,
        }));
        let unmarked = m
            .reactions
            .iter()
            .filter(|r| r.users.contains(&user) && state.reaction_mark(m.id, user, &r.emoji).is_none());
        out.extend(unmarked.map(|r| ReactionChange { message: m.id, emoji: r.emoji.clone(), at: 0, active: true }));
    }
    out.sort_unstable_by_key(|c| std::cmp::Reverse(c.at));
    out.truncate(MAX_REACTION_CHANGES);
    out
}

pub fn apply_reaction_changes(
    state: &mut ServerState,
    user: UserId,
    changes: Vec<ReactionChange>,
    now: DateTime<Utc>,
) -> usize {
    if changes.len() > MAX_REACTION_CHANGES {
        return 0;
    }
    let latest = now.timestamp_millis().saturating_add(MAX_ACTION_AHEAD_MS);
    let channels: HashMap<MessageId, ChannelId> = covered_messages(state, now).map(|m| (m.id, m.channel)).collect();
    let mut changed = 0;
    for c in changes {
        let Some(&channel) = channels.get(&c.message) else { continue };
        let newer = |m: &ReactionMark| c.at > m.at || (c.at == m.at && m.active && !c.active);
        if c.at > latest || state.reaction_mark(c.message, user, &c.emoji).is_some_and(|m| !newer(m)) {
            continue;
        }
        let event = ServerEvent::ReactionSet { channel, message: c.message, emoji: c.emoji.clone(), user, active: c.active };
        match state.apply(event) {
            Ok(true) => changed += 1,
            Ok(false) => {}
            Err(_) => continue,
        }
        if crate::server::valid_reaction(&c.emoji) {
            state.mark_reaction(c.message, user, &c.emoji, c.at, c.active);
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ServerEvent;
    use crate::test_support::{delete, edit, fixture, join, join_moderator, post, post_at, proof, signed, signed_event};
    use chrono::Duration;

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
        assert_eq!(first_tombstone_day(at(20_000, 123)), 20_000 - 59);
        let now = at(20_000, 5);
        let messages = |day| BucketKey::Messages { channel: ChannelId::new(), day };
        assert!(in_window(&messages(20_000 - 29), now) && in_window(&messages(20_001), now));
        assert!(!in_window(&messages(20_000 - 30), now) && !in_window(&messages(20_002), now));
        let tombstones = |day| BucketKey::Tombstones { day };
        assert!(in_window(&tombstones(20_000 - 59), now) && in_window(&tombstones(20_001), now));
        assert!(!in_window(&tombstones(20_000 - 60), now) && !in_window(&tombstones(20_002), now));
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
            ids.push((m.id, message_version(&m), age));
            state.apply(ServerEvent::MessageCreate { message: m }).unwrap();
        }
        let list = digests(&state, now);
        assert_eq!(list.len(), 2);
        let today_bucket = list.iter().find(|d| d.key == BucketKey::Messages { channel, day: today }).unwrap();
        assert_eq!(today_bucket.count, 2);
        let mut expected = [0u8; 16];
        for (id, version, _) in ids.iter().filter(|(_, _, age)| *age == 0) {
            for (e, b) in expected.iter_mut().zip(item_hash(*id, *version)) {
                *e ^= b;
            }
        }
        assert_eq!(today_bucket.hash, expected);
        assert!(list.iter().any(|d| d.key == BucketKey::Messages { channel, day: today - 2 } && d.count == 1));
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
        assert!(out.contains(&(today, vec![(m.id, message_version(&m))])));
        assert!(out.contains(&(other, vec![])));

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
        assert_eq!(items(&state, &[key], now), vec![(key, vec![(tomorrow.id, message_version(&tomorrow))])]);
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
        assert_eq!(they_lack, vec![d, b, a]);
        assert_eq!(we_want, vec![c]);
        assert_eq!(diff(&ours, &ours), (vec![], vec![]));
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
        assert_eq!(reconcile(&mut a, &mut b, now), (vec![], vec![]));
    }

    #[test]
    fn late_tombstone_wins_over_a_peer_still_holding_the_message() {
        let (mut a, channel, owner) = fixture();
        let mut b = a.clone();
        let now = Utc::now();
        let m = post(&a, channel, owner, "gone soon");
        a.apply(ServerEvent::MessageCreate { message: m.clone() }).unwrap();
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
            let restored = ServerState::from_snapshot(a.snapshot());
            assert_eq!(restored.snapshot().accepted, vec![(m.id, author, channel, author)]);
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
        q.apply(proof(&q, &x, author, now).into_event()).unwrap();
        n.apply(proof(&n, &x, moderator, now).into_event()).unwrap();
        reconcile(&mut n, &mut q, now);
        assert!(n.tombstones_for(x.id).iter().any(|p| p.by == author));
        assert!(q.tombstones_for(x.id).iter().any(|p| p.by == moderator));
        assert_eq!(digests(&n, now), digests(&q, now));
        n.apply(ServerEvent::MessageCreate { message: x.clone() }).unwrap();
        assert!(!n.has_message(x.id));
    }

    fn step_down(state: &ServerState, how: &str, user: crate::ids::UserId, role: crate::ids::RoleId) -> ServerEvent {
        let server = state.server.id;
        match how {
            "demote" => ServerEvent::RoleUnassign { server, user, role },
            _ => ServerEvent::MemberLeave { server, user },
        }
    }

    #[test]
    fn a_moderators_delete_reaches_stale_replicas_after_a_demotion_or_leave() {
        for how in ["demote", "leave"] {
            let (mut a, channel, owner) = fixture();
            let author = join(&mut a, "author");
            let moderator = join_moderator(&mut a, "moderator");
            let role = a.server.member(moderator).unwrap().roles[0];
            let mut lacking = a.clone();
            let spam = post(&a, channel, author, "spam");
            a.apply(ServerEvent::MessageCreate { message: spam.clone() }).unwrap();
            let mut holding = a.clone();
            let removal = delete(&a, &spam, moderator);
            assert_eq!(a.apply(removal.clone()), Ok(true));
            let signer = if how == "demote" { owner } else { moderator };
            let out = signed_event(&a, signer, Utc::now() + Duration::seconds(1), step_down(&a, how, moderator, role));
            for state in [&mut a, &mut holding, &mut lacking] {
                assert_eq!(state.apply_signed(&out), Ok(true), "{how}");
            }
            let now = Utc::now();
            assert_eq!(reconcile(&mut a, &mut holding, now).0, vec![spam.id], "{how}");
            assert!(!holding.has_message(spam.id), "{how}");
            assert_eq!(digests(&a, now), digests(&holding, now), "{how}");
            assert_eq!(reconcile(&mut a, &mut holding, now), (vec![], vec![]), "{how}");
            if how == "leave" {
                assert_eq!(lacking.apply(removal.clone()), Ok(true));
                lacking.apply(ServerEvent::MessageCreate { message: spam.clone() }).unwrap();
            }
            reconcile(&mut a, &mut lacking, now);
            assert!(!lacking.has_message(spam.id), "{how}");
            assert_eq!(digests(&a, now), digests(&lacking, now), "{how}");
        }
    }

    #[test]
    fn deletes_from_a_demoted_and_a_regranted_moderator_converge_everywhere() {
        let (mut base, channel, owner) = fixture();
        let sid = base.server.id;
        let author = join(&mut base, "author");
        let moderator = join_moderator(&mut base, "moderator");
        let role = base.server.member(moderator).unwrap().roles[0];
        let [first, second] = ["first", "second"].map(|text| post(&base, channel, author, text));
        for m in [&first, &second] {
            base.apply(ServerEvent::MessageCreate { message: m.clone() }).unwrap();
        }
        let start = Utc::now();
        let at = |s: i64| start + Duration::seconds(s);
        let demote = signed_event(&base, owner, at(1), ServerEvent::RoleUnassign { server: sid, user: moderator, role });
        let regrant = signed_event(&base, owner, at(2), ServerEvent::RoleAssign { server: sid, user: moderator, role });
        let before = proof(&base, &first, moderator, at(0)).into_event();
        let after = proof(&base, &second, moderator, at(3)).into_event();
        let mut saw_all = base.clone();
        assert_eq!(saw_all.apply(before.clone()), Ok(true));
        for role_change in [&demote, &regrant] {
            assert_eq!(saw_all.apply_signed(role_change), Ok(true));
        }
        assert_eq!(saw_all.apply(after.clone()), Ok(true));
        let mut demoted_first = base.clone();
        assert_eq!(demoted_first.apply_signed(&demote), Ok(true));
        assert_eq!(demoted_first.apply(before.clone()), Ok(true), "signed while still a moderator");
        assert_eq!(demoted_first.apply(after.clone()), Ok(false), "signed after the demotion as far as it knows");
        assert!(demoted_first.has_message(second.id));
        assert_eq!(demoted_first.apply_signed(&regrant), Ok(true));
        assert!(!demoted_first.has_message(second.id), "applies once the regrant arrives");
        let mut late_regrant = base.clone();
        assert_eq!(late_regrant.apply(after.clone()), Ok(true));
        for role_change in [&demote, &regrant] {
            assert_eq!(late_regrant.apply_signed(role_change), Ok(true));
        }
        let mut no_roles = base.clone();
        no_roles.apply_signed(&demote).unwrap();
        no_roles.apply_signed(&regrant).unwrap();
        let mut replicas = [saw_all, demoted_first, late_regrant, no_roles, base];
        let now = Utc::now();
        for _ in 0..2 {
            for i in 0..replicas.len() {
                for j in i + 1..replicas.len() {
                    let (left, right) = replicas.split_at_mut(j);
                    reconcile(&mut left[i], &mut right[0], now);
                }
            }
        }
        let expected = digests(&replicas[0], now);
        for (i, state) in replicas.iter().enumerate() {
            assert!(!state.has_message(first.id) && !state.has_message(second.id), "replica {i}");
            assert_eq!(digests(state, now), expected, "replica {i}");
        }
    }

    #[test]
    fn legacy_messages_stay_out_but_their_deletes_reach_replicas_holding_them() {
        let (mut base, channel, _) = fixture();
        let author = join(&mut base, "author");
        let moderator = join_moderator(&mut base, "moderator");
        let mut lacking = base.clone();
        let legacy = |text: &str| Message { id: MessageId::new(), ..Message::new(channel, author, text) };
        let (edited, own, others) = (legacy("edited"), legacy("own"), legacy("others"));
        let mut snapshot = base.snapshot();
        for (c, list) in snapshot.messages.iter_mut() {
            if *c == channel {
                list.extend([edited.clone(), own.clone(), others.clone()]);
            }
        }
        let mut state = ServerState::from_snapshot(snapshot.clone());
        let mut holding = ServerState::from_snapshot(snapshot);
        let now = Utc::now();
        assert!(digests(&state, now).is_empty());
        state.apply(edit(&state, &edited, "new", Utc::now())).unwrap();
        assert!(state.message(edited.id).unwrap().sig.is_some());
        assert!(digests(&state, now).is_empty());
        let by_author = proof(&state, &own, author, Utc::now());
        let by_moderator = proof(&state, &others, moderator, Utc::now());
        for p in [&by_author, &by_moderator] {
            assert_eq!(state.apply(p.clone().into_event()), Ok(true));
        }
        assert!(!state.has_message(own.id) && !state.has_message(others.id));
        let day = |p: &TombstoneProof| BucketKey::Tombstones { day: day_number(p.at) };
        let keys: Vec<BucketKey> = digests(&state, now).iter().map(|d| d.key).collect();
        assert_eq!(keys, dedup_keys([day(&by_author), day(&by_moderator)]));
        for other in [&mut holding, &mut lacking] {
            reconcile(&mut state, other, now);
            assert!(!other.has_message(own.id) && !other.has_message(others.id));
            assert_eq!(digests(&state, now), digests(other, now));
            assert_eq!(reconcile(&mut state, other, now), (vec![], vec![]));
        }
        assert!(holding.has_message(edited.id), "the legacy message itself stays where it is");
    }

    fn dedup_keys(keys: impl IntoIterator<Item = BucketKey>) -> Vec<BucketKey> {
        let set: BTreeSet<BucketKey> = keys.into_iter().collect();
        set.into_iter().collect()
    }

    #[test]
    fn a_delete_made_before_the_days_of_messages_reaches_a_newcomer() {
        let (mut old, channel, _) = fixture();
        let author = join(&mut old, "author");
        let mut newcomer = old.clone();
        let now = Utc::now();
        let m = post_at(&old, channel, author, "regretted", now - Duration::days(45));
        old.apply(ServerEvent::MessageCreate { message: m.clone() }).unwrap();
        old.apply(proof(&old, &m, author, now - Duration::days(40)).into_event()).unwrap();
        reconcile(&mut old, &mut newcomer, now);
        assert!(newcomer.tombstone(m.id).is_some());
        assert_eq!(digests(&old, now), digests(&newcomer, now));
        newcomer.apply(ServerEvent::MessageCreate { message: m.clone() }).unwrap();
        assert!(!newcomer.has_message(m.id));
    }

    #[test]
    fn replicas_holding_two_copies_signed_for_one_time_converge() {
        for edited in [false, true] {
            let (mut a, channel, _) = fixture();
            let author = join(&mut a, "author");
            let mut b = a.clone();
            let now = Utc::now();
            let base = post(&a, channel, author, "base");
            let edited_at = edited.then_some(now);
            let one = signed(&a, Message { content: "one".into(), edited_at, ..base.clone() });
            let two = signed(&a, Message { content: "two".into(), edited_at, ..base.clone() });
            a.apply(ServerEvent::MessageCreate { message: one.clone() }).unwrap();
            b.apply(ServerEvent::MessageCreate { message: two.clone() }).unwrap();
            assert_ne!(digests(&a, now), digests(&b, now), "edited: {edited}");
            reconcile(&mut a, &mut b, now);
            let winner = if one.sig.unwrap().0 < two.sig.unwrap().0 { &one } else { &two };
            for state in [&a, &b] {
                assert_eq!(state.message(base.id).unwrap().content, winner.content, "edited: {edited}");
            }
            assert_eq!(digests(&a, now), digests(&b, now), "edited: {edited}");
            assert_eq!(reconcile(&mut a, &mut b, now), (vec![], vec![]));
        }
    }

    #[test]
    fn versions_order_edits_by_time_then_by_lower_signature() {
        let (state, channel, owner) = fixture();
        let base = post(&state, channel, owner, "base");
        let now = Utc::now();
        let at = |content: &str, t| signed(&state, Message { content: content.into(), edited_at: t, ..base.clone() });
        let later = at("later", Some(now + Duration::milliseconds(1)));
        for content in ["one", "two", "three"] {
            let copy = at(content, Some(now));
            assert!(message_version(&later) > message_version(&copy));
            assert!(message_version(&copy) > message_version(&base));
        }
        let (one, two) = (at("one", Some(now)), at("two", Some(now)));
        assert_ne!(message_version(&one), message_version(&two), "copies for one time differ");
        let lower_first = (one.sig.unwrap().0 < two.sig.unwrap().0) == (message_version(&one) > message_version(&two));
        assert!(lower_first, "the lower signature has the higher version");
        let far = at("far", Some(DateTime::<Utc>::MAX_UTC));
        assert!(message_version(&far) > message_version(&later));
    }

    fn react(state: &mut ServerState, message: &Message, user: UserId, emoji: &str, active: bool) {
        let event = ServerEvent::ReactionSet { channel: message.channel, message: message.id, emoji: emoji.into(), user, active };
        state.apply(event).unwrap();
    }

    fn reactors(state: &ServerState, id: MessageId, emoji: &str) -> Vec<UserId> {
        let message = state.message(id).unwrap();
        message.reactions.iter().find(|r| r.emoji == emoji).map(|r| r.users.clone()).unwrap_or_default()
    }

    fn two_replicas() -> (ServerState, ServerState, Message, UserId) {
        let (mut a, channel, owner) = fixture();
        let friend = join(&mut a, "friend");
        let message = post(&a, channel, owner, "hello");
        a.apply(ServerEvent::MessageCreate { message: message.clone() }).unwrap();
        let b = ServerState::from_snapshot(a.snapshot());
        (a, b, message, friend)
    }

    #[test]
    fn reactions_made_while_a_member_was_away_reach_them() {
        let (mut a, mut b, message, friend) = two_replicas();
        react(&mut a, &message, friend, "👍", true);
        react(&mut a, &message, friend, ":party:", true);
        let now = Utc::now();
        assert_eq!(apply_reaction_changes(&mut b, friend, reaction_changes(&a, friend, now), now), 2);
        assert_eq!(reactors(&b, message.id, "👍"), vec![friend]);
        assert_eq!(reactors(&b, message.id, ":party:"), vec![friend]);
        assert_eq!(apply_reaction_changes(&mut b, friend, reaction_changes(&a, friend, now), now), 0, "nothing moves twice");
    }

    #[test]
    fn a_reaction_taken_back_while_a_member_was_away_goes_for_them_too() {
        let (mut a, mut b, message, friend) = two_replicas();
        react(&mut a, &message, friend, "👍", true);
        react(&mut b, &message, friend, "👍", true);
        b.mark_reaction(message.id, friend, "👍", 1_000, true);
        a.mark_reaction(message.id, friend, "👍", 1_000, true);
        react(&mut a, &message, friend, "👍", false);
        let now = Utc::now();
        assert_eq!(apply_reaction_changes(&mut b, friend, reaction_changes(&a, friend, now), now), 1);
        assert!(reactors(&b, message.id, "👍").is_empty());
        assert_eq!(apply_reaction_changes(&mut a, friend, reaction_changes(&b, friend, now), now), 0, "the older add stays lost");
        assert!(reactors(&a, message.id, "👍").is_empty());
    }

    #[test]
    fn a_stale_device_never_undoes_a_newer_change() {
        let (mut laptop, mut desktop, message, friend) = two_replicas();
        react(&mut laptop, &message, friend, "👍", true);
        laptop.mark_reaction(message.id, friend, "👍", 1_000, true);
        react(&mut desktop, &message, friend, ":party:", true);
        let now = Utc::now();
        let mut other = ServerState::from_snapshot(desktop.snapshot());
        react(&mut desktop, &message, friend, ":party:", false);
        apply_reaction_changes(&mut other, friend, reaction_changes(&desktop, friend, now), now);
        assert!(reactors(&other, message.id, ":party:").is_empty());
        let stale = vec![ReactionChange { message: message.id, emoji: ":party:".into(), at: 2_000, active: true }];
        assert_eq!(apply_reaction_changes(&mut other, friend, stale, now), 0);
        assert!(reactors(&other, message.id, ":party:").is_empty());
        assert_eq!(apply_reaction_changes(&mut other, friend, reaction_changes(&laptop, friend, now), now), 1);
        assert_eq!(reactors(&other, message.id, "👍"), vec![friend]);
    }

    #[test]
    fn reactions_from_before_marks_travel_as_the_oldest_change() {
        let (mut a, mut b, mut message, friend) = two_replicas();
        message.reactions.push(crate::message::Reaction { emoji: "👍".into(), users: vec![friend] });
        let mut fresh = ServerState::from_snapshot(a.snapshot());
        let relayed = Message { id: MessageId::for_author(message.author), ..message.clone() };
        let relayed = crate::test_support::signed(&fresh, relayed);
        fresh.apply(ServerEvent::MessageCreate { message: relayed.clone() }).unwrap();
        a.apply(ServerEvent::MessageCreate { message: Message { reactions: vec![], ..relayed.clone() } }).unwrap();
        b.apply(ServerEvent::MessageCreate { message: Message { reactions: vec![], ..relayed.clone() } }).unwrap();
        let now = Utc::now();
        let changes = reaction_changes(&fresh, friend, now);
        assert_eq!(changes, vec![ReactionChange { message: relayed.id, emoji: "👍".into(), at: 0, active: true }]);
        assert_eq!(apply_reaction_changes(&mut a, friend, changes.clone(), now), 1);
        react(&mut b, &relayed, friend, "👍", true);
        react(&mut b, &relayed, friend, "👍", false);
        assert_eq!(apply_reaction_changes(&mut b, friend, changes, now), 0);
        assert!(reactors(&b, relayed.id, "👍").is_empty());
    }

    #[test]
    fn reaction_changes_are_bounded() {
        let (a, mut b, message, friend) = two_replicas();
        let now = Utc::now();
        let one = |at, emoji: &str| ReactionChange { message: message.id, emoji: emoji.into(), at, active: true };
        let ahead = now.timestamp_millis() + MAX_ACTION_AHEAD_MS + 1;
        assert_eq!(apply_reaction_changes(&mut b, friend, vec![one(ahead, "👍")], now), 0);
        assert_eq!(apply_reaction_changes(&mut b, friend, vec![one(5, "not an emoji")], now), 0);
        assert!(b.reaction_marks(message.id).is_empty());
        let flood = (0..=MAX_REACTION_CHANGES).map(|i| one(i as i64, "👍")).collect();
        assert_eq!(apply_reaction_changes(&mut b, friend, flood, now), 0);
        let unknown = ReactionChange { message: MessageId::new(), ..one(5, "👍") };
        assert_eq!(apply_reaction_changes(&mut b, friend, vec![unknown], now), 0);
        assert!(reaction_changes(&a, friend, now).is_empty());
    }

    #[test]
    fn reaction_marks_survive_a_restart() {
        let (mut a, _, message, friend) = two_replicas();
        react(&mut a, &message, friend, "👍", true);
        react(&mut a, &message, friend, "👍", false);
        let restored = ServerState::from_snapshot(serde_json::from_str(&serde_json::to_string(&a.snapshot()).unwrap()).unwrap());
        assert_eq!(restored.reaction_marks(message.id), a.reaction_marks(message.id));
        assert!(!restored.reaction_mark(message.id, friend, "👍").unwrap().active);
    }
}
