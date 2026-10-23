#![cfg_attr(mobile, allow(dead_code))]

use image::{imageops, Rgba, RgbaImage};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub const CLASSIC: &str = "classic";
pub const CUSTOM: &str = "custom";
pub const FOLLOW_APP: &str = "app";
pub const PRESETS: &[&str] = &[CLASSIC, "amber", "mint", "ocean", "violet", "mono"];
pub const TARGETS: &[&str] = &["app", "tray"];
pub const TILE_COLOURS: &[(&str, &str)] = &[
    ("amber", "#e08a1e"),
    ("mint", "#1f9e6e"),
    ("ocean", "#2a6fd1"),
    ("violet", "#7448d4"),
    ("mono", "#33363d"),
];

const CUSTOM_SIZE: u32 = 256;
const MARK_PNG: &[u8] = include_bytes!("../../ui/images/commy_v3_noback.png");

static CURRENT: RwLock<Option<(RgbaImage, RgbaImage)>> = RwLock::new(None);

fn original() -> Option<&'static RgbaImage> {
    static BASE: std::sync::OnceLock<Option<RgbaImage>> = std::sync::OnceLock::new();
    BASE.get_or_init(|| image::load_from_memory(crate::tray::ICON_PNG).ok().map(|i| i.to_rgba8())).as_ref()
}

fn mark() -> Option<&'static RgbaImage> {
    static MARK: std::sync::OnceLock<Option<RgbaImage>> = std::sync::OnceLock::new();
    MARK.get_or_init(|| {
        let base = original()?;
        let decoded = image::load_from_memory(MARK_PNG).ok()?.to_rgba8();
        Some(imageops::resize(&decoded, base.width(), base.height(), imageops::FilterType::Lanczos3))
    })
    .as_ref()
}

