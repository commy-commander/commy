use crate::{FRAME_SIZE, SAMPLE_RATE};
use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use std::collections::VecDeque;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::Duration;

const BLOCK: usize = 2 * FRAME_SIZE;
const BINS: usize = BLOCK / 2 + 1;
pub const TAIL_PARTITIONS: usize = 16;
pub const TAIL_SAMPLES: usize = TAIL_PARTITIONS * FRAME_SIZE;
pub const MAX_DELAY_MS: usize = 600;
pub const MAX_DELAY_SAMPLES: usize = SAMPLE_RATE as usize * MAX_DELAY_MS / 1000;
const FAR_RING: usize = 1 << 16;
const FAR_RING_MASK: usize = FAR_RING - 1;
const _: () = assert!(FAR_RING >= MAX_DELAY_SAMPLES + TAIL_SAMPLES + BLOCK);

const I16_SCALE: f32 = 32768.0;
const QUEUE_PRIME_FRAMES: usize = 4;
const QUEUE_MAX_FRAMES: usize = 30;
const SINK_CAPACITY: usize = 64;

const PEAK_MARGIN: usize = 3 * FRAME_SIZE;
const REALIGN_LOW: usize = FRAME_SIZE;
const REALIGN_HIGH: usize = TAIL_SAMPLES - 3 * FRAME_SIZE;

const FAR_POWER_SMOOTH: f32 = 0.35 / TAIL_PARTITIONS as f32;
const SPECTRUM_AVERAGE: f32 = FRAME_SIZE as f32 / SAMPLE_RATE as f32;
const LEAK_BETA: f32 = 2.0 * FRAME_SIZE as f32 / SAMPLE_RATE as f32;
const LEAK_BETA_MAX: f32 = 0.5 * FRAME_SIZE as f32 / SAMPLE_RATE as f32;
const MIN_LEAK: f32 = 0.005;
const FAR_ACTIVE_POWER: f32 = FRAME_SIZE as f32 * 1000.0;
const MAX_STEP: f32 = 0.25;
const ADAPTED_LEAK: f32 = 0.03;
const BACKTRACK: f32 = 4.0;
const WINDOW1_UPDATE: f32 = 0.5;
const WINDOW2_UPDATE: f32 = 0.25;
const POWER_REGULARIZATION: f32 = 10.0;

#[derive(Debug, Clone)]
pub struct FarEndFrame {
    pcm: Vec<i16>,
    output_latency: Option<Duration>,
}

#[derive(Clone)]
pub struct FarEndSink {
    tx: SyncSender<FarEndFrame>,
}

impl FarEndSink {
    pub fn push(&self, pcm: &[i16], output_latency: Option<Duration>) -> bool {
        let frame = FarEndFrame {
            pcm: pcm.to_vec(),
            output_latency,
        };
        match self.tx.try_send(frame) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => false,
        }
    }
}

pub struct FarEndInbox {
    tx: SyncSender<FarEndFrame>,
    rx: Receiver<FarEndFrame>,
}

impl Default for FarEndInbox {
    fn default() -> Self {
        Self::new()
    }
}

impl FarEndInbox {
    pub fn new() -> Self {
        let (tx, rx) = sync_channel(SINK_CAPACITY);
        Self { tx, rx }
    }

    pub fn sink(&self) -> FarEndSink {
        FarEndSink {
            tx: self.tx.clone(),
        }
    }

    pub fn drain_into(&self, canceller: &mut EchoCanceller) {
        while let Ok(frame) = self.rx.try_recv() {
            canceller.push_far_end(&frame.pcm, frame.output_latency);
        }
    }

    pub fn discard(&self) {
        while self.rx.try_recv().is_ok() {}
    }
}

pub fn samples_for(latency: Duration) -> usize {
    (latency.as_secs_f64() * SAMPLE_RATE as f64).round() as usize
}

struct Transform {
    forward: Arc<dyn RealToComplex<f32>>,
    inverse: Arc<dyn ComplexToReal<f32>>,
    scratch: Vec<Complex<f32>>,
}

impl Transform {
    fn new() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(BLOCK);
        let inverse = planner.plan_fft_inverse(BLOCK);
        let scratch =
            vec![Complex::default(); forward.get_scratch_len().max(inverse.get_scratch_len())];
        Self {
            forward,
            inverse,
            scratch,
        }
    }

    fn forward(&mut self, time: &mut [f32], spectrum: &mut [Complex<f32>]) {
        self.forward
            .process_with_scratch(time, spectrum, &mut self.scratch)
            .expect("block and spectrum sizes are fixed");
    }

    fn inverse(&mut self, spectrum: &mut [Complex<f32>], time: &mut [f32]) {
        spectrum[0].im = 0.0;
        spectrum[BINS - 1].im = 0.0;
        self.inverse
            .process_with_scratch(spectrum, time, &mut self.scratch)
            .expect("block and spectrum sizes are fixed");
        let scale = 1.0 / BLOCK as f32;
        for t in time.iter_mut() {
            *t *= scale;
        }
    }
}

