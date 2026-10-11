use commy_audio::FRAME_SIZE;
use std::collections::BTreeMap;

/// Reorders incoming voice packets per peer. QUIC streams are ordered,
/// so this mostly absorbs scheduling jitter; it also heals the stream
/// after dropped frames by emitting silence.
#[derive(Debug)]
pub struct JitterBuffer {
    frames: BTreeMap<u32, Vec<i16>>,
    next_seq: Option<u32>,
    /// Frames buffered beyond this are considered hopeless lag and the
    /// buffer skips ahead.
    max_depth: usize,
}

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
            max_depth: max_depth.max(2),
        }
    }

    pub fn push(&mut self, seq: u32, pcm: Vec<i16>) {
        if pcm.len() != FRAME_SIZE {
            return;
        }
        if let Some(next) = self.next_seq {
            if seq < next {
                return; // too late, already played past it
            }
        }
        self.frames.insert(seq, pcm);
        if self.frames.len() > self.max_depth {
            // Lagging badly: jump to the newest contiguous region.
            let newest = *self.frames.keys().next_back().unwrap();
            let cutoff = newest.saturating_sub(self.max_depth as u32 / 2);
            self.frames.retain(|s, _| *s >= cutoff);
            self.next_seq = Some(cutoff);
        }
    }

    /// Next 10 ms frame for the mixer: the due packet, or silence.
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
        assert_eq!(jb.pop()[0], 0); // lost frame 1
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
        jb.push(0, frame(9)); // replays a seq already consumed
        assert!(jb.is_empty());
    }

    #[test]
    fn deep_backlog_skips_ahead() {
        let mut jb = JitterBuffer::new(4);
        for seq in 0..20 {
            jb.push(seq, frame(seq as i16));
        }
        let out = jb.pop();
        // After skipping, playback resumes near the newest packets.
        assert!(out[0] >= 16, "expected recent frame, got {}", out[0]);
    }

    #[test]
    fn wrong_size_frames_are_ignored() {
        let mut jb = JitterBuffer::default();
        jb.push(0, vec![1; 10]);
        assert!(jb.is_empty());
    }
}
