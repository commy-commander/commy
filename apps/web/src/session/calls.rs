use std::rc::Rc;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use chrono::Utc;
use commy_audio::codec::{VoiceDecoder, VoiceEncoder};
use commy_audio::speaking::SpeakingDetector;
use commy_core::dm::{CallSignal, DmAction, DmMessage};
use commy_core::{ChannelId, UserId};
use commy_net::node::{MediaChannel, MediaGate};
use commy_net::{DmEnvelope, MediaHello, MediaPacket};
use commy_view::calls::{self, CallBook, CallStep, DmCall, DmCallView};
use iroh::endpoint::Connection;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use wasm_bindgen_futures::spawn_local;

use super::{to_json, Args, Session};
use crate::dm_book;

const RING_FOR: Duration = Duration::from_millis(calls::RING_FOR_MS as u64);
const REDIAL_EVERY: Duration = Duration::from_millis(calls::REDIAL_EVERY_MS as u64);
const QUIET_CHECK: Duration = Duration::from_millis(250);
const QUIET_AFTER_MS: i64 = 250;

pub(super) type Admit = Arc<StdMutex<Option<(UserId, ChannelId, String)>>>;

#[derive(Default)]
pub(super) struct Voice {
    book: CallBook,
    muted: bool,
    deafened: bool,
    link: Option<Link>,
}

struct Link {
    channel: ChannelId,
    partner: UserId,
    me: UserId,
    encoder: VoiceEncoder,
    decoder: VoiceDecoder,
    seq: u32,
    generation: u64,
    out: Option<mpsc::UnboundedSender<MediaPacket>>,
    connection: Option<Connection>,
    mine: SpeakingDetector,
    theirs: SpeakingDetector,
    spoke_ms: i64,
    heard_ms: i64,
    frames_in: u64,
    frames_out: u64,
}

impl Link {
    fn new(channel: ChannelId, partner: UserId, me: UserId) -> Self {
        Self {
            channel,
            partner,
            me,
            encoder: VoiceEncoder::new(),
            decoder: VoiceDecoder::new(),
            seq: 0,
            generation: 0,
            out: None,
            connection: None,
            mine: SpeakingDetector::default(),
            theirs: SpeakingDetector::default(),
            spoke_ms: 0,
            heard_ms: 0,
            frames_in: 0,
            frames_out: 0,
        }
    }
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub(super) fn media_gate(admit: Admit) -> MediaGate {
    Arc::new(move |peer, hello| {
        let MediaHello::Voice { user, channel } = hello else { return false };
        let node = data_encoding::HEXLOWER.encode(&peer);
        admit.lock().unwrap().as_ref().is_some_and(|(u, c, n)| u == user && c == channel && n.eq_ignore_ascii_case(&node))
    })
}

impl Session {
    pub(super) fn voice_settings(&self) -> Value {
        let voice = self.voice.borrow();
        json!({
            "noise_cancellation": true,
            "muted": voice.muted,
            "deafened": voice.deafened,
            "output_volume": 1,
            "input_gain": 1,
            "afk_timeout_minutes": 10,
        })
    }

    fn call_view(&self, call: &DmCall, state: &str, reason: Option<&str>) -> DmCallView {
        let name = {
            let client = self.client.borrow();
            let servers = client.servers.lock().unwrap();
            dm_book::known_name(&self.dms.borrow().store, &servers, call.partner)
        };
        call.view(name, state, reason, now_ms())
    }

    fn announce(&self, call: &DmCall, state: &str, reason: Option<&str>) -> DmCallView {
        let view = self.call_view(call, state, reason);
        self.emit("dm-call", json!(view));
        view
    }

    fn speaking(&self, user: UserId, speaking: bool) {
        self.emit("voice-speaking", json!({ "user": user.to_string(), "speaking": speaking }));
    }

