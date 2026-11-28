use crate::channel_key::ChannelSecret;
use crate::event::ServerEvent;
use crate::ids::UserId;
use crate::permissions::Permissions;
use crate::server::Server;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fmt;

pub const DAY_MS: i64 = 24 * 60 * 60 * 1000;
pub const HISTORY_DAYS: [u32; 4] = [1, 7, 30, 90];
pub const DEFAULT_HISTORY_DAYS: u32 = 30;
pub const READS_DAY_KEYS: u32 = 1;

const SALT: &[u8] = b"commy-day-chain/v1";
const ADDRESS_INFO: &[u8] = b"address";
const CHAIN_INFO: &[u8] = b"chain";
const NEXT_DAY_INFO: &[u8] = b"next-day";
const ENTRY_INFO: &[u8] = b"entry";

pub(crate) fn derive(key: &[u8; 32], info: &[&[u8]]) -> [u8; 32] {
    let mut extract = <Hmac<Sha256> as KeyInit>::new_from_slice(SALT).expect("HMAC takes any key length");
    extract.update(key);
    let prk = extract.finalize().into_bytes();
    let mut expand = <Hmac<Sha256> as KeyInit>::new_from_slice(&prk).expect("HMAC takes any key length");
    for part in info {
        expand.update(&(part.len() as u32).to_be_bytes());
        expand.update(part);
    }
    expand.update(&[1]);
    expand.finalize().into_bytes().into()
}

#[derive(Clone, PartialEq, Eq)]
pub struct DayStep {
    day: u32,
    key: [u8; 32],
}

impl fmt::Debug for DayStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DayStep(day {}, ..)", self.day)
    }
}

pub fn split(root: &ChannelSecret) -> (ChannelSecret, DayStep) {
    let addressing = ChannelSecret::from_bytes(derive(root.bytes(), &[ADDRESS_INFO]));
    let first = DayStep { day: 0, key: derive(root.bytes(), &[CHAIN_INFO]) };
    (addressing, first)
}

impl DayStep {
    pub fn from_parts(day: u32, key: [u8; 32]) -> Self {
        Self { day, key }
    }

    pub fn day(&self) -> u32 {
        self.day
    }

    pub fn key_bytes(&self) -> &[u8; 32] {
        &self.key
    }

    pub fn advance_to(&self, day: u32) -> Option<DayStep> {
        if day < self.day {
            return None;
        }
        let mut step = self.clone();
        while step.day < day {
            step = DayStep { day: step.day + 1, key: derive(&step.key, &[NEXT_DAY_INFO]) };
        }
        Some(step)
    }

    pub fn entry_key(&self, label: &str, place: &[u8]) -> [u8; 32] {
        derive(&self.key, &[ENTRY_INFO, label.as_bytes(), place])
    }
}

pub fn day_of(epoch_at_ms: i64, now_ms: i64) -> u32 {
    let days = now_ms.saturating_sub(epoch_at_ms).max(0) / DAY_MS;
    u32::try_from(days).unwrap_or(u32::MAX)
}

pub fn clean_history_days(asked: u32) -> u32 {
    HISTORY_DAYS.into_iter().filter(|days| *days <= asked).max().unwrap_or(HISTORY_DAYS[0])
}

