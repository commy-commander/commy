use commy_audio::codec::VoiceDecoder;
use commy_audio::FRAME_SIZE;
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

const FRAME_MS: i64 = 10;
const RESTARTED_STREAM_GAP: u32 = 50;
pub const MIN_CUSHION_FRAMES: usize = 2;
pub const MAX_CUSHION_FRAMES: usize = 20;
const DEFAULT_MAX_DEPTH: usize = 50;
const EXCESS_BEFORE_CATCHING_UP: usize = 3;
const MAX_CONCEALED_IN_A_ROW: u32 = 5;
const QUIET_FRAME_MEAN_SQUARE: f64 = 300.0 * 300.0;
const LATENESS_WINDOW: Duration = Duration::from_secs(8);
const NEW_TALK_SPURT_AFTER: Duration = Duration::from_millis(250);

#[derive(Debug)]
enum Payload {
    Pcm(Vec<i16>),
    Opus(Vec<u8>),
}

#[derive(Debug, Default)]
struct Lateness {
    offsets: VecDeque<(Instant, i64)>,
    late_ms: VecDeque<(Instant, i64)>,
    epoch: Option<Instant>,
    last_arrival: Option<Instant>,
}

impl Lateness {
    fn arrived(&mut self, seq: u32, at: Instant) {
        let epoch = *self.epoch.get_or_insert(at);
        if self.last_arrival.is_none_or(|last| at.saturating_duration_since(last) > NEW_TALK_SPURT_AFTER) {
            self.offsets.clear();
        }
        self.last_arrival = Some(self.last_arrival.map_or(at, |last| last.max(at)));
        let offset = at.saturating_duration_since(epoch).as_millis() as i64 - seq as i64 * FRAME_MS;
        self.offsets.push_back((at, offset));
        Self::forget_before(&mut self.offsets, at);
        let earliest = self.offsets.iter().map(|(_, o)| *o).min().unwrap_or(offset);
        self.late_ms.push_back((at, offset - earliest));
        Self::forget_before(&mut self.late_ms, at);
    }

    fn forget_before(samples: &mut VecDeque<(Instant, i64)>, now: Instant) {
        while samples.front().is_some_and(|(t, _)| now.saturating_duration_since(*t) > LATENESS_WINDOW) {
            samples.pop_front();
        }
    }

    fn cushion_frames(&self) -> usize {
        let worst = self.late_ms.iter().map(|(_, l)| *l).max().unwrap_or(0).max(0);
        let covering = (worst + FRAME_MS - 1) / FRAME_MS;
        (MIN_CUSHION_FRAMES + covering as usize).min(MAX_CUSHION_FRAMES)
    }

    fn restart(&mut self) {
        self.offsets.clear();
        self.last_arrival = None;
        self.epoch = None;
    }
}

#[derive(Debug)]
pub struct JitterBuffer {
    frames: BTreeMap<u32, Payload>,
    next_seq: Option<u32>,
    newest: Option<u32>,
    max_depth: usize,
    decoder: Option<VoiceDecoder>,
    lateness: Lateness,
    refilling: bool,
    concealed_in_a_row: u32,
}

impl Default for JitterBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_DEPTH)
    }
}

impl JitterBuffer {
    pub fn new(max_depth: usize) -> Self {
        Self {
            frames: BTreeMap::new(),
            next_seq: None,
            newest: None,
            max_depth: max_depth.max(2),
            decoder: None,
            lateness: Lateness::default(),
            refilling: true,
            concealed_in_a_row: 0,
        }
    }

    pub fn push(&mut self, seq: u32, pcm: Vec<i16>) {
        self.push_at(seq, pcm, Instant::now());
    }

    pub fn push_at(&mut self, seq: u32, pcm: Vec<i16>, at: Instant) {
        if pcm.len() == FRAME_SIZE {
            self.insert(seq, Payload::Pcm(pcm), at);
        }
    }

