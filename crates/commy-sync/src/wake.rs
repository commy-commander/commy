use serde::{Deserialize, Serialize};

use crate::wire::Bytes;

pub const DAY_MS: i64 = 24 * 60 * 60 * 1000;
pub const MAX_DAYS_AHEAD: i64 = 2;
pub const MAX_RING_TAGS: usize = 8;
pub const MAX_PUSH_TOKEN: usize = 4096;
pub const PUSH_GAP_SECS: u64 = 15;

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushService {
    Fcm,
}

pub fn push_target(push: &str) -> Option<(PushService, &str)> {
    let token = push.strip_prefix("fcm:")?;
    let fits = !token.is_empty() && token.len() <= MAX_PUSH_TOKEN;
    let plain = token.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-' | b'.'));
    (fits && plain).then_some((PushService::Fcm, token))
}

pub fn register_until_ok(until: i64, today: i64) -> bool {
    (today..=today + MAX_DAYS_AHEAD).contains(&until)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StreamSecret;

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
    fn only_plain_fcm_tokens_are_accepted() {
        assert_eq!(push_target("fcm:abc-DEF_123:xyz"), Some((PushService::Fcm, "abc-DEF_123:xyz")));
        assert_eq!(push_target("fcm:"), None);
        assert_eq!(push_target("apns:abc"), None);
        assert_eq!(push_target("fcm:a b"), None);
        assert_eq!(push_target("fcm:a/b"), None);
        assert_eq!(push_target(&format!("fcm:{}", "a".repeat(MAX_PUSH_TOKEN + 1))), None);
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
    }
}
