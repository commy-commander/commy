use std::collections::{HashMap, HashSet, VecDeque};

use commy_core::channel_key::{ChannelSecret, KeyId};
use commy_core::ServerId;
use commy_net::community_streams::{Lane, Want};
use commy_net::entry_list::EntryList;
use commy_net::sync_streams::SharedKey;
use commy_net::{ServerTicket, WirePayload};
use commy_sync::{StreamId, StreamKeys};
use serde::Deserialize;

pub const MAX_PARKED: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "k", rename_all = "snake_case")]
pub enum Record {
    Servers { list: Box<EntryList> },
    CommunityKey { server: ServerId, key: SharedKey, at: i64 },
    #[serde(other)]
    Unknown,
}

impl Record {
    pub fn parse(plain: &[u8]) -> Record {
        serde_json::from_slice(plain).unwrap_or(Record::Unknown)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Nothing,
    Servers,
    Keys,
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
    after: HashMap<StreamId, u64>,
    routes: HashMap<StreamId, (ServerId, Want)>,
    live: HashSet<StreamId>,
    snapshots: HashMap<ServerId, (i64, WirePayload)>,
    parked: HashMap<ServerId, VecDeque<WirePayload>>,
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
            Record::CommunityKey { server, key, at } => {
                let known = self.keys.get(&server);
                if known.is_some_and(|(_, seen)| at <= *seen) {
                    return Change::Nothing;
                }
                let same = known.is_some_and(|(held, _)| *held == key);
                self.keys.insert(server, (key, at));
                if same { Change::Nothing } else { Change::Keys }
            }
            Record::Unknown => Change::Nothing,
        }
    }

    pub fn tickets(&self) -> HashMap<ServerId, ServerTicket> {
        self.list
            .servers
            .values()
            .filter_map(|entry| ServerTicket::decode(&entry.ticket).ok())
            .map(|ticket| (ticket.server_id, ticket))
            .collect()
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
        assert_eq!(Record::parse(json.as_bytes()), Record::CommunityKey { server, key: shared, at: 5 });
        let list = EntryList { servers: [(server, entry(server, Utc::now()))].into(), ..Default::default() };
        let json = serde_json::json!({ "k": "servers", "list": list }).to_string();
        assert_eq!(Record::parse(json.as_bytes()), Record::Servers { list: Box::new(list) });
        assert_eq!(Record::parse(br#"{"k":"read","marks":{}}"#), Record::Unknown);
        assert_eq!(Record::parse(br#"{"k":"prefs","set":{}}"#), Record::Unknown);
        assert_eq!(Record::parse(b"not json"), Record::Unknown);
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
    fn the_newest_community_key_wins() {
        let server = ServerId::new();
        let (old, new) = (KeyId::random(), KeyId::random());
        let mut book = Book::new(EntryList::default());
        let key = |id| SharedKey::of(id, &ChannelSecret::random());
        assert_eq!(book.fold(Record::CommunityKey { server, key: key(new), at: 10 }), Change::Keys);
        assert_eq!(book.fold(Record::CommunityKey { server, key: key(old), at: 5 }), Change::Nothing);
        assert_eq!(book.given()[&server].0, new);
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
