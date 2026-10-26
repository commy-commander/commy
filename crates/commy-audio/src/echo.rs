use crate::{FRAME_SIZE, SAMPLE_RATE};
use sonora::config::EchoCanceller as EchoSettings;
use sonora::{AudioProcessing, Config, StreamConfig};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};

const SINK_CAPACITY: usize = 64;
const I16_SCALE: f32 = 32768.0;

#[derive(Clone)]
pub struct FarEndSink {
    tx: SyncSender<Vec<i16>>,
}

impl FarEndSink {
    pub fn push(&self, pcm: &[i16]) -> bool {
        match self.tx.try_send(pcm.to_vec()) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => false,
        }
    }
}

pub struct FarEndInbox {
    tx: SyncSender<Vec<i16>>,
    rx: Receiver<Vec<i16>>,
}

impl Default for FarEndInbox {
    fn default() -> Self {
        Self::new()
    }
}

impl FarEndInbox {
    pub fn new() -> Self {
        let (tx, rx) = sync_channel(SINK_CAPACITY);
        Self { tx, rx }
    }

    pub fn sink(&self) -> FarEndSink {
        FarEndSink {
            tx: self.tx.clone(),
        }
    }

    pub fn drain_into(&self, canceller: &mut EchoCanceller) {
        while let Ok(pcm) = self.rx.try_recv() {
            canceller.push_far_end(&pcm);
        }
    }

    pub fn discard(&self) {
        while self.rx.try_recv().is_ok() {}
    }
}

pub struct EchoCanceller {
    apm: AudioProcessing,
    far: Vec<f32>,
    played: Vec<f32>,
    near: Vec<f32>,
}

impl Default for EchoCanceller {
    fn default() -> Self {
        Self::new()
    }
}

impl EchoCanceller {
    pub fn new() -> Self {
        let mono = StreamConfig::new(SAMPLE_RATE, 1);
        let apm = AudioProcessing::builder()
            .config(Config {
                echo_canceller: Some(EchoSettings::default()),
                pipeline: crate::full_band_pipeline(),
                ..Default::default()
            })
            .capture_config(mono)
            .render_config(mono)
            .build();
        Self {
            apm,
            far: vec![0.0; FRAME_SIZE],
            played: vec![0.0; FRAME_SIZE],
            near: vec![0.0; FRAME_SIZE],
        }
    }

    pub fn push_far_end(&mut self, pcm: &[i16]) {
        if pcm.len() != FRAME_SIZE {
            return;
        }
        for (dst, src) in self.far.iter_mut().zip(pcm) {
            *dst = f32::from(*src) / I16_SCALE;
        }
        let _ = self
            .apm
            .process_render_f32(&[&self.far], &mut [&mut self.played]);
    }

    pub fn process(&mut self, near: &[f32], out: &mut [f32]) {
        for (dst, src) in self.near.iter_mut().zip(near) {
            *dst = playable(*src);
        }
        if self.apm.process_capture_f32(&[&self.near], &mut [out]).is_err() {
            out.copy_from_slice(&self.near);
        }
        for s in out.iter_mut() {
            *s = playable(*s);
        }
    }
}

