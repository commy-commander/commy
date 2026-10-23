use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use commy_core::dm::{CallSignal, DmAction, DmMessage};
use commy_core::{IdentityKey, MessageId, ServerId, UserId, UserProfile};
use commy_net::inbox;
use commy_net::node::IncomingDm;
use commy_net::sync_streams::{self, DmEntry};
use commy_net::DmEnvelope;
use commy_sync::wake::DAY_MS;
use commy_sync::{StreamId, StreamKeys, SyncEvent};
use tauri::{AppHandle, Manager};

use crate::catchup::AppAccess;
use crate::state::App;
use crate::sync_link::{self, Pool};
use crate::{account_log, outbox, push_wake, safety};

const EPOCHS: u32 = 3;
const MAX_PARTNERS: usize = 40;
const RECENT_MS: i64 = 90 * 24 * 60 * 60 * 1000;
const QUIET_AFTER_SECS: i64 = 10 * 60;
const REFRESH_EVERY: Duration = Duration::from_secs(60 * 60);
const HOLD_WAIT: Duration = Duration::from_secs(8);
const CALL_FRESH_MS: i64 = 60 * 1000;

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
    inboxes: HashMap<StreamId, (ServerId, StreamKeys)>,
    caught_up: HashSet<StreamId>,
    task: tauri::async_runtime::JoinHandle<()>,
}

static LIVE: Mutex<Option<Live>> = Mutex::new(None);

pub fn nudge() {
    if let Some(live) = LIVE.lock().unwrap().as_ref() {
        live.pool.nudge();
    }
}

pub fn syncable(message: &DmMessage) -> bool {
    matches!(message.action, DmAction::Post | DmAction::Edit { .. } | DmAction::Delete { .. } | DmAction::React { .. })
        && !message.content.starts_with(safety::REPORT_PREFIX)
        && safety::FriendSignal::decode(&message.content).is_none()
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn partner_key(app: &App, user: UserId) -> Option<IdentityKey> {
    let pinned = app.dms.lock().unwrap().pinned_key(user);
    pinned.or_else(|| outbox::member_profile(&app.servers.lock().unwrap(), user).and_then(|p| p.identity))
}

fn partner_profile(app: &App, user: UserId) -> Option<UserProfile> {
    let known = app.dms.lock().unwrap().contacts.get(&user).cloned();
    known.or_else(|| outbox::member_profile(&app.servers.lock().unwrap(), user)).filter(|p| p.identity.is_some())
}

fn last_message_ms(app: &App, user: UserId) -> Option<i64> {
    app.dms.lock().unwrap().conversation(user).last().map(|m| m.created_at.timestamp_millis())
}

pub fn pick_partners(noted: &BTreeMap<UserId, i64>, local: &[(UserId, i64)], now: i64) -> Vec<UserId> {
    let mut latest: BTreeMap<UserId, i64> = BTreeMap::new();
    for (user, at) in noted.iter().map(|(u, a)| (*u, *a)).chain(local.iter().copied()) {
        if now - at < RECENT_MS {
            let seen = latest.entry(user).or_insert(at);
            *seen = (*seen).max(at);
        }
    }
    let mut ranked: Vec<(UserId, i64)> = latest.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked.into_iter().take(MAX_PARTNERS).map(|(user, _)| user).collect()
}

fn wanted(app: &App) -> Vec<(UserId, IdentityKey)> {
    let partners = app.dms.lock().unwrap().partners();
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
    if let Some(old) = LIVE.lock().unwrap().take() {
        old.task.abort();
    }
}

fn start(handle: &AppHandle) {
    let app = handle.state::<App>();
    let Some(url) = sync_link::url() else {
        stop();
        return;
    };
    if !account_log::is_live(&app) {
        return;
    }
    let (Some(dir), Some(me), Some(identity)) =
        (app.data_dir().cloned(), app.profile.lock().unwrap().as_ref().map(|p| p.id), app.identity())
    else {
        return;
    };
    let me_key = identity.public();
    let fresh = {
        let mut live = LIVE.lock().unwrap();
        let same = live.as_ref().is_some_and(|l| l.url == url && l.dir == dir && l.me == me && l.me_key == me_key);
        if !same {
            if let Some(old) = live.take() {
                old.task.abort();
            }
            let (pool, events) = Pool::new(&url);
            let task = tauri::async_runtime::spawn(run(handle.clone(), events));
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
                task,
            });
        }
        !same
    };
    refresh(handle);
    if fresh {
        outbox::wake_all_dms(&app);
    }
}

