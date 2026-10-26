use crate::jitter::JitterBuffer;
use commy_audio::codec::soft_limit;
use commy_audio::FRAME_SIZE;
use std::collections::{HashMap, HashSet};

pub const PRIORITY_DUCK_GAIN: f32 = 0.3;
pub const PRIORITY_TAIL_FRAMES: u32 = 60;
const DUCK_STEP: f32 = 0.05;
pub const MAX_VOLUME: f32 = 2.0;
const DB_PER_DOUBLING_ABOVE_FULL: f32 = 10.0;
const FULL_SCALE: f32 = i16::MAX as f32;
const LIMIT_CEILING: f32 = 0.89 * FULL_SCALE;
const LIMIT_RELEASE_PER_FRAME: f32 = 1.012;

pub fn volume_gain(volume: f32) -> f32 {
    let volume = if volume.is_finite() { volume.clamp(0.0, MAX_VOLUME) } else { 1.0 };
    if volume <= 1.0 {
        volume
    } else {
        10f32.powf(DB_PER_DOUBLING_ABOVE_FULL * volume.log2() / 20.0)
    }
}

#[derive(Debug)]
pub struct Mixer {
    peers: HashMap<u64, JitterBuffer>,
    peer_gains: HashMap<u64, f32>,
    master: f32,
    priority: HashSet<u64>,
    priority_talking: HashSet<u64>,
    duck: f32,
    duck_tail: u32,
    limiter: f32,
}

impl Default for Mixer {
    fn default() -> Self {
        Self {
            peers: HashMap::new(),
            peer_gains: HashMap::new(),
            master: 1.0,
            priority: HashSet::new(),
            priority_talking: HashSet::new(),
            duck: 1.0,
            duck_tail: 0,
            limiter: 1.0,
        }
    }
}

pub fn duck_target(priority_talking: bool, tail_left: u32) -> f32 {
    if priority_talking || tail_left > 0 {
        PRIORITY_DUCK_GAIN
    } else {
        1.0
    }
}

pub fn duck_step(current: f32, target: f32) -> f32 {
    if (current - target).abs() <= DUCK_STEP {
        target
    } else if current > target {
        current - DUCK_STEP
    } else {
        current + DUCK_STEP
    }
}

