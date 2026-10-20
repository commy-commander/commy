use anyhow::{Context, Result};
use image::codecs::jpeg::JpegEncoder;
use image::RgbaImage;

#[derive(Debug, Clone)]
pub struct ScreenFrame {
    pub width: u32,
    pub height: u32,
    pub jpeg: Vec<u8>,
}

pub fn encode_frame(image: RgbaImage, max_width: u32, quality: u8) -> Result<ScreenFrame> {
    let image = downscale(image, max_width);
    let (width, height) = (image.width(), image.height());
    let rgb = image::DynamicImage::ImageRgba8(image).to_rgb8();
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, quality)
        .encode_image(&rgb)
        .context("jpeg encode failed")?;
    Ok(ScreenFrame {
        width,
        height,
        jpeg,
    })
}

pub fn downscale(mut image: RgbaImage, max_width: u32) -> RgbaImage {
    let max_width = max_width.max(1);
    while image.width() >= max_width * 2 && image.height() >= 2 {
        image = halve(&image);
    }
    if image.width() > max_width {
        let height = ((image.height() as u64 * max_width as u64) / image.width() as u64).max(1) as u32;
        image = image::imageops::resize(&image, max_width, height, image::imageops::FilterType::Triangle);
    }
    image
}

fn halve(src: &RgbaImage) -> RgbaImage {
    let (w, h) = (src.width() / 2, src.height() / 2);
    let stride = src.width() as usize * 4;
    let raw = src.as_raw();
    let mut out = vec![0u8; w as usize * h as usize * 4];
    for y in 0..h as usize {
        let row0 = &raw[2 * y * stride..];
        let row1 = &raw[(2 * y + 1) * stride..];
        let dst = &mut out[y * w as usize * 4..(y + 1) * w as usize * 4];
        for x in 0..w as usize {
            let i = x * 8;
            for c in 0..4 {
                let sum = row0[i + c] as u16 + row0[i + 4 + c] as u16 + row1[i + c] as u16 + row1[i + 4 + c] as u16;
                dst[x * 4 + c] = ((sum + 2) / 4) as u8;
            }
        }
    }
    RgbaImage::from_raw(w, h, out).expect("buffer sized for the image")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamQuality {
    Sharp,
    #[default]
    Balanced,
    Smooth,
}

impl StreamQuality {
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "sharp" => Self::Sharp,
            "smooth" => Self::Smooth,
            _ => Self::Balanced,
        }
    }

    pub fn fps(self) -> u32 {
        match self {
            Self::Sharp => 15,
            Self::Balanced => 30,
            Self::Smooth => 60,
        }
    }

    pub fn max_width(self) -> u32 {
        match self {
            Self::Sharp => 1920,
            Self::Balanced => 1600,
            Self::Smooth => 1280,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Monitor,
    Window,
}

#[derive(Debug, Clone)]
pub struct ShareSource {
    pub kind: SourceKind,
    pub id: u32,
    pub name: String,
    pub is_primary: bool,
    pub pid: Option<u32>,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod desktop;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub use desktop::{capture_source, list_sources, Capturer};

#[cfg(any(target_os = "android", target_os = "ios"))]
mod phone;
#[cfg(any(target_os = "android", target_os = "ios"))]
pub use phone::{capture_source, list_sources, Capturer};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn capture_speed() {
        let mut capturer = Capturer::open(None).unwrap();
        for width in [1280u32, 1600, 1920] {
            let (mut cap, mut enc) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
            for _ in 0..20 {
                let t = std::time::Instant::now();
                let image = capturer.grab().unwrap();
                cap += t.elapsed();
                let t = std::time::Instant::now();
                encode_frame(image, width, 60).unwrap();
                enc += t.elapsed();
            }
            println!("width {width}: capture {:?}, downscale and encode {:?}", cap / 20, enc / 20);
        }
    }

    #[test]
    fn halving_averages_blocks() {
        let mut img = RgbaImage::new(4, 2);
        img.put_pixel(0, 0, image::Rgba([0, 0, 0, 255]));
        img.put_pixel(1, 0, image::Rgba([100, 0, 0, 255]));
        img.put_pixel(0, 1, image::Rgba([100, 0, 0, 255]));
        img.put_pixel(1, 1, image::Rgba([200, 0, 0, 255]));
        let half = halve(&img);
        assert_eq!((half.width(), half.height()), (2, 1));
        assert_eq!(half.get_pixel(0, 0).0, [100, 0, 0, 255]);
    }

    #[test]
    fn downscale_hits_the_target_width() {
        for (w, h, max) in [(2560, 1440, 1280), (3840, 2160, 1280), (2560, 1440, 1600), (800, 600, 1280)] {
            let out = downscale(RgbaImage::new(w, h), max);
            assert_eq!(out.width(), w.min(max), "{w}x{h} to {max}");
            let expected_h = (h as u64 * out.width() as u64 / w as u64) as i64;
            assert!((out.height() as i64 - expected_h).abs() <= 1);
        }
    }

    fn gradient(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
        })
    }

    #[test]
    fn encodes_valid_jpeg() {
        let frame = encode_frame(gradient(320, 200), 1280, 70).unwrap();
        assert_eq!(frame.width, 320);
        assert_eq!(frame.height, 200);
        assert_eq!(&frame.jpeg[..2], &[0xff, 0xd8]);
        assert!(!frame.jpeg.is_empty());
    }

    #[test]
    fn downscales_wide_frames() {
        let frame = encode_frame(gradient(2560, 1440), 1280, 70).unwrap();
        assert_eq!(frame.width, 1280);
        assert_eq!(frame.height, 720);
    }

    #[test]
    fn quality_parses_known_names() {
        assert_eq!(StreamQuality::from_name("sharp"), StreamQuality::Sharp);
        assert_eq!(StreamQuality::from_name(" Smooth "), StreamQuality::Smooth);
        assert_eq!(StreamQuality::from_name("balanced"), StreamQuality::Balanced);
    }

    #[test]
    fn quality_falls_back_to_balanced() {
        assert_eq!(StreamQuality::from_name(""), StreamQuality::Balanced);
        assert_eq!(StreamQuality::from_name("ultra"), StreamQuality::Balanced);
        assert_eq!(StreamQuality::default(), StreamQuality::Balanced);
    }

    #[test]
    fn sharper_presets_trade_fps_for_width() {
        let presets = [StreamQuality::Smooth, StreamQuality::Balanced, StreamQuality::Sharp];
        for pair in presets.windows(2) {
            assert!(pair[0].max_width() < pair[1].max_width());
            assert!(pair[0].fps() > pair[1].fps());
        }
        assert_eq!((StreamQuality::Balanced.fps(), StreamQuality::Balanced.max_width()), (30, 1600));
    }

    #[test]
    fn narrow_frames_keep_native_size() {
        let frame = encode_frame(gradient(800, 600), 1280, 70).unwrap();
        assert_eq!((frame.width, frame.height), (800, 600));
    }
}
