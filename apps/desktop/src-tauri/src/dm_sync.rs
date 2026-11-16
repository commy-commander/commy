use crate::lock_or_recover::LockOrRecover;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use commy_core::dm::{DmAction, DmMessage};
use commy_core::{IdentityKey, MessageId, ServerId, UserId, UserProfile};
use commy_net::inbox;
use commy_net::node::IncomingDm;
use commy_net::sync_streams::{self, DmEntry};
use commy_net::DmEnvelope;
use commy_sync::wake::DAY_MS;
use data_encoding::BASE64;
use commy_sync::{StreamId, StreamKeys, SyncEvent};
use tauri::{AppHandle, Manager};

pub use commy_view::dms::{call_fresh, is_call, pick_partners, rings, should_ring, syncable};
use commy_view::nudge::is_nudge;
use commy_view::dms::{EPOCHS, RECENT_MS};

use crate::catchup::AppAccess;
use crate::state::App;
use crate::sync_link::{self, Pool};
use crate::{account_log, away_homes, outbox, push_wake};

const REFRESH_EVERY: Duration = Duration::from_secs(60 * 60);
const HOLD_WAIT: Duration = Duration::from_secs(8);

struct Partner {
    key: IdentityKey,
    streams: BTreeMap<String, StreamId>,
}

struct Live {
    url: String,
    dir: PathBuf,
    me: UserId,
    me_key: IdentityKey,
    pool: Pool,
    partners: HashMap<UserId, Partner>,
    owners: HashMap<StreamId, UserId>,
    inboxes: HashMap<StreamId, (Option<ServerId>, StreamKeys)>,
    caught_up: HashSet<StreamId>,
    away: Pool,
    away_streams: HashMap<StreamId, (UserId, String)>,
    away_caught_up: HashSet<StreamId>,
    task: tauri::async_runtime::JoinHandle<()>,
}

static LIVE: Mutex<Option<Live>> = Mutex::new(None);

pub fn nudge() {
    if let Some(live) = LIVE.lock_or_recover().as_ref() {
        live.pool.nudge();
    }
}


fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn partner_key(app: &App, user: UserId) -> Option<IdentityKey> {
    let pinned = app.dms.lock_or_recover().pinned_key(user);
    pinned.or_else(|| outbox::member_profile(&app.servers.lock_or_recover(), user).and_then(|p| p.identity))
}

pub(crate) fn partner_profile(app: &App, user: UserId) -> Option<UserProfile> {
    let known = app.dms.lock_or_recover().contacts.get(&user).cloned();
    known.or_else(|| outbox::member_profile(&app.servers.lock_or_recover(), user)).filter(|p| p.identity.is_some())
}

fn last_message_ms(app: &App, user: UserId) -> Option<i64> {
    app.dms.lock_or_recover().conversation(user).last().map(|m| m.created_at.timestamp_millis())
}


fn wanted(app: &App) -> Vec<(UserId, IdentityKey)> {
    let partners = app.dms.lock_or_recover().partners();
    let local: Vec<(UserId, i64)> = partners.into_iter().filter_map(|user| Some((user, last_message_ms(app, user)?))).collect();
    pick_partners(&account_log::partners(app), &local, now_ms())
        .into_iter()
        .filter_map(|user| Some((user, partner_key(app, user)?)))
        .collect()
}

pub fn ensure(handle: &AppHandle) {
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move { start(&handle) });
}

pub fn stop() {
    if let Some(old) = LIVE.lock_or_recover().take() {
        old.task.abort();
    }
}

fn start(handle: &AppHandle) {
    let app = handle.state::<App>();
    let Some(url) = sync_link::url() else {
        stop();
        return;
    };
    let (Some(dir), Some(me), Some(identity)) =
        (app.data_dir().cloned(), app.profile.lock_or_recover().as_ref().map(|p| p.id), app.identity())
    else {
        return;
    };
    let me_key = identity.public();
    let fresh = {
        let mut live = LIVE.lock_or_recover();
        let same = live.as_ref().is_some_and(|l| l.url == url && l.dir == dir && l.me == me && l.me_key == me_key);
        if !same {
            if let Some(old) = live.take() {
                old.task.abort();
            }
            let (pool, events) = Pool::new(&url);
            let (away, away_events) = Pool::new(&url);
            let task = tauri::async_runtime::spawn(run(handle.clone(), events, away_events));
            *live = Some(Live {
                url,
                dir,
                me,
                me_key,
                pool,
                partners: HashMap::new(),
                owners: HashMap::new(),
                inboxes: HashMap::new(),
                caught_up: HashSet::new(),
                away,
                away_streams: HashMap::new(),
                away_caught_up: HashSet::new(),
                task,
            });
        }
        !same
    };
    if fresh {
        tracing::info!("DM sync is on");
    }
    refresh(handle);
    if fresh {
        outbox::wake_all_dms(&app);
    }
}