const DECIMATION: usize = 8;
const DECIMATED_PER_FRAME: usize = FRAME_SIZE / DECIMATION;
const SEARCH_RING: usize = 1 << 13;
const SEARCH_RING_MASK: usize = SEARCH_RING - 1;
const SEARCH_WINDOW: usize = 1536;
const MAX_LAG: usize = MAX_DELAY_SAMPLES / DECIMATION;
const LAGS_PER_FRAME: usize = 72;
const _: () = assert!(SEARCH_RING >= SEARCH_WINDOW + MAX_LAG + DECIMATED_PER_FRAME);
const CONFIDENT_CORRELATION: f32 = 0.5;
const PLAUSIBLE_CORRELATION: f32 = 0.3;
const CONFIRM_TOLERANCE: usize = DECIMATED_PER_FRAME;
const SEARCH_SILENCE: f64 = SEARCH_WINDOW as f64 * 100.0;

struct DelaySearch {
    far: Vec<f32>,
    near: Vec<f32>,
    far_energy: Vec<f64>,
    near_energy: Vec<f64>,
    head: usize,
    far_last: f32,
    near_last: f32,
    sweep_lag: usize,
    best_lag: usize,
    best_score: f32,
    candidate: Option<usize>,
    confirmations: u32,
}

impl DelaySearch {
    fn new() -> Self {
        Self {
            far: vec![0.0; SEARCH_RING],
            near: vec![0.0; SEARCH_RING],
            far_energy: vec![0.0; SEARCH_RING],
            near_energy: vec![0.0; SEARCH_RING],
            head: 0,
            far_last: 0.0,
            near_last: 0.0,
            sweep_lag: 0,
            best_lag: 0,
            best_score: 0.0,
            candidate: None,
            confirmations: 0,
        }
    }

    fn push(&mut self, far: &[f32], near: &[f32]) {
        for i in 0..DECIMATED_PER_FRAME {
            let chunk = i * DECIMATION..(i + 1) * DECIMATION;
            let far_avg = far[chunk.clone()].iter().sum::<f32>() / DECIMATION as f32;
            let near_avg = near[chunk].iter().sum::<f32>() / DECIMATION as f32;
            let far_diff = far_avg - self.far_last;
            let near_diff = near_avg - self.near_last;
            self.far_last = far_avg;
            self.near_last = near_avg;
            let idx = self.head & SEARCH_RING_MASK;
            let prev = self.head.wrapping_sub(1) & SEARCH_RING_MASK;
            self.far[idx] = far_diff;
            self.near[idx] = near_diff;
            self.far_energy[idx] = self.far_energy[prev] + f64::from(far_diff * far_diff);
            self.near_energy[idx] = self.near_energy[prev] + f64::from(near_diff * near_diff);
            self.head = self.head.wrapping_add(1);
        }
    }

    fn energy(prefix: &[f64], end: usize, len: usize) -> f64 {
        let last = end.wrapping_sub(1) & SEARCH_RING_MASK;
        let first = end.wrapping_sub(len + 1) & SEARCH_RING_MASK;
        (prefix[last] - prefix[first]).max(0.0)
    }

    fn step(&mut self) -> Option<(usize, f32)> {
        if self.head < SEARCH_WINDOW + MAX_LAG + 1 {
            return None;
        }
        let near_energy = Self::energy(&self.near_energy, self.head, SEARCH_WINDOW);
        let end = self.sweep_lag + LAGS_PER_FRAME;
        if near_energy > SEARCH_SILENCE {
            for lag in self.sweep_lag..end.min(MAX_LAG) {
                let far_energy =
                    Self::energy(&self.far_energy, self.head.wrapping_sub(lag), SEARCH_WINDOW);
                if far_energy <= SEARCH_SILENCE {
                    continue;
                }
                let mut dot = 0.0f32;
                let near_start = self.head.wrapping_sub(SEARCH_WINDOW);
                let far_start = near_start.wrapping_sub(lag);
                for i in 0..SEARCH_WINDOW {
                    dot += self.far[far_start.wrapping_add(i) & SEARCH_RING_MASK]
                        * self.near[near_start.wrapping_add(i) & SEARCH_RING_MASK];
                }
                let score = dot / ((far_energy * near_energy).sqrt() as f32 + 1e-9);
                if score > self.best_score {
                    self.best_score = score;
                    self.best_lag = lag;
                }
            }
        }
        self.sweep_lag = end;
        if self.sweep_lag < MAX_LAG {
            return None;
        }
        let (lag, score) = (self.best_lag, self.best_score);
        self.sweep_lag = 0;
        self.best_score = 0.0;
        self.best_lag = 0;
        if score < PLAUSIBLE_CORRELATION {
            self.candidate = None;
            self.confirmations = 0;
            return None;
        }
        match self.candidate {
            Some(c) if c.abs_diff(lag) <= CONFIRM_TOLERANCE => self.confirmations += 1,
            _ => {
                self.candidate = Some(lag);
                self.confirmations = 1;
            }
        }
        if score >= CONFIDENT_CORRELATION || self.confirmations >= 2 {
            Some((lag * DECIMATION, score))
        } else {
            None
        }
    }
}

const SUPPRESS_OVER: f32 = 1.0;
const SUPPRESS_FLOOR: f32 = 0.1;
const SUPPRESS_FLOOR_DOUBLE_TALK: f32 = 0.3;
const DOUBLE_TALK_RATIO: f32 = 4.0;
const SUPPRESS_ATTACK: f32 = 0.6;
const SUPPRESS_RELEASE: f32 = 0.3;
const SUPPRESS_SMOOTH_BINS: usize = 2;