fn refresh(handle: &AppHandle) {
    let app = handle.state::<App>();
    let Some(identity) = app.identity() else { return };
    let wanted = wanted(&app);
    let epochs = sync_streams::recent_dm_epochs(Utc::now(), EPOCHS);
    let mut guard = LIVE.lock().unwrap();
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
    drop(guard);
    tend_inboxes(handle);
    push_wake::register(handle);
}

fn own_inboxes(app: &App, me: UserId) -> Vec<(ServerId, StreamKeys)> {
    let holder = crate::channel_lock::holder_of(app);
    let sids: Vec<ServerId> = app.servers.lock().unwrap().keys().copied().collect();
    let settled: Vec<ServerId> = sids.into_iter().filter(|sid| !app.awaits_snapshot(*sid)).collect();
    let now = now_ms();
    let servers = app.servers.lock().unwrap();
    settled
        .into_iter()
        .filter_map(|sid| {
            let state = servers.get(&sid)?;
            let ring = holder.community_ring(state, None)?;
            Some(inbox::inboxes(state, me, &ring, now).into_iter().map(move |keys| (sid, keys)))
        })
        .flatten()
        .collect()
}

pub fn tend(handle: &AppHandle) {
    if LIVE.lock().unwrap().is_none() {
        start(handle);
    } else {
        tend_inboxes(handle);
    }
}

pub fn tend_inboxes(handle: &AppHandle) {
    let app = handle.state::<App>();
    let Some(me) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { return };
    let mine = own_inboxes(&app, me);
    let changed = {
        let mut guard = LIVE.lock().unwrap();
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
        changed
    };
    if changed {
        push_wake::register_again();
    }
}

pub fn is_call(message: &DmMessage) -> bool {
    matches!(message.action, DmAction::Call { .. })
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
        DmEntry::FromPartner(envelope) if is_call(&envelope.message) && call_fresh(at, now) => Route::Ring,
        DmEntry::FromMe(envelope) if is_call(&envelope.message) => {
            let written_here = here.is_some_and(|node| node.eq_ignore_ascii_case(envelope.message.from_node.trim()));
            if !written_here && call_fresh(at, now) {
                Route::Settled
            } else {
                Route::Skip
            }
        }
        DmEntry::FromMe(envelope) if syncable(&envelope.message) => Route::Mine,
        _ => Route::Skip,
    }
}

pub fn call_fresh(at: i64, now: i64) -> bool {
    (now - at).abs() < CALL_FRESH_MS
}

fn inbox_to(app: &App, me: UserId, to: UserId) -> Option<StreamKeys> {
    let holder = crate::channel_lock::holder_of(app);
    let mut sids: Vec<ServerId> = app.servers.lock().unwrap().keys().copied().collect();
    sids.sort();
    let settled: Vec<ServerId> = sids.into_iter().filter(|sid| !app.awaits_snapshot(*sid)).collect();
    let servers = app.servers.lock().unwrap();
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
    let call = is_call(&envelope.message);
    if (call || syncable(&envelope.message)) && partner_follows(app, to) {
        return call && account_log::is_live(app) && append_pairwise(app, to, envelope).await;
    }
    let Some(me) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { return false };
    let (Some(key), Some(keys)) = (partner_key(app, to), inbox_to(app, me, to)) else { return false };
    let Some(plain) = inbox::seal(&key, envelope) else { return false };
    let stream = keys.id;
    let wake = rings(&envelope.message).then(|| keys.wake_tag(&to.to_string(), push_wake::today()));
    let (client, borrowed) = {
        let mut guard = LIVE.lock().unwrap();
        let Some(live) = guard.as_mut().filter(|l| l.me == me) else { return false };
        let borrowed = !live.pool.follows(stream);
        if borrowed {
            live.pool.follow_together_tail(vec![(keys, 0)], Some(0));
        }
        (live.pool.client_for(stream), borrowed)
    };
    let sent = tokio::time::timeout(HOLD_WAIT, sync_link::append(client, stream, plain)).await;
    if borrowed {
        if let Some(live) = LIVE.lock().unwrap().as_mut() {
            live.pool.unfollow(stream);
        }
    }
    match sent {
        Ok(Ok(_)) => {
            if let Some(tag) = wake {
                let http = app.http.clone();
                tauri::async_runtime::spawn(push_wake::ring(http, vec![tag]));
            }
            true
        }
        Ok(Err(e)) => {
            tracing::debug!("could not leave a first message on the sync service: {e}");
            false
        }
        Err(_) => {
            tracing::debug!("the sync service did not take a first message in time");
            false
        }
    }
}

