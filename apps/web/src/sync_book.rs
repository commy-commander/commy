use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use chrono::{DateTime, Utc};

use commy_core::channel_key::{ChannelSecret, KeyId};
use commy_core::{IdentityKey, ServerId, UserId, UserProfile};
use commy_net::community_streams::{Lane, Want};
use commy_net::entry_list::{Entry, EntryList};
use commy_net::sync_streams::SharedKey;
use commy_net::{ServerTicket, WirePayload};
use commy_sync::{StreamId, StreamKeys};
use commy_view::reads::{PrefPiece, ReadMark, ReadState};
use commy_view::saved::{SavedOp, SavedStore};
use serde::Deserialize;

pub const MAX_PARKED: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "k", rename_all = "snake_case")]
pub enum Record {
    Servers { list: Box<EntryList> },
    CommunityKey {
        server: ServerId,
        key: SharedKey,
        at: i64,
        #[serde(default)]
        owner: Option<(UserId, IdentityKey)>,
    },
    Read { marks: BTreeMap<String, ReadMark> },
    Prefs { set: BTreeMap<String, PrefPiece> },
    Saved { ops: Vec<SavedOp> },
    DmPartner {
        profile: Box<UserProfile>,
        at: i64,
        #[serde(default)]
        accepted: bool,
    },
    #[serde(other)]
    Unknown,
}

impl Record {
    pub fn parse(plain: &[u8]) -> Record {
        serde_json::from_slice(plain).unwrap_or(Record::Unknown)
    }
}

pub fn read_record(marks: &BTreeMap<String, ReadMark>) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "k": "read", "marks": marks })).expect("read marks encode")
}

pub fn prefs_record(set: &BTreeMap<String, PrefPiece>) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "k": "prefs", "set": set })).expect("prefs encode")
}

pub fn partner_record(profile: &UserProfile, at: i64, accepted: bool) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "k": "dm_partner", "profile": profile, "at": at, "accepted": accepted })).expect("partner encode")
}

pub fn joined_record(server: ServerId, entry: &Entry) -> Vec<u8> {
    let list = EntryList { servers: [(server, entry.clone())].into(), ..Default::default() };
    serde_json::to_vec(&serde_json::json!({ "k": "servers", "list": list })).expect("servers encode")
}

pub fn left_record(server: ServerId, at: DateTime<Utc>) -> Vec<u8> {
    let list = EntryList { left: [(server, at)].into(), ..Default::default() };
    serde_json::to_vec(&serde_json::json!({ "k": "servers", "list": list })).expect("servers encode")
}

pub fn community_key_record(server: ServerId, key: &SharedKey, at: i64, owner: Option<(UserId, IdentityKey)>) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "k": "community_key", "server": server, "key": key, "at": at, "owner": owner }))
        .expect("community key encode")
}

pub fn profile_record(profile: &UserProfile, at: DateTime<Utc>) -> Vec<u8> {
    let list = EntryList { profile: Some(profile.clone()), profile_at: Some(at), ..Default::default() };
    serde_json::to_vec(&serde_json::json!({ "k": "servers", "list": list })).expect("servers encode")
}

pub fn saved_record(ops: &[SavedOp]) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "k": "saved", "ops": ops })).expect("saved encode")
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Nothing,
    Servers,
    Keys,
    Read(Vec<String>),
    Prefs(BTreeMap<String, PrefPiece>),
    Saved,
    Partner(Box<UserProfile>, i64, bool),
}

#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    pub follow: Vec<(Lane, StreamKeys, u64)>,
    pub unfollow: Vec<StreamId>,
}

#[derive(Default)]
pub struct Book {
    list: EntryList,
    keys: HashMap<ServerId, (SharedKey, i64)>,
    owners: HashMap<ServerId, (UserId, IdentityKey)>,
    after: HashMap<StreamId, u64>,
    routes: HashMap<StreamId, (ServerId, Want)>,
    live: HashSet<StreamId>,
    snapshots: HashMap<ServerId, (i64, WirePayload)>,
    parked: HashMap<ServerId, VecDeque<WirePayload>>,
    reads: ReadState,
    prefs: BTreeMap<String, PrefPiece>,
    saved: SavedStore,
    saved_at: HashMap<String, i64>,
}

