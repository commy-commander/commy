use crate::jitter::JitterBuffer;
use commy_audio::FRAME_SIZE;
use std::collections::{HashMap, HashSet};

pub const PRIORITY_DUCK_GAIN: f32 = 0.3;
pub const PRIORITY_TAIL_FRAMES: u32 = 60;
const DUCK_STEP: f32 = 0.05;

#[derive(Debug)]
pub struct Mixer {
    peers: HashMap<u64, JitterBuffer>,
    peer_gains: HashMap<u64, f32>,
    master: f32,
    priority: HashSet<u64>,
    priority_talking: HashSet<u64>,
    duck: f32,
    duck_tail: u32,
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

    pub fn set_master(&mut self, gain: f32) {
        self.master = gain.clamp(0.0, 2.0);
    }

    pub fn master(&self) -> f32 {
        self.master
    }

    pub fn set_peer_gain(&mut self, peer: u64, gain: f32) {
        self.peer_gains.insert(peer, gain.clamp(0.0, 2.0));
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
        acc.into_iter()
            .map(|s| (s * self.master).clamp(i16::MIN as f32, i16::MAX as f32) as i16)
            .collect()
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
    fn overflow_saturates() {
        let mut mixer = Mixer::new();
        mixer.push(1, 0, frame(i16::MAX));
        mixer.push(2, 0, frame(i16::MAX));
        assert_eq!(mixer.next_frame(), frame(i16::MAX));
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
        assert_eq!(mixer.master(), 2.0);
    }

    #[test]
    fn per_peer_gain_applies_before_mix() {
        let mut mixer = Mixer::new();
        mixer.set_peer_gain(1, 2.0);
        mixer.set_peer_gain(2, 0.0);
        mixer.push(1, 0, frame(100));
        mixer.push(2, 0, frame(5000));
        assert_eq!(mixer.next_frame(), frame(200));
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
