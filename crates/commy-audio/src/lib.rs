use nnnoiseless::DenoiseState;
use thiserror::Error;

pub const FRAME_SIZE: usize = DenoiseState::FRAME_SIZE;
pub const SAMPLE_RATE: u32 = 48_000;

const RNNOISE_I16_FULL_SCALE: f32 = 32767.0;

pub mod codec;
pub mod echo;
pub mod level;
pub mod ogg;
pub mod speaking;

#[derive(Debug, Error, PartialEq)]
pub enum AudioError {
    #[error("expected a frame of {FRAME_SIZE} samples, got {0}")]
    BadFrameSize(usize),
}

pub const SILENCE_DB: f32 = -100.0;

fn full_band_pipeline() -> sonora::config::Pipeline {
    sonora::config::Pipeline {
        maximum_internal_processing_rate: sonora::config::MaxProcessingRate::Rate48kHz,
        ..Default::default()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedFrame {
    pub samples: Vec<f32>,
    pub voice_probability: f32,
    pub should_transmit: bool,
    pub input_level_db: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sensitivity {
    Automatic,
    Threshold(f32),
}

pub struct VoiceProcessor {
    denoiser: Box<DenoiseState<'static>>,
    noise_cancellation: bool,
    echo: Option<echo::EchoCanceller>,
    far_end: echo::FarEndInbox,
    level: Option<level::LevelControl>,
    muted: bool,
    push_to_talk: bool,
    talk_key_down: bool,
    input_gain: f32,
    sensitivity: Sensitivity,
    last_level_db: f32,
    gate: VoiceGate,
}

pub fn level_db(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return SILENCE_DB;
    }
    let mean_square = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
    if mean_square <= 0.0 {
        return SILENCE_DB;
    }
    (10.0 * mean_square.log10()).max(SILENCE_DB)
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
            echo: Some(echo::EchoCanceller::new()),
            far_end: echo::FarEndInbox::new(),
            level: Some(level::LevelControl::new()),
            muted: false,
            push_to_talk: false,
            talk_key_down: false,
            input_gain: 1.0,
            sensitivity: Sensitivity::Automatic,
            last_level_db: SILENCE_DB,
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

    pub fn set_echo_cancellation(&mut self, enabled: bool) {
        if enabled && self.echo.is_none() {
            self.echo = Some(echo::EchoCanceller::new());
        } else if !enabled {
            self.echo = None;
        }
    }

    pub fn set_auto_level(&mut self, enabled: bool) {
        if enabled && self.level.is_none() {
            self.level = Some(level::LevelControl::new());
        } else if !enabled {
            self.level = None;
        }
    }

    pub fn auto_level(&self) -> bool {
        self.level.is_some()
    }

    pub fn start_fresh(&mut self) {
        self.far_end.discard();
        if self.echo.is_some() {
            self.echo = Some(echo::EchoCanceller::new());
        }
        if self.level.is_some() {
            self.level = Some(level::LevelControl::new());
        }
        self.denoiser = DenoiseState::new();
        self.gate = VoiceGate::default();
    }

    pub fn echo_cancellation(&self) -> bool {
        self.echo.is_some()
    }

    pub fn far_end_sink(&self) -> echo::FarEndSink {
        self.far_end.sink()
    }

    fn cancel_echo(&mut self, gained: &mut [f32; FRAME_SIZE]) {
        match &mut self.echo {
            Some(canceller) => {
                self.far_end.drain_into(canceller);
                let mut cleared = [0.0f32; FRAME_SIZE];
                canceller.process(gained, &mut cleared);
                gained.copy_from_slice(&cleared);
            }
            None => self.far_end.discard(),
        }
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    pub fn set_input_gain(&mut self, gain: f32) {
        self.input_gain = gain.clamp(0.0, 4.0);
    }

    pub fn set_push_to_talk(&mut self, enabled: bool) {
        self.push_to_talk = enabled;
        if !enabled {
            self.talk_key_down = false;
        }
    }

    pub fn push_to_talk(&self) -> bool {
        self.push_to_talk
    }

    pub fn set_talk_key_down(&mut self, down: bool) {
        self.talk_key_down = down;
    }

    pub fn set_sensitivity(&mut self, sensitivity: Sensitivity) {
        self.sensitivity = sensitivity;
    }

    pub fn sensitivity(&self) -> Sensitivity {
        self.sensitivity
    }

    pub fn last_level_db(&self) -> f32 {
        self.last_level_db
    }

    fn held_closed(&self) -> bool {
        self.muted || (self.push_to_talk && !self.talk_key_down)
    }

    fn gate_opens(&self, level: f32, vad: Option<f32>) -> bool {
        match (self.sensitivity, vad) {
            (Sensitivity::Threshold(db), _) => level >= db,
            (Sensitivity::Automatic, Some(vad)) => vad >= VoiceGate::OPEN_AT,
            (Sensitivity::Automatic, None) => true,
        }
    }

    pub fn process(&mut self, input: &[f32]) -> Result<ProcessedFrame, AudioError> {
        if input.len() != FRAME_SIZE {
            return Err(AudioError::BadFrameSize(input.len()));
        }

        let mut gained = [0.0f32; FRAME_SIZE];
        for (raw, src) in gained.iter_mut().zip(input) {
            *raw = if src.is_finite() { (src * self.input_gain).clamp(-1.0, 1.0) } else { 0.0 };
        }
        self.cancel_echo(&mut gained);
        let mut scaled = [0.0f32; FRAME_SIZE];
        for (dst, raw) in scaled.iter_mut().zip(&gained) {
            *dst = raw * RNNOISE_I16_FULL_SCALE;
        }
        let input_level_db = level_db(&gained);
        self.last_level_db = input_level_db;

        if self.held_closed() {
            return Ok(ProcessedFrame {
                samples: vec![0.0; FRAME_SIZE],
                voice_probability: 0.0,
                should_transmit: false,
                input_level_db,
            });
        }

        let (mut samples, vad) = if self.noise_cancellation {
            let mut denoised = [0.0f32; FRAME_SIZE];
            let mut vad = self.denoiser.process_frame(&mut denoised, &scaled);
            if !vad.is_finite() || denoised.iter().any(|s| !s.is_finite()) {
                self.denoiser = DenoiseState::new();
                denoised = [0.0; FRAME_SIZE];
                vad = 0.0;
            }
            let samples = denoised
                .iter()
                .map(|s| (s / RNNOISE_I16_FULL_SCALE).clamp(-1.0, 1.0))
                .collect();
            (samples, Some(vad))
        } else {
            (gained.to_vec(), None)
        };
        if let Some(level) = &mut self.level {
            level.process(&mut samples);
        }
        let open = self.gate_opens(input_level_db, vad);
        if vad.is_some() || self.sensitivity != Sensitivity::Automatic {
            self.gate.apply(&mut samples, open);
        }
        Ok(ProcessedFrame {
            samples,
            voice_probability: vad.unwrap_or(0.0),
            should_transmit: true,
            input_level_db,
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

    pub fn apply(&mut self, samples: &mut [f32], open: bool) {
        if open {
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
        gate.apply(&mut f, true);
        assert!(f.iter().all(|s| (*s - 0.1).abs() < 1e-6));
        for _ in 0..VoiceGate::HOLD_FRAMES - 1 {
            let mut f = frame();
            gate.apply(&mut f, false);
            assert!((f[FRAME_SIZE - 1] - 0.1).abs() < 1e-6);
        }
        let mut faded = Vec::new();
        for _ in 0..6 {
            let mut f = frame();
            gate.apply(&mut f, false);
            faded.extend(f);
        }
        assert!(faded.windows(2).all(|w| (w[0] - w[1]).abs() < 0.001), "no clicks");
        assert_eq!(*faded.last().unwrap(), 0.0);
        assert!(!gate.is_open());
        let mut f = frame();
        gate.apply(&mut f, true);
        assert!((f[FRAME_SIZE - 1] - 0.1).abs() < 1e-6);
    }

    #[test]
    fn push_to_talk_sends_only_while_the_key_is_down() {
        let mut p = VoiceProcessor::new();
        p.set_noise_cancellation(false);
        p.set_echo_cancellation(false);
        p.set_push_to_talk(true);
        let frame = sine_frame(440.0, 0.5);
        let held = p.process(&frame).unwrap();
        assert!(!held.should_transmit);
        assert!(held.input_level_db > -20.0, "the meter still reads while the key is up");
        p.set_talk_key_down(true);
        assert!(p.process(&frame).unwrap().should_transmit);
        p.set_talk_key_down(false);
        assert!(!p.process(&frame).unwrap().should_transmit);
        p.set_push_to_talk(false);
        assert!(p.process(&frame).unwrap().should_transmit);
    }

    #[test]
    fn a_manual_threshold_gates_on_level_even_without_noise_cancellation() {
        let mut p = VoiceProcessor::new();
        p.set_noise_cancellation(false);
        p.set_echo_cancellation(false);
        p.set_sensitivity(Sensitivity::Threshold(-20.0));
        let quiet = sine_frame(440.0, 0.01);
        let loud = sine_frame(440.0, 0.5);
        for _ in 0..(VoiceGate::HOLD_FRAMES + 10) {
            p.process(&quiet).unwrap();
        }
        let out = p.process(&quiet).unwrap();
        assert!(out.should_transmit);
        assert!(out.samples.iter().all(|s| *s == 0.0), "below the threshold the gate is shut");
        assert!(out.input_level_db < -30.0);
        let out = p.process(&loud).unwrap();
        assert!(out.samples.iter().any(|s| s.abs() > 0.1), "above the threshold voice passes");
        assert!(p.last_level_db() > -15.0);
        p.set_sensitivity(Sensitivity::Automatic);
        let out = p.process(&quiet).unwrap();
        assert!(out.samples.iter().any(|s| *s != 0.0), "automatic without noise cancellation passes everything");
    }

    #[test]
    fn level_is_in_decibels_full_scale() {
        assert_eq!(level_db(&[0.0; 10]), SILENCE_DB);
        assert_eq!(level_db(&[]), SILENCE_DB);
        assert!((level_db(&[1.0; 10]) - 0.0).abs() < 1e-4);
        assert!((level_db(&[0.1; 10]) + 20.0).abs() < 1e-3);
    }

    fn sine_frame(freq: f32, amplitude: f32) -> Vec<f32> {
        (0..FRAME_SIZE)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                amplitude * (2.0 * std::f32::consts::PI * freq * t).sin()
            })
            .collect()
    }

    fn talking(amplitude: f32) -> impl FnMut() -> Vec<f32> {
        let mut n = 0usize;
        move || {
            let frame = (0..FRAME_SIZE)
                .map(|i| {
                    let t = (n * FRAME_SIZE + i) as f64 / f64::from(SAMPLE_RATE);
                    let pitch = 160.0 + 25.0 * (std::f64::consts::TAU * 2.3 * t).sin();
                    let envelope = (0.5 + 0.5 * (std::f64::consts::TAU * 0.4 * t).sin()).powi(2);
                    let harmonics: f64 = (1..=8).map(|h| (h as f64 * std::f64::consts::TAU * pitch * t).sin() / h as f64).sum();
                    (f64::from(amplitude) * envelope * harmonics) as f32
                })
                .collect();
            n += 1;
            frame
        }
    }

    fn sent_level_db(auto_level: bool, amplitude: f32) -> (f32, f32) {
        let mut p = VoiceProcessor::new();
        p.set_auto_level(auto_level);
        let mut speech = talking(amplitude);
        let (mut power, mut peak) = (0.0f64, 0.0f32);
        for frame in 0..1500 {
            let out = p.process(&speech()).unwrap();
            if frame >= 1000 {
                power += out.samples.iter().map(|s| f64::from(s * s)).sum::<f64>();
                peak = out.samples.iter().fold(peak, |m, s| m.max(s.abs()));
            }
        }
        ((10.0 * (power / (500 * FRAME_SIZE) as f64).log10()) as f32, peak)
    }

    #[test]
    fn automatic_level_is_on_by_default_and_brings_a_quiet_mic_up() {
        assert!(VoiceProcessor::new().auto_level());
        let (plain, _) = sent_level_db(false, 0.01);
        let (levelled, peak) = sent_level_db(true, 0.01);
        assert!(levelled - plain > 8.0, "a quiet mic only came up {:.1} dB", levelled - plain);
        assert!(peak < 1.0);
    }

    #[test]
    fn automatic_level_does_not_push_a_loud_mic_into_clipping() {
        let (plain, _) = sent_level_db(false, 0.4);
        let (levelled, peak) = sent_level_db(true, 0.4);
        assert!((levelled - plain).abs() < 3.0, "a loud mic moved {:.1} dB", levelled - plain);
        assert!(peak <= 1.0);
    }

    #[test]
    fn automatic_level_can_be_turned_off_and_survives_a_fresh_start() {
        let mut p = VoiceProcessor::new();
        p.set_auto_level(false);
        p.start_fresh();
        assert!(!p.auto_level());
        p.set_auto_level(true);
        p.start_fresh();
        assert!(p.auto_level());
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
        p.set_echo_cancellation(false);
        p.set_auto_level(false);
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
        p.set_echo_cancellation(false);
        p.set_input_gain(100.0);
        let out = p.process(&sine_frame(440.0, 1.0)).unwrap();
        assert!(out.samples.iter().all(|s| (-1.0..=1.0).contains(s)));
    }

    #[test]
    fn echo_cancellation_is_on_by_default_and_cancels_what_the_speakers_play() {
        let mut p = VoiceProcessor::new();
        assert!(p.echo_cancellation());
        p.set_noise_cancellation(false);
        p.set_auto_level(false);
        let sink = p.far_end_sink();
        let mut rng = 12345u64;
        let mut noise = move || {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((rng >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
        };
        const HISTORY: usize = 8192;
        let mut history = vec![0.0f32; HISTORY];
        let mut written = 0usize;
        let mut echo_power = 0.0f64;
        let mut out_power = 0.0f64;
        for frame in 0..700 {
            let far: Vec<f32> = (0..FRAME_SIZE).map(|_| 0.3 * noise()).collect();
            let near: Vec<f32> = far
                .iter()
                .map(|x| {
                    history[written % HISTORY] = *x;
                    let echo = 0.5 * history[(written + HISTORY - 2400) % HISTORY];
                    written += 1;
                    echo
                })
                .collect();
            let pcm: Vec<i16> = far.iter().map(|s| (s * 32767.0) as i16).collect();
            sink.push(&pcm);
            let out = p.process(&near).unwrap();
            if frame >= 500 {
                echo_power += near.iter().map(|s| f64::from(s * s)).sum::<f64>();
                out_power += out.samples.iter().map(|s| f64::from(s * s)).sum::<f64>();
            }
        }
        let erle = 10.0 * (echo_power / out_power.max(1e-12)).log10();
        assert!(erle > 25.0, "ERLE through the processor {erle:.1} dB");
        p.set_echo_cancellation(false);
        assert!(!p.echo_cancellation());
        let out = p.process(&vec![0.1; FRAME_SIZE]).unwrap();
        assert!(out.samples.iter().all(|s| (s - 0.1).abs() < 1e-3), "off means untouched");
        p.set_echo_cancellation(true);
        assert!(p.echo_cancellation());
    }

    #[test]
    fn broken_mic_samples_do_not_silence_the_mic_for_good() {
        let mut p = VoiceProcessor::new();
        p.set_sensitivity(Sensitivity::Threshold(-60.0));
        let mut broken = sine_frame(300.0, 0.4);
        broken[3] = f32::NAN;
        broken[4] = f32::INFINITY;
        for _ in 0..5 {
            let out = p.process(&broken).unwrap();
            assert!(out.samples.iter().all(|s| s.is_finite()));
        }
        let frame = sine_frame(300.0, 0.4);
        let mut out = p.process(&frame).unwrap();
        for _ in 0..50 {
            out = p.process(&frame).unwrap();
        }
        assert!(out.samples.iter().all(|s| s.is_finite()));
        assert!(out.input_level_db > -20.0, "the meter still reads the mic");
    }

    #[test]
    fn starting_fresh_drops_queued_playback_and_keeps_the_settings() {
        let mut p = VoiceProcessor::new();
        p.set_noise_cancellation(false);
        let sink = p.far_end_sink();
        for _ in 0..10 {
            sink.push(&[8000; FRAME_SIZE]);
        }
        p.start_fresh();
        assert!(p.echo_cancellation());
        assert!(!p.noise_cancellation());
        let frame = sine_frame(440.0, 0.3);
        let mut out = p.process(&frame).unwrap();
        for _ in 0..20 {
            out = p.process(&frame).unwrap();
        }
        assert!(out.should_transmit);
        assert!(level_db(&out.samples) > -20.0, "the voice passes after a fresh start");
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
