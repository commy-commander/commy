use commy_core::{ChannelId, ServerId, UserId};
use commy_media::{Mixer, OutputHandle, StreamGuards};
use commy_net::media::MediaPacket;
use commy_net::node::MediaChannel;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

pub fn should_dial(our_node: &str, their_node: &str) -> bool {
    our_node < their_node
}

pub fn mixer_key(user: UserId) -> u64 {
    let bytes = user.0.as_bytes();
    u64::from_le_bytes(bytes[..8].try_into().unwrap())
}

pub fn silenced(until_ms: &AtomicI64) -> bool {
    chrono::Utc::now().timestamp_millis() < until_ms.load(Ordering::Relaxed)
}

pub struct VoiceSession {
    pub server: ServerId,
    pub channel: ChannelId,
    pub mixer: Arc<Mutex<Mixer>>,
    pub deafened: Arc<AtomicBool>,
    pub mic_broadcast: broadcast::Sender<(u32, Arc<Vec<u8>>)>,
    pub last_activity: Arc<std::sync::atomic::AtomicI64>,
    me: UserId,
    own_silence: Arc<AtomicI64>,
    timeouts: HashMap<UserId, i64>,
    peer_silence: HashMap<UserId, Arc<AtomicI64>>,
    peers: HashMap<UserId, JoinHandle<()>>,
    links: HashMap<UserId, (commy_net::node::Connection, Arc<Mutex<JitterMeter>>)>,
    dialing: HashSet<UserId>,
    devices: StreamGuards,
    on_speaking: SpeakingSink,
    mic_tx: mpsc::UnboundedSender<Vec<i16>>,
    _seq_task: JoinHandle<()>,
}

#[derive(Debug, Clone)]
pub struct LinkStats {
    pub user: UserId,
    pub rtt_ms: f32,
    pub loss_pct: f32,
    pub jitter_ms: f32,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub path: commy_net::LinkPath,
}

#[derive(Debug, Default)]
pub struct JitterMeter {
    last: Option<(u32, std::time::Instant)>,
    jitter_ms: f32,
}

impl JitterMeter {
    const RFC3550_SMOOTHING_DIVISOR: f32 = 16.0;

