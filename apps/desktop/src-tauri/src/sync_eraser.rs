use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;

use commy_core::channel_key::{KeyScope, Keyring};
use commy_core::{ServerId, UserId};
use commy_net::community_streams as streams;
use commy_sync::errands::{self, owner_work, ClaimBook, Errand, Finished, Instant, OwnerWork};
use commy_sync::{StreamId, StreamKeys};

use crate::state::App;

static CLAIMS: Mutex<Option<HashMap<String, ClaimBook<ServerId>>>> = Mutex::new(None);
static FRESH: Mutex<Option<HashSet<(ServerId, KeyScope)>>> = Mutex::new(None);
const CLAIM_FIRST_WAIT: Duration = Duration::from_secs(15);

pub fn wants_fresh_streams(sid: ServerId) -> Vec<KeyScope> {
    FRESH.lock().unwrap().iter().flatten().filter(|(server, _)| *server == sid).map(|(_, scope)| *scope).collect()
}

pub fn fresh_streams_made(sid: ServerId, scope: KeyScope) {
    if let Some(wanted) = FRESH.lock().unwrap().as_mut() {
        wanted.remove(&(sid, scope));
    }
}

fn want_fresh_streams(stolen: &[StreamId], scopes: &HashMap<StreamId, (ServerId, KeyScope)>) {
    let mut guard = FRESH.lock().unwrap();
    let wanted = guard.get_or_insert_with(HashSet::new);
    wanted.extend(stolen.iter().filter_map(|stream| scopes.get(stream).copied()));
}

fn scopes_of_streams(app: &App, sids: impl IntoIterator<Item = ServerId>) -> HashMap<StreamId, (ServerId, KeyScope)> {
    let Some(identity) = app.identity() else { return HashMap::new() };
    let Some(me) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { return HashMap::new() };
    let holder = crate::channel_lock::holder_of(app);
    let mut out = HashMap::new();
    for sid in sids {
        let given = crate::community_sync::given_key(app, sid);
        let servers = app.servers.lock().unwrap();
        let Some(state) = servers.get(&sid) else { continue };
        let Some(community) = holder.community_ring(state, given) else { continue };
        for (_, keys) in streams::every_stream(state, &community, |_| Keyring::default()) {
            out.insert(keys.id, (sid, KeyScope::Community));
        }
        for (lane, keys) in streams::every_stream(state, &community, |channel| Keyring::unlock(state, channel, me, &identity)) {
            if let (streams::Lane::Channel(channel), false) = (lane, out.contains_key(&keys.id)) {
                out.insert(keys.id, (sid, KeyScope::Channel(channel)));
            }
        }
    }
    out
}

pub async fn claim_before_sharing(app: &App, sid: ServerId) {
    let (Some(account_url), Some(identity)) = (crate::sync_link::url(), app.identity()) else { return };
    let Some(me) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { return };
    if !owned_servers(app, me).contains(&sid) {
        return;
    }
    let Some(plan) = plan(app, sid) else { return };
    let Some(url) = crate::community_sync::community_url(plan.chosen_host.as_deref(), &account_url) else { return };
    let work = CLAIMS.lock().unwrap().get_or_insert_with(HashMap::new).entry(url.clone()).or_default().claims_for(&url, identity.to_bytes(), &plan.streams, Instant::now());
    if work.is_empty() {
        return;
    }
    let scopes = scopes_of_streams(app, [sid]);
    let asked: Vec<(StreamId, Errand)> = work.iter().map(|w| (w.keys.id, w.errand)).collect();
    let answered = match tokio::time::timeout(CLAIM_FIRST_WAIT, errands::run(&url, work)).await {
        Ok(Ok(answered)) => answered,
        _ => {
            tracing::debug!("could not claim new server streams before sharing their keys");
            return;
        }
    };
    if let Some(book) = CLAIMS.lock().unwrap().as_mut().and_then(|books| books.get_mut(&url)) {
        let stolen = book.finish(&asked, &answered, Instant::now());
        want_fresh_streams(&stolen, &scopes);
    }
}

pub struct Plan {
    root: [u8; 32],
    streams: Vec<StreamKeys>,
    chosen_host: Option<String>,
}

impl Plan {
    pub fn claims(&self) -> Vec<OwnerWork> {
        owner_work(&self.root, &self.streams, |_| Errand::Claim)
    }

    pub fn stream_ids(&self) -> Vec<StreamId> {
        self.streams.iter().map(|keys| keys.id).collect()
    }

    pub fn socket(&self, account_url: &str) -> Option<String> {
        crate::community_sync::community_url(self.chosen_host.as_deref(), account_url)
    }
}

pub fn plan(app: &App, sid: ServerId) -> Option<Plan> {
    let identity = app.identity()?;
    let me = app.profile.lock().unwrap().as_ref()?.id;
    let holder = crate::channel_lock::holder_of(app);
    let given = crate::community_sync::given_key(app, sid);
    let servers = app.servers.lock().unwrap();
    let state = servers.get(&sid)?;
    let community = holder.community_ring(state, given)?;
    let every = streams::every_stream(state, &community, |channel| Keyring::unlock(state, channel, me, &identity));
    Some(Plan { root: identity.to_bytes(), streams: every.into_iter().map(|(_, keys)| keys).collect(), chosen_host: state.server.sync_host.clone() })
}

