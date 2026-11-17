use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use commy_core::home::{fresh_card_fits, request_proof, wants_card, HomeCard};
use commy_core::server_blocklist::Envelope;
use commy_core::server_key::PublishedKey;
use commy_core::{Identity, ServerId, UserId};
use commy_view::homes::HomeGuard;
use commy_view::listed::{JoinHost, ListedServers};
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
const TRUSTED_HOSTS_KEY: &str = "commy.trusted-sync-hosts";
const MAX_TRUSTED_HOSTS: usize = 500;

#[derive(Default)]
pub(super) struct HomeWatch {
    guard: HomeGuard,
    listed: ListedServers,
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
    #[serde(rename = "serverList")]
    server_list: Option<ListPointer>,
}

struct OfficialLists {
    blocklist: Option<(PublishedKey, Envelope)>,
    listed: Option<(PublishedKey, Envelope)>,
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
        let outcome = self.fetch_official_lists().await;
        let (changed, listed_changed) = {
            let mut watch = self.homes.borrow_mut();
            watch.refreshing = false;
            let fetched = outcome.as_ref().is_ok_and(|lists| lists.blocklist.is_some());
            watch.next_refresh = Some(Instant::now() + if fetched { REFRESH_EVERY } else { RETRY_AFTER });
            match outcome {
                Ok(lists) => {
                    let changed = lists.blocklist.is_some_and(|(key, envelope)| watch.guard.take_list(&self.origin, &key, &envelope).unwrap_or(false));
                    let listed_changed = lists.listed.is_some_and(|(key, envelope)| watch.listed.take_list(&self.origin, &key, &envelope).unwrap_or(false));
                    (changed, listed_changed)
                }
                Err(()) => (false, false),
            }
        };
        if changed {
            self.refresh_sync();
            self.emit("homes-changed", Value::Null);
        }
        if listed_changed {
            self.emit("listed-servers-changed", Value::Null);
        }
    }

    async fn fetch_signed(&self, pointer: Option<ListPointer>) -> Option<(PublishedKey, Envelope)> {
        let pointer = pointer?;
        let path = pointer.url.strip_prefix(&self.origin).filter(|path| path.starts_with('/'))?;
        let reply = self.request("GET", path, None).await.ok()?;
        if !reply.ok() {
            return None;
        }
        let envelope: Envelope = serde_json::from_str(&reply.body).ok()?;
        Some((pointer.key, envelope))
    }

    async fn fetch_official_lists(&self) -> Result<OfficialLists, ()> {
        let reply = self.request("GET", account::DISCOVERY_PATH, None).await.map_err(|_| ())?;
        let doc: Discovery = serde_json::from_str(&reply.body).map_err(|_| ())?;
        if doc.issuer != self.origin {
            return Err(());
        }
        let blocklist = self.fetch_signed(doc.server_blocklist).await;
        let listed = self.fetch_signed(doc.server_list).await;
        Ok(OfficialLists { blocklist, listed })
    }

    fn trusted_hosts(&self) -> Vec<String> {
        let saved = self.host.load(TRUSTED_HOSTS_KEY).and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok()).unwrap_or_default();
        saved.into_iter().filter(|host| commy_core::sync_host::is_sync_host(host)).collect()
    }

    pub(super) fn join_host(&self, sync_host: Option<&str>) -> Option<JoinHost> {
        let trusted = self.trusted_hosts();
        self.homes.borrow().listed.join_host(sync_host, &self.origin, &self.origin, &trusted, Utc::now().timestamp_millis())
    }

    pub(super) fn trust_sync_host(&self, args: Args) -> Result<Value, String> {
        let origin = args.host.as_deref().and_then(commy_core::sync_host::clean_sync_host).ok_or("That is not a server address")?;
        let mut trusted = self.trusted_hosts();
        if !trusted.contains(&origin) {
            trusted.push(origin);
            let excess = trusted.len().saturating_sub(MAX_TRUSTED_HOSTS);
            trusted.drain(..excess);
            self.host.save(TRUSTED_HOSTS_KEY, &serde_json::to_string(&trusted).map_err(|e| e.to_string())?);
        }
        Ok(Value::Null)
    }

    pub(super) fn listed_servers(&self) -> Result<Value, String> {
        let watch = self.homes.borrow();
        to_json(&watch.listed.homes(Utc::now().timestamp_millis(), |origin| watch.guard.blocks_origin(origin)))
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
