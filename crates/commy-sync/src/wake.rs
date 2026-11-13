use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use data_encoding::BASE64;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::stream::{StreamKeys, StreamSecret, NONCE_BYTES, TAG_BYTES};
use crate::wire::Bytes;

pub const DAY_MS: i64 = 24 * 60 * 60 * 1000;
pub const MAX_DAYS_AHEAD: i64 = 2;
pub const MAX_RING_TAGS: usize = 8;
pub const MAX_PUSH_TOKEN: usize = 4096;
pub const PUSH_GAP_SECS: u64 = 15;
pub const PREVIEW_PLAIN: usize = 352;
pub const MAX_PREVIEW_TEXT: usize = 200;
pub const MAX_PREVIEW_ENCODED: usize = (PREVIEW_PLAIN + NONCE_BYTES + TAG_BYTES).div_ceil(3) * 4;
const PREVIEW_AAD: &[u8] = b"commy-wake-preview/v1";
const SEAT_LABEL: &str = "commy-wake-seat/v1";
pub const SEAT_BEAT_SECS: u64 = 45;
pub const SEAT_HOLD_SECS: u64 = 120;
pub const MAX_QUIET_SECS: i64 = 7 * 24 * 60 * 60;
pub const QUIET_REFRESH_SECS: u64 = 6 * 60 * 60;
pub const MAX_URGENT_TAGS: usize = 2;
pub const URGENT_GAP_SECS: u64 = 60;
pub const URGENT_PER_HOUR: usize = 6;

pub type WakeTag = Bytes<32>;

