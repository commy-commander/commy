use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use commy_core::home::{fresh_card_fits, request_proof, wants_card, HomeCard};
use commy_core::server_blocklist::Envelope;
use commy_core::server_key::PublishedKey;
use commy_core::{Identity, ServerId, UserId};
use commy_view::homes::HomeGuard;
use serde::Deserialize;
use serde_json::{json, Value};
use wasm_bindgen_futures::spawn_local;
use web_time::Instant;

use super::{to_json, Args, Session};
use crate::account;

const HOME_KEY_PATH: &str = "/api/auth/commy/home-key";
const HOME_CARD_PATH: &str = "/api/auth/commy/home-card";
const REFRESH_EVERY: Duration = Duration::from_secs(15 * 60);
const RETRY_AFTER: Duration = Duration::from_secs(5 * 60);
const CARD_CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const CARD_RETRY_AFTER: Duration = Duration::from_secs(30 * 60);
const MAX_HOMES_PER_ASK: usize = 100;

#[derive(Default)]
pub(super) struct HomeWatch {
    guard: HomeGuard,
    next_refresh: Option<Instant>,
    refreshing: bool,
    next_card: Option<Instant>,
    carding: bool,
}

#[derive(Deserialize)]
struct ListPointer {
    url: String,
    key: PublishedKey,
}

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    #[serde(rename = "serverBlocklist")]
    server_blocklist: Option<ListPointer>,
}

#[derive(Deserialize)]
struct HomeKeyReply {
    key: PublishedKey,
}

#[derive(Deserialize)]
struct CardReply {
    card: HomeCard,
}

fn due(at: Option<Instant>) -> bool {
    at.is_none_or(|at| Instant::now() >= at)
}

impl Session {
    pub(super) fn keep_homes_fresh(self: &Rc<Self>) {
        let (list, card) = {
            let mut watch = self.homes.borrow_mut();
            let list = due(watch.next_refresh) && !watch.refreshing;
            let card = due(watch.next_card) && !watch.carding && self.client.borrow().profile().is_some();
            watch.refreshing |= list;
            watch.carding |= card;
            (list, card)
        };
        if list {
            spawn_local(self.clone().refresh_server_list());
        }
        if card {
            spawn_local(self.clone().renew_home_card());
        }
    }

    async fn refresh_server_list(self: Rc<Self>) {
        let outcome = self.fetch_server_list().await;
        let changed = {
            let mut watch = self.homes.borrow_mut();
            watch.refreshing = false;
            watch.next_refresh = Some(Instant::now() + if outcome.is_ok() { REFRESH_EVERY } else { RETRY_AFTER });
            match outcome {
                Ok((key, envelope)) => watch.guard.take_list(&self.origin, &key, &envelope).unwrap_or(false),
                Err(()) => false,
            }
        };
        if changed {
            self.refresh_sync();
            self.emit("homes-changed", Value::Null);
        }
    }

    async fn fetch_server_list(&self) -> Result<(PublishedKey, Envelope), ()> {
        let reply = self.request("GET", account::DISCOVERY_PATH, None).await.map_err(|_| ())?;
        let doc: Discovery = serde_json::from_str(&reply.body).map_err(|_| ())?;
        let pointer = doc.server_blocklist.filter(|_| doc.issuer == self.origin).ok_or(())?;
        let path = pointer.url.strip_prefix(&self.origin).filter(|path| path.starts_with('/')).ok_or(())?;
        let reply = self.request("GET", path, None).await.map_err(|_| ())?;
        if !reply.ok() {
            return Err(());
        }
        let envelope: Envelope = serde_json::from_str(&reply.body).map_err(|_| ())?;
        Ok((pointer.key, envelope))
    }

    pub(super) fn blocks_sync_origin(&self, origin: &str) -> bool {
        self.homes.borrow().guard.blocks_origin(origin)
    }

    pub(super) fn hides_author(&self, server: ServerId, author: UserId) -> bool {
        let client = self.client.borrow();
        let servers = client.servers.lock().unwrap();
        let watch = self.homes.borrow();
        servers.get(&server).and_then(|state| state.profiles.get(&author)).is_some_and(|profile| watch.guard.hides(profile))
    }

    pub(super) async fn home_status(&self, args: Args) -> Result<Value, String> {
        let mut asked: Vec<String> = args.homes.unwrap_or_default().into_iter().filter(|home| commy_core::sync_host::is_sync_host(home)).collect();
        asked.sort();
        asked.dedup();
        asked.truncate(MAX_HOMES_PER_ASK);
        for home in &asked {
            if self.homes.borrow().guard.needs_lookup(home) {
                let key = self.look_up_home(home).await;
                self.homes.borrow_mut().guard.remember_key(home, key);
            }
        }
        let watch = self.homes.borrow();
        to_json(&asked.iter().map(|home| watch.guard.status(home)).collect::<Vec<_>>())
    }

    async fn look_up_home(&self, home: &str) -> Option<PublishedKey> {
        let path = format!("{HOME_KEY_PATH}?home={}", account::encode_component(home));
        let reply = self.request("GET", &path, None).await.ok()?;
        if !reply.ok() {
            return None;
        }
        serde_json::from_str::<HomeKeyReply>(&reply.body).ok().map(|reply| reply.key)
    }

    async fn renew_home_card(self: Rc<Self>) {
        let fresh = self.fetch_card().await;
        {
            let mut watch = self.homes.borrow_mut();
            watch.carding = false;
            watch.next_card = Some(Instant::now() + if fresh.is_ok() { CARD_CHECK_EVERY } else { CARD_RETRY_AFTER });
        }
        if let Ok(Some(card)) = fresh {
            let _ = self.edit_profile(false, |profile| profile.home = Some(card));
        }
    }

    async fn fetch_card(&self) -> Result<Option<HomeCard>, ()> {
        let Some(secret) = self.client.borrow().identity_secret() else { return Ok(None) };
        let Some(profile) = self.client.borrow().profile() else { return Ok(None) };
        let identity = Identity::from_bytes(&secret);
        let (user, now) = (profile.id, Utc::now().timestamp_millis());
        let current = profile.home.filter(|card| card.user == user && card.identity == identity.public());
        if !wants_card(current.as_ref(), &self.origin, now) {
            return Ok(None);
        }
        let body = json!({
            "user": user.to_string(),
            "identity": identity.public().to_hex(),
            "at": now,
            "proof": request_proof(&identity, &self.origin, user, now),
        });
        let reply = self.request("POST", HOME_CARD_PATH, Some(body)).await.map_err(|_| ())?;
        if !reply.ok() {
            return Err(());
        }
        let card = serde_json::from_str::<CardReply>(&reply.body).map_err(|_| ())?.card;
        Ok(fresh_card_fits(&card, &self.origin, user, &identity.public(), now).then_some(card))
    }
}
