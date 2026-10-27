use crate::SAMPLE_RATE;
use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use std::collections::VecDeque;
use std::f32::consts::PI;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VoiceFilter {
    #[default]
    Off,
    Higher,
    Lower,
    Robot,
    Radio,
    Cave,
    Whisper,
}

impl VoiceFilter {
    pub const ALL: [VoiceFilter; 7] = [
        VoiceFilter::Off,
        VoiceFilter::Higher,
        VoiceFilter::Lower,
        VoiceFilter::Robot,
        VoiceFilter::Radio,
        VoiceFilter::Cave,
        VoiceFilter::Whisper,
    ];

    pub fn name(self) -> &'static str {
        match self {
            VoiceFilter::Off => "off",
            VoiceFilter::Higher => "higher",
            VoiceFilter::Lower => "lower",
            VoiceFilter::Robot => "robot",
            VoiceFilter::Radio => "radio",
            VoiceFilter::Cave => "cave",
            VoiceFilter::Whisper => "whisper",
        }
    }

    pub fn from_name(name: &str) -> Option<VoiceFilter> {
        Self::ALL.into_iter().find(|f| f.name() == name)
    }

    pub fn is_on(self) -> bool {
        self != VoiceFilter::Off
    }
}

const HIGHER_RATIO: f32 = 1.5;
const LOWER_RATIO: f32 = 0.72;
const PITCH_WINDOW: usize = 1920;

const ROBOT_FRAME: usize = 1024;
const ROBOT_PITCH_HZ: u32 = 120;
const ROBOT_HOP: usize = (SAMPLE_RATE / ROBOT_PITCH_HZ) as usize;
const ROBOT_GAIN: f32 = 1.0;

const WHISPER_FRAME: usize = 512;
const WHISPER_HOP: usize = WHISPER_FRAME / 4;
const WHISPER_GAIN: f32 = 2.0;

const RADIO_LOW_CUT_HZ: f32 = 400.0;
const RADIO_HIGH_CUT_HZ: f32 = 3200.0;
const RADIO_DRIVE: f32 = 2.5;
const RADIO_MAKEUP: f32 = 1.6;

const CAVE_ECHOES: [(f32, f32); 2] = [(0.149, 0.5), (0.233, 0.4)];
const CAVE_WET: f32 = 0.45;
const CAVE_DAMPING_HZ: f32 = 2500.0;

pub struct VoiceChanger {
    filter: VoiceFilter,
    effect: Option<Effect>,
}

enum Effect {
    Pitch(PitchShifter),
    Spectral(Spectral),
    Radio(Radio),
    Cave(Cave),
}

impl Default for VoiceChanger {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceChanger {
    pub fn new() -> Self {
        Self { filter: VoiceFilter::Off, effect: None }
    }

    pub fn filter(&self) -> VoiceFilter {
        self.filter
    }

    pub fn set(&mut self, filter: VoiceFilter) {
        if filter == self.filter {
            return;
        }
        self.filter = filter;
        self.effect = Effect::for_filter(filter);
    }

    pub fn clear(&mut self) {
        self.effect = Effect::for_filter(self.filter);
    }

    pub fn process(&mut self, samples: &mut [f32]) {
        let Some(effect) = &mut self.effect else { return };
        for s in samples.iter_mut() {
            if !s.is_finite() {
                *s = 0.0;
            }
        }
        match effect {
            Effect::Pitch(p) => p.process(samples),
            Effect::Spectral(s) => s.process(samples),
            Effect::Radio(r) => r.process(samples),
            Effect::Cave(c) => c.process(samples),
        }
        for s in samples.iter_mut() {
            *s = if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 };
        }
    }
}

impl Effect {
    fn for_filter(filter: VoiceFilter) -> Option<Effect> {
        match filter {
            VoiceFilter::Off => None,
            VoiceFilter::Higher => Some(Effect::Pitch(PitchShifter::new(HIGHER_RATIO))),
            VoiceFilter::Lower => Some(Effect::Pitch(PitchShifter::new(LOWER_RATIO))),
            VoiceFilter::Robot => Some(Effect::Spectral(Spectral::new(ROBOT_FRAME, ROBOT_HOP, Shape::Robot, ROBOT_GAIN))),
            VoiceFilter::Whisper => Some(Effect::Spectral(Spectral::new(WHISPER_FRAME, WHISPER_HOP, Shape::Whisper(Noise::new()), WHISPER_GAIN))),
            VoiceFilter::Radio => Some(Effect::Radio(Radio::new())),
            VoiceFilter::Cave => Some(Effect::Cave(Cave::new())),
        }
    }
}

