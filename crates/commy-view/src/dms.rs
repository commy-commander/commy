use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use commy_core::dm::{CallSignal, DmAction, DmMessage};
use commy_core::{DmStore, IdentityKey, MessageId, ServerId, ServerState, UserId, UserProfile};

use crate::safety::{self, SafetySettings};
use crate::DmThreadView;

pub const EPOCHS: u32 = 3;
pub const MAX_PARTNERS: usize = 40;
pub const RECENT_MS: i64 = 90 * 24 * 60 * 60 * 1000;
pub const QUIET_AFTER_SECS: i64 = 10 * 60;
pub const CALL_FRESH_MS: i64 = 60 * 1000;

pub fn syncable(message: &DmMessage) -> bool {
    matches!(message.action, DmAction::Post | DmAction::Edit { .. } | DmAction::Delete { .. } | DmAction::React { .. })
        && !message.content.starts_with(safety::REPORT_PREFIX)
        && safety::FriendSignal::decode(&message.content).is_none()
}

pub fn is_call(message: &DmMessage) -> bool {
    matches!(message.action, DmAction::Call { .. })
}

pub fn rings(message: &DmMessage) -> bool {
    matches!(message.action, DmAction::Post | DmAction::Call { signal: CallSignal::Ring, .. })
}

pub fn call_fresh(at: i64, now: i64) -> bool {
    (now - at).abs() < CALL_FRESH_MS
}

pub fn should_ring(live_now: bool, created_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    live_now && (now - created_at).num_seconds() < QUIET_AFTER_SECS
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

pub fn follows(dms: &DmStore, partner: UserId, queued: &HashSet<MessageId>, now_ms: i64) -> bool {
    let since = now_ms - RECENT_MS;
    dms.conversation(partner)
        .iter()
        .any(|m| m.created_at.timestamp_millis() > since && (m.from == partner || !queued.contains(&m.id)))
}

pub fn key_matches_pins(key: IdentityKey, pins: impl IntoIterator<Item = Option<IdentityKey>>) -> bool {
    pins.into_iter().flatten().all(|pinned| pinned == key)
}

pub fn member_entries(servers: &HashMap<ServerId, ServerState>, user: UserId) -> impl Iterator<Item = &UserProfile> {
    servers.values().filter(move |s| s.server.is_member(user)).filter_map(move |s| s.profiles.get(&user))
}

pub fn member_profile(servers: &HashMap<ServerId, ServerState>, user: UserId) -> Option<UserProfile> {
    let profile = member_entries(servers, user).next()?.clone();
    let mut keys = member_entries(servers, user).filter_map(|p| p.identity);
    let identity = keys.next().filter(|first| keys.all(|key| key == *first));
    Some(UserProfile { identity, ..profile })
}

pub fn sort_threads(views: &mut Vec<DmThreadView>, dms: &DmStore, safety: &SafetySettings, me: UserId) {
    views.sort_by(|a, b| b.last_at.cmp(&a.last_at));
    views.retain_mut(|view| {
        let Ok(partner) = view.user.parse::<UserId>() else { return true };
        if safety.is_blocked(partner) {
            return false;
        }
        if !safety.accepted.contains(&partner) && !dms.has_sent_to(me, partner) {
            view.kind = if safety.spam.contains(&partner) { "spam" } else { "request" }.to_string();
        }
        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_conversation_messages_go_to_the_service() {
        let (a, b) = (UserId::new(), UserId::new());
        assert!(syncable(&DmMessage::new(a, b, "hello")));
        let target = MessageId::new();
        assert!(syncable(&DmMessage::acting(a, b, DmAction::Edit { target }, "hello!")));
        assert!(syncable(&DmMessage::acting(a, b, DmAction::React { target, emoji: "🔥".into(), active: true }, "")));
        assert!(!syncable(&DmMessage::acting(a, b, DmAction::GroupInvite { ticket: "t".into() }, "")));
        assert!(!syncable(&DmMessage::new(a, b, format!("{}spam", safety::REPORT_PREFIX))));
        assert!(!syncable(&DmMessage::new(a, b, safety::FriendSignal::Request.encode())));
    }

    #[test]
    fn a_call_rings_only_while_fresh() {
        let (a, b) = (UserId::new(), UserId::new());
        let channel = commy_core::ChannelId::new();
        let ring = DmMessage::acting(a, b, DmAction::Call { call: channel, signal: CallSignal::Ring }, "");
        let hang_up = DmMessage::acting(a, b, DmAction::Call { call: channel, signal: CallSignal::HangUp }, "");
        assert!(is_call(&ring) && is_call(&hang_up));
        assert!(!is_call(&DmMessage::new(a, b, "hello")));
        assert!(!syncable(&ring), "calls never become conversation history");
        assert!(rings(&ring));
        assert!(!rings(&hang_up));
        let now = 1_000_000_000_000;
        assert!(call_fresh(now - 5_000, now));
        assert!(call_fresh(now + 5_000, now), "a clock a little ahead still rings");
        assert!(!call_fresh(now - CALL_FRESH_MS - 1, now));
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
        let target = MessageId::new();
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

    #[test]
    fn a_quiet_partner_follows_again_once_they_write_or_we_reach_them() {
        let (me, partner) = (UserId::new(), UserId::new());
        let mut dms = DmStore::new();
        let ours = DmMessage::new(me, partner, "are you around?");
        let now = ours.created_at.timestamp_millis();
        dms.apply(partner, ours.clone());
        assert!(!follows(&dms, partner, &HashSet::from([ours.id]), now));
        assert!(follows(&dms, partner, &HashSet::new(), now));
        dms.apply(partner, DmMessage::new(partner, me, "hi"));
        assert!(follows(&dms, partner, &HashSet::from([ours.id]), now));
    }

    #[test]
    fn a_key_matches_only_when_no_pin_disagrees() {
        let key = commy_core::Identity::generate().public();
        let other = commy_core::Identity::generate().public();
        assert!(key_matches_pins(key, []));
        assert!(key_matches_pins(key, [Some(key), None]));
        assert!(!key_matches_pins(key, [None, Some(other)]));
    }
}