pub fn window_start(epoch_at_ms: i64, now_ms: i64, history_days: u32) -> u32 {
    day_of(epoch_at_ms, now_ms).saturating_sub(clean_history_days(history_days).saturating_sub(1))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayRing {
    oldest: DayStep,
}

impl DayRing {
    pub fn starting_at(oldest: DayStep) -> Self {
        Self { oldest }
    }

    pub fn oldest_day(&self) -> u32 {
        self.oldest.day
    }

    pub fn step(&self, day: u32) -> Option<DayStep> {
        self.oldest.advance_to(day)
    }

    pub fn forget_before(&mut self, day: u32) {
        if let Some(step) = self.oldest.advance_to(day) {
            self.oldest = step;
        }
    }

    pub fn handoff(&self, day: u32) -> Option<DayStep> {
        self.step(day.max(self.oldest.day))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayKeys {
    pub on: bool,
    pub since: i64,
    pub history_days: u32,
}

impl DayKeys {
    pub fn new(on: bool, since: i64, history_days: u32) -> Self {
        Self { on, since: if on { since.max(0) } else { 0 }, history_days: clean_history_days(history_days) }
    }
}

pub fn is_on(server: &Server) -> bool {
    server.day_keys.is_some_and(|keys| keys.on)
}

pub fn history_days(server: &Server) -> u32 {
    server.day_keys.map_or(DEFAULT_HISTORY_DAYS, |keys| keys.history_days)
}

pub fn reads_day_keys(server: &Server, user: UserId) -> bool {
    server.day_keys_ready.get(&user).is_some_and(|level| *level >= READS_DAY_KEYS)
}

pub fn waiting_on(server: &Server) -> Vec<UserId> {
    server.members.iter().map(|m| m.user).filter(|user| server.is_current_member(*user) && !reads_day_keys(server, *user)).collect()
}

pub fn ready_event(server: &Server, me: UserId) -> Option<ServerEvent> {
    (server.is_current_member(me) && !reads_day_keys(server, me)).then_some(ServerEvent::DayKeysReady { server: server.id, user: me, level: READS_DAY_KEYS })
}

pub fn switch_event(server: &Server, on: bool, history_days: u32, now_ms: i64) -> ServerEvent {
    let since = server.day_keys.filter(|keys| keys.on).map_or(now_ms, |keys| keys.since);
    ServerEvent::DayKeysSet { server: server.id, on, since, history_days: clean_history_days(history_days) }
}

pub fn auto_switch_event(server: &Server, me: UserId, now_ms: i64) -> Option<ServerEvent> {
    let may = server.permissions_for(me).allows(Permissions::MANAGE_SERVER);
    (server.day_keys.is_none() && may && waiting_on(server).is_empty()).then(|| switch_event(server, true, DEFAULT_HISTORY_DAYS, now_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> ChannelSecret {
        ChannelSecret::from_bytes([7; 32])
    }

    #[test]
    fn the_root_splits_into_addressing_and_a_first_day_that_differ() {
        let (addressing, first) = split(&root());
        assert_eq!(first.day(), 0);
        assert_ne!(addressing.bytes(), root().bytes());
        assert_ne!(first.key_bytes(), addressing.bytes());
        assert_ne!(first.key_bytes(), root().bytes());
        assert_eq!(split(&root()), (addressing, first), "the same root always splits the same way");
    }

    #[test]
    fn days_move_forward_only() {
        let (_, first) = split(&root());
        let tenth = first.advance_to(10).unwrap();
        assert_eq!(tenth.day(), 10);
        assert_eq!(first.advance_to(4).unwrap().advance_to(10).unwrap(), tenth, "stepping in parts lands on the same day");
        assert_eq!(tenth.advance_to(9), None, "an earlier day can't be reached from a later one");
        assert_eq!(tenth.advance_to(10).unwrap(), tenth);
        assert_ne!(first.advance_to(11).unwrap().key_bytes(), tenth.key_bytes());
    }

    #[test]
    fn entry_keys_differ_by_day_lane_and_place() {
        let (_, first) = split(&root());
        let today = first.advance_to(3).unwrap();
        let key = today.entry_key("commy-community-channel/v1", b"general");
        assert_ne!(key, today.entry_key("commy-community-state/v1", b"general"));
        assert_ne!(key, today.entry_key("commy-community-channel/v1", b"random"));
        assert_ne!(key, first.advance_to(4).unwrap().entry_key("commy-community-channel/v1", b"general"));
        assert_ne!(today.entry_key("ab", b"c"), today.entry_key("a", b"bc"), "parts can't run into each other");
        assert_ne!(&key, today.key_bytes());
    }

    #[test]
    fn a_ring_that_forgot_earlier_days_cannot_open_them() {
        let (_, first) = split(&root());
        let mut ring = DayRing::starting_at(first.clone());
        let fifth = ring.step(5).unwrap();
        ring.forget_before(5);
        assert_eq!(ring.oldest_day(), 5);
        assert_eq!(ring.step(4), None);
        assert_eq!(ring.step(5), Some(fifth));
        assert_eq!(ring.step(9), first.advance_to(9));
        ring.forget_before(2);
        assert_eq!(ring.oldest_day(), 5, "forgetting never moves back");
        assert_eq!(ring.handoff(1).unwrap().day(), 5, "a handoff can't give out days the ring no longer holds");
    }

    #[test]
    fn the_history_window_counts_today_and_snaps_to_a_known_length() {
        let epoch = 1_000;
        let now = epoch + 40 * DAY_MS + 5;
        assert_eq!(day_of(epoch, now), 40);
        assert_eq!(day_of(epoch, epoch - 1), 0, "a clock behind the epoch counts as its first day");
        assert_eq!(window_start(epoch, now, 30), 11);
        assert_eq!(window_start(epoch, now, 1), 40, "one day means today only");
        assert_eq!(window_start(epoch, now, 90), 0);
        assert_eq!(clean_history_days(0), 1);
        assert_eq!(clean_history_days(20), 7);
        assert_eq!(clean_history_days(1_000), 90);
        assert_eq!(DEFAULT_HISTORY_DAYS, clean_history_days(DEFAULT_HISTORY_DAYS));
    }

    mod switch {
        use super::super::*;
        use crate::ids::RoleId;
        use crate::server::Role;
        use crate::store::{ServerState, StoreError};
        use crate::test_support::{fixture, join, signed_event};
        use chrono::{Duration, Utc};

        fn by(event: ServerEvent, state: &mut ServerState, user: UserId, ago_ms: i64) -> Result<bool, StoreError> {
            let at = Utc::now() - Duration::milliseconds(ago_ms);
            let signed = signed_event(state, user, at, event);
            state.apply_signed(&signed)
        }

        fn manager(state: &mut ServerState, name: &str) -> UserId {
            let server = state.server.id;
            let user = join(state, name);
            let role = Role {
                id: RoleId::new(),
                name: "managers".into(),
                color: None,
                permissions: Permissions::MANAGE_SERVER,
                position: 1,
                hoist: false,
                self_assignable: false,
                mentionable: false,
                tag: None,
                banner: None,
            };
            let id = role.id;
            state.apply(ServerEvent::RoleCreate { server, role }).unwrap();
            state.apply(ServerEvent::RoleAssign { server, user, role: id }).unwrap();
            user
        }

        #[test]
        fn the_switch_turns_on_by_itself_once_every_member_can_read_day_keys() {
            let (mut state, _, owner) = fixture();
            let friend = join(&mut state, "friend");
            assert!(!is_on(&state.server));
            assert_eq!(waiting_on(&state.server), vec![owner, friend]);
            assert_eq!(auto_switch_event(&state.server, owner, 5), None, "not while someone's app can't read them");

            let ready = ready_event(&state.server, owner).unwrap();
            assert_eq!(by(ready.clone(), &mut state, friend, 0), Err(StoreError::NotAllowed), "nobody says it for someone else");
            assert_eq!(by(ready, &mut state, owner, 0), Ok(true));
            assert_eq!(ready_event(&state.server, owner), None, "once said, it isn't said again");
            assert_eq!(waiting_on(&state.server), vec![friend]);
            by(ready_event(&state.server, friend).unwrap(), &mut state, friend, 0).unwrap();
            assert!(waiting_on(&state.server).is_empty());

            assert_eq!(auto_switch_event(&state.server, friend, 5), None, "only someone who manages the server flips it");
            let on = auto_switch_event(&state.server, owner, 5).unwrap();
            assert_eq!(on, ServerEvent::DayKeysSet { server: state.server.id, on: true, since: 5, history_days: DEFAULT_HISTORY_DAYS });
            assert_eq!(by(on, &mut state, owner, 0), Ok(true));
            assert!(is_on(&state.server));
            assert_eq!(history_days(&state.server), 30);
            assert_eq!(auto_switch_event(&state.server, owner, 9), None);
            assert_eq!(state.audit_log().last().map(|e| (e.kind, e.detail.clone())), Some((crate::AuditKind::DayKeys, Some("30".into()))));
        }

        #[test]
        fn an_owner_who_chose_off_is_not_overruled_and_only_the_owner_turns_it_off() {
            let (mut state, _, owner) = fixture();
            let admin = manager(&mut state, "admin");
            assert_eq!(by(switch_event(&state.server, true, 7, 100), &mut state, admin, 30), Ok(true), "managers may turn it on sooner");
            assert_eq!(state.server.day_keys, Some(DayKeys { on: true, since: 100, history_days: 7 }));
            assert_eq!(by(switch_event(&state.server, false, 7, 200), &mut state, admin, 20), Err(StoreError::NotAllowed));
            assert_eq!(by(switch_event(&state.server, false, 7, 300), &mut state, owner, 10), Ok(true));
            assert_eq!(state.server.day_keys, Some(DayKeys { on: false, since: 0, history_days: 7 }));
            for user in [owner, admin] {
                by(ready_event(&state.server, user).unwrap(), &mut state, user, 0).unwrap();
            }
            assert_eq!(auto_switch_event(&state.server, owner, 400), None, "a server switched off stays off");
        }

        #[test]
        fn changing_the_history_keeps_the_day_the_ratchet_began() {
            let (mut state, _, owner) = fixture();
            by(switch_event(&state.server, true, 30, 100), &mut state, owner, 30).unwrap();
            let shorter = switch_event(&state.server, true, 20, 900);
            assert_eq!(shorter, ServerEvent::DayKeysSet { server: state.server.id, on: true, since: 100, history_days: 7 });
            by(shorter, &mut state, owner, 20).unwrap();
            assert_eq!(state.server.day_keys, Some(DayKeys { on: true, since: 100, history_days: 7 }));
        }

        #[test]
        fn the_newest_switch_wins_whatever_order_it_arrives_in() {
            let (mut first, _, owner) = fixture();
            let mut second = first.clone();
            let older = signed_event(&first, owner, Utc::now() - Duration::seconds(20), ServerEvent::DayKeysSet { server: first.server.id, on: true, since: 1, history_days: 30 });
            let newer = signed_event(&first, owner, Utc::now() - Duration::seconds(10), ServerEvent::DayKeysSet { server: first.server.id, on: true, since: 2, history_days: 90 });
            first.apply_signed(&older).unwrap();
            first.apply_signed(&newer).unwrap();
            second.apply_signed(&newer).unwrap();
            let _ = second.apply_signed(&older);
            assert_eq!(first.server.day_keys, second.server.day_keys);
            assert_eq!(first.server.day_keys, Some(DayKeys { on: true, since: 2, history_days: 90 }));
        }

        #[test]
        fn only_current_members_are_counted() {
            let (mut state, _, owner) = fixture();
            let friend = join(&mut state, "friend");
            let leaver = join(&mut state, "leaver");
            by(ServerEvent::DayKeysReady { server: state.server.id, user: leaver, level: READS_DAY_KEYS }, &mut state, leaver, 30).unwrap();
            state.apply(ServerEvent::MemberLeave { server: state.server.id, user: leaver }).unwrap();
            assert_eq!(ready_event(&state.server, leaver), None, "someone who left has nothing to say");
            by(ready_event(&state.server, owner).unwrap(), &mut state, owner, 20).unwrap();
            assert!(!state.server.day_keys_ready.contains_key(&leaver), "people who left drop off the list");
            assert_eq!(waiting_on(&state.server), vec![friend]);
            let later = ServerEvent::DayKeysReady { server: state.server.id, user: friend, level: READS_DAY_KEYS + 1 };
            by(later, &mut state, friend, 10).unwrap();
            assert!(reads_day_keys(&state.server, friend), "a newer app counts too");
            assert_eq!(ready_event(&state.server, friend), None, "an older device doesn't take a newer one's word back");
        }

        #[test]
        fn the_setting_travels_in_snapshots_and_older_layouts_skip_it() {
            let (mut state, _, owner) = fixture();
            by(ready_event(&state.server, owner).unwrap(), &mut state, owner, 20).unwrap();
            by(switch_event(&state.server, true, 90, 100), &mut state, owner, 10).unwrap();
            let bytes = crate::wire_layout::encode(&state.server).unwrap();
            let back: Server = crate::wire_layout::decode(&bytes).unwrap();
            assert_eq!(back.day_keys, state.server.day_keys);
            assert_eq!(back.day_keys_ready, state.server.day_keys_ready);
            let old: Server = crate::wire_layout::read_as_first_layout(&bytes).unwrap();
            assert_eq!(old.name, state.server.name);
        }

        #[test]
        fn snapshots_keep_day_key_changes_where_older_apps_skip_them() {
            use crate::store::{older_apps_read_audit, older_apps_read_event, Snapshot, FIRST_NEWER_AUDIT_TAG, FIRST_NEWER_EVENT_TAG};
            let (mut state, _, owner) = fixture();
            let server = state.server.id;
            by(ServerEvent::ServerRename { server, name: "renamed".into() }, &mut state, owner, 30).unwrap();
            by(ready_event(&state.server, owner).unwrap(), &mut state, owner, 20).unwrap();
            by(switch_event(&state.server, true, 7, 100), &mut state, owner, 10).unwrap();
            let snapshot = state.snapshot();
            assert!(snapshot.signed_actions.iter().all(|s| older_apps_read_event(&s.event)));
            assert!(snapshot.signed_actions.iter().any(|s| matches!(s.event, ServerEvent::ServerRename { .. })));
            assert_eq!(snapshot.newer_actions.len(), 2);
            assert!(snapshot.audit.iter().all(|e| older_apps_read_audit(&e.kind)) && !snapshot.audit.is_empty());
            assert_eq!(snapshot.newer_audit.len(), 1);

            let bytes = crate::wire_layout::encode(&snapshot).unwrap();
            let old: Snapshot = crate::wire_layout::read_as_first_layout(&bytes).unwrap();
            assert!(old.newer_actions.is_empty() && old.newer_audit.is_empty());
            assert_eq!(old.signed_actions.len(), snapshot.signed_actions.len());

            let back = ServerState::from_snapshot(crate::wire_layout::decode(&bytes).unwrap());
            assert_eq!(back.signed_actions().len(), state.signed_actions().len());
            assert!(back.signed_actions().iter().any(|s| matches!(s.event, ServerEvent::DayKeysSet { .. })));
            assert!(back.audit_log().iter().any(|e| e.kind == crate::AuditKind::DayKeys));
            let json = serde_json::to_string(&snapshot).unwrap();
            let from_disk = ServerState::from_snapshot(serde_json::from_str(&json).unwrap());
            assert_eq!(from_disk.signed_actions().len(), state.signed_actions().len());
            assert_eq!(from_disk.audit_log().len(), state.audit_log().len());

            let tag = |event: ServerEvent| postcard::take_from_bytes::<u32>(&postcard::to_stdvec(&event).unwrap()).unwrap().0;
            assert_eq!(tag(ServerEvent::HomesBlocked { server, homes: vec![], require_home: false }) + 2, FIRST_NEWER_EVENT_TAG);
            assert_eq!(tag(ServerEvent::DayKeysReady { server, user: owner, level: 1 }), FIRST_NEWER_EVENT_TAG);
            let kind = |kind: crate::AuditKind| postcard::take_from_bytes::<u32>(&postcard::to_stdvec(&kind).unwrap()).unwrap().0;
            assert_eq!(kind(crate::AuditKind::BlockedHomes) + 1, FIRST_NEWER_AUDIT_TAG);
            assert_eq!(kind(crate::AuditKind::DayKeys), FIRST_NEWER_AUDIT_TAG);
        }
    }

    #[test]
    fn a_day_step_never_prints_its_key() {
        let (_, first) = split(&root());
        let shown = format!("{first:?}");
        assert!(shown.contains("day 0") && !shown.contains(&format!("{:?}", first.key_bytes())));
    }
}
