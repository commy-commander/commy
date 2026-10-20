use crate::mixer::Mixer;
use anyhow::{anyhow, Context, Result};
use commy_audio::echo::FarEndSink;
use commy_audio::{VoiceProcessor, FRAME_SIZE, SAMPLE_RATE};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

pub struct FrameAssembler {
    src_rate: u32,
    channels: u16,
    mono: VecDeque<f32>,
    pos: f64,
}

impl FrameAssembler {
    pub fn new(src_rate: u32, channels: u16) -> Self {
        Self {
            src_rate: src_rate.max(1),
            channels: channels.max(1),
            mono: VecDeque::new(),
            pos: 0.0,
        }
    }

    pub fn push(&mut self, interleaved: &[f32]) {
        for frame in interleaved.chunks_exact(self.channels as usize) {
            let sum: f32 = frame.iter().sum();
            self.mono.push_back(sum / self.channels as f32);
        }
    }

    pub fn next_frame(&mut self) -> Option<Vec<f32>> {
        let ratio = self.src_rate as f64 / SAMPLE_RATE as f64;
        let last_needed = self.pos + (FRAME_SIZE as f64) * ratio + 1.0;
        if (self.mono.len() as f64) < last_needed {
            return None;
        }
        let mut out = Vec::with_capacity(FRAME_SIZE);
        for i in 0..FRAME_SIZE {
            out.push(sample_at(&self.mono, self.pos + i as f64 * ratio));
        }
        self.pos += FRAME_SIZE as f64 * ratio;
        let consumed = self.pos as usize;
        self.mono.drain(..consumed.min(self.mono.len()));
        self.pos -= consumed as f64;
        Some(out)
    }
}

fn sample_at(samples: &VecDeque<f32>, pos: f64) -> f32 {
    let idx = pos as usize;
    let frac = (pos - idx as f64) as f32;
    let a = *samples.get(idx).unwrap_or(&0.0);
    let b = *samples.get(idx + 1).unwrap_or(&a);
    a + (b - a) * frac
}

pub struct OutputShaper {
    dev_rate: u32,
    buffer: VecDeque<f32>,
    pos: f64,
}

impl OutputShaper {
    pub fn new(dev_rate: u32) -> Self {
        Self {
            dev_rate: dev_rate.max(1),
            buffer: VecDeque::new(),
            pos: 0.0,
        }
    }

    pub fn fill(
        &mut self,
        out: &mut [f32],
        channels: u16,
        mut next_frame: impl FnMut() -> Vec<i16>,
    ) {
        let channels = channels.max(1) as usize;
        let ratio = SAMPLE_RATE as f64 / self.dev_rate as f64;
        for chunk in out.chunks_exact_mut(channels) {
            while (self.buffer.len() as f64) < self.pos + ratio + 2.0 {
                let frame = next_frame();
                self.buffer
                    .extend(frame.iter().map(|s| *s as f32 / 32768.0));
            }
            let sample = sample_at(&self.buffer, self.pos);
            for slot in chunk.iter_mut() {
                *slot = sample;
            }
            self.pos += ratio;
            let consumed = self.pos as usize;
            if consumed > 0 {
                self.buffer.drain(..consumed.min(self.buffer.len()));
                self.pos -= consumed as f64;
            }
        }
    }
}

pub struct StreamHandle {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

pub type InputHandle = StreamHandle;
pub type OutputHandle = StreamHandle;

impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

#[derive(Clone)]
pub struct StreamFault {
    raised: Arc<AtomicBool>,
    thread: std::thread::Thread,
}

impl StreamFault {
    fn for_current_thread() -> Self {
        Self {
            raised: Arc::new(AtomicBool::new(false)),
            thread: std::thread::current(),
        }
    }

    pub fn raise(&self) {
        self.raised.store(true, Ordering::Relaxed);
        self.thread.unpark();
    }

