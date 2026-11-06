use commy_audio::filters::{VoiceChanger, VoiceFilter};
use commy_audio::{level_db, Sensitivity, VoiceGate, FRAME_SIZE};
use nnnoiseless::DenoiseState;
use serde_json::Value;

const FULL_SCALE: f32 = 32767.0;
const OPEN_AT: f32 = 0.7;
pub const DEFAULT_TALK_KEY: &str = "Ctrl+Shift+Space";
pub const DEFAULT_RELEASE_MS: u32 = 200;
const MAX_RELEASE_MS: u32 = 2000;
const DEFAULT_SENSITIVITY_DB: f32 = -45.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MicSettings {
    pub push_to_talk: bool,
    pub release_ms: u32,
    pub sensitivity: Sensitivity,
    pub filter: VoiceFilter,
}

impl Default for MicSettings {
    fn default() -> Self {
        Self { push_to_talk: false, release_ms: DEFAULT_RELEASE_MS, sensitivity: Sensitivity::Automatic, filter: VoiceFilter::Off }
    }
}

impl MicSettings {
    pub fn of(settings: &Value) -> Self {
        let release_ms = settings["talk_release_ms"].as_u64().map_or(DEFAULT_RELEASE_MS, |ms| ms.min(u64::from(MAX_RELEASE_MS)) as u32);
        let db = settings["sensitivity_db"].as_f64().map(|db| db as f32).filter(|db| db.is_finite()).unwrap_or(DEFAULT_SENSITIVITY_DB).clamp(-100.0, 0.0);
        Self {
            push_to_talk: settings["input_mode"] == "push_to_talk",
            release_ms,
            sensitivity: if settings["auto_sensitivity"] == false { Sensitivity::Threshold(db) } else { Sensitivity::Automatic },
            filter: settings["voice_filter"].as_str().and_then(VoiceFilter::from_name).unwrap_or_default(),
        }
    }
}

fn to_pcm(sample: f32) -> i16 {
    (sample * FULL_SCALE).round().clamp(-FULL_SCALE, FULL_SCALE) as i16
}

pub struct MicGate {
    settings: MicSettings,
    talk_down: bool,
    talk_press: u64,
    vad: Box<DenoiseState<'static>>,
    gate: VoiceGate,
    filter_allowed: bool,
    was_held: bool,
    changer: VoiceChanger,
    test_changer: VoiceChanger,
}

impl Default for MicGate {
    fn default() -> Self {
        Self {
            settings: MicSettings::default(),
            talk_down: false,
            talk_press: 0,
            vad: DenoiseState::new(),
            gate: VoiceGate::default(),
            filter_allowed: true,
            was_held: false,
            changer: VoiceChanger::new(),
            test_changer: VoiceChanger::new(),
        }
    }
}

impl MicGate {
    pub fn configure(&mut self, settings: MicSettings) -> Option<bool> {
        if settings.push_to_talk != self.settings.push_to_talk {
            self.talk_down = false;
            self.talk_press += 1;
        }
        let was = self.filter_held_back();
        self.settings = settings;
        self.changer.set(self.sent_filter());
        let now = self.filter_held_back();
        (was != now).then_some(now)
    }

    pub fn allow_filter(&mut self, allowed: bool) -> Option<bool> {
        let was = self.filter_held_back();
        self.filter_allowed = allowed;
        self.changer.set(self.sent_filter());
        let now = self.filter_held_back();
        (was != now).then_some(now)
    }

    pub fn filter_held_back(&self) -> bool {
        self.settings.filter.is_on() && !self.filter_allowed
    }

    fn sent_filter(&self) -> VoiceFilter {
        if self.filter_allowed {
            self.settings.filter
        } else {
            VoiceFilter::Off
        }
    }

    pub fn test_frame(&mut self, pcm: &[i16]) -> Vec<i16> {
        self.test_changer.set(self.sent_filter());
        let mut out = Vec::with_capacity(pcm.len());
        for chunk in pcm.chunks(FRAME_SIZE) {
            let mut samples: Vec<f32> = chunk.iter().map(|s| f32::from(*s) / FULL_SCALE).collect();
            self.test_changer.process(&mut samples);
            out.extend(samples.into_iter().map(to_pcm));
        }
        out
    }

    pub fn settings(&self) -> MicSettings {
        self.settings
    }

    pub fn press(&mut self, down: bool) -> Option<u64> {
        self.talk_press += 1;
        if down {
            if !self.settings.push_to_talk {
                return None;
            }
            self.talk_down = true;
            return None;
        }
        Some(self.talk_press)
    }

    pub fn release(&mut self, press: u64) -> bool {
        if press != self.talk_press || !self.talk_down {
            return false;
        }
        self.talk_down = false;
        true
    }

    pub fn held(&self) -> bool {
        self.settings.push_to_talk && !self.talk_down
    }

    pub fn process(&mut self, pcm: &mut [i16]) -> bool {
        if self.held() {
            if !self.was_held {
                self.changer.clear();
            }
            self.was_held = true;
            return false;
        }
        self.was_held = false;
        for chunk in pcm.chunks_mut(FRAME_SIZE) {
            let mut samples: Vec<f32> = chunk.iter().map(|s| f32::from(*s) / FULL_SCALE).collect();
            let open = match self.settings.sensitivity {
                Sensitivity::Threshold(db) => level_db(&samples) >= db,
                Sensitivity::Automatic => self.voiced(chunk),
            };
            self.gate.apply(&mut samples, open);
            self.changer.process(&mut samples);
            for (out, s) in chunk.iter_mut().zip(samples) {
                *out = to_pcm(s);
            }
        }
        true
    }