struct ResidualSuppressor {
    enabled: bool,
    window: Vec<f32>,
    prev_error: Vec<f32>,
    prev_echo: Vec<f32>,
    overlap: Vec<f32>,
    gain: Vec<f32>,
    smoothed: Vec<f32>,
    error_block: Vec<f32>,
    echo_block: Vec<f32>,
    error_spec: Vec<Complex<f32>>,
    echo_spec: Vec<Complex<f32>>,
}

impl ResidualSuppressor {
    fn new() -> Self {
        let window = (0..BLOCK)
            .map(|n| (std::f32::consts::PI * (n as f32 + 0.5) / BLOCK as f32).sin())
            .collect();
        Self {
            enabled: true,
            window,
            prev_error: vec![0.0; FRAME_SIZE],
            prev_echo: vec![0.0; FRAME_SIZE],
            overlap: vec![0.0; FRAME_SIZE],
            gain: vec![1.0; BINS],
            smoothed: vec![1.0; BINS],
            error_block: vec![0.0; BLOCK],
            echo_block: vec![0.0; BLOCK],
            error_spec: vec![Complex::default(); BINS],
            echo_spec: vec![Complex::default(); BINS],
        }
    }

    fn process(
        &mut self,
        transform: &mut Transform,
        error: &[f32],
        echo: &[f32],
        leak: f32,
        out: &mut [f32],
    ) {
        if !self.enabled {
            out.copy_from_slice(error);
            return;
        }
        for i in 0..FRAME_SIZE {
            self.error_block[i] = self.prev_error[i] * self.window[i];
            self.error_block[i + FRAME_SIZE] = error[i] * self.window[i + FRAME_SIZE];
            self.echo_block[i] = self.prev_echo[i] * self.window[i];
            self.echo_block[i + FRAME_SIZE] = echo[i] * self.window[i + FRAME_SIZE];
        }
        self.prev_error.copy_from_slice(error);
        self.prev_echo.copy_from_slice(echo);
        transform.forward(&mut self.error_block, &mut self.error_spec);
        transform.forward(&mut self.echo_block, &mut self.echo_spec);
        let error_total: f32 = self.error_spec.iter().map(|e| e.norm_sqr()).sum();
        let residual_total: f32 = self
            .echo_spec
            .iter()
            .map(|y| SUPPRESS_OVER * leak * y.norm_sqr())
            .sum();
        let floor = if error_total > DOUBLE_TALK_RATIO * residual_total {
            SUPPRESS_FLOOR_DOUBLE_TALK
        } else {
            SUPPRESS_FLOOR
        };
        for k in 0..BINS {
            let error_power = self.error_spec[k].norm_sqr();
            let residual = SUPPRESS_OVER * leak * self.echo_spec[k].norm_sqr();
            let wanted = ((error_power - residual) / (error_power + 1e-6)).clamp(floor, 1.0);
            let rate = if wanted < self.gain[k] {
                SUPPRESS_ATTACK
            } else {
                SUPPRESS_RELEASE
            };
            self.gain[k] += rate * (wanted - self.gain[k]);
        }
        for k in 0..BINS {
            let lo = k.saturating_sub(SUPPRESS_SMOOTH_BINS);
            let hi = (k + SUPPRESS_SMOOTH_BINS).min(BINS - 1);
            let span = &self.gain[lo..=hi];
            self.smoothed[k] = span.iter().sum::<f32>() / span.len() as f32;
        }
        for (bin, g) in self.error_spec.iter_mut().zip(&self.smoothed) {
            *bin *= *g;
        }
        transform.inverse(&mut self.error_spec, &mut self.error_block);
        for i in 0..FRAME_SIZE {
            out[i] = self.overlap[i] + self.error_block[i] * self.window[i];
            self.overlap[i] = self.error_block[i + FRAME_SIZE] * self.window[i + FRAME_SIZE];
        }
    }
}

pub struct EchoCanceller {
    transform: Transform,
    far_queue: VecDeque<Vec<f32>>,
    queue_primed: bool,
    far_ring: Vec<f32>,
    far_head: usize,
    delay: usize,
    locked: bool,
    reported_delay: Option<usize>,
    capture_latency: Option<Duration>,
    far_spectra: Vec<Complex<f32>>,
    spectra_slot: usize,
    far_power: Vec<f32>,
    background: Vec<Complex<f32>>,
    foreground: Vec<Complex<f32>>,
    prop: Vec<f32>,
    step: Vec<f32>,
    error_high: Vec<f32>,
    echo_high: Vec<f32>,
    pey: f32,
    pyy: f32,
    leak: f32,
    sum_adapt: f32,
    adapted: bool,
    davg1: f32,
    davg2: f32,
    dvar1: f32,
    dvar2: f32,
    constrain_slot: usize,
    search: DelaySearch,
    suppressor: ResidualSuppressor,
    block: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    error_spectrum: Vec<Complex<f32>>,
    echo_spectrum: Vec<Complex<f32>>,
    near: Vec<f32>,
    far_frame: Vec<f32>,
    echo_fg: Vec<f32>,
    echo_bg: Vec<f32>,
    error_fg: Vec<f32>,
    error_bg: Vec<f32>,
    cleaned: Vec<f32>,
    last_erle_db: f32,
}

