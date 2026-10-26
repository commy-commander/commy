use crate::{FRAME_SIZE, SAMPLE_RATE};
use sonora::config::{AdaptiveDigital, GainController2};
use sonora::{AudioProcessing, Config, StreamConfig};

const MAX_BOOST_DB: f32 = 30.0;
const STARTING_BOOST_DB: f32 = 10.0;
const BOOST_CHANGE_DB_PER_SECOND: f32 = 10.0;
const HEADROOM_DB: f32 = 2.0;

pub struct LevelControl {
    apm: AudioProcessing,
    input: Vec<f32>,
}

impl Default for LevelControl {
    fn default() -> Self {
        Self::new()
    }
}

impl LevelControl {
    pub fn new() -> Self {
        let mono = StreamConfig::new(SAMPLE_RATE, 1);
        let adaptive = AdaptiveDigital {
            max_gain_db: MAX_BOOST_DB,
            initial_gain_db: STARTING_BOOST_DB,
            max_gain_change_db_per_second: BOOST_CHANGE_DB_PER_SECOND,
            headroom_db: HEADROOM_DB,
            ..AdaptiveDigital::default()
        };
        let apm = AudioProcessing::builder()
            .config(Config {
                gain_controller2: Some(GainController2 {
                    adaptive_digital: Some(adaptive),
                    ..GainController2::default()
                }),
                pipeline: crate::full_band_pipeline(),
                ..Default::default()
            })
            .capture_config(mono)
            .render_config(mono)
            .build();
        Self { apm, input: vec![0.0; FRAME_SIZE] }
    }

    pub fn process(&mut self, samples: &mut [f32]) {
        if samples.len() != FRAME_SIZE {
            return;
        }
        for (dst, src) in self.input.iter_mut().zip(samples.iter()) {
            *dst = if src.is_finite() { src.clamp(-1.0, 1.0) } else { 0.0 };
        }
        if self.apm.process_capture_f32(&[&self.input], &mut [samples]).is_err() {
            samples.copy_from_slice(&self.input);
        }
        for s in samples.iter_mut() {
            *s = if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broken_samples_never_reach_the_output() {
        let mut level = LevelControl::new();
        let mut frame = vec![0.2f32; FRAME_SIZE];
        frame[3] = f32::NAN;
        frame[4] = f32::INFINITY;
        for _ in 0..5 {
            let mut samples = frame.clone();
            level.process(&mut samples);
            assert!(samples.iter().all(|s| s.is_finite() && (-1.0..=1.0).contains(s)));
        }
    }

    #[test]
    fn a_frame_of_the_wrong_size_is_left_alone() {
        let mut level = LevelControl::new();
        let mut samples = vec![0.01f32; 100];
        level.process(&mut samples);
        assert!(samples.iter().all(|s| *s == 0.01));
    }
}
