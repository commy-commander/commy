use anyhow::{anyhow, bail, Context as _, Result};
use image::RgbaImage;
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::time::Duration;
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, GraphicsCaptureItemType,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

struct Handler {
    frames: SyncSender<RgbaImage>,
    scratch: Vec<u8>,
}

impl GraphicsCaptureApiHandler for Handler {
    type Flags = SyncSender<RgbaImage>;
    type Error = anyhow::Error;

    fn new(ctx: Context<Self::Flags>) -> Result<Self> {
        Ok(Self { frames: ctx.flags, scratch: Vec::new() })
    }

    fn on_frame_arrived(&mut self, frame: &mut Frame, control: InternalCaptureControl) -> Result<()> {
        let buffer = frame.buffer_without_title_bar().context("frame buffer")?;
        let (width, height) = (buffer.width(), buffer.height());
        let pixels = buffer.as_nopadding_buffer(&mut self.scratch).to_vec();
        let image = RgbaImage::from_raw(width, height, pixels).context("frame size mismatch")?;
        if let Err(std::sync::mpsc::TrySendError::Disconnected(_)) = self.frames.try_send(image) {
            control.stop();
        }
        Ok(())
    }

    fn on_closed(&mut self) -> Result<()> {
        Ok(())
    }
}

pub struct Capture {
    control: Option<CaptureControl<Handler, anyhow::Error>>,
    frames: Receiver<RgbaImage>,
    last: Option<RgbaImage>,
}

const FIRST_FRAME_WAIT: Duration = Duration::from_secs(2);
const REPEAT_AFTER: Duration = Duration::from_millis(250);

impl Capture {
    pub fn window(hwnd: u32) -> Result<Self> {
        let window = Window::from_raw_hwnd(hwnd as usize as *mut std::ffi::c_void);
        if !window.is_valid() {
            bail!("window is gone or closed");
        }
        Self::start(window)
    }

    pub fn monitor(hmonitor: u32) -> Result<Self> {
        Self::start(Monitor::from_raw_hmonitor(hmonitor as usize as *mut std::ffi::c_void))
    }

    fn start<T>(item: T) -> Result<Self>
    where
        T: TryInto<GraphicsCaptureItemType> + Send + 'static,
    {
        let (tx, frames) = sync_channel(1);
        let settings = Settings::new(
            item,
            CursorCaptureSettings::Default,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Default,
            DirtyRegionSettings::Default,
            ColorFormat::Rgba8,
            tx,
        );
        let control = Handler::start_free_threaded(settings).map_err(|e| anyhow!("graphics capture: {e:?}"))?;
        Ok(Self { control: Some(control), frames, last: None })
    }

    pub fn grab(&mut self) -> Result<RgbaImage> {
        let wait = if self.last.is_some() { REPEAT_AFTER } else { FIRST_FRAME_WAIT };
        match self.frames.recv_timeout(wait) {
            Ok(mut image) => {
                while let Ok(newer) = self.frames.try_recv() {
                    image = newer;
                }
                self.last = Some(image.clone());
                Ok(image)
            }
            Err(RecvTimeoutError::Timeout) => self.last.clone().context("no frame arrived from graphics capture"),
            Err(RecvTimeoutError::Disconnected) => bail!("graphics capture ended"),
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(control) = self.control.take() {
            let _ = control.stop();
        }
    }
}
