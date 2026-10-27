#![cfg_attr(mobile, allow(dead_code))]

use crate::state::default_true;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
#[cfg(desktop)]
use tauri::{Emitter, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "overlay";
pub const CHAT_LABEL: &str = "overlay-chat";
const WIDTH: f64 = 280.0;
const HEIGHT: f64 = 420.0;
const MARGIN: i32 = 16;

pub const CORNERS: &[&str] = &["top-left", "top-right", "bottom-left", "bottom-right"];
pub const MAX_LAYOUTS: usize = 8;
pub const MAX_LAYOUT_NAME: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Spot {
    pub x: f32,
    pub y: f32,
}

impl Spot {
    fn clamped(self) -> Option<Self> {
        (self.x.is_finite() && self.y.is_finite()).then(|| Spot { x: self.x.clamp(0.0, 1.0), y: self.y.clamp(0.0, 1.0) })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub voice: Spot,
    pub chat: Spot,
}

impl Layout {
    fn clamped(self) -> Option<Self> {
        Some(Layout { voice: self.voice.clamped()?, chat: self.chat.clamped()? })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedLayout {
    pub name: String,
    #[serde(flatten)]
    pub layout: Layout,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverlaySettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_corner")]
    pub corner: String,
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    #[serde(default = "default_true")]
    pub show_chat: bool,
    #[serde(default)]
    pub show_everyone: bool,
    #[serde(default)]
    pub pictures_only: bool,
    #[serde(default = "default_toggle_hotkey")]
    pub toggle_hotkey: String,
    #[serde(default = "default_chat_hotkey")]
    pub chat_hotkey: String,
    #[serde(default)]
    pub layout: Option<Layout>,
    #[serde(default)]
    pub layouts: Vec<SavedLayout>,
}

fn default_corner() -> String {
    "top-left".to_string()
}
fn default_opacity() -> f32 {
    0.9
}
fn default_toggle_hotkey() -> String {
    "Ctrl+Shift+F10".to_string()
}
fn default_chat_hotkey() -> String {
    "Ctrl+Shift+F11".to_string()
}

impl Default for OverlaySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            corner: default_corner(),
            opacity: default_opacity(),
            show_chat: true,
            show_everyone: false,
            pictures_only: false,
            toggle_hotkey: default_toggle_hotkey(),
            chat_hotkey: default_chat_hotkey(),
            layout: None,
            layouts: Vec::new(),
        }
    }
}

impl OverlaySettings {
    pub fn sanitized(mut self) -> Self {
        if !CORNERS.contains(&self.corner.as_str()) {
            self.corner = default_corner();
        }
        self.opacity = if self.opacity.is_finite() { self.opacity.clamp(0.2, 1.0) } else { default_opacity() };
        self.layout = self.layout.and_then(Layout::clamped);
        let mut kept: Vec<SavedLayout> = Vec::new();
        for saved in self.layouts.drain(..) {
            let name: String = saved.name.trim().chars().filter(|c| !c.is_control()).take(MAX_LAYOUT_NAME).collect();
            let Some(layout) = saved.layout.clamped() else { continue };
            if name.is_empty() {
                continue;
            }
            kept.retain(|k| !k.name.eq_ignore_ascii_case(&name));
            kept.push(SavedLayout { name, layout });
        }
        let extra = kept.len().saturating_sub(MAX_LAYOUTS);
        kept.drain(..extra);
        self.layouts = kept;
        self
    }
}

#[cfg(mobile)]
pub fn valid_hotkey(_hotkey: &str) -> bool {
    false
}

#[cfg(desktop)]
pub fn valid_hotkey(hotkey: &str) -> bool {
    hotkey.parse::<tauri_plugin_global_shortcut::Shortcut>().is_ok()
}

pub fn corner_position(area: (i32, i32, u32, u32), size: (u32, u32), corner: &str) -> (i32, i32) {
    let (x, y, w, h) = area;
    let right = x + w as i32 - size.0 as i32 - MARGIN;
    let bottom = y + h as i32 - size.1 as i32 - MARGIN;
    match corner {
        "top-right" => (right, y + MARGIN),
        "bottom-left" => (x + MARGIN, bottom),
        "bottom-right" => (right, bottom),
        _ => (x + MARGIN, y + MARGIN),
    }
}

pub fn spot_position(area: (i32, i32, u32, u32), size: (u32, u32), spot: Spot) -> (i32, i32) {
    let (x, y, w, h) = area;
    let room_x = (w as i32 - size.0 as i32).max(0) as f32;
    let room_y = (h as i32 - size.1 as i32).max(0) as f32;
    (x + (spot.x * room_x).round() as i32, y + (spot.y * room_y).round() as i32)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place<'a> {
    Corner(&'a str),
    Spot(Spot),
}

pub fn places(settings: &OverlaySettings) -> (Place<'_>, Option<Place<'_>>) {
    match settings.layout {
        None => (Place::Corner(&settings.corner), None),
        Some(layout) => (Place::Spot(layout.voice), settings.show_chat.then_some(Place::Spot(layout.chat))),
    }
}

#[cfg(desktop)]
fn ensure_window(app: &AppHandle, label: &str) -> tauri::Result<tauri::WebviewWindow> {
    if let Some(window) = app.get_webview_window(label) {
        return Ok(window);
    }
    WebviewWindowBuilder::new(app, label, WebviewUrl::App("overlay.html".into()))
        .title("Commy Overlay")
        .inner_size(WIDTH, HEIGHT)
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        .focusable(false)
        .visible(false)
        .build()
}

pub fn wanted(toggled_on: bool, in_voice: bool, game_in_front: bool, previewing: bool, needs_game: bool) -> bool {
    previewing || (toggled_on && in_voice && (game_in_front || !needs_game))
}

#[cfg(desktop)]
static APPLYING: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn refresh(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(desktop)]
        let _one_at_a_time = APPLYING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (settings, visible) = wanted_now(&app);
        apply(&app, &settings, visible);
    });
}