impl Default for EchoCanceller {
    fn default() -> Self {
        Self::new()
    }
}

impl EchoCanceller {
    pub fn new() -> Self {
        let mut canceller = Self {
            transform: Transform::new(),
            far_queue: VecDeque::new(),
            queue_primed: false,
            far_ring: vec![0.0; FAR_RING],
            far_head: 0,
            delay: 0,
            locked: false,
            reported_delay: None,
            capture_latency: None,
            far_spectra: vec![Complex::default(); TAIL_PARTITIONS * BINS],
            spectra_slot: 0,
            far_power: vec![0.0; BINS],
            background: vec![Complex::default(); TAIL_PARTITIONS * BINS],
            foreground: vec![Complex::default(); TAIL_PARTITIONS * BINS],
            prop: vec![0.0; TAIL_PARTITIONS],
            step: vec![0.0; BINS],
            error_high: vec![0.0; BINS],
            echo_high: vec![0.0; BINS],
            pey: 1.0,
            pyy: 1.0,
            leak: 1.0,
            sum_adapt: 0.0,
            adapted: false,
            davg1: 0.0,
            davg2: 0.0,
            dvar1: 0.0,
            dvar2: 0.0,
            constrain_slot: 0,
            search: DelaySearch::new(),
            suppressor: ResidualSuppressor::new(),
            block: vec![0.0; BLOCK],
            spectrum: vec![Complex::default(); BINS],
            error_spectrum: vec![Complex::default(); BINS],
            echo_spectrum: vec![Complex::default(); BINS],
            near: vec![0.0; FRAME_SIZE],
            far_frame: vec![0.0; FRAME_SIZE],
            echo_fg: vec![0.0; FRAME_SIZE],
            echo_bg: vec![0.0; FRAME_SIZE],
            error_fg: vec![0.0; FRAME_SIZE],
            error_bg: vec![0.0; FRAME_SIZE],
            cleaned: vec![0.0; FRAME_SIZE],
            last_erle_db: 0.0,
        };
        canceller.reset_filters();
        canceller
    }

    pub fn set_residual_suppression(&mut self, enabled: bool) {
        self.suppressor.enabled = enabled;
    }

    pub fn delay_samples(&self) -> usize {
        self.delay
    }

    pub fn is_adapted(&self) -> bool {
        self.adapted
    }

    pub fn leak(&self) -> f32 {
        self.leak
    }

    pub fn last_erle_db(&self) -> f32 {
        self.last_erle_db
    }

    pub fn note_capture_latency(&mut self, latency: Option<Duration>) {
        self.capture_latency = latency;
    }

    pub fn push_far_end(&mut self, pcm: &[i16], output_latency: Option<Duration>) {
        let mut frame = vec![0.0f32; FRAME_SIZE];
        for (dst, src) in frame.iter_mut().zip(pcm) {
            *dst = f32::from(*src);
        }
        self.far_queue.push_back(frame);
        if let (Some(out), None) = (output_latency, self.reported_delay) {
            let total = out + self.capture_latency.unwrap_or_default();
            let reported = samples_for(total).min(MAX_DELAY_SAMPLES);
            self.reported_delay = Some(reported);
            if !self.locked {
                self.realign(reported.saturating_sub(PEAK_MARGIN));
            }
        }
    }

    pub fn queued_far_frames(&self) -> usize {
        self.far_queue.len()
    }

    fn next_far_frame(&mut self) {
        if !self.queue_primed {
            if self.far_queue.len() < QUEUE_PRIME_FRAMES {
                self.far_frame.iter_mut().for_each(|s| *s = 0.0);
                return;
            }
            self.queue_primed = true;
        }
        while self.far_queue.len() > QUEUE_MAX_FRAMES {
            self.far_queue.pop_front();
        }
        match self.far_queue.pop_front() {
            Some(frame) => self.far_frame.copy_from_slice(&frame),
            None => {
                self.queue_primed = false;
                self.far_frame.iter_mut().for_each(|s| *s = 0.0);
            }
        }
    }

    fn write_far_frame(&mut self) {
        for (i, s) in self.far_frame.iter().enumerate() {
            self.far_ring[(self.far_head + i) & FAR_RING_MASK] = *s;
        }
        self.far_head = self.far_head.wrapping_add(FRAME_SIZE);
    }

    fn far_block(&self, end: usize, out: &mut [f32]) {
        let start = end.wrapping_sub(BLOCK);
        for (i, o) in out.iter_mut().enumerate() {
            *o = self.far_ring[start.wrapping_add(i) & FAR_RING_MASK];
        }
    }

    fn slot_for(&self, partition: usize) -> usize {
        (self.spectra_slot + TAIL_PARTITIONS - partition) % TAIL_PARTITIONS
    }

    fn reset_filters(&mut self) {
        self.background
            .iter_mut()
            .for_each(|w| *w = Complex::default());
        self.foreground
            .iter_mut()
            .for_each(|w| *w = Complex::default());
        self.far_power.iter_mut().for_each(|p| *p = 0.0);
        self.error_high.iter_mut().for_each(|p| *p = 0.0);
        self.echo_high.iter_mut().for_each(|p| *p = 0.0);
        self.pey = 1.0;
        self.pyy = 1.0;
        self.leak = 1.0;
        self.sum_adapt = 0.0;
        self.adapted = false;
        self.reset_filter_stats();
        let total: f32 = (1..=TAIL_PARTITIONS).map(|n| n as f32).sum();
        for (p, prop) in self.prop.iter_mut().enumerate() {
            *prop = (TAIL_PARTITIONS - p) as f32 / total;
        }
    }