pub(crate) fn refresh(handle: &AppHandle) {
    let app = handle.state::<App>();
    let Some(identity) = app.identity() else { return };
    let wanted = wanted(&app);
    let away = away_wanted(&app, &identity, &wanted);
    let epochs = sync_streams::recent_dm_epochs(Utc::now(), EPOCHS);
    let mut guard = LIVE.lock_or_recover();
    let Some(live) = guard.as_mut() else { return };
    if live.me_key != identity.public() {
        return;
    }
    let keep: HashSet<UserId> = wanted.iter().map(|(user, _)| *user).collect();
    let gone: Vec<UserId> = live.partners.keys().filter(|user| !keep.contains(user)).copied().collect();
    for user in gone {
        if let Some(partner) = live.partners.remove(&user) {
            for stream in partner.streams.into_values() {
                live.pool.unfollow(stream);
                live.owners.remove(&stream);
            }
        }
    }
    for (user, key) in wanted {
        if live.partners.get(&user).is_some_and(|p| p.key != key) {
            if let Some(old) = live.partners.remove(&user) {
                for stream in old.streams.into_values() {
                    live.pool.unfollow(stream);
                    live.owners.remove(&stream);
                }
            }
        }
        let partner = live.partners.entry(user).or_insert_with(|| Partner { key, streams: BTreeMap::new() });
        let stale: Vec<String> = partner.streams.keys().filter(|epoch| !epochs.contains(epoch)).cloned().collect();
        for epoch in stale {
            if let Some(stream) = partner.streams.remove(&epoch) {
                live.pool.unfollow(stream);
                live.owners.remove(&stream);
            }
        }
        let mut group = Vec::new();
        for epoch in &epochs {
            if partner.streams.contains_key(epoch) {
                continue;
            }
            let Some(keys) = sync_streams::dm_stream(&identity, &key, epoch) else { continue };
            partner.streams.insert(epoch.clone(), keys.id);
            live.owners.insert(keys.id, user);
            let after = account_log::dm_cursor(&app, &keys.id.to_hex());
            group.push((keys, after));
        }
        live.pool.follow_together(group);
    }
    follow_away(&app, live, away.streams);
    drop(guard);
    away_homes::look_up(handle, away.unknown);
    tend_inboxes(handle);
    push_wake::register(handle);
}

#[derive(Default)]
struct AwayWanted {
    streams: HashMap<StreamId, (UserId, String, StreamKeys)>,
    unknown: Vec<String>,
}

fn away_wanted(app: &App, identity: &commy_core::Identity, wanted: &[(UserId, IdentityKey)]) -> AwayWanted {
    let epochs = sync_streams::recent_dm_epochs(Utc::now(), EPOCHS);
    let mut away = AwayWanted::default();
    for (user, key) in wanted {
        let Some(home) = partner_profile(app, *user).and_then(|profile| away_homes::away_home(app, &profile)) else { continue };
        let Some(url) = away_homes::known_sync(&home) else {
            away.unknown.push(home);
            continue;
        };
        for epoch in &epochs {
            if let Some(keys) = sync_streams::dm_stream(identity, key, epoch) {
                away.streams.insert(keys.id, (*user, url.clone(), keys));
            }
        }
    }
    away
}

fn follow_away(app: &App, live: &mut Live, wanted: HashMap<StreamId, (UserId, String, StreamKeys)>) {
    let stale: Vec<StreamId> = live
        .away_streams
        .iter()
        .filter(|(stream, (user, url))| wanted.get(stream).is_none_or(|(want_user, want_url, _)| want_user != user || want_url != url))
        .map(|(stream, _)| *stream)
        .collect();
    for stream in stale {
        live.away_streams.remove(&stream);
        live.away_caught_up.remove(&stream);
        live.away.unfollow(stream);
    }
    let mut by_url: BTreeMap<String, Vec<(StreamKeys, u64)>> = BTreeMap::new();
    for (stream, (user, url, keys)) in wanted {
        if live.away_streams.contains_key(&stream) {
            continue;
        }
        let after = account_log::dm_cursor(app, &away_cursor(stream, &url));
        live.away_streams.insert(stream, (user, url.clone()));
        by_url.entry(url).or_default().push((keys, after));
    }
    for (url, group) in by_url {
        tracing::info!("reading {} DM streams kept on another home", group.len());
        live.away.follow_together_tail_at(&url, group, None);
    }
}

