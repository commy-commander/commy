use anyhow::{bail, Result};
use image::RgbaImage;

use super::{ScreenFrame, ShareSource, SourceKind};

const UNAVAILABLE: &str = "screen sharing is not available on phones yet";

pub struct Capturer;

impl Capturer {
    pub fn open(_source: Option<(SourceKind, u32)>) -> Result<Self> {
        bail!(UNAVAILABLE)
    }

    pub fn grab(&mut self) -> Result<RgbaImage> {
        bail!(UNAVAILABLE)
    }
}

pub fn list_sources() -> Result<Vec<ShareSource>> {
    Ok(Vec::new())
}

pub fn capture_source(_kind: SourceKind, _id: u32, _max_width: u32, _quality: u8) -> Result<ScreenFrame> {
    bail!(UNAVAILABLE)
}