    pub fn arrived(&mut self, seq: u32, at: std::time::Instant) {
        if let Some((last_seq, last_at)) = self.last {
            if seq > last_seq {
                let expected = (seq - last_seq) as f32 * VOICE_FRAME_MS as f32;
                let actual = at.duration_since(last_at).as_secs_f32() * 1000.0;
                let d = (actual - expected).abs();
                self.jitter_ms += (d - self.jitter_ms) / Self::RFC3550_SMOOTHING_DIVISOR;
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

pub type SpeakingSink = Arc<dyn Fn(UserId, bool) + Send + Sync>;

const SPEAKING_TICK: std::time::Duration = std::time::Duration::from_millis(50);

const VOICE_FRAME_MS: u32 = 10;

const SILENT_FRAMES_PER_TICK: u32 = (SPEAKING_TICK.as_millis() / VOICE_FRAME_MS as u128) as u32;

fn credit_silence(detector: &mut commy_media::SpeakingDetector) -> Option<bool> {
    let mut change = None;
    for _ in 0..SILENT_FRAMES_PER_TICK {
        if let Some(c) = detector.push_voiced(false) {
            change = Some(c);
        }
    }
    change
}

impl VoiceSession {
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
        let own_silence = Arc::new(AtomicI64::new(0));
        let seq_task = {
            let mic_broadcast = mic_broadcast.clone();
            let last_activity = last_activity.clone();
            let on_speaking = on_speaking.clone();
            let own_silence = own_silence.clone();
            tokio::spawn(async move {
                let mut mic_rx = mic_rx;
                let mut seq: u32 = 0;
                let mut encoder = commy_audio::codec::VoiceEncoder::new();
                let mut detector = commy_media::SpeakingDetector::default();
                let mut tick = tokio::time::interval(SPEAKING_TICK);
                let mut heard_since_tick = false;
                loop {
                    tokio::select! {
                        pcm = mic_rx.recv() => {
                            let Some(pcm) = pcm else { break };
                            heard_since_tick = true;
                            if commy_media::frame_has_signal(&pcm) {
                                last_activity.store(chrono::Utc::now().timestamp(), Ordering::Relaxed);
                            }
                            if silenced(&own_silence) {
                                continue;
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
            me,
            own_silence,
            timeouts: HashMap::new(),
            peer_silence: HashMap::new(),
            peers: HashMap::new(),
            links: HashMap::new(),
            dialing: HashSet::new(),
            devices,
            on_speaking,
            mic_tx,
            _seq_task: seq_task,
        }
    }

    pub fn set_speaker(&mut self, speaker: Option<OutputHandle>) {
        self.devices.speaker = speaker;
    }

    pub fn switch_devices(
        &mut self,
        processor: Arc<Mutex<commy_audio::VoiceProcessor>>,
        input: Option<String>,
        output: Option<String>,
    ) {
        let far_end = processor.lock().unwrap().far_end_sink();
        self.devices.mic = None;
        self.devices.mic = match commy_media::start_microphone(processor, self.mic_tx.clone(), input) {
            Ok(mic) => Some(mic),
            Err(e) => {
                tracing::warn!("microphone switch failed: {e}");
                None
            }
        };
        self.devices.speaker = None;
        self.devices.speaker = match commy_media::start_playback(self.mixer.clone(), output, Some(far_end)) {
            Ok(speaker) => Some(speaker),
            Err(e) => {
                tracing::warn!("speaker switch failed: {e}");
                None
            }
        };
    }

    pub fn has_peer(&self, user: UserId) -> bool {
        self.peers.get(&user).is_some_and(|task| !task.is_finished())
    }

    pub fn claim_dial(&mut self, user: UserId) -> bool {
        !self.has_peer(user) && self.dialing.insert(user)
    }

    pub fn dial_failed(&mut self, user: UserId) {
        self.dialing.remove(&user);
    }

    pub fn set_timeouts(&mut self, timeouts: HashMap<UserId, i64>) {
        let until = |user: UserId| timeouts.get(&user).copied().unwrap_or(0);
        self.own_silence.store(until(self.me), Ordering::Relaxed);
        for (user, silence) in &self.peer_silence {
            silence.store(until(*user), Ordering::Relaxed);
        }
        self.timeouts = timeouts;
    }

    pub fn live_peers(&self) -> Vec<UserId> {
        self.peers.iter().filter(|(_, task)| !task.is_finished()).map(|(user, _)| *user).collect()
    }

    pub fn attach_peer(&mut self, user: UserId, media: MediaChannel) {
        let meter = Arc::new(Mutex::new(JitterMeter::default()));
        self.links.insert(user, (media.connection.clone(), meter.clone()));
        let MediaChannel { incoming, outgoing, .. } = media;
        self.pump_peer(user, incoming, outgoing, meter);
    }

    fn pump_peer(
        &mut self,
        user: UserId,
        mut incoming: mpsc::UnboundedReceiver<MediaPacket>,
        outgoing: mpsc::UnboundedSender<MediaPacket>,
        meter: Arc<Mutex<JitterMeter>>,
    ) {
        self.dialing.remove(&user);
        let key = mixer_key(user);
        if let Some(old) = self.peers.remove(&user) {
            old.abort();
            self.mixer.lock().unwrap().remove_peer(key);
        }
        let mixer = self.mixer.clone();
        let deafened = self.deafened.clone();
        let mut mic_rx = self.mic_broadcast.subscribe();
        let on_speaking = self.on_speaking.clone();
        let silence = Arc::new(AtomicI64::new(self.timeouts.get(&user).copied().unwrap_or(0)));
        self.peer_silence.insert(user, silence.clone());

        let task = tokio::spawn(async move {
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
                    packet = incoming.recv() => {
                        if packet.is_some() && silenced(&silence) {
                            continue;
                        }
                        let (seq, pcm) = match packet {
                            Some(MediaPacket::Opus { seq, data }) => {
                                heard_since_tick = true;
                                meter.lock().unwrap().arrived(seq, std::time::Instant::now());
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
                                (seq, decoder.decode(&data))
                            }
                            Some(MediaPacket::Voice { seq, pcm }) => {
                                heard_since_tick = true;
                                meter.lock().unwrap().arrived(seq, std::time::Instant::now());
                                (seq, pcm)
                            }
                            Some(_) => continue,
                            None => break,
                        };
                        if let Some(speaking) = detector.push(&pcm) {
                            on_speaking(user, speaking);
                        }
                        if !deafened.load(Ordering::Relaxed) {
                            mixer.lock().unwrap().push(key, seq, pcm);
                        }
                    }
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

        self.peers.insert(user, task);
    }

    pub fn link_stats(&self) -> Vec<LinkStats> {
        self.links
            .iter()
            .filter(|(user, _)| self.has_peer(**user))
            .map(|(user, (conn, meter))| {
                let stats = conn.stats();
                let sent = stats.udp_tx.datagrams.max(1);
                let rtt = conn.paths().iter().find(|p| p.is_selected()).map(|p| p.rtt()).unwrap_or_default();
                LinkStats {
                    user: *user,
                    rtt_ms: rtt.as_secs_f32() * 1000.0,
                    loss_pct: stats.lost_packets as f32 * 100.0 / sent as f32,
                    jitter_ms: meter.lock().unwrap().jitter_ms(),
                    tx_bytes: stats.udp_tx.bytes,
                    rx_bytes: stats.udp_rx.bytes,
                    path: commy_net::link_path(conn),
                }
            })
            .collect()
    }

    pub fn detach_peer(&mut self, user: UserId) {
        self.links.remove(&user);
        self.peer_silence.remove(&user);
        if let Some(task) = self.peers.remove(&user) {
            task.abort();
        }
        self.mixer.lock().unwrap().remove_peer(mixer_key(user));
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

pub struct ScreenShareSession {
    pub server: ServerId,
    pub channel: ChannelId,
    pub subscribers: Arc<Mutex<Vec<mpsc::UnboundedSender<MediaPacket>>>>,
    latest: Arc<Mutex<Option<commy_media::screen::ScreenFrame>>>,
    stop: Arc<AtomicBool>,
    encoded: Arc<std::sync::atomic::AtomicU64>,
}

const ENCODERS: usize = 2;
const JPEG_QUALITY: u8 = 60;

pub type FrameSource = Box<dyn FnMut() -> anyhow::Result<image::RgbaImage>>;
pub type FrameOpener = Box<dyn FnMut() -> anyhow::Result<FrameSource> + Send>;

impl ScreenShareSession {
    pub fn start(
        server: ServerId,
        channel: ChannelId,
        fps: u32,
        max_width: u32,
        source: Option<(commy_media::screen::SourceKind, u32)>,
    ) -> Self {
        let opener: FrameOpener = Box::new(move || {
            let mut capturer = commy_media::screen::Capturer::open(source)?;
            Ok(Box::new(move || capturer.grab()) as FrameSource)
        });
        Self::start_with(server, channel, fps, max_width, opener)
    }

    pub fn start_with(server: ServerId, channel: ChannelId, fps: u32, max_width: u32, mut open: FrameOpener) -> Self {
        let subscribers: Arc<Mutex<Vec<mpsc::UnboundedSender<MediaPacket>>>> =
            Arc::new(Mutex::new(Vec::new()));
        let latest = Arc::new(Mutex::new(None));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let encoded = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (raw_tx, raw_rx) = std::sync::mpsc::sync_channel::<(u64, image::RgbaImage)>(ENCODERS);
        let raw_rx = Arc::new(Mutex::new(raw_rx));
        {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let interval = std::time::Duration::from_micros(1_000_000 / fps.max(1) as u64);
                let mut capturer: Option<FrameSource> = None;
                let mut seq: u64 = 0;
                while !stop.load(Ordering::Relaxed) {
                    let started = std::time::Instant::now();
                    if capturer.is_none() {
                        match open() {
                            Ok(c) => capturer = Some(c),
                            Err(e) => {
                                tracing::warn!("screen capture unavailable: {e}");
                                std::thread::sleep(std::time::Duration::from_secs(1));
                                continue;
                            }
                        }
                    }
                    match capturer.as_mut().map(|grab| grab()) {
                        Some(Ok(image)) => {
                            match raw_tx.try_send((seq, image)) {
                                Ok(()) | Err(std::sync::mpsc::TrySendError::Full(_)) => {}
                                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => break,
                            }
                            seq += 1;
                        }
                        Some(Err(e)) => {
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

pub struct ScreenWatch {
    pub task: JoinHandle<()>,
    _channel: Option<mpsc::UnboundedSender<MediaPacket>>,
}

impl ScreenWatch {
    pub fn start(
        user: UserId,
        media: MediaChannel,
        emit: impl Fn(UserId, u64, u32, u32, Vec<u8>) + Send + 'static,
    ) -> Self {
        let MediaChannel { incoming, outgoing, .. } = media;
        Self::from_frames(user, incoming, Some(outgoing), emit)
    }

    pub fn from_frames(
        user: UserId,
        mut incoming: mpsc::UnboundedReceiver<MediaPacket>,
        channel: Option<mpsc::UnboundedSender<MediaPacket>>,
        emit: impl Fn(UserId, u64, u32, u32, Vec<u8>) + Send + 'static,
    ) -> Self {
        let task = tokio::spawn(async move {
            let mut last_seq: Option<u64> = None;
            while let Some(mut packet) = incoming.recv().await {
                while let Ok(newer) = incoming.try_recv() {
                    packet = newer;
                }
                if let MediaPacket::Video { seq, width, height, jpeg } = packet {
                    if last_seq.is_some_and(|last| seq <= last) {
                        continue;
                    }
                    last_seq = Some(seq);
                    emit(user, seq, width, height, jpeg);
                }
            }
        });
        Self { task, _channel: channel }
    }
}

impl Drop for ScreenWatch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

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

    pub fn push(&mut self, width: u32, height: u32, jpeg: Vec<u8>) -> usize {
        let packet = MediaPacket::Video { seq: self.seq, width, height, jpeg };
        self.seq += 1;
        self.subscribers.retain(|tx| tx.send(packet.clone()).is_ok());
        self.subscribers.len()
    }
}

pub const MIC_TEST_KEY: u64 = u64::MAX;

pub struct MicTest {
    _devices: StreamGuards,
    task: JoinHandle<()>,
    call_mixer: Option<Arc<Mutex<Mixer>>>,
}

impl MicTest {
    pub fn start(
        processor: std::sync::Arc<Mutex<commy_audio::VoiceProcessor>>,
        input: Option<String>,
        output: Option<String>,
    ) -> anyhow::Result<Self> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<i16>>();
        let far_end = processor.lock().unwrap().far_end_sink();
        let mic = commy_media::start_microphone(processor, tx, input)?;
        let mixer = Arc::new(Mutex::new(Mixer::new()));
        let speaker = commy_media::start_playback(mixer.clone(), output, Some(far_end))?;
        let task = tokio::spawn(async move {
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
            call_mixer: None,
        })
    }

    pub fn within_call(session: &VoiceSession) -> Self {
        let mixer = session.mixer.clone();
        let mut packets = session.mic_broadcast.subscribe();
        let task = {
            let mixer = mixer.clone();
            tokio::spawn(async move {
                let mut decoder = commy_audio::codec::VoiceDecoder::new();
                loop {
                    match packets.recv().await {
                        Ok((seq, packet)) => {
                            let pcm = decoder.decode(&packet);
                            mixer.lock().unwrap().push(MIC_TEST_KEY, seq, pcm);
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            })
        };
        Self {
            _devices: StreamGuards { mic: None, speaker: None },
            task,
            call_mixer: Some(mixer),
        }
    }
}

impl Drop for MicTest {
    fn drop(&mut self) {
        self.task.abort();
        if let Some(mixer) = &self.call_mixer {
            mixer.lock().unwrap().remove_peer(MIC_TEST_KEY);
        }
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

    fn silent_call() -> (VoiceSession, mpsc::UnboundedSender<Vec<i16>>) {
        let (mic_tx, mic_rx) = mpsc::unbounded_channel();
        let session = VoiceSession::start(
            ServerId::new(),
            ChannelId::new(),
            StreamGuards { mic: None, speaker: None },
            mic_rx,
            mic_tx.clone(),
            false,
            UserId::new(),
            Arc::new(|_, _| {}),
        );
        (session, mic_tx)
    }

    async fn wait_for_sources(mixer: &Arc<Mutex<Mixer>>, count: usize) {
        for _ in 0..200 {
            if mixer.lock().unwrap().peer_count() == count {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("mixer never reached {count} sources");
    }

    #[tokio::test]
    async fn mic_test_during_a_call_plays_the_call_mic_through_the_call_speaker() {
        let (session, mic) = silent_call();
        let test = MicTest::within_call(&session);
        for _ in 0..5 {
            mic.send(vec![1000; commy_audio::FRAME_SIZE]).unwrap();
        }
        wait_for_sources(&session.mixer, 1).await;
        drop(test);
        assert_eq!(session.mixer.lock().unwrap().peer_count(), 0);
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
mod link_tests {
    use super::*;

    fn session() -> VoiceSession {
        let (mic_tx, mic_rx) = mpsc::unbounded_channel();
        VoiceSession::start(
            ServerId::new(),
            ChannelId::new(),
            StreamGuards { mic: None, speaker: None },
            mic_rx,
            mic_tx,
            false,
            UserId::new(),
            Arc::new(|_, _| {}),
        )
    }

    fn link(session: &mut VoiceSession, user: UserId) -> mpsc::UnboundedSender<MediaPacket> {
        let (in_tx, in_rx) = mpsc::unbounded_channel();
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        std::mem::forget(out_rx);
        session.pump_peer(user, in_rx, out_tx, Default::default());
        in_tx
    }

    async fn until(mut done: impl FnMut() -> bool) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        while !done() {
            assert!(tokio::time::Instant::now() < deadline, "timed out waiting");
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    fn voice(seq: u32, level: i16) -> MediaPacket {
        MediaPacket::Voice { seq, pcm: vec![level; commy_audio::FRAME_SIZE] }
    }

    #[tokio::test]
    async fn a_failed_dial_can_be_retried() {
        let mut session = session();
        let friend = UserId::new();
        assert!(session.claim_dial(friend));
        assert!(!session.claim_dial(friend), "one dial at a time");
        session.dial_failed(friend);
        assert!(session.claim_dial(friend));
    }

    #[tokio::test]
    async fn a_dropped_link_frees_the_peer_for_a_redial() {
        let mut session = session();
        let friend = UserId::new();
        assert!(session.claim_dial(friend));
        let wire = link(&mut session, friend);
        assert!(session.has_peer(friend));
        assert!(!session.claim_dial(friend));
        drop(wire);
        until(|| !session.has_peer(friend)).await;
        assert!(session.claim_dial(friend));
    }

    #[tokio::test]
    async fn a_timed_out_peer_is_not_heard_until_the_timeout_runs_out() {
        let mut session = session();
        let loud = UserId::new();
        let far = chrono::Utc::now().timestamp_millis() + 60_000;
        session.set_timeouts(HashMap::from([(loud, far)]));
        let wire = link(&mut session, loud);
        for seq in 0..5 {
            wire.send(voice(seq, 100)).unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let mixer = session.mixer.clone();
        assert_eq!(mixer.lock().unwrap().peer_count(), 0, "their frames are dropped");
        session.set_timeouts(HashMap::new());
        wire.send(voice(5, 200)).unwrap();
        until(|| mixer.lock().unwrap().peer_count() == 1).await;
        assert_eq!(mixer.lock().unwrap().next_frame()[0], 200);
        let past = chrono::Utc::now().timestamp_millis() - 1;
        session.set_timeouts(HashMap::from([(loud, past)]));
        wire.send(voice(6, 150)).unwrap();
        until(|| mixer.lock().unwrap().next_frame()[0] == 150).await;
    }

    #[tokio::test]
    async fn our_own_mic_stays_quiet_while_we_are_timed_out() {
        let (mic_tx, mic_rx) = mpsc::unbounded_channel();
        let me = UserId::new();
        let mut session = VoiceSession::start(
            ServerId::new(),
            ChannelId::new(),
            StreamGuards { mic: None, speaker: None },
            mic_rx,
            mic_tx.clone(),
            false,
            me,
            Arc::new(|_, _| {}),
        );
        let mut heard = session.mic_broadcast.subscribe();
        session.set_timeouts(HashMap::from([(me, chrono::Utc::now().timestamp_millis() + 60_000)]));
        assert!(silenced(&session.own_silence));
        for _ in 0..3 {
            mic_tx.send(vec![1000; commy_audio::FRAME_SIZE]).unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(matches!(heard.try_recv(), Err(broadcast::error::TryRecvError::Empty)), "nothing was encoded");
        session.set_timeouts(HashMap::new());
        assert!(!silenced(&session.own_silence));
        for _ in 0..3 {
            mic_tx.send(vec![1000; commy_audio::FRAME_SIZE]).unwrap();
        }
        until(|| !matches!(heard.try_recv(), Err(broadcast::error::TryRecvError::Empty))).await;
    }

    #[tokio::test]
    async fn a_replaced_link_is_heard_from_its_first_frame() {
        let mut session = session();
        let friend = UserId::new();
        let old = link(&mut session, friend);
        for seq in 5_000..5_005 {
            old.send(voice(seq, 100)).unwrap();
        }
        let mixer = session.mixer.clone();
        until(|| mixer.lock().unwrap().peer_count() == 1).await;
        let new = link(&mut session, friend);
        assert_eq!(mixer.lock().unwrap().peer_count(), 0, "the old link's audio is gone");
        new.send(voice(0, 200)).unwrap();
        until(|| mixer.lock().unwrap().peer_count() == 1).await;
        assert_eq!(mixer.lock().unwrap().next_frame()[0], 200);
    }
}

#[cfg(test)]
mod share_tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    fn synthetic() -> FrameOpener {
        Box::new(|| {
            let mut n: u8 = 0;
            Ok(Box::new(move || {
                n = n.wrapping_add(1);
                Ok(image::RgbaImage::from_pixel(64, 48, image::Rgba([n, 0, 0, 255])))
            }) as FrameSource)
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn own_preview_and_viewers_see_frames_at_stream_rate() {
        let session = ScreenShareSession::start_with(ServerId::new(), ChannelId::new(), 30, 64, synthetic());
        let (preview_tx, preview_rx) = mpsc::unbounded_channel();
        let (viewer_tx, mut viewer_rx) = mpsc::unbounded_channel();
        session.add_subscriber(preview_tx);
        session.add_subscriber(viewer_tx);
        let shown = Arc::new(AtomicU64::new(0));
        let counter = shown.clone();
        let _preview = ScreenWatch::from_frames(UserId::new(), preview_rx, None, move |_, _, w, h, jpeg| {
            assert_eq!((w, h), (64, 48));
            assert_eq!(&jpeg[..2], &[0xff, 0xd8]);
            counter.fetch_add(1, Ordering::Relaxed);
        });
        let mut viewer_frames = 0u64;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        while let Ok(Some(packet)) = tokio::time::timeout_at(deadline, viewer_rx.recv()).await {
            if matches!(packet, MediaPacket::Video { .. }) {
                viewer_frames += 1;
            }
        }
        drop(viewer_rx);
        let previewed = shown.load(Ordering::Relaxed);
        assert!(viewer_frames >= 8, "viewer got {viewer_frames} frames in a second");
        assert!(previewed >= 8, "own preview showed {previewed} frames in a second");
        assert_eq!(session.viewer_count(), 1, "the dropped viewer is no longer counted");
        drop(session);
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
