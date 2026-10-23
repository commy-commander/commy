use crate::frame_has_signal;

pub const DEFAULT_HOLD_FRAMES: u32 = 25;

pub const DEFAULT_ATTACK_FRAMES: u32 = 2;

#[derive(Debug, Clone)]
pub struct SpeakingDetector {
    speaking: bool,
    voiced_run: u32,
    silent_run: u32,
    attack: u32,
    hold: u32,
}

impl Default for SpeakingDetector {
    fn default() -> Self {
        Self::new(DEFAULT_ATTACK_FRAMES, DEFAULT_HOLD_FRAMES)
    }
}

impl SpeakingDetector {
    pub fn new(attack: u32, hold: u32) -> Self {
        Self {
            speaking: false,
            voiced_run: 0,
            silent_run: 0,
            attack: attack.max(1),
            hold: hold.max(1),
        }
    }

    pub fn is_speaking(&self) -> bool {
        self.speaking
    }

    pub fn push(&mut self, pcm: &[i16]) -> Option<bool> {
        self.push_voiced(frame_has_signal(pcm))
    }

    pub fn push_voiced(&mut self, voiced: bool) -> Option<bool> {
        if voiced {
            self.voiced_run = self.voiced_run.saturating_add(1);
            self.silent_run = 0;
            if !self.speaking && self.voiced_run >= self.attack {
                self.speaking = true;
                return Some(true);
            }
        } else {
            self.silent_run = self.silent_run.saturating_add(1);
            self.voiced_run = self.voiced_run.saturating_sub(1);
            if self.speaking && self.silent_run >= self.hold {
                self.speaking = false;
                return Some(false);
            }
        }
        None
    }

    pub fn reset(&mut self) -> bool {
        let was = self.speaking;
        *self = Self::new(self.attack, self.hold);
        was
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loud() -> Vec<i16> {
        vec![4000; 480]
    }

    fn quiet() -> Vec<i16> {
        vec![0; 480]
    }

    #[test]
    fn starts_silent() {
        assert!(!SpeakingDetector::default().is_speaking());
    }

    #[test]
    fn turns_on_after_attack_frames() {
        let mut d = SpeakingDetector::new(2, 5);
        assert_eq!(d.push(&loud()), None);
        assert_eq!(d.push(&loud()), Some(true));
        assert_eq!(d.push(&loud()), None, "no repeat events while talking");
        assert!(d.is_speaking());
    }

    #[test]
    fn single_click_does_not_trigger() {
        let mut d = SpeakingDetector::new(2, 5);
        assert_eq!(d.push(&loud()), None);
        assert_eq!(d.push(&quiet()), None);
        assert_eq!(d.push(&loud()), None);
        assert!(!d.is_speaking());
    }

    #[test]
    fn broken_up_speech_still_triggers() {
        let mut d = SpeakingDetector::new(2, 5);
        assert_eq!(d.push(&loud()), None);
        assert_eq!(d.push(&quiet()), None);
        assert_eq!(d.push(&loud()), None);
        assert_eq!(d.push(&loud()), Some(true));
    }

    #[test]
    fn holds_through_short_pauses() {
        let mut d = SpeakingDetector::new(1, 5);
        d.push(&loud());
        for _ in 0..4 {
            assert_eq!(d.push(&quiet()), None);
        }
        assert_eq!(d.push(&loud()), None, "pause shorter than hold keeps it on");
        assert!(d.is_speaking());
    }

    #[test]
    fn turns_off_after_hold_frames_of_silence() {
        let mut d = SpeakingDetector::new(1, 3);
        d.push(&loud());
        assert_eq!(d.push(&quiet()), None);
        assert_eq!(d.push(&quiet()), None);
        assert_eq!(d.push(&quiet()), Some(false));
        assert_eq!(d.push(&quiet()), None);
    }

    #[test]
    fn reset_reports_previous_state() {
        let mut d = SpeakingDetector::new(1, 3);
        assert!(!d.reset());
        d.push(&loud());
        assert!(d.reset());
        assert!(!d.is_speaking());
    }

    #[test]
    fn zero_thresholds_are_clamped() {
        let mut d = SpeakingDetector::new(0, 0);
        assert_eq!(d.push_voiced(true), Some(true));
        assert_eq!(d.push_voiced(false), Some(false));
    }
}