    pub fn push_opus(&mut self, seq: u32, packet: Vec<u8>) {
        self.push_opus_at(seq, packet, Instant::now());
    }

    pub fn push_opus_at(&mut self, seq: u32, packet: Vec<u8>, at: Instant) {
        if packet.is_empty() {
            return;
        }
        self.decoder.get_or_insert_with(VoiceDecoder::new);
        self.insert(seq, Payload::Opus(packet), at);
    }

    fn insert(&mut self, seq: u32, payload: Payload, at: Instant) {
        if self.newest.is_some_and(|newest| newest > seq && newest - seq > RESTARTED_STREAM_GAP) {
            self.restart();
        }
        if self.next_seq.is_some_and(|next| seq < next) || self.frames.contains_key(&seq) {
            return;
        }
        self.lateness.arrived(seq, at);
        self.newest = Some(self.newest.map_or(seq, |newest| newest.max(seq)));
        self.frames.insert(seq, payload);
        if self.frames.len() > self.max_depth {
            self.skip_ahead_past_hopeless_lag();
        }
    }

    fn skip_ahead_past_hopeless_lag(&mut self) {
        let newest = *self.frames.keys().next_back().unwrap();
        let keep = self.cushion().min(self.max_depth / 2).max(1) as u32;
        let cutoff = newest.saturating_sub(keep - 1);
        self.frames.retain(|s, _| *s >= cutoff);
        self.next_seq = Some(cutoff);
    }

    fn restart(&mut self) {
        self.frames.clear();
        self.next_seq = None;
        self.newest = None;
        self.refilling = true;
        self.concealed_in_a_row = 0;
        self.lateness.restart();
        if self.decoder.is_some() {
            self.decoder = Some(VoiceDecoder::new());
        }
    }

    pub fn cushion(&self) -> usize {
        self.lateness.cushion_frames()
    }

    pub fn depth(&self) -> usize {
        match (self.next_seq.or_else(|| self.frames.keys().next().copied()), self.newest) {
            (Some(next), Some(newest)) if newest >= next => (newest - next + 1) as usize,
            _ => 0,
        }
    }

    pub fn pop(&mut self) -> Vec<i16> {
        let Some(next) = self.next_seq.or_else(|| self.frames.keys().next().copied()) else {
            return self.fill_gap();
        };
        self.next_seq = Some(next);
        if self.refilling {
            if self.depth() < self.cushion() {
                return self.fill_gap();
            }
            self.refilling = false;
        }
        if let Some(payload) = self.frames.remove(&next) {
            self.next_seq = Some(next.wrapping_add(1));
            self.concealed_in_a_row = 0;
            let pcm = self.play(payload);
            return self.catch_up_in_a_pause(pcm);
        }
        if self.frames.range(next..).next().is_some() {
            self.next_seq = Some(next.wrapping_add(1));
            return self.rebuild_lost(next);
        }
        self.refilling = true;
        self.fill_gap()
    }

    fn play(&mut self, payload: Payload) -> Vec<i16> {
        match (payload, self.decoder.as_mut()) {
            (Payload::Pcm(pcm), _) => pcm,
            (Payload::Opus(packet), Some(decoder)) => decoder.decode(&packet),
            (Payload::Opus(_), None) => vec![0; FRAME_SIZE],
        }
    }

    fn catch_up_in_a_pause(&mut self, pcm: Vec<i16>) -> Vec<i16> {
        let Some(next) = self.next_seq else { return pcm };
        let excess = self.depth() > self.cushion() + EXCESS_BEFORE_CATCHING_UP;
        if !excess || !is_quiet(&pcm) || !self.frames.contains_key(&next) {
            return pcm;
        }
        let payload = self.frames.remove(&next).unwrap();
        self.next_seq = Some(next.wrapping_add(1));
        self.play(payload)
    }

