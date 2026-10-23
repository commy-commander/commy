use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use commy_core::{Message, ServerEvent, ServerId};
use commy_net::community_streams::{self as streams, Lane};
use commy_net::entry_list::EntryList;
use commy_net::sealed::SealKey;
use commy_net::{sync_streams, ServerTicket, WirePayload};
use commy_sync::{Pool, StreamId, SyncClient, SyncEvent};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use wasm_bindgen_futures::spawn_local;

use super::Session;
use crate::client::Applied;
use crate::sync_book::{Book, Change, Record};

const REFRESH_EVERY: Duration = Duration::from_secs(30);

pub(super) struct Live {
    pool: Pool,
    account: StreamId,
    account_live: bool,
    book: Book,
}

impl Session {
    pub(super) async fn start_sync(self: Rc<Self>, key: SealKey, list: EntryList) {
        if self.sync.borrow().is_some() {
            return;
        }
        let Some(url) = self.sync_service_url().await else { return };
        let (mut pool, events) = Pool::new(&url);
        let stream = sync_streams::account_stream(&key);
        let account = stream.id;
        pool.follow(stream, 0);
        *self.sync.borrow_mut() = Some(Live { pool, account, account_live: false, book: Book::new(list) });
        spawn_local(self.clone().run_sync(events));
        spawn_local(self.clone().tend_sync());
        self.refresh_sync();
    }

    pub(super) fn stop_sync(&self) {
        self.sync.borrow_mut().take();
    }

    pub(super) fn nudge_sync(&self) {
        if let Some(live) = self.sync.borrow().as_ref() {
            live.pool.nudge();
        }
    }

    async fn run_sync(self: Rc<Self>, mut events: mpsc::UnboundedReceiver<SyncEvent>) {
        while let Some(event) = events.recv().await {
            match event {
                SyncEvent::Entry { stream, seq, plain, .. } => self.on_sync_entry(stream, seq, &plain),
                SyncEvent::Unreadable { stream, seq } => {
                    if let Some(live) = self.sync.borrow_mut().as_mut() {
                        live.book.advance(stream, seq);
                    }
                }
                SyncEvent::CaughtUp { stream, seq } => self.on_sync_caught_up(stream, seq),
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
        }
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
    }

    fn on_sync_entry(self: &Rc<Self>, stream: StreamId, seq: u64, plain: &[u8]) {
        let routed = {
            let mut guard = self.sync.borrow_mut();
            let Some(live) = guard.as_mut() else { return };
            if stream == live.account {
                let change = live.book.fold(Record::parse(plain));
                let settled = live.account_live;
                drop(guard);
                if settled {
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
        let applied = self.apply_event(server, event, action, rings, now_live);
        if key_event && now_live && applied != Applied::Nothing {
            self.refresh_sync();
        }
    }

    fn on_sync_caught_up(self: &Rc<Self>, stream: StreamId, seq: u64) {
        let server = {
            let mut guard = self.sync.borrow_mut();
            let Some(live) = guard.as_mut() else { return };
            if stream == live.account {
                live.account_live = true;
                drop(guard);
                self.on_account_change(Change::Servers);
                return;
            }
            live.book.caught_up(stream, seq);
            let Some((server, _)) = live.book.route(stream) else { return };
            server
        };
        if self.client.borrow().has(server) {
            self.release_parked(server);
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
        self.refresh_sync();
    }

    fn release_parked(&self, server: ServerId) {
        let parked = self.sync.borrow_mut().as_mut().map(|live| {
            live.book.take_snapshot(server);
            live.book.take_parked(server)
        });
        for payload in parked.unwrap_or_default() {
            if let Some((event, action)) = streams::event_of(&payload) {
                self.apply_event(server, event.clone(), action, false, false);
            }
        }
    }

    fn writers(&self, server: ServerId, lane: Lane) -> Vec<(StreamId, SyncClient)> {
        let guard = self.sync.borrow();
        let Some(live) = guard.as_ref() else { return Vec::new() };
        live.book.writers(server, lane).into_iter().filter_map(|stream| Some((stream, live.pool.client_for(stream)?))).collect()
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
