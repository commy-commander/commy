use commy_audio::codec::VoiceEncoder;
use commy_audio::ogg::OggOpusWriter;
use commy_audio::FRAME_SIZE;

use crate::files::MAX_VOICE_MESSAGE_BYTES;

pub const MAX_VOICE_MESSAGE_SECS: u64 = 300;
pub const MIN_VOICE_MESSAGE_MS: u64 = 500;
const FRAMES_PER_SECOND: u64 = 100;
const MAX_FRAMES: u64 = MAX_VOICE_MESSAGE_SECS * FRAMES_PER_SECOND;

pub struct Recorder {
    encoder: VoiceEncoder,
    writer: OggOpusWriter,
    frames: u64,
}

impl Recorder {
    pub fn new(serial: u32) -> Self {
        Self { encoder: VoiceEncoder::new(), writer: OggOpusWriter::new(serial), frames: 0 }
    }

    pub fn push(&mut self, pcm: &[i16]) {
        if self.frames >= MAX_FRAMES || pcm.len() != FRAME_SIZE {
            return;
        }
        if let Some(packet) = self.encoder.encode(pcm) {
            self.writer.push(&packet, FRAME_SIZE as u32);
            self.frames += 1;
        }
    }

    pub fn finish(self) -> Result<Vec<u8>, String> {
        if self.writer.duration_ms() < MIN_VOICE_MESSAGE_MS {
            return Err("that voice message is too short to send".into());
        }
        let bytes = self.writer.finish();
        if bytes.len() as u64 > MAX_VOICE_MESSAGE_BYTES {
            return Err("that voice message is too long to send".into());
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(frame: u64) -> Vec<i16> {
        (0..FRAME_SIZE)
            .map(|i| {
                let t = (frame as usize * FRAME_SIZE + i) as f32 / 48_000.0;
                ((t * 330.0 * std::f32::consts::TAU).sin() * 9000.0) as i16
            })
            .collect()
    }

    #[test]
    fn two_seconds_of_microphone_become_an_ogg_opus_clip() {
        let mut recorder = Recorder::new(7);
        for f in 0..200 {
            recorder.push(&tone(f));
        }
        recorder.push(&[0; 7]);
        let bytes = recorder.finish().unwrap();
        assert_eq!(&bytes[..4], b"OggS");
        assert_eq!(&bytes[28..36], b"OpusHead");
    }

    #[test]
    fn a_tap_on_record_is_too_short_to_send() {
        let mut recorder = Recorder::new(7);
        for f in 0..20 {
            recorder.push(&tone(f));
        }
        assert_eq!(recorder.finish().unwrap_err(), "that voice message is too short to send");
    }

    #[test]
    fn recordings_stop_growing_at_five_minutes() {
        let mut recorder = Recorder::new(7);
        let frame = tone(0);
        for _ in 0..MAX_FRAMES + 50 {
            recorder.push(&frame);
        }
        assert_eq!(recorder.frames, MAX_FRAMES);
    }
}
