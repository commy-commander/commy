use std::ops::Range;
use std::time::Duration;

use chrono::Utc;
use commy_core::{Message, ServerId, UserId};
use commy_sync::wake::{wake_day, WakeRing, MAX_RING_TAGS};
use commy_sync::wire::Bytes;
use commy_sync::StreamKeys;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::spawn_local;

use super::Session;
use crate::host::Host;

const RING_DELAY_MS: Range<f64> = 1_000.0..4_000.0;

pub(super) fn dm_wake_tag(keys: &StreamKeys, partner: UserId) -> [u8; 32] {
    keys.wake_tag(&partner.to_string(), wake_day(Utc::now().timestamp_millis()))
}

fn ring_delay() -> Duration {
    let spread = RING_DELAY_MS.end - RING_DELAY_MS.start;
    Duration::from_millis((RING_DELAY_MS.start + js_sys::Math::random() * spread) as u64)
}

impl Session {
    pub(super) fn server_wake_tags(&self, server: ServerId, message: &Message) -> Vec<[u8; 32]> {
        let mut guard = self.sync.borrow_mut();
        let Some(live) = guard.as_mut() else { return Vec::new() };
        let now = Utc::now().timestamp_millis();
        let found = self.client.borrow().wake_whos(server, message, || live.all_rings.due(server, now));
        let Some((private, whos)) = found else { return Vec::new() };
        let Some(keys) = live.book.wake_keys(server, private.then_some(message.channel)) else { return Vec::new() };
        let today = wake_day(now);
        whos.iter().map(|who| keys.wake_tag(who, today)).collect()
    }

    pub(super) fn ring_wake(&self, tags: Vec<[u8; 32]>) {
        let Some(url) = self.sync.borrow().as_ref().and_then(|live| live.wake.clone()) else { return };
        if tags.is_empty() {
            return;
        }
        let host: Host = JsValue::clone(&self.host).unchecked_into();
        spawn_local(async move {
            n0_future::time::sleep(ring_delay()).await;
            for chunk in tags.chunks(MAX_RING_TAGS) {
                let ring = WakeRing { tags: chunk.iter().map(|tag| Bytes(*tag)).collect() };
                let Ok(body) = serde_json::to_string(&ring) else { return };
                let _ = host.request("POST", &url, Some(body)).await;
            }
        });
    }
}