impl Book {
    pub fn new(list: EntryList) -> Self {
        Self { list, ..Default::default() }
    }

    pub fn fold(&mut self, record: Record) -> Change {
        match record {
            Record::Servers { mut list } => {
                list.identity = None;
                let next = EntryList::merge(&self.list, list.as_ref());
                if next == self.list {
                    return Change::Nothing;
                }
                self.list = next;
                Change::Servers
            }
            Record::CommunityKey { server, key, at, owner } => {
                let owned = owner.filter(|owner| self.owners.get(&server) != Some(owner)).map(|owner| self.owners.insert(server, owner)).is_some();
                let known = self.keys.get(&server);
                if known.is_some_and(|(_, seen)| at <= *seen) {
                    return if owned { Change::Keys } else { Change::Nothing };
                }
                let same = known.is_some_and(|(held, _)| *held == key);
                self.keys.insert(server, (key, at));
                if same && !owned { Change::Nothing } else { Change::Keys }
            }
            Record::Read { marks } => {
                let changed = self.note_reads(marks);
                if changed.is_empty() { Change::Nothing } else { Change::Read(changed) }
            }
            Record::Prefs { set } => {
                let taken = self.note_prefs(set);
                if taken.is_empty() { Change::Nothing } else { Change::Prefs(taken) }
            }
            Record::Saved { ops } => {
                if self.note_saved(ops).is_empty() { Change::Nothing } else { Change::Saved }
            }
            Record::DmPartner { profile, at, accepted } => Change::Partner(profile, at, accepted),
            Record::Unknown => Change::Nothing,
        }
    }

    pub fn note_saved(&mut self, ops: Vec<SavedOp>) -> Vec<SavedOp> {
        let taken: Vec<SavedOp> = ops
            .into_iter()
            .filter(|op| op.saved.as_ref().is_none_or(|s| s.entry.message == op.message))
            .filter(|op| {
                let newer = self.saved_at.get(&op.message).is_none_or(|at| op.at > *at);
                if newer {
                    self.saved_at.insert(op.message.clone(), op.at);
                }
                newer
            })
            .collect();
        if !taken.is_empty() {
            self.saved.apply_ops(taken.clone());
        }
        taken
    }

    pub fn saved(&self) -> &SavedStore {
        &self.saved
    }

    pub fn note_reads(&mut self, marks: BTreeMap<String, ReadMark>) -> Vec<String> {
        let next = ReadState::merge(&self.reads, &ReadState { channels: marks });
        let changed = next.changed_from(&self.reads);
        self.reads = next;
        changed
    }

    pub fn note_prefs(&mut self, set: BTreeMap<String, PrefPiece>) -> BTreeMap<String, PrefPiece> {
        let taken: BTreeMap<String, PrefPiece> = set.into_iter().filter(|(key, piece)| piece.beats(self.prefs.get(key))).collect();
        self.prefs.extend(taken.clone());
        taken
    }

    pub fn reads(&self) -> &ReadState {
        &self.reads
    }

    pub fn prefs(&self) -> &BTreeMap<String, PrefPiece> {
        &self.prefs
    }

    pub fn tickets(&self) -> HashMap<ServerId, ServerTicket> {
        self.list
            .servers
            .values()
            .filter_map(|entry| ServerTicket::decode(&entry.ticket).ok())
            .map(|mut ticket| {
                ticket.owner = self.owner(ticket.server_id).or(ticket.owner);
                (ticket.server_id, ticket)
            })
            .collect()
    }

    pub fn owner(&self, server: ServerId) -> Option<(UserId, IdentityKey)> {
        self.owners.get(&server).copied()
    }

    pub fn shared_key(&self, server: ServerId) -> Option<SharedKey> {
        self.keys.get(&server).map(|(key, _)| key.clone())
    }

    pub fn join(&mut self, server: ServerId, entry: Entry, key: SharedKey, at: i64) -> bool {
        if self.list.servers.contains_key(&server) {
            return false;
        }
        self.list.left.retain(|id, left| *id != server || *left > entry.since);
        if self.list.left.contains_key(&server) {
            return false;
        }
        self.list.servers.insert(server, entry);
        self.offer_key(server, key, at);
        true
    }

