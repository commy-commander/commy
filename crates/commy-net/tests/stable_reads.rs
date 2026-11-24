use commy_core::home::{card_payload, HomeCard, CARD_PURPOSE};
use commy_core::identity::Identity;
use commy_core::{wire_layout, Message, Server, ServerEvent, SignedEvent, UserProfile};
use commy_core_0_2_4 as stable;
use commy_core_1_0_0 as stable_1_0_0;
use commy_net::protocol::{sign_sync_response, verify_sync_response, Envelope, WirePayload};

type Sample = (Server, Vec<UserProfile>, Vec<Message>, Vec<ServerEvent>, u64);
type StableSample = (stable::Server, Vec<stable::UserProfile>, Vec<stable::Message>, Vec<stable::ServerEvent>, u64);
type Stable100Sample = (
    stable_1_0_0::Server,
    Vec<stable_1_0_0::UserProfile>,
    Vec<stable_1_0_0::Message>,
    Vec<stable_1_0_0::ServerEvent>,
    u64,
);

const HOME: &str = "https://commy.chat";
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

fn written_by_0_2_4() -> Vec<u8> {
    let hex = include_str!("../../commy-core/tests/fixtures/wire-0.2.4.hex").trim();
    data_encoding::HEXLOWER.decode(hex.as_bytes()).expect("the fixture is hex")
}

fn person(seed: u8) -> Identity {
    Identity::from_bytes(&[seed; 32])
}

fn with_card(mut profile: UserProfile, seed: u8) -> UserProfile {
    let home_key = person(200);
    let identity = profile.identity.unwrap_or_else(|| person(seed).public());
    let issued = 1_800_000_000_000;
    let expires = issued + 30 * DAY_MS;
    let payload = card_payload(profile.id, &identity, issued, expires);
    let sig = commy_core::server_key::sign(&home_key, CARD_PURPOSE, HOME, payload.as_bytes());
    profile.home = Some(HomeCard { home: HOME.into(), user: profile.id, identity, key: home_key.public(), issued, expires, sig });
    profile
}

fn with_every_new_field_set() -> Sample {
    let (mut server, profiles, messages, mut events, after) = wire_layout::decode::<Sample>(&written_by_0_2_4()).unwrap();
    server.sync_host = Some("https://sync.home.example".into());
    server.blocked_homes = vec!["spam.example".into()];
    server.require_home = true;
    let profiles: Vec<UserProfile> = profiles.into_iter().enumerate().map(|(i, p)| with_card(p, 10 + i as u8)).collect();
    for event in &mut events {
        if let ServerEvent::MemberJoin { profile, .. } = event {
            *profile = with_card(profile.clone(), 30);
        }
    }
    (server, profiles, messages, events, after)
}

#[test]
fn stable_0_2_4_reads_values_written_with_every_new_field_set() {
    let sample = with_every_new_field_set();
    let bytes = wire_layout::encode(&sample).unwrap();
    let read: StableSample = postcard::from_bytes(&bytes).expect("Commy 0.2.4 reads what this build writes");
    assert_eq!(postcard::to_stdvec(&read).unwrap(), written_by_0_2_4(), "0.2.4 sees exactly the values it knows");
    assert_eq!(wire_layout::decode::<Sample>(&bytes).unwrap(), sample, "this build keeps the new fields");
}

#[test]
fn stable_0_2_4_accepts_events_signed_by_this_build() {
    let (server, profiles, ..) = with_every_new_field_set();
    let signer = person(10);
    let by = profiles[0].id;
    let events = [
        ServerEvent::ProfileUpdate { profile: profiles[0].clone() },
        ServerEvent::ServerRename { server: server.id, name: "Review".into() },
    ];
    for event in events {
        let signed = SignedEvent::sign(server.id, by, 1_800_000_000_000, event, &signer);
        let bytes = wire_layout::encode(&signed).unwrap();
        let read: stable::SignedEvent = postcard::from_bytes(&bytes).expect("Commy 0.2.4 reads the event");
        let stable_server: stable::ServerId = postcard::from_bytes(&postcard::to_stdvec(&server.id).unwrap()).unwrap();
        let stable_key = stable::identity::Identity::from_bytes(&[10; 32]).public();
        assert!(read.verify(stable_server, &stable_key), "Commy 0.2.4 accepts the signature");
        let back: SignedEvent = wire_layout::decode(&bytes).unwrap();
        assert_eq!(back, signed);
        assert!(back.verify(server.id, &signer.public()));
    }
}

