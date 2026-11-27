use crate::channel_key::{self, ChannelSecret, KeyId};
use crate::day_chain::{self, DayStep, DAY_MS};
use crate::event::ServerEvent;
use crate::identity::{Identity, IdentityKey};
use crate::ids::{ServerId, UserId};
use crate::store::ServerState;
use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use curve25519_dalek::montgomery::MontgomeryPoint;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

const GRANT_DOMAIN: &[u8] = b"commy-day-grant/v1";
const LINK_DOMAIN: &[u8] = b"commy-day-link/v1";
const SEAL_DOMAIN: &[u8] = b"commy-day-seal/v1";
const TAG_BYTES: usize = 16;
const SHARE_BYTES: usize = 32 + 4 + 32;
pub const SEALED_SHARE_BYTES: usize = SHARE_BYTES + TAG_BYTES;
pub const MAX_DAY_EPOCHS: usize = 64;
pub const MAX_DAY_LINKS: usize = 16;
pub const FRESH_EPOCH_DAYS: i64 = 7;

fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    postcard::to_stdvec(value).expect("postcard encodes plain tuples")
}

fn seal_with(key: &[u8; 32], aad: &[u8], plain: &[u8]) -> ([u8; 12], Vec<u8>) {
    let nonce: [u8; 12] = rand::random();
    let cipher = Aes256Gcm::new_from_slice(key).expect("32 byte key");
    let sealed = cipher.encrypt(&Nonce::from(nonce), Payload { msg: plain, aad }).expect("in-memory encryption cannot fail");
    (nonce, sealed)
}

fn open_with(key: &[u8; 32], nonce: [u8; 12], aad: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key).expect("32 byte key");
    cipher.decrypt(&Nonce::from(nonce), Payload { msg: sealed, aad }).ok()
}

#[derive(Clone, PartialEq, Eq)]
pub struct DayShare {
    addressing: ChannelSecret,
    step: DayStep,
}

impl fmt::Debug for DayShare {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DayShare(from day {}, ..)", self.step.day())
    }
}

impl DayShare {
    pub fn fresh() -> Self {
        let (addressing, step) = day_chain::split(&ChannelSecret::random());
        Self { addressing, step }
    }

    pub fn addressing(&self) -> &ChannelSecret {
        &self.addressing
    }

    pub fn first_day(&self) -> u32 {
        self.step.day()
    }

    pub fn step(&self, day: u32) -> Option<DayStep> {
        self.step.advance_to(day)
    }

    pub fn from_day(&self, day: u32) -> Self {
        let step = self.step.advance_to(day.max(self.step.day())).expect("a later day is always reachable");
        Self { addressing: self.addressing.clone(), step }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(SHARE_BYTES);
        out.extend_from_slice(self.addressing.bytes());
        out.extend_from_slice(&self.step.day().to_be_bytes());
        out.extend_from_slice(self.step.key_bytes());
        out
    }

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != SHARE_BYTES {
            return None;
        }
        let addressing = ChannelSecret::from_bytes(bytes[..32].try_into().ok()?);
        let day = u32::from_be_bytes(bytes[32..36].try_into().ok()?);
        let key: [u8; 32] = bytes[36..].try_into().ok()?;
        Some(Self { addressing, step: DayStep::from_parts(day, key) })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayLink {
    pub prev: KeyId,
    pub nonce: [u8; 12],
    pub sealed: Vec<u8>,
}

impl DayLink {
    fn key(server: ServerId, from: KeyId, prev: KeyId, addressing: &ChannelSecret) -> [u8; 32] {
        day_chain::derive(addressing.bytes(), &[LINK_DOMAIN, &encode(&(server, from, prev))])
    }

    fn seal(server: ServerId, from: KeyId, addressing: &ChannelSecret, prev: KeyId, given: &DayShare) -> Self {
        let aad = encode(&(LINK_DOMAIN, server, from, prev));
        let (nonce, sealed) = seal_with(&Self::key(server, from, prev, addressing), &aad, &given.to_bytes());
        Self { prev, nonce, sealed }
    }

