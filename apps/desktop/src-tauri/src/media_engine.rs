//! Voice and screen-share orchestration.
//!
//! Voice is a full mesh: every pair of members in a voice channel has
//! one direct QUIC media connection. The peer with the smaller node id
//! dials, the other accepts, so exactly one connection exists per pair.
//! Screen share is one-to-many: viewers subscribe to the sharer.

use commy_core::{ChannelId, ServerId, UserId};
use commy_media::{Mixer, OutputHandle, StreamGuards};
use commy_net::media::{MediaHello, MediaPacket};
use commy_net::node::MediaChannel;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

/// Deterministic rule: the lexicographically smaller node id dials.
pub fn should_dial(our_node: &str, their_node: &str) -> bool {
    our_node < their_node
}

/// Stable mixer key for a user.
pub fn mixer_key(user: UserId) -> u64 {
    let bytes = user.0.as_bytes();
    u64::from_le_bytes(bytes[..8].try_into().unwrap())
}

pub struct VoiceSession {
    pub server: ServerId,
    pub channel: ChannelId,
    pub mixer: Arc<Mutex<Mixer>>,
    pub deafened: Arc<AtomicBool>,
    /// Sequenced microphone frames, fanned out to every peer task.
    pub mic_broadcast: broadcast::Sender<(u32, Arc<Vec<u8>>)>,
    /// Unix seconds of the last transmitted mic frame, for AFK moves.
    pub last_activity: Arc<std::sync::atomic::AtomicI64>,
    peers: HashMap<UserId, JoinHandle<()>>,
    /// Each peer's connection and measured voice jitter, for the stats view.
    links: HashMap<UserId, (commy_net::node::Connection, Arc<Mutex<JitterMeter>>)>,
    /// Keeps cpal streams alive; dropped on leave.
    devices: StreamGuards,
    on_speaking: SpeakingSink,
    /// Sender the microphone feeds, kept to restart capture on a new device.
    mic_tx: mpsc::UnboundedSender<Vec<i16>>,
    _seq_task: JoinHandle<()>,
}

/// One peer link's numbers for the connection details view.
#[derive(Debug, Clone)]
pub struct LinkStats {
    pub user: UserId,
    pub rtt_ms: f32,
    pub loss_pct: f32,
    pub jitter_ms: f32,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
}

/// Interarrival jitter of voice frames (RFC 3550 style). Frames carry a
/// sequence number and are 10 ms apart, so any spread beyond that is jitter.
#[derive(Debug, Default)]
pub struct JitterMeter {
    last: Option<(u32, std::time::Instant)>,
    jitter_ms: f32,
}

impl JitterMeter {
    const FRAME_MS: f32 = 10.0;

    pub fn arrived(&mut self, seq: u32, at: std::time::Instant) {
        if let Some((last_seq, last_at)) = self.last {
            if seq > last_seq {
                let expected = (seq - last_seq) as f32 * Self::FRAME_MS;
                let actual = at.duration_since(last_at).as_secs_f32() * 1000.0;
                let d = (actual - expected).abs();
                self.jitter_ms += (d - self.jitter_ms) / 16.0;
            }
        }
        if self.last.map_or(true, |(s, _)| seq > s) {
            self.last = Some((seq, at));
        }
    }

    pub fn jitter_ms(&self) -> f32 {
        self.jitter_ms
    }
}

/// Receives (user, speaking) transitions for the UI.
pub type SpeakingSink = Arc<dyn Fn(UserId, bool) + Send + Sync>;

/// How often speaking state is re-checked when no audio arrives, since
/// a muted mic or a gated peer simply stops sending frames.
const SPEAKING_TICK: std::time::Duration = std::time::Duration::from_millis(50);

/// Silent 10 ms frames credited per tick without audio.
const FRAMES_PER_TICK: u32 = 5;