fn away_cursor(stream: StreamId, url: &str) -> String {
    commy_view::away::cursor_key(&stream.to_hex(), url)
}

fn partner_sync(app: &App, partner: UserId) -> Option<String> {
    let home = partner_profile(app, partner).and_then(|profile| away_homes::away_home(app, &profile))?;
    away_homes::known_sync(&home)
}

fn own_inboxes(app: &App, me: UserId) -> Vec<(Option<ServerId>, StreamKeys)> {
    let personal = app.identity().map(|identity| {
        let epochs = sync_streams::recent_dm_epochs(Utc::now(), EPOCHS);
        inbox::personal_inboxes(me, &identity.public(), &epochs)
    });
    let holder = crate::channel_lock::holder_of(app);
    let sids: Vec<ServerId> = app.servers.lock_or_recover().keys().copied().collect();
    let settled: Vec<ServerId> = sids.into_iter().filter(|sid| !app.awaits_snapshot(*sid)).collect();
    let now = now_ms();
    let servers = app.servers.lock_or_recover();
    settled
        .into_iter()
        .filter_map(|sid| {
            let state = servers.get(&sid)?;
            let ring = holder.community_ring(state, None)?;
            Some(inbox::inboxes(state, me, &ring, now).into_iter().map(move |keys| (Some(sid), keys)))
        })
        .flatten()
        .chain(personal.into_iter().flatten().map(|keys| (None, keys)))
        .collect()
}

pub fn tend(handle: &AppHandle) {
    if LIVE.lock_or_recover().is_none() {
        start(handle);
    } else {
        tend_inboxes(handle);
    }
}

pub fn tend_inboxes(handle: &AppHandle) {
    let app = handle.state::<App>();
    let Some(me) = app.profile.lock_or_recover().as_ref().map(|p| p.id) else { return };
    let mine = own_inboxes(&app, me);
    let changed = {
        let mut guard = LIVE.lock_or_recover();
        let Some(live) = guard.as_mut().filter(|l| l.me == me) else { return };
        let keep: HashSet<StreamId> = mine.iter().map(|(_, keys)| keys.id).collect();
        let gone: Vec<StreamId> = live.inboxes.keys().filter(|stream| !keep.contains(stream)).copied().collect();
        for stream in &gone {
            live.inboxes.remove(stream);
            live.pool.unfollow(*stream);
        }
        let mut group = Vec::new();
        for (server, keys) in mine {
            if live.inboxes.contains_key(&keys.id) {
                continue;
            }
            group.push((keys.clone(), account_log::dm_cursor(&app, &keys.id.to_hex())));
            live.inboxes.insert(keys.id, (server, keys));
        }
        let changed = !gone.is_empty() || !group.is_empty();
        live.pool.follow_together(group);
        if changed {
            tracing::info!("reading {} first-message inboxes", live.inboxes.len());
        }
        changed
    };
    if changed {
        push_wake::register_again();
    }
}


#[derive(Debug, PartialEq)]
pub enum Route {
    Receive,
    Ring,
    Settled,
    Mine,
    Skip,
}

pub fn route(entry: &DmEntry, here: Option<&str>, at: i64, now: i64) -> Route {
    match entry {
        DmEntry::FromPartner(envelope) if syncable(&envelope.message) => Route::Receive,
        DmEntry::FromPartner(envelope) if passing(&envelope.message) && call_fresh(at, now) => Route::Ring,
        DmEntry::FromMe(envelope) if is_call(&envelope.message) => {
            if !commy_view::dms::written_on(&envelope.message, here) && call_fresh(at, now) {
                Route::Settled
            } else {
                Route::Skip
            }
        }
        DmEntry::FromMe(envelope) if syncable(&envelope.message) => Route::Mine,
        _ => Route::Skip,
    }
}


fn inbox_to(app: &App, me: UserId, to: UserId) -> Option<StreamKeys> {
    let holder = crate::channel_lock::holder_of(app);
    let mut sids: Vec<ServerId> = app.servers.lock_or_recover().keys().copied().collect();
    sids.sort();
    let settled: Vec<ServerId> = sids.into_iter().filter(|sid| !app.awaits_snapshot(*sid)).collect();
    let servers = app.servers.lock_or_recover();
    settled.into_iter().find_map(|sid| {
        let state = servers.get(&sid)?;
        let ring = holder.community_ring(state, None)?;
        inbox::writable(state, me, to, &ring)
    })
}