fn playable(sample: f32) -> f32 {
    if sample.is_finite() {
        sample.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
        }
    }

    fn voice(n: usize, pitch: f64, syllables_per_second: f64) -> f32 {
        let t = n as f64 / f64::from(SAMPLE_RATE);
        let pitch = pitch + 25.0 * (2.0 * std::f64::consts::PI * 2.3 * t).sin();
        let phase = 2.0 * std::f64::consts::PI * pitch * t;
        let envelope = (0.5 + 0.5 * (2.0 * std::f64::consts::PI * syllables_per_second * t).sin()).powi(2);
        let harmonics: f64 = (1..=8).map(|h| (h as f64 * phase).sin() / h as f64).sum();
        (0.2 * envelope * harmonics) as f32
    }

    fn their_voice(n: usize) -> f32 {
        voice(n, 120.0, 0.23)
    }

    fn my_voice(n: usize) -> f32 {
        voice(n, 210.0, 0.11)
    }

    struct Room {
        played: Vec<f32>,
        delay: f64,
        clock_drift: f64,
        loudness: f32,
        floor: Lcg,
    }

    impl Room {
        fn speakers() -> Self {
            Self {
                played: Vec::new(),
                delay: 4800.0,
                clock_drift: 0.0,
                loudness: 0.5,
                floor: Lcg(99),
            }
        }

        fn echo_at(&self, n: usize) -> f32 {
            let pos = n as f64 * (1.0 + self.clock_drift) - self.delay;
            if pos < 0.0 {
                return 0.0;
            }
            let at = |k: usize| self.played.get(k).copied().unwrap_or(0.0);
            let k = pos as usize;
            let frac = (pos - k as f64) as f32;
            let direct = at(k) + (at(k + 1) - at(k)) * frac;
            self.loudness * (direct + 0.4 * at(k.saturating_sub(300)))
        }

        fn mic(&mut self, frame: usize, me: impl Fn(usize) -> f32) -> Vec<f32> {
            (0..FRAME_SIZE)
                .map(|i| {
                    let n = frame * FRAME_SIZE + i;
                    self.echo_at(n) + 0.001 * self.floor.next() + me(n)
                })
                .collect()
        }

        fn play(&mut self, far: &[i16]) {
            self.played.extend(far.iter().map(|s| f32::from(*s) / I16_SCALE));
        }
    }

    fn power(samples: &[f32]) -> f64 {
        samples.iter().map(|s| f64::from(s * s)).sum()
    }

    fn db(ratio: f64) -> f64 {
        10.0 * ratio.max(1e-12).log10()
    }

    fn far_frame(frame: usize) -> Vec<i16> {
        (0..FRAME_SIZE)
            .map(|i| (their_voice(frame * FRAME_SIZE + i) * 32767.0) as i16)
            .collect()
    }

    fn removed_db(
        room: &mut Room,
        aec: &mut EchoCanceller,
        frames: std::ops::Range<usize>,
        me: impl Fn(usize) -> f32 + Copy,
        burst: usize,
    ) -> f64 {
        let mut echo = 0.0;
        let mut left = 0.0;
        let mut queued = Vec::new();
        for frame in frames {
            let far = far_frame(frame);
            room.play(&far);
            queued.push(far);
            if queued.len() >= burst {
                for far in queued.drain(..) {
                    aec.push_far_end(&far);
                }
            }
            let mic = room.mic(frame, me);
            let mut out = vec![0.0; FRAME_SIZE];
            aec.process(&mic, &mut out);
            echo += power(&mic);
            left += power(&out);
        }
        db(echo / left)
    }

    fn silent(_: usize) -> f32 {
        0.0
    }

    #[test]
    fn echo_from_speakers_is_removed() {
        let mut room = Room::speakers();
        let mut aec = EchoCanceller::new();
        removed_db(&mut room, &mut aec, 0..500, silent, 1);
        let removed = removed_db(&mut room, &mut aec, 500..1500, silent, 1);
        assert!(removed > 30.0, "removed {removed:.1} dB");
    }

    #[test]
    fn echo_is_still_removed_when_the_speaker_and_mic_clocks_drift() {
        let mut room = Room::speakers();
        room.clock_drift = 0.0003;
        let mut aec = EchoCanceller::new();
        removed_db(&mut room, &mut aec, 0..1000, silent, 2);
        let removed = removed_db(&mut room, &mut aec, 1000..6000, silent, 2);
        assert!(removed > 25.0, "removed {removed:.1} dB");
    }

    #[test]
    fn echo_is_removed_again_soon_after_the_delay_changes() {
        let mut room = Room::speakers();
        let mut aec = EchoCanceller::new();
        removed_db(&mut room, &mut aec, 0..1000, silent, 3);
        room.delay = 7200.0;
        removed_db(&mut room, &mut aec, 1000..1300, silent, 3);
        let removed = removed_db(&mut room, &mut aec, 1300..2300, silent, 3);
        assert!(removed > 30.0, "removed {removed:.1} dB");
    }

    #[test]
    fn a_muted_mic_sends_nothing_of_the_other_person_back() {
        let mut room = Room::speakers();
        let mut aec = EchoCanceller::new();
        removed_db(&mut room, &mut aec, 0..1000, my_voice, 1);
        let mut sent = 0.0;
        for frame in 1000..1100 {
            let far = far_frame(frame);
            room.play(&far);
            aec.push_far_end(&far);
            let mut out = vec![0.0; FRAME_SIZE];
            aec.process(&[0.0; FRAME_SIZE], &mut out);
            sent += power(&out);
        }
        let played = power(&room.played[1000 * FRAME_SIZE..]);
        let below = db(played / sent);
        assert!(below > 50.0, "a muted mic sends something only {below:.1} dB below the other voice");
    }

    #[test]
    fn my_voice_gets_through_while_the_other_person_talks() {
        let mut room = Room::speakers();
        let mut aec = EchoCanceller::new();
        removed_db(&mut room, &mut aec, 0..500, silent, 1);
        let mut mine = 0.0;
        let mut sent = 0.0;
        for frame in 500..1500 {
            let far = far_frame(frame);
            room.play(&far);
            aec.push_far_end(&far);
            let mic = room.mic(frame, my_voice);
            let mut out = vec![0.0; FRAME_SIZE];
            aec.process(&mic, &mut out);
            mine += (0..FRAME_SIZE)
                .map(|i| f64::from(my_voice(frame * FRAME_SIZE + i).powi(2)))
                .sum::<f64>();
            sent += power(&out);
        }
        let kept = db(sent / mine);
        assert!(kept > -6.0, "my voice lost {:.1} dB", -kept);
    }

    #[test]
    fn a_headset_with_no_echo_is_left_alone() {
        let mut room = Room::speakers();
        room.loudness = 0.0;
        let mut aec = EchoCanceller::new();
        let mut mine = 0.0;
        let mut sent = 0.0;
        for frame in 0..1000 {
            aec.push_far_end(&far_frame(frame));
            let mic = room.mic(frame, my_voice);
            let mut out = vec![0.0; FRAME_SIZE];
            aec.process(&mic, &mut out);
            if frame >= 200 {
                mine += power(&mic);
                sent += power(&out);
            }
        }
        let kept = db(sent / mine);
        assert!(kept > -3.0, "my voice lost {:.1} dB", -kept);
    }

    #[test]
    fn the_top_of_the_voice_band_is_kept() {
        let mut aec = EchoCanceller::new();
        let air = |n: usize| 0.05 * (std::f64::consts::TAU * 18_000.0 * n as f64 / f64::from(SAMPLE_RATE)).sin() as f32;
        let mut kept = 0.0;
        let mut sent = 0.0;
        for frame in 0..600 {
            let mic: Vec<f32> = (0..FRAME_SIZE).map(|i| air(frame * FRAME_SIZE + i) + my_voice(frame * FRAME_SIZE + i)).collect();
            let mut out = vec![0.0; FRAME_SIZE];
            aec.process(&mic, &mut out);
            if frame >= 300 {
                let at_18k = |s: &[f32]| {
                    let (re, im) = s.iter().enumerate().fold((0.0f64, 0.0f64), |(re, im), (i, v)| {
                        let w = std::f64::consts::TAU * 18_000.0 * i as f64 / f64::from(SAMPLE_RATE);
                        (re + f64::from(*v) * w.cos(), im + f64::from(*v) * w.sin())
                    });
                    re * re + im * im
                };
                kept += at_18k(&mic);
                sent += at_18k(&out);
            }
        }
        let lost = -db(sent / kept);
        assert!(lost < 3.0, "lost {lost:.1} dB at 18 kHz");
    }

    #[test]
    fn broken_samples_never_reach_the_output() {
        let mut aec = EchoCanceller::new();
        let mut near = vec![0.1f32; FRAME_SIZE];
        near[7] = f32::NAN;
        near[8] = f32::INFINITY;
        let mut out = vec![0.0; FRAME_SIZE];
        for _ in 0..5 {
            aec.push_far_end(&[1000; FRAME_SIZE]);
            aec.process(&near, &mut out);
            assert!(out.iter().all(|s| s.is_finite() && (-1.0..=1.0).contains(s)));
        }
    }

    #[test]
    fn the_sink_drops_frames_instead_of_growing_without_bound() {
        let inbox = FarEndInbox::new();
        let sink = inbox.sink();
        let frame = vec![0i16; FRAME_SIZE];
        for _ in 0..SINK_CAPACITY {
            assert!(sink.push(&frame));
        }
        assert!(!sink.push(&frame));
        inbox.discard();
        assert!(sink.push(&frame));
    }
}