/// Feeds one tick of missing audio into a detector as silence.
fn credit_silence(detector: &mut commy_media::SpeakingDetector) -> Option<bool> {
    let mut change = None;
    for _ in 0..FRAMES_PER_TICK {
        if let Some(c) = detector.push_voiced(false) {
            change = Some(c);
        }
    }
    change
}

impl VoiceSession {
    /// Builds the local half of a voice session: mic capture, playback,
    /// and the sequencer that stamps packet numbers on mic frames.
    pub fn start(
        server: ServerId,
        channel: ChannelId,
        devices: StreamGuards,
        mic_rx: mpsc::UnboundedReceiver<Vec<i16>>,
        mic_tx: mpsc::UnboundedSender<Vec<i16>>,
        deafened: bool,
        me: UserId,
        on_speaking: SpeakingSink,
    ) -> Self {
        let (mic_broadcast, _) = broadcast::channel(64);
        let now = || chrono::Utc::now().timestamp();
        let last_activity = Arc::new(std::sync::atomic::AtomicI64::new(now()));
        let seq_task = {
            let mic_broadcast = mic_broadcast.clone();
            let last_activity = last_activity.clone();
            let on_speaking = on_speaking.clone();
            tokio::spawn(async move {
                let mut mic_rx = mic_rx;
                let mut seq: u32 = 0;
                // Encoded once here, then fanned out to every peer.
                let mut encoder = commy_audio::codec::VoiceEncoder::new();
                let mut detector = commy_media::SpeakingDetector::default();
                let mut tick = tokio::time::interval(SPEAKING_TICK);
                let mut heard_since_tick = false;
                loop {
                    tokio::select! {
                        pcm = mic_rx.recv() => {
                            let Some(pcm) = pcm else { break };
                            heard_since_tick = true;
                            // Only audible frames count as activity; otherwise an
                            // open mic in a silent room never goes AFK.
                            if commy_media::frame_has_signal(&pcm) {
                                last_activity.store(chrono::Utc::now().timestamp(), Ordering::Relaxed);
                            }
                            if let Some(speaking) = detector.push(&pcm) {
                                on_speaking(me, speaking);
                            }
                            if let Some(packet) = encoder.encode(&pcm) {
                                let _ = mic_broadcast.send((seq, Arc::new(packet)));
                            }
                            seq = seq.wrapping_add(1);
                        }
                        _ = tick.tick() => {
                            if !std::mem::take(&mut heard_since_tick) {
                                if let Some(speaking) = credit_silence(&mut detector) {
                                    on_speaking(me, speaking);
                                }
                            }
                        }
                    }
                }
                if detector.reset() {
                    on_speaking(me, false);
                }
            })
        };
        Self {
            server,
            channel,
            mixer: Arc::new(Mutex::new(Mixer::new())),
            deafened: Arc::new(AtomicBool::new(deafened)),
            mic_broadcast,
            last_activity,
            peers: HashMap::new(),
            links: HashMap::new(),
            devices,
            on_speaking,
            mic_tx,
            _seq_task: seq_task,
        }
    }

    /// Playback is created after the session because it needs the
    /// session's mixer.
    pub fn set_speaker(&mut self, speaker: Option<OutputHandle>) {
        self.devices.speaker = speaker;
    }

    /// Moves capture and playback to other devices mid-call. Old streams
    /// are dropped first so exclusive-mode devices are free to reopen.
    pub fn switch_devices(
        &mut self,
        processor: Arc<Mutex<commy_audio::VoiceProcessor>>,
        input: Option<String>,
        output: Option<String>,
    ) {
        self.devices.mic = None;
        self.devices.mic = match commy_media::start_microphone(processor, self.mic_tx.clone(), input) {
            Ok(mic) => Some(mic),
            Err(e) => {
                tracing::warn!("microphone switch failed: {e}");
                None
            }
        };
        self.devices.speaker = None;
        self.devices.speaker = match commy_media::start_playback(self.mixer.clone(), output) {
            Ok(speaker) => Some(speaker),
            Err(e) => {
                tracing::warn!("speaker switch failed: {e}");
                None
            }
        };
    }