struct DelayLine {
    buffer: Vec<f32>,
    mask: usize,
    written: usize,
}

impl DelayLine {
    fn new(longest: usize) -> Self {
        let size = (longest + 2).next_power_of_two();
        Self { buffer: vec![0.0; size], mask: size - 1, written: 0 }
    }

    fn push(&mut self, sample: f32) {
        self.buffer[self.written & self.mask] = sample;
        self.written = self.written.wrapping_add(1);
    }

    fn at(&self, delay: usize) -> f32 {
        self.buffer[self.written.wrapping_sub(1 + delay) & self.mask]
    }

    fn between(&self, delay: f32) -> f32 {
        let whole = delay.floor();
        let part = delay - whole;
        let near = self.at(whole as usize);
        let far = self.at(whole as usize + 1);
        near + (far - near) * part
    }
}

struct PitchShifter {
    line: DelayLine,
    phase: f32,
    step: f32,
}

impl PitchShifter {
    fn new(ratio: f32) -> Self {
        Self { line: DelayLine::new(PITCH_WINDOW + 2), phase: 0.0, step: (1.0 - ratio) / PITCH_WINDOW as f32 }
    }

    fn tap(&self, phase: f32) -> f32 {
        let fade = (PI * phase).sin();
        self.line.between(phase * PITCH_WINDOW as f32) * fade * fade
    }

    fn process(&mut self, samples: &mut [f32]) {
        for s in samples.iter_mut() {
            self.line.push(*s);
            let other = (self.phase + 0.5).fract();
            *s = self.tap(self.phase) + self.tap(other);
            self.phase = (self.phase + self.step).rem_euclid(1.0);
        }
    }
}

enum Shape {
    Robot,
    Whisper(Noise),
}

struct Noise(u32);

impl Noise {
    fn new() -> Self {
        Self(0x9e37_79b9)
    }

    fn next_unit(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32
    }
}

struct Spectral {
    shape: Shape,
    hop: usize,
    window: Vec<f32>,
    scale: f32,
    forward: Arc<dyn RealToComplex<f32>>,
    inverse: Arc<dyn ComplexToReal<f32>>,
    recent: Vec<f32>,
    grain: Vec<f32>,
    spectrum: Vec<Complex32>,
    scratch: Vec<Complex32>,
    sum: Vec<f32>,
    arriving: Vec<f32>,
    ready: VecDeque<f32>,
}