    pub fn note_profile(&mut self, profile: UserProfile, at: DateTime<Utc>) {
        self.list.profile = Some(profile);
        self.list.profile_at = Some(at);
    }

    pub fn profile(&self) -> Option<&UserProfile> {
        self.list.profile.as_ref()
    }

    pub fn leave(&mut self, server: ServerId, at: DateTime<Utc>) {
        self.list.servers.remove(&server);
        self.list.left.insert(server, at);
    }

    pub fn offer_key(&mut self, server: ServerId, key: SharedKey, at: i64) -> bool {
        if self.keys.contains_key(&server) {
            return false;
        }
        self.keys.insert(server, (key, at));
        true
    }

    pub fn joined_ms(&self, server: ServerId) -> Option<i64> {
        self.list.servers.get(&server).map(|entry| entry.since.timestamp_millis())
    }

    pub fn given(&self) -> HashMap<ServerId, (KeyId, ChannelSecret)> {
        self.keys.iter().filter_map(|(server, (key, _))| Some((*server, key.open()?))).collect()
    }

    pub fn replan(&mut self, wanted: HashMap<ServerId, Vec<Want>>) -> Plan {
        let mut plan = Plan::default();
        let keep: HashSet<StreamId> = wanted.values().flatten().map(|w| w.keys.id).collect();
        let gone: Vec<StreamId> = self.routes.keys().filter(|id| !keep.contains(id)).copied().collect();
        for stream in gone {
            self.routes.remove(&stream);
            self.live.remove(&stream);
            plan.unfollow.push(stream);
        }
        self.snapshots.retain(|server, _| wanted.contains_key(server));
        self.parked.retain(|server, _| wanted.contains_key(server));
        for (server, wants) in wanted {
            for want in wants {
                let stream = want.keys.id;
                if let Some((_, held)) = self.routes.get_mut(&stream) {
                    held.writes = want.writes;
                    continue;
                }
                plan.follow.push((want.lane, want.keys.clone(), self.after(stream)));
                self.routes.insert(stream, (server, want));
            }
        }
        plan
    }

    pub fn route(&self, stream: StreamId) -> Option<(ServerId, Want)> {
        self.routes.get(&stream).cloned()
    }

    pub fn after(&self, stream: StreamId) -> u64 {
        self.after.get(&stream).copied().unwrap_or(0)
    }

    pub fn advance(&mut self, stream: StreamId, seq: u64) {
        let slot = self.after.entry(stream).or_default();
        *slot = (*slot).max(seq);
    }

    pub fn caught_up(&mut self, stream: StreamId, seq: u64) {
        self.advance(stream, seq);
        if self.routes.contains_key(&stream) {
            self.live.insert(stream);
        }
    }

    pub fn is_live(&self, stream: StreamId) -> bool {
        self.live.contains(&stream)
    }

    pub fn holds_state(&self, server: ServerId) -> bool {
        self.routes.iter().any(|(stream, (sid, want))| *sid == server && want.lane == Lane::State && want.writes && self.live.contains(stream))
    }

    pub fn writers(&self, server: ServerId, lane: Lane) -> Vec<StreamId> {
        self.routes
            .iter()
            .filter(|(_, (sid, want))| *sid == server && want.lane == lane && want.writes)
            .map(|(stream, _)| *stream)
            .collect()
    }

    pub fn park(&mut self, server: ServerId, payload: WirePayload) {
        if let WirePayload::SyncResponse { at, .. } = &payload {
            if self.snapshots.get(&server).is_none_or(|(held, _)| at > held) {
                self.snapshots.insert(server, (*at, payload));
            }
            return;
        }
        let parked = self.parked.entry(server).or_default();
        parked.push_back(payload);
        while parked.len() > MAX_PARKED {
            parked.pop_front();
        }
    }

    pub fn take_snapshot(&mut self, server: ServerId) -> Option<WirePayload> {
        self.snapshots.remove(&server).map(|(_, payload)| payload)
    }