    pub fn has_peer(&self, user: UserId) -> bool {
        self.peers.contains_key(&user)
    }

    /// Wires one peer's media channel into the session: their packets
    /// feed the mixer, our mic frames feed their connection.
    pub fn attach_peer(&mut self, user: UserId, media: MediaChannel) {
        let mixer = self.mixer.clone();
        let deafened = self.deafened.clone();
        let mut mic_rx = self.mic_broadcast.subscribe();
        let key = mixer_key(user);
        let on_speaking = self.on_speaking.clone();
        let meter = Arc::new(Mutex::new(JitterMeter::default()));
        self.links.insert(user, (media.connection.clone(), meter.clone()));

        let task = tokio::spawn(async move {
            let MediaChannel {
                mut incoming,
                outgoing,
                ..
            } = media;
            let mut decoder = commy_audio::codec::VoiceDecoder::new();
            let mut last_seq: Option<u32> = None;
            let mut detector = commy_media::SpeakingDetector::default();
            let mut tick = tokio::time::interval(SPEAKING_TICK);
            let mut heard_since_tick = false;
            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        if !std::mem::take(&mut heard_since_tick) {
                            if let Some(speaking) = credit_silence(&mut detector) {
                                on_speaking(user, speaking);
                            }
                        }
                    }
                    packet = incoming.recv() => match packet {
                        Some(MediaPacket::Opus { seq, data }) => {
                            heard_since_tick = true;
                            meter.lock().unwrap().arrived(seq, std::time::Instant::now());
                            // Conceal a short gap so the decoder state stays smooth;
                            // the jitter buffer handles longer silences.
                            if let Some(last) = last_seq {
                                let gap = seq.wrapping_sub(last);
                                if (2..=3).contains(&gap) {
                                    for missing in 1..gap {
                                        let pcm = decoder.decode(&[]);
                                        if !deafened.load(Ordering::Relaxed) {
                                            mixer.lock().unwrap().push(key, last.wrapping_add(missing), pcm);
                                        }
                                    }
                                }
                            }
                            last_seq = Some(seq);
                            let pcm = decoder.decode(&data);
                            if let Some(speaking) = detector.push(&pcm) {
                                on_speaking(user, speaking);
                            }
                            if !deafened.load(Ordering::Relaxed) {
                                mixer.lock().unwrap().push(key, seq, pcm);
                            }
                        }
                        Some(MediaPacket::Voice { seq, pcm }) => {
                            heard_since_tick = true;
                            meter.lock().unwrap().arrived(seq, std::time::Instant::now());
                            if let Some(speaking) = detector.push(&pcm) {
                                on_speaking(user, speaking);
                            }
                            if !deafened.load(Ordering::Relaxed) {
                                mixer.lock().unwrap().push(key, seq, pcm);
                            }
                        }
                        Some(_) => {} // video on a voice stream: ignore
                        None => break,
                    },
                    frame = mic_rx.recv() => match frame {
                        Ok((seq, data)) => {
                            if outgoing.send(MediaPacket::Opus { seq, data: data.as_ref().clone() }).is_err() {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    },
                }
            }
            mixer.lock().unwrap().remove_peer(key);
            if detector.reset() {
                on_speaking(user, false);
            }
        });

