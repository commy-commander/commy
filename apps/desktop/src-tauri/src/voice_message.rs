use crate::state::VoiceSettings;
use commy_audio::codec::VoiceEncoder;
use commy_audio::ogg::OggOpusWriter;
use commy_audio::{VoiceProcessor, FRAME_SIZE};
use commy_media::InputHandle;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub const MAX_VOICE_MESSAGE_SECS: u64 = 300;
pub const MIN_VOICE_MESSAGE_MS: u64 = 500;
pub const MAX_VOICE_MESSAGE_BYTES: u64 = 8 * 1024 * 1024;
const FRAMES_PER_SECOND: u64 = 100;
const MAX_FRAMES: u64 = MAX_VOICE_MESSAGE_SECS * FRAMES_PER_SECOND;
const STOP_GRACE: Duration = Duration::from_secs(5);

pub struct Recording {
    pub bytes: Vec<u8>,
    pub duration_ms: u64,
}

pub struct VoiceRecorder {
    mic: Option<InputHandle>,
    clip: JoinHandle<OggOpusWriter>,
}

impl VoiceRecorder {
    pub fn start(voice: &VoiceSettings) -> anyhow::Result<Self> {
        let mut processor = VoiceProcessor::new();
        processor.set_echo_cancellation(false);
        processor.set_noise_cancellation(voice.noise_cancellation);
        processor.set_input_gain(voice.input_gain);
        let (tx, rx) = mpsc::unbounded_channel::<Vec<i16>>();
        let mic = commy_media::start_microphone(Arc::new(Mutex::new(processor)), tx, voice.input_device.clone())?;
        let serial = stream_serial();
        let clip = tokio::spawn(encode_clip(rx, serial, MAX_FRAMES));
        Ok(Self { mic: Some(mic), clip })
    }

    pub async fn stop(mut self) -> anyhow::Result<Recording> {
        drop(self.mic.take());
        let writer = tokio::time::timeout(STOP_GRACE, self.clip).await??;
        let duration_ms = writer.duration_ms();
        Ok(Recording { bytes: writer.finish(), duration_ms })
    }

    pub async fn discard(mut self) {
        drop(self.mic.take());
        let _ = tokio::time::timeout(STOP_GRACE, self.clip).await;
    }
}

async fn encode_clip(mut frames: mpsc::UnboundedReceiver<Vec<i16>>, serial: u32, max_frames: u64) -> OggOpusWriter {
    let mut encoder = VoiceEncoder::new();
    let mut writer = OggOpusWriter::new(serial);
    let mut count = 0u64;
    while let Some(pcm) = frames.recv().await {
        if count >= max_frames {
            continue;
        }
        if let Some(packet) = encoder.encode(&pcm) {
            writer.push(&packet, FRAME_SIZE as u32);
            count += 1;
        }
    }
    writer
}

fn stream_serial() -> u32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    nanos ^ (std::process::id().rotate_left(16))
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

    #[tokio::test]
    async fn frames_become_an_ogg_opus_clip_with_the_right_length() {
        let (tx, rx) = mpsc::unbounded_channel();
        let clip = tokio::spawn(encode_clip(rx, 42, MAX_FRAMES));
        for f in 0..200 {
            tx.send(tone(f)).unwrap();
        }
        tx.send(vec![0; 7]).unwrap();
        drop(tx);
        let writer = clip.await.unwrap();
        let expected = (200 * FRAME_SIZE as u64 - u64::from(commy_audio::ogg::OPUS_PRE_SKIP)) * 1000 / 48_000;
        assert_eq!(writer.duration_ms(), expected);
        let bytes = writer.finish();
        assert_eq!(&bytes[..4], b"OggS");
        assert_eq!(&bytes[28..36], b"OpusHead");
        assert!(bytes.len() > 2000 && bytes.len() < 20_000, "{} bytes for two seconds of tone", bytes.len());
    }

    #[tokio::test]
    async fn recordings_stop_growing_at_the_cap() {
        let (tx, rx) = mpsc::unbounded_channel();
        let clip = tokio::spawn(encode_clip(rx, 1, 300));
        let silence = vec![0i16; FRAME_SIZE];
        for _ in 0..800 {
            tx.send(silence.clone()).unwrap();
        }
        drop(tx);
        let writer = clip.await.unwrap();
        assert_eq!(writer.duration_ms(), (300 * FRAME_SIZE as u64 - u64::from(commy_audio::ogg::OPUS_PRE_SKIP)) * 1000 / 48_000);
        assert_eq!(MAX_FRAMES * 10, MAX_VOICE_MESSAGE_SECS * 1000);
    }
}