    fn take(&self) -> bool {
        self.raised.swap(false, Ordering::Relaxed)
    }
}

trait AudioStream {
    fn start(&self) -> Result<()>;
}

impl AudioStream for cpal::Stream {
    fn start(&self) -> Result<()> {
        Ok(self.play()?)
    }
}

const STOP_POLL: std::time::Duration = std::time::Duration::from_millis(200);
const REOPEN_PAUSE: std::time::Duration = std::time::Duration::from_millis(500);

fn run_stream_thread<S: AudioStream>(
    died: &'static str,
    mut build: impl FnMut(StreamFault) -> Result<S> + Send + 'static,
) -> Result<StreamHandle> {
    let stop = Arc::new(AtomicBool::new(false));
    let (ready, ready_rx) = std::sync::mpsc::channel();
    let stopped = stop.clone();
    let thread = std::thread::spawn(move || {
        let fault = StreamFault::for_current_thread();
        let mut stream = match build(fault.clone()) {
            Ok(stream) => {
                let _ = ready.send(Ok(()));
                started(stream)
            }
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        };
        let mut reported = false;
        while !stopped.load(Ordering::Relaxed) {
            if fault.take() {
                tracing::warn!("audio stream stopped, reopening it");
                stream = None;
            }
            if stream.is_some() {
                std::thread::park_timeout(STOP_POLL);
                continue;
            }
            std::thread::park_timeout(REOPEN_PAUSE);
            if stopped.load(Ordering::Relaxed) {
                break;
            }
            match build(fault.clone()) {
                Ok(fresh) => {
                    stream = started(fresh);
                    reported = false;
                }
                Err(e) if !reported => {
                    tracing::warn!("audio device unavailable, retrying: {e}");
                    reported = true;
                }
                Err(_) => {}
            }
        }
        drop(stream);
    });
    ready_rx.recv().context(died)?.map(|_| StreamHandle {
        stop,
        thread: Some(thread),
    })
}

fn started<S: AudioStream>(stream: S) -> Option<S> {
    match stream.start() {
        Ok(()) => Some(stream),
        Err(e) => {
            tracing::error!("audio stream failed to start: {e}");
            None
        }
    }
}

pub fn list_device_names(input: bool) -> Vec<String> {
    let host = cpal::default_host();
    let devices = if input { host.input_devices() } else { host.output_devices() };
    let mut names: Vec<String> = devices
        .map(|list| list.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default();
    names.dedup();
    names
}

pub fn select_device(available: &[String], wanted: Option<&str>) -> Option<usize> {
    let wanted = wanted?.trim();
    if wanted.is_empty() {
        return None;
    }
    available.iter().position(|n| n == wanted)
}

fn find_device(host: &cpal::Host, input: bool, wanted: Option<&str>) -> Option<cpal::Device> {
    let chosen = wanted.and_then(|wanted| {
        let devices: Vec<cpal::Device> = if input {
            host.input_devices().ok()?.collect()
        } else {
            host.output_devices().ok()?.collect()
        };
        let names: Vec<String> = devices.iter().map(|d| d.name().unwrap_or_default()).collect();
        let index = select_device(&names, Some(wanted))?;
        devices.into_iter().nth(index)
    });
    if chosen.is_none() && wanted.is_some() {
        tracing::warn!("audio device {wanted:?} not found, using the system default");
    }
    chosen.or_else(|| if input { host.default_input_device() } else { host.default_output_device() })
}

pub fn start_microphone(
    processor: Arc<Mutex<VoiceProcessor>>,
    tx: UnboundedSender<Vec<i16>>,
    device_name: Option<String>,
) -> Result<InputHandle> {
    run_stream_thread("microphone thread died", move |fault| {
        let processor = processor.clone();
        let tx = tx.clone();
        let host = cpal::default_host();
        let device = find_device(&host, true, device_name.as_deref())
            .ok_or_else(|| anyhow!("no microphone found"))?;
        let config = device
            .default_input_config()
            .context("no default input config")?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels();
        let mut assembler = FrameAssembler::new(sample_rate, channels);

        let mut data_cb = move |data: &[f32], latency: Option<std::time::Duration>| {
            assembler.push(data);
            let mut processor = processor.lock().unwrap();
            processor.note_capture_latency(latency);
            while let Some(frame) = assembler.next_frame() {
                let processed = processor.process(&frame);
                if let Ok(processed) = processed {
                    if processed.should_transmit {
                        let pcm: Vec<i16> = processed
                            .samples
                            .iter()
                            .map(|s| (s * 32767.0) as i16)
                            .collect();
                        let _ = tx.send(pcm);
                    }
                }
            }
        };

        let err_cb = move |e| {
            tracing::error!("microphone stream error: {e}");
            fault.raise();
        };
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config.into(),
                move |data: &[f32], info| data_cb(data, capture_latency(info)),
                err_cb,
                None,
            )?,
            cpal::SampleFormat::I16 => {
                let mut floats: Vec<f32> = Vec::new();
                device.build_input_stream(
                    &config.into(),
                    move |data: &[i16], info| {
                        floats.clear();
                        floats.extend(data.iter().map(|s| *s as f32 / 32768.0));
                        data_cb(&floats, capture_latency(info));
                    },
                    err_cb,
                    None,
                )?
            }
            cpal::SampleFormat::U16 => {
                let mut floats: Vec<f32> = Vec::new();
                device.build_input_stream(
                    &config.into(),
                    move |data: &[u16], info| {
                        floats.clear();
                        floats.extend(data.iter().map(|s| (*s as f32 - 32768.0) / 32768.0));
                        data_cb(&floats, capture_latency(info));
                    },
                    err_cb,
                    None,
                )?
            }
            other => return Err(anyhow!("unsupported input format {other:?}")),
        };
        Ok(stream)
    })
}

