use image::{imageops, RgbaImage};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub const CLASSIC: &str = "classic";
pub const CUSTOM: &str = "custom";
pub const FOLLOW_APP: &str = "app";
pub const PRESETS: &[&str] = &[CLASSIC, "amber", "mint", "ocean", "violet", "mono"];
pub const TARGETS: &[&str] = &["app", "tray"];

const CUSTOM_SIZE: u32 = 256;

static CURRENT: RwLock<Option<(RgbaImage, RgbaImage)>> = RwLock::new(None);

fn original() -> Option<&'static RgbaImage> {
    static BASE: std::sync::OnceLock<Option<RgbaImage>> = std::sync::OnceLock::new();
    BASE.get_or_init(|| image::load_from_memory(crate::tray::ICON_PNG).ok().map(|i| i.to_rgba8())).as_ref()
}

fn hue_degrees(name: &str) -> Option<i32> {
    match name {
        "amber" => Some(40),
        "mint" => Some(130),
        "ocean" => Some(200),
        "violet" => Some(270),
        _ => None,
    }
}

pub fn preset(name: &str) -> Option<RgbaImage> {
    let base = original()?;
    match name {
        CLASSIC => Some(base.clone()),
        "mono" => {
            let mut grey = base.clone();
            for px in grey.pixels_mut() {
                let luma = ((px[0] as u32 * 299 + px[1] as u32 * 587 + px[2] as u32 * 114) / 1000) as u8;
                px[0] = luma;
                px[1] = luma;
                px[2] = luma;
            }
            Some(grey)
        }
        other => hue_degrees(other).map(|degrees| imageops::huerotate(base, degrees)),
    }
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
    fn every_preset_builds_and_keeps_the_shape() {
        let classic = preset(CLASSIC).unwrap();
        for name in PRESETS {
            let icon = preset(name).unwrap();
            assert_eq!(icon.dimensions(), classic.dimensions());
            let opaque = |i: &RgbaImage| i.pixels().filter(|p| p[3] > 0).count();
            assert_eq!(opaque(&icon), opaque(&classic), "{name} keeps the same silhouette");
        }
        assert!(preset("nope").is_none());
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