    fn reset_filter_stats(&mut self) {
        self.davg1 = 0.0;
        self.davg2 = 0.0;
        self.dvar1 = 0.0;
        self.dvar2 = 0.0;
    }

    fn realign(&mut self, delay: usize) {
        self.delay = delay.min(MAX_DELAY_SAMPLES);
        self.reset_filters();
        for p in 0..TAIL_PARTITIONS {
            let end = self.far_head.wrapping_sub(self.delay + p * FRAME_SIZE);
            let mut block = std::mem::take(&mut self.block);
            self.far_block(end, &mut block);
            let slot = self.slot_for(p);
            self.transform.forward(
                &mut block,
                &mut self.far_spectra[slot * BINS..(slot + 1) * BINS],
            );
            self.block = block;
        }
    }

    fn consider_peak(&mut self, peak: usize) {
        let offset = peak.wrapping_sub(self.delay) as isize;
        let in_window = self.locked
            && peak >= self.delay
            && (REALIGN_LOW as isize..=REALIGN_HIGH as isize).contains(&offset);
        self.locked = true;
        if !in_window {
            self.realign(peak.saturating_sub(PEAK_MARGIN));
        }
    }

    fn store_far_spectrum(&mut self) -> f32 {
        self.spectra_slot = (self.spectra_slot + 1) % TAIL_PARTITIONS;
        let end = self.far_head.wrapping_sub(self.delay);
        let mut block = std::mem::take(&mut self.block);
        self.far_block(end, &mut block);
        let sxx: f32 = block[FRAME_SIZE..].iter().map(|x| x * x).sum();
        let slot = self.spectra_slot;
        self.transform.forward(
            &mut block,
            &mut self.far_spectra[slot * BINS..(slot + 1) * BINS],
        );
        self.block = block;
        for k in 0..BINS {
            let xf = self.far_spectra[slot * BINS + k].norm_sqr();
            self.far_power[k] =
                (1.0 - FAR_POWER_SMOOTH) * self.far_power[k] + 1.0 + FAR_POWER_SMOOTH * xf;
        }
        sxx
    }

    fn filter_output(&mut self, weights: &[Complex<f32>], out: &mut [f32]) {
        self.spectrum
            .iter_mut()
            .for_each(|y| *y = Complex::default());
        for p in 0..TAIL_PARTITIONS {
            let slot = self.slot_for(p);
            let x = &self.far_spectra[slot * BINS..(slot + 1) * BINS];
            let w = &weights[p * BINS..(p + 1) * BINS];
            for k in 0..BINS {
                self.spectrum[k] += w[k] * x[k];
            }
        }
        let mut block = std::mem::take(&mut self.block);
        self.transform.inverse(&mut self.spectrum, &mut block);
        out.copy_from_slice(&block[FRAME_SIZE..]);
        self.block = block;
    }

    fn judge_filters(&mut self, sff: f32, see: f32, dbf: f32) {
        let diff = sff - see;
        self.davg1 = 0.6 * self.davg1 + 0.4 * diff;
        self.davg2 = 0.85 * self.davg2 + 0.15 * diff;
        self.dvar1 = 0.36 * self.dvar1 + 0.16 * sff * dbf;
        self.dvar2 = 0.7225 * self.dvar2 + 0.0225 * sff * dbf;
        let better = diff * diff.abs() > sff * dbf
            || self.davg1 * self.davg1.abs() > WINDOW1_UPDATE * sff * self.dvar1
            || self.davg2 * self.davg2.abs() > WINDOW2_UPDATE * sff * self.dvar2;
        if better {
            self.reset_filter_stats();
            self.foreground.copy_from_slice(&self.background);
            let n = FRAME_SIZE as f32;
            for i in 0..FRAME_SIZE {
                let w = (i as f32 + 1.0) / n;
                self.cleaned[i] = w * self.error_bg[i] + (1.0 - w) * self.error_fg[i];
            }
            self.echo_fg.copy_from_slice(&self.echo_bg);
            return;
        }
        let worse = -diff * diff.abs() > BACKTRACK * sff * dbf
            || -self.davg1 * self.davg1.abs() > BACKTRACK * sff * self.dvar1
            || -self.davg2 * self.davg2.abs() > BACKTRACK * sff * self.dvar2;
        if worse {
            self.background.copy_from_slice(&self.foreground);
            self.echo_bg.copy_from_slice(&self.echo_fg);
            self.error_bg.copy_from_slice(&self.error_fg);
            self.reset_filter_stats();
        }
        self.cleaned.copy_from_slice(&self.error_fg);
    }

