use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use chrono::Utc;
use commy_core::{ServerId, UserId, UserProfile};
use commy_net::sync_streams::{self, DmEntry};
use commy_sync::endpoint::accepted_url;
use commy_sync::wake::RingTags;
use commy_sync::{Pool, StreamId, StreamKeys, SyncEvent};
use commy_view::away;
use commy_view::dms::{self, EPOCHS};
use serde::Deserialize;
use tokio::sync::mpsc;
use wasm_bindgen_futures::spawn_local;

use super::Session;
use crate::account;
use crate::away_book::{AwayBook, Wanted};

const HOME_KEY_PATH: &str = "/api/auth/commy/home-key";

#[derive(Default)]
pub(super) struct Away {
    pool: Option<Pool>,
    book: AwayBook,
}

#[derive(Deserialize)]
struct HomeReply {
    sync: Option<String>,
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

impl Away {
    pub(super) fn wait_for_homes(&mut self, homes: Vec<String>) {
        self.book.wait_for(homes);
    }
}

impl Session {
    pub(super) fn known_homes(&self) -> away::HomeBook {
        self.away.borrow().book.homes.clone()
    }

    pub(super) fn away_home_of(&self, profile: &UserProfile) -> Option<String> {
        away::away_home(&self.origin, profile).filter(|home| !self.blocks_sync_origin(home))
    }

    fn partner_home(&self, partner: UserId) -> Option<String> {
        let client = self.client.borrow();
        let servers = client.servers.lock().unwrap();
        let profile = self.dms.borrow().partner_profile(&servers, partner)?;
        self.away_home_of(&profile)
    }

    pub(super) fn partner_sync(&self, partner: UserId) -> Option<String> {
        let home = self.partner_home(partner)?;
        self.away.borrow().book.homes.url(&home).map(str::to_string)
    }

    pub(super) async fn partner_sync_now(&self, partner: UserId) -> Result<Option<String>, ()> {
        let Some(home) = self.partner_home(partner) else { return Ok(None) };
        self.home_sync_now(&home).await.map(Some).ok_or(())
    }

    pub(super) async fn home_sync_now(&self, home: &str) -> Option<String> {
        if let Some(url) = self.away.borrow().book.homes.url(home) {
            return Some(url.to_string());
        }
        let asking = self.away.borrow_mut().book.homes.to_ask([home], now_ms());
        if !asking.is_empty() {
            self.learn_home(home).await;
        }
        self.away.borrow().book.homes.url(home).map(str::to_string)
    }

    async fn learn_home(&self, home: &str) -> bool {
        let path = format!("{HOME_KEY_PATH}?home={}", account::encode_component(home));
        let reply = self.request("GET", &path, None).await.ok().filter(|reply| reply.ok());
        let offered = reply.and_then(|reply| serde_json::from_str::<HomeReply>(&reply.body).ok()).and_then(|reply| reply.sync);
        let url = accepted_url(home, offered.as_deref());
        self.away.borrow_mut().book.homes.learn(home, url, now_ms())
    }

    pub(super) fn start_away(self: &Rc<Self>) {
        if self.away.borrow().pool.is_some() {
            return;
        }
        let (pool, events) = Pool::new(&self.origin);
        self.away.borrow_mut().pool = Some(pool);
        spawn_local(self.clone().run_away(events));
    }

    pub(super) fn stop_away(&self) {
        let mut away = self.away.borrow_mut();
        away.pool = None;
        away.book.forget_streams();
    }

    pub(super) fn tend_away(self: &Rc<Self>) {
        let pending = self.away.borrow_mut().book.take_waiting();
        let due = self.away.borrow_mut().book.homes.to_ask(pending.iter().map(String::as_str), now_ms());
        if due.is_empty() {
            return;
        }
        let this = self.clone();
        spawn_local(async move {
            let mut changed = false;
            for home in due {
                changed |= this.learn_home(&home).await;
            }
            if changed {
                this.refresh_away_dms();
                this.refresh_sync();
            }
        });
    }