pub async fn post_inbox(app: &App, to: UserId, envelope: &DmEnvelope) -> bool {
    if sync_link::url().is_none() {
        return false;
    }
    let passes = passing(&envelope.message);
    if (passes || syncable(&envelope.message)) && partner_follows(app, to) {
        return passes && account_log::is_live(app) && append_pairwise(app, to, envelope).await;
    }
    let Some(me) = app.profile.lock_or_recover().as_ref().map(|p| p.id) else { return false };
    let Some(key) = partner_key(app, to) else {
        tracing::info!("no key to seal a first message for {to} with");
        return false;
    };
    let keys = inbox_to(app, me, to).unwrap_or_else(|| inbox::personal_inbox_now(to, &key, Utc::now()));
    let Some(plain) = inbox::seal(&key, envelope) else { return false };
    let stream = keys.id;
    let away = match partner_profile(app, to).and_then(|profile| away_homes::away_home(app, &profile)) {
        Some(home) => match away_homes::sync_now(app, &home).await {
            Some(url) => Some(url),
            None => {
                tracing::info!("could not reach the home of {to} to leave a first message");
                return false;
            }
        },
        None => None,
    };
    let wake = wakes(&envelope.message).then(|| (keys.wake_tag(&to.to_string(), push_wake::today()), wake_preview(app, &keys, &envelope.message)));
    let (client, borrowed) = {
        let mut guard = LIVE.lock_or_recover();
        let Some(live) = guard.as_mut().filter(|l| l.me == me) else { return false };
        let pool = if away.is_some() { &mut live.away } else { &mut live.pool };
        let borrowed = !pool.follows(stream);
        if borrowed {
            match &away {
                Some(url) => pool.follow_together_tail_at(url, vec![(keys, 0)], Some(0)),
                None => pool.follow_together_tail(vec![(keys, 0)], Some(0)),
            }
        }
        (pool.client_for(stream), borrowed)
    };
    let sent = tokio::time::timeout(HOLD_WAIT, sync_link::append(client, stream, plain)).await;
    if borrowed {
        if let Some(live) = LIVE.lock_or_recover().as_mut() {
            match away {
                Some(_) => live.away.unfollow(stream),
                None => live.pool.unfollow(stream),
            }
        }
    }
    match sent {
        Ok(Ok(_)) => {
            tracing::info!("left a first message for {to} on the sync service");
            if let Some((tag, preview)) = wake {
                ring_phones(app, &envelope.message, tag, preview, partner_sync(app, to));
            }
            true
        }
        Ok(Err(e)) => {
            tracing::info!("could not leave a first message on the sync service: {e}");
            false
        }
        Err(_) => {
            tracing::debug!("the sync service did not take a first message in time");
            false
        }
    }
}

fn known_sender(app: &App, server: Option<ServerId>, envelope: &DmEnvelope) -> bool {
    let Some(server) = server else { return true };
    let from = envelope.message.from;
    app.servers.lock_or_recover().get(&server).is_some_and(|state| {
        state.server.is_current_member(from) && state.profiles.get(&from).and_then(|p| p.identity) == envelope.sender.identity
    })
}

fn on_inbox(handle: &AppHandle, server: Option<ServerId>, plain: &[u8], at: i64, live_now: bool) {
    let app = handle.state::<App>();
    let Some(identity) = app.identity() else { return };
    let Some(envelope) = inbox::open(&identity, plain) else {
        tracing::info!("an entry in our first-message inbox did not open");
        return;
    };
    let from = envelope.message.from;
    if !known_sender(&app, server, &envelope) {
        tracing::info!("dropped a first message from {from}, who is not a member we know with that key");
        return;
    }
    if passing(&envelope.message) && !call_fresh(at, now_ms()) {
        return;
    }
    let Some(node) = data_encoding::HEXLOWER_PERMISSIVE
        .decode(envelope.message.from_node.trim().as_bytes())
        .ok()
        .and_then(|node| <[u8; 32]>::try_from(node).ok())
    else {
        tracing::info!("dropped a first message from {from} without a device");
        return;
    };
    let ring = should_ring(live_now, envelope.message.created_at, Utc::now());
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        crate::take_dm(&handle, IncomingDm { envelope, node }, ring).await;
    });
}

fn passing(message: &DmMessage) -> bool {
    is_call(message) || is_nudge(message)
}

fn wakes(message: &DmMessage) -> bool {
    rings(message) || is_nudge(message)
}

fn ring_phones(app: &App, message: &DmMessage, tag: [u8; 32], preview: Option<String>, home_sync: Option<String>) {
    let http = app.http.clone();
    tauri::async_runtime::spawn(push_wake::ring_at(http, home_sync, vec![tag], preview, is_nudge(message)));
}