fn capture_latency(info: &cpal::InputCallbackInfo) -> Option<std::time::Duration> {
    let stamp = info.timestamp();
    stamp.callback.duration_since(&stamp.capture)
}

fn playback_latency(info: &cpal::OutputCallbackInfo) -> Option<std::time::Duration> {
    let stamp = info.timestamp();
    stamp.playback.duration_since(&stamp.callback)
}

pub fn start_playback(
    mixer: Arc<Mutex<Mixer>>,
    device_name: Option<String>,
    far_end: Option<FarEndSink>,
) -> Result<OutputHandle> {
    run_stream_thread("playback thread died", move |fault| {
        let mixer = mixer.clone();
        let far_end = far_end.clone();
        let host = cpal::default_host();
        let device = find_device(&host, false, device_name.as_deref())
            .ok_or_else(|| anyhow!("no output device found"))?;
        let config = device
            .default_output_config()
            .context("no default output config")?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels();
        let mut shaper = OutputShaper::new(sample_rate);

        let stream = device.build_output_stream(
            &config.into(),
            move |data: &mut [f32], info| {
                let latency = playback_latency(info);
                shaper.fill(data, channels, || {
                    let frame = mixer.lock().unwrap().next_frame();
                    if let Some(sink) = &far_end {
                        sink.push(&frame, latency);
                    }
                    frame
                });
            },
            move |e| {
                tracing::error!("playback stream error: {e}");
                fault.raise();
            },
            None,
        )?;
        Ok(stream)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_at_native_rate() {
        let mut fa = FrameAssembler::new(SAMPLE_RATE, 1);
        let input: Vec<f32> = (0..FRAME_SIZE * 2).map(|i| i as f32).collect();
        fa.push(&input);
        let frame = fa.next_frame().unwrap();
        assert_eq!(frame.len(), FRAME_SIZE);
        assert!((frame[0] - 0.0).abs() < 1e-3);
        assert!((frame[100] - 100.0).abs() < 1e-3);
    }

    #[test]
    fn stereo_is_downmixed() {
        let mut fa = FrameAssembler::new(SAMPLE_RATE, 2);
        let mut input = Vec::new();
        for _ in 0..FRAME_SIZE * 2 {
            input.push(1.0);
            input.push(0.0);
        }
        fa.push(&input);
        let frame = fa.next_frame().unwrap();
        assert!(frame.iter().all(|s| (s - 0.5).abs() < 1e-3));
    }

    #[test]
    fn resamples_44100_to_48000() {
        let mut fa = FrameAssembler::new(44_100, 1);
        fa.push(&vec![0.25f32; 44_100]);
        let mut frames = 0;
        while fa.next_frame().is_some() {
            frames += 1;
        }
        assert!(
            (98..=100).contains(&frames),
            "expected about 100 frames, got {frames}"
        );
    }

    #[test]
    fn insufficient_input_yields_none() {
        let mut fa = FrameAssembler::new(SAMPLE_RATE, 1);
        fa.push(&vec![0.0; 10]);
        assert!(fa.next_frame().is_none());
    }

    #[test]
    fn output_shaper_fills_stereo_from_mono_frames() {
        let mut shaper = OutputShaper::new(SAMPLE_RATE);
        let mut out = vec![0.0f32; 960];
        shaper.fill(&mut out, 2, || vec![16384i16; FRAME_SIZE]);
        assert!((out[0] - 0.5).abs() < 0.01);
        assert!((out[1] - 0.5).abs() < 0.01);
        assert!((out[958] - 0.5).abs() < 0.01);
    }

    #[test]
    fn output_shaper_handles_rate_mismatch() {
        let mut shaper = OutputShaper::new(44_100);
        let mut out = vec![0.0f32; 441];
        let mut calls = 0;
        shaper.fill(&mut out, 1, || {
            calls += 1;
            vec![0i16; FRAME_SIZE]
        });
        assert!((1..=3).contains(&calls), "pulled {calls} frames");
    }

    #[test]
    fn saved_device_is_found_by_exact_name() {
        let names = vec!["Speakers (Realtek)".to_string(), "Headset (USB)".to_string()];
        assert_eq!(select_device(&names, Some("Headset (USB)")), Some(1));
    }

    #[test]
    fn missing_or_unset_device_falls_back_to_default() {
        let names = vec!["Speakers (Realtek)".to_string()];
        assert_eq!(select_device(&names, None), None);
        assert_eq!(select_device(&names, Some("")), None);
        assert_eq!(select_device(&names, Some("Unplugged Mic")), None);
    }

    struct FakeStream;

    impl AudioStream for FakeStream {
        fn start(&self) -> Result<()> {
            Ok(())
        }
    }

    fn wait_for(what: &str, done: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done() {
            assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn a_stream_that_errors_is_reopened() {
        let faults: Arc<Mutex<Vec<StreamFault>>> = Arc::default();
        let opened = faults.clone();
        let handle = run_stream_thread("test", move |fault| {
            opened.lock().unwrap().push(fault);
            Ok(FakeStream)
        })
        .unwrap();
        assert_eq!(faults.lock().unwrap().len(), 1);
        faults.lock().unwrap()[0].raise();
        wait_for("the reopen", || faults.lock().unwrap().len() == 2);
        drop(handle);
    }

    #[test]
    fn reopening_keeps_trying_until_the_device_is_back() {
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let fault_slot: Arc<Mutex<Option<StreamFault>>> = Arc::default();
        let (counted, slot) = (attempts.clone(), fault_slot.clone());
        let handle = run_stream_thread("test", move |fault| {
            let attempt = counted.fetch_add(1, Ordering::SeqCst);
            *slot.lock().unwrap() = Some(fault);
            match attempt {
                1 | 2 => Err(anyhow!("unplugged")),
                _ => Ok(FakeStream),
            }
        })
        .unwrap();
        fault_slot.lock().unwrap().as_ref().unwrap().raise();
        wait_for("the device to come back", || attempts.load(Ordering::SeqCst) == 4);
        std::thread::sleep(REOPEN_PAUSE * 2);
        assert_eq!(attempts.load(Ordering::SeqCst), 4);
        drop(handle);
    }

    #[test]
    fn a_device_that_never_opens_reports_why() {
        let result = run_stream_thread::<FakeStream>("test", |_| Err(anyhow!("no microphone found")));
        assert_eq!(result.err().unwrap().to_string(), "no microphone found");
    }
}
