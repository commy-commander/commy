use serde::Serialize;

const RATE_RANGE: (f64, f64) = (0.5, 2.0);
const PITCH_RANGE: (f64, f64) = (0.5, 2.0);
const MAX_SPOKEN_CHARS: usize = 1000;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Voice {
    pub id: String,
    pub name: String,
    pub lang: String,
    pub default: bool,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Voices {
    pub native: bool,
    pub voices: Vec<Voice>,
}

fn within(value: f64, (low, high): (f64, f64)) -> f64 {
    if value.is_finite() {
        value.clamp(low, high)
    } else {
        1.0
    }
}

fn spoken(text: &str) -> Option<String> {
    let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_SPOKEN_CHARS).collect();
    (!text.is_empty()).then_some(text)
}

#[tauri::command]
pub fn speech_voices() -> Voices {
    match platform::voices() {
        Some(voices) => Voices { native: true, voices },
        None => Voices { native: false, voices: Vec::new() },
    }
}

#[tauri::command]
pub fn speech_speak(text: String, voice: Option<String>, rate: Option<f64>, pitch: Option<f64>) -> Result<(), String> {
    let Some(text) = spoken(&text) else { return Ok(()) };
    let rate = within(rate.unwrap_or(1.0), RATE_RANGE);
    let pitch = within(pitch.unwrap_or(1.0), PITCH_RANGE);
    platform::speak(text, voice.filter(|v| !v.is_empty()), rate, pitch)
}

#[tauri::command]
pub fn speech_stop() {
    platform::stop();
}

#[cfg(windows)]
mod platform {
    use std::sync::mpsc::{channel, Sender};
    use std::sync::OnceLock;

    use windows::core::HSTRING;
    use windows::Media::Core::MediaSource;
    use windows::Media::Playback::{MediaPlayer, MediaPlayerAudioCategory};
    use windows::Media::SpeechSynthesis::{SpeechSynthesizer, VoiceInformation};

    use super::Voice;

    enum Job {
        Speak { text: String, voice: Option<String>, rate: f64, pitch: f64 },
        Stop,
    }

    static WORKER: OnceLock<Sender<Job>> = OnceLock::new();

    fn worker() -> &'static Sender<Job> {
        WORKER.get_or_init(|| {
            let (tx, rx) = channel::<Job>();
            std::thread::Builder::new()
                .name("speech".into())
                .spawn(move || {
                    let mut playing: Option<MediaPlayer> = None;
                    for job in rx {
                        if let Some(player) = playing.take() {
                            let _ = player.Pause();
                            let _ = player.Close();
                        }
                        if let Job::Speak { text, voice, rate, pitch } = job {
                            match play(&text, voice.as_deref(), rate, pitch) {
                                Ok(player) => playing = Some(player),
                                Err(e) => tracing::warn!("could not read the message aloud: {e}"),
                            }
                        }
                    }
                })
                .expect("speech thread starts");
            tx
        })
    }

    fn find_voice(id: &str) -> windows::core::Result<Option<VoiceInformation>> {
        for voice in SpeechSynthesizer::AllVoices()? {
            if voice.Id()?.to_string_lossy() == id {
                return Ok(Some(voice));
            }
        }
        Ok(None)
    }

    fn play(text: &str, voice: Option<&str>, rate: f64, pitch: f64) -> windows::core::Result<MediaPlayer> {
        let synth = SpeechSynthesizer::new()?;
        if let Some(chosen) = voice.map(find_voice).transpose()?.flatten() {
            synth.SetVoice(&chosen)?;
        }
        let options = synth.Options()?;
        options.SetSpeakingRate(rate)?;
        options.SetAudioPitch(pitch)?;
        let stream = synth.SynthesizeTextToStreamAsync(&HSTRING::from(text))?.get()?;
        let source = MediaSource::CreateFromStream(&stream, &stream.ContentType()?)?;
        let player = MediaPlayer::new()?;
        player.CommandManager()?.SetIsEnabled(false)?;
        player.SetAudioCategory(MediaPlayerAudioCategory::Speech)?;
        player.SetSource(&source)?;
        player.Play()?;
        Ok(player)
    }

    fn listed() -> windows::core::Result<Vec<Voice>> {
        let default = SpeechSynthesizer::DefaultVoice().and_then(|v| v.Id()).map(|id| id.to_string_lossy()).unwrap_or_default();
        let mut voices = Vec::new();
        for voice in SpeechSynthesizer::AllVoices()? {
            let id = voice.Id()?.to_string_lossy();
            voices.push(Voice {
                default: id == default,
                id,
                name: voice.DisplayName()?.to_string_lossy(),
                lang: voice.Language()?.to_string_lossy(),
            });
        }
        Ok(voices)
    }

    pub fn voices() -> Option<Vec<Voice>> {
        match listed() {
            Ok(voices) if !voices.is_empty() => Some(voices),
            Ok(_) => None,
            Err(e) => {
                tracing::warn!("could not list the computer's voices: {e}");
                None
            }
        }
    }

    pub fn speak(text: String, voice: Option<String>, rate: f64, pitch: f64) -> Result<(), String> {
        worker().send(Job::Speak { text, voice, rate, pitch }).map_err(|_| "text-to-speech stopped working".to_string())
    }

    pub fn stop() {
        if let Some(tx) = WORKER.get() {
            let _ = tx.send(Job::Stop);
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::Voice;

    pub fn voices() -> Option<Vec<Voice>> {
        None
    }

    pub fn speak(_text: String, _voice: Option<String>, _rate: f64, _pitch: f64) -> Result<(), String> {
        Err("this app reads aloud through its window's own voices here".into())
    }

    pub fn stop() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_and_pitch_stay_within_what_a_voice_can_do() {
        assert_eq!(within(0.1, RATE_RANGE), 0.5);
        assert_eq!(within(9.0, RATE_RANGE), 2.0);
        assert_eq!(within(1.3, PITCH_RANGE), 1.3);
        assert_eq!(within(f64::NAN, PITCH_RANGE), 1.0);
        assert_eq!(within(f64::INFINITY, RATE_RANGE), 1.0);
    }

    #[test]
    fn spoken_text_is_tidied_and_capped() {
        assert_eq!(spoken("  hello \n  there  "), Some("hello there".into()));
        assert_eq!(spoken("   "), None);
        assert_eq!(spoken(&"a".repeat(5000)).map(|t| t.chars().count()), Some(MAX_SPOKEN_CHARS));
    }

    #[cfg(not(windows))]
    #[test]
    fn without_a_native_voice_the_window_speaks_instead() {
        assert_eq!(speech_voices(), Voices { native: false, voices: Vec::new() });
        assert!(speech_speak("hi".into(), None, None, None).is_err());
        assert!(speech_speak("   ".into(), None, None, None).is_ok());
    }
}