    pub(super) fn refresh_away_dms(&self) {
        let Some((_, identity)) = self.me_and_key() else { return };
        let partners = self.dms.borrow().pair_partners();
        let epochs = sync_streams::recent_dm_epochs(Utc::now(), EPOCHS);
        let mut wanted: Wanted = HashMap::new();
        let mut unknown: Vec<String> = Vec::new();
        for (user, key) in partners {
            let Some(home) = self.partner_home(user) else { continue };
            let Some(url) = self.away.borrow().book.homes.url(&home).map(str::to_string) else {
                unknown.push(home);
                continue;
            };
            for epoch in &epochs {
                if let Some(keys) = sync_streams::dm_stream(&identity, &key, epoch) {
                    wanted.insert(keys.id, (user, key, url.clone(), keys));
                }
            }
        }
        self.follow_away(wanted);
        self.away.borrow_mut().book.wait_for(unknown);
    }

    fn follow_away(&self, wanted: Wanted) {
        let mut guard = self.away.borrow_mut();
        let away = &mut *guard;
        let Some(pool) = away.pool.as_mut() else { return };
        let plan = away.book.plan(wanted);
        for stream in plan.unfollow {
            pool.unfollow(stream);
        }
        for (url, group) in plan.follow {
            pool.follow_together_tail_at(&url, group, None);
        }
    }

    async fn run_away(self: Rc<Self>, mut events: mpsc::UnboundedReceiver<SyncEvent>) {
        while let Some(event) = events.recv().await {
            match event {
                SyncEvent::Entry { stream, seq, at, plain } => self.on_away_entry(stream, seq, at, &plain),
                SyncEvent::Unreadable { stream, seq } => {
                    self.away.borrow_mut().book.advance(stream, seq);
                }
                SyncEvent::CaughtUp { stream, seq } => self.away.borrow_mut().book.caught_up(stream, seq),
                _ => {}
            }
        }
    }

    fn on_away_entry(self: &Rc<Self>, stream: StreamId, seq: u64, at: i64, plain: &[u8]) {
        let Some((partner, key, live_now)) = self.away.borrow_mut().book.advance(stream, seq) else { return };
        let Some((me, identity)) = self.me_and_key() else { return };
        let Some(DmEntry::FromPartner(envelope)) = sync_streams::decode_dm(plain, me.id, &identity.public(), partner, &key) else { return };
        let fresh_call = dms::is_call(&envelope.message) && dms::call_fresh(at, now_ms());
        if !dms::syncable(&envelope.message) && !fresh_call {
            return;
        }
        let Some(node) = super::dms::node_of(&envelope.message.from_node) else { return };
        self.take_dm(envelope, node, live_now);
    }

    pub(super) async fn append_away(&self, url: &str, keys: StreamKeys, plain: Vec<u8>) -> bool {
        let stream = keys.id;
        let (client, borrowed) = {
            let mut away = self.away.borrow_mut();
            let Some(pool) = away.pool.as_mut() else { return false };
            let borrowed = !pool.follows(stream);
            if borrowed {
                pool.follow_together_tail_at(url, vec![(keys, 0)], Some(0));
            }
            (pool.client_for(stream), borrowed)
        };
        let sent = match client {
            Some(client) => matches!(n0_future::time::timeout(super::dms::HOLD_WAIT, client.append(stream, plain)).await, Ok(Ok(_))),
            None => false,
        };
        if borrowed {
            if let Some(pool) = self.away.borrow_mut().pool.as_mut() {
                pool.unfollow(stream);
            }
        }
        sent
    }

    pub(super) fn member_homes(&self, server: ServerId) -> BTreeMap<String, String> {
        let client = self.client.borrow();
        let servers = client.servers.lock().unwrap();
        let Some(state) = servers.get(&server) else { return BTreeMap::new() };
        state
            .server
            .members
            .iter()
            .filter_map(|member| Some((member.user.to_string(), self.away_home_of(state.profiles.get(&member.user)?)?)))
            .collect()
    }

    pub(super) fn ring_server_wake_everywhere(self: &Rc<Self>, split: BTreeMap<Option<String>, RingTags>, preview: Option<String>) {
        for (home, tags) in split {
            match home {
                None => self.ring_server_wake(tags, preview.clone(), None),
                Some(home) => {
                    let (this, preview) = (self.clone(), preview.clone());
                    spawn_local(async move {
                        if let Some(url) = this.home_sync_now(&home).await {
                            this.ring_server_wake(tags, preview, Some(url));
                        }
                    });
                }
            }
        }
    }
}
