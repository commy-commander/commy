use crate::devices::FrameSource;
use crate::mixer::{ramp_at, volume_gain, Limiter};
use commy_audio::codec::soft_limit;
use commy_audio::{FRAME_SIZE, SAMPLE_RATE};
use std::collections::VecDeque;

const MIN_RATE: f32 = 4_000.0;
const MAX_RATE: f32 = 384_000.0;

#[derive(Debug)]
pub struct ClipQueue {
    samples: VecDeque<f32>,
    gain: f32,
    idle_frames: u32,
}

impl Default for ClipQueue {
    fn default() -> Self {
        Self { samples: VecDeque::new(), gain: 1.0, idle_frames: 0 }
    }
}

impl ClipQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.gain = volume_gain(volume);
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    pub fn push(&mut self, samples: &[f32], rate: f32) {
        let resampled = resample(samples, rate, SAMPLE_RATE);
        if !resampled.is_empty() {
            self.idle_frames = 0;
        }
        self.samples.extend(resampled);
    }

    pub fn queued(&self) -> usize {
        self.samples.len()
    }

    pub fn idle_frames(&self) -> u32 {
        self.idle_frames
    }
}

impl FrameSource for ClipQueue {
    fn next_frame(&mut self) -> Vec<i16> {
        if self.samples.is_empty() {
            self.idle_frames = self.idle_frames.saturating_add(1);
            return vec![0; FRAME_SIZE];
        }
        let gain = self.gain;
        (0..FRAME_SIZE)
            .map(|_| {
                let sample = self.samples.pop_front().unwrap_or(0.0);
                let sample = if sample.is_finite() { sample * gain } else { 0.0 };
                (soft_limit(sample) * i16::MAX as f32).round() as i16
            })
            .collect()
    }
}

pub const MAX_MIXED_CLIPS: usize = 4;
const STOP_FADE_SAMPLES: usize = 240;

#[derive(Debug)]
struct MixedClip {
    id: u64,
    samples: VecDeque<f32>,
}

#[derive(Debug)]
pub struct ClipMixer {
    clips: VecDeque<MixedClip>,
    gain: f32,
    limiter: Limiter,
    idle_frames: u32,
}

impl Default for ClipMixer {
    fn default() -> Self {
        Self { clips: VecDeque::new(), gain: 1.0, limiter: Limiter::default(), idle_frames: 0 }
    }
}

impl ClipMixer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.gain = volume_gain(volume);
    }

    pub fn push(&mut self, id: u64, samples: &[f32], rate: f32, level: f32) {
        let level = if level.is_finite() { level.clamp(0.0, 1.0) } else { 1.0 };
        let samples: VecDeque<f32> = resample(samples, rate, SAMPLE_RATE).into_iter().map(|s| s * level).collect();
        if samples.is_empty() {
            return;
        }
        self.clips.retain(|clip| clip.id != id);
        while self.clips.len() >= MAX_MIXED_CLIPS {
            self.clips.pop_front();
        }
        self.idle_frames = 0;
        self.clips.push_back(MixedClip { id, samples });
    }

    pub fn stop(&mut self, id: u64) {
        for clip in self.clips.iter_mut().filter(|clip| clip.id == id) {
            fade_out(&mut clip.samples);
        }
    }

    pub fn clear(&mut self) {
        self.clips.clear();
    }

    pub fn playing(&self) -> usize {
        self.clips.len()
    }

    pub fn idle_frames(&self) -> u32 {
        self.idle_frames
    }
}

impl FrameSource for ClipMixer {
    fn next_frame(&mut self) -> Vec<i16> {
        if self.clips.is_empty() {
            self.idle_frames = self.idle_frames.saturating_add(1);
            return vec![0; FRAME_SIZE];
        }
        let gain = self.gain;
        let mixed: Vec<f32> = (0..FRAME_SIZE)
            .map(|_| {
                let sum: f32 = self.clips.iter_mut().filter_map(|clip| clip.samples.pop_front()).filter(|s| s.is_finite()).sum();
                sum * gain
            })
            .collect();
        self.clips.retain(|clip| !clip.samples.is_empty());
        let (start, end) = self.limiter.follow(peak(&mixed));
        mixed
            .iter()
            .enumerate()
            .map(|(i, sample)| (soft_limit(sample * ramp_at(start, end, i)) * i16::MAX as f32).round() as i16)
            .collect()
    }
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0, |peak, s| peak.max(s.abs()))
}

fn fade_out(samples: &mut VecDeque<f32>) {
    samples.truncate(STOP_FADE_SAMPLES);
    let len = samples.len() as f32;
    for (i, sample) in samples.iter_mut().enumerate() {
        *sample *= 1.0 - (i + 1) as f32 / len;
    }
}

