use std::sync::OnceLock;

use commy_media::ClipMixer;

use crate::device_out::{DeviceOut, Hearing};
use crate::speech::pcm_samples;
use crate::state::App;

const MAX_SOUND_BYTES: usize = 32 * 1024 * 1024;

fn mixer() -> &'static DeviceOut<ClipMixer> {
    static OUT: OnceLock<DeviceOut<ClipMixer>> = OnceLock::new();
    OUT.get_or_init(|| DeviceOut::new(ClipMixer::new()))
}

#[tauri::command]
pub fn sound_output() -> bool {
    true
}

#[tauri::command]
pub fn sound_play(app: tauri::State<'_, App>, request: tauri::ipc::Request<'_>) -> Result<(), String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected the sound".into());
    };
    if bytes.len() > MAX_SOUND_BYTES {
        return Err("the sound is too long".into());
    }
    let header = |name: &str| request.headers().get(name).and_then(|v| v.to_str().ok());
    let rate: f32 = header("x-commy-rate").and_then(|v| v.parse().ok()).ok_or("the sound has no sample rate")?;
    let level: f32 = header("x-commy-level").and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let clip: u64 = header("x-commy-clip").and_then(|v| v.parse().ok()).unwrap_or(0);
    let hearing = Hearing::of(&app);
    {
        let mut mixer = mixer().source();
        mixer.set_volume(hearing.volume);
        mixer.push(clip, &pcm_samples(bytes), rate, level);
    }
    mixer().open(hearing.device)
}

#[tauri::command]
pub fn sound_stop(clip: u64) {
    mixer().source().stop(clip);
}
