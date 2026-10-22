#![cfg_attr(mobile, allow(dead_code))]

use tauri::image::Image;
#[cfg(desktop)]
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
#[cfg(desktop)]
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
#[cfg(desktop)]
use tauri::Emitter;
use tauri::{AppHandle, Manager};

pub const TRAY_ID: &str = "commy-tray";

pub const ICON_PNG: &[u8] = include_bytes!("../icons/icon.png");

const BADGE_RGB: [u8; 3] = [255, 77, 77];
const RING_RGB: [u8; 3] = [11, 13, 23];

pub fn draw_bubble(rgba: &mut [u8], width: u32, height: u32, cx: f32, cy: f32, radius: f32) {
    let ring = (radius * 0.14).max(1.0);
    for y in 0..height {
        for x in 0..width {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            let idx = ((y * width + x) * 4) as usize;
            if d <= radius {
                let a = (radius - d).clamp(0.0, 1.0);
                blend(&mut rgba[idx..idx + 4], BADGE_RGB, a);
            } else if d <= radius + ring {
                let a = (radius + ring - d).clamp(0.0, 1.0);
                blend(&mut rgba[idx..idx + 4], RING_RGB, a);
            }
        }
    }
}

pub fn draw_badge(rgba: &mut [u8], width: u32, height: u32) {
    let size = width.min(height) as f32;
    let radius = size * 0.3;
    let ring = radius * 0.14;
    draw_bubble(rgba, width, height, width as f32 - radius - ring, height as f32 - radius - ring, radius);
}

fn blend(px: &mut [u8], rgb: [u8; 3], alpha: f32) {
    for i in 0..3 {
        px[i] = (rgb[i] as f32 * alpha + px[i] as f32 * (1.0 - alpha)).round() as u8;
    }
    px[3] = px[3].max((alpha * 255.0) as u8);
}

static UNREAD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn base_icon() -> Option<image::RgbaImage> {
    let source = match crate::app_icon::tray_image() {
        Some(chosen) => chosen,
        None => image::load_from_memory(ICON_PNG).ok()?.to_rgba8(),
    };
    Some(image::imageops::resize(&source, 64, 64, image::imageops::FilterType::Triangle))
}

pub fn icon(unread: bool) -> Option<Image<'static>> {
    let small = base_icon()?;
    let (w, h) = small.dimensions();
    let mut rgba = small.as_raw().clone();
    if unread {
        draw_badge(&mut rgba, w, h);
    }
    Some(Image::new_owned(rgba, w, h))
}

#[cfg(windows)]
pub fn overlay() -> Image<'static> {
    let size = 32u32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let half = size as f32 / 2.0;
    draw_bubble(&mut rgba, size, size, half, half, half - 1.5);
    Image::new_owned(rgba, size, size)
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(desktop)]
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg(mobile)]
pub fn build(_app: &AppHandle) -> tauri::Result<()> {
    Ok(())
}

#[cfg(desktop)]
pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Commy", true, None::<&str>)?;
    let mute = MenuItem::with_id(app, "mute", "Mute / Unmute", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Commy", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &mute, &separator, &quit])?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Commy - running in the background")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "mute" => {
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

#[cfg(mobile)]
pub fn refresh_icon(_app: &AppHandle) {}

#[cfg(desktop)]
pub fn refresh_icon(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_icon(icon(UNREAD.load(std::sync::atomic::Ordering::Relaxed) > 0));
    }
}

pub fn set_unread(app: &AppHandle, count: u32) {
    UNREAD.store(count, std::sync::atomic::Ordering::Relaxed);
    #[cfg(mobile)]
    let _ = app;
    #[cfg(desktop)]
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_icon(icon(count > 0));
        let tip = match count {
            0 => "Commy - running in the background".to_string(),
            1 => "Commy - 1 unread".to_string(),
            n => format!("Commy - {n} unread"),
        };
        let _ = tray.set_tooltip(Some(tip));
    }
    #[cfg(desktop)]
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(windows)]
        {
            let _ = window.set_overlay_icon((count > 0).then(overlay));
        }
        #[cfg(all(desktop, not(windows)))]
        {
            let _ = window.set_badge_count((count > 0).then_some(count as i64));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    }

    #[test]
    fn badge_paints_a_solid_bubble_bottom_right_only() {
        let (w, h) = (64u32, 64u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        draw_badge(&mut rgba, w, h);
        assert_eq!(px(&rgba, w, 4, 4), [0, 0, 0, 0], "top-left untouched");
        let r = BADGE_RGB;
        assert_eq!(px(&rgba, w, w - 10, h - 10), [r[0], r[1], r[2], 255], "the bubble is solid and blank");
    }

    #[test]
    fn bundled_icon_decodes_with_and_without_badge() {
        assert!(icon(false).is_some());
        assert!(icon(true).is_some());
    }
}