pub fn resample(samples: &[f32], from_rate: f32, to_rate: u32) -> Vec<f32> {
    if samples.is_empty() || !from_rate.is_finite() {
        return Vec::new();
    }
    let from_rate = from_rate.clamp(MIN_RATE, MAX_RATE) as f64;
    let step = from_rate / to_rate as f64;
    let count = (samples.len() as f64 / step).round() as usize;
    let last = samples.len() - 1;
    (0..count)
        .map(|i| {
            let at = i as f64 * step;
            let index = (at as usize).min(last);
            let next = (index + 1).min(last);
            let frac = (at - index as f64) as f32;
            samples[index] + (samples[next] - samples[index]) * frac
        })
        .collect()
}

pub fn decode_wav(bytes: &[u8]) -> Option<(Vec<f32>, u32)> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut format = None;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        let body_start = at + 8;
        let body_end = body_start.saturating_add(size).min(bytes.len());
        let body = &bytes[body_start..body_end];
        if id == b"fmt " && body.len() >= 16 {
            let mut tag = u16::from_le_bytes([body[0], body[1]]);
            let channels = u16::from_le_bytes([body[2], body[3]]).max(1) as usize;
            let rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
            let bits = u16::from_le_bytes([body[14], body[15]]);
            if tag == 0xFFFE && body.len() >= 26 {
                tag = u16::from_le_bytes([body[24], body[25]]);
            }
            format = Some((tag, channels, rate, bits));
        } else if id == b"data" {
            let (tag, channels, rate, bits) = format?;
            let samples = match (tag, bits) {
                (1, 16) => body.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect(),
                (1, 8) => body.iter().map(|b| (*b as f32 - 128.0) / 128.0).collect(),
                (3, 32) => body.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect(),
                _ => return None,
            };
            return Some((downmix(samples, channels), rate));
        }
        at = body_start.saturating_add(size + (size & 1));
    }
    None
}

