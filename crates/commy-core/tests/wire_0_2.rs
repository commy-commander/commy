use commy_core::{wire_layout, Message, Server, ServerEvent, UserProfile};

type Sample = (Server, Vec<UserProfile>, Vec<Message>, Vec<ServerEvent>, u64);

fn written_by_0_2_4() -> Vec<u8> {
    let hex = include_str!("fixtures/wire-0.2.4.hex").trim();
    data_encoding::HEXLOWER.decode(hex.as_bytes()).expect("the fixture is hex")
}

#[test]
fn values_written_by_0_2_4_still_read() {
    let bytes = written_by_0_2_4();
    assert!(postcard::from_bytes::<Sample>(&bytes).is_err(), "the full layout alone cannot read 0.2.4 values");
    let (server, profiles, messages, events, after) = wire_layout::decode::<Sample>(&bytes).expect("0.2.4 values read");
    assert_eq!(server.name, "Commy Review");
    assert_eq!(server.upload_limit_mb, Some(25));
    assert_eq!(server.pronouns.values().next().map(String::as_str), Some("she / her"));
    assert_eq!(server.sync_host, None);
    assert_eq!(profiles.len(), 2);
    assert_eq!(profiles[0].about.as_deref(), Some("hello"));
    assert_eq!(profiles[0].phone_until, Some(1_700_000_200_000));
    assert_eq!(profiles[1].username, "ben");
    assert_eq!(messages[0].content, "welcome **in**");
    assert!(matches!(&events[0], ServerEvent::MemberJoin { profile, .. } if profile.username == "ana"));
    assert!(matches!(&events[2], ServerEvent::ServerRename { name, .. } if name == "Review"));
    assert_eq!(after, 7);
}

#[test]
fn values_without_new_fields_are_written_exactly_as_0_2_4_wrote_them() {
    let bytes = written_by_0_2_4();
    let sample = wire_layout::decode::<Sample>(&bytes).unwrap();
    assert_eq!(wire_layout::encode(&sample).unwrap(), bytes);
}

#[test]
fn values_with_new_fields_keep_them() {
    let (mut server, mut profiles, messages, events, after) = wire_layout::decode::<Sample>(&written_by_0_2_4()).unwrap();
    server.sync_host = Some("https://sync.home.example".into());
    server.blocked_homes = vec!["bad.example".into()];
    server.require_home = true;
    profiles[1].home = None;
    let sample = (server, profiles, messages, events, after);
    let bytes = wire_layout::encode(&sample).unwrap();
    assert_ne!(bytes, written_by_0_2_4());
    assert_eq!(wire_layout::decode::<Sample>(&bytes).unwrap(), sample);
}