    pub fn take_parked(&mut self, server: ServerId) -> Vec<WirePayload> {
        self.parked.remove(&server).map(Vec::from).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use commy_core::{Server, ServerEvent, UserProfile};
    use commy_net::community_streams as streams;
    use commy_net::entry_list::Entry;

    fn entry(server: ServerId, since: chrono::DateTime<Utc>) -> Entry {
        Entry { name: "Hangout".into(), ticket: ServerTicket::new(server, "Hangout", [7; 32], vec![]).encode(), since }
    }

    fn want(server: ServerId, lane: Lane, writes: bool) -> Want {
        let mut want = streams::bootstrap(server, KeyId::random(), &ChannelSecret::random());
        want.lane = lane;
        want.writes = writes;
        want
    }

    #[test]
    fn reads_the_desktop_records_it_needs_and_skips_the_rest() {
        let server = ServerId::new();
        let shared = SharedKey::of(KeyId::random(), &ChannelSecret::random());
        let json = serde_json::json!({ "k": "community_key", "server": server, "key": shared, "at": 5 }).to_string();
        assert_eq!(Record::parse(json.as_bytes()), Record::CommunityKey { server, key: shared, at: 5, owner: None });
        let list = EntryList { servers: [(server, entry(server, Utc::now()))].into(), ..Default::default() };
        let json = serde_json::json!({ "k": "servers", "list": list }).to_string();
        assert_eq!(Record::parse(json.as_bytes()), Record::Servers { list: Box::new(list) });
        assert_eq!(Record::parse(br#"{"k":"read","marks":{}}"#), Record::Read { marks: BTreeMap::new() });
        assert_eq!(Record::parse(br#"{"k":"prefs","set":{}}"#), Record::Prefs { set: BTreeMap::new() });
        assert_eq!(Record::parse(br#"{"k":"saved","ops":[]}"#), Record::Saved { ops: Vec::new() });
        assert_eq!(Record::parse(br#"{"k":"dm_partner","at":1}"#), Record::Unknown);
        let friend = commy_core::UserProfile::new("friend");
        let older_build = serde_json::json!({ "k": "dm_partner", "profile": friend, "at": 3 }).to_string();
        assert_eq!(Record::parse(older_build.as_bytes()), Record::DmPartner { profile: Box::new(friend.clone()), at: 3, accepted: false });
        let ours = Record::parse(&partner_record(&friend, 4, true));
        assert_eq!(ours, Record::DmPartner { profile: Box::new(friend), at: 4, accepted: true });
        assert_eq!(Record::parse(b"not json"), Record::Unknown);
    }

    #[test]
    fn saved_messages_from_any_device_keep_the_newest_change() {
        use commy_view::saved::{Entry, SavedMessage};
        let save = |at| SavedOp { message: "m".into(), at, saved: Some(SavedMessage { saved_at: at, entry: Entry { message: "m".into(), ..Entry::default() } }) };
        let unsave = |at| SavedOp { message: "m".into(), at, saved: None };
        let mut book = Book::default();
        assert_eq!(book.note_saved(vec![save(5)]).len(), 1);
        let echo = serde_json::from_slice(&saved_record(&[save(5)])).unwrap();
        assert_eq!(book.fold(echo), Change::Nothing, "our own change coming back is not news");
        assert_eq!(book.fold(Record::Saved { ops: vec![unsave(3)] }), Change::Nothing, "an older change loses");
        assert_eq!(book.saved().saved.len(), 1);
        assert_eq!(book.fold(Record::Saved { ops: vec![unsave(9)] }), Change::Saved);
        assert!(book.saved().saved.is_empty());
    }

    #[test]
    fn server_lists_from_the_log_join_and_leave_servers() {
        let (kept, joined) = (ServerId::new(), ServerId::new());
        let base = EntryList { identity: Some("id".into()), servers: [(kept, entry(kept, Utc::now() - Duration::days(1)))].into(), ..Default::default() };
        let mut book = Book::new(base);
        let joining = EntryList { identity: Some("other".into()), servers: [(joined, entry(joined, Utc::now()))].into(), ..Default::default() };
        assert_eq!(book.fold(Record::Servers { list: Box::new(joining.clone()) }), Change::Servers);
        assert_eq!(book.fold(Record::Servers { list: Box::new(joining) }), Change::Nothing);
        assert_eq!(book.tickets().len(), 2);
        let leaving = EntryList { left: [(kept, Utc::now())].into(), ..Default::default() };
        assert_eq!(book.fold(Record::Servers { list: Box::new(leaving) }), Change::Servers);
        assert_eq!(book.tickets().keys().copied().collect::<Vec<_>>(), vec![joined]);
    }

    #[test]
    fn a_profile_edit_echoes_quietly_and_a_newer_one_from_another_device_wins() {
        let mut me = UserProfile::new("sam");
        let mut book = Book::new(EntryList { profile: Some(me.clone()), profile_at: Some(Utc::now() - Duration::days(1)), ..Default::default() });
        me.display_name = Some("Sammy".into());
        let at = Utc::now();
        book.note_profile(me.clone(), at);
        assert_eq!(book.fold(Record::parse(&profile_record(&me, at))), Change::Nothing);
        let mut older = me.clone();
        older.display_name = Some("Old".into());
        assert_eq!(book.fold(Record::parse(&profile_record(&older, at - Duration::hours(1)))), Change::Nothing);
        let mut newer = me.clone();
        newer.display_name = Some("Sam B".into());
        assert_eq!(book.fold(Record::parse(&profile_record(&newer, at + Duration::hours(1)))), Change::Servers);
        assert_eq!(book.profile().and_then(|p| p.display_name.as_deref()), Some("Sam B"));
    }

    #[test]
    fn joining_from_an_invite_lists_the_server_with_its_key_and_reads_back_as_our_own_records() {
        let mut book = Book::new(EntryList::default());
        let server = ServerId::new();
        let key = SharedKey::of(KeyId::random(), &ChannelSecret::random());
        let joined = entry(server, Utc::now());
        assert!(book.join(server, joined.clone(), key.clone(), 5));
        assert!(!book.join(server, joined.clone(), key.clone(), 6), "joining twice changes nothing");
        assert!(!book.offer_key(server, key.clone(), 7), "a key we hold is kept");
        let listed = ServerId::new();
        let mut from_log = Book::new(EntryList { servers: [(listed, entry(listed, Utc::now()))].into(), ..Default::default() });
        assert!(!from_log.join(listed, entry(listed, Utc::now()), key.clone(), 5), "another device listed it first");
        assert!(from_log.offer_key(listed, key.clone(), 5), "but the invite still hands over its key");
        assert!(from_log.given().contains_key(&listed));
        assert!(book.given().contains_key(&server));
        assert_eq!(book.joined_ms(server), Some(joined.since.timestamp_millis()));
        assert_eq!(book.fold(Record::parse(&joined_record(server, &joined))), Change::Nothing, "our own server record echoes back quietly");
        assert_eq!(book.fold(Record::parse(&community_key_record(server, &key, 5, None))), Change::Nothing);
        let mut left = Book::new(EntryList { left: [(server, Utc::now() + Duration::days(1))].into(), ..Default::default() });
        assert!(!left.join(server, joined, key, 5), "a server we left after this invite stays left");
    }

    #[test]
    fn the_newest_community_key_wins() {
        let server = ServerId::new();
        let (old, new) = (KeyId::random(), KeyId::random());
        let mut book = Book::new(EntryList::default());
        let key = |id| SharedKey::of(id, &ChannelSecret::random());
        assert_eq!(book.fold(Record::CommunityKey { server, key: key(new), at: 10, owner: None }), Change::Keys);
        assert_eq!(book.fold(Record::CommunityKey { server, key: key(old), at: 5, owner: None }), Change::Nothing);
        assert_eq!(book.given()[&server].0, new);
    }

    #[test]
    fn the_server_list_names_the_owner_another_device_last_shared() {
        let server = ServerId::new();
        let mut list = EntryList::default();
        let old_owner = (UserId::new(), commy_core::Identity::generate().public());
        let heir = (UserId::new(), commy_core::Identity::generate().public());
        let mut ticket = ServerTicket::new(server, "s", [0; 32], Vec::new());
        ticket.owner = Some(old_owner);
        list.servers.insert(server, Entry { name: "s".into(), ticket: ticket.encode(), since: Utc::now() });
        let mut book = Book::new(list);
        assert_eq!(book.tickets()[&server].owner, Some(old_owner));
        let shared = SharedKey::of(KeyId::random(), &ChannelSecret::random());
        assert_eq!(book.fold(Record::parse(&community_key_record(server, &shared, 10, Some(heir)))), Change::Keys);
        assert_eq!(book.tickets()[&server].owner, Some(heir));
        assert_eq!(book.fold(Record::parse(&community_key_record(server, &shared, 5, Some(heir)))), Change::Nothing, "nothing new");
        let other = (UserId::new(), commy_core::Identity::generate().public());
        assert_eq!(book.fold(Record::parse(&community_key_record(server, &shared, 5, Some(other)))), Change::Keys, "an older key record still names the owner");
        assert_eq!(book.owner(server), Some(other));
    }

    #[test]
    fn read_positions_and_prefs_from_any_device_merge_and_report_what_moved() {
        let mut book = Book::default();
        let mark = |id: &str, at: i64| ReadMark { id: Some(id.into()), at, set: at, back: false };
        let read = |marks: Vec<(&str, ReadMark)>| Record::parse(&read_record(&marks.into_iter().map(|(c, m)| (c.to_string(), m)).collect()));
        assert_eq!(book.fold(read(vec![("a", mark("m2", 2)), ("b", mark("m1", 1))])), Change::Read(vec!["a".into(), "b".into()]));
        assert_eq!(book.fold(read(vec![("a", mark("m1", 1))])), Change::Nothing, "an older position never moves a channel back");
        assert_eq!(book.reads().channels["a"].id.as_deref(), Some("m2"));
        let piece = |value: &str, at| PrefPiece { value: value.into(), at };
        let prefs = |set: Vec<(&str, PrefPiece)>| Record::parse(&prefs_record(&set.into_iter().map(|(k, p)| (k.to_string(), p)).collect()));
        let newer = BTreeMap::from([("notify.level".to_string(), piece("mentions", 5))]);
        assert_eq!(book.fold(prefs(vec![("notify.level", piece("mentions", 5))])), Change::Prefs(newer));
        assert_eq!(book.fold(prefs(vec![("notify.level", piece("all", 1))])), Change::Nothing);
    }

    #[test]
    fn replanning_follows_new_streams_once_and_drops_the_rest() {
        let server = ServerId::new();
        let channel = commy_core::ChannelId::new();
        let (state, text) = (want(server, Lane::State, true), want(server, Lane::Channel(channel), true));
        let mut book = Book::new(EntryList::default());
        let plan = book.replan([(server, vec![state.clone(), text.clone()])].into());
        assert_eq!(plan.follow.len(), 2);
        book.caught_up(text.keys.id, 9);
        assert!(book.is_live(text.keys.id));
        assert_eq!(book.writers(server, Lane::Channel(channel)), vec![text.keys.id]);
        let plan = book.replan([(server, vec![state.clone()])].into());
        assert!(plan.follow.is_empty());
        assert_eq!(plan.unfollow, vec![text.keys.id]);
        assert!(!book.is_live(text.keys.id));
        let plan = book.replan([(server, vec![state, text.clone()])].into());
        assert_eq!(plan.follow[0].2, 9, "a stream picks up where it left off");
    }

    #[test]
    fn parking_keeps_the_newest_snapshot_and_a_bounded_queue() {
        let server = ServerId::new();
        let owner = UserProfile::new("owner");
        let state = commy_core::ServerState::new(Server::new("s", owner.id));
        let snapshot = |at| {
            let mut payload = commy_net::protocol::build_sync_response(&state, 0, owner.id);
            if let WirePayload::SyncResponse { at: slot, .. } = &mut payload {
                *slot = at;
            }
            payload
        };
        let mut book = Book::new(EntryList::default());
        book.park(server, snapshot(20));
        book.park(server, snapshot(10));
        assert!(matches!(book.take_snapshot(server), Some(WirePayload::SyncResponse { at: 20, .. })));
        for _ in 0..MAX_PARKED + 5 {
            book.park(server, WirePayload::Event(ServerEvent::ServerRename { server, name: "x".into() }));
        }
        assert_eq!(book.take_parked(server).len(), MAX_PARKED);
        assert!(book.take_parked(server).is_empty());
    }
}
