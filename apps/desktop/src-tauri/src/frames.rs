use std::collections::HashMap;
use std::sync::Mutex;

use commy_core::UserId;

#[derive(Default)]
pub struct FrameStore {
    latest: Mutex<HashMap<String, Vec<u8>>>,
}

impl FrameStore {
    pub fn put(&self, key: String, jpeg: Vec<u8>) {
        self.latest.lock().unwrap().insert(key, jpeg);
    }

    pub fn take(&self, key: &str) -> Option<Vec<u8>> {
        self.latest.lock().unwrap().remove(key)
    }
}

pub fn screen_key(user: UserId) -> String {
    format!("screen:{user}")
}

pub fn camera_key(user: UserId) -> String {
    format!("camera:{user}")
}

pub fn ready_payload(user: UserId, key: &str, seq: Option<u64>, width: u32, height: u32) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "user": user.to_string(),
        "key": key,
        "width": width,
        "height": height,
    });
    if let Some(seq) = seq {
        payload["seq"] = serde_json::json!(seq);
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_newest_frame_waits_and_each_is_taken_once() {
        let store = FrameStore::default();
        store.put("screen:a".into(), vec![1]);
        store.put("screen:a".into(), vec![2]);
        store.put("camera:a".into(), vec![9]);
        assert_eq!(store.take("screen:a"), Some(vec![2]));
        assert_eq!(store.take("screen:a"), None);
        assert_eq!(store.take("camera:a"), Some(vec![9]));
    }

    #[test]
    fn the_ready_event_carries_no_picture() {
        let user = UserId::new();
        let payload = ready_payload(user, &screen_key(user), Some(7), 1600, 900);
        assert!(payload.get("jpeg").is_none());
        assert_eq!(payload["key"], format!("screen:{user}"));
        assert_eq!(payload["seq"], 7);
        assert_eq!(camera_key(user), format!("camera:{user}"));
    }
}