        if let Some(old) = self.peers.insert(user, task) {
            old.abort();
        }
    }

    /// Per peer: round trip, network loss, jitter and bytes moved.
    pub fn link_stats(&self) -> Vec<LinkStats> {
        self.links
            .iter()
            .map(|(user, (conn, meter))| {
                let stats = conn.stats();
                let sent = stats.path.sent_packets.max(1);
                LinkStats {
                    user: *user,
                    rtt_ms: conn.rtt().as_secs_f32() * 1000.0,
                    loss_pct: stats.path.lost_packets as f32 * 100.0 / sent as f32,
                    jitter_ms: meter.lock().unwrap().jitter_ms(),
                    tx_bytes: stats.udp_tx.bytes,
                    rx_bytes: stats.udp_rx.bytes,
                }
            })
            .collect()
    }

    pub fn detach_peer(&mut self, user: UserId) {
        self.links.remove(&user);
        if let Some(task) = self.peers.remove(&user) {
            task.abort();
        }
        self.mixer.lock().unwrap().remove_peer(mixer_key(user));
        // An aborted task cannot report its own reset.
        (self.on_speaking)(user, false);
    }
}

impl Drop for VoiceSession {
    fn drop(&mut self) {
        for (_, task) in self.peers.drain() {
            task.abort();
        }
        self._seq_task.abort();
    }
}

/// Our outgoing screen share: capture task plus subscriber fan-out.
pub struct ScreenShareSession {
    pub server: ServerId,
    pub channel: ChannelId,
    pub subscribers: Arc<Mutex<Vec<mpsc::UnboundedSender<MediaPacket>>>>,
    /// Most recent captured JPEG, for the sharer's own preview.
    latest: Arc<Mutex<Option<commy_media::screen::ScreenFrame>>>,
    /// Tells the capture thread to stop; encoders end when it does.
    stop: Arc<AtomicBool>,
    /// Frames encoded so far, for a live fps readout.
    encoded: Arc<std::sync::atomic::AtomicU64>,
}

/// Encoder threads per share. Two keep up with 60 fps at 720p.
const ENCODERS: usize = 2;
const JPEG_QUALITY: u8 = 60;

impl ScreenShareSession {
    pub fn start(
        server: ServerId,
        channel: ChannelId,
        fps: u32,
        max_width: u32,
        source: Option<(commy_media::screen::SourceKind, u32)>,
    ) -> Self {
        let subscribers: Arc<Mutex<Vec<mpsc::UnboundedSender<MediaPacket>>>> =
            Arc::new(Mutex::new(Vec::new()));
        let latest = Arc::new(Mutex::new(None));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let encoded = Arc::new(std::sync::atomic::AtomicU64::new(0));
        // Capture and encoding run on their own threads so a slow encode
        // never delays the next grab. Frames that arrive while every
        // encoder is busy are dropped rather than queued.
        let (raw_tx, raw_rx) = std::sync::mpsc::sync_channel::<(u64, image::RgbaImage)>(ENCODERS);
        let raw_rx = Arc::new(Mutex::new(raw_rx));
        {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let interval = std::time::Duration::from_micros(1_000_000 / fps.max(1) as u64);
                let mut capturer = None;
                let mut seq: u64 = 0;
                while !stop.load(Ordering::Relaxed) {
                    let started = std::time::Instant::now();
                    if capturer.is_none() {
                        match commy_media::screen::Capturer::open(source) {
                            Ok(c) => capturer = Some(c),
                            Err(e) => {
                                tracing::warn!("screen capture unavailable: {e}");
                                std::thread::sleep(std::time::Duration::from_secs(1));
                                continue;
                            }
                        }
                    }
                    match capturer.as_ref().map(|c| c.grab()) {
                        Some(Ok(image)) => {
                            match raw_tx.try_send((seq, image)) {
                                Ok(()) | Err(std::sync::mpsc::TrySendError::Full(_)) => {}
                                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => break,
                            }
                            seq += 1;
                        }
                        Some(Err(e)) => {
                            // The window may have closed or the monitor changed; reopen.
                            tracing::warn!("screen capture failed: {e}");
                            capturer = None;
                            std::thread::sleep(std::time::Duration::from_millis(500));
                        }
                        None => {}
                    }
                    if let Some(rest) = interval.checked_sub(started.elapsed()) {
                        std::thread::sleep(rest);
                    }
                }
            });
        }
        for _ in 0..ENCODERS {
            let raw_rx = raw_rx.clone();
            let encoded = encoded.clone();
            let subscribers = subscribers.clone();
            let latest = latest.clone();
            std::thread::spawn(move || loop {
                let next = raw_rx.lock().unwrap().recv();
                let Ok((seq, image)) = next else { break };
                let frame = match commy_media::screen::encode_frame(image, max_width, JPEG_QUALITY) {
                    Ok(frame) => frame,
                    Err(e) => {
                        tracing::warn!("screen encode failed: {e}");
                        continue;
                    }
                };
                encoded.fetch_add(1, Ordering::Relaxed);
                *latest.lock().unwrap() = Some(frame.clone());
                let packet = MediaPacket::Video { seq, width: frame.width, height: frame.height, jpeg: frame.jpeg };
                subscribers.lock().unwrap().retain(|tx| tx.send(packet.clone()).is_ok());
            });
        }
        Self {
            server,
            channel,
            subscribers,
            latest,
            stop,
            encoded,
        }
    }

    pub fn add_subscriber(&self, tx: mpsc::UnboundedSender<MediaPacket>) {
        self.subscribers.lock().unwrap().push(tx);
    }

    pub fn frames_encoded(&self) -> u64 {
        self.encoded.load(Ordering::Relaxed)
    }

    pub fn latest_frame(&self) -> Option<commy_media::screen::ScreenFrame> {
        self.latest.lock().unwrap().clone()
    }

    pub fn viewer_count(&self) -> usize {
        let mut subs = self.subscribers.lock().unwrap();
        subs.retain(|tx| !tx.is_closed());
        subs.len()
    }
}