fn downmix(samples: Vec<f32>, channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return samples;
    }
    samples.chunks_exact(channels).map(|frame| frame.iter().sum::<f32>() / channels as f32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(tag: u16, channels: u16, rate: u32, bits: u16, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * channels as u32 * bits as u32 / 8).to_le_bytes());
        out.extend_from_slice(&(channels * bits / 8).to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        out
    }

    #[test]
    fn a_speech_wav_decodes_to_mono_samples() {
        let data: Vec<u8> = [16384i16, -16384, 0].iter().flat_map(|s| s.to_le_bytes()).collect();
        let (samples, rate) = decode_wav(&wav(1, 1, 22_050, 16, &data)).unwrap();
        assert_eq!(rate, 22_050);
        assert_eq!(samples, vec![0.5, -0.5, 0.0]);
    }

    #[test]
    fn stereo_and_float_wavs_decode_too() {
        let data: Vec<u8> = [0.5f32, 0.25, -1.0, 0.0].iter().flat_map(|s| s.to_le_bytes()).collect();
        let (samples, rate) = decode_wav(&wav(3, 2, 48_000, 32, &data)).unwrap();
        assert_eq!(rate, 48_000);
        assert_eq!(samples, vec![0.375, -0.5]);
        assert!(decode_wav(b"not a wav file").is_none());
        assert!(decode_wav(&wav(2, 1, 8_000, 4, &[0, 0])).is_none());
    }

    #[test]
    fn resampling_keeps_the_length_in_time() {
        let second: Vec<f32> = vec![0.25; 22_050];
        let out = resample(&second, 22_050.0, SAMPLE_RATE);
        assert_eq!(out.len(), SAMPLE_RATE as usize);
        assert!(out.iter().all(|s| (s - 0.25).abs() < 1e-6));
        assert_eq!(resample(&second, 44_100.0, SAMPLE_RATE).len(), SAMPLE_RATE as usize / 2);
        assert!(resample(&[], 22_050.0, SAMPLE_RATE).is_empty());
        assert!(resample(&second, f32::NAN, SAMPLE_RATE).is_empty());
    }

    #[test]
    fn the_queue_plays_its_clip_then_counts_silence() {
        let mut queue = ClipQueue::new();
        queue.push(&vec![0.5; FRAME_SIZE + 10], SAMPLE_RATE as f32);
        let first = queue.next_frame();
        assert_eq!(first.len(), FRAME_SIZE);
        assert!(first.iter().all(|s| (*s as f32 / i16::MAX as f32 - 0.5).abs() < 1e-3));
        let second = queue.next_frame();
        assert!(second[..10].iter().all(|s| *s > 0) && second[10..].iter().all(|s| *s == 0));
        assert_eq!(queue.idle_frames(), 0);
        queue.next_frame();
        queue.next_frame();
        assert_eq!(queue.idle_frames(), 2);
        queue.push(&[0.1; 48], SAMPLE_RATE as f32);
        assert_eq!(queue.idle_frames(), 0);
        queue.clear();
        assert_eq!(queue.queued(), 0);
    }

    #[test]
    fn the_queue_follows_the_output_volume() {
        let mut queue = ClipQueue::new();
        queue.set_volume(0.0);
        queue.push(&vec![0.5; FRAME_SIZE], SAMPLE_RATE as f32);
        assert!(queue.next_frame().iter().all(|s| *s == 0));
    }

    #[test]
    fn mixed_clips_play_together_and_end_on_their_own() {
        let mut mixer = ClipMixer::new();
        mixer.push(1, &vec![0.25; FRAME_SIZE], SAMPLE_RATE as f32, 1.0);
        mixer.push(2, &vec![0.25; FRAME_SIZE * 2], SAMPLE_RATE as f32, 1.0);
        assert_eq!(mixer.playing(), 2);
        let both = mixer.next_frame();
        let one = mixer.next_frame();
        assert!(both[0] > one[0] && one[0] > 0);
        assert_eq!(mixer.playing(), 0);
        mixer.next_frame();
        assert_eq!(mixer.idle_frames(), 1);
    }

    #[test]
    fn a_mixed_clip_can_be_stopped_or_turned_down() {
        let mut mixer = ClipMixer::new();
        mixer.push(7, &vec![0.5; FRAME_SIZE], SAMPLE_RATE as f32, 0.0);
        assert!(mixer.next_frame().iter().all(|s| *s == 0));
        mixer.push(7, &vec![0.5; FRAME_SIZE], SAMPLE_RATE as f32, 1.0);
        mixer.push(7, &vec![0.5; FRAME_SIZE], SAMPLE_RATE as f32, 1.0);
        assert_eq!(mixer.playing(), 1, "the same clip id replaces itself");
        mixer.stop(7);
        mixer.next_frame();
        assert_eq!(mixer.playing(), 0);
        for id in 0..10 {
            mixer.push(id, &[0.1; 48], SAMPLE_RATE as f32, 1.0);
        }
        assert_eq!(mixer.playing(), MAX_MIXED_CLIPS);
    }

    fn tone(amplitude: f32, len: usize) -> Vec<f32> {
        (0..len).map(|i| amplitude * (i as f32 * 0.05).sin()).collect()
    }

    #[test]
    fn overlapping_alerts_are_turned_down_instead_of_distorted() {
        let mut mixer = ClipMixer::new();
        for id in 0..MAX_MIXED_CLIPS as u64 {
            mixer.push(id, &tone(0.47, FRAME_SIZE * 8), SAMPLE_RATE as f32, 1.0);
        }
        mixer.next_frame();
        let frame = mixer.next_frame();
        let shape = tone(1.0, FRAME_SIZE * 2);
        let scale = frame.iter().map(|s| s.unsigned_abs()).max().unwrap() as f32 / i16::MAX as f32;
        assert!(scale <= 0.9, "{scale}");
        assert!(scale > 0.8, "{scale}");
        for (i, s) in frame.iter().enumerate() {
            let want = shape[FRAME_SIZE + i] * scale;
            assert!((*s as f32 / i16::MAX as f32 - want).abs() < 0.01, "sample {i} is bent");
        }
    }

    #[test]
    fn a_quiet_clip_after_a_loud_mix_comes_back_to_full_level() {
        let mut mixer = ClipMixer::new();
        for id in 0..MAX_MIXED_CLIPS as u64 {
            mixer.push(id, &vec![0.5; FRAME_SIZE], SAMPLE_RATE as f32, 1.0);
        }
        mixer.next_frame();
        mixer.push(9, &vec![0.25; FRAME_SIZE * 200], SAMPLE_RATE as f32, 1.0);
        let mut last = Vec::new();
        for _ in 0..200 {
            last = mixer.next_frame();
        }
        assert!(last.iter().all(|s| (*s as f32 / i16::MAX as f32 - 0.25).abs() < 0.01));
    }

    #[test]
    fn a_stopped_clip_fades_instead_of_clicking() {
        let mut mixer = ClipMixer::new();
        mixer.push(3, &vec![0.5; FRAME_SIZE * 4], SAMPLE_RATE as f32, 1.0);
        mixer.next_frame();
        mixer.stop(3);
        let frame = mixer.next_frame();
        assert!(frame[0] > frame[STOP_FADE_SAMPLES / 2] && frame[STOP_FADE_SAMPLES / 2] > 0);
        assert!(frame[STOP_FADE_SAMPLES - 1..].iter().all(|s| *s == 0));
        assert_eq!(mixer.playing(), 0);
    }
}
