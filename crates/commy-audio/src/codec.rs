use rusty_opus::{Application, OpusDecoder, OpusEncoder};

use crate::{FRAME_SIZE, SAMPLE_RATE};

pub const VOICE_BITRATE: i32 = 32_000;
pub const LOSSY_VOICE_BITRATE: i32 = 24_000;
pub const HEAVY_LOSS_VOICE_BITRATE: i32 = 16_000;
pub const STREAM_AUDIO_BITRATE: i32 = 96_000;
pub const MIN_EXPECTED_LOSS_PCT: i32 = 5;
pub const MAX_EXPECTED_LOSS_PCT: i32 = 30;
const LOSSY_FROM_PCT: f32 = 5.0;
const HEAVY_LOSS_FROM_PCT: f32 = 15.0;
const RFC6716_MAX_PACKET_BYTES: usize = 1275;

pub struct VoiceEncoder {
    encoder: OpusEncoder,
    input: Vec<f32>,
    output: Vec<u8>,
}

impl VoiceEncoder {
    pub fn new() -> Self {
        Self::with_bitrate(VOICE_BITRATE)
    }

    pub fn with_bitrate(bitrate: i32) -> Self {
        let mut voice = Self::tuned(Application::Voip, rusty_opus::SignalType::Voice, bitrate);
        voice.encoder.use_inband_fec = true;
        voice.encoder.packet_loss_perc = MIN_EXPECTED_LOSS_PCT;
        voice
    }

    pub fn adapt_to_loss(&mut self, loss_pct: f32) {
        let loss = if loss_pct.is_finite() { loss_pct.max(0.0) } else { 0.0 };
        self.encoder.packet_loss_perc = (loss.ceil() as i32).clamp(MIN_EXPECTED_LOSS_PCT, MAX_EXPECTED_LOSS_PCT);
        self.encoder.bitrate_bps = bitrate_for_loss(loss);
    }

    pub fn bitrate(&self) -> i32 {
        self.encoder.bitrate_bps
    }

    pub fn expected_loss_pct(&self) -> i32 {
        self.encoder.packet_loss_perc
    }

    pub fn for_stream_audio() -> Self {
        Self::tuned(Application::Audio, rusty_opus::SignalType::Music, STREAM_AUDIO_BITRATE)
    }

    fn tuned(application: Application, signal: rusty_opus::SignalType, bitrate: i32) -> Self {
        let mut encoder = OpusEncoder::new(SAMPLE_RATE as i32, 1, application)
            .expect("48 kHz mono is a valid Opus configuration");
        encoder.bitrate_bps = bitrate;
        encoder.complexity = 10;
        encoder.signal_type = Some(signal);
        Self { encoder, input: vec![0.0; FRAME_SIZE], output: vec![0; RFC6716_MAX_PACKET_BYTES] }
    }

    pub fn encode(&mut self, pcm: &[i16]) -> Option<Vec<u8>> {
        if pcm.len() != FRAME_SIZE {
            return None;
        }
        for (dst, &s) in self.input.iter_mut().zip(pcm) {
            *dst = s as f32 / 32768.0;
        }
        let len = self.encoder.encode(&self.input, FRAME_SIZE, &mut self.output).ok()?;
        Some(self.output[..len].to_vec())
    }
}

pub fn bitrate_for_loss(loss_pct: f32) -> i32 {
    if loss_pct >= HEAVY_LOSS_FROM_PCT {
        HEAVY_LOSS_VOICE_BITRATE
    } else if loss_pct >= LOSSY_FROM_PCT {
        LOSSY_VOICE_BITRATE
    } else {
        VOICE_BITRATE
    }
}

impl Default for VoiceEncoder {
    fn default() -> Self {
        Self::new()
    }
}

pub fn soft_limit(x: f32) -> f32 {
    const KNEE: f32 = 0.9;
    let a = x.abs();
    if a <= KNEE {
        return x;
    }
    let over = (a - KNEE) / (1.0 - KNEE);
    let bent = KNEE + (1.0 - KNEE) * over.tanh();
    bent.copysign(x)
}

pub struct VoiceDecoder {
    decoder: OpusDecoder,
    output: Vec<f32>,
}

impl VoiceDecoder {
    pub fn new() -> Self {
        let decoder = OpusDecoder::new(SAMPLE_RATE as i32, 1).expect("48 kHz mono is a valid Opus configuration");
        Self { decoder, output: vec![0.0; FRAME_SIZE] }
    }