fn on_inbox(handle: &AppHandle, server: ServerId, plain: &[u8], at: i64, live_now: bool) {
    let app = handle.state::<App>();
    let Some(identity) = app.identity() else { return };
    let Some(envelope) = inbox::open(&identity, plain) else {
        tracing::info!("an entry in our first-message inbox did not open");
        return;
    };
    let from = envelope.message.from;
    let member = app.servers.lock().unwrap().get(&server).is_some_and(|state| {
        state.server.is_current_member(from) && state.profiles.get(&from).and_then(|p| p.identity) == envelope.sender.identity
    });
    if !member {
        tracing::info!("dropped a first message from {from}, who is not a member we know with that key");
        return;
    }
    if is_call(&envelope.message) && !call_fresh(at, now_ms()) {
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

pub fn wake_tags(app: &App, days: &[i64]) -> Option<Vec<([u8; 32], i64)>> {
    let (Some(identity), Some(me)) = (app.identity(), app.profile.lock().unwrap().as_ref().map(|p| p.id)) else { return None };
    let partners: Vec<IdentityKey> = match LIVE.lock().unwrap().as_ref() {
        Some(live) if live.me_key == identity.public() => live.partners.values().map(|p| p.key).collect(),
        _ => return None,
    };
    let inboxes: Vec<StreamKeys> =
        LIVE.lock().unwrap().as_ref().map(|live| live.inboxes.values().map(|(_, keys)| keys.clone()).collect()).unwrap_or_default();
    let who = me.to_string();
    let name = who.as_str();
    let mut tags: Vec<([u8; 32], i64)> =
        days.iter().flat_map(|day| inboxes.iter().map(move |keys| (keys.wake_tag(name, *day), *day))).collect();
    for day in days {
        let Some(start) = DateTime::from_timestamp_millis(day * DAY_MS) else { continue };
        let epoch = sync_streams::dm_epoch(start);
        for key in &partners {
            if let Some(keys) = sync_streams::dm_stream(&identity, key, &epoch) {
                tags.push((keys.wake_tag(&who, *day), *day));
            }
        }
    }
    Some(tags)
}

pub fn rings(message: &DmMessage) -> bool {
    matches!(message.action, DmAction::Post | DmAction::Call { signal: CallSignal::Ring, .. })
}

async fn run(handle: AppHandle, mut events: tokio::sync::mpsc::UnboundedReceiver<SyncEvent>) {
    let mut tick = tokio::time::interval(REFRESH_EVERY);
    tick.tick().await;
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Some(event) => on_event(&handle, event),
                None => break,
            },
            _ = tick.tick() => refresh(&handle),
        }
    }
}