impl Mixer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, peer: u64, seq: u32, pcm: Vec<i16>) {
        self.peers.entry(peer).or_default().push(seq, pcm);
    }

    pub fn remove_peer(&mut self, peer: u64) {
        self.peers.remove(&peer);
        self.priority_talking.remove(&peer);
    }

    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    pub fn set_master(&mut self, volume: f32) {
        self.master = volume_gain(volume);
    }

    pub fn master(&self) -> f32 {
        self.master
    }

    pub fn set_peer_gain(&mut self, peer: u64, volume: f32) {
        self.peer_gains.insert(peer, volume_gain(volume));
    }

    pub fn set_priority(&mut self, peer: u64, priority: bool) {
        if priority {
            self.priority.insert(peer);
        } else {
            self.priority.remove(&peer);
            self.priority_talking.remove(&peer);
        }
    }

    pub fn is_priority(&self, peer: u64) -> bool {
        self.priority.contains(&peer)
    }

    pub fn set_speaking(&mut self, peer: u64, speaking: bool) {
        if speaking && self.priority.contains(&peer) {
            self.priority_talking.insert(peer);
        } else {
            self.priority_talking.remove(&peer);
        }
    }

    pub fn duck_level(&self) -> f32 {
        self.duck
    }

    fn advance_duck(&mut self) -> f32 {
        let talking = !self.priority_talking.is_empty();
        if talking {
            self.duck_tail = PRIORITY_TAIL_FRAMES;
        } else if self.duck_tail > 0 {
            self.duck_tail -= 1;
        }
        self.duck = duck_step(self.duck, duck_target(talking, self.duck_tail));
        self.duck
    }

    pub fn next_frame(&mut self) -> Vec<i16> {
        let duck = self.advance_duck();
        let mut acc = [0f32; FRAME_SIZE];
        for (peer, jb) in self.peers.iter_mut() {
            let ducked = if self.priority.contains(peer) { 1.0 } else { duck };
            let gain = *self.peer_gains.get(peer).unwrap_or(&1.0) * ducked;
            let frame = jb.pop();
            for (a, s) in acc.iter_mut().zip(frame) {
                *a += s as f32 * gain;
            }
        }
        let master = self.master;
        let peak = acc.iter().fold(0.0f32, |m, s| m.max((s * master).abs()));
        let (start, end) = self.advance_limiter(peak);
        let n = FRAME_SIZE as f32;
        acc.iter()
            .enumerate()
            .map(|(i, s)| {
                let limit = start + (end - start) * (i as f32 + 1.0) / n;
                (soft_limit(s * master * limit / FULL_SCALE) * FULL_SCALE).round() as i16
            })
            .collect()
    }

    fn advance_limiter(&mut self, peak: f32) -> (f32, f32) {
        let wanted = if peak > LIMIT_CEILING { LIMIT_CEILING / peak } else { 1.0 };
        let start = self.limiter;
        self.limiter = if wanted < start { wanted } else { (start * LIMIT_RELEASE_PER_FRAME).min(wanted) };
        (start, self.limiter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(value: i16) -> Vec<i16> {
        vec![value; FRAME_SIZE]
    }

    #[test]
    fn empty_mixer_outputs_silence() {
        let mut mixer = Mixer::new();
        assert_eq!(mixer.next_frame(), frame(0));
    }

    #[test]
    fn two_peers_are_summed() {
        let mut mixer = Mixer::new();
        mixer.push(1, 0, frame(100));
        mixer.push(2, 0, frame(-30));
        assert_eq!(mixer.next_frame(), frame(70));
    }

    #[test]
    fn overflow_is_held_under_full_scale() {
        let mut mixer = Mixer::new();
        mixer.push(1, 0, frame(i16::MAX));
        mixer.push(2, 0, frame(i16::MAX));
        let out = mixer.next_frame();
        assert!(out.iter().all(|s| *s > 25_000), "{}", out[0]);
        mixer.push(1, 1, frame(i16::MIN));
        mixer.push(2, 1, frame(i16::MIN));
        let out = mixer.next_frame();
        assert!(out.iter().all(|s| *s < -25_000), "{}", out[0]);
    }

    #[test]
    fn master_volume_scales_output() {
        let mut mixer = Mixer::new();
        mixer.set_master(0.5);
        mixer.push(1, 0, frame(1000));
        assert_eq!(mixer.next_frame(), frame(500));
        mixer.set_master(0.0);
        mixer.push(1, 1, frame(1000));
        assert_eq!(mixer.next_frame(), frame(0));
        mixer.set_master(10.0);
        assert_eq!(mixer.master(), volume_gain(MAX_VOLUME));
    }

    #[test]
    fn double_volume_is_ten_decibels_louder_and_full_volume_is_unchanged() {
        assert_eq!(volume_gain(1.0), 1.0);
        assert_eq!(volume_gain(0.5), 0.5);
        assert_eq!(volume_gain(0.0), 0.0);
        let boost_db = 20.0 * volume_gain(2.0).log10();
        assert!((boost_db - 10.0).abs() < 0.01, "{boost_db}");
        let halfway_db = 20.0 * volume_gain(1.5).log10();
        assert!(halfway_db > 5.0 && halfway_db < boost_db);
        assert_eq!(volume_gain(f32::NAN), 1.0);
        assert_eq!(volume_gain(9.0), volume_gain(MAX_VOLUME));
    }

    fn sine(seq: usize, amplitude: f32) -> Vec<i16> {
        (0..FRAME_SIZE)
            .map(|i| {
                let t = (seq * FRAME_SIZE + i) as f32 / commy_audio::SAMPLE_RATE as f32;
                (amplitude * (std::f32::consts::TAU * 200.0 * t).sin()) as i16
            })
            .collect()
    }

    #[test]
    fn a_boosted_loud_voice_is_turned_down_whole_instead_of_clipped() {
        let mut mixer = Mixer::new();
        mixer.set_peer_gain(1, 2.0);
        let mut last = Vec::new();
        let mut input = Vec::new();
        for seq in 0..20 {
            input = sine(seq, 20_000.0);
            mixer.push(1, seq as u32, input.clone());
            last = mixer.next_frame();
        }
        let peak = last.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(peak <= LIMIT_CEILING as u16 + 1 && peak > 25_000, "{peak}");
        let scale = f32::from(peak) / 20_000.0;
        let worst = input
            .iter()
            .zip(&last)
            .map(|(i, o)| (f32::from(*i) * scale - f32::from(*o)).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 600.0, "the waveform kept its shape, off by at most {worst}");
    }

    #[test]
    fn a_boosted_quiet_voice_is_really_louder() {
        let mut mixer = Mixer::new();
        mixer.set_peer_gain(1, 2.0);
        mixer.push(1, 0, sine(0, 2_000.0));
        let peak = mixer.next_frame().iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(peak > 6_000, "{peak}");
    }

    #[test]
    fn the_limiter_lets_go_after_a_loud_moment() {
        let mut mixer = Mixer::new();
        mixer.set_peer_gain(1, 2.0);
        for seq in 0..5 {
            mixer.push(1, seq, frame(30_000));
            mixer.next_frame();
        }
        for seq in 5..300 {
            mixer.push(1, seq, sine(seq as usize, 2_000.0));
            mixer.next_frame();
        }
        mixer.push(1, 300, sine(300, 2_000.0));
        let peak = mixer.next_frame().iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(peak > 6_000, "{peak}");
    }

    #[test]
    fn per_peer_gain_applies_before_mix() {
        let mut mixer = Mixer::new();
        mixer.set_peer_gain(1, 2.0);
        mixer.set_peer_gain(2, 0.0);
        mixer.push(1, 0, frame(100));
        mixer.push(2, 0, frame(5000));
        assert_eq!(mixer.next_frame(), frame((100.0 * volume_gain(2.0)).round() as i16));
    }

    #[test]
    fn removed_peer_goes_quiet() {
        let mut mixer = Mixer::new();
        mixer.push(1, 0, frame(50));
        mixer.push(1, 1, frame(50));
        assert_eq!(mixer.next_frame(), frame(50));
        mixer.remove_peer(1);
        assert_eq!(mixer.next_frame(), frame(0));
        assert_eq!(mixer.peer_count(), 0);
    }

    #[test]
    fn peer_volume_survives_reconnect() {
        let mut mixer = Mixer::new();
        mixer.set_peer_gain(7, 0.0);
        mixer.push(7, 0, frame(1000));
        mixer.remove_peer(7);
        mixer.push(7, 0, frame(1000));
        let loudest = (0..5).flat_map(|_| mixer.next_frame()).map(|s| s.abs()).max().unwrap();
        assert_eq!(loudest, 0, "a muted peer stays muted after reconnecting");
    }

    fn settle(mixer: &mut Mixer, frames: usize) {
        for _ in 0..frames {
            mixer.next_frame();
        }
    }

    #[test]
    fn a_talking_priority_speaker_ducks_everyone_else_but_not_themselves() {
        let mut mixer = Mixer::new();
        mixer.set_priority(1, true);
        mixer.set_peer_gain(2, 0.5);
        mixer.set_speaking(1, true);
        settle(&mut mixer, 40);
        assert!((mixer.duck_level() - PRIORITY_DUCK_GAIN).abs() < 1e-6);
        mixer.push(1, 0, frame(1000));
        mixer.push(2, 0, frame(1000));
        mixer.push(3, 0, frame(1000));
        let mixed = mixer.next_frame()[0] as f32;
        let expected = 1000.0 + 1000.0 * 0.5 * PRIORITY_DUCK_GAIN + 1000.0 * PRIORITY_DUCK_GAIN;
        assert!((mixed - expected).abs() <= 2.0, "{mixed} vs {expected}");
    }

    #[test]
    fn ducking_ramps_holds_a_tail_and_releases() {
        let mut mixer = Mixer::new();
        mixer.set_priority(1, true);
        mixer.set_speaking(1, true);
        mixer.next_frame();
        assert!(mixer.duck_level() < 1.0 && mixer.duck_level() > PRIORITY_DUCK_GAIN, "it eases in");
        settle(&mut mixer, 40);
        mixer.set_speaking(1, false);
        settle(&mut mixer, PRIORITY_TAIL_FRAMES as usize / 2);
        assert!((mixer.duck_level() - PRIORITY_DUCK_GAIN).abs() < 1e-6, "the tail keeps the room quiet");
        settle(&mut mixer, PRIORITY_TAIL_FRAMES as usize + 40);
        assert!((mixer.duck_level() - 1.0).abs() < 1e-6, "then everyone comes back");
    }

    #[test]
    fn only_priority_voices_duck_and_losing_the_flag_stops_it() {
        let mut mixer = Mixer::new();
        mixer.set_speaking(5, true);
        settle(&mut mixer, 40);
        assert_eq!(mixer.duck_level(), 1.0, "a plain member talking changes nothing");
        mixer.set_priority(5, true);
        mixer.set_speaking(5, true);
        settle(&mut mixer, 40);
        assert!(mixer.duck_level() < 1.0);
        mixer.set_priority(5, false);
        assert!(!mixer.is_priority(5));
        settle(&mut mixer, PRIORITY_TAIL_FRAMES as usize + 40);
        assert_eq!(mixer.duck_level(), 1.0);
        assert_eq!(duck_target(false, 0), 1.0);
        assert_eq!(duck_target(false, 3), PRIORITY_DUCK_GAIN);
        assert_eq!(duck_step(0.32, PRIORITY_DUCK_GAIN), PRIORITY_DUCK_GAIN);
        assert!((duck_step(1.0, PRIORITY_DUCK_GAIN) - 0.95).abs() < 1e-6);
    }
}
