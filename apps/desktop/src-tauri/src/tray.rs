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

const BADGE_RGB: [u8; 3] = [242, 239, 230];
const FIGURE_RGB: [u8; 3] = [11, 13, 23];
const RING_RGB: [u8; 3] = [11, 13, 23];

const GLYPH_WIDTH: u32 = 3;
const GLYPH_HEIGHT: u32 = 5;
const GLYPH_ADVANCE: u32 = GLYPH_WIDTH + 1;
const GLYPH_LEFTMOST_BIT: u8 = 1 << (GLYPH_WIDTH - 1);
type GlyphRows = [u8; GLYPH_HEIGHT as usize];
const PLUS_GLYPH: GlyphRows = [0b000, 0b010, 0b111, 0b010, 0b000];

fn glyph(c: char) -> GlyphRows {
    match c {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b001, 0b001, 0b001],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        _ => PLUS_GLYPH,
    }
}

pub fn badge_label(count: u32) -> String {
    if count > 99 {
        "99+".to_string()
    } else {
        count.to_string()
    }
}

pub fn draw_count_badge(rgba: &mut [u8], width: u32, height: u32, cx: f32, cy: f32, radius: f32, count: u32) {
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
    let label = badge_label(count);
    let (scale, total_w, total_h) = figure_layout(radius, label.chars().count() as u32);
    let x0 = (cx - total_w as f32 / 2.0).round() as i64;
    let y0 = (cy - total_h as f32 / 2.0).round() as i64;
    for (i, c) in label.chars().enumerate() {
        let rows = glyph(c);
        let gx = x0 + (i as i64) * GLYPH_ADVANCE as i64 * scale as i64;
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..GLYPH_WIDTH {
                if bits & (GLYPH_LEFTMOST_BIT >> col) == 0 {
                    continue;
                }
                for sy in 0..scale as i64 {
                    for sx in 0..scale as i64 {
                        let px = gx + col as i64 * scale as i64 + sx;
                        let py = y0 + row as i64 * scale as i64 + sy;
                        if px < 0 || py < 0 || px >= width as i64 || py >= height as i64 {
                            continue;
                        }
                        let idx = ((py as u32 * width + px as u32) * 4) as usize;
                        blend(&mut rgba[idx..idx + 4], FIGURE_RGB, 1.0);
                    }
                }
            }
        }
    }
}

fn figure_layout(radius: f32, glyphs: u32) -> (u32, u32, u32) {
    let text_w = glyphs * GLYPH_WIDTH + glyphs.saturating_sub(1);
    let fit_width = (radius * 2.0 * 0.78) / text_w as f32;
    let fit_height = (radius * 2.0 * 0.66) / GLYPH_HEIGHT as f32;
    let scale = fit_width.min(fit_height).floor().max(1.0) as u32;
    (scale, text_w * scale, GLYPH_HEIGHT * scale)
}

pub fn draw_badge(rgba: &mut [u8], width: u32, height: u32, count: u32) {
    let size = width.min(height) as f32;
    let radius = size * 0.27;
    let ring = radius * 0.14;
    draw_count_badge(rgba, width, height, width as f32 - radius - ring, height as f32 - radius - ring, radius, count);
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

pub fn icon(count: u32) -> Option<Image<'static>> {
    let small = base_icon()?;
    let (w, h) = small.dimensions();
    let mut rgba = small.as_raw().clone();
    if count > 0 {
        draw_badge(&mut rgba, w, h, count);
    }
    Some(Image::new_owned(rgba, w, h))
}

#[cfg(windows)]
pub fn overlay(count: u32) -> Image<'static> {
    let size = 32u32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let half = size as f32 / 2.0;
    draw_count_badge(&mut rgba, size, size, half, half, half - 1.5, count);
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
        .tooltip("Commy")
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
    if let Some(icon) = icon(0) {
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
        let _ = tray.set_icon(icon(UNREAD.load(std::sync::atomic::Ordering::Relaxed)));
    }
}

pub fn set_unread(app: &AppHandle, count: u32) {
    UNREAD.store(count, std::sync::atomic::Ordering::Relaxed);
    #[cfg(mobile)]
    let _ = app;
    #[cfg(desktop)]
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_icon(icon(count));
        let tip = match count {
            0 => "Commy".to_string(),
            1 => "Commy - 1 unread".to_string(),
            n => format!("Commy - {n} unread"),
        };
        let _ = tray.set_tooltip(Some(tip));
    }
    #[cfg(desktop)]
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(windows)]
        {
            let _ = window.set_overlay_icon((count > 0).then(|| overlay(count)));
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
    fn badge_paints_bottom_right_only() {
        let (w, h) = (64u32, 64u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        draw_badge(&mut rgba, w, h, 1);
        assert_eq!(px(&rgba, w, 4, 4), [0, 0, 0, 0], "top-left untouched");
        let r = BADGE_RGB;
        assert_eq!(px(&rgba, w, w - 6, h - 18), [r[0], r[1], r[2], 255], "the disc is solid");
        let f = FIGURE_RGB;
        let inked = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).filter(|&(x, y)| px(&rgba, w, x, y) == [f[0], f[1], f[2], 255]).count();
        assert!(inked > 0, "the figure is drawn");
    }

    #[test]
    fn figures_fit_inside_the_disc() {
        for radius in [8.0f32, 14.5, 17.3, 40.0] {
            for count in [1, 7, 10, 42, 99, 100, 5000] {
                let (scale, w, h) = figure_layout(radius, badge_label(count).chars().count() as u32);
                assert!(scale >= 1);
                let corner = ((w as f32 / 2.0).powi(2) + (h as f32 / 2.0).powi(2)).sqrt();
                assert!(corner < radius, "{count} at radius {radius}: {w}x{h} pokes out of the disc");
            }
        }
    }

    #[test]
    fn label_caps_at_99() {
        assert_eq!(badge_label(1), "1");
        assert_eq!(badge_label(99), "99");
        assert_eq!(badge_label(100), "99+");
    }

    #[test]
    fn bundled_icon_decodes_with_and_without_badge() {
        assert!(icon(0).is_some());
        assert!(icon(12).is_some());
    }
}