fn on_event(handle: &AppHandle, event: SyncEvent) {
    let app = handle.state::<App>();
    match event {
        SyncEvent::Entry { stream, seq, at, plain } => {
            let inbox = {
                let live = LIVE.lock().unwrap();
                live.as_ref().and_then(|l| l.inboxes.get(&stream).map(|(server, _)| (*server, l.caught_up.contains(&stream))))
            };
            if let Some((server, live_now)) = inbox {
                on_inbox(handle, server, &plain, at, live_now);
                account_log::set_dm_cursor(&app, stream.to_hex(), seq, live_now);
                return;
            }
            let found = {
                let live = LIVE.lock().unwrap();
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
            let here = app.node_hex.lock().unwrap().clone();
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
            let ours = match LIVE.lock().unwrap().as_mut() {
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

pub fn should_ring(live_now: bool, created_at: chrono::DateTime<Utc>, now: chrono::DateTime<Utc>) -> bool {
    live_now && (now - created_at).num_seconds() < QUIET_AFTER_SECS
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
    let applied = app.dms.lock().unwrap().apply(partner, message);
    if applied {
        app.persist_dms();
        handle.emit("dm-changed", partner.to_string().into());
    }
}

pub fn partner_follows(app: &App, partner: UserId) -> bool {
    let since = now_ms() - RECENT_MS;
    let queued = app.outbox.lock().unwrap().pending_dms(partner);
    let dms = app.dms.lock().unwrap();
    dms.conversation(partner)
        .iter()
        .any(|m| m.created_at.timestamp_millis() > since && (m.from == partner || !queued.contains(&m.id)))
}

pub fn waiting_dms(app: &App, partner: UserId) -> HashSet<MessageId> {
    let follows = partner_follows(app, partner);
    let outbox = app.outbox.lock().unwrap();
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
    let wake = rings(&envelope.message).then(|| keys.wake_tag(&partner.to_string(), push_wake::today()));
    let mut new_partner = false;
    let client = {
        let mut guard = LIVE.lock().unwrap();
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
            if let Some(tag) = wake {
                let http = app.http.clone();
                tauri::async_runtime::spawn(push_wake::ring(http, vec![tag]));
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
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let sent = me.is_some_and(|me| app.dms.lock().unwrap().has_sent_to(me, partner));
    let safety = app.safety.lock().unwrap();
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
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    if Some(profile.id) == me {
        return;
    }
    let partner = profile.id;
    let changed = {
        let mut dms = app.dms.lock().unwrap();
        !dms.contacts.contains_key(&partner) && dms.remember_contact(profile)
    };
    if changed {
        app.persist_dms();
    }
    let accepted = {
        let mut safety = app.safety.lock().unwrap();
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
    let partners = app.dms.lock().unwrap().partners();
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

    #[test]
    fn only_conversation_messages_go_to_the_service() {
        let (a, b) = (UserId::new(), UserId::new());
        assert!(syncable(&DmMessage::new(a, b, "hello")));
        let target = commy_core::MessageId::new();
        assert!(syncable(&DmMessage::acting(a, b, DmAction::Edit { target }, "hello!")));
        assert!(syncable(&DmMessage::acting(a, b, DmAction::React { target, emoji: "🔥".into(), active: true }, "")));
        assert!(!syncable(&DmMessage::acting(a, b, DmAction::GroupInvite { ticket: "t".into() }, "")));
        assert!(!syncable(&DmMessage::new(a, b, format!("{}spam", safety::REPORT_PREFIX))));
        let signal = safety::FriendSignal::Request.encode();
        assert!(!syncable(&DmMessage::new(a, b, signal)));
    }

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
    fn a_call_rings_only_while_fresh_and_never_from_history() {
        let (a, b) = (UserId::new(), UserId::new());
        let channel = commy_core::ChannelId::new();
        let ring = DmMessage::acting(a, b, DmAction::Call { call: channel, signal: CallSignal::Ring }, "");
        let hang_up = DmMessage::acting(a, b, DmAction::Call { call: channel, signal: CallSignal::HangUp }, "");
        assert!(is_call(&ring) && is_call(&hang_up));
        assert!(!is_call(&DmMessage::new(a, b, "hello")));
        assert!(!syncable(&ring), "calls never become conversation history");
        assert!(rings(&ring), "a ring wakes the partner's phone");
        assert!(!rings(&hang_up));
        let now = 1_000_000_000_000;
        assert!(call_fresh(now - 5_000, now));
        assert!(call_fresh(now + 5_000, now), "a clock a little ahead still rings");
        assert!(!call_fresh(now - CALL_FRESH_MS - 1, now), "an old ring read back later stays quiet");
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
        assert_eq!(route(&mine(accept, &other), Some(&here), now - CALL_FRESH_MS - 1, now), Route::Skip);
    }

    #[test]
    fn our_delivered_messages_show_the_partner_follows() {
        let app = App::default();
        let (me, partner) = (UserId::new(), UserId::new());
        let delivered = DmMessage::new(me, partner, "got this one");
        app.dms.lock().unwrap().apply(partner, delivered);
        assert!(partner_follows(&app, partner));
    }

    #[test]
    fn the_most_recent_partners_are_followed() {
        let now = 1_000_000_000_000;
        let (old, fresh, local, renoted) = (UserId::new(), UserId::new(), UserId::new(), UserId::new());
        let noted = BTreeMap::from([(old, now - RECENT_MS - 1), (fresh, now - 10), (renoted, now - RECENT_MS - 5)]);
        let picked = pick_partners(&noted, &[(local, now - 5), (renoted, now - 1)], now);
        assert_eq!(picked, vec![renoted, local, fresh]);
        let many: Vec<(UserId, i64)> = (0..(MAX_PARTNERS as i64 + 5)).map(|n| (UserId::new(), now - n)).collect();
        let picked = pick_partners(&BTreeMap::new(), &many, now);
        assert_eq!(picked.len(), MAX_PARTNERS);
        assert_eq!(picked[0], many[0].0);
    }

    #[test]
    fn only_new_messages_wake_the_partners_phone() {
        let (a, b) = (UserId::new(), UserId::new());
        let target = commy_core::MessageId::new();
        assert!(rings(&DmMessage::new(a, b, "hello")));
        assert!(!rings(&DmMessage::acting(a, b, DmAction::Edit { target }, "hello!")));
        assert!(!rings(&DmMessage::acting(a, b, DmAction::React { target, emoji: "🔥".into(), active: true }, "")));
        assert!(!rings(&DmMessage::acting(a, b, DmAction::Delete { target }, "")));
    }

    #[test]
    fn history_from_the_service_never_rings() {
        let now = Utc::now();
        assert!(should_ring(true, now - chrono::Duration::seconds(5), now));
        assert!(!should_ring(false, now - chrono::Duration::seconds(5), now), "catching up stays quiet");
        assert!(!should_ring(true, now - chrono::Duration::hours(2), now), "old news stays quiet");
    }
}
