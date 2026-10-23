use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use commy_core::channel_key::{ChannelSecret, KeyId};
use commy_core::dm::{CallSignal, DmAction, DmMessage};
use commy_core::{ChannelKind, DmStore, IdentityKey, MessageId, ServerId, ServerState, UserId, UserProfile};
use serde::{Deserialize, Serialize};

use crate::safety::{self, SafetySettings};
use crate::DmThreadView;

#[derive(Serialize, Deserialize)]
struct InviteKey {
    #[serde(rename = "i")]
    id: String,
    #[serde(rename = "s")]
    secret: String,
}

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

pub fn group_invite(from: UserId, to: UserId, ticket: String, key: Option<(KeyId, &ChannelSecret)>) -> DmMessage {
    let content = key
        .map(|(id, secret)| {
            let key = InviteKey { id: id.to_hex(), secret: data_encoding::BASE64URL_NOPAD.encode(secret.bytes()) };
            serde_json::to_string(&key).expect("invite key encodes")
        })
        .unwrap_or_default();
    DmMessage::acting(from, to, DmAction::GroupInvite { ticket }, content)
}

pub fn invite_key_of(message: &DmMessage) -> Option<(KeyId, ChannelSecret)> {
    if !matches!(message.action, DmAction::GroupInvite { .. }) {
        return None;
    }
    let key: InviteKey = serde_json::from_str(&message.content).ok()?;
    let id: [u8; 16] = data_encoding::HEXLOWER_PERMISSIVE.decode(key.id.as_bytes()).ok()?.try_into().ok()?;
    let secret: [u8; 32] = data_encoding::BASE64URL_NOPAD.decode(key.secret.as_bytes()).ok()?.try_into().ok()?;
    Some((KeyId(id), ChannelSecret::from_bytes(secret)))
}

pub fn invite_welcome(dms: &DmStore, safety: &SafetySettings, servers: &HashMap<ServerId, ServerState>, me: UserId, from: UserId) -> bool {
    !safety.is_blocked(from)
        && (dms.has_sent_to(me, from) || safety.friends.contains(&from) || servers.values().any(|s| s.server.is_member(from)))
}

pub fn group_threads<'a>(servers: impl IntoIterator<Item = &'a ServerState>, me: UserId) -> Vec<DmThreadView> {
    servers
        .into_iter()
        .filter(|s| s.server.group)
        .map(|state| {
            let last = state
                .server
                .sorted_channels()
                .into_iter()
                .filter(|c| c.kind == ChannelKind::Text)
                .flat_map(|c| state.messages(c.id).iter())
                .max_by_key(|m| m.created_at);
            DmThreadView {
                user: state.server.id.to_string(),
                name: crate::server_name(state, Some(me)),
                last_message: last
                    .map(|m| {
                        let who = if m.author == me { "You".to_string() } else { crate::display_name(state, m.author) };
                        let text = if m.content.is_empty() {
                            m.files().next().map(|a| a.filename.clone()).unwrap_or_default()
                        } else {
                            safety::truncate(&m.content, 60)
                        };
                        format!("{who}: {text}")
                    })
                    .unwrap_or_default(),
                last_at: last.map(|m| m.created_at.to_rfc3339()).unwrap_or_default(),
                kind: "group".to_string(),
            }
        })
        .collect()
}

pub fn sort_threads(views: &mut Vec<DmThreadView>, dms: &DmStore, safety: &SafetySettings, me: UserId) {
    views.sort_by(|a, b| b.last_at.cmp(&a.last_at));
    views.retain_mut(|view| {
        if view.kind == "group" {
            return true;
        }
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
    fn a_group_invite_carries_the_group_key_and_old_invites_still_read() {
        let (a, b) = (UserId::new(), UserId::new());
        let (id, secret) = (KeyId::random(), ChannelSecret::random());
        let invite = group_invite(a, b, "ticket".into(), Some((id, &secret)));
        assert!(matches!(&invite.action, DmAction::GroupInvite { ticket } if ticket == "ticket"));
        let (back, opened) = invite_key_of(&invite).unwrap();
        assert_eq!(back, id);
        assert_eq!(opened.bytes(), secret.bytes());
        assert!(!syncable(&invite));
        assert!(invite_key_of(&group_invite(a, b, "ticket".into(), None)).is_none());
        assert!(invite_key_of(&DmMessage::new(a, b, invite.content.clone())).is_none(), "only an invite carries a key");
    }

    #[test]
    fn group_invites_come_only_from_people_we_know() {
        let (me, friend, member, stranger) = (UserId::new(), UserId::new(), UserId::new(), UserId::new());
        let mut safety = SafetySettings::default();
        safety.friends.insert(friend);
        let mut dms = DmStore::new();
        let talked = UserId::new();
        dms.apply(talked, DmMessage::new(me, talked, "hi"));
        let server = commy_core::Server::new("club", member);
        let servers = HashMap::from([(server.id, ServerState::new(server))]);
        assert!(invite_welcome(&dms, &safety, &servers, me, friend));
        assert!(invite_welcome(&dms, &safety, &servers, me, member));
        assert!(invite_welcome(&dms, &safety, &servers, me, talked));
        assert!(!invite_welcome(&dms, &safety, &servers, me, stranger));
        safety.blocked.push(friend);
        assert!(!invite_welcome(&dms, &safety, &servers, me, friend));
    }

    #[test]
    fn a_group_stays_a_group_in_the_conversation_list() {
        let me = UserId::new();
        let state = ServerState::new(commy_core::Server::group(me));
        let mut views = group_threads([&state], me);
        sort_threads(&mut views, &DmStore::new(), &SafetySettings::default(), me);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].kind, "group");
        assert_eq!(views[0].user, state.server.id.to_string());
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
