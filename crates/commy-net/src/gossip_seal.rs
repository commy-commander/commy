use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use commy_core::channel_key::{ChannelSecret, KeyId, KeyScope, Keyring};
use commy_core::day_keys::{DayKeyring, DaySealer};
use commy_core::{ServerId, ServerState};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::protocol::WirePayload;

const SEAL_DOMAIN: &str = "commy-gossip-seal/v1";
const DAY_LABEL: &str = "commy-gossip-day/v1";
const NONCE_BYTES: usize = 12;

pub const SEAL_OVERHEAD_BYTES: usize = 160;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SealedPayload {
    pub key: KeyId,
    pub nonce: [u8; NONCE_BYTES],
    pub body: Vec<u8>,
}

#[derive(Clone, Default)]
pub struct ServerKeys {
    pub sealing: Option<(KeyId, ChannelSecret)>,
    pub held: Vec<(KeyId, ChannelSecret)>,
    pub days: Option<DayKeys>,
    pub keyed: bool,
}

#[derive(Clone)]
pub struct DayKeys {
    pub sealer: Option<DaySealer>,
    pub ring: DayKeyring,
}

impl ServerKeys {
    pub fn secret(&self, key: KeyId) -> Option<&ChannelSecret> {
        self.sealing.iter().chain(self.held.iter()).find(|(id, _)| *id == key).map(|(_, secret)| secret)
    }

    pub fn may_send(&self, payload: &WirePayload) -> bool {
        if self.days.is_some() && day_lane(payload) {
            return false;
        }
        !self.keyed || rides_plain(payload) || matches!(payload, WirePayload::Sealed(_) | WirePayload::DaySealed(_))
    }
}

pub fn server_keys(state: &ServerState, community: &Keyring) -> ServerKeys {
    let held: Vec<(KeyId, ChannelSecret)> = community.held_records(state, KeyScope::Community).into_iter().map(|(record, secret)| (record.id, secret.clone())).collect();
    let newest = community.newest_in(state, KeyScope::Community).map(|(key, secret)| (key, secret.clone()));
    let sealing = newest.or_else(|| held.first().cloned());
    let keyed = state.community_keys().newest().is_some();
    ServerKeys { sealing, held, days: None, keyed }
}

pub fn server_keys_with_days(state: &ServerState, community: &Keyring, days: &DayKeyring) -> ServerKeys {
    let on = commy_core::day_chain::is_on(&state.server);
    let days = on.then(|| DayKeys { sealer: days.sealer(state), ring: days.clone() });
    ServerKeys { days, ..server_keys(state, community) }
}

pub fn with_given(mut keys: ServerKeys, given: Option<(KeyId, ChannelSecret)>) -> ServerKeys {
    let Some((id, secret)) = given else { return keys };
    if keys.secret(id).is_none() {
        keys.held.push((id, secret.clone()));
    }
    if keys.sealing.is_none() {
        keys.sealing = Some((id, secret));
    }
    keys.keyed = true;
    keys
}