    pub(super) async fn call_command(self: &Rc<Self>, cmd: &str, args: Args) -> Result<Value, String> {
        match cmd {
            "get_dm_call" => {
                let call = self.voice.borrow().book.current().cloned();
                match call {
                    Some(call) => to_json(&self.call_view(&call, call.state(), None)),
                    None => Ok(Value::Null),
                }
            }
            "start_dm_call" => {
                let (_, partner) = self.recipient(args.user_id.as_ref())?;
                let call = self.voice.borrow_mut().book.start(partner, now_ms())?;
                if let Err(e) = self.send_call_signal(partner, call.channel, CallSignal::Ring).await {
                    self.voice.borrow_mut().book.end();
                    return Err(e);
                }
                self.ring_timeout(call.channel);
                to_json(&self.announce(&call, "ringing", None))
            }
            "answer_dm_call" => {
                let accept = args.accept.unwrap_or(false);
                let call = self.voice.borrow_mut().book.answer(accept)?;
                if !accept {
                    let _ = self.send_call_signal(call.partner, call.channel, CallSignal::Decline).await;
                    self.announce(&call, "ended", Some("declined"));
                    return Ok(Value::Null);
                }
                if let Err(e) = self.send_call_signal(call.partner, call.channel, CallSignal::Accept).await {
                    self.end_call(false, "failed").await;
                    return Err(e);
                }
                self.connect(call).await.map(|_| Value::Null)
            }
            "end_dm_call" => {
                self.end_call(true, "hung_up").await;
                Ok(Value::Null)
            }
            "get_voice_settings" => Ok(self.voice_settings()),
            "voice_stats" => self.voice_stats(),
            "set_muted" => {
                self.voice.borrow_mut().muted = args.muted.unwrap_or(false);
                self.quiet_down(true);
                Ok(self.voice_settings())
            }
            "set_deafened" => {
                {
                    let mut voice = self.voice.borrow_mut();
                    voice.deafened = args.deafened.unwrap_or(false);
                    voice.muted |= voice.deafened;
                }
                self.quiet_down(true);
                Ok(self.voice_settings())
            }
            _ => Err(format!("{cmd} is not available in the web app yet")),
        }
    }

    async fn send_call_signal(self: &Rc<Self>, to: UserId, call: ChannelId, signal: CallSignal) -> Result<(), String> {
        let (me, identity) = self.me_and_key().ok_or("Your servers are not unlocked yet")?;
        let node = self.node_hex().ok_or("Your servers are not unlocked yet")?;
        let mut message = DmMessage::acting(me.id, to, DmAction::Call { call, signal }, "");
        message.sign(&node, &identity);
        let envelope = DmEnvelope::new(message, self.profile_for(&me, to));
        if self.deliver_call(&identity, to, envelope).await {
            Ok(())
        } else {
            Err("they are offline right now".into())
        }
    }

    pub(super) fn on_call_signal(self: &Rc<Self>, from: UserId, node: String, channel: ChannelId, signal: CallSignal) {
        let step = self.voice.borrow_mut().book.on_signal(from, &node, channel, signal, now_ms());
        match step {
            CallStep::Nothing => {}
            CallStep::Reply(reply) => {
                let this = self.clone();
                spawn_local(async move {
                    let _ = this.send_call_signal(from, channel, reply).await;
                });
            }
            CallStep::Ringing(call) => {
                self.announce(&call, "ringing", None);
                self.ring_timeout(channel);
            }
            CallStep::Connect(call) => {
                let this = self.clone();
                spawn_local(async move {
                    if this.connect(call).await.is_err() {
                        this.end_call(true, "failed").await;
                    }
                });
            }
            CallStep::Ended { call, reason } => {
                self.hang_up_media();
                self.announce(&call, "ended", Some(reason));
            }
        }
    }

    pub(super) fn settled_elsewhere(&self, partner: UserId, channel: ChannelId, signal: CallSignal) {
        let settled = self.voice.borrow_mut().book.settled_elsewhere(partner, channel, signal);
        if let Some(call) = settled {
            self.announce(&call, "ended", Some("elsewhere"));
        }
    }

    fn ring_timeout(self: &Rc<Self>, channel: ChannelId) {
        let this = self.clone();
        spawn_local(async move {
            n0_future::time::sleep(RING_FOR).await;
            let step = this.voice.borrow_mut().book.ring_out(channel);
            if let CallStep::Ended { call, reason } = step {
                if call.outgoing {
                    let _ = this.send_call_signal(call.partner, call.channel, CallSignal::HangUp).await;
                }
                this.announce(&call, "ended", Some(reason));
            }
        });
    }