    pub fn decode(&mut self, packet: &[u8]) -> Vec<i16> {
        let decoded = self.decoder.decode(packet, FRAME_SIZE, &mut self.output);
        self.to_pcm(decoded)
    }

    pub fn conceal(&mut self) -> Vec<i16> {
        self.decode(&[])
    }

    pub fn recover_from_next(&mut self, next_packet: &[u8]) -> Vec<i16> {
        let decoded = self.decoder.decode_fec(next_packet, FRAME_SIZE, &mut self.output);
        self.to_pcm(decoded)
    }

    fn to_pcm(&self, decoded: Result<usize, rusty_opus::Error>) -> Vec<i16> {
        match decoded {
            Ok(n) => self.output[..n.min(FRAME_SIZE)]
                .iter()
                .map(|&s| (soft_limit(s) * 32767.0) as i16)
                .chain(std::iter::repeat(0))
                .take(FRAME_SIZE)
                .collect(),
            Err(_) => vec![0; FRAME_SIZE],
        }
    }
}

const MAX_FILLED_GAP: u32 = 3;
const RESTARTED_STREAM_GAP: u32 = 50;

#[derive(Debug, Default)]
pub struct ArrivalOrderDecoder {
    decoder: VoiceDecoder,
    last_seq: Option<u32>,
}

impl ArrivalOrderDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn decode(&mut self, seq: u32, packet: &[u8]) -> Vec<Vec<i16>> {
        let mut frames = Vec::new();
        if let Some(last) = self.last_seq {
            let ahead = seq.wrapping_sub(last);
            let behind = last.wrapping_sub(seq);
            if ahead == 0 || (ahead > u32::MAX / 2 && behind <= RESTARTED_STREAM_GAP) {
                return frames;
            }
            if (2..=MAX_FILLED_GAP + 1).contains(&ahead) {
                for _ in 2..ahead {
                    frames.push(self.decoder.conceal());
                }
                frames.push(self.decoder.recover_from_next(packet));
            }
        }
        self.last_seq = Some(seq);
        frames.push(self.decoder.decode(packet));
        frames
    }
}

impl std::fmt::Debug for VoiceDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceDecoder").finish_non_exhaustive()
    }
}