fn owned_servers(app: &App, me: UserId) -> HashSet<ServerId> {
    app.servers.lock().unwrap().iter().filter(|(_, state)| state.server.owner == me && !state.is_deleted()).map(|(sid, _)| *sid).collect()
}

fn still_here(app: &App, sid: ServerId) -> bool {
    app.servers.lock().unwrap().get(&sid).is_some_and(|state| !state.is_deleted())
}

fn host_of(sid: ServerId, hosts: &HashMap<ServerId, String>, account_url: &str) -> String {
    hosts.get(&sid).cloned().unwrap_or_else(|| account_url.to_string())
}

pub fn by_host<V>(items: HashMap<ServerId, V>, hosts: &HashMap<ServerId, String>, account_url: &str) -> HashMap<String, HashMap<ServerId, V>> {
    let mut grouped: HashMap<String, HashMap<ServerId, V>> = HashMap::new();
    for (sid, value) in items {
        grouped.entry(host_of(sid, hosts, account_url)).or_default().insert(sid, value);
    }
    grouped
}

pub fn tend(app: &App) {
    let (Some(account_url), Some(identity)) = (crate::sync_link::url(), app.identity()) else { return };
    let Some(me) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { return };
    let hosts = crate::community_sync::community_hosts();
    let owned = owned_servers(app, me);
    let followed = by_host(crate::community_sync::followed_streams(), &hosts, &account_url);
    let mut every_host: HashSet<String> = followed.keys().cloned().collect();
    every_host.insert(account_url.clone());
    for url in every_host {
        let owned_here: HashSet<ServerId> = owned.iter().copied().filter(|sid| host_of(*sid, &hosts, &account_url) == url).collect();
        let followed_here = followed.get(&url).cloned().unwrap_or_default();
        tend_host(app, url, identity.to_bytes(), owned_here, followed_here);
    }
}

fn tend_host(app: &App, url: String, root: [u8; 32], owned: HashSet<ServerId>, followed: HashMap<ServerId, Vec<StreamKeys>>) {
    let owned_here = owned.clone();
    let round = {
        let mut guard = CLAIMS.lock().unwrap();
        let books = guard.get_or_insert_with(HashMap::new);
        books.entry(url.clone()).or_default().round(&url, root, owned, &followed, Instant::now())
    };
    let mut work: Vec<OwnerWork> = Vec::new();
    for sid in round.given_up.into_iter().filter(|sid| still_here(app, *sid)) {
        if let Some(plan) = plan(app, sid) {
            tracing::info!("letting go of the sync erasers of {sid} for its new owner");
            work.extend(owner_work(&plan.root, &plan.streams, |_| Errand::Release));
        }
    }
    work.extend(round.claims);
    if work.is_empty() {
        return;
    }
    let scopes = scopes_of_streams(app, owned_here);
    {
        let mut guard = CLAIMS.lock().unwrap();
        let Some(book) = guard.as_mut().and_then(|books| books.get_mut(&url)) else { return };
        book.letting_go(work.iter().filter(|w| w.errand == Errand::Release).map(|w| w.keys.id));
        if !book.start() {
            return;
        }
    }
    tauri::async_runtime::spawn(async move {
        let asked: Vec<(StreamId, Errand)> = work.iter().map(|w| (w.keys.id, w.errand)).collect();
        let answered = errands::run(&url, work).await;
        if let Err(e) = &answered {
            tracing::debug!("could not claim server streams on the sync service: {e}");
        }
        let stolen = match CLAIMS.lock().unwrap().as_mut().and_then(|books| books.get_mut(&url)) {
            Some(book) => book.finish(&asked, answered.as_deref().unwrap_or_default(), Instant::now()),
            None => Vec::new(),
        };
        want_fresh_streams(&stolen, &scopes);
    });
}

pub fn erase_deleted(plan: Plan, sid: ServerId, notices: Vec<(StreamId, u64)>) {
    let Some(account_url) = crate::sync_link::url() else { return };
    let Some(url) = crate::community_sync::community_url(plan.chosen_host.as_deref(), &account_url) else { return };
    let work = owner_work(&plan.root, &plan.streams, |stream| Errand::Erase { through: streams::erase_through(stream, &notices) });
    tauri::async_runtime::spawn(async move {
        let finished = errands::keep_at(&url, work, &errands::ERASE_WAITS).await;
        match finished {
            Finished { too_old: true, .. } => tracing::warn!("the sync service cannot erase yet; the copies of {sid} expire on their own"),
            Finished { left: 0, ref refused, done, .. } if refused.is_empty() => tracing::info!("erased the {done} sync streams of deleted server {sid}"),
            Finished { left, refused, done, .. } => {
                tracing::warn!("erased {done} sync streams of deleted server {sid}; {} were refused and {left} unreachable expire on their own", refused.len())
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_community_is_tended_on_its_own_sync_host() {
        let (home, away, moved) = (ServerId::new(), ServerId::new(), ServerId::new());
        let hosts = HashMap::from([(away, "wss://sync.example.org/sync".to_string()), (moved, "wss://commy.chat/sync".to_string())]);
        let items = HashMap::from([(home, 1), (away, 2), (moved, 3)]);
        let grouped = by_host(items, &hosts, "wss://commy.chat/sync");
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped["wss://commy.chat/sync"], HashMap::from([(home, 1), (moved, 3)]));
        assert_eq!(grouped["wss://sync.example.org/sync"], HashMap::from([(away, 2)]));
    }
}
