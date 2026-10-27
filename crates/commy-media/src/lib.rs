pub mod clip;
pub mod devices;
#[cfg(windows)]
mod display;
#[cfg(any(windows, test))]
mod hdr;
pub mod jitter;
#[cfg(windows)]
mod loopback;
pub mod mixer;
pub mod screen;
#[cfg(windows)]
pub mod wgc;

pub use commy_audio::speaking::{frame_has_signal, SpeakingDetector};
pub use commy_audio::{FRAME_SIZE, SAMPLE_RATE};
pub use clip::{decode_wav, ClipQueue};
pub use devices::{
    list_device_names, start_microphone, start_playback, start_system_audio, system_audio_supported, FrameSource,
    InputHandle, OutputHandle,
};
pub use jitter::JitterBuffer;
pub use mixer::Mixer;

pub struct StreamGuards {
    pub mic: Option<InputHandle>,
    pub speaker: Option<OutputHandle>,
}
