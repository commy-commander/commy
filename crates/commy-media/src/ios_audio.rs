use anyhow::{anyhow, Result};
use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_avf_audio::{
    AVAudioSession, AVAudioSessionCategoryOptions, AVAudioSessionCategoryPlayAndRecord, AVAudioSessionModeDefault,
    AVAudioSessionRecordPermission,
};
use std::sync::Mutex;
use tokio::sync::oneshot;

fn voice_options() -> AVAudioSessionCategoryOptions {
    AVAudioSessionCategoryOptions::DefaultToSpeaker
        | AVAudioSessionCategoryOptions::AllowBluetoothHFP
        | AVAudioSessionCategoryOptions::AllowBluetoothA2DP
}

pub fn prepare_session_for_voice() -> Result<()> {
    // SAFETY: the shared audio session and the category and mode constants are provided by AVFAudio for the life of the process, and the setters only read the values passed in.
    unsafe {
        let category = AVAudioSessionCategoryPlayAndRecord.ok_or_else(|| anyhow!("play and record category missing"))?;
        let mode = AVAudioSessionModeDefault.ok_or_else(|| anyhow!("default audio mode missing"))?;
        let session = AVAudioSession::sharedInstance();
        session
            .setCategory_mode_options_error(category, mode, voice_options())
            .map_err(|e| anyhow!("could not set the audio session for voice: {}", e.localizedDescription()))?;
        session
            .setActive_error(true)
            .map_err(|e| anyhow!("could not start the audio session: {}", e.localizedDescription()))?;
    }
    Ok(())
}

pub fn wake_session() {
    // SAFETY: the shared audio session lives for the whole process, and activating it only changes its own state.
    let woke = unsafe { AVAudioSession::sharedInstance().setActive_error(true) };
    if let Err(e) = woke {
        tracing::debug!("the audio session is not back yet: {}", e.localizedDescription());
    }
}

#[allow(deprecated)]
fn known_microphone_answer() -> Option<bool> {
    // SAFETY: reading the record permission of the shared audio session has no side effects.
    let permission = unsafe { AVAudioSession::sharedInstance().recordPermission() };
    if permission == AVAudioSessionRecordPermission::Granted {
        Some(true)
    } else if permission == AVAudioSessionRecordPermission::Denied {
        Some(false)
    } else {
        None
    }
}

#[allow(deprecated)]
fn request_microphone(answer: oneshot::Sender<bool>) {
    let answer = Mutex::new(Some(answer));
    let reply = RcBlock::new(move |granted: Bool| {
        if let Some(answer) = answer.lock().unwrap().take() {
            let _ = answer.send(granted.as_bool());
        }
    });
    // SAFETY: AVFAudio copies the block and keeps it until it answers, and the block only sends that answer on a channel.
    unsafe { AVAudioSession::sharedInstance().requestRecordPermission(&reply) };
}

pub async fn ask_for_microphone() -> bool {
    if let Some(answer) = known_microphone_answer() {
        return answer;
    }
    let (tx, rx) = oneshot::channel();
    request_microphone(tx);
    rx.await.unwrap_or(false)
}
