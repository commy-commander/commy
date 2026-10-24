use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use commy_core::channel_key::{ChannelSecret, KeyId, KeyScope};
use commy_core::{Message, ServerEvent, ServerId, UserId};
use commy_net::community_streams::{self as streams, Lane};
use commy_net::entry_list::{self, EntryList};
use commy_net::sync_streams::SharedKey;
use commy_net::sealed::SealKey;
use commy_net::{sync_streams, ServerTicket, WirePayload};
use commy_sync::{Pool, StreamId, SyncClient, SyncEvent};
use commy_view::liveness::{SERVICE_BEAT, SWEEP_EVERY};
use commy_view::reads::{PrefPiece, ReadMark, ReadView};
use commy_view::saved::{Entry, SavedOp, SavedStore};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use wasm_bindgen_futures::spawn_local;

use super::Session;
use crate::client::Applied;
use crate::sync_book::{community_key_record, joined_record, prefs_record, read_record, saved_record, Book, Change, Record};

const REFRESH_EVERY: Duration = Duration::from_secs(30);
const SNAPSHOT_EVERY_MS: i64 = 6 * 60 * 60 * 1000;
const MAX_MARKS_PER_RECORD: usize = 300;

fn chunks(marks: BTreeMap<String, ReadMark>, size: usize) -> Vec<BTreeMap<String, ReadMark>> {
    let mut out = Vec::new();
    let mut current = BTreeMap::new();
    for (channel, mark) in marks {
        current.insert(channel, mark);
        if current.len() >= size {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

pub(super) struct Live {
    pub(super) pool: Pool,
    account: StreamId,
    account_live: bool,
    pub(super) book: Book,
    pub(super) node: [u8; 32],
    greeted: HashSet<ServerId>,
    gone: bool,
    published: HashMap<ServerId, i64>,
    key_work: HashSet<(ServerId, KeyScope)>,
    pub(super) pictures: Option<String>,
}

impl Session {
    pub(super) async fn start_sync(self: Rc<Self>, key: SealKey, list: EntryList) {
        if self.sync.borrow().is_some() {
            return;
        }
        let Some(url) = self.sync_service_url().await else { return };
        let Ok(account) = self.account() else { return };
        let node = *iroh::SecretKey::from_bytes(&self.device_secret(&account)).public().as_bytes();
        let (mut pool, events) = Pool::new(&url);
        let stream = sync_streams::account_stream(&key);
        let account = stream.id;
        pool.follow(stream, 0);
        let pictures = commy_view::pictures::pictures_url(&url);
        *self.sync.borrow_mut() = Some(Live { pool, account, account_live: false, book: Book::new(list), node, greeted: HashSet::new(), gone: false, published: HashMap::new(), key_work: HashSet::new(), pictures });
        spawn_local(self.clone().run_sync(events));
        spawn_local(self.clone().tend_sync());
        spawn_local(self.clone().tend_presence());
        self.refresh_sync();
    }

    pub(super) fn stop_sync(&self) {
        self.sync.borrow_mut().take();
        self.dms.borrow_mut().forget_streams();
    }

    pub(super) fn nudge_sync(&self) {
        if let Some(live) = self.sync.borrow().as_ref() {
            live.pool.nudge();
        }
    }

    async fn run_sync(self: Rc<Self>, mut events: mpsc::UnboundedReceiver<SyncEvent>) {
        while let Some(event) = events.recv().await {
            match event {
                SyncEvent::Entry { stream, seq, at, plain } => self.on_sync_entry(stream, seq, at, &plain),
                SyncEvent::Unreadable { stream, seq } => {
                    self.dms.borrow_mut().advance(stream, seq);
                    if let Some(live) = self.sync.borrow_mut().as_mut() {
                        live.book.advance(stream, seq);
                    }
                }
                SyncEvent::CaughtUp { stream, seq } => self.on_sync_caught_up(stream, seq),
                SyncEvent::Signal { stream, plain } => self.on_sync_signal(stream, &plain),
                SyncEvent::Refused { .. } | SyncEvent::Connected | SyncEvent::Disconnected => {}
            }
        }
    }

    async fn tend_sync(self: Rc<Self>) {
        loop {
            n0_future::time::sleep(REFRESH_EVERY).await;
            if self.sync.borrow().is_none() {
                return;
            }
            self.refresh_sync();
            self.tend_owned();
        }
    }

    fn tend_owned(&self) {
        let now = Utc::now().timestamp_millis();
        for server in self.client.borrow().owned_servers() {
            let (due, copy) = {
                let mut guard = self.sync.borrow_mut();
                let Some(live) = guard.as_mut() else { return };
                if !live.book.holds_state(server) {
                    continue;
                }
                let work: HashSet<(ServerId, KeyScope)> = self.client.borrow().key_work(server).into_iter().map(|scope| (server, scope)).collect();
                let due: Vec<KeyScope> = work.iter().filter(|w| live.key_work.contains(w)).map(|(_, scope)| *scope).collect();
                live.key_work.retain(|(sid, _)| *sid != server);
                live.key_work.extend(work);
                let copy = live.published.get(&server).is_none_or(|at| now - at >= SNAPSHOT_EVERY_MS);
                if copy {
                    live.published.insert(server, now);
                }
                (due, copy)
            };
            for scope in due {
                for payload in self.client.borrow().tend_scope(server, scope) {
                    self.publish(server, &payload);
                }
            }
            if copy {
                for payload in self.client.borrow().owner_copy(server) {
                    self.publish(server, &payload);
                }
            }
        }
    }

    pub(super) async fn start_group(self: &Rc<Self>) -> Result<ServerId, String> {
        self.start_server("", commy_core::Server::group)
    }

    pub(super) fn start_server(self: &Rc<Self>, name: &str, make: impl FnOnce(UserId) -> commy_core::Server) -> Result<ServerId, String> {
        let (me, identity) = self.client.borrow().me.lock().unwrap().clone().ok_or("Your servers are not unlocked yet")?;
        if self.sync.borrow().is_none() {
            return Err("Your servers are not unlocked yet".into());
        }
        let server = ServerId::new();
        let mut ticket = ServerTicket::new(server, name, commy_net::node::topic_for(server), Vec::new());
        ticket.owner = Some((me.id, identity.public()));
        let keys = self.client.borrow().start(server, make)?;
        let (id, secret) = self.client.borrow().newest_community_key(server).ok_or("Could not make a key for the server")?;
        let now = Utc::now();
        let entry = entry_list::Entry { name: name.to_string(), ticket: ticket.encode(), since: now };
        let shared = SharedKey::of(id, &secret);
        if let Some(live) = self.sync.borrow_mut().as_mut() {
            live.book.join(server, entry.clone(), shared.clone(), now.timestamp_millis());
            live.published.insert(server, now.timestamp_millis());
            live.greeted.insert(server);
        }
        self.client.borrow_mut().tickets.insert(server, ticket.clone());
        self.append_account(joined_record(server, &entry));
        self.append_account(community_key_record(server, &shared, now.timestamp_millis(), ticket.owner));
        self.refresh_sync();
        let copy = self.client.borrow().owner_copy(server);
        for payload in keys.iter().chain(copy.iter()) {
            self.publish(server, payload);
        }
        self.emit("servers-changed", Value::Null);
        if self.client.borrow().is_group(server) {
            self.emit("dm-changed", json!(server.to_string()));
        }
        Ok(server)
    }

    async fn tend_presence(self: Rc<Self>) {
        let mut beat_due = web_time::Instant::now() + SERVICE_BEAT;
        loop {
            n0_future::time::sleep(SWEEP_EVERY).await;
            if self.sync.borrow().is_none() {
                return;
            }
            if web_time::Instant::now() >= beat_due {
                beat_due = web_time::Instant::now() + SERVICE_BEAT;
                self.say_here_everywhere();
            }
            for server in self.client.borrow().sweep_presence(Utc::now().timestamp_millis()) {
                self.emit("server-event", json!(server.to_string()));
            }
        }
    }

    pub(super) fn say_here(&self, server: ServerId) {
        let Some(node) = self.sync.borrow().as_ref().map(|live| live.node) else { return };
        let voice = self.voice_here(server);
        for payload in self.client.borrow().here(server, &node, voice) {
            self.publish(server, &payload);
        }
    }

    pub(super) fn say_here_everywhere(&self) {
        let servers: Vec<ServerId> = match self.sync.borrow_mut().as_mut() {
            Some(live) => {
                live.gone = false;
                live.greeted.iter().copied().collect()
            }
            None => return,
        };
        for server in servers {
            self.say_here(server);
        }
    }

    pub(super) fn say_goodbye(&self) {
        match self.sync.borrow_mut().as_mut() {
            Some(live) => live.gone = true,
            None => return,
        }
        for (server, payload) in self.client.borrow().farewell() {
            self.publish(server, &payload);
        }
    }

    pub(super) fn said_goodbye(&self) -> bool {
        self.sync.borrow().as_ref().is_some_and(|live| live.gone)
    }

    pub(super) fn refresh_sync(&self) {
        let Some(given) = self.sync.borrow().as_ref().map(|live| live.book.given()) else { return };
        let wanted = self.client.borrow().sync_wants(&given, Utc::now().timestamp_millis());
        let mut guard = self.sync.borrow_mut();
        let Some(live) = guard.as_mut() else { return };
        let plan = live.book.replan(wanted);
        for stream in plan.unfollow {
            live.pool.unfollow(stream);
        }
        let (lasting, texts): (Vec<_>, Vec<_>) = plan.follow.into_iter().partition(|(lane, _, _)| *lane == Lane::State);
        for (part, tail) in [(lasting, streams::STATE_TAIL), (texts, streams::FRESH_TAIL)] {
            let part: Vec<_> = part.into_iter().map(|(_, keys, after)| (keys, after)).collect();
            if !part.is_empty() {
                live.pool.follow_together_tail(part, Some(tail));
            }
        }
        drop(guard);
        self.refresh_dms();
    }

    fn on_sync_entry(self: &Rc<Self>, stream: StreamId, seq: u64, at: i64, plain: &[u8]) {
        let dm = self.dms.borrow().source(stream);
        if let Some(source) = dm {
            self.on_dm_entry(stream, seq, at, source, plain);
            return;
        }
        let routed = {
            let mut guard = self.sync.borrow_mut();
            let Some(live) = guard.as_mut() else { return };
            if stream == live.account {
                let change = live.book.fold(Record::parse(plain));
                let settled = live.account_live;
                drop(guard);
                if settled || matches!(change, Change::Partner(..)) {
                    self.on_account_change(change);
                }
                return;
            }
            let Some((server, want)) = live.book.route(stream) else { return };
            live.book.advance(stream, seq);
            (server, want.lane, live.book.is_live(stream))
        };
        let (server, lane, now_live) = routed;
        let Some(payload) = streams::decode_entry(plain, lane) else { return };
        if !self.client.borrow().has(server) {
            let snapshot = matches!(payload, WirePayload::SyncResponse { .. });
            if snapshot && self.client.borrow().vouched(server, &payload, streams::RETENTION_MS).is_err() {
                return;
            }
            if let Some(live) = self.sync.borrow_mut().as_mut() {
                live.book.park(server, payload);
            }
            if now_live {
                self.adopt(server);
            }
            return;
        }
        self.release_parked(server);
        if matches!(payload, WirePayload::SyncResponse { .. }) {
            if self.client.borrow().learn_profiles(server, &payload) && now_live {
                self.emit("server-event", json!(server.to_string()));
            }
            return;
        }
        let Some((event, action)) = streams::event_of(&payload).map(|(event, action)| (event.clone(), action)) else { return };
        let key_event = streams::is_key_event(&event);
        let rings = matches!(&event, ServerEvent::MessageCreate { message } if streams::rings(now_live, message.created_at, Utc::now()));
        let applied = self.apply_stored(server, event, action, rings, now_live);
        if key_event && now_live && applied != Applied::Nothing {
            self.refresh_sync();
        }
    }

    fn on_sync_signal(self: &Rc<Self>, stream: StreamId, plain: &[u8]) {
        let found = {
            let guard = self.sync.borrow();
            let Some(live) = guard.as_ref() else { return };
            let Some((server, want)) = live.book.route(stream) else { return };
            let Some((node, payload)) = streams::decode_signal(plain, want.lane) else { return };
            (node != live.node).then_some((server, node, payload))
        };
        let Some((server, node, payload)) = found else { return };
        let Some((event, action)) = streams::event_of(&payload).map(|(event, action)| (event.clone(), action)) else { return };
        if let ServerEvent::TypingStart { channel, user } = &event {
            if self.client.borrow().admits(server, &event, action) {
                self.emit("typing", json!({ "server": server.to_string(), "channel": channel.to_string(), "user": user.to_string() }));
            }
            return;
        }
        let returning = match &event {
            ServerEvent::PresenceUpdate { user, presence } => {
                presence.is_visible_online() && !self.client.borrow().is_shown_online(server, *user)
            }
            _ => false,
        };
        if !self.client.borrow().apply_signal(server, &node, event, action) {
            return;
        }
        self.emit("server-event", json!(server.to_string()));
        if returning && !self.said_goodbye() {
            self.say_here(server);
        }
    }

    fn on_sync_caught_up(self: &Rc<Self>, stream: StreamId, seq: u64) {
        if self.dms.borrow_mut().caught_up(stream, seq) {
            return;
        }
        let (server, state_lane) = {
            let mut guard = self.sync.borrow_mut();
            let Some(live) = guard.as_mut() else { return };
            if stream == live.account {
                live.account_live = true;
                drop(guard);
                self.on_account_change(Change::Servers);
                return;
            }
            live.book.caught_up(stream, seq);
            let Some((server, want)) = live.book.route(stream) else { return };
            (server, want.lane == Lane::State)
        };
        if self.client.borrow().has(server) {
            self.release_parked(server);
            let greet = state_lane
                && self.sync.borrow_mut().as_mut().is_some_and(|live| !live.gone && live.greeted.insert(server));
            if greet {
                self.say_here(server);
            }
            self.emit("server-event", json!(server.to_string()));
            self.refresh_sync();
        } else {
            self.adopt(server);
        }
    }

    fn on_account_change(self: &Rc<Self>, change: Change) {
        match change {
            Change::Nothing => {}
            Change::Keys => self.refresh_sync(),
            Change::Servers => self.follow_server_list(),
            Change::Read(changed) => {
                let view = self.sync.borrow().as_ref().map(|live| self.client.borrow().read_view(live.book.reads(), Some(&changed)));
                if let Some(view) = view {
                    self.emit("read-state-changed", json!(view));
                }
            }
            Change::Prefs(set) => self.emit("sync-prefs-changed", json!(set)),
            Change::Saved => self.emit("saved-changed", json!(self.saved())),
            Change::Partner(profile, at, accepted) => self.adopt_partner(*profile, at, accepted),
        }
    }

    pub(super) fn read_view(&self, only: Option<&[String]>) -> ReadView {
        let guard = self.sync.borrow();
        let reads = guard.as_ref().map(|live| live.book.reads().clone()).unwrap_or_default();
        self.client.borrow().read_view(&reads, only)
    }

    pub(super) fn note_reads(&self, marks: BTreeMap<String, ReadMark>) {
        let changed = {
            let mut guard = self.sync.borrow_mut();
            let Some(live) = guard.as_mut() else { return };
            let changed = live.book.note_reads(marks);
            changed.iter().filter_map(|c| live.book.reads().channels.get(c).map(|m| (c.clone(), m.clone()))).collect::<BTreeMap<_, _>>()
        };
        for chunk in chunks(changed, MAX_MARKS_PER_RECORD) {
            self.append_account(read_record(&chunk));
        }
    }

    pub(super) fn prefs(&self) -> BTreeMap<String, PrefPiece> {
        self.sync.borrow().as_ref().map(|live| live.book.prefs().clone()).unwrap_or_default()
    }

    pub(super) fn note_prefs(&self, set: BTreeMap<String, PrefPiece>) {
        let taken = match self.sync.borrow_mut().as_mut() {
            Some(live) => live.book.note_prefs(set),
            None => return,
        };
        if !taken.is_empty() {
            self.append_account(prefs_record(&taken));
        }
    }

    pub(super) fn saved(&self) -> SavedStore {
        self.sync.borrow().as_ref().map(|live| live.book.saved().clone()).unwrap_or_default()
    }

    pub(super) fn set_saved(&self, entry: Entry, saved: bool) -> Result<SavedStore, String> {
        let at = Utc::now().timestamp_millis();
        let message = entry.message.clone();
        let mut draft = SavedStore::default();
        draft.set_saved(entry, saved, at);
        let kept = draft.saved.into_iter().next().filter(|_| saved);
        let op = SavedOp { message, at, saved: kept };
        let taken = match self.sync.borrow_mut().as_mut() {
            Some(live) => live.book.note_saved(vec![op]),
            None => return Err("Your servers are not unlocked yet".into()),
        };
        if !taken.is_empty() {
            self.append_account(saved_record(&taken));
        }
        Ok(self.saved())
    }

    pub(super) fn append_account(&self, plain: Vec<u8>) {
        let Some((stream, client)) = self.sync.borrow().as_ref().and_then(|live| Some((live.account, live.pool.client_for(live.account)?))) else {
            return;
        };
        spawn_local(async move {
            let _ = client.append(stream, plain).await;
        });
    }

    pub(super) fn join_group(self: &Rc<Self>, raw: &str, key: Option<(KeyId, ChannelSecret)>) {
        let Ok(ticket) = ServerTicket::decode(raw) else { return };
        let Some((id, secret)) = key else { return };
        let server = ticket.server_id;
        if ticket.owner.is_none() || self.client.borrow().has(server) {
            return;
        }
        let now = Utc::now();
        let entry = entry_list::Entry { name: ticket.server_name.clone(), ticket: raw.to_string(), since: now };
        let shared = SharedKey::of(id, &secret);
        let listed = self.client.borrow().tickets.contains_key(&server);
        let (joined, keyed) = match self.sync.borrow_mut().as_mut() {
            Some(live) => {
                let joined = !listed && live.book.join(server, entry.clone(), shared.clone(), now.timestamp_millis());
                (joined, live.book.offer_key(server, shared.clone(), now.timestamp_millis()))
            }
            None => return,
        };
        if !joined && !keyed {
            return;
        }
        if joined {
            self.client.borrow_mut().tickets.insert(server, ticket.clone());
            self.append_account(joined_record(server, &entry));
        }
        self.append_account(community_key_record(server, &shared, now.timestamp_millis(), ticket.owner));
        self.refresh_sync();
        let details = self.dms.borrow().safety.profile_policy.shows_details_to_servers();
        let state = streams::bootstrap(server, id, &secret).keys.id;
        let payload = self.client.borrow().join_payload(server, details);
        let plain = payload.as_ref().and_then(streams::encode_entry);
        let client = self.sync.borrow().as_ref().and_then(|live| live.pool.client_for(state));
        if let (Some(plain), Some(client)) = (plain, client) {
            spawn_local(async move {
                let _ = client.append(state, plain).await;
            });
        }
    }

    fn follow_server_list(self: &Rc<Self>) {
        let Some(tickets) = self.sync.borrow().as_ref().map(|live| live.book.tickets()) else { return };
        let (added, removed) = {
            let mut client = self.client.borrow_mut();
            let removed: Vec<ServerId> = client.tickets.keys().filter(|id| !tickets.contains_key(id)).copied().collect();
            for server in &removed {
                client.forget(*server);
            }
            let added: Vec<ServerTicket> = tickets.into_values().filter(|t| !client.tickets.contains_key(&t.server_id)).collect();
            for ticket in &added {
                client.tickets.insert(ticket.server_id, ticket.clone());
            }
            (added, removed)
        };
        if !removed.is_empty() {
            let this = self.clone();
            spawn_local(async move {
                if let Some(node) = this.node.lock().await.as_mut() {
                    for server in removed {
                        node.leave_server(server);
                    }
                }
                this.emit("servers-changed", Value::Null);
            });
        }
        if !added.is_empty() {
            spawn_local(self.clone().load_servers(added));
        }
        self.refresh_sync();
    }

    fn adopt(self: &Rc<Self>, server: ServerId) {
        let Some((snapshot, joined)) =
            self.sync.borrow_mut().as_mut().and_then(|live| Some((live.book.take_snapshot(server)?, live.book.joined_ms(server))))
        else {
            return;
        };
        if self.client.borrow().install_service_snapshot(server, snapshot, joined) != Ok(true) {
            return;
        }
        self.release_parked(server);
        self.emit("servers-changed", Value::Null);
        self.emit("server-event", json!(server.to_string()));
        if self.client.borrow().is_group(server) {
            self.emit("dm-changed", json!(server.to_string()));
        }
        self.announce_if_loaded();
        self.refresh_sync();
        if self.sync.borrow_mut().as_mut().is_some_and(|live| !live.gone && live.greeted.insert(server)) {
            self.say_here(server);
        }
    }

    fn release_parked(&self, server: ServerId) {
        let parked = self.sync.borrow_mut().as_mut().map(|live| {
            live.book.take_snapshot(server);
            live.book.take_parked(server)
        });
        for payload in parked.unwrap_or_default() {
            if let Some((event, action)) = streams::event_of(&payload) {
                self.apply_stored(server, event.clone(), action, false, false);
            }
        }
    }

    fn writers(&self, server: ServerId, lane: Lane) -> Vec<(StreamId, SyncClient)> {
        let guard = self.sync.borrow();
        let Some(live) = guard.as_ref() else { return Vec::new() };
        live.book.writers(server, lane).into_iter().filter_map(|stream| Some((stream, live.pool.client_for(stream)?))).collect()
    }

    pub(super) fn publish(&self, server: ServerId, payload: &WirePayload) {
        if let Some(lane) = streams::lane_of_payload(payload) {
            let Some(plain) = streams::encode_entry(payload) else { return };
            for (stream, client) in self.writers(server, lane) {
                let plain = plain.clone();
                spawn_local(async move {
                    let _ = client.append(stream, plain).await;
                });
            }
            return;
        }
        let Some((event, _)) = streams::event_of(payload) else { return };
        let Some(node) = self.sync.borrow().as_ref().map(|live| live.node) else { return };
        let Some(plain) = streams::encode_signal(node, payload) else { return };
        for (stream, client) in self.writers(server, streams::signal_lane(event)) {
            client.signal(stream, plain.clone());
        }
    }

    pub(super) fn send_to_service(self: &Rc<Self>, server: ServerId, message: &Message) {
        let payload = WirePayload::Event(ServerEvent::MessageCreate { message: message.clone() });
        let Some(plain) = streams::encode_entry(&payload) else { return };
        for (stream, client) in self.writers(server, Lane::Channel(message.channel)) {
            let (this, plain, id) = (self.clone(), plain.clone(), message.id);
            spawn_local(async move {
                if client.append(stream, plain).await.is_ok() {
                    this.settle_delivered(server, &[id]);
                }
            });
        }
    }
}