    async fn connect(self: &Rc<Self>, call: DmCall) -> Result<(), String> {
        let (me, _) = self.me_and_key().ok_or("Your servers are not unlocked yet")?;
        let _ = self.host.start_voice().await;
        let Some(call) = self.voice.borrow_mut().book.connected(call.channel, now_ms()) else {
            self.host.stop_voice();
            return Err("the call ended".into());
        };
        self.voice.borrow_mut().link = Some(Link::new(call.channel, call.partner, me.id));
        *self.admit.lock().unwrap() = call.node.clone().map(|node| (call.partner, call.channel, node));
        self.announce(&call, "active", None);
        spawn_local(self.clone().keep_call(call.channel));
        Ok(())
    }

    pub(super) async fn end_call(self: &Rc<Self>, notify: bool, reason: &str) {
        let Some(call) = self.voice.borrow_mut().book.end() else { return };
        self.hang_up_media();
        if notify {
            let _ = self.send_call_signal(call.partner, call.channel, CallSignal::HangUp).await;
        }
        self.announce(&call, "ended", Some(reason));
    }

    fn hang_up_media(&self) {
        *self.admit.lock().unwrap() = None;
        let link = self.voice.borrow_mut().link.take();
        if let Some(mut link) = link {
            if let Some(connection) = link.connection.take() {
                connection.close(0u32.into(), b"call ended");
            }
            if link.mine.reset() {
                self.speaking(link.me, false);
            }
            if link.theirs.reset() {
                self.speaking(link.partner, false);
            }
        }
        self.host.stop_voice();
    }

    async fn keep_call(self: Rc<Self>, channel: ChannelId) {
        let mut since_dial = REDIAL_EVERY;
        loop {
            if self.voice.borrow().link.as_ref().map(|link| link.channel) != Some(channel) {
                return;
            }
            self.quiet_down(false);
            if since_dial >= REDIAL_EVERY {
                since_dial = Duration::ZERO;
                self.dial(channel).await;
            }
            n0_future::time::sleep(QUIET_CHECK).await;
            since_dial += QUIET_CHECK;
        }
    }

    async fn dial(self: &Rc<Self>, channel: ChannelId) {
        let target = {
            let voice = self.voice.borrow();
            let node = voice.book.active().filter(|call| call.channel == channel).and_then(|call| call.node.clone());
            voice.link.as_ref().filter(|link| link.out.is_none()).zip(node).map(|(link, node)| (link.partner, link.me, node))
        };
        let Some((partner, me, theirs)) = target else { return };
        let Some(ours) = self.node_hex() else { return };
        if !calls::should_dial(&ours, &theirs) {
            return;
        }
        let dialer = {
            let node = self.node.lock().await;
            let Some(node) = node.as_ref() else { return };
            node.reach_via_relays(std::slice::from_ref(&theirs));
            node.media_dialer()
        };
        if let Ok(media) = dialer.connect(&theirs, MediaHello::Voice { user: me, channel }).await {
            self.attach(channel, partner, media);
        }
    }

    pub(super) async fn run_media(self: Rc<Self>, mut incoming: mpsc::UnboundedReceiver<MediaChannel>) {
        while let Some(media) = incoming.recv().await {
            let MediaHello::Voice { user, channel } = media.hello else { continue };
            let node = data_encoding::HEXLOWER.encode(&media.peer);
            if self.voice.borrow().book.admits(user, channel, &node) {
                self.attach(channel, user, media);
            }
        }
    }

    fn attach(self: &Rc<Self>, channel: ChannelId, partner: UserId, media: MediaChannel) {
        let MediaChannel { mut incoming, outgoing, connection, .. } = media;
        let generation = {
            let mut voice = self.voice.borrow_mut();
            let Some(link) = voice.link.as_mut().filter(|link| link.channel == channel && link.partner == partner) else {
                connection.close(0u32.into(), b"call ended");
                return;
            };
            if let Some(old) = link.connection.replace(connection) {
                old.close(0u32.into(), b"replaced");
            }
            link.generation += 1;
            link.out = Some(outgoing);
            link.decoder = VoiceDecoder::new();
            link.generation
        };
        let this = self.clone();
        spawn_local(async move {
            while let Some(packet) = incoming.recv().await {
                if !this.heard(generation, packet) {
                    return;
                }
            }
            let mut voice = this.voice.borrow_mut();
            if let Some(link) = voice.link.as_mut().filter(|link| link.generation == generation) {
                link.out = None;
                link.connection = None;
            }
        });
    }