#[test]
fn stable_1_0_0_reads_values_written_with_every_new_field_set() {
    let sample = with_every_new_field_set();
    let bytes = wire_layout::encode(&sample).unwrap();
    let (server, profiles, messages, events, after): Stable100Sample =
        stable_1_0_0::wire_layout::decode(&bytes).expect("Commy 1.0.0 reads what this build writes");
    assert_eq!(server.name, sample.0.name);
    assert_eq!(server.sync_host, sample.0.sync_host);
    assert_eq!(server.blocked_homes, sample.0.blocked_homes);
    assert_eq!(server.require_home, sample.0.require_home);
    assert_eq!(profiles.len(), sample.1.len());
    assert!(profiles.iter().all(|p| p.home.is_some()), "1.0.0 keeps the home cards");
    assert_eq!(messages.len(), sample.2.len());
    assert_eq!(events.len(), sample.3.len());
    assert_eq!(after, sample.4);
}

#[test]
fn stable_1_0_0_accepts_events_signed_by_this_build() {
    let (server, profiles, ..) = with_every_new_field_set();
    let signer = person(10);
    let by = profiles[0].id;
    let events = [
        ServerEvent::ProfileUpdate { profile: profiles[0].clone() },
        ServerEvent::ServerRename { server: server.id, name: "Review".into() },
    ];
    for event in events {
        let signed = SignedEvent::sign(server.id, by, 1_800_000_000_000, event, &signer);
        let bytes = wire_layout::encode(&signed).unwrap();
        let read: stable_1_0_0::SignedEvent = stable_1_0_0::wire_layout::decode(&bytes).expect("Commy 1.0.0 reads the event");
        let stable_server: stable_1_0_0::ServerId = stable_1_0_0::wire_layout::decode(&wire_layout::encode(&server.id).unwrap()).unwrap();
        let stable_key = stable_1_0_0::identity::Identity::from_bytes(&[10; 32]).public();
        assert!(read.verify(stable_server, &stable_key), "Commy 1.0.0 accepts the signature");
    }
}

fn signed_sync_response() -> (WirePayload, Identity) {
    let (server, profiles, messages, ..) = with_every_new_field_set();
    let responder = person(10);
    let mut payload = WirePayload::SyncResponse {
        by: profiles[0].id,
        server,
        members: Vec::new(),
        profiles,
        recent_messages: messages,
        voice: Vec::new(),
        at: 0,
        sig: commy_core::identity::Signature([0; 64]),
        full_sig: None,
    };
    sign_sync_response(&mut payload, 1_800_000_000_000, &responder);
    (payload, responder)
}

#[test]
fn a_build_with_only_the_first_layout_reads_and_trusts_a_sync_response() {
    let (payload, responder) = signed_sync_response();
    let bytes = Envelope::new([1; 32], payload).encode().unwrap();
    let read: Envelope = wire_layout::read_as_first_layout(&bytes).expect("read like Commy 0.2.4");
    let WirePayload::SyncResponse { server, profiles, .. } = &read.payload else { panic!("a sync response") };
    assert_eq!(server.name, "Commy Review");
    assert!(server.sync_host.is_none() && profiles.iter().all(|p| p.home.is_none()));
    assert!(verify_sync_response(&read.payload, &responder.public()), "the signature covers what 0.2.4 reads");
}

#[test]
fn this_build_trusts_new_fields_in_a_sync_response_only_when_signed() {
    let (payload, responder) = signed_sync_response();
    let bytes = Envelope::new([1; 32], payload.clone()).encode().unwrap();
    let read = Envelope::decode(&bytes).unwrap();
    assert_eq!(read.payload, payload);
    assert!(verify_sync_response(&read.payload, &responder.public()));
    let mut moved = read.payload.clone();
    if let WirePayload::SyncResponse { server, .. } = &mut moved {
        server.sync_host = Some("https://elsewhere.example".into());
    }
    assert!(!verify_sync_response(&moved, &responder.public()), "a changed sync host is caught");
    let mut unsigned = read.payload;
    if let WirePayload::SyncResponse { full_sig, .. } = &mut unsigned {
        *full_sig = None;
    }
    assert!(!verify_sync_response(&unsigned, &responder.public()), "new fields need the full signature");
}
