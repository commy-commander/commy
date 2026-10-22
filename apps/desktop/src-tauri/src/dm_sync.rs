use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use commy_core::dm::{DmAction, DmMessage};
use commy_core::{IdentityKey, UserId, UserProfile};
use commy_net::node::IncomingDm;
use commy_net::sync_streams::{self, DmEntry};
use commy_net::DmEnvelope;
use commy_sync::wake::DAY_MS;
use commy_sync::{StreamId, SyncEvent};
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
    {
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
                caught_up: HashSet::new(),
                task,
            });
        }
    }
    refresh(handle);
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
    push_wake::register(handle);
}

pub fn wake_tags(app: &App, days: &[i64]) -> Vec<([u8; 32], i64)> {
    let (Some(identity), Some(me)) = (app.identity(), app.profile.lock().unwrap().as_ref().map(|p| p.id)) else { return Vec::new() };
    let partners: Vec<IdentityKey> = match LIVE.lock().unwrap().as_ref() {
        Some(live) if live.me_key == identity.public() => live.partners.values().map(|p| p.key).collect(),
        _ => return Vec::new(),
    };
    let who = me.to_string();
    let mut tags = Vec::new();
    for day in days {
        let Some(start) = DateTime::from_timestamp_millis(day * DAY_MS) else { continue };
        let epoch = sync_streams::dm_epoch(start);
        for key in &partners {
            if let Some(keys) = sync_streams::dm_stream(&identity, key, &epoch) {
                tags.push((keys.wake_tag(&who, *day), *day));
            }
        }
    }
    tags
}

pub fn rings(message: &DmMessage) -> bool {
    matches!(message.action, DmAction::Post)
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
        SyncEvent::Entry { stream, seq, plain, .. } => {
            let found = {
                let live = LIVE.lock().unwrap();
                live.as_ref().and_then(|l| {
                    let partner = *l.owners.get(&stream)?;
                    let key = l.partners.get(&partner)?.key;
                    Some((l.me, l.me_key, partner, key, l.caught_up.contains(&stream)))
                })
            };
            let Some((me, me_key, partner, key, live_now)) = found else { return };
            match sync_streams::decode_dm(&plain, me, &me_key, partner, &key) {
                Some(DmEntry::FromPartner(envelope)) if syncable(&envelope.message) => receive(handle, envelope, live_now),
                Some(DmEntry::FromMe(envelope)) if syncable(&envelope.message) => take_mine(handle, partner, envelope.message),
                _ => {}
            }
            account_log::set_dm_cursor(&app, stream.to_hex(), seq, live_now);
        }
        SyncEvent::CaughtUp { stream, seq } => {
            let ours = match LIVE.lock().unwrap().as_mut() {
                Some(live) if live.owners.contains_key(&stream) => {
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

struct Quiet(AppHandle);

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

fn take_mine(handle: &AppHandle, partner: UserId, message: DmMessage) {
    let app = handle.state::<App>();
    let applied = app.dms.lock().unwrap().apply(partner, message);
    if applied {
        app.persist_dms();
        handle.emit("dm-changed", partner.to_string().into());
    }
}

pub fn publish(app: &App, partner: UserId, envelope: &DmEnvelope) {
    if !syncable(&envelope.message) || sync_link::url().is_none() || !account_log::is_live(app) {
        return;
    }
    if let Some(profile) = partner_profile(app, partner) {
        account_log::note_partner(app, profile, true);
    }
    let (Some(key), Some(identity)) = (partner_key(app, partner), app.identity()) else { return };
    let epoch = sync_streams::dm_epoch(Utc::now());
    let Some(keys) = sync_streams::dm_stream(&identity, &key, &epoch) else { return };
    let stream = keys.id;
    let wake = rings(&envelope.message).then(|| keys.wake_tag(&partner.to_string(), push_wake::today()));
    let http = app.http.clone();
    let mut new_partner = false;
    let client = {
        let mut guard = LIVE.lock().unwrap();
        let Some(live) = guard.as_mut().filter(|l| l.me_key == identity.public()) else { return };
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
    tauri::async_runtime::spawn(async move {
        match sync_link::append(client, stream, plain).await {
            Ok(_) => {
                if let Some(tag) = wake {
                    push_wake::ring(http, vec![tag]).await;
                }
            }
            Err(e) => tracing::debug!("could not add a DM to the sync service: {e}"),
        }
    });
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