impl Drop for ScreenShareSession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// A screen we are watching: pump remote frames to the webview.
pub struct ScreenWatch {
    pub task: JoinHandle<()>,
}

impl ScreenWatch {
    pub fn start(
        user: UserId,
        mut media: MediaChannel,
        emit: impl Fn(UserId, u64, u32, u32, Vec<u8>) + Send + 'static,
    ) -> Self {
        let task = tokio::spawn(async move {
            let mut last_seq: Option<u64> = None;
            while let Some(mut packet) = media.incoming.recv().await {
                // Show only the newest frame; anything that queued up
                // while the webview was busy is already stale.
                while let Ok(newer) = media.incoming.try_recv() {
                    packet = newer;
                }
                if let MediaPacket::Video { seq, width, height, jpeg } = packet {
                    // Encoders can finish out of order; never step backwards.
                    if last_seq.is_some_and(|last| seq <= last) {
                        continue;
                    }
                    last_seq = Some(seq);
                    emit(user, seq, width, height, jpeg);
                }
            }
        });
        Self { task }
    }
}

impl Drop for ScreenWatch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Our outgoing camera. Frames are captured and JPEG-encoded by the
/// webview (getUserMedia), then fanned out to every subscribed viewer
/// over the same MJPEG packets screen share uses.
pub struct CameraSession {
    pub server: ServerId,
    pub channel: ChannelId,
    subscribers: Vec<mpsc::UnboundedSender<MediaPacket>>,
    seq: u64,
}

impl CameraSession {
    pub fn new(server: ServerId, channel: ChannelId) -> Self {
        Self { server, channel, subscribers: Vec::new(), seq: 0 }
    }

    pub fn add_subscriber(&mut self, tx: mpsc::UnboundedSender<MediaPacket>) {
        self.subscribers.push(tx);
    }

    /// Sends one frame to every viewer, dropping viewers that left.
    /// Returns how many are still watching.
    pub fn push(&mut self, width: u32, height: u32, jpeg: Vec<u8>) -> usize {
        let packet = MediaPacket::Video { seq: self.seq, width, height, jpeg };
        self.seq += 1;
        self.subscribers.retain(|tx| tx.send(packet.clone()).is_ok());
        self.subscribers.len()
    }
}