    fn heard(&self, generation: u64, packet: MediaPacket) -> bool {
        let mut voice = self.voice.borrow_mut();
        let deafened = voice.deafened;
        let Some(link) = voice.link.as_mut().filter(|link| link.generation == generation) else { return false };
        let pcm = match packet {
            MediaPacket::Opus { data, .. } => link.decoder.decode(&data),
            MediaPacket::Voice { pcm, .. } => pcm,
            _ => return true,
        };
        link.heard_ms = now_ms();
        link.frames_in += 1;
        let partner = link.partner;
        let change = link.theirs.push(&pcm);
        drop(voice);
        if let Some(speaking) = change {
            self.speaking(partner, speaking);
        }
        if !deafened {
            self.host.play_voice(&partner.to_string(), &pcm);
        }
        true
    }

    pub(crate) fn voice_frame(&self, pcm: &[i16]) {
        let mut voice = self.voice.borrow_mut();
        if voice.muted {
            return;
        }
        let Some(link) = voice.link.as_mut() else { return };
        link.spoke_ms = now_ms();
        let change = link.mine.push(pcm);
        let me = link.me;
        if let Some(data) = link.encoder.encode(pcm) {
            if let Some(out) = &link.out {
                if out.send(MediaPacket::Opus { seq: link.seq, data }).is_ok() {
                    link.frames_out += 1;
                }
            }
        }
        link.seq = link.seq.wrapping_add(1);
        drop(voice);
        if let Some(speaking) = change {
            self.speaking(me, speaking);
        }
    }

    fn voice_stats(&self) -> Result<Value, String> {
        let voice = self.voice.borrow();
        let link = voice.link.as_ref().ok_or("not in voice")?;
        let name = {
            let client = self.client.borrow();
            let servers = client.servers.lock().unwrap();
            dm_book::known_name(&self.dms.borrow().store, &servers, link.partner)
        };
        let links: Vec<Value> = link
            .connection
            .iter()
            .filter(|_| link.out.is_some())
            .map(|connection| {
                let stats = connection.stats();
                let rtt = connection.paths().iter().find(|p| p.is_selected()).map(|p| p.rtt()).unwrap_or_default();
                let path = commy_net::link_path(connection);
                json!({
                    "user": link.partner.to_string(),
                    "name": name,
                    "rtt_ms": rtt.as_secs_f32() * 1000.0,
                    "loss_pct": stats.lost_packets as f32 * 100.0 / stats.udp_tx.datagrams.max(1) as f32,
                    "jitter_ms": 0.0,
                    "path": path.kind,
                    "relay": path.relay,
                    "addr": path.addr,
                    "tx_bytes": stats.udp_tx.bytes,
                    "rx_bytes": stats.udp_rx.bytes,
                    "frames_in": link.frames_in,
                    "frames_out": link.frames_out,
                })
            })
            .collect();
        Ok(json!({
            "links": links,
            "codec": "Opus, 48 kHz mono, 10 ms frames",
            "bitrate_kbps": commy_audio::codec::VOICE_BITRATE / 1000,
        }))
    }

    fn quiet_down(&self, mine_now: bool) {
        let now = now_ms();
        let mut stopped = Vec::new();
        {
            let mut voice = self.voice.borrow_mut();
            let muted = voice.muted;
            let Some(link) = voice.link.as_mut() else { return };
            let mine_quiet = (mine_now && muted) || now - link.spoke_ms >= QUIET_AFTER_MS;
            if mine_quiet && link.mine.reset() {
                stopped.push(link.me);
            }
            if now - link.heard_ms >= QUIET_AFTER_MS && link.theirs.reset() {
                stopped.push(link.partner);
            }
        }
        for user in stopped {
            self.speaking(user, false);
        }
    }
}
