use crate::state::default_true;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "overlay";
const WIDTH: f64 = 280.0;
const HEIGHT: f64 = 420.0;
const MARGIN: i32 = 16;

pub const CORNERS: &[&str] = &["top-left", "top-right", "bottom-left", "bottom-right"];

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
    #[serde(default = "default_toggle_hotkey")]
    pub toggle_hotkey: String,
    #[serde(default = "default_chat_hotkey")]
    pub chat_hotkey: String,
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
            toggle_hotkey: default_toggle_hotkey(),
            chat_hotkey: default_chat_hotkey(),
        }
    }
}

impl OverlaySettings {
    pub fn sanitized(mut self) -> Self {
        if !CORNERS.contains(&self.corner.as_str()) {
            self.corner = default_corner();
        }
        self.opacity = if self.opacity.is_finite() { self.opacity.clamp(0.2, 1.0) } else { default_opacity() };
        self
    }
}

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

fn ensure_window(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    if let Some(window) = app.get_webview_window(LABEL) {
        return Ok(window);
    }
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("overlay.html".into()))
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

pub fn wanted(toggled_on: bool, game_in_front: bool, previewing: bool, needs_game: bool) -> bool {
    previewing || (toggled_on && (game_in_front || !needs_game))
}

pub fn refresh(app: &AppHandle) {
    use std::sync::atomic::Ordering::Relaxed;
    let state = app.state::<crate::state::App>();
    let settings = state.voice.lock().unwrap().overlay.clone();
    let visible = wanted(
        state.overlay_visible.load(Relaxed),
        state.game_in_front.load(Relaxed),
        state.overlay_preview.load(Relaxed),
        cfg!(windows),
    );
    apply(app, &settings, visible);
}

pub fn apply(app: &AppHandle, settings: &OverlaySettings, visible: bool) {
    if !settings.enabled || !visible {
        if let Some(window) = app.get_webview_window(LABEL) {
            hide_window(&window);
        }
        return;
    }
    let window = match ensure_window(app) {
        Ok(window) => window,
        Err(e) => {
            tracing::warn!("could not create the overlay window: {e}");
            return;
        }
    };
    let monitor = window.primary_monitor().ok().flatten().or_else(|| window.current_monitor().ok().flatten());
    if let Some(monitor) = monitor {
        let scale = monitor.scale_factor();
        let area = monitor.work_area();
        let size = ((WIDTH * scale) as u32, (HEIGHT * scale) as u32);
        let (x, y) = corner_position(
            (area.position.x, area.position.y, area.size.width, area.size.height),
            size,
            &settings.corner,
        );
        let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
    }
    let _ = window.set_ignore_cursor_events(true);
    show_without_focus(&window);
    let _ = app.emit_to(LABEL, "overlay-config", settings);
}

#[cfg(windows)]
fn show_without_focus(window: &tauri::WebviewWindow) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE, HWND_TOPMOST, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNOACTIVATE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
    };
    let Ok(hwnd) = window.hwnd() else { return };
    let hwnd = hwnd.0 as isize;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let wanted = (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT) as isize;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | wanted);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

#[cfg(windows)]
fn hide_window(window: &tauri::WebviewWindow) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    if let Ok(hwnd) = window.hwnd() {
        unsafe {
            ShowWindow(hwnd.0 as isize, SW_HIDE);
        }
    }
}

#[cfg(not(windows))]
fn hide_window(window: &tauri::WebviewWindow) {
    let _ = window.hide();
}

#[cfg(not(windows))]
fn show_without_focus(window: &tauri::WebviewWindow) {
    let _ = window.show();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_overlay_only_shows_over_a_game_when_it_is_on() {
        assert!(!wanted(true, false, false, true));
        assert!(wanted(true, true, false, true));
        assert!(!wanted(false, true, false, true));
        assert!(wanted(true, false, false, false));
        assert!(wanted(false, false, true, true));
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
        assert!(!s.show_everyone && s.show_chat);
        assert_eq!(s.corner, "bottom-right");
    }
}
