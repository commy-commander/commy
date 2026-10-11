//! System tray icon, unread badge and desktop notifications.

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

pub const TRAY_ID: &str = "commy-tray";

pub const ICON_PNG: &[u8] = include_bytes!("../icons/icon.png");

/// Bright yellow: stands out against the red logo and the dark taskbar.
const BADGE_RGB: [u8; 3] = [255, 200, 61];

/// Unread dot in the bottom-right corner, drawn into a copy of
/// RGBA pixels. Pure so it can be tested without a display.
pub fn draw_badge(rgba: &mut [u8], width: u32, height: u32) {
    let size = width.min(height) as f32;
    let radius = size * 0.24;
    let ring = size * 0.05;
    let cx = width as f32 - radius - ring;
    let cy = height as f32 - radius - ring;
    for y in 0..height {
        for x in 0..width {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            let idx = ((y * width + x) * 4) as usize;
            if d <= radius {
                // Anti-aliased edge for small tray sizes.
                let a = (radius - d).clamp(0.0, 1.0);
                blend(&mut rgba[idx..idx + 4], BADGE_RGB, a);
            } else if d <= radius + ring {
                // Dark ring separates the dot from the artwork.
                let a = (radius + ring - d).clamp(0.0, 1.0);
                blend(&mut rgba[idx..idx + 4], [30, 31, 34], a);
            }
        }
    }
}

fn blend(px: &mut [u8], rgb: [u8; 3], alpha: f32) {
    for i in 0..3 {
        px[i] = (rgb[i] as f32 * alpha + px[i] as f32 * (1.0 - alpha)).round() as u8;
    }
    px[3] = px[3].max((alpha * 255.0) as u8);
}

/// The app icon, optionally with the unread badge.
pub fn icon(badged: bool) -> Option<Image<'static>> {
    let decoded = image::load_from_memory(ICON_PNG).ok()?.to_rgba8();
    let small = image::imageops::resize(&decoded, 64, 64, image::imageops::FilterType::Triangle);
    let (w, h) = small.dimensions();
    let mut rgba = small.into_raw();
    if badged {
        draw_badge(&mut rgba, w, h);
    }
    Some(Image::new_owned(rgba, w, h))
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Builds the tray icon with Open, Mute and Quit.
pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Commy", true, None::<&str>)?;
    let mute = MenuItem::with_id(app, "mute", "Mute / Unmute", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Commy", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &mute, &separator, &quit])?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Commy")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "mute" => {
                // The webview owns the mute flow (sound, UI, persistence).
                let _ = app.emit("tray-toggle-mute", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = icon(false) {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

/// Updates the tray tooltip, tray badge and Windows taskbar overlay.
pub fn set_unread(app: &AppHandle, count: u32) {
    let badged = count > 0;
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_icon(icon(badged));
        let tip = match count {
            0 => "Commy".to_string(),
            1 => "Commy - 1 unread".to_string(),
            n => format!("Commy - {n} unread"),
        };
        let _ = tray.set_tooltip(Some(tip));
    }
    #[cfg(windows)]
    if let Some(window) = app.get_webview_window("main") {
        let overlay = badged.then(|| {
            let mut rgba = vec![0u8; 32 * 32 * 4];
            draw_badge(&mut rgba, 32, 32);
            Image::new_owned(rgba, 32, 32)
        });
        let _ = window.set_overlay_icon(overlay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_paints_bottom_right_only() {
        let (w, h) = (64u32, 64u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        draw_badge(&mut rgba, w, h);
        let px = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
        };
        assert_eq!(px(4, 4), [0, 0, 0, 0], "top-left untouched");
        let center = px(w - 18, h - 18);
        assert_eq!(center, [BADGE_RGB[0], BADGE_RGB[1], BADGE_RGB[2], 255], "badge center is solid");
    }

    #[test]
    fn bundled_icon_decodes_with_and_without_badge() {
        assert!(icon(false).is_some());
        assert!(icon(true).is_some());
    }
}
