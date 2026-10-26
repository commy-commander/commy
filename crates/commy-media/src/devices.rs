use crate::mixer::Mixer;
use anyhow::{anyhow, Context, Result};
use commy_audio::echo::FarEndSink;
use commy_audio::{VoiceProcessor, FRAME_SIZE, SAMPLE_RATE};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

const DEAD_CHANNEL_POWER_RATIO: f32 = 0.01;
const CHANNEL_POWER_SMOOTHING: f32 = 0.2;

pub struct FrameAssembler {
    src_rate: u32,
    channels: u16,
    mono: VecDeque<f32>,
    pos: f64,
    channel_power: Option<Vec<f32>>,
}

impl FrameAssembler {
    pub fn new(src_rate: u32, channels: u16) -> Self {
        Self {
            src_rate: src_rate.max(1),
            channels: channels.max(1),
            mono: VecDeque::new(),
            pos: 0.0,
            channel_power: None,
        }
    }

    pub fn for_microphone(src_rate: u32, channels: u16) -> Self {
        let mut assembler = Self::new(src_rate, channels);
        if assembler.channels > 1 {
            assembler.channel_power = Some(vec![0.0; assembler.channels as usize]);
        }
        assembler
    }

    pub fn push(&mut self, interleaved: &[f32]) {
        let live = self.live_channels(interleaved);
        for frame in interleaved.chunks_exact(self.channels as usize) {
            let (sum, count) = frame
                .iter()
                .zip(&live)
                .filter(|(_, live)| **live)
                .fold((0.0f32, 0usize), |(sum, count), (s, _)| (sum + s, count + 1));
            self.mono.push_back(sum / count.max(1) as f32);
        }
    }

