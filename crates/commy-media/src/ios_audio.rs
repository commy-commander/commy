use anyhow::{anyhow, Result};
use objc2_avf_audio::{
    AVAudioSession, AVAudioSessionCategoryOptions, AVAudioSessionCategoryPlayAndRecord, AVAudioSessionModeDefault,
};

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