    fn update_leak_and_step(&mut self, sxx: f32, see: f32, sey: f32, syy: f32) {
        let mut pey = 1.0f32;
        let mut pyy = 1.0f32;
        for k in 0..BINS {
            let rf = self.error_spectrum[k].norm_sqr();
            let yf = self.echo_spectrum[k].norm_sqr();
            let eh = rf - self.error_high[k];
            let yh = yf - self.echo_high[k];
            pey += eh * yh;
            pyy += yh * yh;
            self.error_high[k] =
                (1.0 - SPECTRUM_AVERAGE) * self.error_high[k] + SPECTRUM_AVERAGE * rf;
            self.echo_high[k] =
                (1.0 - SPECTRUM_AVERAGE) * self.echo_high[k] + SPECTRUM_AVERAGE * yf;
        }
        let pyy = pyy.sqrt();
        let pey = pey / pyy;
        let alpha = (LEAK_BETA * syy).min(LEAK_BETA_MAX * see) / see.max(1.0);
        self.pey = (1.0 - alpha) * self.pey + alpha * pey;
        self.pyy = ((1.0 - alpha) * self.pyy + alpha * pyy).max(1.0);
        self.pey = self.pey.clamp(MIN_LEAK * self.pyy, self.pyy);
        self.leak = (2.0 * self.pey / self.pyy).min(1.0);

        let see = see.max(1.0);
        let mut rer = (0.0001 * sxx + 3.0 * self.leak * syy) / see;
        let coherent = sey * sey / (1.0 + see * syy);
        rer = rer.max(coherent).min(0.5);

        if !self.adapted
            && self.sum_adapt > TAIL_PARTITIONS as f32
            && self.leak * syy > ADAPTED_LEAK * syy
        {
            self.adapted = true;
        }
        if self.adapted {
            for k in 0..BINS {
                let rf = self.error_spectrum[k].norm_sqr() + 1.0;
                let yf = self.echo_spectrum[k].norm_sqr();
                let r = (self.leak * yf).min(0.5 * rf);
                let r = 0.7 * r + 0.3 * rer * rf;
                self.step[k] = r / (rf * (self.far_power[k] + POWER_REGULARIZATION));
            }
            self.adjust_prop();
        } else {
            let rate = if sxx > FAR_ACTIVE_POWER {
                (MAX_STEP * sxx).min(MAX_STEP * see) / see
            } else {
                0.0
            };
            for k in 0..BINS {
                self.step[k] = rate / (self.far_power[k] + POWER_REGULARIZATION);
            }
            self.sum_adapt += rate;
        }
    }

    fn adjust_prop(&mut self) {
        let mut max = 0.0f32;
        for p in 0..TAIL_PARTITIONS {
            let norm: f32 = self.background[p * BINS..(p + 1) * BINS]
                .iter()
                .map(|w| w.norm_sqr())
                .sum();
            self.prop[p] = norm.sqrt();
            max = max.max(self.prop[p]);
        }
        let mut sum = 1.0f32;
        for prop in self.prop.iter_mut() {
            *prop += 0.1 * max;
            sum += *prop;
        }
        for prop in self.prop.iter_mut() {
            *prop = 0.99 * *prop / sum;
        }
    }

    fn adapt_background(&mut self) {
        for p in 0..TAIL_PARTITIONS {
            let slot = self.slot_for(p);
            let prop = self.prop[p];
            let x = &self.far_spectra[slot * BINS..(slot + 1) * BINS];
            let w = &mut self.background[p * BINS..(p + 1) * BINS];
            for k in 0..BINS {
                w[k] += self.step[k] * prop * x[k].conj() * self.error_spectrum[k];
            }
        }
        let p = self.constrain_slot;
        self.constrain_slot = (self.constrain_slot + 1) % TAIL_PARTITIONS;
        let mut block = std::mem::take(&mut self.block);
        let w = &mut self.background[p * BINS..(p + 1) * BINS];
        self.transform.inverse(w, &mut block);
        block[FRAME_SIZE..].iter_mut().for_each(|t| *t = 0.0);
        self.transform.forward(&mut block, w);
        self.block = block;
    }