    fn rebuild_lost(&mut self, lost: u32) -> Vec<i16> {
        self.concealed_in_a_row += 1;
        let following = match self.frames.get(&lost.wrapping_add(1)) {
            Some(Payload::Opus(packet)) => Some(packet.clone()),
            _ => None,
        };
        match (self.decoder.as_mut(), following) {
            (Some(decoder), Some(packet)) => decoder.recover_from_next(&packet),
            (Some(decoder), None) => decoder.conceal(),
            (None, _) => vec![0; FRAME_SIZE],
        }
    }

    fn fill_gap(&mut self) -> Vec<i16> {
        match self.decoder.as_mut() {
            Some(decoder) if self.concealed_in_a_row < MAX_CONCEALED_IN_A_ROW => {
                self.concealed_in_a_row += 1;
                decoder.conceal()
            }
            _ => vec![0; FRAME_SIZE],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

fn is_quiet(pcm: &[i16]) -> bool {
    let mean_square = pcm.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / pcm.len().max(1) as f64;
    mean_square < QUIET_FRAME_MEAN_SQUARE
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_audio::codec::VoiceEncoder;

    fn frame(value: i16) -> Vec<i16> {
        vec![value; FRAME_SIZE]
    }

    fn primed(jb: &mut JitterBuffer, seqs: std::ops::Range<u32>) {
        for seq in seqs {
            jb.push(seq, frame(seq as i16 + 1));
        }
    }

    #[test]
    fn in_order_passthrough_after_a_small_cushion() {
        let mut jb = JitterBuffer::default();
        jb.push(0, frame(1));
        assert_eq!(jb.pop()[0], 0, "one frame is not yet a cushion");
        jb.push(1, frame(2));
        assert_eq!(jb.pop()[0], 1);
        assert_eq!(jb.pop()[0], 2);
    }

    #[test]
    fn reorders_out_of_order_packets() {
        let mut jb = JitterBuffer::default();
        let start = Instant::now();
        jb.push_at(1, frame(2), start + Duration::from_millis(10));
        jb.push_at(0, frame(1), start + Duration::from_millis(10));
        jb.push_at(2, frame(3), start + Duration::from_millis(20));
        assert_eq!(jb.cushion(), MIN_CUSHION_FRAMES + 1, "the early frame that came second was late");
        assert_eq!(jb.pop()[0], 1);
        assert_eq!(jb.pop()[0], 2);
        assert_eq!(jb.pop()[0], 3);
    }

    #[test]
    fn a_lost_pcm_frame_is_silent_then_playback_resumes() {
        let mut jb = JitterBuffer::default();
        jb.push(0, frame(1));
        jb.push(2, frame(3));
        assert_eq!(jb.pop()[0], 1);
        assert_eq!(jb.pop()[0], 0);
        assert_eq!(jb.pop()[0], 3);
    }

    #[test]
    fn empty_buffer_yields_silence() {
        let mut jb = JitterBuffer::default();
        assert_eq!(jb.pop(), frame(0));
    }

    #[test]
    fn late_packet_is_dropped() {
        let mut jb = JitterBuffer::default();
        primed(&mut jb, 0..2);
        let _ = jb.pop();
        jb.push(0, frame(9));
        assert_eq!(jb.pop()[0], 2);
        assert!(jb.is_empty());
    }

    #[test]
    fn duplicates_are_ignored() {
        let mut jb = JitterBuffer::default();
        primed(&mut jb, 0..2);
        jb.push(1, frame(9));
        assert_eq!(jb.pop()[0], 1);
        assert_eq!(jb.pop()[0], 2);
    }

    #[test]
    fn deep_backlog_skips_ahead() {
        let mut jb = JitterBuffer::new(4);
        primed(&mut jb, 0..20);
        let out = jb.pop();
        assert!(out[0] >= 18, "expected recent frame, got {}", out[0]);
    }

    fn play_through(jb: &mut JitterBuffer, seqs: std::ops::Range<u32>) -> Vec<i16> {
        seqs.map(|seq| {
            jb.push(seq, frame(7));
            jb.pop()[0]
        })
        .collect()
    }

    #[test]
    fn a_sender_who_unmutes_is_heard_again() {
        let mut jb = JitterBuffer::default();
        assert!(play_through(&mut jb, 0..50).iter().skip(1).all(|s| *s == 7));
        assert_eq!(jb.pop()[0], 7);
        for _ in 0..300 {
            assert_eq!(jb.pop()[0], 0);
        }
        assert!(play_through(&mut jb, 50..100).iter().skip(1).all(|s| *s == 7));
    }

    #[test]
    fn a_sender_whose_clock_runs_slow_stays_audible() {
        let mut jb = JitterBuffer::default();
        let mut heard = 0;
        for seq in 0..10_000u32 {
            jb.push(seq, frame(7));
            heard += usize::from(jb.pop()[0] == 7);
            if seq % 100 == 99 {
                jb.pop();
            }
        }
        assert!(heard > 9_800, "heard {heard} of 10000 frames");
    }

    #[test]
    fn a_sender_whose_clock_runs_fast_does_not_build_up_delay() {
        let mut jb = JitterBuffer::default();
        for seq in 0..10_000u32 {
            jb.push(seq, frame(if seq % 50 < 10 { 0 } else { 7 }));
            if seq % 100 != 99 {
                jb.pop();
            }
        }
        assert!(jb.depth() <= jb.cushion() + EXCESS_BEFORE_CATCHING_UP + 1, "depth {} cushion {}", jb.depth(), jb.cushion());
    }

    #[test]
    fn a_restarted_stream_is_heard() {
        let mut jb = JitterBuffer::default();
        play_through(&mut jb, 5_000..5_050);
        assert!(play_through(&mut jb, 0..50).iter().skip(1).all(|s| *s == 7));
    }

    #[test]
    fn a_stall_is_absorbed_instead_of_skipped() {
        let mut jb = JitterBuffer::default();
        let spoken = |seq: u32| frame(1_000 + seq as i16);
        for seq in 0..10 {
            jb.push(seq, spoken(seq));
        }
        for _ in 0..18 {
            jb.pop();
        }
        for seq in 10..20 {
            jb.push(seq, spoken(seq));
        }
        let heard: Vec<i16> = (0..10).map(|_| jb.pop()[0]).collect();
        assert_eq!(heard, (1_010..1_020).collect::<Vec<i16>>(), "every delayed frame plays, in order");
    }

    #[test]
    fn a_backlog_catches_up_during_pauses_only() {
        let mut jb = JitterBuffer::default();
        for seq in 0..10 {
            jb.push(seq, frame(1_000));
        }
        for seq in 10..20 {
            jb.push(seq, frame(0));
        }
        let speech = (0..10).filter(|_| jb.pop()[0] == 1_000).count();
        assert_eq!(speech, 10, "speech is never skipped");
        let mut pops = 0;
        while jb.depth() > jb.cushion() {
            jb.pop();
            pops += 1;
        }
        assert!(pops < 10, "the pause is shortened, {pops} pops for 10 frames");
    }

    #[test]
    fn wrong_size_frames_are_ignored() {
        let mut jb = JitterBuffer::default();
        jb.push(0, vec![1; 10]);
        assert!(jb.is_empty());
    }

    #[test]
    fn jittery_arrivals_grow_the_cushion_and_a_calm_link_shrinks_it() {
        let mut jb = JitterBuffer::default();
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        for seq in 0..100u32 {
            jb.push_at(seq, frame(1), at(seq as u64 * 10));
        }
        assert_eq!(jb.cushion(), MIN_CUSHION_FRAMES);
        for seq in 100..200u32 {
            let late = if seq % 20 == 0 { 70 } else { 0 };
            jb.push_at(seq, frame(1), at(seq as u64 * 10 + late));
        }
        assert_eq!(jb.cushion(), MIN_CUSHION_FRAMES + 7);
        for seq in 200..1200u32 {
            jb.push_at(seq, frame(1), at(seq as u64 * 10));
        }
        assert_eq!(jb.cushion(), MIN_CUSHION_FRAMES);
    }

    #[test]
    fn a_pause_in_speech_is_not_mistaken_for_lateness() {
        let mut jb = JitterBuffer::default();
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        for seq in 0..50u32 {
            jb.push_at(seq, frame(1), at(seq as u64 * 10));
        }
        for seq in 50..100u32 {
            jb.push_at(seq, frame(1), at(3_000 + seq as u64 * 10));
        }
        assert_eq!(jb.cushion(), MIN_CUSHION_FRAMES);
    }

    #[test]
    fn the_cushion_has_a_ceiling() {
        let mut jb = JitterBuffer::default();
        let start = Instant::now();
        jb.push_at(0, frame(1), start);
        jb.push_at(1, frame(1), start + Duration::from_millis(240));
        assert_eq!(jb.cushion(), MAX_CUSHION_FRAMES);
    }

    fn voice(n: usize) -> Vec<Vec<u8>> {
        let mut enc = VoiceEncoder::new();
        (0..n)
            .map(|f| {
                let pcm: Vec<i16> = (0..FRAME_SIZE)
                    .map(|i| {
                        let t = (f * FRAME_SIZE + i) as f32 / commy_audio::SAMPLE_RATE as f32;
                        ((t * 200.0 * std::f32::consts::TAU).sin() * 8000.0) as i16
                    })
                    .collect();
                enc.encode(&pcm).unwrap()
            })
            .collect()
    }

    fn loud(pcm: &[i16]) -> bool {
        pcm.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / pcm.len() as f64 > 1e5
    }

    #[test]
    fn lost_voice_is_rebuilt_instead_of_going_silent() {
        let packets = voice(60);
        let mut jb = JitterBuffer::default();
        let mut heard = Vec::new();
        for (seq, packet) in packets.iter().enumerate() {
            if seq != 40 && seq != 41 {
                jb.push_opus(seq as u32, packet.clone());
            }
            heard.push(jb.pop());
        }
        assert!(heard[5..].iter().all(|pcm| loud(pcm)), "no silent holes where frames 40 and 41 were lost");
    }

    #[test]
    fn a_late_voice_stream_is_smoothed_over_briefly_then_falls_silent() {
        let packets = voice(30);
        let mut jb = JitterBuffer::default();
        for (seq, packet) in packets.iter().enumerate() {
            jb.push_opus(seq as u32, packet.clone());
        }
        for _ in 0..30 {
            jb.pop();
        }
        let tail: Vec<Vec<i16>> = (0..20).map(|_| jb.pop()).collect();
        assert!(loud(&tail[0]), "the first missing frame is concealed");
        assert!(tail[MAX_CONCEALED_IN_A_ROW as usize..].iter().all(|pcm| pcm.iter().all(|s| *s == 0)), "then it goes quiet");
    }

    #[test]
    fn a_restarted_voice_stream_gets_a_fresh_decoder() {
        let packets = voice(10);
        let mut jb = JitterBuffer::default();
        for (seq, packet) in packets.iter().enumerate() {
            jb.push_opus(5_000 + seq as u32, packet.clone());
        }
        jb.pop();
        for (seq, packet) in packets.iter().enumerate() {
            jb.push_opus(seq as u32, packet.clone());
        }
        assert_eq!(jb.depth(), 10);
        assert_eq!(jb.pop().len(), FRAME_SIZE);
    }

    #[test]
    fn empty_voice_packets_are_ignored() {
        let mut jb = JitterBuffer::default();
        jb.push_opus(0, Vec::new());
        assert!(jb.is_empty());
        assert!(jb.decoder.is_none());
    }
}
