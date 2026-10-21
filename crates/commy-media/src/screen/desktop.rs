use anyhow::{Context, Result};
use image::RgbaImage;

use super::{encode_frame, ScreenFrame, ShareSource, SourceKind};

pub enum Capturer {
    Monitor(xcap::Monitor),
    Window(xcap::Window),
    #[cfg(windows)]
    Graphics(crate::wgc::Capture),
}

impl Capturer {
    pub fn open(source: Option<(SourceKind, u32)>) -> Result<Self> {
        #[cfg(windows)]
        {
            let graphics = match source {
                Some((SourceKind::Window, id)) => crate::wgc::Capture::window(id),
                Some((SourceKind::Monitor, id)) => crate::wgc::Capture::monitor(id),
                None => xcap::Monitor::all()
                    .context("cannot enumerate monitors")
                    .and_then(|monitors| {
                        monitors
                            .iter()
                            .find(|m| m.is_primary().unwrap_or(false))
                            .or_else(|| monitors.first())
                            .and_then(|m| m.id().ok())
                            .context("no monitor found")
                    })
                    .and_then(crate::wgc::Capture::monitor),
            };
            match graphics {
                Ok(capture) => return Ok(Self::Graphics(capture)),
                Err(e) => tracing::warn!("graphics capture unavailable, using GDI: {e:#}"),
            }
        }
        Self::open_xcap(source)
    }

    fn open_xcap(source: Option<(SourceKind, u32)>) -> Result<Self> {
        match source {
            Some((SourceKind::Window, id)) => {
                let window = xcap::Window::all()
                    .context("cannot enumerate windows")?
                    .into_iter()
                    .find(|w| w.id().map(|w_id| w_id == id).unwrap_or(false))
                    .context("window is gone or closed")?;
                Ok(Self::Window(window))
            }
            Some((SourceKind::Monitor, id)) => {
                let monitor = xcap::Monitor::all()
                    .context("cannot enumerate monitors")?
                    .into_iter()
                    .find(|m| m.id().map(|m_id| m_id == id).unwrap_or(false))
                    .context("monitor is gone")?;
                Ok(Self::Monitor(monitor))
            }
            None => {
                let monitors = xcap::Monitor::all().context("cannot enumerate monitors")?;
                let primary = monitors.iter().position(|m| m.is_primary().unwrap_or(false)).unwrap_or(0);
                let monitor = monitors.into_iter().nth(primary).context("no monitor found")?;
                Ok(Self::Monitor(monitor))
            }
        }
    }

    pub fn grab(&mut self) -> Result<RgbaImage> {
        match self {
            Self::Monitor(m) => m.capture_image().context("monitor capture failed"),
            Self::Window(w) => w.capture_image().context("window capture failed"),
            #[cfg(windows)]
            Self::Graphics(c) => c.grab(),
        }
    }
}

pub fn list_sources() -> Result<Vec<ShareSource>> {
    let mut sources = Vec::new();
    for monitor in xcap::Monitor::all().context("cannot enumerate monitors")? {
        let id = monitor.id().unwrap_or(0);
        let name = monitor
            .name()
            .unwrap_or_else(|_| format!("Monitor {id}"));
        sources.push(ShareSource {
            kind: SourceKind::Monitor,
            id,
            name,
            is_primary: monitor.is_primary().unwrap_or(false),
            pid: None,
        });
    }
    for window in xcap::Window::all().context("cannot enumerate windows")? {
        if window.is_minimized().unwrap_or(true) {
            continue;
        }
        let title = window.title().unwrap_or_default();
        if title.trim().is_empty() {
            continue;
        }
        sources.push(ShareSource {
            kind: SourceKind::Window,
            id: window.id().unwrap_or(0),
            name: title,
            is_primary: false,
            pid: window.pid().ok(),
        });
    }
    Ok(sources)
}

pub fn capture_source(
    kind: SourceKind,
    id: u32,
    max_width: u32,
    quality: u8,
) -> Result<ScreenFrame> {
    let image = Capturer::open_xcap(Some((kind, id)))?.grab()?;
    encode_frame(image, max_width, quality)
}
