use std::collections::HashSet;
use std::sync::Mutex;

use commy_core::channel_key::Keyring;
use commy_core::{ServerId, UserId};
use commy_net::community_streams as streams;
use commy_sync::errands::{self, owner_work, ClaimBook, Errand, Finished, Instant, OwnerWork};
use commy_sync::{StreamId, StreamKeys};

use crate::state::App;

static CLAIMS: Mutex<Option<ClaimBook<ServerId>>> = Mutex::new(None);

pub struct Plan {
    root: [u8; 32],
    streams: Vec<StreamKeys>,
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
    Some(Plan { root: identity.to_bytes(), streams: every.into_iter().map(|(_, keys)| keys).collect() })
}

fn owned_servers(app: &App, me: UserId) -> HashSet<ServerId> {
    app.servers.lock().unwrap().iter().filter(|(_, state)| state.server.owner == me && !state.is_deleted()).map(|(sid, _)| *sid).collect()
}

fn still_here(app: &App, sid: ServerId) -> bool {
    app.servers.lock().unwrap().get(&sid).is_some_and(|state| !state.is_deleted())
}

pub fn tend(app: &App) {
    let (Some(url), Some(identity)) = (crate::sync_link::url(), app.identity()) else { return };
    let Some(me) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { return };
    let owned = owned_servers(app, me);
    let followed = crate::community_sync::followed_streams();
    let round = CLAIMS.lock().unwrap().get_or_insert_with(ClaimBook::default).round(&url, identity.to_bytes(), owned, &followed, Instant::now());
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
    {
        let mut guard = CLAIMS.lock().unwrap();
        let Some(book) = guard.as_mut() else { return };
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
        if let Some(book) = CLAIMS.lock().unwrap().as_mut() {
            book.finish(&asked, answered.as_deref().unwrap_or_default(), Instant::now());
        }
    });
}

pub fn erase_deleted(plan: Plan, sid: ServerId, notices: Vec<(StreamId, u64)>) {
    let Some(url) = crate::sync_link::url() else { return };
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
