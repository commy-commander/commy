pub mod devices;
pub mod jitter;
pub mod mixer;
pub mod screen;
pub mod speaking;
#[cfg(windows)]
pub mod wgc;

pub use commy_audio::{FRAME_SIZE, SAMPLE_RATE};
pub use devices::{
    list_device_names, start_microphone, start_playback, start_system_audio, system_audio_supported, InputHandle,
    OutputHandle,
};
pub use jitter::JitterBuffer;
pub use mixer::Mixer;
pub use speaking::SpeakingDetector;

pub struct StreamGuards {
    pub mic: Option<InputHandle>,
    pub speaker: Option<OutputHandle>,
}

const ACTIVITY_FLOOR: i32 = i16::MAX as i32 / 100;

pub fn frame_has_signal(pcm: &[i16]) -> bool {
    if pcm.is_empty() {
        return false;
    }
    let sum: i64 = pcm.iter().map(|s| i64::from(s.unsigned_abs())).sum();
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
        assert!(frame_has_signal(&[i16::MIN; FRAME_SIZE]));
    }
}