/// Convenience: the hello we send when dialing a voice peer.
pub fn voice_hello(user: UserId, channel: ChannelId) -> MediaHello {
    MediaHello::Voice { user, channel }
}

/// Mic test: loops your own microphone through the entire voice
/// pipeline (capture, noise cancellation, sequencing, jitter buffer,
/// mixer, playback) so voice can be exercised on a single machine.
pub struct MicTest {
    _devices: StreamGuards,
    task: JoinHandle<()>,
}

impl MicTest {
    pub fn start(
        processor: std::sync::Arc<Mutex<commy_audio::VoiceProcessor>>,
        input: Option<String>,
        output: Option<String>,
    ) -> anyhow::Result<Self> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<i16>>();
        let mic = commy_media::start_microphone(processor, tx, input)?;
        let mixer = Arc::new(Mutex::new(Mixer::new()));
        let speaker = commy_media::start_playback(mixer.clone(), output)?;
        let task = tokio::spawn(async move {
            // Through the same codec as calls, so the test sounds like you do to others.
            let mut encoder = commy_audio::codec::VoiceEncoder::new();
            let mut decoder = commy_audio::codec::VoiceDecoder::new();
            let mut seq: u32 = 0;
            while let Some(pcm) = rx.recv().await {
                let pcm = match encoder.encode(&pcm) {
                    Some(packet) => decoder.decode(&packet),
                    None => pcm,
                };
                mixer.lock().unwrap().push(0, seq, pcm);
                seq = seq.wrapping_add(1);
            }
        });
        Ok(Self {
            _devices: StreamGuards {
                mic: Some(mic),
                speaker: Some(speaker),
            },
            task,
        })
    }
}

impl Drop for MicTest {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dial_rule_is_asymmetric_and_total() {
        let a = "0a".to_string();
        let b = "ff".to_string();
        assert!(should_dial(&a, &b));
        assert!(!should_dial(&b, &a));
        assert!(!should_dial(&a, &a));
    }

    #[test]
    fn jitter_is_zero_for_steady_frames_and_grows_with_spread() {
        let start = std::time::Instant::now();
        let mut steady = JitterMeter::default();
        for i in 0..50u32 {
            steady.arrived(i, start + std::time::Duration::from_millis(i as u64 * 10));
        }
        assert!(steady.jitter_ms() < 0.01);
        let mut bumpy = JitterMeter::default();
        for i in 0..50u32 {
            let wobble = if i % 2 == 0 { 0 } else { 8 };
            bumpy.arrived(i, start + std::time::Duration::from_millis(i as u64 * 10 + wobble));
        }
        assert!(bumpy.jitter_ms() > 3.0, "{}", bumpy.jitter_ms());
    }

    #[test]
    fn mixer_key_is_stable_and_distinct() {
        let u1 = UserId::new();
        let u2 = UserId::new();
        assert_eq!(mixer_key(u1), mixer_key(u1));
        assert_ne!(mixer_key(u1), mixer_key(u2));
    }
}

#[cfg(test)]
mod camera_tests {
    use super::*;

    #[test]
    fn frames_reach_viewers_in_order_and_dead_viewers_drop() {
        let mut cam = CameraSession::new(ServerId::new(), ChannelId::new());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (gone_tx, gone_rx) = mpsc::unbounded_channel();
        cam.add_subscriber(tx);
        cam.add_subscriber(gone_tx);
        drop(gone_rx);
        assert_eq!(cam.push(640, 360, vec![1]), 1);
        assert_eq!(cam.push(640, 360, vec![2]), 1);
        for expected in 0..2u64 {
            match rx.try_recv().unwrap() {
                MediaPacket::Video { seq, width, .. } => {
                    assert_eq!((seq, width), (expected, 640));
                }
                other => panic!("unexpected packet {other:?}"),
            }
        }
    }
}