impl Spectral {
    fn new(frame: usize, hop: usize, shape: Shape, gain: f32) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(frame);
        let inverse = planner.plan_fft_inverse(frame);
        let window: Vec<f32> = (0..frame).map(|i| (PI * i as f32 / frame as f32).sin().powi(2)).collect();
        let window_power: f32 = window.iter().map(|w| w * w).sum();
        let scale = gain * hop as f32 / (window_power * frame as f32);
        let spectrum = forward.make_output_vec();
        let scratch = forward.make_scratch_vec();
        let mut ready = VecDeque::with_capacity(2 * hop);
        ready.extend(std::iter::repeat_n(0.0, hop));
        Self {
            shape,
            hop,
            window,
            scale,
            forward,
            inverse,
            recent: vec![0.0; frame],
            grain: vec![0.0; frame],
            spectrum,
            scratch,
            sum: vec![0.0; frame],
            arriving: Vec::with_capacity(hop),
            ready,
        }
    }

    fn process(&mut self, samples: &mut [f32]) {
        for s in samples.iter_mut() {
            self.arriving.push(*s);
            if self.arriving.len() == self.hop {
                self.run_frame();
            }
            *s = self.ready.pop_front().unwrap_or(0.0);
        }
    }

    fn run_frame(&mut self) {
        let frame = self.recent.len();
        self.recent.copy_within(self.hop.., 0);
        self.recent[frame - self.hop..].copy_from_slice(&self.arriving);
        self.arriving.clear();
        for ((g, r), w) in self.grain.iter_mut().zip(&self.recent).zip(&self.window) {
            *g = r * w;
        }
        if self.forward.process_with_scratch(&mut self.grain, &mut self.spectrum, &mut self.scratch).is_err() {
            self.grain.fill(0.0);
        } else {
            self.reshape();
            let last = self.spectrum.len() - 1;
            self.spectrum[0].im = 0.0;
            self.spectrum[last].im = 0.0;
            if self.inverse.process_with_scratch(&mut self.spectrum, &mut self.grain, &mut self.scratch).is_err() {
                self.grain.fill(0.0);
            }
        }
        if matches!(self.shape, Shape::Robot) {
            self.grain.rotate_left(frame / 2);
        }
        for ((acc, g), w) in self.sum.iter_mut().zip(&self.grain).zip(&self.window) {
            *acc += g * w * self.scale;
        }
        self.ready.extend(self.sum[..self.hop].iter().copied());
        self.sum.copy_within(self.hop.., 0);
        self.sum[frame - self.hop..].fill(0.0);
    }

    fn reshape(&mut self) {
        match &mut self.shape {
            Shape::Robot => {
                for bin in self.spectrum.iter_mut() {
                    *bin = Complex32::new(bin.norm(), 0.0);
                }
            }
            Shape::Whisper(noise) => {
                for bin in self.spectrum.iter_mut() {
                    *bin = Complex32::from_polar(bin.norm(), 2.0 * PI * noise.next_unit());
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    const BUTTERWORTH_Q: f32 = std::f32::consts::FRAC_1_SQRT_2;

    fn from_coefficients(b: [f32; 3], a: [f32; 3]) -> Self {
        Self { b0: b[0] / a[0], b1: b[1] / a[0], b2: b[2] / a[0], a1: a[1] / a[0], a2: a[2] / a[0], x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0 }
    }

    fn corner(freq: f32) -> (f32, f32) {
        let w = 2.0 * PI * freq / SAMPLE_RATE as f32;
        (w.cos(), w.sin() / (2.0 * Self::BUTTERWORTH_Q))
    }

    fn low_pass(freq: f32) -> Self {
        let (cos, alpha) = Self::corner(freq);
        let b1 = 1.0 - cos;
        Self::from_coefficients([b1 / 2.0, b1, b1 / 2.0], [1.0 + alpha, -2.0 * cos, 1.0 - alpha])
    }

    fn high_pass(freq: f32) -> Self {
        let (cos, alpha) = Self::corner(freq);
        let b1 = -(1.0 + cos);
        Self::from_coefficients([-b1 / 2.0, b1, -b1 / 2.0], [1.0 + alpha, -2.0 * cos, 1.0 - alpha])
    }

    fn next(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

struct Radio {
    band: [Biquad; 4],
}

impl Radio {
    fn new() -> Self {
        let low = Biquad::high_pass(RADIO_LOW_CUT_HZ);
        let high = Biquad::low_pass(RADIO_HIGH_CUT_HZ);
        Self { band: [low, low, high, high] }
    }

    fn process(&mut self, samples: &mut [f32]) {
        for s in samples.iter_mut() {
            let banded = self.band.iter_mut().fold(*s, |x, filter| filter.next(x));
            *s = (RADIO_DRIVE * banded).tanh() / RADIO_DRIVE * RADIO_MAKEUP;
        }
    }
}

struct Echo {
    line: DelayLine,
    delay: usize,
    feedback: f32,
    damped: f32,
    damping: f32,
}

impl Echo {
    fn new(seconds: f32, feedback: f32) -> Self {
        let delay = (seconds * SAMPLE_RATE as f32) as usize;
        let damping = 1.0 - (-2.0 * PI * CAVE_DAMPING_HZ / SAMPLE_RATE as f32).exp();
        Self { line: DelayLine::new(delay), delay, feedback, damped: 0.0, damping }
    }

    fn next(&mut self, x: f32) -> f32 {
        let returned = self.line.at(self.delay - 1);
        self.damped += self.damping * (returned - self.damped);
        self.line.push(x + self.feedback * self.damped);
        self.damped
    }
}

struct Cave {
    echoes: Vec<Echo>,
}

impl Cave {
    fn new() -> Self {
        Self { echoes: CAVE_ECHOES.iter().map(|(seconds, feedback)| Echo::new(*seconds, *feedback)).collect() }
    }

    fn process(&mut self, samples: &mut [f32]) {
        let share = CAVE_WET / self.echoes.len() as f32;
        for s in samples.iter_mut() {
            let wet: f32 = self.echoes.iter_mut().map(|echo| echo.next(*s)).sum();
            *s += share * wet;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{level_db, FRAME_SIZE};

    fn tone(freq: f32, amplitude: f32, frames: usize) -> Vec<f32> {
        (0..frames * FRAME_SIZE)
            .map(|i| amplitude * (2.0 * PI * freq * i as f32 / SAMPLE_RATE as f32).sin())
            .collect()
    }

    fn voice_like(frames: usize) -> Vec<f32> {
        (0..frames * FRAME_SIZE)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                let phase = 2.0 * PI * 150.0 * t - (20.0 / 3.0) * (2.0 * PI * 3.0 * t).cos();
                (1..=10).map(|h| (h as f32 * phase).sin() / h as f32).sum::<f32>() * 0.15
            })
            .collect()
    }

    fn run(filter: VoiceFilter, input: &[f32]) -> Vec<f32> {
        let mut changer = VoiceChanger::new();
        changer.set(filter);
        let mut out = input.to_vec();
        for frame in out.chunks_mut(FRAME_SIZE) {
            changer.process(frame);
        }
        out
    }

    fn strongest_frequency(samples: &[f32]) -> f32 {
        let n = samples.len();
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(n);
        let mut input: Vec<f32> = samples.iter().enumerate().map(|(i, s)| s * (PI * i as f32 / n as f32).sin().powi(2)).collect();
        let mut spectrum = fft.make_output_vec();
        fft.process(&mut input, &mut spectrum).unwrap();
        let (bin, _) = spectrum.iter().enumerate().skip(1).fold((0, 0.0f32), |best, (i, c)| if c.norm() > best.1 { (i, c.norm()) } else { best });
        bin as f32 * SAMPLE_RATE as f32 / n as f32
    }

    fn periodicity(samples: &[f32]) -> f32 {
        let (shortest, longest) = (SAMPLE_RATE as usize / 400, SAMPLE_RATE as usize / 80);
        let blocks: Vec<&[f32]> = samples.chunks_exact(4 * longest).collect();
        let best = |block: &[f32]| {
            let (head, _) = block.split_at(2 * longest);
            let energy: f32 = head.iter().map(|s| s * s).sum::<f32>().max(1e-9);
            (shortest..longest).map(|lag| head.iter().zip(&block[lag..]).map(|(a, b)| a * b).sum::<f32>() / energy).fold(0.0, f32::max)
        };
        blocks.iter().map(|b| best(b)).sum::<f32>() / blocks.len() as f32
    }

    fn settled(samples: &[f32]) -> &[f32] {
        &samples[samples.len() / 2..]
    }

    #[test]
    fn every_filter_has_a_name_that_reads_back() {
        for filter in VoiceFilter::ALL {
            assert_eq!(VoiceFilter::from_name(filter.name()), Some(filter));
        }
        assert_eq!(VoiceFilter::from_name("squeaky"), None);
        assert_eq!(VoiceFilter::default(), VoiceFilter::Off);
        assert!(!VoiceFilter::Off.is_on());
        assert!(VoiceFilter::Robot.is_on());
    }

    #[test]
    fn off_leaves_the_voice_untouched() {
        let input = voice_like(20);
        assert_eq!(run(VoiceFilter::Off, &input), input);
    }

    #[test]
    fn higher_and_lower_move_the_pitch() {
        let input = tone(400.0, 0.4, 100);
        let higher = strongest_frequency(settled(&run(VoiceFilter::Higher, &input)));
        let lower = strongest_frequency(settled(&run(VoiceFilter::Lower, &input)));
        assert!((higher - 400.0 * HIGHER_RATIO).abs() < 15.0, "higher landed on {higher} Hz");
        assert!((lower - 400.0 * LOWER_RATIO).abs() < 15.0, "lower landed on {lower} Hz");
    }

    #[test]
    fn pitch_filters_keep_the_voice_level() {
        let input = voice_like(200);
        let before = level_db(settled(&input));
        for filter in [VoiceFilter::Higher, VoiceFilter::Lower] {
            let after = level_db(settled(&run(filter, &input)));
            assert!((after - before).abs() < 3.0, "{filter:?} moved the level {:.1} dB", after - before);
        }
    }

    #[test]
    fn robot_speaks_on_one_steady_note() {
        let out = run(VoiceFilter::Robot, &voice_like(200));
        let note = strongest_frequency(settled(&out));
        let harmonic = (note / ROBOT_PITCH_HZ as f32).round().max(1.0);
        assert!((note - harmonic * ROBOT_PITCH_HZ as f32).abs() < 4.0, "robot hummed at {note} Hz");
        let gliding = run(VoiceFilter::Robot, &tone(333.0, 0.3, 200));
        let held = strongest_frequency(settled(&gliding));
        let nearest = (held / ROBOT_PITCH_HZ as f32).round() * ROBOT_PITCH_HZ as f32;
        assert!((held - nearest).abs() < 4.0, "a 333 Hz voice came out at {held} Hz");
    }

    #[test]
    fn radio_keeps_the_middle_and_drops_the_ends() {
        let keep = |freq: f32| {
            let input = tone(freq, 0.2, 100);
            level_db(settled(&run(VoiceFilter::Radio, &input))) - level_db(settled(&input))
        };
        assert!(keep(1000.0).abs() < 4.0, "1 kHz changed {:.1} dB", keep(1000.0));
        assert!(keep(80.0) < -20.0, "80 Hz only dropped {:.1} dB", keep(80.0));
        assert!(keep(9000.0) < -20.0, "9 kHz only dropped {:.1} dB", keep(9000.0));
    }

    #[test]
    fn cave_echoes_after_you_stop_and_then_dies_away() {
        let mut input = voice_like(30);
        input.extend(vec![0.0; 400 * FRAME_SIZE]);
        let out = run(VoiceFilter::Cave, &input);
        let window = |from_s: f32, to_s: f32| {
            let at = |s: f32| (s * SAMPLE_RATE as f32) as usize;
            level_db(&out[at(from_s)..at(to_s)])
        };
        let speaking = window(0.0, 0.3);
        assert!(window(0.35, 0.6) > speaking - 20.0, "no echo after you stop");
        assert!(window(3.5, 4.0) < speaking - 50.0, "the echo never died away");
    }

    #[test]
    fn whisper_keeps_the_level_but_drops_the_note() {
        let input = voice_like(200);
        let out = run(VoiceFilter::Whisper, &input);
        let change = level_db(settled(&out)) - level_db(settled(&input));
        assert!(change.abs() < 4.0, "whisper moved the level {change:.1} dB");
        let (heard, said) = (periodicity(settled(&out)), periodicity(settled(&input)));
        assert!(said > 0.8 && heard < 0.4, "whisper still carries the note: {heard:.2} against {said:.2}");
    }

    #[test]
    fn every_filter_stays_in_range_and_survives_broken_samples() {
        for filter in VoiceFilter::ALL.into_iter().filter(|f| f.is_on()) {
            let mut input = tone(220.0, 1.0, 50);
            input[100] = f32::NAN;
            input[200] = f32::INFINITY;
            let mut changer = VoiceChanger::new();
            changer.set(filter);
            for frame in input.chunks_mut(FRAME_SIZE) {
                changer.process(frame);
                assert!(frame.iter().all(|s| s.is_finite() && (-1.0..=1.0).contains(s)), "{filter:?}");
            }
        }
    }

    #[test]
    fn clearing_forgets_what_was_said_before() {
        for filter in [VoiceFilter::Cave, VoiceFilter::Robot, VoiceFilter::Higher] {
            let mut changer = VoiceChanger::new();
            changer.set(filter);
            let mut talk = voice_like(20);
            for frame in talk.chunks_mut(FRAME_SIZE) {
                changer.process(frame);
            }
            changer.clear();
            let mut quiet = vec![0.0f32; FRAME_SIZE];
            changer.process(&mut quiet);
            assert!(quiet.iter().all(|s| *s == 0.0), "{filter:?} leaked old audio after clearing");
            assert_eq!(changer.filter(), filter);
        }
    }
}
