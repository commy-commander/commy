use image::RgbaImage;
use rayon::prelude::*;

const HALF_FLOAT_VALUES: usize = 1 << 16;
const SCRGB_BYTES_PER_PIXEL: usize = 8;

pub struct ToneMap {
    white: f32,
    levels: Box<[u8; HALF_FLOAT_VALUES]>,
}

impl ToneMap {
    pub fn new(white: f32) -> Self {
        let white = if white.is_finite() && white > 0.0 { white } else { 1.0 };
        let mut levels = Box::new([0u8; HALF_FLOAT_VALUES]);
        for (bits, level) in levels.iter_mut().enumerate() {
            *level = encode_srgb(half_to_f32(bits as u16) / white);
        }
        Self { white, levels }
    }

    pub fn white(&self) -> f32 {
        self.white
    }

    pub fn to_rgba8(&self, width: u32, height: u32, scrgb: &[u8]) -> Option<RgbaImage> {
        let pixels = width as usize * height as usize;
        if pixels == 0 || scrgb.len() != pixels * SCRGB_BYTES_PER_PIXEL {
            return None;
        }
        let row = width as usize;
        let mut rgba = vec![u8::MAX; pixels * 4];
        rgba.par_chunks_mut(row * 4)
            .zip(scrgb.par_chunks(row * SCRGB_BYTES_PER_PIXEL))
            .for_each(|(out, scrgb)| self.map_row(out, scrgb));
        RgbaImage::from_raw(width, height, rgba)
    }

    fn map_row(&self, out: &mut [u8], scrgb: &[u8]) {
        for (out, pixel) in out.chunks_exact_mut(4).zip(scrgb.chunks_exact(SCRGB_BYTES_PER_PIXEL)) {
            out[0] = self.level(pixel[0], pixel[1]);
            out[1] = self.level(pixel[2], pixel[3]);
            out[2] = self.level(pixel[4], pixel[5]);
        }
    }

    fn level(&self, low: u8, high: u8) -> u8 {
        self.levels[usize::from(u16::from_le_bytes([low, high]))]
    }
}

fn encode_srgb(linear: f32) -> u8 {
    let linear = if linear.is_nan() { 0.0 } else { linear.clamp(0.0, 1.0) };
    let encoded = if linear <= 0.003_130_8 { linear * 12.92 } else { 1.055 * linear.powf(1.0 / 2.4) - 0.055 };
    (encoded * 255.0).round() as u8
}

fn half_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let mantissa = f32::from(bits & 0x3ff);
    let magnitude = match exponent {
        0 => mantissa * 2f32.powi(-24),
        0x1f if mantissa == 0.0 => f32::INFINITY,
        0x1f => f32::NAN,
        _ => (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15),
    };
    sign * magnitude
}

#[cfg(test)]
mod tests {
    use super::*;

    fn half(value: f32) -> u16 {
        (0..=u16::MAX)
            .min_by(|a, b| (half_to_f32(*a) - value).abs().total_cmp(&(half_to_f32(*b) - value).abs()))
            .unwrap()
    }

    fn scrgb(pixels: &[[f32; 3]]) -> Vec<u8> {
        pixels
            .iter()
            .flat_map(|rgb| rgb.iter().copied().chain([1.0]))
            .flat_map(|channel| half(channel).to_le_bytes())
            .collect()
    }

    fn linear(srgb: u8) -> f32 {
        let encoded = f32::from(srgb) / 255.0;
        if encoded <= 0.04045 { encoded / 12.92 } else { ((encoded + 0.055) / 1.055).powf(2.4) }
    }

    #[test]
    fn half_floats_decode() {
        assert_eq!(half_to_f32(0x3c00), 1.0);
        assert_eq!(half_to_f32(0x4000), 2.0);
        assert_eq!(half_to_f32(0x3800), 0.5);
        assert_eq!(half_to_f32(0xbc00), -1.0);
        assert_eq!(half_to_f32(0x0001), 2f32.powi(-24));
        assert_eq!(half_to_f32(0x7c00), f32::INFINITY);
        assert!(half_to_f32(0x7e00).is_nan());
    }

    #[test]
    fn sdr_white_on_an_hdr_desktop_becomes_full_white() {
        let white = 2.5;
        let image = ToneMap::new(white).to_rgba8(1, 1, &scrgb(&[[white, white, white]])).unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [255, 255, 255, 255]);
    }

    #[test]
    fn sdr_colours_keep_their_values_after_scaling() {
        let white = 3.0;
        let map = ToneMap::new(white);
        for srgb in [0u8, 1, 16, 64, 128, 200, 254, 255] {
            let value = linear(srgb) * white;
            let image = map.to_rgba8(1, 1, &scrgb(&[[value, value, value]])).unwrap();
            let got = image.get_pixel(0, 0).0[0];
            assert!(got.abs_diff(srgb) <= 1, "{srgb} came back as {got}");
        }
    }

    #[test]
    fn an_sdr_desktop_passes_through() {
        let map = ToneMap::new(1.0);
        let grey = linear(128);
        let image = map.to_rgba8(1, 1, &scrgb(&[[grey, grey, grey]])).unwrap();
        assert!(image.get_pixel(0, 0).0[0].abs_diff(128) <= 1);
    }

    #[test]
    fn highlights_above_sdr_white_clip_and_negatives_go_black() {
        let image = ToneMap::new(2.0).to_rgba8(2, 1, &scrgb(&[[12.0, 4.0, 2.0], [-0.5, 0.0, -3.0]])).unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [255, 255, 255, 255]);
        assert_eq!(image.get_pixel(1, 0).0, [0, 0, 0, 255]);
    }

    #[test]
    fn a_frame_with_the_wrong_size_is_rejected() {
        assert!(ToneMap::new(1.0).to_rgba8(2, 2, &scrgb(&[[1.0, 1.0, 1.0]])).is_none());
        assert!(ToneMap::new(1.0).to_rgba8(0, 4, &[]).is_none());
    }

    #[test]
    fn a_bad_white_level_falls_back_to_one() {
        assert_eq!(ToneMap::new(0.0).white(), 1.0);
        assert_eq!(ToneMap::new(f32::NAN).white(), 1.0);
    }
}