fn day_place(server: ServerId) -> Vec<u8> {
    postcard::to_allocvec(&server).expect("ids always encode")
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn day_lane(payload: &WirePayload) -> bool {
    !matches!(payload, WirePayload::Sealed(_) | WirePayload::DaySealed(_))
        && matches!(crate::community_streams::lane_of_payload(payload), Some(crate::community_streams::Lane::Channel(_)))
}

pub fn seal_days(server: ServerId, payload: &WirePayload, sealer: &DaySealer, now_ms: i64) -> Option<WirePayload> {
    if !day_lane(payload) {
        return None;
    }
    let plain = commy_core::wire_layout::encode(payload).ok()?;
    sealer.seal(DAY_LABEL, &day_place(server), &plain, now_ms).map(WirePayload::DaySealed)
}

pub fn open_days(server: ServerId, sealed: &commy_core::day_keys::DaySealed, ring: &DayKeyring) -> Option<WirePayload> {
    let plain = ring.open(server, sealed, DAY_LABEL, &day_place(server))?;
    match commy_core::wire_layout::decode::<WirePayload>(&plain).ok()? {
        WirePayload::Sealed(_) | WirePayload::DaySealed(_) => None,
        inner => Some(inner),
    }
}

pub type GossipKeys = Arc<dyn Fn(ServerId) -> Option<ServerKeys> + Send + Sync>;

fn bound_to(server: ServerId, key: KeyId) -> Vec<u8> {
    postcard::to_allocvec(&(SEAL_DOMAIN, server, key)).expect("postcard encodes plain data")
}

fn cipher(secret: &ChannelSecret) -> Aes256Gcm {
    Aes256Gcm::new_from_slice(secret.bytes()).expect("32 byte key")
}

pub fn seal(server: ServerId, payload: &WirePayload, key: KeyId, secret: &ChannelSecret) -> Option<WirePayload> {
    if matches!(payload, WirePayload::Sealed(_)) {
        return None;
    }
    let plain = commy_core::wire_layout::encode(payload).ok()?;
    let nonce: [u8; NONCE_BYTES] = rand::random();
    let body = cipher(secret).encrypt(&Nonce::from(nonce), Payload { msg: &plain, aad: &bound_to(server, key) }).ok()?;
    Some(WirePayload::Sealed(SealedPayload { key, nonce, body }))
}

pub fn open(server: ServerId, sealed: &SealedPayload, secret: &ChannelSecret) -> Option<WirePayload> {
    let plain = cipher(secret)
        .decrypt(&Nonce::from(sealed.nonce), Payload { msg: &sealed.body, aad: &bound_to(server, sealed.key) })
        .ok()?;
    match commy_core::wire_layout::decode::<WirePayload>(&plain).ok()? {
        WirePayload::Sealed(_) => None,
        inner => Some(inner),
    }
}

pub fn older_apps_may_still_join(server: &commy_core::Server) -> bool {
    server.day_keys.is_none()
}

pub fn rides_plain(payload: &WirePayload) -> bool {
    match payload {
        WirePayload::SyncRequest => true,
        WirePayload::SyncResponse { server, recent_messages, .. } => recent_messages.is_empty() && older_apps_may_still_join(server),
        _ => false,
    }
}

pub fn seal_with(keys: Option<&ServerKeys>, server: ServerId, payload: WirePayload) -> WirePayload {
    if rides_plain(&payload) {
        return payload;
    }
    let days = keys.and_then(|k| k.days.as_ref());
    let payload = match days {
        Some(days) if day_lane(&payload) => match days.sealer.as_ref().and_then(|sealer| seal_days(server, &payload, sealer, now_ms())) {
            Some(sealed) => sealed,
            None => return payload,
        },
        _ => payload,
    };
    let Some((key, secret)) = keys.and_then(|k| k.sealing.as_ref()) else { return payload };
    seal(server, &payload, *key, secret).unwrap_or(payload)
}

pub fn open_with(keys: Option<&ServerKeys>, server: ServerId, sealed: &SealedPayload) -> Option<WirePayload> {
    match open(server, sealed, keys?.secret(sealed.key)?)? {
        WirePayload::DaySealed(inner) => open_days(server, &inner, &keys?.days.as_ref()?.ring),
        inner => Some(inner),
    }
}

pub fn open_day_payload(keys: Option<&ServerKeys>, server: ServerId, sealed: &commy_core::day_keys::DaySealed) -> Option<WirePayload> {
    open_days(server, sealed, &keys?.days.as_ref()?.ring)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{gossip_size, Envelope, MAX_GOSSIP_BYTES};
    use commy_core::{ChannelId, Message, ServerEvent, ServerState, UserId};

    fn message(text: &str) -> WirePayload {
        WirePayload::Event(ServerEvent::MessageCreate { message: Message::new(ChannelId::new(), UserId::new(), text) })
    }

    fn keys() -> (KeyId, ChannelSecret) {
        (KeyId::random(), ChannelSecret::random())
    }

    #[test]
    fn a_newcomer_gets_the_server_layout_in_plain_but_never_its_messages() {
        let (mut state, channel, owner) = commy_core::test_support::fixture();
        let server = state.server.id;
        let (key, secret) = keys();
        let keyed = ServerKeys { sealing: Some((key, secret)), keyed: true, ..ServerKeys::default() };
        assert_eq!(seal_with(Some(&keyed), server, WirePayload::SyncRequest), WirePayload::SyncRequest);
        let layout = crate::protocol::build_sync_response(&state, 0, owner);
        assert_eq!(seal_with(Some(&keyed), server, layout.clone()), layout);
        assert!(keyed.may_send(&layout));
        let words = Message::new(channel, owner, "meet at the harbour");
        state.apply(ServerEvent::MessageCreate { message: commy_core::test_support::signed(&state, words) }).unwrap();
        let with_messages = crate::protocol::build_sync_response(&state, 50, owner);
        assert!(!rides_plain(&with_messages));
        assert!(matches!(seal_with(Some(&keyed), server, with_messages), WirePayload::Sealed(_)));
    }

    #[test]
    fn the_plain_layout_also_stops_once_day_keys_were_turned_off_on_purpose() {
        let (mut state, _, owner) = commy_core::test_support::fixture();
        let server = state.server.id;
        state.apply(ServerEvent::DayKeysSet { server, on: false, since: 0, history_days: 30 }).unwrap();
        assert!(!older_apps_may_still_join(&state.server));
        assert!(!rides_plain(&crate::protocol::build_sync_response(&state, 0, owner)));
    }

    #[test]
    fn the_plain_layout_stops_once_the_server_has_moved_to_day_keys() {
        let (mut state, _, owner) = commy_core::test_support::fixture();
        let server = state.server.id;
        let (key, secret) = keys();
        let keyed = ServerKeys { sealing: Some((key, secret)), keyed: true, ..ServerKeys::default() };
        assert!(rides_plain(&crate::protocol::build_sync_response(&state, 0, owner)));
        state.apply(ServerEvent::DayKeysSet { server, on: true, since: chrono::Utc::now().timestamp_millis(), history_days: 30 }).unwrap();
        assert!(!older_apps_may_still_join(&state.server));
        let layout = crate::protocol::build_sync_response(&state, 0, owner);
        assert!(!rides_plain(&layout));
        assert!(matches!(seal_with(Some(&keyed), server, layout.clone()), WirePayload::Sealed(_)));
        assert!(!keyed.may_send(&layout));
    }

    #[test]
    fn a_keyed_server_never_lets_a_plain_frame_out() {
        let server = ServerId::new();
        let plain = message("hi");
        let unkeyed = ServerKeys::default();
        assert!(unkeyed.may_send(&plain));
        let keyed_without_key = ServerKeys { keyed: true, ..ServerKeys::default() };
        let out = seal_with(Some(&keyed_without_key), server, plain.clone());
        assert_eq!(out, plain);
        assert!(!keyed_without_key.may_send(&out));
        let (key, secret) = keys();
        let keyed = ServerKeys { sealing: Some((key, secret)), keyed: true, ..ServerKeys::default() };
        assert!(keyed.may_send(&seal_with(Some(&keyed), server, plain)));
    }

    #[test]
    fn a_sealed_frame_hides_the_words_and_opens_with_the_same_key() {
        let server = ServerId::new();
        let (key, secret) = keys();
        let plain = message("meet at the harbour");
        let sealed = seal(server, &plain, key, &secret).unwrap();
        let bytes = Envelope::new([1; 32], sealed.clone()).encode().unwrap();
        assert!(!bytes.windows(7).any(|w| w == b"harbour"));
        let WirePayload::Sealed(inner) = sealed else { panic!("not sealed") };
        assert_eq!(open(server, &inner, &secret), Some(plain));
    }

    #[test]
    fn a_sealed_frame_does_not_open_with_another_key_or_for_another_server() {
        let server = ServerId::new();
        let (key, secret) = keys();
        let WirePayload::Sealed(inner) = seal(server, &message("hi"), key, &secret).unwrap() else { panic!("not sealed") };
        assert_eq!(open(server, &inner, &ChannelSecret::random()), None);
        assert_eq!(open(ServerId::new(), &inner, &secret), None);
        let relabelled = SealedPayload { key: KeyId::random(), ..inner.clone() };
        assert_eq!(open(server, &relabelled, &secret), None);
    }

    #[test]
    fn sealed_frames_never_nest() {
        let server = ServerId::new();
        let (key, secret) = keys();
        let once = seal(server, &message("hi"), key, &secret).unwrap();
        assert_eq!(seal(server, &once, key, &secret), None);
        let inner_bytes = commy_core::wire_layout::encode(&once).unwrap();
        let nonce = [3u8; NONCE_BYTES];
        let body = cipher(&secret).encrypt(&Nonce::from(nonce), Payload { msg: &inner_bytes, aad: &bound_to(server, key) }).unwrap();
        assert_eq!(open(server, &SealedPayload { key, nonce, body }, &secret), None);
    }

    #[test]
    fn without_a_key_the_payload_goes_as_it_was_and_cannot_be_opened() {
        let server = ServerId::new();
        let plain = message("hi");
        assert_eq!(seal_with(None, server, plain.clone()), plain);
        assert_eq!(seal_with(Some(&ServerKeys::default()), server, plain.clone()), plain);
        let (key, secret) = keys();
        let WirePayload::Sealed(inner) = seal(server, &plain, key, &secret).unwrap() else { panic!("not sealed") };
        assert_eq!(open_with(None, server, &inner), None);
        assert_eq!(open_with(Some(&ServerKeys::default()), server, &inner), None);
        let older = ServerKeys { sealing: Some(keys()), held: vec![(key, secret)], days: None, keyed: true };
        assert_eq!(open_with(Some(&older), server, &inner), Some(plain));
    }

    #[test]
    fn the_largest_gossip_payload_still_fits_once_sealed() {
        let server = ServerId::new();
        let (key, secret) = keys();
        let mut text = "x".repeat(commy_core::message::MAX_MESSAGE_BYTES);
        while gossip_size(&message(&text)) > MAX_GOSSIP_BYTES {
            text.pop();
        }
        let sealed = seal(server, &message(&text), key, &secret).unwrap();
        assert!(gossip_size(&sealed) <= MAX_GOSSIP_BYTES + SEAL_OVERHEAD_BYTES, "{}", gossip_size(&sealed));
    }

    fn community_ring(state: &mut ServerState, me: commy_core::UserId) -> Keyring {
        use commy_core::test_support::identity_for;
        let now = chrono::Utc::now().timestamp_millis();
        for _ in 0..4 {
            let mut ring = Keyring::unlock_community(state, me, &identity_for(me));
            let Some(task) = commy_core::channel_key::community_key_task(state, me, &ring) else { break };
            for event in ring.task_events(state, KeyScope::Community, task, now) {
                state.apply(event).unwrap();
            }
        }
        Keyring::unlock_community(state, me, &identity_for(me))
    }

    #[test]
    fn a_newer_key_that_has_not_reached_this_device_leaves_it_sealing_with_the_newest_it_holds() {
        let (mut state, _, owner) = commy_core::test_support::fixture();
        let server = state.server.id;
        let ring = community_ring(&mut state, owner);
        let held = ring.newest_in(&state, KeyScope::Community).unwrap().0;
        let newer = Keyring::default().rotate(server, KeyScope::Community, &[], chrono::Utc::now().timestamp_millis() + 1000);
        let newer_id = newer.id;
        state.apply(ServerEvent::CommunityKeyAdd { server, key: newer }).unwrap();
        assert_eq!(state.community_keys().newest().map(|k| k.id), Some(newer_id));
        let keys = server_keys(&state, &Keyring::unlock_community(&state, owner, &commy_core::test_support::identity_for(owner)));
        assert!(keys.keyed);
        assert_eq!(keys.sealing.as_ref().map(|(id, _)| *id), Some(held));
        assert!(keys.may_send(&seal_with(Some(&keys), server, message("hi"))));
    }

    fn day_keys_for(state: &mut ServerState) -> (DayKeyring, DayKeyring, commy_core::UserId) {
        use commy_core::test_support::{identity_for, join};
        let owner = state.server.owner;
        let member = join(state, "member");
        let server = state.server.id;
        let now = chrono::Utc::now().timestamp_millis();
        state.apply(ServerEvent::DayKeysSet { server, on: true, since: now, history_days: 30 }).unwrap();
        let mut ring = DayKeyring::unlock(state, owner, &identity_for(owner));
        let task = commy_core::day_keys::day_key_task(state, owner, &ring, now).unwrap();
        for event in ring.task_events(state, task, now) {
            state.apply(event).unwrap();
        }
        (DayKeyring::unlock(state, owner, &identity_for(owner)), DayKeyring::unlock(state, member, &identity_for(member)), member)
    }

    fn with_days(root: Option<(KeyId, ChannelSecret)>, state: &ServerState, ring: &DayKeyring) -> ServerKeys {
        ServerKeys { sealing: root.clone(), held: root.into_iter().collect(), days: Some(DayKeys { sealer: ring.sealer(state), ring: ring.clone() }), keyed: true }
    }

    #[test]
    fn a_newcomer_whose_copy_has_no_key_records_yet_opens_and_seals_with_the_invite_key() {
        let (mut owner_state, channel, owner) = commy_core::test_support::fixture();
        let server = owner_state.server.id;
        let ring = community_ring(&mut owner_state, owner);
        let owners = server_keys(&owner_state, &ring);
        let invite = owners.sealing.clone().unwrap();
        let placeholder = ServerState::new(owner_state.server.clone());
        let bare = server_keys(&placeholder, &Keyring::default());
        assert!(!bare.keyed && bare.held.is_empty(), "a copy without key records holds nothing on its own");
        let newcomer = with_given(bare, Some(invite.clone()));
        let plain = WirePayload::Event(ServerEvent::MessageCreate { message: Message::new(channel, owner, "welcome") });
        let WirePayload::Sealed(from_owner) = seal_with(Some(&owners), server, plain.clone()) else { panic!("not sealed") };
        assert_eq!(open_with(Some(&newcomer), server, &from_owner), Some(plain.clone()));
        let sent = seal_with(Some(&newcomer), server, plain.clone());
        assert!(newcomer.may_send(&sent));
        let WirePayload::Sealed(from_newcomer) = sent else { panic!("the newcomer sends sealed") };
        assert_eq!(open_with(Some(&owners), server, &from_newcomer), Some(plain));
        assert!(!newcomer.may_send(&message("plain")), "nothing goes out in plain");
        let again = with_given(owners.clone(), Some(invite));
        assert_eq!(again.held.len(), owners.held.len(), "a key already held is not added twice");
    }

    #[test]
    fn without_a_day_key_messages_are_not_sent_on_a_server_with_day_keys_on() {
        let (mut state, channel, _) = commy_core::test_support::fixture();
        let server = state.server.id;
        day_keys_for(&mut state);
        let root = keys();
        let waiting = ServerKeys { sealing: Some(root.clone()), held: vec![root.clone()], days: Some(DayKeys { sealer: None, ring: DayKeyring::default() }), keyed: true };
        let plain = WirePayload::Event(ServerEvent::MessageCreate { message: Message::new(channel, state.server.owner, "after the switch") });
        let sent = seal_with(Some(&waiting), server, plain.clone());
        assert_eq!(sent, plain, "the message is not sealed with the community key alone");
        assert!(!waiting.may_send(&sent));
        let unkeyed = ServerKeys { keyed: false, ..waiting.clone() };
        assert!(!unkeyed.may_send(&seal_with(Some(&unkeyed), server, plain)));
        let rename = WirePayload::Event(ServerEvent::ServerRename { server, name: "Still sent".into() });
        let sealed = seal_with(Some(&waiting), server, rename);
        assert!(matches!(sealed, WirePayload::Sealed(_)) && waiting.may_send(&sealed), "server changes still go out");
    }

    #[test]
    fn messages_are_sealed_with_the_day_key_inside_the_community_seal() {
        let (mut state, channel, _) = commy_core::test_support::fixture();
        let server = state.server.id;
        let (owners, members, _) = day_keys_for(&mut state);
        let root = keys();
        let plain = WirePayload::Event(ServerEvent::MessageCreate { message: Message::new(channel, state.server.owner, "meet at the harbour") });
        let sealed = seal_with(Some(&with_days(Some(root.clone()), &state, &owners)), server, plain.clone());
        let WirePayload::Sealed(outer) = sealed else { panic!("not sealed") };
        assert!(matches!(open(server, &outer, &root.1), Some(WirePayload::DaySealed(_))), "the community key alone shows only the day seal");
        assert_eq!(open_with(Some(&with_days(Some(root.clone()), &state, &members)), server, &outer), Some(plain.clone()));
        let root_only = ServerKeys { sealing: Some(root.clone()), held: vec![root.clone()], days: None, keyed: true };
        assert_eq!(open_with(Some(&root_only), server, &outer), None);
        let rename = WirePayload::Event(ServerEvent::ServerRename { server, name: "x".into() });
        let WirePayload::Sealed(outer) = seal_with(Some(&with_days(Some(root.clone()), &state, &owners)), server, rename.clone()) else { panic!("not sealed") };
        assert_eq!(open_with(Some(&root_only), server, &outer), Some(rename), "server changes stay on the community key");
        let bare = seal_with(Some(&with_days(None, &state, &owners)), server, plain.clone());
        let WirePayload::DaySealed(inner) = bare else { panic!("not day sealed") };
        assert_eq!(open_day_payload(Some(&with_days(None, &state, &members)), server, &inner), Some(plain));
        assert_eq!(open_day_payload(Some(&with_days(None, &state, &members)), ServerId::new(), &inner), None);
    }

    #[test]
    fn the_largest_message_still_fits_with_both_seals() {
        let (mut state, channel, _) = commy_core::test_support::fixture();
        let server = state.server.id;
        let (owners, _, _) = day_keys_for(&mut state);
        let mut text = "x".repeat(commy_core::message::MAX_MESSAGE_BYTES);
        let make = |text: &str| WirePayload::Event(ServerEvent::MessageCreate { message: Message::new(channel, state.server.owner, text) });
        while gossip_size(&make(&text)) > MAX_GOSSIP_BYTES {
            text.pop();
        }
        let sealed = seal_with(Some(&with_days(Some(keys()), &state, &owners)), server, make(&text));
        assert!(matches!(sealed, WirePayload::Sealed(_)));
        assert!(gossip_size(&sealed) <= MAX_GOSSIP_BYTES + SEAL_OVERHEAD_BYTES, "{}", gossip_size(&sealed));
    }
}
