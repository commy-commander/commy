#![cfg_attr(mobile, allow(dead_code))]

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

pub const LABEL: &str = "plex";
pub const PLEX_URL: &str = "https://app.plex.tv/desktop/";

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlexReport {
    pub playing: bool,
    pub position: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlexTake {
    #[serde(default)]
    pub events: Vec<PlexReport>,
    #[serde(default)]
    pub video: bool,
    #[serde(default)]
    pub playing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct PlexState {
    pub open: bool,
    pub video: bool,
    pub playing: bool,
}

pub fn parse_take(json: &str) -> PlexTake {
    serde_json::from_str::<Option<PlexTake>>(json).ok().flatten().unwrap_or_default()
}

pub fn latest_report(take: &PlexTake) -> Option<PlexReport> {
    take.events.iter().copied().rfind(|r| r.position.is_finite() && r.position >= 0.0)
}

pub fn apply_script(playing: bool) -> String {
    format!("window.__commyWatch && window.__commyWatch.apply({playing})")
}

#[cfg(desktop)]
mod desktop {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;
    use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

    const POLL: Duration = Duration::from_millis(400);
    const TAKE_SCRIPT: &str = "window.__commyWatch ? window.__commyWatch.take() : null";
    const SYNC_SCRIPT: &str = include_str!("../../ui/scripts/plex-sync.js");
    static OPENED: AtomicU64 = AtomicU64::new(0);
    static LATEST: Mutex<PlexState> = Mutex::new(PlexState { open: false, video: false, playing: false });

    fn closed() -> PlexState {
        PlexState { open: false, video: false, playing: false }
    }

    fn just_opened() -> PlexState {
        PlexState { open: true, video: false, playing: false }
    }

    fn publish(handle: &AppHandle, now: PlexState) {
        let changed = {
            let mut latest = LATEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::replace(&mut *latest, now) != now
        };
        if changed {
            let _ = handle.emit_to("main", "plex-state", now);
        }
    }

    pub fn open(handle: &AppHandle) -> Result<(), String> {
        if let Some(window) = handle.get_webview_window(LABEL) {
            let _ = window.unminimize();
            let _ = window.show();
            return window.set_focus().map_err(|e| e.to_string());
        }
        let url = PLEX_URL.parse().map_err(|e: url::ParseError| e.to_string())?;
        let window = WebviewWindowBuilder::new(handle, LABEL, WebviewUrl::External(url))
            .title("Plex")
            .inner_size(1180.0, 720.0)
            .min_inner_size(640.0, 400.0)
            .initialization_script(SYNC_SCRIPT)
            .build()
            .map_err(|e| e.to_string())?;
        let on_close = handle.clone();
        window.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                publish(&on_close, closed());
            }
        });
        publish(handle, just_opened());
        let generation = OPENED.fetch_add(1, Ordering::Relaxed) + 1;
        tauri::async_runtime::spawn(poll(handle.clone(), generation));
        Ok(())
    }

    async fn poll(handle: AppHandle, generation: u64) {
        loop {
            tokio::time::sleep(POLL).await;
            if OPENED.load(Ordering::Relaxed) != generation {
                return;
            }
            let Some(window) = handle.get_webview_window(LABEL) else { return };
            let emitter = handle.clone();
            let _ = window.eval_with_callback(TAKE_SCRIPT, move |json| {
                let take = parse_take(&json);
                if let Some(report) = latest_report(&take) {
                    let _ = emitter.emit_to("main", "plex-report", report);
                }
                if emitter.get_webview_window(LABEL).is_some() {
                    publish(&emitter, PlexState { open: true, video: take.video, playing: take.playing });
                }
            });
        }
    }

    pub fn apply(handle: &AppHandle, playing: bool) -> Result<(), String> {
        let Some(window) = handle.get_webview_window(LABEL) else { return Ok(()) };
        window.eval(apply_script(playing)).map_err(|e| e.to_string())
    }

    pub fn close(handle: &AppHandle) -> Result<(), String> {
        let Some(window) = handle.get_webview_window(LABEL) else { return Ok(()) };
        window.close().map_err(|e| e.to_string())
    }

    pub fn state(handle: &AppHandle) -> PlexState {
        if handle.get_webview_window(LABEL).is_none() {
            return closed();
        }
        *LATEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(mobile)]
mod phone {
    use super::*;

    pub const UNSUPPORTED: &str = "Plex sync needs the Commy desktop app";

    pub fn open(_handle: &AppHandle) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }

    pub fn apply(_handle: &AppHandle, _playing: bool) -> Result<(), String> {
        Ok(())
    }

    pub fn close(_handle: &AppHandle) -> Result<(), String> {
        Ok(())
    }

    pub fn state(_handle: &AppHandle) -> PlexState {
        PlexState { open: false, video: false, playing: false }
    }
}

#[cfg(desktop)]
use desktop as platform;
#[cfg(mobile)]
use phone as platform;

#[tauri::command]
pub async fn open_plex_window(handle: AppHandle) -> Result<(), String> {
    platform::open(&handle)
}

#[tauri::command]
pub fn plex_play(handle: AppHandle, playing: bool) -> Result<(), String> {
    platform::apply(&handle, playing)
}

#[tauri::command]
pub fn close_plex_window(handle: AppHandle) -> Result<(), String> {
    platform::close(&handle)
}

#[tauri::command]
pub fn get_plex_state(handle: AppHandle) -> PlexState {
    platform::state(&handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_odd_take_reads_as_nothing_happened() {
        assert_eq!(parse_take("null"), PlexTake::default());
        assert_eq!(parse_take(""), PlexTake::default());
        assert_eq!(parse_take("\"text\""), PlexTake::default());
        assert_eq!(parse_take("{}"), PlexTake::default());
    }

    #[test]
    fn only_the_last_sound_press_in_a_take_is_passed_on() {
        let take = parse_take(r#"{"events":[{"playing":false,"position":3},{"playing":true,"position":12.5},{"playing":false,"position":-1}],"video":true,"playing":true}"#);
        assert!(take.video && take.playing);
        assert_eq!(latest_report(&take), Some(PlexReport { playing: true, position: 12.5 }));
        assert_eq!(latest_report(&PlexTake::default()), None);
    }

    #[test]
    fn the_apply_script_only_carries_a_flag() {
        assert_eq!(apply_script(true), "window.__commyWatch && window.__commyWatch.apply(true)");
        assert_eq!(apply_script(false), "window.__commyWatch && window.__commyWatch.apply(false)");
    }

    #[test]
    fn plex_opens_on_its_own_site_over_https() {
        let url = url::Url::parse(PLEX_URL).unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("app.plex.tv"));
    }
}