fn wake_preview(app: &App, keys: &StreamKeys, message: &DmMessage) -> Option<String> {
    push_wake::preview(app, keys, &message.from.to_string(), None, &commy_view::dms::wake_text(message), is_call(message), commy_view::dms::call_ended(message))
}

fn wake_streams(app: &App, days: &[i64]) -> Option<(String, Vec<(StreamKeys, i64)>)> {
    let (Some(identity), Some(me)) = (app.identity(), app.profile.lock_or_recover().as_ref().map(|p| p.id)) else { return None };
    let partners: Vec<IdentityKey> = match LIVE.lock_or_recover().as_ref() {
        Some(live) if live.me_key == identity.public() => live.partners.values().map(|p| p.key).collect(),
        _ => return None,
    };
    let inboxes: Vec<StreamKeys> =
        LIVE.lock_or_recover().as_ref().map(|live| live.inboxes.values().map(|(_, keys)| keys.clone()).collect()).unwrap_or_default();
    let mut streams: Vec<(StreamKeys, i64)> = days.iter().flat_map(|day| inboxes.iter().map(move |keys| (keys.clone(), *day))).collect();
    for day in days {
        let Some(start) = DateTime::from_timestamp_millis(day * DAY_MS) else { continue };
        let epoch = sync_streams::dm_epoch(start);
        for key in &partners {
            if let Some(keys) = sync_streams::dm_stream(&identity, key, &epoch) {
                streams.push((keys, *day));
            }
        }
    }
    Some((me.to_string(), streams))
}

pub fn wake_tags(app: &App, days: &[i64]) -> Option<Vec<([u8; 32], i64)>> {
    let (who, streams) = wake_streams(app, days)?;
    Some(streams.iter().map(|(keys, day)| (keys.wake_tag(&who, *day), *day)).collect())
}

pub fn signer_keys(app: &App) -> BTreeMap<String, String> {
    let mut users: Vec<UserId> = app.dms.lock_or_recover().contacts.keys().copied().collect();
    users.extend(LIVE.lock_or_recover().as_ref().map(|live| live.partners.keys().copied().collect::<Vec<_>>()).unwrap_or_default());
    users.into_iter().filter_map(|user| Some((user.to_string(), BASE64.encode(&partner_key(app, user)?.0)))).collect()
}

pub fn preview_keys(app: &App, days: &[i64]) -> Vec<[u8; 32]> {
    wake_streams(app, days).map(|(_, streams)| streams.iter().map(|(keys, day)| keys.preview_key(*day)).collect()).unwrap_or_default()
}


type Events = tokio::sync::mpsc::UnboundedReceiver<SyncEvent>;

async fn run(handle: AppHandle, mut events: Events, mut away_events: Events) {
    let mut tick = tokio::time::interval(REFRESH_EVERY);
    tick.tick().await;
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Some(event) => on_event(&handle, event),
                None => break,
            },
            Some(event) = away_events.recv() => on_away_event(&handle, event),
            _ = tick.tick() => refresh(&handle),
        }
    }
}

fn on_away_event(handle: &AppHandle, event: SyncEvent) {
    let app = handle.state::<App>();
    match event {
        SyncEvent::Entry { stream, seq, at, plain } => {
            let found = {
                let live = LIVE.lock_or_recover();
                live.as_ref().and_then(|l| {
                    let (partner, url) = l.away_streams.get(&stream)?.clone();
                    let key = partner_key(&app, partner)?;
                    Some((l.me, l.me_key, partner, key, url, l.away_caught_up.contains(&stream)))
                })
            };
            let Some((me, me_key, partner, key, url, live_now)) = found else { return };
            let entry = sync_streams::decode_dm(&plain, me, &me_key, partner, &key);
            match entry.as_ref().map(|entry| away_route(entry, at, now_ms())) {
                Some(Route::Receive) => receive(handle, entry.unwrap().into_envelope(), live_now),
                Some(Route::Ring) => ring(handle, entry.unwrap().into_envelope()),
                _ => {}
            }
            account_log::set_dm_cursor(&app, away_cursor(stream, &url), seq, live_now);
        }
        SyncEvent::CaughtUp { stream, seq } => {
            let url = LIVE.lock_or_recover().as_mut().and_then(|live| {
                let url = live.away_streams.get(&stream)?.1.clone();
                live.away_caught_up.insert(stream);
                Some(url)
            });
            if let Some(url) = url {
                account_log::set_dm_cursor(&app, away_cursor(stream, &url), seq, true);
            }
        }
        SyncEvent::Refused { stream, code } => tracing::debug!("another home refused a DM stream {stream}: {code:?}"),
        _ => {}
    }
}

