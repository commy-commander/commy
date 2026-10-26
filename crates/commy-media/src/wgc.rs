use anyhow::{anyhow, bail, Context as _, Result};
use image::RgbaImage;
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant};
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, GraphicsCaptureItemType,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

use crate::display;
use crate::hdr::ToneMap;

#[derive(Debug, Clone, Copy)]
pub enum Target {
    Window(u32),
    Monitor(u32),
}

impl Target {
    pub fn hdr_white(self) -> Option<f32> {
        let monitor = match self {
            Self::Window(hwnd) => display::monitor_of_window(hwnd),
            Self::Monitor(hmonitor) => hmonitor,
        };
        display::hdr_white_level(monitor)
    }
}

struct RawFrame {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

struct Handler {
    frames: SyncSender<RawFrame>,
    scratch: Vec<u8>,
}

impl GraphicsCaptureApiHandler for Handler {
    type Flags = SyncSender<RawFrame>;
    type Error = anyhow::Error;

    fn new(ctx: Context<Self::Flags>) -> Result<Self> {
        Ok(Self { frames: ctx.flags, scratch: Vec::new() })
    }

    fn on_frame_arrived(&mut self, frame: &mut Frame, control: InternalCaptureControl) -> Result<()> {
        let buffer = frame.buffer_without_title_bar().context("frame buffer")?;
        let (width, height) = (buffer.width(), buffer.height());
        let pixels = buffer.as_nopadding_buffer(&mut self.scratch).to_vec();
        let frame = RawFrame { width, height, pixels };
        if let Err(std::sync::mpsc::TrySendError::Disconnected(_)) = self.frames.try_send(frame) {
            control.stop();
        }
        Ok(())
    }

    fn on_closed(&mut self) -> Result<()> {
        Ok(())
    }
}

type Control = CaptureControl<Handler, anyhow::Error>;

pub struct Capture {
    target: Target,
    control: Option<Control>,
    frames: Receiver<RawFrame>,
    tone_map: Option<ToneMap>,
    hdr_checked: Instant,
    last: Option<RgbaImage>,
}

const FIRST_FRAME_WAIT: Duration = Duration::from_secs(2);
const REPEAT_AFTER: Duration = Duration::from_millis(250);
const HDR_CHECK_EVERY: Duration = Duration::from_secs(2);

impl Capture {
    pub fn start(target: Target) -> Result<Self> {
        let tone_map = target.hdr_white().map(ToneMap::new);
        let (control, frames) = open(target, tone_map.is_some())?;
        log_hdr(tone_map.as_ref());
        Ok(Self { target, control: Some(control), frames, tone_map, hdr_checked: Instant::now(), last: None })
    }

    pub fn grab(&mut self) -> Result<RgbaImage> {
        self.follow_hdr_changes()?;
        let wait = if self.last.is_some() { REPEAT_AFTER } else { FIRST_FRAME_WAIT };
        match self.frames.recv_timeout(wait) {
            Ok(mut raw) => {
                while let Ok(newer) = self.frames.try_recv() {
                    raw = newer;
                }
                let image = self.to_image(raw)?;
                self.last = Some(image.clone());
                Ok(image)
            }
            Err(RecvTimeoutError::Timeout) => self.last.clone().context("no frame arrived from graphics capture"),
            Err(RecvTimeoutError::Disconnected) => bail!("graphics capture ended"),
        }
    }

    fn to_image(&self, raw: RawFrame) -> Result<RgbaImage> {
        match &self.tone_map {
            Some(tone_map) => tone_map.to_rgba8(raw.width, raw.height, &raw.pixels),
            None => RgbaImage::from_raw(raw.width, raw.height, raw.pixels),
        }
        .context("frame size mismatch")
    }

    fn follow_hdr_changes(&mut self) -> Result<()> {
        if self.hdr_checked.elapsed() < HDR_CHECK_EVERY {
            return Ok(());
        }
        self.hdr_checked = Instant::now();
        let white = self.target.hdr_white();
        if white == self.tone_map.as_ref().map(ToneMap::white) {
            return Ok(());
        }
        let restart = white.is_some() != self.tone_map.is_some();
        self.tone_map = white.map(ToneMap::new);
        log_hdr(self.tone_map.as_ref());
        if restart {
            self.stop();
            let (control, frames) = open(self.target, white.is_some())?;
            self.control = Some(control);
            self.frames = frames;
        }
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(control) = self.control.take() {
            let _ = control.stop();
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
    }
}

fn log_hdr(tone_map: Option<&ToneMap>) {
    match tone_map {
        Some(tone_map) => {
            tracing::info!("shared display is in HDR, mapping SDR white {:.2} to full white", tone_map.white())
        }
        None => tracing::info!("shared display is in SDR"),
    }
}

fn open(target: Target, hdr: bool) -> Result<(Control, Receiver<RawFrame>)> {
    let (tx, frames) = sync_channel(1);
    let format = if hdr { ColorFormat::Rgba16F } else { ColorFormat::Rgba8 };
    let control = match target {
        Target::Window(hwnd) => {
            let window = Window::from_raw_hwnd(hwnd as usize as *mut std::ffi::c_void);
            if !window.is_valid() {
                bail!("window is gone or closed");
            }
            run(window, format, tx)?
        }
        Target::Monitor(hmonitor) => {
            run(Monitor::from_raw_hmonitor(hmonitor as usize as *mut std::ffi::c_void), format, tx)?
        }
    };
    Ok((control, frames))
}

fn run<T>(item: T, format: ColorFormat, frames: SyncSender<RawFrame>) -> Result<Control>
where
    T: TryInto<GraphicsCaptureItemType> + Send + 'static,
{
    let settings = Settings::new(
        item,
        CursorCaptureSettings::Default,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        format,
        frames,
    );
    Handler::start_free_threaded(settings).map_err(|e| anyhow!("graphics capture: {e:?}"))
}
