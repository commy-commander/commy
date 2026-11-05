use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use commy_media::{ClipMixer, ClipQueue, FrameSource, OutputHandle};

use crate::state::App;

const IDLE_CLOSE_FRAMES: u32 = 300;
const IDLE_CHECK: Duration = Duration::from_secs(1);

pub trait Drains: FrameSource {
    fn idle_frames(&self) -> u32;
}

impl Drains for ClipQueue {
    fn idle_frames(&self) -> u32 {
        ClipQueue::idle_frames(self)
    }
}

impl Drains for ClipMixer {
    fn idle_frames(&self) -> u32 {
        ClipMixer::idle_frames(self)
    }
}

#[derive(Clone, Debug)]
pub struct Hearing {
    pub device: Option<String>,
    pub volume: f32,
}

impl Hearing {
    pub fn of(app: &App) -> Self {
        let voice = app.voice.lock().unwrap();
        Self { device: voice.output_device.clone(), volume: voice.output_volume }
    }
}

struct Output {
    device: Option<String>,
    id: u64,
    _handle: OutputHandle,
}

pub struct DeviceOut<S> {
    source: Arc<Mutex<S>>,
    output: Mutex<Option<Output>>,
    opened: AtomicU64,
}

impl<S: Drains> DeviceOut<S> {
    pub fn new(source: S) -> Self {
        Self { source: Arc::new(Mutex::new(source)), output: Mutex::new(None), opened: AtomicU64::new(0) }
    }

    pub fn source(&self) -> MutexGuard<'_, S> {
        self.source.lock().unwrap()
    }

    pub fn open(&'static self, device: Option<String>) -> Result<(), String> {
        let mut output = self.output.lock().unwrap();
        if output.as_ref().is_some_and(|o| o.device == device) {
            return Ok(());
        }
        output.take();
        let handle = commy_media::start_playback(self.source.clone(), device.clone(), None)
            .map_err(|e| format!("could not open the speakers: {e}"))?;
        let id = self.opened.fetch_add(1, Ordering::SeqCst) + 1;
        *output = Some(Output { device, id, _handle: handle });
        let _ = std::thread::Builder::new().name("output-idle".into()).spawn(move || self.close_when_idle(id));
        Ok(())
    }

    fn close_when_idle(&self, id: u64) {
        loop {
            std::thread::sleep(IDLE_CHECK);
            let mut output = self.output.lock().unwrap();
            if output.as_ref().map(|o| o.id) != Some(id) {
                return;
            }
            if self.source().idle_frames() >= IDLE_CLOSE_FRAMES {
                output.take();
                return;
            }
        }
    }
}
