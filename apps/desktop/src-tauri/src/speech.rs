use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use commy_media::{ClipQueue, OutputHandle};
use serde::Serialize;

use crate::state::App;

const RATE_RANGE: (f64, f64) = (0.5, 2.0);
const PITCH_RANGE: (f64, f64) = (0.5, 2.0);
const MAX_SPOKEN_CHARS: usize = 1000;
const MAX_CLIP_BYTES: usize = 64 * 1024 * 1024;
const IDLE_CLOSE_FRAMES: u32 = 300;
const IDLE_CHECK: Duration = Duration::from_secs(1);

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

#[derive(Clone, Debug)]
pub struct Hearing {
    device: Option<String>,
    volume: f32,
}

impl Hearing {
    fn of(app: &App) -> Self {
        let voice = app.voice.lock().unwrap();
        Self { device: voice.output_device.clone(), volume: voice.output_volume }
    }
}

struct Output {
    device: Option<String>,
    id: u64,
    _handle: OutputHandle,
}

struct SpeechOut {
    queue: Arc<Mutex<ClipQueue>>,
    output: Mutex<Option<Output>>,
    generation: AtomicU64,
    opened: AtomicU64,
}

fn out() -> &'static SpeechOut {
    static OUT: OnceLock<SpeechOut> = OnceLock::new();
    OUT.get_or_init(|| SpeechOut {
        queue: Arc::new(Mutex::new(ClipQueue::new())),
        output: Mutex::new(None),
        generation: AtomicU64::new(0),
        opened: AtomicU64::new(0),
    })
}

impl SpeechOut {
    fn stop(&self) -> u64 {
        let mut queue = self.queue.lock().unwrap();
        queue.clear();
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn play(&'static self, expected: Option<u64>, hearing: &Hearing, samples: &[f32], rate: f32, append: bool) -> Result<(), String> {
        {
            let mut queue = self.queue.lock().unwrap();
            match expected {
                Some(generation) if generation != self.generation.load(Ordering::SeqCst) => return Ok(()),
                Some(_) => queue.clear(),
                None if !append => {
                    self.generation.fetch_add(1, Ordering::SeqCst);
                    queue.clear();
                }
                None => {}
            }
            queue.set_volume(hearing.volume);
            queue.push(samples, rate);
        }
        self.open(hearing.device.clone())
    }

    fn open(&'static self, device: Option<String>) -> Result<(), String> {
        let mut output = self.output.lock().unwrap();
        if output.as_ref().is_some_and(|o| o.device == device) {
            return Ok(());
        }
        output.take();
        let handle = commy_media::start_playback(self.queue.clone(), device.clone(), None)
            .map_err(|e| format!("could not open the speakers: {e}"))?;
        let id = self.opened.fetch_add(1, Ordering::SeqCst) + 1;
        *output = Some(Output { device, id, _handle: handle });
        let _ = std::thread::Builder::new().name("speech-idle".into()).spawn(move || self.close_when_idle(id));
        Ok(())
    }

    fn close_when_idle(&self, id: u64) {
        loop {
            std::thread::sleep(IDLE_CHECK);
            let mut output = self.output.lock().unwrap();
            if output.as_ref().map(|o| o.id) != Some(id) {
                return;
            }
            if self.queue.lock().unwrap().idle_frames() >= IDLE_CLOSE_FRAMES {
                output.take();
                return;
            }
        }
    }
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

fn pcm_samples(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
}

#[tauri::command]
pub fn speech_voices() -> Voices {
    match platform::voices() {
        Some(voices) => Voices { native: true, voices },
        None => Voices { native: false, voices: Vec::new() },
    }
}

#[tauri::command]
pub fn speech_speak(app: tauri::State<'_, App>, text: String, voice: Option<String>, rate: Option<f64>, pitch: Option<f64>) -> Result<(), String> {
    let Some(text) = spoken(&text) else { return Ok(()) };
    let rate = within(rate.unwrap_or(1.0), RATE_RANGE);
    let pitch = within(pitch.unwrap_or(1.0), PITCH_RANGE);
    platform::speak(text, voice.filter(|v| !v.is_empty()), rate, pitch, Hearing::of(&app), out().stop())
}

#[tauri::command]
pub fn speech_play(app: tauri::State<'_, App>, request: tauri::ipc::Request<'_>) -> Result<(), String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected the speech audio".into());
    };
    if bytes.len() > MAX_CLIP_BYTES {
        return Err("the speech audio is too long".into());
    }
    let header = |name: &str| request.headers().get(name).and_then(|v| v.to_str().ok());
    let rate: f32 = header("x-commy-rate").and_then(|v| v.parse().ok()).ok_or("the speech audio has no sample rate")?;
    let append = header("x-commy-append") == Some("1");
    out().play(None, &Hearing::of(&app), &pcm_samples(bytes), rate, append)
}

