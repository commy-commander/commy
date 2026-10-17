use nnnoiseless::DenoiseState;
use thiserror::Error;

pub const FRAME_SIZE: usize = DenoiseState::FRAME_SIZE;
pub const SAMPLE_RATE: u32 = 48_000;

const RNNOISE_I16_FULL_SCALE: f32 = 32767.0;

pub mod codec;

#[derive(Debug, Error, PartialEq)]
pub enum AudioError {
    #[error("expected a frame of {FRAME_SIZE} samples, got {0}")]
    BadFrameSize(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedFrame {
    pub samples: Vec<f32>,
    pub voice_probability: f32,
    pub should_transmit: bool,
}

pub struct VoiceProcessor {
    denoiser: Box<DenoiseState<'static>>,
    noise_cancellation: bool,
    muted: bool,
    input_gain: f32,
    vad_threshold: f32,
    gate: VoiceGate,
}

impl Default for VoiceProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceProcessor {
    pub fn new() -> Self {
        Self {
            denoiser: DenoiseState::new(),
            noise_cancellation: true,
            muted: false,
            input_gain: 1.0,
            vad_threshold: 0.0,
            gate: VoiceGate::default(),
        }
    }

    pub fn set_noise_cancellation(&mut self, enabled: bool) {
        if enabled && !self.noise_cancellation {
            self.denoiser = DenoiseState::new();
            self.gate = VoiceGate::default();
        }
        self.noise_cancellation = enabled;
    }

    pub fn noise_cancellation(&self) -> bool {
        self.noise_cancellation
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    pub fn set_input_gain(&mut self, gain: f32) {
        self.input_gain = gain.clamp(0.0, 4.0);
    }

    pub fn set_vad_threshold(&mut self, threshold: f32) {
        self.vad_threshold = threshold.clamp(0.0, 1.0);
    }

    pub fn process(&mut self, input: &[f32]) -> Result<ProcessedFrame, AudioError> {
        if input.len() != FRAME_SIZE {
            return Err(AudioError::BadFrameSize(input.len()));
        }

        if self.muted {
            return Ok(ProcessedFrame {
                samples: vec![0.0; FRAME_SIZE],
                voice_probability: 0.0,
                should_transmit: false,
            });
        }

        let mut scaled = [0.0f32; FRAME_SIZE];
        for (dst, src) in scaled.iter_mut().zip(input) {
            *dst = (src * self.input_gain).clamp(-1.0, 1.0) * RNNOISE_I16_FULL_SCALE;
        }

        if !self.noise_cancellation {
            let samples = scaled.iter().map(|s| s / RNNOISE_I16_FULL_SCALE).collect();
            return Ok(ProcessedFrame {
                samples,
                voice_probability: 0.0,
                should_transmit: true,
            });
        }

        let mut denoised = [0.0f32; FRAME_SIZE];
        let vad = self.denoiser.process_frame(&mut denoised, &scaled);
        let mut samples: Vec<f32> = denoised
            .iter()
            .map(|s| (s / RNNOISE_I16_FULL_SCALE).clamp(-1.0, 1.0))
            .collect();
        self.gate.apply(&mut samples, vad);
        Ok(ProcessedFrame {
            samples,
            voice_probability: vad,
            should_transmit: self.vad_threshold == 0.0 || vad >= self.vad_threshold,
        })
    }
}

#[derive(Debug, Clone)]
pub struct VoiceGate {
    gain: f32,
    hold: u32,
}

impl Default for VoiceGate {
    fn default() -> Self {
        Self { gain: 1.0, hold: Self::HOLD_FRAMES }
    }
}

impl VoiceGate {
    const OPEN_AT: f32 = 0.7;
    const HOLD_FRAMES: u32 = 30;
    const RELEASE_PER_FRAME: f32 = 0.25;

    pub fn apply(&mut self, samples: &mut [f32], voice_probability: f32) {
        if voice_probability >= Self::OPEN_AT {
            self.hold = Self::HOLD_FRAMES;
        } else {
            self.hold = self.hold.saturating_sub(1);
        }
        let target = if self.hold > 0 { 1.0 } else { 0.0 };
        let end = if target > self.gain { target } else { (self.gain - Self::RELEASE_PER_FRAME).max(target) };
        let start = self.gain;
        let n = samples.len().max(1) as f32;
        for (i, s) in samples.iter_mut().enumerate() {
            *s *= start + (end - start) * (i as f32 + 1.0) / n;
        }
        self.gain = end;
    }

    pub fn is_open(&self) -> bool {
        self.gain > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_holds_then_fades_to_silence() {
        let mut gate = VoiceGate::default();
        let frame = || vec![0.1f32; FRAME_SIZE];
        let mut f = frame();
        gate.apply(&mut f, 0.9);
        assert!(f.iter().all(|s| (*s - 0.1).abs() < 1e-6));
        for _ in 0..VoiceGate::HOLD_FRAMES - 1 {
            let mut f = frame();
            gate.apply(&mut f, 0.0);
            assert!((f[FRAME_SIZE - 1] - 0.1).abs() < 1e-6);
        }
        let mut faded = Vec::new();
        for _ in 0..6 {
            let mut f = frame();
            gate.apply(&mut f, 0.0);
            faded.extend(f);
        }
        assert!(faded.windows(2).all(|w| (w[0] - w[1]).abs() < 0.001), "no clicks");
        assert_eq!(*faded.last().unwrap(), 0.0);
        assert!(!gate.is_open());
        let mut f = frame();
        gate.apply(&mut f, 0.8);
        assert!((f[FRAME_SIZE - 1] - 0.1).abs() < 1e-6);
    }

    fn sine_frame(freq: f32, amplitude: f32) -> Vec<f32> {
        (0..FRAME_SIZE)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                amplitude * (2.0 * std::f32::consts::PI * freq * t).sin()
            })
            .collect()
    }

    #[test]
    fn rejects_wrong_frame_size() {
        let mut p = VoiceProcessor::new();
        assert_eq!(p.process(&[0.0; 100]), Err(AudioError::BadFrameSize(100)));
    }

    #[test]
    fn bypass_preserves_signal() {
        let mut p = VoiceProcessor::new();
        p.set_noise_cancellation(false);
        let frame = sine_frame(440.0, 0.5);
        let out = p.process(&frame).unwrap();
        assert!(out.should_transmit);
        assert_eq!(out.samples.len(), FRAME_SIZE);
        for (a, b) in frame.iter().zip(&out.samples) {
            assert!((a - b).abs() < 1e-3, "bypass altered the signal");
        }
    }

    #[test]
    fn denoised_output_is_sane() {
        let mut p = VoiceProcessor::new();
        assert!(p.noise_cancellation());
        let frame = sine_frame(300.0, 0.4);
        let mut out = p.process(&frame).unwrap();
        for _ in 0..20 {
            out = p.process(&frame).unwrap();
        }
        assert_eq!(out.samples.len(), FRAME_SIZE);
        assert!(out.samples.iter().all(|s| s.is_finite()));
        assert!(out.samples.iter().all(|s| (-1.0..=1.0).contains(s)));
        assert!((0.0..=1.0).contains(&out.voice_probability));
    }

    #[test]
    fn mute_outputs_silence() {
        let mut p = VoiceProcessor::new();
        p.set_muted(true);
        let out = p.process(&sine_frame(440.0, 0.5)).unwrap();
        assert!(!out.should_transmit);
        assert!(out.samples.iter().all(|s| *s == 0.0));
        p.set_muted(false);
        assert!(p.process(&sine_frame(440.0, 0.5)).unwrap().should_transmit);
    }

    #[test]
    fn gain_is_clamped() {
        let mut p = VoiceProcessor::new();
        p.set_noise_cancellation(false);
        p.set_input_gain(100.0);
        let out = p.process(&sine_frame(440.0, 1.0)).unwrap();
        assert!(out.samples.iter().all(|s| (-1.0..=1.0).contains(s)));
    }

    #[test]
    fn toggle_resets_cleanly() {
        let mut p = VoiceProcessor::new();
        let frame = sine_frame(200.0, 0.3);
        p.process(&frame).unwrap();
        p.set_noise_cancellation(false);
        p.process(&frame).unwrap();
        p.set_noise_cancellation(true);
        let out = p.process(&frame).unwrap();
        assert!(out.samples.iter().all(|s| s.is_finite()));
    }
}