    pub fn process(&mut self, near: &[f32], out: &mut [f32]) {
        for (dst, src) in self.near.iter_mut().zip(near) {
            *dst = src * I16_SCALE;
        }
        self.next_far_frame();
        self.write_far_frame();
        let mut far_frame = std::mem::take(&mut self.far_frame);
        let mut near_frame = std::mem::take(&mut self.near);
        self.search.push(&far_frame, &near_frame);
        if let Some((peak, _)) = self.search.step() {
            self.consider_peak(peak);
        }
        let sxx = self.store_far_spectrum();

        let foreground = std::mem::take(&mut self.foreground);
        let mut echo_fg = std::mem::take(&mut self.echo_fg);
        self.filter_output(&foreground, &mut echo_fg);
        self.foreground = foreground;
        let background = std::mem::take(&mut self.background);
        let mut echo_bg = std::mem::take(&mut self.echo_bg);
        self.filter_output(&background, &mut echo_bg);
        self.background = background;

        let (mut sff, mut see, mut dbf, mut sdd) = (0.0f32, 0.0f32, POWER_REGULARIZATION, 0.0f32);
        for i in 0..FRAME_SIZE {
            let d = near_frame[i];
            self.error_fg[i] = d - echo_fg[i];
            self.error_bg[i] = d - echo_bg[i];
            sff += self.error_fg[i] * self.error_fg[i];
            see += self.error_bg[i] * self.error_bg[i];
            let diff = echo_fg[i] - echo_bg[i];
            dbf += diff * diff;
            sdd += d * d;
        }
        self.echo_fg = echo_fg;
        self.echo_bg = echo_bg;
        self.judge_filters(sff, see, dbf);
        let see: f32 = self.error_bg.iter().map(|e| e * e).sum();
        let sey: f32 = self
            .error_bg
            .iter()
            .zip(&self.echo_bg)
            .map(|(e, y)| e * y)
            .sum();
        let syy: f32 = self.echo_bg.iter().map(|y| y * y).sum();

        let mut block = std::mem::take(&mut self.block);
        block[..FRAME_SIZE].iter_mut().for_each(|t| *t = 0.0);
        block[FRAME_SIZE..].copy_from_slice(&self.error_bg);
        self.transform.forward(&mut block, &mut self.error_spectrum);
        block[..FRAME_SIZE].iter_mut().for_each(|t| *t = 0.0);
        block[FRAME_SIZE..].copy_from_slice(&self.echo_bg);
        self.transform.forward(&mut block, &mut self.echo_spectrum);
        self.block = block;

        self.update_leak_and_step(sxx, see, sey, syy);
        self.adapt_background();

        let sff: f32 = self.cleaned.iter().map(|e| e * e).sum();
        self.last_erle_db = 10.0 * ((sdd + 1.0) / (sff + 1.0)).log10();

        let cleaned = std::mem::take(&mut self.cleaned);
        let echo_fg = std::mem::take(&mut self.echo_fg);
        let leak = self.leak;
        self.suppressor.process(
            &mut self.transform,
            &cleaned,
            &echo_fg,
            leak,
            &mut near_frame,
        );
        self.cleaned = cleaned;
        self.echo_fg = echo_fg;
        for (dst, src) in out.iter_mut().zip(&near_frame) {
            *dst = (src / I16_SCALE).clamp(-1.0, 1.0);
        }
        far_frame.iter_mut().for_each(|s| *s = 0.0);
        self.far_frame = far_frame;
        self.near = near_frame;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
        }
    }

    struct EchoPath {
        taps: Vec<(usize, f32)>,
        history: Vec<f32>,
        written: usize,
    }

    impl EchoPath {
        fn new(delay: usize, seed: u64) -> Self {
            let mut rng = Lcg(seed);
            let mut taps = vec![
                (delay, 0.6),
                (delay + 37, -0.3),
                (delay + 95, 0.2),
                (delay + 150, -0.1),
            ];
            for i in 0..40 {
                let at = delay + 200 + i * 45;
                let decay = (-(i as f32) / 12.0).exp();
                taps.push((at, 0.08 * decay * rng.next()));
            }
            Self {
                taps,
                history: vec![0.0; 1 << 16],
                written: 0,
            }
        }

        fn echo_of(&mut self, far: &[f32]) -> Vec<f32> {
            let mask = self.history.len() - 1;
            far.iter()
                .map(|x| {
                    self.history[self.written & mask] = *x;
                    let out = self
                        .taps
                        .iter()
                        .map(|(lag, gain)| {
                            gain * self.history[self.written.wrapping_sub(*lag) & mask]
                        })
                        .sum();
                    self.written += 1;
                    out
                })
                .collect()
        }
    }

    fn to_i16(frame: &[f32]) -> Vec<i16> {
        frame.iter().map(|s| (s * 32767.0) as i16).collect()
    }

    fn voice_sample(t: f32) -> f32 {
        let pitch = 180.0 + 25.0 * (2.0 * std::f32::consts::PI * 2.3 * t).sin();
        let phase = 2.0 * std::f32::consts::PI * pitch * t;
        let envelope = 0.5 + 0.5 * (2.0 * std::f32::consts::PI * 3.1 * t).sin();
        let harmonics: f32 = (1..=8).map(|h| (h as f32 * phase).sin() / h as f32).sum();
        0.18 * envelope * harmonics
    }

    fn power(samples: &[f32]) -> f64 {
        samples.iter().map(|s| f64::from(s * s)).sum::<f64>()
    }

    fn db(ratio: f64) -> f64 {
        10.0 * ratio.max(1e-12).log10()
    }

    struct Run {
        far: Lcg,
        floor: Lcg,
        path: EchoPath,
        aec: EchoCanceller,
        frames: usize,
    }

    impl Run {
        fn new(delay: usize, suppression: bool) -> Self {
            let mut aec = EchoCanceller::new();
            aec.set_residual_suppression(suppression);
            Self {
                far: Lcg(7),
                floor: Lcg(99),
                path: EchoPath::new(delay, 3),
                aec,
                frames: 0,
            }
        }

        fn step(&mut self, near_speech: Option<&[f32]>) -> (Vec<f32>, Vec<f32>) {
            let far: Vec<f32> = (0..FRAME_SIZE).map(|_| 0.3 * self.far.next()).collect();
            let echo = self.path.echo_of(&far);
            let near: Vec<f32> = (0..FRAME_SIZE)
                .map(|i| echo[i] + 0.0005 * self.floor.next() + near_speech.map_or(0.0, |s| s[i]))
                .collect();
            self.aec.push_far_end(&to_i16(&far), None);
            let mut out = vec![0.0; FRAME_SIZE];
            self.aec.process(&near, &mut out);
            self.frames += 1;
            (echo, out)
        }

        fn run(&mut self, frames: usize) -> f64 {
            let mut echo_power = 0.0;
            let mut out_power = 0.0;
            for _ in 0..frames {
                let (echo, out) = self.step(None);
                echo_power += power(&echo);
                out_power += power(&out);
            }
            db(echo_power / out_power)
        }
    }

    #[test]
    fn linear_filter_removes_a_delayed_reverberant_echo() {
        let mut run = Run::new(4000, false);
        run.run(500);
        let erle = run.run(200);
        assert!(erle > 20.0, "linear ERLE {erle:.1} dB");
        assert!(run.aec.is_adapted());
    }

    #[test]
    fn suppressor_raises_erle_further() {
        let mut run = Run::new(4000, true);
        run.run(500);
        let erle = run.run(200);
        assert!(erle > 30.0, "ERLE with suppression {erle:.1} dB");
    }

    #[test]
    fn delay_search_finds_a_long_playback_delay() {
        let mut run = Run::new(14_400, false);
        run.run(600);
        let erle = run.run(200);
        assert!(
            erle > 20.0,
            "ERLE {erle:.1} dB at delay {}",
            run.aec.delay_samples()
        );
        assert!(
            run.aec.delay_samples() > 8_000,
            "delay {}",
            run.aec.delay_samples()
        );
    }

    #[test]
    fn near_end_speech_survives_double_talk_and_the_filter_does_not_diverge() {
        let mut run = Run::new(4000, true);
        run.run(500);
        let mut speech_power = 0.0;
        let mut distortion = 0.0;
        let mut dot = 0.0;
        let mut out_power = 0.0;
        let mut previous_speech = vec![0.0f32; FRAME_SIZE];
        for frame in 0..300 {
            let speech: Vec<f32> = (0..FRAME_SIZE)
                .map(|i| voice_sample((frame * FRAME_SIZE + i) as f32 / SAMPLE_RATE as f32))
                .collect();
            let (_, out) = run.step(Some(&speech));
            if frame > 20 {
                for i in 0..FRAME_SIZE {
                    let s = previous_speech[i];
                    speech_power += f64::from(s * s);
                    distortion += f64::from((out[i] - s) * (out[i] - s));
                    dot += f64::from(out[i] * s);
                    out_power += f64::from(out[i] * out[i]);
                }
            }
            previous_speech = speech;
        }
        let kept = db(speech_power / distortion);
        let correlation = dot / (speech_power * out_power).sqrt();
        assert!(kept > 6.0, "near-end speech to distortion {kept:.1} dB");
        assert!(
            correlation > 0.85,
            "correlation with near-end speech {correlation:.3}"
        );
        let erle = run.run(200);
        assert!(erle > 15.0, "ERLE after double-talk {erle:.1} dB");
    }

    #[test]
    fn without_far_end_the_canceller_only_delays_by_one_frame() {
        let mut aec = EchoCanceller::new();
        let frame_a: Vec<f32> = (0..FRAME_SIZE)
            .map(|i| 0.4 * ((i as f32) * 0.05).sin())
            .collect();
        let frame_b: Vec<f32> = (0..FRAME_SIZE)
            .map(|i| 0.2 * ((i as f32) * 0.11).cos())
            .collect();
        let mut out = vec![0.0; FRAME_SIZE];
        aec.process(&frame_a, &mut out);
        assert!(out.iter().all(|s| s.abs() < 1e-4));
        aec.process(&frame_b, &mut out);
        for (a, b) in frame_a.iter().zip(&out) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
    }

    #[test]
    fn the_sink_drops_frames_instead_of_growing_without_bound() {
        let inbox = FarEndInbox::new();
        let sink = inbox.sink();
        let frame = vec![0i16; FRAME_SIZE];
        for _ in 0..SINK_CAPACITY {
            assert!(sink.push(&frame, None));
        }
        assert!(!sink.push(&frame, None));
        inbox.discard();
        assert!(sink.push(&frame, None));
    }

    #[test]
    fn reported_latency_seeds_the_alignment_until_the_search_locks() {
        let mut aec = EchoCanceller::new();
        aec.note_capture_latency(Some(Duration::from_millis(10)));
        aec.push_far_end(&[0; FRAME_SIZE], Some(Duration::from_millis(100)));
        assert_eq!(
            aec.delay_samples(),
            samples_for(Duration::from_millis(110)) - PEAK_MARGIN
        );
        aec.push_far_end(&[0; FRAME_SIZE], Some(Duration::from_millis(400)));
        assert_eq!(
            aec.delay_samples(),
            samples_for(Duration::from_millis(110)) - PEAK_MARGIN
        );
    }

    #[test]
    fn a_stalled_far_end_is_treated_as_silence() {
        let mut aec = EchoCanceller::new();
        for _ in 0..QUEUE_PRIME_FRAMES {
            aec.push_far_end(&[1000; FRAME_SIZE], None);
        }
        let near = vec![0.0; FRAME_SIZE];
        let mut out = vec![0.0; FRAME_SIZE];
        for _ in 0..QUEUE_PRIME_FRAMES + 1 {
            aec.process(&near, &mut out);
        }
        assert_eq!(aec.queued_far_frames(), 0);
        assert!(!aec.queue_primed);
        for _ in 0..QUEUE_MAX_FRAMES * 2 {
            aec.push_far_end(&[0; FRAME_SIZE], None);
        }
        aec.process(&near, &mut out);
        assert!(aec.queued_far_frames() <= QUEUE_MAX_FRAMES);
    }
}
