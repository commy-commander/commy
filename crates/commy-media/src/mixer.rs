use crate::jitter::JitterBuffer;
use commy_audio::FRAME_SIZE;
use std::collections::HashMap;

/// Sums the voice streams of every connected peer into one output
/// frame. Keyed by an opaque peer id chosen by the caller.
#[derive(Debug)]
pub struct Mixer {
    peers: HashMap<u64, JitterBuffer>,
    peer_gains: HashMap<u64, f32>,
    master: f32,
}

impl Default for Mixer {
    fn default() -> Self {
        Self {
            peers: HashMap::new(),
            peer_gains: HashMap::new(),
            master: 1.0,
        }
    }
}

impl Mixer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, peer: u64, seq: u32, pcm: Vec<i16>) {
        self.peers.entry(peer).or_default().push(seq, pcm);
    }

    /// Drops a peer's stream. Its volume is kept, so a personal volume
    /// or mute survives the peer reconnecting.
    pub fn remove_peer(&mut self, peer: u64) {
        self.peers.remove(&peer);
    }

    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Master output volume; 1.0 is unity, clamped to [0, 2].
    pub fn set_master(&mut self, gain: f32) {
        self.master = gain.clamp(0.0, 2.0);
    }

    pub fn master(&self) -> f32 {
        self.master
    }

    /// Per-peer volume; 1.0 is unity, clamped to [0, 2].
    pub fn set_peer_gain(&mut self, peer: u64, gain: f32) {
        self.peer_gains.insert(peer, gain.clamp(0.0, 2.0));
    }

    /// Next mixed 10 ms frame, saturating on overflow.
    pub fn next_frame(&mut self) -> Vec<i16> {
        let mut acc = vec![0f32; FRAME_SIZE];
        for (peer, jb) in self.peers.iter_mut() {
            let gain = *self.peer_gains.get(peer).unwrap_or(&1.0);
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
        // Clamped to at most 2x.
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
        // Jitter buffers may hold the first frame back; drain a few.
        let loudest = (0..5).flat_map(|_| mixer.next_frame()).map(|s| s.abs()).max().unwrap();
        assert_eq!(loudest, 0, "a muted peer stays muted after reconnecting");
    }
}