pub fn wake_day(ms: i64) -> i64 {
    ms.div_euclid(DAY_MS)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WakeRegister {
    pub tag: WakeTag,
    pub push: String,
    pub until: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WakeForget {
    pub tag: WakeTag,
    pub push: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WakeRing {
    pub tags: Vec<WakeTag>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub urgent: bool,
}

impl WakeRing {
    pub fn urgent_ok(&self) -> bool {
        !self.urgent || self.tags.len() <= MAX_URGENT_TAGS
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WakeSeat {
    pub seat: WakeTag,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WakeQuiet {
    pub seat: WakeTag,
    pub until_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quiet {
    Off,
    Until(i64),
    UntilTurnedOff,
}

impl Quiet {
    pub fn of(do_not_disturb: bool, back_at_ms: Option<i64>) -> Self {
        match (do_not_disturb, back_at_ms) {
            (false, _) => Quiet::Off,
            (true, Some(back)) => Quiet::Until(back),
            (true, None) => Quiet::UntilTurnedOff,
        }
    }

    pub fn until_ms(self, now_ms: i64) -> i64 {
        match self {
            Quiet::Off => 0,
            Quiet::Until(back) => back,
            Quiet::UntilTurnedOff => now_ms + MAX_QUIET_SECS * 1000,
        }
    }
}

pub fn quiet_due(wanted: Quiet, sent: Option<(Quiet, i64)>, now_ms: i64) -> bool {
    match sent {
        None => true,
        Some((told, _)) if told != wanted => true,
        Some((Quiet::Off, _)) => false,
        Some((_, at)) => now_ms - at >= QUIET_REFRESH_SECS as i64 * 1000,
    }
}

pub fn quiet_until_ok(until_ms: i64, now_ms: i64) -> Option<i64> {
    (until_ms > now_ms).then(|| until_ms.min(now_ms + MAX_QUIET_SECS * 1000))
}

pub fn seat_tag(account_secret: &[u8; 32], who: &str, day: i64) -> [u8; 32] {
    StreamSecret::derive(account_secret, SEAT_LABEL, who.as_bytes()).keys().wake_tag(who, day)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preview {
    #[serde(rename = "a")]
    pub author: String,
    #[serde(rename = "c", default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(rename = "t")]
    pub text: String,
    #[serde(rename = "k", default, skip_serializing_if = "std::ops::Not::not")]
    pub call: bool,
    #[serde(rename = "e", default, skip_serializing_if = "std::ops::Not::not")]
    pub ended: bool,
}

pub fn preview_text(text: &str) -> String {
    let flat = text.split(|c: char| c.is_whitespace() || c.is_control()).filter(|word| !word.is_empty()).collect::<Vec<_>>().join(" ");
    if flat.len() <= MAX_PREVIEW_TEXT {
        return flat;
    }
    let mut short = String::new();
    for c in flat.chars() {
        if short.len() + c.len_utf8() + 3 > MAX_PREVIEW_TEXT {
            break;
        }
        short.push(c);
    }
    format!("{}...", short.trim_end())
}

pub fn seal_preview(key: &[u8; 32], preview: &Preview) -> Option<String> {
    let mut shown = preview.clone();
    let mut plain = serde_json::to_vec(&shown).ok()?;
    while plain.len() > PREVIEW_PLAIN {
        let keep = shown.text.chars().count().checked_sub(8)?;
        shown.text = shown.text.chars().take(keep).collect();
        plain = serde_json::to_vec(&shown).ok()?;
    }
    plain.resize(PREVIEW_PLAIN, b' ');
    let mut nonce = [0u8; NONCE_BYTES];
    rand::thread_rng().fill_bytes(&mut nonce);
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let sealed = cipher.encrypt(&Nonce::from(nonce), Payload { msg: &plain, aad: PREVIEW_AAD }).ok()?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&sealed);
    Some(BASE64.encode(&out))
}

pub fn sealed_preview(keys: &StreamKeys, day: i64, author: &str, channel: Option<String>, text: &str, call: bool, ended: bool) -> Option<String> {
    seal_preview(&keys.preview_key(day), &Preview { author: author.to_string(), channel, text: preview_text(text), call, ended })
}

pub fn open_preview(key: &[u8; 32], encoded: &str) -> Option<Preview> {
    let bytes = BASE64.decode(encoded.as_bytes()).ok()?;
    if bytes.len() < NONCE_BYTES + TAG_BYTES {
        return None;
    }
    let (nonce, sealed) = bytes.split_at(NONCE_BYTES);
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let plain = cipher.decrypt(&Nonce::try_from(nonce).ok()?, Payload { msg: sealed, aad: PREVIEW_AAD }).ok()?;
    serde_json::from_slice(&plain).ok()
}

pub fn preview_ok(encoded: &str) -> bool {
    encoded.len() <= MAX_PREVIEW_ENCODED && BASE64.decode(encoded.as_bytes()).is_ok_and(|b| b.len() > NONCE_BYTES + TAG_BYTES)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushService {
    Fcm,
    Apns,
    ApnsSandbox,
}

impl PushService {
    pub fn prefix(self) -> &'static str {
        match self {
            PushService::Fcm => "fcm",
            PushService::Apns => "apns",
            PushService::ApnsSandbox => "apns-dev",
        }
    }

    fn from_prefix(prefix: &str) -> Option<Self> {
        [PushService::Fcm, PushService::Apns, PushService::ApnsSandbox].into_iter().find(|service| service.prefix() == prefix)
    }

    fn token_char_ok(self, b: u8) -> bool {
        match self {
            PushService::Fcm => b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-' | b'.'),
            PushService::Apns | PushService::ApnsSandbox => b.is_ascii_hexdigit(),
        }
    }
}

pub fn push_target(push: &str) -> Option<(PushService, &str)> {
    let (prefix, token) = push.split_once(':')?;
    let service = PushService::from_prefix(prefix)?;
    let fits = !token.is_empty() && token.len() <= MAX_PUSH_TOKEN;
    let plain = token.bytes().all(|b| service.token_char_ok(b));
    (fits && plain).then_some((service, token))
}

pub fn register_until_ok(until: i64, today: i64) -> bool {
    (today..=today + MAX_DAYS_AHEAD).contains(&until)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StreamSecret;

    #[test]
    fn an_urgent_ring_names_one_person_at_most() {
        let tag = Bytes([1u8; 32]);
        let ring = |tags: usize, urgent: bool| WakeRing { tags: vec![tag.clone(); tags], preview: None, urgent };
        assert!(ring(MAX_URGENT_TAGS, true).urgent_ok());
        assert!(!ring(MAX_URGENT_TAGS + 1, true).urgent_ok(), "a whole server cannot be woken through quiet hours");
        assert!(ring(MAX_RING_TAGS, false).urgent_ok());
    }

    #[test]
    fn an_old_service_reads_an_ordinary_ring_unchanged() {
        let plain = serde_json::to_value(WakeRing { tags: vec![Bytes([2u8; 32])], preview: None, urgent: false }).unwrap();
        assert!(plain.get("urgent").is_none());
        let read: WakeRing = serde_json::from_value(serde_json::json!({ "tags": plain["tags"] })).unwrap();
        assert!(!read.urgent);
    }

    #[test]
    fn the_service_hears_about_do_not_disturb_when_it_changes_and_now_and_then_while_it_lasts() {
        let hour = 60 * 60 * 1000;
        let on = Quiet::of(true, None);
        let off = Quiet::of(false, Some(5));
        assert_eq!(off, Quiet::Off, "a return time without do not disturb means nothing");
        assert!(quiet_due(off, None, 0), "a fresh start clears whatever was left behind");
        assert!(!quiet_due(off, Some((off, 0)), 100 * hour));
        assert!(quiet_due(on, Some((off, 0)), 1));
        assert!(!quiet_due(on, Some((on, 0)), hour));
        assert!(quiet_due(on, Some((on, 0)), QUIET_REFRESH_SECS as i64 * 1000), "kept fresh while it lasts");
        assert!(quiet_due(Quiet::of(true, Some(9 * hour)), Some((on, 0)), 1), "a new return time is sent");
        assert!(quiet_due(off, Some((on, 0)), 1));
        assert_eq!(Quiet::Until(42).until_ms(0), 42);
        assert_eq!(Quiet::Off.until_ms(7), 0);
        assert_eq!(on.until_ms(7), 7 + MAX_QUIET_SECS * 1000);
    }

    #[test]
    fn quiet_hours_end_in_the_future_and_within_a_week() {
        let now = 1_000_000;
        assert_eq!(quiet_until_ok(now, now), None, "an end time already passed lifts the quiet");
        assert_eq!(quiet_until_ok(0, now), None);
        assert_eq!(quiet_until_ok(now + 60_000, now), Some(now + 60_000));
        assert_eq!(quiet_until_ok(i64::MAX, now), Some(now + MAX_QUIET_SECS * 1000));
    }

    #[test]
    fn days_start_at_midnight_utc() {
        assert_eq!(wake_day(0), 0);
        assert_eq!(wake_day(DAY_MS - 1), 0);
        assert_eq!(wake_day(DAY_MS), 1);
        assert_eq!(wake_day(-1), -1);
    }

    #[test]
    fn tags_differ_by_person_day_and_stream_and_need_the_stream_secret() {
        let keys = StreamSecret::from_bytes([7; 32]).keys();
        let other = StreamSecret::from_bytes([8; 32]).keys();
        let tag = keys.wake_tag("alice", 20_000);
        assert_eq!(tag, StreamSecret::from_bytes([7; 32]).keys().wake_tag("alice", 20_000));
        assert_ne!(tag, keys.wake_tag("bob", 20_000));
        assert_ne!(tag, keys.wake_tag("alice", 20_001));
        assert_ne!(tag, other.wake_tag("alice", 20_000));
        assert_ne!(keys.wake_tag("ab", 1), keys.wake_tag("a", 1), "lengths are bound into the tag");
        assert_ne!(&tag, &keys.read_token());
        assert_ne!(&tag, &keys.id.0);
    }

    #[test]
    fn a_seat_needs_the_account_secret_and_changes_every_day() {
        let seat = seat_tag(&[1; 32], "alice", 20_000);
        assert_eq!(seat, seat_tag(&[1; 32], "alice", 20_000));
        assert_ne!(seat, seat_tag(&[2; 32], "alice", 20_000));
        assert_ne!(seat, seat_tag(&[1; 32], "bob", 20_000));
        assert_ne!(seat, seat_tag(&[1; 32], "alice", 20_001));
        assert!(SEAT_BEAT_SECS * 2 < SEAT_HOLD_SECS, "one lost beat does not wake the phone");
        let beat = WakeSeat { seat: Bytes(seat) };
        assert_eq!(serde_json::from_str::<WakeSeat>(&serde_json::to_string(&beat).unwrap()).unwrap(), beat);
    }

    #[test]
    fn only_plain_fcm_tokens_are_accepted() {
        assert_eq!(push_target("fcm:abc-DEF_123:xyz"), Some((PushService::Fcm, "abc-DEF_123:xyz")));
        assert_eq!(push_target("fcm:"), None);
        assert_eq!(push_target("fcm:a b"), None);
        assert_eq!(push_target("fcm:a/b"), None);
        assert_eq!(push_target(&format!("fcm:{}", "a".repeat(MAX_PUSH_TOKEN + 1))), None);
    }

    #[test]
    fn apple_tokens_are_hex_and_name_their_apns_environment() {
        assert_eq!(push_target("apns:0a1B2c"), Some((PushService::Apns, "0a1B2c")));
        assert_eq!(push_target("apns-dev:ff00"), Some((PushService::ApnsSandbox, "ff00")));
        assert_eq!(push_target("apns:"), None);
        assert_eq!(push_target("apns:xyz"), None);
        assert_eq!(push_target("apns:ab:cd"), None);
        assert_eq!(push_target("other:abc"), None);
        assert_eq!(push_target("abc"), None);
        for service in [PushService::Fcm, PushService::Apns, PushService::ApnsSandbox] {
            assert_eq!(push_target(&format!("{}:abcd", service.prefix())).map(|(s, _)| s), Some(service));
        }
    }

    #[test]
    fn registrations_reach_at_most_two_days_ahead() {
        assert!(register_until_ok(100, 100));
        assert!(register_until_ok(102, 100));
        assert!(!register_until_ok(103, 100));
        assert!(!register_until_ok(99, 100));
    }

    #[test]
    fn requests_travel_as_small_json() {
        let register = WakeRegister { tag: Bytes([1; 32]), push: "fcm:t".into(), until: 5 };
        let text = serde_json::to_string(&register).unwrap();
        assert_eq!(serde_json::from_str::<WakeRegister>(&text).unwrap(), register);
        let ring: WakeRing = serde_json::from_str(r#"{"tags":["AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE"]}"#).unwrap();
        assert_eq!(ring.tags, vec![Bytes([1; 32])]);
        assert_eq!(ring.preview, None, "rings from older apps carry no preview");
        assert!(!serde_json::to_string(&ring).unwrap().contains("preview"));
    }

    #[test]
    fn a_preview_opens_only_with_its_stream_and_day() {
        let keys = StreamSecret::from_bytes([7; 32]).keys();
        let other = StreamSecret::from_bytes([8; 32]).keys();
        let preview = Preview { author: "alice".into(), channel: Some("general".into()), text: "hello there".into(), call: false, ended: false };
        let sealed = seal_preview(&keys.preview_key(20_000), &preview).unwrap();
        assert!(preview_ok(&sealed));
        assert_eq!(open_preview(&keys.preview_key(20_000), &sealed), Some(preview.clone()));
        assert_eq!(open_preview(&keys.preview_key(20_001), &sealed), None);
        assert_eq!(open_preview(&other.preview_key(20_000), &sealed), None);
        assert_ne!(keys.preview_key(20_000), keys.wake_tag("", 20_000), "the preview key is not a wake tag");
        let again = seal_preview(&keys.preview_key(20_000), &preview).unwrap();
        assert_ne!(sealed, again, "every preview gets a fresh nonce");
    }

    #[test]
    fn every_preview_is_the_same_size_whatever_it_says() {
        let key = StreamSecret::from_bytes([7; 32]).keys().preview_key(1);
        let short = seal_preview(&key, &Preview { author: "a".into(), channel: None, text: "hi".into(), call: true, ended: false }).unwrap();
        let long = seal_preview(&key, &Preview { author: "a".into(), channel: None, text: "\"\u{1}".repeat(400), call: false, ended: false }).unwrap();
        assert_eq!(short.len(), long.len());
        assert!(short.len() <= MAX_PREVIEW_ENCODED);
        assert!(!preview_ok(&"A".repeat(MAX_PREVIEW_ENCODED + 4)));
        assert!(!preview_ok("not base64!"));
        assert!(open_preview(&key, &long).is_some_and(|p| !p.call));
        assert!(open_preview(&key, &short).is_some_and(|p| p.call && !p.ended), "a call says so");
        let missed = seal_preview(&key, &Preview { author: "a".into(), channel: None, text: "Missed call".into(), call: true, ended: true }).unwrap();
        assert_eq!(missed.len(), short.len());
        assert!(open_preview(&key, &missed).is_some_and(|p| p.call && p.ended), "a call that stopped ringing says so");
    }

    #[test]
    fn preview_text_is_one_short_line() {
        assert_eq!(preview_text("  hello\n\tthere  "), "hello there");
        let long = preview_text(&"word ".repeat(100));
        assert!(long.len() <= MAX_PREVIEW_TEXT);
        assert!(long.ends_with("..."));
        let wide = preview_text(&"é".repeat(300));
        assert!(wide.len() <= MAX_PREVIEW_TEXT && wide.ends_with("..."));
    }
}