pub fn away_route(entry: &DmEntry, at: i64, now: i64) -> Route {
    match entry {
        DmEntry::FromPartner(_) => route(entry, None, at, now),
        DmEntry::FromMe(_) => Route::Skip,
    }
}

fn on_event(handle: &AppHandle, event: SyncEvent) {
    let app = handle.state::<App>();
    match event {
        SyncEvent::Entry { stream, seq, at, plain } => {
            let inbox = {
                let live = LIVE.lock_or_recover();
                live.as_ref().and_then(|l| l.inboxes.get(&stream).map(|(server, _)| (*server, l.caught_up.contains(&stream))))
            };
            if let Some((server, live_now)) = inbox {
                on_inbox(handle, server, &plain, at, live_now);
                account_log::set_dm_cursor(&app, stream.to_hex(), seq, live_now);
                return;
            }
            let found = {
                let live = LIVE.lock_or_recover();
                live.as_ref().and_then(|l| {
                    let partner = *l.owners.get(&stream)?;
                    let key = l.partners.get(&partner)?.key;
                    Some((l.me, l.me_key, partner, key, l.caught_up.contains(&stream)))
                })
            };
            let Some((me, me_key, partner, key, live_now)) = found else { return };
            let Some(entry) = sync_streams::decode_dm(&plain, me, &me_key, partner, &key) else {
                account_log::set_dm_cursor(&app, stream.to_hex(), seq, live_now);
                return;
            };
            let here = app.node_hex.lock_or_recover().clone();
            match route(&entry, here.as_deref(), at, now_ms()) {
                Route::Receive => receive(handle, entry.into_envelope(), live_now),
                Route::Ring => ring(handle, entry.into_envelope()),
                Route::Settled => {
                    if let DmAction::Call { call, signal } = entry.into_envelope().message.action {
                        crate::calls::settled_elsewhere(&app, handle, partner, call, signal);
                    }
                }
                Route::Mine => take_mine(handle, partner, entry.into_envelope().message),
                Route::Skip => {}
            }
            account_log::set_dm_cursor(&app, stream.to_hex(), seq, live_now);
        }
        SyncEvent::CaughtUp { stream, seq } => {
            let ours = match LIVE.lock_or_recover().as_mut() {
                Some(live) if live.owners.contains_key(&stream) || live.inboxes.contains_key(&stream) => {
                    live.caught_up.insert(stream);
                    true
                }
                _ => false,
            };
            if ours {
                account_log::set_dm_cursor(&app, stream.to_hex(), seq, true);
            }
        }
        SyncEvent::Refused { stream, code } => tracing::debug!("the sync service refused a DM stream {stream}: {code:?}"),
        _ => {}
    }
}

pub(crate) struct Quiet(pub(crate) AppHandle);

impl AppAccess for Quiet {
    fn app(&self) -> &App {
        self.0.app()
    }

    fn emit(&self, event: &str, payload: serde_json::Value) {
        if event != "dm-received" {
            self.0.emit(event, payload);
        }
    }

    fn save_later(&self) {
        self.0.save_later();
    }

    fn catch_up(&self, server: commy_core::ServerId, peer: [u8; 32]) {
        self.0.catch_up(server, peer);
    }
}


fn receive(handle: &AppHandle, envelope: DmEnvelope, live_now: bool) {
    let Ok(node) = data_encoding::HEXLOWER_PERMISSIVE.decode(envelope.message.from_node.trim().as_bytes()) else { return };
    let Ok(node) = <[u8; 32]>::try_from(node) else { return };
    let ring = should_ring(live_now, envelope.message.created_at, Utc::now());
    let dm = IncomingDm { envelope, node };
    if ring {
        outbox::receive_dm(handle, dm);
    } else {
        outbox::receive_dm(&Quiet(handle.clone()), dm);
    }
}

fn ring(handle: &AppHandle, envelope: DmEnvelope) {
    let Ok(node) = data_encoding::HEXLOWER_PERMISSIVE.decode(envelope.message.from_node.trim().as_bytes()) else { return };
    let Ok(node) = <[u8; 32]>::try_from(node) else { return };
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        crate::take_dm(&handle, IncomingDm { envelope, node }, true).await;
    });
}

fn take_mine(handle: &AppHandle, partner: UserId, message: DmMessage) {
    let app = handle.state::<App>();
    let applied = app.dms.lock_or_recover().apply(partner, message);
    if applied {
        app.persist_dms();
        handle.emit("dm-changed", partner.to_string().into());
    }
}

pub fn partner_follows(app: &App, partner: UserId) -> bool {
    let queued = app.outbox.lock_or_recover().pending_dms(partner);
    commy_view::dms::follows(&app.dms.lock_or_recover(), partner, &queued, now_ms())
}

