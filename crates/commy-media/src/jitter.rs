use commy_audio::FRAME_SIZE;
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct JitterBuffer {
    frames: BTreeMap<u32, Vec<i16>>,
    next_seq: Option<u32>,
    newest: Option<u32>,
    max_depth: usize,
}

const RESTARTED_STREAM_GAP: u32 = 50;

impl Default for JitterBuffer {
    fn default() -> Self {
        Self::new(10)
    }
}

impl JitterBuffer {
    pub fn new(max_depth: usize) -> Self {
        Self {
            frames: BTreeMap::new(),
            next_seq: None,
            newest: None,
            max_depth: max_depth.max(2),
        }
    }

    pub fn push(&mut self, seq: u32, pcm: Vec<i16>) {
        if pcm.len() != FRAME_SIZE {
            return;
        }
        if self.newest.is_some_and(|newest| newest > seq && newest - seq > RESTARTED_STREAM_GAP) {
            self.restart();
        }
        let behind_playout = self.next_seq.is_some_and(|next| seq < next);
        if behind_playout {
            let fresh = self.newest.is_none_or(|newest| seq > newest);
            if !fresh {
                return;
            }
            self.next_seq = Some(seq);
        }
        self.newest = Some(self.newest.map_or(seq, |newest| newest.max(seq)));
        self.frames.insert(seq, pcm);
        if self.frames.len() > self.max_depth {
            self.skip_ahead_past_hopeless_lag();
        }
    }

    fn skip_ahead_past_hopeless_lag(&mut self) {
        let newest = *self.frames.keys().next_back().unwrap();
        let cutoff = newest.saturating_sub(self.max_depth as u32 / 2);
        self.frames.retain(|s, _| *s >= cutoff);
        self.next_seq = Some(cutoff);
    }

    fn restart(&mut self) {
        self.frames.clear();
        self.next_seq = None;
        self.newest = None;
    }

    pub fn pop(&mut self) -> Vec<i16> {
        let next = match self.next_seq {
            Some(seq) => seq,
            None => match self.frames.keys().next() {
                Some(first) => *first,
                None => return vec![0; FRAME_SIZE],
            },
        };
        self.next_seq = Some(next + 1);
        self.frames
            .remove(&next)
            .unwrap_or_else(|| vec![0; FRAME_SIZE])
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(value: i16) -> Vec<i16> {
        vec![value; FRAME_SIZE]
    }

    #[test]
    fn in_order_passthrough() {
        let mut jb = JitterBuffer::default();
        jb.push(0, frame(1));
        jb.push(1, frame(2));
        assert_eq!(jb.pop()[0], 1);
        assert_eq!(jb.pop()[0], 2);
    }

    #[test]
    fn reorders_out_of_order_packets() {
        let mut jb = JitterBuffer::default();
        jb.push(1, frame(2));
        jb.push(0, frame(1));
        assert_eq!(jb.pop()[0], 1);
        assert_eq!(jb.pop()[0], 2);
    }

    #[test]
    fn gap_yields_silence_then_resumes() {
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
        jb.push(0, frame(1));
        let _ = jb.pop();
        jb.push(0, frame(9));
        assert!(jb.is_empty());
    }

    #[test]
    fn deep_backlog_skips_ahead() {
        let mut jb = JitterBuffer::new(4);
        for seq in 0..20 {
            jb.push(seq, frame(seq as i16));
        }
        let out = jb.pop();
        assert!(out[0] >= 16, "expected recent frame, got {}", out[0]);
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
        assert!(play_through(&mut jb, 0..50).iter().all(|s| *s == 7));
        for _ in 0..300 {
            assert_eq!(jb.pop()[0], 0);
        }
        assert!(play_through(&mut jb, 50..100).iter().all(|s| *s == 7));
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
    fn a_restarted_stream_is_heard() {
        let mut jb = JitterBuffer::default();
        play_through(&mut jb, 5_000..5_050);
        assert!(play_through(&mut jb, 0..50).iter().all(|s| *s == 7));
    }

    #[test]
    fn a_stall_flushes_late_frames_then_plays_on() {
        let mut jb = JitterBuffer::default();
        play_through(&mut jb, 0..10);
        for _ in 0..30 {
            jb.pop();
        }
        for seq in 10..40 {
            jb.push(seq, frame(seq as i16));
        }
        let first = jb.pop()[0];
        assert!(first >= 29, "expected recent speech after the stall, got {first}");
    }

    #[test]
    fn wrong_size_frames_are_ignored() {
        let mut jb = JitterBuffer::default();
        jb.push(0, vec![1; 10]);
        assert!(jb.is_empty());
    }
}