fn wanted_now(app: &AppHandle) -> (OverlaySettings, bool) {
    use std::sync::atomic::Ordering::Relaxed;
    let state = app.state::<crate::state::App>();
    let settings = state.voice.lock().unwrap().overlay.clone();
    let visible = wanted(
        state.overlay_visible.load(Relaxed),
        state.voice_session.lock().unwrap().is_some(),
        state.game_in_front.load(Relaxed),
        state.overlay_preview.load(Relaxed),
        cfg!(windows),
    );
    (settings, visible)
}

#[cfg(mobile)]
pub fn hold_corner(_app: &AppHandle) {}

#[cfg(desktop)]
pub fn hold_corner(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _one_at_a_time = APPLYING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (settings, visible) = wanted_now(&app);
        let showing = settings.enabled && visible;
        let (voice, chat) = places(&settings);
        for (label, place) in [(LABEL, Some(voice)), (CHAT_LABEL, chat)] {
            let Some(window) = app.get_webview_window(label) else { continue };
            if !window.is_visible().unwrap_or(false) {
                continue;
            }
            let Some(place) = place.filter(|_| showing) else {
                tracing::info!("{label} was showing when it should not; hiding it");
                hide_window(&app, &window);
                continue;
            };
            let (Some(spot), Ok(at)) = (place_spot(&window, place), window.outer_position()) else { continue };
            if (at.x, at.y) != spot {
                tracing::info!("{label} drifted to {},{}; moving it back", at.x, at.y);
                show_at(&app, &window, Some(spot));
            }
        }
    });
}

#[cfg(mobile)]
fn apply(_app: &AppHandle, _settings: &OverlaySettings, _visible: bool) {}

#[cfg(desktop)]
fn apply(app: &AppHandle, settings: &OverlaySettings, visible: bool) {
    let (voice, chat) = places(settings);
    let showing = settings.enabled && visible;
    for (label, place) in [(LABEL, Some(voice)), (CHAT_LABEL, chat)] {
        match place.filter(|_| showing) {
            Some(place) => show(app, label, place, settings),
            None => {
                if let Some(window) = app.get_webview_window(label) {
                    hide_window(app, &window);
                    if app.state::<crate::state::App>().voice_session.lock().unwrap().is_none() {
                        let _ = app.emit_to(label, "overlay-reset", ());
                    }
                }
            }
        }
    }
}

#[cfg(desktop)]
fn show(app: &AppHandle, label: &str, place: Place<'_>, settings: &OverlaySettings) {
    let window = match ensure_window(app, label) {
        Ok(window) => window,
        Err(e) => {
            tracing::warn!("could not create the {label} window: {e}");
            return;
        }
    };
    let _ = window.set_ignore_cursor_events(true);
    show_at(app, &window, place_spot(&window, place));
    let _ = app.emit_to(label, "overlay-config", settings);
}