    fn voiced(&mut self, chunk: &[i16]) -> bool {
        if chunk.len() != FRAME_SIZE {
            return true;
        }
        let input: Vec<f32> = chunk.iter().map(|s| f32::from(*s)).collect();
        let mut output = [0.0f32; FRAME_SIZE];
        let vad = self.vad.process_frame(&mut output, &input);
        if !vad.is_finite() {
            self.vad = DenoiseState::new();
            return false;
        }
        vad >= OPEN_AT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tone(amplitude: f32) -> Vec<i16> {
        (0..FRAME_SIZE).map(|i| ((i as f32 * 0.07).sin() * amplitude * FULL_SCALE) as i16).collect()
    }

    fn loudness(pcm: &[i16]) -> i64 {
        pcm.iter().map(|s| i64::from(s.unsigned_abs())).sum()
    }

    #[test]
    fn settings_come_from_the_saved_voice_choices() {
        assert_eq!(MicSettings::of(&json!({})), MicSettings::default());
        let saved = MicSettings::of(&json!({ "input_mode": "push_to_talk", "talk_release_ms": 9000, "auto_sensitivity": false, "sensitivity_db": -300, "voice_filter": "robot" }));
        assert_eq!(saved, MicSettings { push_to_talk: true, release_ms: 2000, sensitivity: Sensitivity::Threshold(-100.0), filter: VoiceFilter::Robot });
        assert_eq!(MicSettings::of(&json!({ "voice_filter": "kazoo" })).filter, VoiceFilter::Off);
    }

    #[test]
    fn push_to_talk_sends_only_while_the_key_is_held_and_a_moment_after() {
        let mut gate = MicGate::default();
        gate.configure(MicSettings { push_to_talk: true, ..MicSettings::default() });
        assert!(!gate.process(&mut tone(0.3)));
        assert_eq!(gate.press(true), None);
        assert!(gate.process(&mut tone(0.3)));
        let release = gate.press(false).unwrap();
        assert!(gate.process(&mut tone(0.3)));
        assert!(gate.release(release));
        assert!(!gate.process(&mut tone(0.3)));
    }

    #[test]
    fn pressing_again_before_the_release_delay_keeps_the_mic_open() {
        let mut gate = MicGate::default();
        gate.configure(MicSettings { push_to_talk: true, ..MicSettings::default() });
        gate.press(true);
        let first = gate.press(false).unwrap();
        gate.press(true);
        assert!(!gate.release(first));
        assert!(gate.process(&mut tone(0.3)));
    }

    #[test]
    fn the_talk_key_does_nothing_with_voice_activity() {
        let mut gate = MicGate::default();
        gate.press(true);
        assert!(gate.process(&mut tone(0.3)));
        let release = gate.press(false).unwrap();
        assert!(!gate.release(release));
        assert!(gate.process(&mut tone(0.3)));
    }

    #[test]
    fn a_manual_threshold_lets_loud_sound_through_and_fades_out_quiet_sound() {
        let mut gate = MicGate::default();
        gate.configure(MicSettings { sensitivity: Sensitivity::Threshold(-30.0), ..MicSettings::default() });
        let mut loud = tone(0.5);
        assert!(gate.process(&mut loud));
        assert_eq!(loudness(&loud), loudness(&tone(0.5)));
        let mut quiet = tone(0.005);
        for _ in 0..40 {
            quiet = tone(0.005);
            gate.process(&mut quiet);
        }
        assert_eq!(loudness(&quiet), 0);
    }

    #[test]
    fn automatic_sensitivity_closes_on_steady_fan_noise_that_is_not_a_voice() {
        let mut gate = MicGate::default();
        let mut seed: u32 = 7;
        let mut noise = || -> Vec<i16> {
            (0..FRAME_SIZE)
                .map(|_| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    ((seed >> 16) as i16 as i32 / 20) as i16
                })
                .collect()
        };
        let mut fan = noise();
        for _ in 0..200 {
            fan = noise();
            gate.process(&mut fan);
        }
        assert_eq!(loudness(&fan), 0);
    }

    #[test]
    fn a_voice_filter_changes_what_is_sent_unless_the_server_switches_filters_off() {
        let manual = MicSettings { sensitivity: Sensitivity::Threshold(-60.0), ..MicSettings::default() };
        let mut plain = MicGate::default();
        plain.configure(manual);
        let mut sent_plain = tone(0.3);
        plain.process(&mut sent_plain);

        let mut gate = MicGate::default();
        assert_eq!(gate.configure(MicSettings { filter: VoiceFilter::Radio, ..manual }), None);
        let mut filtered = tone(0.3);
        gate.process(&mut filtered);
        assert_ne!(filtered, sent_plain);

        assert_eq!(gate.allow_filter(false), Some(true), "the server holding filters back is news");
        assert!(gate.filter_held_back());
        let mut held = tone(0.3);
        gate.process(&mut held);
        let mut fresh = MicGate::default();
        fresh.configure(manual);
        let mut expected = tone(0.3);
        fresh.process(&mut expected);
        assert_eq!(held, expected);
        assert_eq!(gate.allow_filter(false), None);
        assert_eq!(gate.allow_filter(true), Some(false));
    }

    #[test]
    fn the_mic_test_plays_the_filter_back_without_touching_what_is_sent() {
        let mut gate = MicGate::default();
        assert_eq!(gate.test_frame(&tone(0.3)), tone(0.3));
        gate.configure(MicSettings { filter: VoiceFilter::Radio, ..MicSettings::default() });
        assert_ne!(gate.test_frame(&tone(0.3)), tone(0.3));
    }
}