#[tauri::command]
pub fn speech_stop() {
    out().stop();
}

#[cfg(windows)]
mod platform {
    use std::sync::mpsc::{channel, Sender};
    use std::sync::OnceLock;

    use windows::core::HSTRING;
    use windows::Media::SpeechSynthesis::{SpeechSynthesizer, VoiceInformation};
    use windows::Storage::Streams::DataReader;

    use super::{Hearing, Voice};

    struct Job {
        text: String,
        voice: Option<String>,
        rate: f64,
        pitch: f64,
        hearing: Hearing,
        generation: u64,
    }

    static WORKER: OnceLock<Sender<Job>> = OnceLock::new();

    fn worker() -> &'static Sender<Job> {
        WORKER.get_or_init(|| {
            let (tx, rx) = channel::<Job>();
            std::thread::Builder::new()
                .name("speech".into())
                .spawn(move || {
                    for job in rx {
                        let played = synthesize(&job.text, job.voice.as_deref(), job.rate, job.pitch)
                            .map_err(|e| e.to_string())
                            .and_then(|clip| clip.ok_or_else(|| "the voice made audio this app cannot read".to_string()))
                            .and_then(|(samples, rate)| super::out().play(Some(job.generation), &job.hearing, &samples, rate as f32, false));
                        if let Err(e) = played {
                            tracing::warn!("could not read the message aloud: {e}");
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

    fn synthesize(text: &str, voice: Option<&str>, rate: f64, pitch: f64) -> windows::core::Result<Option<(Vec<f32>, u32)>> {
        let synth = SpeechSynthesizer::new()?;
        if let Some(chosen) = voice.map(find_voice).transpose()?.flatten() {
            synth.SetVoice(&chosen)?;
        }
        let options = synth.Options()?;
        options.SetSpeakingRate(rate)?;
        options.SetAudioPitch(pitch)?;
        let stream = synth.SynthesizeTextToStreamAsync(&HSTRING::from(text))?.get()?;
        let size = u32::try_from(stream.Size()?).unwrap_or(u32::MAX);
        let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
        let loaded = reader.LoadAsync(size)?.get()?;
        let mut bytes = vec![0u8; loaded as usize];
        reader.ReadBytes(&mut bytes)?;
        Ok(commy_media::decode_wav(&bytes))
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

    pub fn speak(text: String, voice: Option<String>, rate: f64, pitch: f64, hearing: Hearing, generation: u64) -> Result<(), String> {
        worker()
            .send(Job { text, voice, rate, pitch, hearing, generation })
            .map_err(|_| "text-to-speech stopped working".to_string())
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{Hearing, Voice};

    pub fn voices() -> Option<Vec<Voice>> {
        None
    }

    pub fn speak(_text: String, _voice: Option<String>, _rate: f64, _pitch: f64, _hearing: Hearing, _generation: u64) -> Result<(), String> {
        Err("this app reads aloud through its window's own voices here".into())
    }
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

    #[test]
    fn speech_audio_arrives_as_little_endian_floats() {
        let bytes: Vec<u8> = [0.5f32, -0.25].iter().flat_map(|s| s.to_le_bytes()).chain([1, 2]).collect();
        assert_eq!(pcm_samples(&bytes), vec![0.5, -0.25]);
    }

    #[test]
    fn stopping_makes_older_speech_stale() {
        let first = out().stop();
        let second = out().stop();
        assert!(second > first);
        let hearing = Hearing { device: None, volume: 1.0 };
        assert!(out().play(Some(first), &hearing, &[0.5; 480], 48_000.0, false).is_ok());
        assert_eq!(out().queue.lock().unwrap().queued(), 0);
    }

    #[cfg(not(windows))]
    #[test]
    fn without_a_native_voice_the_window_speaks_instead() {
        assert_eq!(speech_voices(), Voices { native: false, voices: Vec::new() });
        assert!(platform::speak("hi".into(), None, 1.0, 1.0, Hearing { device: None, volume: 1.0 }, 0).is_err());
    }
}