#[cfg(desktop)]
fn place_spot(window: &tauri::WebviewWindow, place: Place<'_>) -> Option<(i32, i32)> {
    let monitor = window.primary_monitor().ok().flatten().or_else(|| window.current_monitor().ok().flatten())?;
    let area = monitor.work_area();
    if area.size.width == 0 || area.size.height == 0 {
        return None;
    }
    let scale = monitor.scale_factor();
    let size = ((WIDTH * scale) as u32, (HEIGHT * scale) as u32);
    let area = (area.position.x, area.position.y, area.size.width, area.size.height);
    Some(match place {
        Place::Corner(corner) => corner_position(area, size, corner),
        Place::Spot(spot) => spot_position(area, size, spot),
    })
}

#[cfg(windows)]
fn show_at(app: &AppHandle, window: &tauri::WebviewWindow, spot: Option<(i32, i32)>) {
    let Ok(hwnd) = window.hwnd() else { return };
    let hwnd = hwnd.0 as isize;
    let _ = app.run_on_main_thread(move || {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE, HWND_TOPMOST, SWP_NOACTIVATE,
            SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_SHOWNOACTIVATE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
            WS_EX_TRANSPARENT,
        };
        let hwnd = hwnd as windows_sys::Win32::Foundation::HWND;
        let (x, y, keep) = match spot {
            Some((x, y)) => (x, y, 0),
            None => (0, 0, SWP_NOMOVE),
        };
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let wanted = (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT) as isize;
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | wanted);
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            SetWindowPos(hwnd, HWND_TOPMOST, x, y, 0, 0, keep | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
    });
}

#[cfg(windows)]
fn hide_window(app: &AppHandle, window: &tauri::WebviewWindow) {
    let Ok(hwnd) = window.hwnd() else { return };
    let hwnd = hwnd.0 as isize;
    let _ = app.run_on_main_thread(move || {
        use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
        unsafe {
            ShowWindow(hwnd as windows_sys::Win32::Foundation::HWND, SW_HIDE);
        }
    });
}

#[cfg(all(desktop, not(windows)))]
fn hide_window(_app: &AppHandle, window: &tauri::WebviewWindow) {
    let _ = window.hide();
}

