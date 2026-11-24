#[cfg(target_os = "android")]
mod android_audio;
pub mod clip;
pub mod devices;
#[cfg(windows)]
mod display;
#[cfg(any(windows, test))]
mod hdr;
#[cfg(target_os = "ios")]
mod ios_audio;
pub mod jitter;
#[cfg(windows)]
mod loopback;
pub mod mixer;
pub mod screen;
pub mod sound_route;
#[cfg(windows)]
pub mod wgc;

pub use commy_audio::speaking::{frame_has_signal, SpeakingDetector};
pub use commy_audio::{FRAME_SIZE, SAMPLE_RATE};
pub use clip::{decode_wav, ClipMixer, ClipQueue};
pub use devices::{
    list_device_names, start_microphone, start_playback, start_system_audio, system_audio_supported, FrameSource,
    InputHandle, OutputHandle,
};
#[cfg(target_os = "android")]
pub use android_audio::{choose_sound, on_sound_route_change, sound_route};
#[cfg(target_os = "ios")]
pub use ios_audio::{ask_for_microphone, choose_sound, on_sound_route_change, sound_route, want_speaker};
pub use jitter::JitterBuffer;
pub use mixer::Mixer;

pub struct StreamGuards {
    pub mic: Option<InputHandle>,
    pub speaker: Option<OutputHandle>,
}
