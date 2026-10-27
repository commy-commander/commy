use std::time::{Duration, Instant};

use base64::Engine;
use commy_media::screen::ScreenFrame;

use crate::media_engine::Captured;

const RESEND_STILL_FRAME: Duration = Duration::from_secs(1);

#[derive(Debug, Default, serde::Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub seq: u64,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub jpeg: Option<String>,
    #[serde(default)]
    pub ended: bool,
}

#[derive(Default)]
pub struct Pacer {
    seen: u64,
    last: Option<(ScreenFrame, Instant)>,
    ended: bool,
}

impl Pacer {
    pub fn seen(&self) -> u64 {
        self.seen
    }

    pub fn ended(&self) -> bool {
        self.ended
    }

    pub fn next(&mut self, snapshot: Snapshot, now: Instant) -> Option<Captured> {
        if self.ended || snapshot.ended {
            self.ended = true;
            self.last = None;
            return None;
        }
        Some(self.paced(snapshot, now))
    }

    fn paced(&mut self, snapshot: Snapshot, now: Instant) -> Captured {
        if snapshot.seq > self.seen {
            if let Some(frame) = snapshot.jpeg.as_deref().and_then(|jpeg| decode(jpeg, snapshot.width, snapshot.height)) {
                self.seen = snapshot.seq;
                self.last = Some((frame.clone(), now));
                return Captured::Encoded(frame);
            }
        }
        match &mut self.last {
            Some((frame, sent)) if now.duration_since(*sent) >= RESEND_STILL_FRAME => {
                *sent = now;
                Captured::Encoded(frame.clone())
            }
            _ => Captured::Unchanged,
        }
    }
}

fn decode(jpeg: &str, width: u32, height: u32) -> Option<ScreenFrame> {
    let jpeg = base64::engine::general_purpose::STANDARD.decode(jpeg).ok()?;
    if width == 0 || height == 0 || !jpeg.starts_with(&[0xff, 0xd8, 0xff]) {
        return None;
    }
    Some(ScreenFrame { width: width.min(4096), height: height.min(4096), jpeg })
}

#[cfg(target_os = "android")]
mod android {
    use std::sync::OnceLock;

    use tauri::plugin::PluginHandle;
    use tauri::{Emitter, Wry};

    use super::{Pacer, Snapshot};
    use crate::media_engine::{Captured, FrameOpener, FrameSource};

    pub const MAX_FPS: u32 = 15;
    pub const MAX_WIDTH: u32 = 1280;

    static PLUGIN: OnceLock<PluginHandle<Wry>> = OnceLock::new();
    static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

    pub fn init() -> tauri::plugin::TauriPlugin<Wry> {
        tauri::plugin::Builder::new("screen-share")
            .setup(|app, api| {
                let plugin = api.register_android_plugin("chat.commy.mobile", "ScreenSharePlugin")?;
                let _ = PLUGIN.set(plugin);
                let _ = APP.set(app.clone());
                Ok(())
            })
            .build()
    }

    fn plugin() -> Result<&'static PluginHandle<Wry>, String> {
        PLUGIN.get().ok_or_else(|| "screen sharing is still starting up".to_string())
    }

    pub fn ask(max_width: u32, fps: u32) -> Result<(), String> {
        plugin()?
            .run_mobile_plugin::<serde_json::Value>("start", serde_json::json!({ "maxWidth": max_width, "fps": fps }))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn stop() {
        if let Some(plugin) = PLUGIN.get() {
            if let Err(e) = plugin.run_mobile_plugin::<serde_json::Value>("stop", serde_json::json!({})) {
                tracing::warn!("stopping the screen share service: {e}");
            }
        }
    }

    struct StopOnDrop;

    impl Drop for StopOnDrop {
        fn drop(&mut self) {
            stop();
        }
    }

    pub fn opener() -> FrameOpener {
        let guard = StopOnDrop;
        Box::new(move || {
            let _ = &guard;
            let plugin = plugin().map_err(anyhow::Error::msg)?.clone();
            let mut pacer = Pacer::default();
            Ok(Box::new(move || {
                if pacer.ended() {
                    return Ok(Captured::Unchanged);
                }
                let snapshot: Snapshot = plugin.run_mobile_plugin("frame", serde_json::json!({ "after": pacer.seen() }))?;
                match pacer.next(snapshot, std::time::Instant::now()) {
                    Some(captured) => Ok(captured),
                    None => {
                        if let Some(app) = APP.get() {
                            let _ = app.emit("screen-share-ended", ());
                        }
                        Ok(Captured::Unchanged)
                    }
                }
            }) as FrameSource)
        })
    }
}

#[cfg(target_os = "android")]
pub use android::{ask, init, opener, MAX_FPS, MAX_WIDTH};

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(tag: u8) -> String {
        base64::engine::general_purpose::STANDARD.encode([0xff, 0xd8, 0xff, tag])
    }

    fn snapshot(seq: u64, tag: u8) -> Snapshot {
        Snapshot { seq, width: 640, height: 360, jpeg: Some(jpeg(tag)), ended: false }
    }

    fn sent(captured: Option<Captured>) -> Option<u8> {
        match captured {
            Some(Captured::Encoded(frame)) => frame.jpeg.last().copied(),
            _ => None,
        }
    }

    #[test]
    fn new_frames_pass_through_and_a_still_screen_is_resent_once_a_second() {
        let start = Instant::now();
        let mut pacer = Pacer::default();
        assert_eq!(sent(pacer.next(Snapshot::default(), start)), None);
        assert_eq!(sent(pacer.next(snapshot(1, 1), start)), Some(1));
        assert_eq!(pacer.seen(), 1);
        assert_eq!(sent(pacer.next(Snapshot { seq: 1, ..Snapshot::default() }, start + Duration::from_millis(500))), None);
        assert_eq!(sent(pacer.next(Snapshot { seq: 1, ..Snapshot::default() }, start + RESEND_STILL_FRAME)), Some(1));
        assert_eq!(sent(pacer.next(Snapshot { seq: 1, ..Snapshot::default() }, start + RESEND_STILL_FRAME + Duration::from_millis(10))), None);
        assert_eq!(sent(pacer.next(snapshot(2, 2), start + RESEND_STILL_FRAME + Duration::from_millis(20))), Some(2));
    }

    #[test]
    fn once_android_ends_the_share_nothing_more_is_sent() {
        let now = Instant::now();
        let mut pacer = Pacer::default();
        assert_eq!(sent(pacer.next(snapshot(1, 1), now)), Some(1));
        assert!(pacer.next(Snapshot { ended: true, ..Snapshot::default() }, now).is_none());
        assert!(pacer.ended());
        assert!(pacer.next(snapshot(2, 2), now + RESEND_STILL_FRAME).is_none());
    }

    #[test]
    fn frames_that_are_not_jpeg_or_have_no_size_are_dropped() {
        let now = Instant::now();
        let mut pacer = Pacer::default();
        let png = base64::engine::general_purpose::STANDARD.encode([0x89, b'P', b'N', b'G']);
        assert_eq!(sent(pacer.next(Snapshot { seq: 1, width: 10, height: 10, jpeg: Some(png), ended: false }, now)), None);
        assert_eq!(sent(pacer.next(Snapshot { seq: 2, width: 0, height: 10, jpeg: Some(jpeg(3)), ended: false }, now)), None);
        assert_eq!(sent(pacer.next(Snapshot { seq: 3, jpeg: Some("not base64!".into()), ..snapshot(3, 0) }, now)), None);
        assert_eq!(pacer.seen(), 0);
    }
}
