use std::ops::Range;
use std::time::Duration;

use chrono::Utc;
use commy_core::{Identity, Message, ServerId, UserId};
use commy_sync::wake::{sealed_preview, wake_day, RingTags, WakeRing, MAX_RING_TAGS};
use commy_core::dm::DmMessage;
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

pub(super) fn dm_wake_preview(identity: &Identity, keys: &StreamKeys, message: &DmMessage) -> Option<String> {
    let day = wake_day(Utc::now().timestamp_millis());
    sealed_preview(keys, day, |msg| identity.sign(msg).0, &message.from.to_string(), None, &commy_view::dms::wake_text(message), commy_view::dms::is_call(message), commy_view::dms::call_ended(message))
}

fn ring_delay() -> Duration {
    let spread = RING_DELAY_MS.end - RING_DELAY_MS.start;
    Duration::from_millis((RING_DELAY_MS.start + js_sys::Math::random() * spread) as u64)
}

impl Session {
    pub(super) fn server_wake_tags(&self, server: ServerId, message: &Message) -> (RingTags, Option<String>) {
        let mut guard = self.sync.borrow_mut();
        let Some(live) = guard.as_mut() else { return (RingTags::default(), None) };
        let now = Utc::now().timestamp_millis();
        let found = self.client.borrow().wake_whos(server, message, || live.all_rings.ring_due(server, message.channel, now));
        let Some((private, whos, text)) = found else { return (RingTags::default(), None) };
        let Some(keys) = live.book.wake_keys(server, private.then_some(message.channel)) else { return (RingTags::default(), None) };
        let today = wake_day(now);
        let preview = self.me_and_key().and_then(|(_, identity)| {
            sealed_preview(&keys, today, |msg| identity.sign(msg).0, &message.author.to_string(), Some(message.channel.to_string()), &text, false, false)
        });
        let tagged = |whos: &[String]| -> Vec<[u8; 32]> { whos.iter().map(|who| keys.wake_tag(who, today)).collect() };
        (RingTags { broad: tagged(&whos.broad), direct: tagged(&whos.direct), hush: tagged(&whos.hush), quiet: tagged(&whos.quiet) }, preview)
    }

    pub(super) fn ring_server_wake(&self, tags: RingTags, preview: Option<String>) {
        let Some(url) = self.sync.borrow().as_ref().and_then(|live| live.wake.clone()) else { return };
        if tags.is_empty() {
            return;
        }
        let host: Host = JsValue::clone(&self.host).unchecked_into();
        spawn_local(async move {
            n0_future::time::sleep(ring_delay()).await;
            for ring in tags.rings(preview) {
                let Ok(body) = serde_json::to_string(&ring) else { return };
                let _ = host.request("POST", &url, Some(body)).await;
            }
        });
    }

    pub(super) fn ring_wake(&self, tags: Vec<[u8; 32]>, preview: Option<String>) {
        let Some(url) = self.sync.borrow().as_ref().and_then(|live| live.wake.clone()) else { return };
        if tags.is_empty() {
            return;
        }
        let host: Host = JsValue::clone(&self.host).unchecked_into();
        spawn_local(async move {
            n0_future::time::sleep(ring_delay()).await;
            for chunk in tags.chunks(MAX_RING_TAGS) {
                let ring = WakeRing { tags: chunk.iter().map(|tag| Bytes(*tag)).collect(), preview: preview.clone(), ..Default::default() };
                let Ok(body) = serde_json::to_string(&ring) else { return };
                let _ = host.request("POST", &url, Some(body)).await;
            }
        });
    }
}