#[cfg(all(desktop, not(windows)))]
fn show_at(_app: &AppHandle, window: &tauri::WebviewWindow, spot: Option<(i32, i32)>) {
    if let Some((x, y)) = spot {
        let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
    }
    let _ = window.show();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_overlay_only_shows_over_a_game_when_it_is_on() {
        assert!(!wanted(true, true, false, false, true));
        assert!(wanted(true, true, true, false, true));
        assert!(!wanted(false, true, true, false, true));
        assert!(wanted(true, true, false, false, false));
        assert!(wanted(false, false, false, true, true));
    }

    #[test]
    fn the_overlay_goes_away_after_leaving_voice() {
        assert!(!wanted(true, false, true, false, true));
        assert!(!wanted(true, false, false, false, false));
        assert!(wanted(true, false, true, true, true));
    }

    #[test]
    fn corners_stay_inside_the_work_area() {
        let area = (0, 0, 1920, 1040);
        assert_eq!(corner_position(area, (280, 420), "top-left"), (16, 16));
        assert_eq!(corner_position(area, (280, 420), "top-right"), (1920 - 280 - 16, 16));
        assert_eq!(corner_position(area, (280, 420), "bottom-right"), (1920 - 280 - 16, 1040 - 420 - 16));
        assert_eq!(corner_position((1920, 0, 2560, 1400), (280, 420), "bottom-left"), (1936, 1400 - 420 - 16));
    }

    #[test]
    fn settings_are_clamped() {
        let s = OverlaySettings { corner: "middle".into(), opacity: 7.0, ..Default::default() }.sanitized();
        assert_eq!((s.corner.as_str(), s.opacity), ("top-left", 1.0));
        let s = OverlaySettings { opacity: f32::NAN, ..Default::default() }.sanitized();
        assert_eq!(s.opacity, 0.9);
    }

    #[test]
    fn spots_spread_over_the_room_left_by_the_window() {
        let area = (0, 0, 1920, 1040);
        assert_eq!(spot_position(area, (280, 420), Spot { x: 0.0, y: 0.0 }), (0, 0));
        assert_eq!(spot_position(area, (280, 420), Spot { x: 1.0, y: 1.0 }), (1640, 620));
        assert_eq!(spot_position((1920, 0, 2560, 1400), (280, 420), Spot { x: 0.5, y: 0.5 }), (1920 + 1140, 490));
        assert_eq!(spot_position((0, 0, 200, 200), (280, 420), Spot { x: 1.0, y: 1.0 }), (0, 0));
    }

    #[test]
    fn a_layout_splits_voice_and_chat_and_corners_keep_them_together() {
        let s = OverlaySettings { corner: "bottom-right".into(), ..Default::default() };
        assert_eq!(places(&s), (Place::Corner("bottom-right"), None));
        let layout = Layout { voice: Spot { x: 0.0, y: 0.2 }, chat: Spot { x: 1.0, y: 1.0 } };
        let s = OverlaySettings { layout: Some(layout), ..Default::default() };
        assert_eq!(places(&s), (Place::Spot(layout.voice), Some(Place::Spot(layout.chat))));
        let s = OverlaySettings { show_chat: false, ..s };
        assert_eq!(places(&s), (Place::Spot(layout.voice), None));
    }

    #[test]
    fn layouts_are_clamped_named_and_capped() {
        let spot = |x, y| Spot { x, y };
        let saved = |name: &str, x| SavedLayout { name: name.into(), layout: Layout { voice: spot(x, 0.0), chat: spot(0.0, 1.0) } };
        let mut layouts: Vec<SavedLayout> = (0..12).map(|i| saved(&format!("Raid {i}"), 0.5)).collect();
        layouts.push(saved("  ", 0.1));
        layouts.push(saved("raid 11", 0.9));
        layouts.push(SavedLayout { name: "Broken".into(), layout: Layout { voice: spot(f32::NAN, 0.0), chat: spot(0.0, 0.0) } });
        layouts.push(saved(&"x".repeat(80), 0.2));
        let s = OverlaySettings { layout: Some(Layout { voice: spot(4.0, -1.0), chat: spot(0.3, 0.3) }), layouts, ..Default::default() }.sanitized();
        assert_eq!(s.layout, Some(Layout { voice: spot(1.0, 0.0), chat: spot(0.3, 0.3) }));
        assert_eq!(s.layouts.len(), MAX_LAYOUTS);
        assert!(s.layouts.iter().all(|l| !l.name.is_empty() && l.name.chars().count() <= MAX_LAYOUT_NAME && l.name != "Broken"));
        assert_eq!(s.layouts.iter().filter(|l| l.name.eq_ignore_ascii_case("raid 11")).count(), 1);
        assert_eq!(s.layouts.iter().find(|l| l.name == "raid 11").unwrap().layout.voice.x, 0.9);
        let broken = OverlaySettings { layout: Some(Layout { voice: spot(f32::INFINITY, 0.0), chat: spot(0.0, 0.0) }), ..Default::default() };
        assert_eq!(broken.sanitized().layout, None);
    }

    #[test]
    fn saved_layouts_read_back_flat() {
        let json = r#"{"layout":{"voice":{"x":0,"y":0},"chat":{"x":1,"y":1}},"layouts":[{"name":"Raid","voice":{"x":0.5,"y":0},"chat":{"x":0.5,"y":1}}]}"#;
        let s: OverlaySettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.layouts[0].layout.chat, Spot { x: 0.5, y: 1.0 });
        assert!(serde_json::to_string(&s).unwrap().contains(r#""name":"Raid","voice""#));
    }

    #[test]
    fn hotkeys_parse() {
        assert!(valid_hotkey("Ctrl+Shift+O"));
        assert!(valid_hotkey("Alt+F9"));
        assert!(!valid_hotkey("Ctrl+Shift+"));
        assert!(!valid_hotkey("Banana+Q"));
    }

    #[test]
    fn defaults_from_empty_json() {
        let s: OverlaySettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, OverlaySettings::default());
        assert!(!s.show_everyone);
    }

    #[test]
    fn older_settings_files_keep_the_talking_only_view() {
        let s: OverlaySettings = serde_json::from_str(r#"{"enabled":true,"corner":"bottom-right","opacity":0.8}"#).unwrap();
        assert!(!s.show_everyone && !s.pictures_only && s.show_chat);
        assert_eq!(s.corner, "bottom-right");
    }
}