impl Default for VoiceDecoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(frame: usize) -> Vec<i16> {
        (0..FRAME_SIZE)
            .map(|i| {
                let t = (frame * FRAME_SIZE + i) as f32 / SAMPLE_RATE as f32;
                ((t * 440.0 * std::f32::consts::TAU).sin() * 8000.0) as i16
            })
            .collect()
    }

    fn energy(pcm: &[i16]) -> f64 {
        pcm.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / pcm.len() as f64
    }

    #[test]
    fn speech_frames_are_small_and_survive_the_round_trip() {
        let mut enc = VoiceEncoder::new();
        let mut dec = VoiceDecoder::new();
        let mut sizes = Vec::new();
        let mut last = Vec::new();
        for f in 0..50 {
            let packet = enc.encode(&tone(f)).expect("encodes");
            sizes.push(packet.len());
            last = dec.decode(&packet);
        }
        let avg = sizes.iter().sum::<usize>() as f64 / sizes.len() as f64;
        assert!(avg < 120.0, "average packet {avg} bytes");
        assert_eq!(last.len(), FRAME_SIZE);
        let ratio = energy(&last) / energy(&tone(49));
        assert!((0.3..3.0).contains(&ratio), "decoded energy ratio {ratio}");
    }

    #[test]
    fn stream_audio_spends_more_bits_on_fidelity_and_decodes_with_the_voice_decoder() {
        let mut voice = VoiceEncoder::new();
        let mut stream = VoiceEncoder::for_stream_audio();
        let mut dec = VoiceDecoder::new();
        let (mut voice_bytes, mut stream_bytes) = (0, 0);
        let mut last = Vec::new();
        for f in 0..50 {
            voice_bytes += voice.encode(&tone(f)).unwrap().len();
            let packet = stream.encode(&tone(f)).unwrap();
            stream_bytes += packet.len();
            last = dec.decode(&packet);
        }
        assert!(stream_bytes > voice_bytes, "stream {stream_bytes} bytes, voice {voice_bytes} bytes");
        assert!(stream_bytes < 50 * RFC6716_MAX_PACKET_BYTES);
        let ratio = energy(&last) / energy(&tone(49));
        assert!((0.5..2.0).contains(&ratio), "decoded energy ratio {ratio}");
        assert!(stream.encode(&[0; 10]).is_none());
    }

    #[test]
    fn lost_frames_are_concealed_not_dropped() {
        let mut enc = VoiceEncoder::new();
        let mut dec = VoiceDecoder::new();
        for f in 0..20 {
            dec.decode(&enc.encode(&tone(f)).unwrap());
        }
        let concealed = dec.decode(&[]);
        assert_eq!(concealed.len(), FRAME_SIZE);
        assert!(energy(&concealed) > 0.0, "concealment keeps the voice going");
    }

    fn speechlike(frame: usize) -> Vec<i16> {
        (0..FRAME_SIZE)
            .map(|i| {
                let t = (frame * FRAME_SIZE + i) as f32 / SAMPLE_RATE as f32;
                let syllable = 0.6 + 0.4 * (t * 4.0 * std::f32::consts::TAU).sin();
                let voiced = (t * 180.0 * std::f32::consts::TAU).sin() + 0.5 * (t * 360.0 * std::f32::consts::TAU).sin() + 0.25 * (t * 720.0 * std::f32::consts::TAU).sin();
                (voiced * syllable * 6000.0) as i16
            })
            .collect()
    }

    fn error_energy(a: &[i16], b: &[i16]) -> f64 {
        a.iter().zip(b).map(|(x, y)| (*x as f64 - *y as f64).powi(2)).sum::<f64>() / a.len() as f64
    }

    #[test]
    fn voice_packets_carry_a_backup_of_the_frame_before() {
        let mut enc = VoiceEncoder::new();
        let packets: Vec<Vec<u8>> = (0..120).map(|f| enc.encode(&speechlike(f)).unwrap()).collect();
        let lost = 100;
        let mut reference = VoiceDecoder::new();
        let mut recovered = VoiceDecoder::new();
        let mut concealed = VoiceDecoder::new();
        for p in &packets[..lost] {
            reference.decode(p);
            recovered.decode(p);
            concealed.decode(p);
        }
        let truth = reference.decode(&packets[lost]);
        let rebuilt = recovered.recover_from_next(&packets[lost + 1]);
        let guessed = concealed.conceal();
        assert!(energy(&rebuilt) > 0.0);
        assert!(
            error_energy(&truth, &rebuilt) < error_energy(&truth, &guessed),
            "the backup rebuilds the lost frame closer than a guess: {} vs {}",
            error_energy(&truth, &rebuilt),
            error_energy(&truth, &guessed)
        );
        assert_eq!(recovered.decode(&packets[lost + 1]).len(), FRAME_SIZE);
    }

    #[test]
    fn arrival_order_decoding_fills_short_gaps_and_drops_late_packets() {
        let mut enc = VoiceEncoder::new();
        let packets: Vec<Vec<u8>> = (0..10).map(|f| enc.encode(&speechlike(f)).unwrap()).collect();
        let mut dec = ArrivalOrderDecoder::new();
        assert_eq!(dec.decode(0, &packets[0]).len(), 1);
        assert_eq!(dec.decode(1, &packets[1]).len(), 1);
        assert_eq!(dec.decode(3, &packets[3]).len(), 2, "one lost frame is rebuilt");
        assert!(dec.decode(2, &packets[2]).is_empty(), "the late copy is dropped");
        assert!(dec.decode(3, &packets[3]).is_empty(), "so is a duplicate");
        assert_eq!(dec.decode(6, &packets[6]).len(), 3);
        assert_eq!(dec.decode(9_000, &packets[7]).len(), 1, "a long gap is not filled");
        assert_eq!(dec.decode(4, &packets[8]).len(), 1, "a restarted sender is heard");
        assert!(dec.decode(u32::MAX, &packets[9]).is_empty());
    }

    #[test]
    fn a_lossy_link_gets_more_protection_and_a_lower_bitrate() {
        let mut enc = VoiceEncoder::new();
        assert_eq!(enc.bitrate(), VOICE_BITRATE);
        assert_eq!(enc.expected_loss_pct(), MIN_EXPECTED_LOSS_PCT);
        enc.adapt_to_loss(8.2);
        assert_eq!(enc.bitrate(), LOSSY_VOICE_BITRATE);
        assert_eq!(enc.expected_loss_pct(), 9);
        enc.adapt_to_loss(60.0);
        assert_eq!(enc.bitrate(), HEAVY_LOSS_VOICE_BITRATE);
        assert_eq!(enc.expected_loss_pct(), MAX_EXPECTED_LOSS_PCT);
        enc.adapt_to_loss(f32::NAN);
        assert_eq!(enc.bitrate(), VOICE_BITRATE);
        assert_eq!(enc.expected_loss_pct(), MIN_EXPECTED_LOSS_PCT);
        enc.adapt_to_loss(-3.0);
        assert_eq!(enc.bitrate(), VOICE_BITRATE);
        let mut dec = VoiceDecoder::new();
        enc.adapt_to_loss(20.0);
        let sizes: Vec<usize> = (0..50).map(|f| enc.encode(&speechlike(f)).unwrap()).inspect(|p| assert_eq!(dec.decode(p).len(), FRAME_SIZE)).map(|p| p.len()).collect();
        let avg = sizes.iter().sum::<usize>() as f64 / sizes.len() as f64;
        assert!(avg < 60.0, "heavy loss packets average {avg} bytes");
    }

    #[test]
    fn limiter_is_transparent_below_the_knee_and_never_clips() {
        assert_eq!(soft_limit(0.5), 0.5);
        assert_eq!(soft_limit(-0.9), -0.9);
        assert!(soft_limit(1.4) < 1.0 && soft_limit(1.4) > 0.95);
        assert!(soft_limit(-3.0) >= -1.0);
        let mut prev = -2.0f32;
        for i in 0..400 {
            let y = soft_limit(-2.0 + i as f32 * 0.01);
            assert!(y >= prev);
            prev = y;
        }
    }

    #[test]
    fn bad_input_is_harmless() {
        let mut enc = VoiceEncoder::new();
        assert!(enc.encode(&[0; 100]).is_none());
        let mut dec = VoiceDecoder::new();
        assert_eq!(dec.decode(&[0xff, 0x00, 0x13]).len(), FRAME_SIZE);
    }

    fn read_wav(path: &str) -> Vec<i16> {
        let bytes = std::fs::read(path).unwrap();
        let pos = bytes.windows(4).position(|w| w == b"data").unwrap() + 8;
        bytes[pos..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
    }

    fn write_wav(path: &str, pcm: &[i16]) {
        let data_len = (pcm.len() * 2) as u32;
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
        out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for s in pcm {
            out.extend_from_slice(&s.to_le_bytes());
        }
        std::fs::write(path, out).unwrap();
    }

    fn glitches(reference: &[i16], decoded: &[i16]) -> usize {
        reference
            .chunks(FRAME_SIZE)
            .zip(decoded.chunks(FRAME_SIZE))
            .filter(|(r, d)| {
                let (er, ed) = (energy(r), energy(d));
                er > 1e4 && (ed / er < 0.05 || ed / er > 8.0)
            })
            .count()
    }

    fn aligned_snr(reference: &[i16], decoded: &[i16]) -> f64 {
        let best = (0..1000usize)
            .max_by_key(|&lag| {
                reference.iter().zip(&decoded[lag.min(decoded.len())..]).map(|(a, b)| *a as i64 * *b as i64).sum::<i64>()
            })
            .unwrap();
        let pairs = reference.iter().zip(&decoded[best..]);
        let (mut sig, mut err) = (0f64, 0f64);
        for (a, b) in pairs {
            sig += (*a as f64).powi(2);
            err += (*a as f64 - *b as f64).powi(2);
        }
        10.0 * (sig / err.max(1.0)).log10()
    }

    #[test]
    #[ignore]
    fn frame_timing() {
        let mut processor = crate::VoiceProcessor::new();
        let mut enc = VoiceEncoder::new();
        let mut dec = VoiceDecoder::new();
        let input: Vec<f32> = tone(0).iter().map(|&s| s as f32 / 32768.0).collect();
        let start = std::time::Instant::now();
        for _ in 0..500 {
            processor.process(&input).unwrap();
        }
        let nc = start.elapsed() / 500;
        let start = std::time::Instant::now();
        for f in 0..500 {
            dec.decode(&enc.encode(&tone(f)).unwrap());
        }
        println!("per 10 ms frame: noise cancel {nc:?}, opus encode+decode {:?}", start.elapsed() / 500);
    }

    #[test]
    #[ignore]
    fn speech_pipeline() {
        let path = std::env::var("SPEECH_WAV").unwrap();
        let mut speech = read_wav(&path);
        let clean = speech.clone();
        let mut seed = 1u32;
        for s in speech.iter_mut() {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise = ((seed >> 16) as i32 % 600) - 300;
            *s = (*s as i32 + noise).clamp(-32768, 32767) as i16;
        }
        speech.truncate(speech.len() / FRAME_SIZE * FRAME_SIZE);
        let run = |nc: bool, bitrate: i32| {
            let mut processor = crate::VoiceProcessor::new();
            processor.set_noise_cancellation(nc);
            let mut enc = VoiceEncoder::with_bitrate(bitrate);
            let mut dec = VoiceDecoder::new();
            let (mut processed, mut decoded) = (Vec::new(), Vec::new());
            for frame in speech.chunks(FRAME_SIZE) {
                let input: Vec<f32> = frame.iter().map(|&s| s as f32 / 32768.0).collect();
                let out = processor.process(&input).unwrap();
                let pcm: Vec<i16> = out.samples.iter().map(|s| (s * 32767.0) as i16).collect();
                decoded.extend(dec.decode(&enc.encode(&pcm).unwrap()));
                processed.extend(pcm);
            }
            (processed, decoded)
        };
        let mut seed = 7u32;
        let hiss: Vec<i16> = (0..FRAME_SIZE * 200)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (((seed >> 16) as i32 % 600) - 300) as i16
            })
            .collect();
        for nc in [false, true] {
            let mut processor = crate::VoiceProcessor::new();
            processor.set_noise_cancellation(nc);
            let (mut enc, mut dec) = (VoiceEncoder::new(), VoiceDecoder::new());
            let mut tail = Vec::new();
            let (mut vads, mut pre) = (Vec::new(), Vec::<i16>::new());
            for (i, frame) in hiss.chunks(FRAME_SIZE).enumerate() {
                let input: Vec<f32> = frame.iter().map(|&s| s as f32 / 32768.0).collect();
                let out = processor.process(&input).unwrap();
                let pcm: Vec<i16> = out.samples.iter().map(|s| (s * 32767.0) as i16).collect();
                let decoded = dec.decode(&enc.encode(&pcm).unwrap());
                if i >= 100 {
                    vads.push(out.voice_probability);
                    pre.extend(pcm.iter().copied());
                    tail.extend(decoded);
                }
            }
            println!("silence nc={nc}: rms after 1 s {:.1}, before codec {:.1}, mean vad {:.2}, max vad {:.2}", energy(&tail).sqrt(), energy(&pre).sqrt(), vads.iter().sum::<f32>() / vads.len() as f32, vads.iter().cloned().fold(0.0, f32::max));
        }
        let dir = std::path::Path::new(&path).parent().unwrap();
        for (nc, bitrate) in [(false, 32_000), (true, 32_000), (true, 48_000), (true, 64_000)] {
            let (processed, decoded) = run(nc, bitrate);
            println!("nc={nc} {bitrate}: snr {:.1} dB", aligned_snr(&processed, &decoded));
            let gaps: Vec<f64> = clean
                .chunks(FRAME_SIZE)
                .zip(decoded.chunks(FRAME_SIZE))
                .filter(|(c, _)| energy(c) < 1e3)
                .map(|(_, d)| energy(d))
                .collect();
            let floor = (gaps.iter().sum::<f64>() / gaps.len().max(1) as f64).sqrt();
            println!("  pause noise rms {floor:.1} over {} frames", gaps.len());
            let peak = decoded.iter().map(|s| s.unsigned_abs()).max().unwrap();
            let clipped = decoded.iter().filter(|s| s.unsigned_abs() >= 32760).count();
            println!(
                "nc={nc}: glitch frames {} of {}, peak {peak}, clipped {clipped}, energy in {:.0} out {:.0}",
                glitches(&processed, &decoded),
                processed.len() / FRAME_SIZE,
                energy(&processed),
                energy(&decoded)
            );
            let tag = if nc { "nc" } else { "raw" };
            write_wav(dir.join(format!("out_{tag}_processed.wav")).to_str().unwrap(), &processed);
            write_wav(dir.join(format!("out_{tag}_opus.wav")).to_str().unwrap(), &decoded);
        }
    }
}
