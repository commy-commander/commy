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
        !self.keyed || matches!(payload, WirePayload::Sealed(_) | WirePayload::DaySealed(_))
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

fn day_place(server: ServerId) -> Vec<u8> {
    postcard::to_allocvec(&server).expect("ids always encode")
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn seal_days(server: ServerId, payload: &WirePayload, sealer: &DaySealer, now_ms: i64) -> Option<WirePayload> {
    if matches!(payload, WirePayload::Sealed(_) | WirePayload::DaySealed(_)) || !matches!(crate::community_streams::lane_of_payload(payload), Some(crate::community_streams::Lane::Channel(_))) {
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

pub fn seal_with(keys: Option<&ServerKeys>, server: ServerId, payload: WirePayload) -> WirePayload {
    let sealer = keys.and_then(|k| k.days.as_ref()).and_then(|d| d.sealer.as_ref());
    let payload = sealer.and_then(|sealer| seal_days(server, &payload, sealer, now_ms())).unwrap_or(payload);
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
