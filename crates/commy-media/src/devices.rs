//! cpal device glue. The sample-rate/channel shaping lives in pure
//! structs (`FrameAssembler`, `OutputShaper`) so it can be tested;
//! the cpal streams themselves are created on dedicated threads
//! because they are not Send.

use crate::mixer::Mixer;
use anyhow::{anyhow, Context, Result};
use commy_audio::{VoiceProcessor, FRAME_SIZE, SAMPLE_RATE};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

/// Turns arbitrary interleaved device input into 48 kHz mono frames
/// of FRAME_SIZE samples, with linear resampling.
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

    /// Next 10 ms 48 kHz mono frame, or None until enough input landed.
    pub fn next_frame(&mut self) -> Option<Vec<f32>> {
        let ratio = self.src_rate as f64 / SAMPLE_RATE as f64;
        let last_needed = self.pos + (FRAME_SIZE as f64) * ratio + 1.0;
        if (self.mono.len() as f64) < last_needed {
            return None;
        }
        let mut out = Vec::with_capacity(FRAME_SIZE);
        for i in 0..FRAME_SIZE {
            let p = self.pos + i as f64 * ratio;
            let idx = p as usize;
            let frac = (p - idx as f64) as f32;
            let a = *self.mono.get(idx).unwrap_or(&0.0);
            let b = *self.mono.get(idx + 1).unwrap_or(&a);
            out.push(a + (b - a) * frac);
        }
        self.pos += FRAME_SIZE as f64 * ratio;
        let consumed = self.pos as usize;
        self.mono.drain(..consumed.min(self.mono.len()));
        self.pos -= consumed as f64;
        Some(out)
    }
}

/// Feeds a device output callback from 48 kHz mono mixer frames,
/// resampling and fanning out to the device channel count.
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
            let idx = self.pos as usize;
            let frac = (self.pos - idx as f64) as f32;
            let a = *self.buffer.get(idx).unwrap_or(&0.0);
            let b = *self.buffer.get(idx + 1).unwrap_or(&a);
            let sample = a + (b - a) * frac;
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

/// Keeps a device stream alive on its own thread until dropped.
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

fn run_stream_thread(
    stop: Arc<AtomicBool>,
    build: impl FnOnce() -> Result<cpal::Stream> + Send + 'static,
    ready: std::sync::mpsc::Sender<Result<()>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let stream = match build() {
            Ok(stream) => {
                let _ = ready.send(Ok(()));
                stream
            }
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        };
        if let Err(e) = stream.play() {
            tracing::error!("audio stream failed to start: {e}");
        }
        while !stop.load(Ordering::Relaxed) {
            std::thread::park_timeout(std::time::Duration::from_millis(200));
        }
        drop(stream);
    })
}

/// Names of the system's microphones (`input`) or speakers.
pub fn list_device_names(input: bool) -> Vec<String> {
    let host = cpal::default_host();
    let devices = if input { host.input_devices() } else { host.output_devices() };
    let mut names: Vec<String> = devices
        .map(|list| list.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default();
    names.dedup();
    names
}

/// Index of the device to use: the saved name if it is still plugged
/// in, otherwise None so the caller falls back to the system default.
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

/// Captures a microphone (the named one, else the default), runs it through the shared
/// `VoiceProcessor`, and emits transmit-ready PCM frames.
pub fn start_microphone(
    processor: Arc<Mutex<VoiceProcessor>>,
    tx: UnboundedSender<Vec<i16>>,
    device_name: Option<String>,
) -> Result<InputHandle> {
    let stop = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();

    let thread = run_stream_thread(
        stop.clone(),
        move || {
            let host = cpal::default_host();
            let device = find_device(&host, true, device_name.as_deref())
                .ok_or_else(|| anyhow!("no microphone found"))?;
            let config = device
                .default_input_config()
                .context("no default input config")?;
            let sample_rate = config.sample_rate().0;
            let channels = config.channels();
            let assembler = Arc::new(Mutex::new(FrameAssembler::new(sample_rate, channels)));

            let data_cb = {
                let assembler = assembler.clone();
                let processor = processor.clone();
                let tx = tx.clone();
                move |data: &[f32]| {
                    let mut assembler = assembler.lock().unwrap();
                    assembler.push(data);
                    while let Some(frame) = assembler.next_frame() {
                        let processed = processor.lock().unwrap().process(&frame);
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
                }
            };

            let err_cb = |e| tracing::error!("microphone stream error: {e}");
            let stream = match config.sample_format() {
                cpal::SampleFormat::F32 => device.build_input_stream(
                    &config.into(),
                    move |data: &[f32], _| data_cb(data),
                    err_cb,
                    None,
                )?,
                cpal::SampleFormat::I16 => device.build_input_stream(
                    &config.into(),
                    move |data: &[i16], _| {
                        let floats: Vec<f32> =
                            data.iter().map(|s| *s as f32 / 32768.0).collect();
                        data_cb(&floats);
                    },
                    err_cb,
                    None,
                )?,
                cpal::SampleFormat::U16 => device.build_input_stream(
                    &config.into(),
                    move |data: &[u16], _| {
                        let floats: Vec<f32> = data
                            .iter()
                            .map(|s| (*s as f32 - 32768.0) / 32768.0)
                            .collect();
                        data_cb(&floats);
                    },
                    err_cb,
                    None,
                )?,
                other => return Err(anyhow!("unsupported input format {other:?}")),
            };
            Ok(stream)
        },
        ready_tx,
    );

    ready_rx
        .recv()
        .context("microphone thread died")?
        .map(|_| StreamHandle {
            stop,
            thread: Some(thread),
        })
}

/// Plays the mixed voices of every peer through the default output.
pub fn start_playback(mixer: Arc<Mutex<Mixer>>, device_name: Option<String>) -> Result<OutputHandle> {
    let stop = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();

    let thread = run_stream_thread(
        stop.clone(),
        move || {
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
                move |data: &mut [f32], _| {
                    let mixer = mixer.clone();
                    shaper.fill(data, channels, move || mixer.lock().unwrap().next_frame());
                },
                |e| tracing::error!("playback stream error: {e}"),
                None,
            )?;
            Ok(stream)
        },
        ready_tx,
    );

    ready_rx
        .recv()
        .context("playback thread died")?
        .map(|_| StreamHandle {
            stop,
            thread: Some(thread),
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
        // L=1.0, R=0.0 -> mono 0.5
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
        // One second of input should yield close to 100 frames.
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
        let mut out = vec![0.0f32; 960]; // 480 stereo samples
        shaper.fill(&mut out, 2, || vec![16384i16; FRAME_SIZE]);
        // Both channels carry the same upscaled value (~0.5).
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
        // 441 samples at 44.1 kHz is 10 ms, about one 48 kHz frame.
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
}