pub fn waiting_dms(app: &App, partner: UserId) -> HashSet<MessageId> {
    let follows = partner_follows(app, partner);
    let outbox = app.outbox.lock_or_recover();
    if follows {
        outbox.unstored_dms(partner)
    } else {
        outbox.pending_dms(partner)
    }
}

pub async fn publish(app: &App, partner: UserId, envelope: &DmEnvelope) -> bool {
    if !syncable(&envelope.message) || sync_link::url().is_none() || !account_log::is_live(app) {
        return false;
    }
    if let Some(profile) = partner_profile(app, partner) {
        account_log::note_partner(app, profile, true);
    }
    append_pairwise(app, partner, envelope).await
}

async fn append_pairwise(app: &App, partner: UserId, envelope: &DmEnvelope) -> bool {
    let (Some(key), Some(identity)) = (partner_key(app, partner), app.identity()) else { return false };
    let epoch = sync_streams::dm_epoch(Utc::now());
    let Some(keys) = sync_streams::dm_stream(&identity, &key, &epoch) else { return false };
    let stream = keys.id;
    let wake = wakes(&envelope.message).then(|| (keys.wake_tag(&partner.to_string(), push_wake::today()), wake_preview(app, &keys, &envelope.message)));
    let mut new_partner = false;
    let client = {
        let mut guard = LIVE.lock_or_recover();
        let Some(live) = guard.as_mut().filter(|l| l.me_key == identity.public()) else { return false };
        if !live.pool.follows(stream) {
            new_partner = true;
            let after = account_log::dm_cursor(app, &stream.to_hex());
            live.partners.entry(partner).or_insert_with(|| Partner { key, streams: BTreeMap::new() }).streams.insert(epoch, stream);
            live.owners.insert(stream, partner);
            live.pool.follow(keys, after);
        }
        live.pool.client_for(stream)
    };
    if new_partner {
        push_wake::register_again();
    }
    let plain = sync_streams::encode_dm(envelope);
    match tokio::time::timeout(HOLD_WAIT, sync_link::append(client, stream, plain)).await {
        Ok(Ok(_)) => {
            if let Some((tag, preview)) = wake {
                ring_phones(app, &envelope.message, tag, preview, partner_sync(app, partner));
            }
            true
        }
        Ok(Err(e)) => {
            tracing::debug!("could not add a DM to the sync service: {e}");
            false
        }
        Err(_) => {
            tracing::debug!("the sync service did not take a DM in time");
            false
        }
    }
}

fn talks_with(app: &App, partner: UserId) -> bool {
    let me = app.profile.lock_or_recover().as_ref().map(|p| p.id);
    let sent = me.is_some_and(|me| app.dms.lock_or_recover().has_sent_to(me, partner));
    let safety = app.safety.lock_or_recover();
    !safety.is_blocked(partner) && (sent || safety.accepted.contains(&partner) || safety.friends.contains(&partner))
}

pub fn noticed(handle: &AppHandle, partner: UserId) {
    let app = handle.state::<App>();
    if sync_link::url().is_none() || last_message_ms(&app, partner).is_none() {
        return;
    }
    if let Some(profile) = partner_profile(&app, partner) {
        account_log::note_partner(&app, profile, talks_with(&app, partner));
    }
    ensure(handle);
}

pub fn adopt_partner(handle: &AppHandle, profile: UserProfile, accepted: bool) {
    let app = handle.state::<App>();
    let me = app.profile.lock_or_recover().as_ref().map(|p| p.id);
    if Some(profile.id) == me {
        return;
    }
    let partner = profile.id;
    let changed = {
        let mut dms = app.dms.lock_or_recover();
        !dms.contacts.contains_key(&partner) && dms.remember_contact(profile)
    };
    if changed {
        app.persist_dms();
    }
    let accepted = {
        let mut safety = app.safety.lock_or_recover();
        accepted && !safety.is_blocked(partner) && safety.accepted.insert(partner)
    };
    if accepted {
        app.persist_safety();
    }
    ensure(handle);
}

