use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use commy_core::channel_key::{ChannelSecret, KeyId, KeyScope, Keyring};
use commy_core::{ServerId, ServerState};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::protocol::WirePayload;

const SEAL_DOMAIN: &str = "commy-gossip-seal/v1";
const NONCE_BYTES: usize = 12;

pub const SEAL_OVERHEAD_BYTES: usize = 64;

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
}

impl ServerKeys {
    pub fn secret(&self, key: KeyId) -> Option<&ChannelSecret> {
        self.sealing.iter().chain(self.held.iter()).find(|(id, _)| *id == key).map(|(_, secret)| secret)
    }
}

pub fn server_keys(state: &ServerState, community: &Keyring) -> ServerKeys {
    let sealing = community.newest_in(state, KeyScope::Community).map(|(key, secret)| (key, secret.clone()));
    let held = community.held_records(state, KeyScope::Community).into_iter().map(|(record, secret)| (record.id, secret.clone())).collect();
    ServerKeys { sealing, held }
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
    let Some((key, secret)) = keys.and_then(|k| k.sealing.as_ref()) else { return payload };
    seal(server, &payload, *key, secret).unwrap_or(payload)
}

pub fn open_with(keys: Option<&ServerKeys>, server: ServerId, sealed: &SealedPayload) -> Option<WirePayload> {
    open(server, sealed, keys?.secret(sealed.key)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{gossip_size, Envelope, MAX_GOSSIP_BYTES};
    use commy_core::{ChannelId, Message, ServerEvent, UserId};

    fn message(text: &str) -> WirePayload {
        WirePayload::Event(ServerEvent::MessageCreate { message: Message::new(ChannelId::new(), UserId::new(), text) })
    }

    fn keys() -> (KeyId, ChannelSecret) {
        (KeyId::random(), ChannelSecret::random())
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
        let older = ServerKeys { sealing: Some(keys()), held: vec![(key, secret)] };
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
}