    fn live_channels(&mut self, interleaved: &[f32]) -> Vec<bool> {
        let channels = self.channels as usize;
        let Some(power) = &mut self.channel_power else {
            return vec![true; channels];
        };
        let frames = (interleaved.len() / channels).max(1) as f32;
        for (channel, smoothed) in power.iter_mut().enumerate() {
            let now = interleaved.iter().skip(channel).step_by(channels).map(|s| s * s).sum::<f32>() / frames;
            *smoothed += (now - *smoothed) * CHANNEL_POWER_SMOOTHING;
        }
        let loudest = power.iter().copied().fold(0.0f32, f32::max);
        power.iter().map(|p| *p >= loudest * DEAD_CHANNEL_POWER_RATIO).collect()
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
pub(crate) struct StreamFault {
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

    pub(crate) fn raise(&self) {
        self.raised.store(true, Ordering::Relaxed);
        self.thread.unpark();
    }

    fn take(&self) -> bool {
        self.raised.swap(false, Ordering::Relaxed)
    }
}

pub(crate) trait AudioStream {
    fn start(&self) -> Result<()>;
}

impl AudioStream for cpal::Stream {
    fn start(&self) -> Result<()> {
        Ok(self.play()?)
    }
}

const STOP_POLL: std::time::Duration = std::time::Duration::from_millis(200);
const REOPEN_PAUSE: std::time::Duration = std::time::Duration::from_millis(500);
const POLLS_PER_DEVICE_CHECK: u32 = 10;

fn run_stream_thread<S: AudioStream>(
    died: &'static str,
    mut build: impl FnMut(StreamFault) -> Result<S> + Send + 'static,
    mut device_moved: impl FnMut() -> bool + Send + 'static,
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
        let mut polls = 0u32;
        while !stopped.load(Ordering::Relaxed) {
            if fault.take() {
                tracing::warn!("audio stream stopped, reopening it");
                stream = None;
            }
            if stream.is_some() {
                polls += 1;
                if polls >= POLLS_PER_DEVICE_CHECK {
                    polls = 0;
                    if device_moved() {
                        tracing::info!("the audio device to use changed, switching to it");
                        stream = None;
                        continue;
                    }
                }
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
    let mut names = device_names(&cpal::default_host(), input);
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

type OpenedDevice = Arc<Mutex<Option<String>>>;

fn device_names(host: &cpal::Host, input: bool) -> Vec<String> {
    let devices = if input { host.input_devices() } else { host.output_devices() };
    devices
        .map(|list| list.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

fn default_device_name(host: &cpal::Host, input: bool) -> Option<String> {
    let device = if input { host.default_input_device() } else { host.default_output_device() };
    device.and_then(|d| d.name().ok())
}

pub fn device_to_switch_to(
    wanted: Option<&str>,
    opened: Option<&str>,
    wanted_is_back: impl FnOnce(&str) -> bool,
    default_now: impl FnOnce() -> Option<String>,
) -> Option<String> {
    let opened = opened?;
    match wanted.map(str::trim).filter(|w| !w.is_empty()) {
        Some(wanted) if wanted == opened => None,
        Some(wanted) => wanted_is_back(wanted).then(|| wanted.to_string()),
        None => default_now().filter(|now| now != opened),
    }
}

fn device_moved(input: bool, wanted: Option<&str>, opened: &OpenedDevice) -> bool {
    let opened = opened.lock().unwrap().clone();
    let host = cpal::default_host();
    device_to_switch_to(
        wanted,
        opened.as_deref(),
        |wanted| device_names(&host, input).iter().any(|n| n == wanted),
        || default_device_name(&host, input),
    )
    .is_some()
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
    let opened = OpenedDevice::default();
    let watched = opened.clone();
    let wanted = device_name.clone();
    let build = move |fault: StreamFault| {
        let processor = processor.clone();
        let tx = tx.clone();
        let host = cpal::default_host();
        let device = find_device(&host, true, device_name.as_deref())
            .ok_or_else(|| anyhow!("no microphone found"))?;
        *opened.lock().unwrap() = device.name().ok();
        let config = device
            .default_input_config()
            .context("no default input config")?;
        let mut assembler = FrameAssembler::for_microphone(config.sample_rate().0, config.channels());
        processor.lock().unwrap().start_fresh();

        let data_cb = move |data: &[f32]| {
            assembler.push(data);
            let mut processor = processor.lock().unwrap();
            while let Some(frame) = assembler.next_frame() {
                let processed = processor.process(&frame);
                if let Ok(processed) = processed {
                    if processed.should_transmit {
                        let _ = tx.send(pcm16(&processed.samples));
                    }
                }
            }
        };
        let err_cb = move |e| {
            tracing::error!("microphone stream error: {e}");
            fault.raise();
        };
        open_input_stream(&device, config, data_cb, err_cb)
    };
    run_stream_thread("microphone thread died", build, move || device_moved(true, wanted.as_deref(), &watched))
}

pub fn pcm16(samples: &[f32]) -> Vec<i16> {
    samples.iter().map(|s| (s * 32767.0) as i16).collect()
}

pub fn system_audio_supported() -> bool {
    cfg!(windows)
}

#[cfg(windows)]
pub fn start_system_audio(tx: UnboundedSender<Vec<i16>>) -> Result<InputHandle> {
    let without_commy = tx.clone();
    let opened = run_stream_thread(
        "shared audio thread died",
        move |fault| crate::loopback::ProcessLoopback::open(without_commy.clone(), fault),
        || false,
    );
    opened.or_else(|e| {
        tracing::warn!("sharing all system audio, Commy's own sounds included: {e}");
        start_device_loopback(tx)
    })
}

#[cfg(windows)]
fn start_device_loopback(tx: UnboundedSender<Vec<i16>>) -> Result<InputHandle> {
    run_stream_thread("system audio thread died", move |fault| {
        let tx = tx.clone();
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| anyhow!("no output device to capture from"))?;
        let config = device
            .default_output_config()
            .context("no default output config")?;
        let mut assembler = FrameAssembler::new(config.sample_rate().0, config.channels());
        let data_cb = move |data: &[f32]| {
            assembler.push(data);
            while let Some(frame) = assembler.next_frame() {
                let _ = tx.send(pcm16(&frame));
            }
        };
        let err_cb = move |e| {
            tracing::error!("system audio stream error: {e}");
            fault.raise();
        };
        open_input_stream(&device, config, data_cb, err_cb)
    }, || false)
}

#[cfg(not(windows))]
pub fn start_system_audio(_tx: UnboundedSender<Vec<i16>>) -> Result<InputHandle> {
    Err(anyhow!("system audio capture is not available on this platform"))
}

fn open_input_stream(
    device: &cpal::Device,
    config: cpal::SupportedStreamConfig,
    mut data_cb: impl FnMut(&[f32]) + Send + 'static,
    err_cb: impl Fn(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream> {
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config.into(),
            move |data: &[f32], _| data_cb(data),
            err_cb,
            None,
        )?,
        cpal::SampleFormat::I16 => {
            let mut floats: Vec<f32> = Vec::new();
            device.build_input_stream(
                &config.into(),
                move |data: &[i16], _| {
                    floats.clear();
                    floats.extend(data.iter().map(|s| *s as f32 / 32768.0));
                    data_cb(&floats);
                },
                err_cb,
                None,
            )?
        }
        cpal::SampleFormat::U16 => {
            let mut floats: Vec<f32> = Vec::new();
            device.build_input_stream(
                &config.into(),
                move |data: &[u16], _| {
                    floats.clear();
                    floats.extend(data.iter().map(|s| (*s as f32 - 32768.0) / 32768.0));
                    data_cb(&floats);
                },
                err_cb,
                None,
            )?
        }
        other => return Err(anyhow!("unsupported input format {other:?}")),
    };
    Ok(stream)
}

pub fn start_playback(
    mixer: Arc<Mutex<Mixer>>,
    device_name: Option<String>,
    far_end: Option<FarEndSink>,
) -> Result<OutputHandle> {
    let opened = OpenedDevice::default();
    let watched = opened.clone();
    let wanted = device_name.clone();
    let build = move |fault: StreamFault| {
        let mixer = mixer.clone();
        let far_end = far_end.clone();
        let host = cpal::default_host();
        let device = find_device(&host, false, device_name.as_deref())
            .ok_or_else(|| anyhow!("no output device found"))?;
        *opened.lock().unwrap() = device.name().ok();
        let config = device
            .default_output_config()
            .context("no default output config")?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels();
        let mut shaper = OutputShaper::new(sample_rate);

        let stream = device.build_output_stream(
            &config.into(),
            move |data: &mut [f32], _| {
                shaper.fill(data, channels, || {
                    let frame = mixer.lock().unwrap().next_frame();
                    if let Some(sink) = &far_end {
                        sink.push(&frame);
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
    };
    run_stream_thread("playback thread died", build, move || device_moved(false, wanted.as_deref(), &watched))
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
    fn a_microphone_with_one_silent_channel_keeps_its_full_level() {
        let mut fa = FrameAssembler::for_microphone(SAMPLE_RATE, 2);
        for _ in 0..20 {
            let input: Vec<f32> = (0..FRAME_SIZE).flat_map(|_| [0.4, 0.0]).collect();
            fa.push(&input);
        }
        let mut last = Vec::new();
        while let Some(frame) = fa.next_frame() {
            last = frame;
        }
        assert!(last.iter().all(|s| (s - 0.4).abs() < 1e-3), "{}", last[0]);
    }

    #[test]
    fn a_microphone_with_two_live_channels_is_averaged() {
        let mut fa = FrameAssembler::for_microphone(SAMPLE_RATE, 2);
        let input: Vec<f32> = (0..FRAME_SIZE * 2).flat_map(|_| [0.4, 0.2]).collect();
        fa.push(&input);
        let frame = fa.next_frame().unwrap();
        assert!(frame.iter().all(|s| (s - 0.3).abs() < 1e-3));
        let mut quiet = FrameAssembler::for_microphone(SAMPLE_RATE, 2);
        quiet.push(&vec![0.0; FRAME_SIZE * 4]);
        assert!(quiet.next_frame().unwrap().iter().all(|s| *s == 0.0));
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
    fn pcm16_scales_and_saturates() {
        assert_eq!(pcm16(&[0.0, 0.5, -0.5]), vec![0, 16383, -16383]);
        assert_eq!(pcm16(&[2.0, -2.0]), vec![i16::MAX, i16::MIN]);
    }

    #[test]
    fn system_audio_is_only_offered_where_loopback_capture_exists() {
        assert_eq!(system_audio_supported(), cfg!(windows));
        if !system_audio_supported() {
            let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
            assert!(start_system_audio(tx).is_err());
        }
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
        }, || false)
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
        }, || false)
        .unwrap();
        fault_slot.lock().unwrap().as_ref().unwrap().raise();
        wait_for("the device to come back", || attempts.load(Ordering::SeqCst) == 4);
        std::thread::sleep(REOPEN_PAUSE * 2);
        assert_eq!(attempts.load(Ordering::SeqCst), 4);
        drop(handle);
    }

    #[test]
    fn a_stream_moves_when_the_device_to_use_changes() {
        let opens = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let moved = Arc::new(AtomicBool::new(false));
        let (counted, switch) = (opens.clone(), moved.clone());
        let handle = run_stream_thread(
            "test",
            move |_| {
                counted.fetch_add(1, Ordering::SeqCst);
                Ok(FakeStream)
            },
            move || switch.swap(false, Ordering::SeqCst),
        )
        .unwrap();
        std::thread::sleep(STOP_POLL * POLLS_PER_DEVICE_CHECK * 2);
        assert_eq!(opens.load(Ordering::SeqCst), 1, "nothing changed, nothing reopened");
        moved.store(true, Ordering::SeqCst);
        wait_for("the switch", || opens.load(Ordering::SeqCst) == 2);
        drop(handle);
    }

    #[test]
    fn the_chosen_device_is_used_again_once_it_is_back() {
        let back = |_: &str| true;
        let gone = |_: &str| false;
        let no_default = || None;
        assert_eq!(
            device_to_switch_to(Some("Headset"), Some("Laptop Mic"), back, no_default),
            Some("Headset".to_string())
        );
        assert_eq!(device_to_switch_to(Some("Headset"), Some("Laptop Mic"), gone, no_default), None);
        assert_eq!(device_to_switch_to(Some("Headset"), Some("Headset"), back, no_default), None);
        assert_eq!(device_to_switch_to(Some("Headset"), None, back, no_default), None);
    }

    #[test]
    fn following_the_system_default_moves_when_the_default_changes() {
        let unused = |_: &str| panic!("no chosen device to look for");
        assert_eq!(
            device_to_switch_to(None, Some("Laptop Mic"), unused, || Some("Headset".to_string())),
            Some("Headset".to_string())
        );
        assert_eq!(device_to_switch_to(Some("  "), Some("Headset"), unused, || Some("Headset".to_string())), None);
        assert_eq!(device_to_switch_to(None, Some("Headset"), unused, || None), None);
    }

    #[test]
    fn a_device_that_never_opens_reports_why() {
        let result = run_stream_thread::<FakeStream>("test", |_| Err(anyhow!("no microphone found")), || false);
        assert_eq!(result.err().unwrap().to_string(), "no microphone found");
    }
}