pub fn seed(handle: &AppHandle) {
    let app = handle.state::<App>();
    let now = now_ms();
    let partners = app.dms.lock_or_recover().partners();
    for user in partners {
        if last_message_ms(&app, user).is_some_and(|at| now - at < RECENT_MS) {
            if let Some(profile) = partner_profile(&app, user) {
                account_log::note_partner(&app, profile, talks_with(&app, user));
            }
        }
    }
    ensure(handle);
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::dm::CallSignal;

    #[test]
    fn a_dm_the_service_holds_counts_as_sent_once_the_partner_follows_the_conversation() {
        let app = App::default();
        let (me, partner) = (UserId::new(), UserId::new());
        let ours = DmMessage::new(me, partner, "are you around?");
        let id = ours.id;
        app.dms.lock().unwrap().apply(partner, ours);
        app.outbox.lock().unwrap().add_dm(partner, id);
        assert!(!partner_follows(&app, partner), "a first message can reach a stranger only directly");
        assert!(app.outbox.lock().unwrap().dm_stored(partner, id));
        assert_eq!(waiting_dms(&app, partner), HashSet::from([id]));

        let mut old = DmMessage::new(partner, me, "long ago");
        old.created_at = Utc::now() - chrono::Duration::milliseconds(RECENT_MS + 1000);
        app.dms.lock().unwrap().apply(partner, old);
        assert_eq!(waiting_dms(&app, partner), HashSet::from([id]), "they stopped following a quiet conversation");

        app.dms.lock().unwrap().apply(partner, DmMessage::new(partner, me, "hi"));
        assert!(partner_follows(&app, partner));
        assert!(waiting_dms(&app, partner).is_empty());

        let next = DmMessage::new(me, partner, "still there?");
        let next_id = next.id;
        app.dms.lock().unwrap().apply(partner, next);
        app.outbox.lock().unwrap().add_dm(partner, next_id);
        assert_eq!(waiting_dms(&app, partner), HashSet::from([next_id]), "not yet held by the service");
        assert_eq!(app.outbox.lock().unwrap().dm_queue(partner), vec![id, next_id]);
    }

    #[test]
    fn our_own_entries_reach_our_other_devices_and_only_their_calls_settle_ours() {
        let (me, partner) = (UserId::new(), UserId::new());
        let profile = UserProfile::new("me");
        let channel = commy_core::ChannelId::new();
        let (here, other) = ("aa".repeat(32), "bb".repeat(32));
        let mine = |action: DmAction, node: &str| {
            let mut message = DmMessage::acting(me, partner, action, "");
            message.from_node = node.to_string();
            DmEntry::FromMe(DmEnvelope::new(message, profile.clone()))
        };
        let now = 1_000_000_000_000;
        let posted = DmEntry::FromMe(DmEnvelope::new(DmMessage::new(me, partner, "sent from my phone"), profile.clone()));
        assert_eq!(route(&posted, Some(&here), now, now), Route::Mine, "a fresh message of ours still reaches this device");
        let reacted = mine(DmAction::React { target: commy_core::MessageId::new(), emoji: "👍".into(), active: true }, &other);
        assert_eq!(route(&reacted, Some(&here), now, now), Route::Mine);
        let accept = DmAction::Call { call: channel, signal: CallSignal::Accept };
        assert_eq!(route(&mine(accept.clone(), &other), Some(&here), now, now), Route::Settled);
        assert_eq!(route(&mine(accept.clone(), &here), Some(&here), now, now), Route::Skip, "our own answer never ends our call");
        assert_eq!(route(&mine(accept, &other), Some(&here), now - commy_view::dms::CALL_FRESH_MS - 1, now), Route::Skip);
    }

    #[test]
    fn on_a_partners_home_only_their_own_entries_count() {
        let (me, partner) = (UserId::new(), UserId::new());
        let now = 1_000_000_000_000;
        let theirs = DmEntry::FromPartner(DmEnvelope::new(DmMessage::new(partner, me, "hello from my home"), UserProfile::new("partner")));
        assert_eq!(away_route(&theirs, now, now), Route::Receive);
        let ours = DmEntry::FromMe(DmEnvelope::new(DmMessage::new(me, partner, "planted there"), UserProfile::new("me")));
        assert_eq!(away_route(&ours, now, now), Route::Skip, "our side lives on our own home, never theirs");
        let mut ring = DmMessage::acting(partner, me, DmAction::Call { call: commy_core::ChannelId::new(), signal: CallSignal::Ring }, "");
        ring.from_node = "cc".repeat(32);
        let ringing = DmEntry::FromPartner(DmEnvelope::new(ring, UserProfile::new("partner")));
        assert_eq!(away_route(&ringing, now, now), Route::Ring);
        assert_eq!(away_route(&ringing, now - commy_view::dms::CALL_FRESH_MS - 1, now), Route::Skip);
    }

    #[test]
    fn our_delivered_messages_show_the_partner_follows() {
        let app = App::default();
        let (me, partner) = (UserId::new(), UserId::new());
        let delivered = DmMessage::new(me, partner, "got this one");
        app.dms.lock().unwrap().apply(partner, delivered);
        assert!(partner_follows(&app, partner));
    }

}