fn parse_hex(hex: &str) -> Option<[u8; 3]> {
    let digits = hex.strip_prefix('#').filter(|d| d.len() == 6 && d.is_ascii())?;
    let channel = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

fn tile_colour(name: &str) -> Option<[u8; 3]> {
    TILE_COLOURS.iter().find(|(preset, _)| *preset == name).and_then(|(_, hex)| parse_hex(hex))
}

const CLASSIC_RGB: [u8; 3] = [239, 68, 60];
const STAR_FIT: f32 = 0.7;

fn star_geometry(star: &RgbaImage) -> (f32, f32, f32) {
    let solid: Vec<(f32, f32)> = star
        .enumerate_pixels()
        .filter(|(_, _, p)| p[3] >= 128)
        .map(|(x, y, _)| (x as f32 + 0.5, y as f32 + 0.5))
        .collect();
    let n = solid.len().max(1) as f32;
    let cx = solid.iter().map(|p| p.0).sum::<f32>() / n;
    let cy = solid.iter().map(|p| p.1).sum::<f32>() / n;
    let reach = solid.iter().map(|p| ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()).fold(0.0, f32::max);
    (cx, cy, reach)
}

fn round_tile(rgb: [u8; 3]) -> Option<RgbaImage> {
    let star = mark()?;
    let (w, h) = star.dimensions();
    let (cx, cy, reach) = star_geometry(star);
    let size = w.min(h) as f32;
    let disc = size / 2.0;
    let scale = disc * STAR_FIT / reach;
    let scaled = imageops::resize(star, (w as f32 * scale) as u32, (h as f32 * scale) as u32, imageops::FilterType::Lanczos3);
    let mut tile = RgbaImage::from_pixel(w, h, Rgba([rgb[0], rgb[1], rgb[2], 255]));
    imageops::overlay(&mut tile, &scaled, (disc - cx * scale).round() as i64, (disc - cy * scale).round() as i64);
    for (x, y, px) in tile.enumerate_pixels_mut() {
        let d = ((x as f32 + 0.5 - disc).powi(2) + (y as f32 + 0.5 - disc).powi(2)).sqrt();
        px[3] = (255.0 * (disc - d).clamp(0.0, 1.0)).round() as u8;
    }
    Some(tile)
}

pub fn preset(name: &str) -> Option<RgbaImage> {
    if name == CLASSIC {
        return round_tile(CLASSIC_RGB);
    }
    tile_colour(name).and_then(round_tile)
}

pub fn custom_path(dir: &Path, target: &str) -> PathBuf {
    dir.join(format!("custom-{target}-icon.png"))
}

pub fn valid_choice(target: &str, choice: &str) -> bool {
    choice == CUSTOM || PRESETS.contains(&choice) || (target == "tray" && choice == FOLLOW_APP)
}

pub fn save_custom(dir: &Path, target: &str, bytes: &[u8]) -> Result<(), String> {
    if !TARGETS.contains(&target) {
        return Err("unknown icon".into());
    }
    let decoded = image::load_from_memory(bytes).map_err(|_| "choose a PNG image".to_string())?.to_rgba8();
    let (w, h) = decoded.dimensions();
    if w < 16 || h < 16 {
        return Err("that image is too small for an icon".into());
    }
    let side = w.min(h);
    let square = imageops::crop_imm(&decoded, (w - side) / 2, (h - side) / 2, side, side).to_image();
    let icon = imageops::resize(&square, CUSTOM_SIZE, CUSTOM_SIZE, imageops::FilterType::Lanczos3);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    icon.save(custom_path(dir, target)).map_err(|e| e.to_string())
}

fn load(dir: Option<&Path>, target: &str, choice: &str) -> Option<RgbaImage> {
    if choice == CUSTOM {
        if let Some(custom) = dir.and_then(|d| image::open(custom_path(d, target)).ok()) {
            return Some(custom.to_rgba8());
        }
    }
    preset(choice).or_else(|| preset(CLASSIC))
}

pub fn tray_image() -> Option<RgbaImage> {
    CURRENT.read().ok()?.as_ref().map(|(_, tray)| tray.clone())
}

pub fn apply(handle: &tauri::AppHandle, dir: Option<&Path>, app_choice: &str, tray_choice: &str) {
    let Some(app_icon) = load(dir, "app", app_choice) else { return };
    let tray_icon = if tray_choice == FOLLOW_APP { Some(app_icon.clone()) } else { load(dir, "tray", tray_choice) };
    let Some(tray_icon) = tray_icon else { return };
    if let Ok(mut current) = CURRENT.write() {
        *current = Some((app_icon.clone(), tray_icon));
    }
    #[cfg(desktop)]
    if let Some(window) = tauri::Manager::get_webview_window(handle, "main") {
        let (w, h) = app_icon.dimensions();
        let _ = window.set_icon(tauri::image::Image::new_owned(app_icon.into_raw(), w, h));
    }
    crate::tray::refresh_icon(handle);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("commy-icon-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn every_preset_builds_as_a_round_tile_with_the_star_centred() {
        let classic = preset(CLASSIC).unwrap();
        let (w, h) = classic.dimensions();
        for name in PRESETS {
            let icon = preset(name).unwrap();
            assert_eq!(icon.dimensions(), classic.dimensions());
            assert_eq!(icon.get_pixel(0, 0)[3], 0, "{name} corner is see-through");
            assert_eq!(icon.get_pixel(w / 2, h / 2)[3], 255, "{name} centre is solid");
            assert_ne!(icon.get_pixel(w / 2, h / 2).0[..3], icon.get_pixel(w / 2, h / 12).0[..3], "{name} star sits at the centre");
        }
        assert!(preset("nope").is_none());
    }

    #[test]
    fn colour_tiles_share_the_same_star() {
        let amber = preset("amber").unwrap();
        let mono = preset("mono").unwrap();
        let (w, h) = amber.dimensions();
        let centre = (w / 2, h / 2);
        assert_eq!(amber.get_pixel(centre.0, centre.1).0[..3], mono.get_pixel(centre.0, centre.1).0[..3]);
    }

    #[test]
    fn every_colour_preset_has_a_tile_colour() {
        let coloured: Vec<&str> = PRESETS.iter().copied().filter(|p| *p != CLASSIC).collect();
        let named: Vec<&str> = TILE_COLOURS.iter().map(|(name, _)| *name).collect();
        assert_eq!(coloured, named);
        assert!(TILE_COLOURS.iter().all(|(_, hex)| parse_hex(hex).is_some()));
        assert_eq!(parse_hex("#e08a1e"), Some([0xe0, 0x8a, 0x1e]));
        assert_eq!(parse_hex("e08a1e"), None);
        assert_eq!(parse_hex("#e08a1"), None);
        assert_eq!(parse_hex("#zz8a1e"), None);
    }

    #[test]
    fn colour_presets_differ_from_the_classic_icon() {
        let classic = preset(CLASSIC).unwrap();
        for name in ["amber", "mint", "ocean", "violet", "mono"] {
            assert_ne!(preset(name).unwrap().as_raw(), classic.as_raw(), "{name}");
        }
    }

    #[test]
    fn only_known_choices_are_accepted() {
        assert!(valid_choice("app", "mint"));
        assert!(valid_choice("app", CUSTOM));
        assert!(valid_choice("tray", FOLLOW_APP));
        assert!(!valid_choice("app", FOLLOW_APP));
        assert!(!valid_choice("tray", "../x"));
    }

    #[test]
    fn custom_icons_are_cropped_square_and_sized() {
        let dir = temp_dir("custom");
        let wide = RgbaImage::from_pixel(600, 300, image::Rgba([200, 30, 30, 255]));
        let mut png = Vec::new();
        wide.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        save_custom(&dir, "app", &png).unwrap();
        let saved = image::open(custom_path(&dir, "app")).unwrap();
        assert_eq!((saved.width(), saved.height()), (CUSTOM_SIZE, CUSTOM_SIZE));
        assert_eq!(load(Some(&dir), "app", CUSTOM).unwrap().dimensions(), (CUSTOM_SIZE, CUSTOM_SIZE));
        assert_eq!(load(Some(&dir), "tray", CUSTOM).unwrap().dimensions(), preset(CLASSIC).unwrap().dimensions());
        assert!(save_custom(&dir, "app", b"not an image").is_err());
        assert!(save_custom(&dir, "other", &png).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
