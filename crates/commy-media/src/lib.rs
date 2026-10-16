//! Media pipelines for Commy.
//!
//! Pure, tested pieces: `JitterBuffer` (reorders voice packets),
//! `Mixer` (sums peers into one output frame), `FrameAssembler`
//! (device samples -> 48 kHz mono frames), JPEG encoding.
//!
//! Device glue (cpal streams, xcap capture) is thin and kept apart so
//! the logic stays testable without hardware.

pub mod devices;
pub mod jitter;
pub mod mixer;
pub mod screen;
pub mod speaking;
#[cfg(windows)]
pub mod wgc;

pub use commy_audio::{FRAME_SIZE, SAMPLE_RATE};
pub use devices::{list_device_names, start_microphone, start_playback, InputHandle, OutputHandle};
pub use jitter::JitterBuffer;
pub use mixer::Mixer;
pub use speaking::SpeakingDetector;

/// Owns the cpal stream handles for one voice session. Dropping it
/// stops capture and playback.
pub struct StreamGuards {
    pub mic: Option<InputHandle>,
    pub speaker: Option<OutputHandle>,
}

/// Mean absolute amplitude above which a frame counts as real voice
/// activity rather than room noise: about 1% of full scale.
const ACTIVITY_FLOOR: i32 = 327;

/// Whether a PCM frame carries enough energy to count as the user
/// actually speaking. Used for AFK idle detection, so it errs toward
/// counting quiet speech as activity rather than the reverse.
pub fn frame_has_signal(pcm: &[i16]) -> bool {
    if pcm.is_empty() {
        return false;
    }
    let sum: i64 = pcm.iter().map(|s| (*s as i64).abs()).sum();
    (sum / pcm.len() as i64) as i32 >= ACTIVITY_FLOOR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_has_no_signal() {
        assert!(!frame_has_signal(&[0i16; FRAME_SIZE]));
        assert!(!frame_has_signal(&[]));
    }

    #[test]
    fn room_noise_has_no_signal() {
        let noise: Vec<i16> = (0..FRAME_SIZE)
            .map(|i| if i % 2 == 0 { 100 } else { -100 })
            .collect();
        assert!(!frame_has_signal(&noise));
    }

    #[test]
    fn speech_level_audio_has_signal() {
        let tone: Vec<i16> = (0..FRAME_SIZE)
            .map(|i| if i % 2 == 0 { 2000 } else { -2000 })
            .collect();
        assert!(frame_has_signal(&tone));
        // i16::MIN must not overflow the abs().
        assert!(frame_has_signal(&[i16::MIN; FRAME_SIZE]));
    }
}