    fn open(&self, server: ServerId, from: KeyId, addressing: &ChannelSecret) -> Option<DayShare> {
        let aad = encode(&(LINK_DOMAIN, server, from, self.prev));
        DayShare::from_bytes(&open_with(&Self::key(server, from, self.prev, addressing), self.nonce, &aad, &self.sealed)?)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayEpoch {
    pub id: KeyId,
    pub at: i64,
    pub links: Vec<DayLink>,
}

impl DayEpoch {
    fn well_formed(&self) -> bool {
        self.links.len() <= MAX_DAY_LINKS && self.links.iter().all(|l| l.sealed.len() == SEALED_SHARE_BYTES && l.prev != self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedDays {
    pub key: KeyId,
    pub ephemeral: [u8; 32],
    pub nonce: [u8; 12],
    pub sealed: Vec<u8>,
}

impl SealedDays {
    fn info(server: ServerId, user: UserId, key: KeyId, ephemeral: [u8; 32], point: [u8; 32]) -> Vec<u8> {
        encode(&(server, user, key, ephemeral, point))
    }

    pub fn seal(server: ServerId, user: UserId, recipient: &IdentityKey, key: KeyId, given: &DayShare) -> Option<Self> {
        let point = recipient.agreement_point()?;
        let ephemeral_secret: [u8; 32] = rand::random();
        let ephemeral = MontgomeryPoint::mul_base_clamped(ephemeral_secret).to_bytes();
        let shared = MontgomeryPoint(point).mul_clamped(ephemeral_secret).to_bytes();
        if shared == [0u8; 32] {
            return None;
        }
        let wrap = day_chain::derive(&shared, &[GRANT_DOMAIN, &Self::info(server, user, key, ephemeral, point)]);
        let (nonce, sealed) = seal_with(&wrap, &[], &given.to_bytes());
        Some(Self { key, ephemeral, nonce, sealed })
    }

    pub fn open(&self, server: ServerId, me: UserId, identity: &Identity) -> Option<DayShare> {
        let point = identity.public().agreement_point()?;
        let shared = identity.agree(self.ephemeral)?;
        let wrap = day_chain::derive(&shared, &[GRANT_DOMAIN, &Self::info(server, me, self.key, self.ephemeral, point)]);
        DayShare::from_bytes(&open_with(&wrap, self.nonce, &[], &self.sealed)?)
    }

    fn well_formed(&self) -> bool {
        self.sealed.len() == SEALED_SHARE_BYTES
    }

    fn released() -> Self {
        Self { key: KeyId::random(), ephemeral: [0; 32], nonce: [0; 12], sealed: vec![0; SEALED_SHARE_BYTES] }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayChain {
    pub epochs: Vec<DayEpoch>,
    pub grants: Vec<(UserId, SealedDays)>,
}

impl DayChain {
    pub fn epoch(&self, id: KeyId) -> Option<&DayEpoch> {
        self.epochs.iter().find(|e| e.id == id)
    }

    pub fn newest(&self) -> Option<&DayEpoch> {
        self.epochs.iter().max_by_key(|e| (e.at, e.id))
    }

    pub fn grant_for(&self, user: UserId) -> Option<&SealedDays> {
        self.grants.iter().find(|(u, _)| *u == user).map(|(_, g)| g)
    }

    fn next_after(&self, epoch: &DayEpoch) -> Option<i64> {
        self.epochs.iter().filter(|e| (e.at, e.id) > (epoch.at, epoch.id)).map(|e| e.at).min()
    }

    pub(crate) fn add_epoch(&mut self, epoch: DayEpoch) -> bool {
        if !epoch.well_formed() || self.epoch(epoch.id).is_some() {
            return false;
        }
        self.epochs.push(epoch);
        while self.epochs.len() > MAX_DAY_EPOCHS {
            let Some(oldest) = self.epochs.iter().enumerate().min_by_key(|(_, e)| (e.at, e.id)).map(|(i, _)| i) else { break };
            self.epochs.remove(oldest);
        }
        true
    }

    pub(crate) fn set_grant(&mut self, user: UserId, grant: SealedDays, room: usize, member: impl Fn(UserId) -> bool) -> Option<bool> {
        if !grant.well_formed() {
            return Some(false);
        }
        if self.grants.len() >= room {
            self.grants.retain(|(u, _)| member(*u));
        }
        let full = self.grants.len() >= room;
        match self.grants.iter_mut().find(|(u, _)| *u == user) {
            Some((_, held)) if *held == grant => Some(false),
            Some((_, held)) => {
                *held = grant;
                Some(true)
            }
            None if full => None,
            None => {
                self.grants.push((user, grant));
                Some(true)
            }
        }
    }
}

pub fn grant_subject(user: UserId) -> String {
    format!("day.grant:{user}")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaySealed {
    pub epoch: KeyId,
    pub day: u32,
    pub nonce: [u8; 12],
    pub body: Vec<u8>,
}

fn sealed_aad(server: ServerId, epoch: KeyId, day: u32, label: &str, place: &[u8]) -> Vec<u8> {
    encode(&(SEAL_DOMAIN, server, epoch, day, label, place))
}

#[derive(Debug, Default, Clone)]
pub struct DayKeyring {
    shares: HashMap<KeyId, DayShare>,
}

impl DayKeyring {
    pub fn unlock(state: &ServerState, me: UserId, identity: &Identity) -> Self {
        let mut ring = Self::default();
        let chain = state.day_chain();
        if let Some(grant) = chain.grant_for(me) {
            if let Some(share) = grant.open(state.server.id, me, identity) {
                ring.shares.insert(grant.key, share);
            }
        }
        ring.follow_links(state.server.id, chain);
        ring
    }

    pub fn admit(&mut self, state: &ServerState, id: KeyId, share: DayShare) {
        let known = self.shares.get(&id).is_some_and(|held| held.first_day() <= share.first_day());
        if known {
            return;
        }
        self.shares.insert(id, share);
        self.follow_links(state.server.id, state.day_chain());
    }

    fn follow_links(&mut self, server: ServerId, chain: &DayChain) {
        let mut unvisited: Vec<KeyId> = self.shares.keys().copied().collect();
        while let Some(id) = unvisited.pop() {
            let (Some(epoch), Some(share)) = (chain.epoch(id), self.shares.get(&id).cloned()) else { continue };
            for link in &epoch.links {
                let Some(prev) = link.open(server, id, share.addressing()) else { continue };
                let better = self.shares.get(&link.prev).is_none_or(|held| prev.first_day() < held.first_day());
                if better {
                    self.shares.insert(link.prev, prev);
                    unvisited.push(link.prev);
                }
            }
        }
    }

    pub fn holds(&self, id: KeyId) -> bool {
        self.shares.contains_key(&id)
    }

    pub fn share(&self, id: KeyId) -> Option<&DayShare> {
        self.shares.get(&id)
    }

    pub fn newest<'a>(&'a self, state: &'a ServerState) -> Option<(&'a DayEpoch, &'a DayShare)> {
        let epoch = state.day_chain().newest()?;
        Some((epoch, self.share(epoch.id)?))
    }

    pub fn held<'a>(&'a self, state: &'a ServerState) -> Vec<(&'a DayEpoch, &'a DayShare)> {
        let mut held: Vec<(&DayEpoch, &DayShare)> =
            state.day_chain().epochs.iter().filter_map(|e| Some((e, self.share(e.id)?))).collect();
        held.sort_by_key(|(e, _)| std::cmp::Reverse((e.at, e.id)));
        held
    }

    pub fn rotate(&mut self, state: &ServerState, links: &[KeyId], at: i64) -> DayEpoch {
        let server = state.server.id;
        let history = day_chain::history_days(&state.server);
        let id = KeyId::random();
        let share = DayShare::fresh();
        let links = links
            .iter()
            .filter_map(|prev| {
                let epoch = state.day_chain().epoch(*prev)?;
                let given = self.share(*prev)?.from_day(day_chain::window_start(epoch.at, at, history));
                Some(DayLink::seal(server, id, share.addressing(), *prev, &given))
            })
            .take(MAX_DAY_LINKS)
            .collect();
        self.shares.insert(id, share);
        DayEpoch { id, at, links }
    }

    pub fn grant_events(&self, state: &ServerState, key: KeyId, to: &[UserId], now_ms: i64) -> Vec<ServerEvent> {
        let Some(epoch) = state.day_chain().epoch(key) else { return Vec::new() };
        self.grants_for(state, epoch, to, now_ms)
    }

    fn grants_for(&self, state: &ServerState, epoch: &DayEpoch, to: &[UserId], now_ms: i64) -> Vec<ServerEvent> {
        let (key, Some(share)) = (epoch.id, self.share(epoch.id)) else { return Vec::new() };
        let server = state.server.id;
        let given = share.from_day(day_chain::window_start(epoch.at, now_ms, day_chain::history_days(&state.server)));
        to.iter()
            .filter_map(|user| {
                let recipient = state.identity_of(*user)?;
                let grant = SealedDays::seal(server, *user, &recipient, key, &given)?;
                Some(ServerEvent::DayKeyGrant { server, user: *user, grant })
            })
            .collect()
    }

    pub fn task_events(&mut self, state: &ServerState, task: DayTask, now_ms: i64) -> Vec<ServerEvent> {
        let server = state.server.id;
        match task {
            DayTask::Rotate { links } => {
                let epoch = self.rotate(state, &links, rotation_time(state, now_ms));
                let grants = self.grants_for(state, &epoch, &members(state), now_ms);
                std::iter::once(ServerEvent::DayEpochAdd { server, epoch }).chain(grants).collect()
            }
            DayTask::Grant { key, to } => self.grant_events(state, key, &to, now_ms),
            DayTask::Release { me } => vec![ServerEvent::DayKeyGrant { server, user: me, grant: SealedDays::released() }],
        }
    }

    pub fn sealer(&self, state: &ServerState) -> Option<DaySealer> {
        let (epoch, share) = self.newest(state)?;
        Some(DaySealer { server: state.server.id, epoch: epoch.id, at: epoch.at, share: share.clone() })
    }

    pub fn seal(&self, state: &ServerState, label: &str, place: &[u8], plain: &[u8], now_ms: i64) -> Option<DaySealed> {
        self.sealer(state)?.seal(label, place, plain, now_ms)
    }

    pub fn open(&self, server: ServerId, sealed: &DaySealed, label: &str, place: &[u8]) -> Option<Vec<u8>> {
        let key = self.share(sealed.epoch)?.step(sealed.day)?.entry_key(label, place);
        open_with(&key, sealed.nonce, &sealed_aad(server, sealed.epoch, sealed.day, label, place), &sealed.body)
    }
}

#[derive(Debug, Clone)]
pub struct DaySealer {
    server: ServerId,
    epoch: KeyId,
    at: i64,
    share: DayShare,
}

impl DaySealer {
    pub fn epoch(&self) -> KeyId {
        self.epoch
    }

    pub fn seal(&self, label: &str, place: &[u8], plain: &[u8], now_ms: i64) -> Option<DaySealed> {
        let day = day_chain::day_of(self.at, now_ms).max(self.share.first_day());
        let key = self.share.step(day)?.entry_key(label, place);
        let (nonce, body) = seal_with(&key, &sealed_aad(self.server, self.epoch, day, label, place), plain);
        Some(DaySealed { epoch: self.epoch, day, nonce, body })
    }
}

fn members(state: &ServerState) -> Vec<UserId> {
    state.server.members.iter().map(|m| m.user).filter(|u| state.server.is_current_member(*u)).collect()
}

fn latest_grant_time(state: &ServerState) -> i64 {
    state.day_chain().grants.iter().filter_map(|(user, _)| state.action_time(&grant_subject(*user))).max().unwrap_or(i64::MIN)
}

pub fn rotation_time(state: &ServerState, now_ms: i64) -> i64 {
    now_ms.max(latest_grant_time(state).saturating_add(1))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DayTask {
    Rotate { links: Vec<KeyId> },
    Grant { key: KeyId, to: Vec<UserId> },
    Release { me: UserId },
}

fn worth_linking(chain: &DayChain, epoch: &DayEpoch, now_ms: i64, history_days: u32) -> bool {
    let window_opens = now_ms - i64::from(history_days) * DAY_MS;
    chain.next_after(epoch).is_none_or(|ended| ended > window_opens)
}

pub fn day_key_task(state: &ServerState, me: UserId, ring: &DayKeyring, now_ms: i64) -> Option<DayTask> {
    let server = &state.server;
    if !day_chain::is_on(server) || !server.is_current_member(me) || state.is_deleted() {
        return None;
    }
    let rotates = channel_key::may_rotate_community(server, me);
    let chain = state.day_chain();
    let Some(newest) = chain.newest() else {
        return rotates.then_some(DayTask::Rotate { links: Vec::new() });
    };
    let history = day_chain::history_days(server);
    if rotates && ring.holds(newest.id) {
        let exposed = chain.grants.iter().any(|(user, grant)| {
            let written = state.action_time(&grant_subject(*user)).unwrap_or(i64::MIN);
            channel_key::community_closed_to(server, *user) && (grant.key == newest.id || written >= newest.at)
        });
        let stale = now_ms - newest.at >= FRESH_EPOCH_DAYS * DAY_MS;
        let before_approval = server.join_approval && newest.at < server.join_approval_since;
        if exposed || stale || before_approval {
            let links: Vec<KeyId> = ring
                .held(state)
                .into_iter()
                .filter(|(epoch, _)| worth_linking(chain, epoch, now_ms, history))
                .map(|(epoch, _)| epoch.id)
                .take(MAX_DAY_LINKS)
                .collect();
            return Some(DayTask::Rotate { links });
        }
    }
    if chain.grant_for(me).is_some_and(|g| chain.epoch(g.key).is_some() && !ring.holds(g.key)) {
        return Some(DayTask::Release { me });
    }
    if !ring.holds(newest.id) {
        return None;
    }
    let to: Vec<UserId> = members(state)
        .into_iter()
        .filter(|u| state.identity_of(*u).is_some() && chain.grant_for(*u).map(|g| g.key) != Some(newest.id))
        .collect();
    (!to.is_empty()).then_some(DayTask::Grant { key: newest.id, to })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{fixture, identity_for, join};

    const NOW: i64 = 1_800_000_000_000;
    const LABEL: &str = "commy-community-channel/v1";

    fn switch_on(state: &mut ServerState, history_days: u32) {
        let server = state.server.id;
        state.apply(ServerEvent::DayKeysSet { server, on: true, since: NOW, history_days }).unwrap();
    }

    fn ring(state: &ServerState, user: UserId) -> DayKeyring {
        DayKeyring::unlock(state, user, &identity_for(user))
    }

    fn settle(state: &mut ServerState, user: UserId, now_ms: i64) -> DayKeyring {
        for _ in 0..8 {
            let mut held = ring(state, user);
            let Some(task) = day_key_task(state, user, &held, now_ms) else { return held };
            for event in held.task_events(state, task, now_ms) {
                state.apply(event).unwrap();
            }
        }
        panic!("the key task never settled");
    }

    fn day(n: i64) -> i64 {
        NOW + n * DAY_MS
    }

    #[test]
    fn nothing_happens_until_the_server_is_switched_on() {
        let (mut state, _, owner) = fixture();
        assert_eq!(day_key_task(&state, owner, &DayKeyring::default(), NOW), None);
        switch_on(&mut state, 30);
        assert_eq!(day_key_task(&state, owner, &DayKeyring::default(), NOW), Some(DayTask::Rotate { links: vec![] }));
        let member = join(&mut state, "member");
        assert_eq!(day_key_task(&state, member, &DayKeyring::default(), NOW), None, "only someone who may rotate starts the chain");
    }

    #[test]
    fn every_member_gets_the_same_day_keys_and_outsiders_get_nothing() {
        let (mut state, _, owner) = fixture();
        let member = join(&mut state, "member");
        switch_on(&mut state, 30);
        let owners = settle(&mut state, owner, NOW);
        let members = ring(&state, member);
        let (epoch, share) = owners.newest(&state).unwrap();
        assert_eq!(members.newest(&state).map(|(e, s)| (e.id, s.clone())), Some((epoch.id, share.clone())));
        assert_eq!(share.first_day(), 0);
        let outsider = UserId::new();
        assert!(!ring(&state, outsider).holds(epoch.id));
        let grant = state.day_chain().grant_for(member).unwrap();
        assert_eq!(grant.open(state.server.id, owner, &identity_for(member)), None, "a grant only opens for the person it names");
        assert_eq!(grant.open(state.server.id, member, &identity_for(owner)), None);

        let sealed = owners.seal(&state, LABEL, b"general", b"see you at noon", day(2)).unwrap();
        assert_eq!(sealed.day, 2);
        assert!(!sealed.body.windows(4).any(|w| w == b"noon"));
        assert_eq!(members.open(state.server.id, &sealed, LABEL, b"general"), Some(b"see you at noon".to_vec()));
        assert_eq!(members.open(state.server.id, &sealed, LABEL, b"random"), None, "a sealed entry belongs to its place");
        assert_eq!(members.open(state.server.id, &DaySealed { day: 3, ..sealed.clone() }, LABEL, b"general"), None);
        assert_eq!(members.open(ServerId::new(), &sealed, LABEL, b"general"), None);
    }

    #[test]
    fn someone_who_joins_later_reads_back_only_as_far_as_the_setting_allows() {
        let (mut state, _, owner) = fixture();
        switch_on(&mut state, 1);
        let owners = settle(&mut state, owner, NOW);
        let early = owners.seal(&state, LABEL, b"general", b"before they came", day(1)).unwrap();
        let newcomer = join(&mut state, "newcomer");
        settle(&mut state, owner, day(4));
        let theirs = ring(&state, newcomer);
        assert_eq!(theirs.newest(&state).unwrap().1.first_day(), 4);
        assert_eq!(theirs.open(state.server.id, &early, LABEL, b"general"), None, "day one is outside a one-day window");
        let today = owners.seal(&state, LABEL, b"general", b"welcome", day(4)).unwrap();
        assert_eq!(theirs.open(state.server.id, &today, LABEL, b"general"), Some(b"welcome".to_vec()));
    }

    #[test]
    fn a_week_on_a_fresh_epoch_links_back_only_to_the_start_of_the_window() {
        let (mut state, _, owner) = fixture();
        switch_on(&mut state, 7);
        let owners = settle(&mut state, owner, NOW);
        let first = owners.newest(&state).unwrap().0.id;
        let old = owners.seal(&state, LABEL, b"general", b"long ago", day(1)).unwrap();
        let recent = owners.seal(&state, LABEL, b"general", b"last week", day(6)).unwrap();
        assert_eq!(day_key_task(&state, owner, &owners, day(6)), None, "no fresh epoch before a week has passed");
        let later = settle(&mut state, owner, day(8));
        let (second, _) = later.newest(&state).unwrap();
        assert_ne!(second.id, first);
        assert_eq!(second.links.len(), 1);

        let newcomer = join(&mut state, "newcomer");
        settle(&mut state, owner, day(8));
        let theirs = ring(&state, newcomer);
        assert_eq!(theirs.share(first).unwrap().first_day(), 2, "the old epoch opens from the start of the week before the link");
        assert_eq!(theirs.open(state.server.id, &old, LABEL, b"general"), None);
        assert_eq!(theirs.open(state.server.id, &recent, LABEL, b"general"), Some(b"last week".to_vec()));
        assert!(ring(&state, owner).holds(first), "members keep reading the old epoch through the link");
    }

    #[test]
    fn epochs_that_ended_before_the_window_are_not_linked() {
        let (mut state, _, owner) = fixture();
        switch_on(&mut state, 1);
        let first = settle(&mut state, owner, NOW).newest(&state).unwrap().0.id;
        settle(&mut state, owner, day(7));
        let second = ring(&state, owner).newest(&state).unwrap().0.clone();
        assert_eq!(second.links.iter().map(|l| l.prev).collect::<Vec<_>>(), vec![first]);
        settle(&mut state, owner, day(14));
        let third = ring(&state, owner).newest(&state).unwrap().0.clone();
        assert_eq!(third.links.iter().map(|l| l.prev).collect::<Vec<_>>(), vec![second.id], "the first epoch ended a week before the window");
    }

    #[test]
    fn someone_who_leaves_a_private_server_is_left_behind_by_a_fresh_epoch() {
        let (mut state, _, owner) = fixture();
        let leaver = join(&mut state, "leaver");
        switch_on(&mut state, 30);
        let first = settle(&mut state, owner, NOW).newest(&state).unwrap().0.id;
        assert!(ring(&state, leaver).holds(first));
        state.apply(ServerEvent::MemberLeave { server: state.server.id, user: leaver }).unwrap();
        let owners = settle(&mut state, owner, day(1));
        let (second, _) = owners.newest(&state).unwrap();
        assert_ne!(second.id, first);
        let theirs = ring(&state, leaver);
        assert!(!theirs.holds(second.id));
        let after = owners.seal(&state, LABEL, b"general", b"after they left", day(1)).unwrap();
        assert_eq!(theirs.open(state.server.id, &after, LABEL, b"general"), None);
    }

    #[test]
    fn when_two_moderators_rotate_at_once_everyone_settles_on_the_later_epoch() {
        let (mut state, _, owner) = fixture();
        let member = join(&mut state, "member");
        let other = join(&mut state, "other");
        switch_on(&mut state, 30);
        settle(&mut state, owner, NOW);
        let first = state.day_chain().newest().unwrap().id;
        let mut owners = ring(&state, owner);
        let mut copy = owners.clone();
        let from_a = owners.task_events(&state, DayTask::Rotate { links: vec![first] }, day(8));
        let from_b = copy.task_events(&state, DayTask::Rotate { links: vec![first] }, day(8) + 1);
        for event in from_a.into_iter().chain(from_b) {
            state.apply(event).unwrap();
        }
        let newest = state.day_chain().newest().unwrap().id;
        for user in [owner, member, other] {
            let theirs = ring(&state, user);
            assert_eq!(theirs.newest(&state).map(|(e, _)| e.id), Some(newest));
            assert!(theirs.holds(first));
            assert_eq!(day_key_task(&state, user, &theirs, day(8) + 2), None);
        }
    }

    #[test]
    fn a_grant_that_no_longer_opens_is_released_and_given_again() {
        let (mut state, _, owner) = fixture();
        let member = join(&mut state, "member");
        switch_on(&mut state, 30);
        settle(&mut state, owner, NOW);
        let newest = state.day_chain().newest().unwrap().id;
        let stranger = Identity::generate();
        let bad = SealedDays::seal(state.server.id, member, &stranger.public(), newest, &DayShare::fresh()).unwrap();
        state.apply(ServerEvent::DayKeyGrant { server: state.server.id, user: member, grant: bad }).unwrap();
        let theirs = ring(&state, member);
        assert_eq!(day_key_task(&state, member, &theirs, NOW), Some(DayTask::Release { me: member }));
        settle(&mut state, member, NOW);
        assert!(matches!(day_key_task(&state, owner, &ring(&state, owner), NOW), Some(DayTask::Grant { to, .. }) if to == vec![member]));
        settle(&mut state, owner, NOW);
        assert!(ring(&state, member).holds(newest));
    }

    #[test]
    fn the_day_chain_travels_in_snapshots_and_older_layouts_skip_it() {
        let (mut state, _, owner) = fixture();
        let member = join(&mut state, "member");
        switch_on(&mut state, 30);
        settle(&mut state, owner, NOW);
        let bytes = crate::wire_layout::encode(&state.snapshot()).unwrap();
        let back = ServerState::from_snapshot(crate::wire_layout::decode(&bytes).unwrap());
        assert_eq!(back.day_chain(), state.day_chain());
        assert!(ring(&back, member).newest(&back).is_some());
        let old = ServerState::from_snapshot(crate::wire_layout::read_as_first_layout(&bytes).unwrap());
        assert!(old.day_chain().epochs.is_empty());
    }

    #[test]
    fn malformed_epochs_and_grants_are_refused() {
        let (mut state, _, owner) = fixture();
        let server = state.server.id;
        let link = DayLink { prev: KeyId::random(), nonce: [0; 12], sealed: vec![0; 3] };
        let epoch = DayEpoch { id: KeyId::random(), at: NOW, links: vec![link] };
        assert_eq!(state.apply(ServerEvent::DayEpochAdd { server, epoch }), Ok(false));
        let grant = SealedDays { key: KeyId::random(), ephemeral: [0; 32], nonce: [0; 12], sealed: vec![0; 5] };
        assert_eq!(state.apply(ServerEvent::DayKeyGrant { server, user: owner, grant }), Ok(false));
        let shown = format!("{:?}", DayShare::fresh());
        assert!(shown.starts_with("DayShare(from day 0"));
    }
}
