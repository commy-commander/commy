use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use chrono::Utc;
use commy_audio::codec::{VoiceDecoder, VoiceEncoder};
use commy_audio::speaking::SpeakingDetector;
use commy_core::dm::{CallSignal, DmAction, DmMessage};
use commy_core::{ChannelId, ServerId, UserId};
use commy_net::node::{MediaChannel, MediaGate};
use commy_net::{DmEnvelope, MediaHello, MediaPacket, WirePayload};
use commy_view::calls::{self, CallBook, CallStep, DmCall, DmCallView};
use iroh::endpoint::Connection;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use wasm_bindgen_futures::spawn_local;

use super::{parse, to_json, Args, Session};
use crate::dm_book;
use crate::recording::Recorder;

const RING_FOR: Duration = Duration::from_millis(calls::RING_FOR_MS as u64);
const TEND_EVERY: Duration = Duration::from_secs(1);
const QUIET_CHECK: Duration = Duration::from_millis(250);
const QUIET_AFTER_MS: i64 = 250;

pub(super) enum Gate {
    Call(UserId, ChannelId, String),
    Room(ChannelId),
}

pub(super) type Admit = Arc<StdMutex<Option<Gate>>>;

#[derive(Default)]
pub(super) struct Voice {
    pub(super) personal: Option<super::profile::Personal>,
    book: CallBook,
    muted: bool,
    deafened: bool,
    link: Option<Link>,
    recorder: Option<Recorder>,
}

struct Link {
    server: Option<ServerId>,
    channel: ChannelId,
    me: UserId,
    since_ms: i64,
    encoder: VoiceEncoder,
    seq: u32,
    mine: SpeakingDetector,
    spoke_ms: i64,
    peers: HashMap<UserId, Peer>,
    generation: u64,
}

struct Peer {
    node: String,
    decoder: VoiceDecoder,
    generation: u64,
    out: Option<mpsc::UnboundedSender<MediaPacket>>,
    connection: Option<Connection>,
    dialing: bool,
    theirs: SpeakingDetector,
    heard_ms: i64,
    frames_in: u64,
    frames_out: u64,
}

impl Peer {
    fn new(node: &str) -> Self {
        Self {
            node: node.to_string(),
            decoder: VoiceDecoder::new(),
            generation: 0,
            out: None,
            connection: None,
            dialing: false,
            theirs: SpeakingDetector::default(),
            heard_ms: 0,
            frames_in: 0,
            frames_out: 0,
        }
    }

    fn close(&mut self, why: &[u8]) {
        self.out = None;
        if let Some(connection) = self.connection.take() {
            connection.close(0u32.into(), why);
        }
    }
}

impl Link {
    fn new(server: Option<ServerId>, channel: ChannelId, me: UserId) -> Self {
        Self {
            server,
            channel,
            me,
            since_ms: now_ms(),
            encoder: VoiceEncoder::new(),
            seq: 0,
            mine: SpeakingDetector::default(),
            spoke_ms: 0,
            peers: HashMap::new(),
            generation: 0,
        }
    }

    fn peer(&mut self, user: UserId, node: &str) -> &mut Peer {
        let peer = self.peers.entry(user).or_insert_with(|| Peer::new(node));
        if !peer.node.eq_ignore_ascii_case(node) {
            peer.close(b"moved");
            *peer = Peer::new(node);
        }
        peer
    }
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub(super) fn media_gate(admit: Admit) -> MediaGate {
    Arc::new(move |peer, hello| {
        let MediaHello::Voice { user, channel } = hello else { return false };
        let node = data_encoding::HEXLOWER.encode(&peer);
        match admit.lock().unwrap().as_ref() {
            Some(Gate::Call(u, c, n)) => u == user && c == channel && n.eq_ignore_ascii_case(&node),
            Some(Gate::Room(c)) => c == channel,
            None => false,
        }
    })
}

impl Session {
    pub(super) fn voice_settings(&self) -> Value {
        let personal = self.personal();
        let voice = self.voice.borrow();
        json!({
            "noise_cancellation": true,
            "muted": voice.muted,
            "deafened": voice.deafened,
            "pronouns": personal.pronouns,
            "presence_until": personal.presence_until,
            "status_until": personal.status_until,
            "output_volume": 1,
            "input_gain": 1,
            "afk_timeout_minutes": 10,
        })
    }

    pub(super) fn voice_link_open(&self) -> bool {
        self.voice.borrow().link.is_some()
    }

    pub(super) fn set_recorder(&self, recorder: Option<Recorder>) -> Option<Recorder> {
        std::mem::replace(&mut self.voice.borrow_mut().recorder, recorder)
    }

    pub(super) fn voice_here(&self, server: ServerId) -> Option<(ChannelId, i64)> {
        let voice = self.voice.borrow();
        voice.link.as_ref().filter(|link| link.server == Some(server)).map(|link| (link.channel, link.since_ms))
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
                self.leave_room().await;
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
                self.leave_room().await;
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
            "set_voice_channel" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                let channel: Option<ChannelId> = args.channel_id.as_ref().map(|c| parse(Some(c), "channel")).transpose()?;
                match channel {
                    Some(channel) => self.join_room(server, channel).await?,
                    None => {
                        if self.voice_here(server).is_some() {
                            self.leave_room().await;
                        }
                    }
                }
                Ok(Value::Null)
            }
            "get_voice_settings" => Ok(self.voice_settings()),
            "voice_stats" => self.voice_stats(),
            "set_muted" => {
                self.voice.borrow_mut().muted = args.muted.unwrap_or(false);
                self.quiet_down(true);
                self.say_voice_flags();
                Ok(self.voice_settings())
            }
            "set_deafened" => {
                {
                    let mut voice = self.voice.borrow_mut();
                    voice.deafened = args.deafened.unwrap_or(false);
                    voice.muted |= voice.deafened;
                }
                self.quiet_down(true);
                self.say_voice_flags();
                Ok(self.voice_settings())
            }
            _ => Err(format!("{cmd} is not available in the web app yet")),
        }
    }

    async fn join_room(self: &Rc<Self>, server: ServerId, channel: ChannelId) -> Result<(), String> {
        if self.voice_here(server).is_some_and(|(here, _)| here == channel) {
            return Ok(());
        }
        let (muted, deafened) = {
            let voice = self.voice.borrow();
            (voice.muted, voice.deafened)
        };
        let since = Utc::now().timestamp();
        let joined = self.client.borrow().voice_state(server, Some(channel), muted, deafened, Some(since))?;
        let me = self.client.borrow().profile().ok_or("Your servers are not unlocked yet")?.id;
        self.end_call(true, "hung_up").await;
        let previous = self.voice.borrow().link.as_ref().and_then(|link| link.server).filter(|s| *s != server);
        if let Some(previous) = previous {
            self.say_left(previous);
        }
        self.hang_up_media();
        let _ = self.host.start_voice().await;
        self.voice.borrow_mut().link = Some(Link::new(Some(server), channel, me));
        *self.admit.lock().unwrap() = Some(Gate::Room(channel));
        self.send_voice(server, &joined).await;
        self.say_here(server);
        self.emit("server-event", json!(server.to_string()));
        spawn_local(self.clone().keep_voice(channel));
        Ok(())
    }

    pub(super) async fn leave_room(self: &Rc<Self>) {
        let server = self.voice.borrow().link.as_ref().and_then(|link| link.server);
        let Some(server) = server else { return };
        self.hang_up_media();
        self.say_left(server);
    }

    fn say_left(self: &Rc<Self>, server: ServerId) {
        let Ok(left) = self.client.borrow().voice_state(server, None, false, false, None) else { return };
        let this = self.clone();
        spawn_local(async move {
            this.send_voice(server, &left).await;
            this.say_here(server);
            this.emit("server-event", json!(server.to_string()));
        });
    }

    fn say_voice_flags(self: &Rc<Self>) {
        let (server, channel, since, muted, deafened) = {
            let voice = self.voice.borrow();
            let Some(link) = voice.link.as_ref() else { return };
            let Some(server) = link.server else { return };
            (server, link.channel, link.since_ms / 1000, voice.muted, voice.deafened)
        };
        let Ok(payload) = self.client.borrow().voice_state(server, Some(channel), muted, deafened, Some(since)) else { return };
        let this = self.clone();
        spawn_local(async move {
            this.send_voice(server, &payload).await;
            this.emit("server-event", json!(server.to_string()));
        });
    }

    async fn send_voice(&self, server: ServerId, payload: &WirePayload) {
        self.publish(server, payload);
        let node = self.node.lock().await;
        if let Some(node) = node.as_ref().filter(|node| node.neighbor_count(server) > 0) {
            let _ = node.broadcast(server, payload.clone()).await;
        }
    }

    pub(super) async fn hang_up_on_farewell(self: &Rc<Self>) {
        let server = self.voice.borrow().link.as_ref().and_then(|link| link.server);
        let Some(server) = server else { return };
        self.hang_up_media();
        if let Ok(left) = self.client.borrow().voice_state(server, None, false, false, None) {
            self.publish(server, &left);
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
        self.leave_room().await;
        let _ = self.host.start_voice().await;
        let Some(call) = self.voice.borrow_mut().book.connected(call.channel, now_ms()) else {
            self.release_mic();
            return Err("the call ended".into());
        };
        let mut link = Link::new(None, call.channel, me.id);
        if let Some(node) = call.node.as_deref() {
            link.peer(call.partner, node);
        }
        self.voice.borrow_mut().link = Some(link);
        *self.admit.lock().unwrap() = call.node.clone().map(|node| Gate::Call(call.partner, call.channel, node));
        self.announce(&call, "active", None);
        spawn_local(self.clone().keep_voice(call.channel));
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

    pub(super) fn hang_up_media(&self) {
        *self.admit.lock().unwrap() = None;
        let link = self.voice.borrow_mut().link.take();
        let Some(mut link) = link else { return };
        let mut quiet = Vec::new();
        if link.mine.reset() {
            quiet.push(link.me);
        }
        for (user, peer) in link.peers.iter_mut() {
            peer.close(b"call ended");
            if peer.theirs.reset() {
                quiet.push(*user);
            }
        }
        for user in quiet {
            self.speaking(user, false);
        }
        self.release_mic();
    }

    fn release_mic(&self) {
        if self.voice.borrow().recorder.is_none() {
            self.host.stop_voice();
        }
    }

    async fn keep_voice(self: Rc<Self>, channel: ChannelId) {
        let mut since_tend = TEND_EVERY;
        loop {
            if self.voice.borrow().link.as_ref().map(|link| link.channel) != Some(channel) {
                return;
            }
            self.quiet_down(false);
            if since_tend >= TEND_EVERY {
                since_tend = Duration::ZERO;
                self.tend(channel);
            }
            n0_future::time::sleep(QUIET_CHECK).await;
            since_tend += QUIET_CHECK;
        }
    }

    fn targets(&self, channel: ChannelId) -> Option<Vec<(UserId, String)>> {
        let voice = self.voice.borrow();
        let link = voice.link.as_ref().filter(|link| link.channel == channel)?;
        match link.server {
            Some(server) => Some(self.client.borrow().voice_targets(server, channel)),
            None => Some(voice.book.active().filter(|call| call.channel == channel).and_then(|call| Some((call.partner, call.node.clone()?))).into_iter().collect()),
        }
    }

    fn tend(self: &Rc<Self>, channel: ChannelId) {
        let Some(ours) = self.node_hex() else { return };
        let room = self.voice.borrow().link.as_ref().and_then(|link| link.server);
        if let Some(server) = room {
            if self.client.borrow().voice_moved_away(server, channel, &ours) {
                self.hang_up_media();
                self.say_here(server);
                self.emit("voice-elsewhere", json!(server.to_string()));
                self.emit("server-event", json!(server.to_string()));
                return;
            }
        }
        let Some(targets) = self.targets(channel) else { return };
        let mut gone = Vec::new();
        let mut dial = Vec::new();
        {
            let mut voice = self.voice.borrow_mut();
            let Some(link) = voice.link.as_mut() else { return };
            let wanted: HashMap<UserId, String> = targets.into_iter().collect();
            if link.server.is_some() {
                link.peers.retain(|user, peer| {
                    let keep = wanted.contains_key(user);
                    if !keep {
                        peer.close(b"left");
                        if peer.theirs.reset() {
                            gone.push(*user);
                        }
                    }
                    keep
                });
            }
            for (user, node) in wanted {
                let peer = link.peer(user, &node);
                if peer.out.is_none() && !peer.dialing && calls::should_dial(&ours, &node) {
                    peer.dialing = true;
                    dial.push((user, node));
                }
            }
        }
        for user in gone {
            self.speaking(user, false);
        }
        for (user, node) in dial {
            spawn_local(self.clone().dial(channel, user, node));
        }
    }

    async fn dial(self: Rc<Self>, channel: ChannelId, user: UserId, theirs: String) {
        let me = self.voice.borrow().link.as_ref().map(|link| link.me);
        let dialer = {
            let node = self.node.lock().await;
            node.as_ref().map(|node| {
                node.reach_via_relays(std::slice::from_ref(&theirs));
                node.media_dialer()
            })
        };
        let dialed = match (me, dialer) {
            (Some(me), Some(dialer)) => dialer.connect(&theirs, MediaHello::Voice { user: me, channel }).await.ok(),
            _ => None,
        };
        if let Some(peer) = self.voice.borrow_mut().link.as_mut().and_then(|link| link.peers.get_mut(&user)) {
            peer.dialing = false;
        }
        if let Some(media) = dialed {
            self.attach(channel, user, theirs, media);
        }
    }

    pub(super) async fn run_media(self: Rc<Self>, mut incoming: mpsc::UnboundedReceiver<MediaChannel>) {
        while let Some(media) = incoming.recv().await {
            let MediaHello::Voice { user, channel } = media.hello else { continue };
            let node = data_encoding::HEXLOWER.encode(&media.peer);
            let room = self.voice.borrow().link.as_ref().filter(|link| link.channel == channel).map(|link| link.server);
            let welcome = match room {
                Some(Some(server)) => self.client.borrow().voice_peer_ok(server, user, &node, channel),
                Some(None) => self.voice.borrow().book.admits(user, channel, &node),
                None => false,
            };
            if welcome {
                self.attach(channel, user, node, media);
            } else {
                media.connection.close(0u32.into(), b"not in this call");
            }
        }
    }

    fn attach(self: &Rc<Self>, channel: ChannelId, user: UserId, node: String, media: MediaChannel) {
        let MediaChannel { mut incoming, outgoing, connection, .. } = media;
        let generation = {
            let mut voice = self.voice.borrow_mut();
            let Some(link) = voice.link.as_mut().filter(|link| link.channel == channel) else {
                connection.close(0u32.into(), b"call ended");
                return;
            };
            link.generation += 1;
            let generation = link.generation;
            let peer = link.peer(user, &node);
            if let Some(old) = peer.connection.replace(connection) {
                old.close(0u32.into(), b"replaced");
            }
            peer.generation = generation;
            peer.out = Some(outgoing);
            peer.decoder = VoiceDecoder::new();
            generation
        };
        let this = self.clone();
        spawn_local(async move {
            while let Some(packet) = incoming.recv().await {
                if !this.heard(user, generation, packet) {
                    return;
                }
            }
            let mut voice = this.voice.borrow_mut();
            let peer = voice.link.as_mut().and_then(|link| link.peers.get_mut(&user)).filter(|peer| peer.generation == generation);
            if let Some(peer) = peer {
                peer.out = None;
                peer.connection = None;
            }
        });
    }

    fn heard(&self, user: UserId, generation: u64, packet: MediaPacket) -> bool {
        let mut voice = self.voice.borrow_mut();
        let deafened = voice.deafened;
        let peer = voice.link.as_mut().and_then(|link| link.peers.get_mut(&user)).filter(|peer| peer.generation == generation);
        let Some(peer) = peer else { return false };
        let pcm = match packet {
            MediaPacket::Opus { data, .. } => peer.decoder.decode(&data),
            MediaPacket::Voice { pcm, .. } => pcm,
            _ => return true,
        };
        peer.heard_ms = now_ms();
        peer.frames_in += 1;
        let change = peer.theirs.push(&pcm);
        drop(voice);
        if let Some(speaking) = change {
            self.speaking(user, speaking);
        }
        if !deafened {
            self.host.play_voice(&user.to_string(), &pcm);
        }
        true
    }

    pub(crate) fn voice_frame(&self, pcm: &[i16]) {
        let mut voice = self.voice.borrow_mut();
        if let Some(recorder) = voice.recorder.as_mut() {
            recorder.push(pcm);
        }
        if voice.muted {
            return;
        }
        let Some(link) = voice.link.as_mut() else { return };
        link.spoke_ms = now_ms();
        let change = link.mine.push(pcm);
        let me = link.me;
        if let Some(data) = link.encoder.encode(pcm) {
            for peer in link.peers.values_mut() {
                if let Some(out) = &peer.out {
                    if out.send(MediaPacket::Opus { seq: link.seq, data: data.clone() }).is_ok() {
                        peer.frames_out += 1;
                    }
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
        let client = self.client.borrow();
        let servers = client.servers.lock().unwrap();
        let name_of = |user: UserId| match link.server.and_then(|server| servers.get(&server)) {
            Some(state) => state.profiles.get(&user).map(|p| p.visible_name().to_string()).unwrap_or_default(),
            None => dm_book::known_name(&self.dms.borrow().store, &servers, user),
        };
        let links: Vec<Value> = link
            .peers
            .iter()
            .filter(|(_, peer)| peer.out.is_some())
            .filter_map(|(user, peer)| Some((user, peer, peer.connection.as_ref()?)))
            .map(|(user, peer, connection)| {
                let stats = connection.stats();
                let rtt = connection.paths().iter().find(|p| p.is_selected()).map(|p| p.rtt()).unwrap_or_default();
                let path = commy_net::link_path(connection);
                json!({
                    "user": user.to_string(),
                    "name": name_of(*user),
                    "rtt_ms": rtt.as_secs_f32() * 1000.0,
                    "loss_pct": stats.lost_packets as f32 * 100.0 / stats.udp_tx.datagrams.max(1) as f32,
                    "jitter_ms": 0.0,
                    "path": path.kind,
                    "relay": path.relay,
                    "addr": path.addr,
                    "tx_bytes": stats.udp_tx.bytes,
                    "rx_bytes": stats.udp_rx.bytes,
                    "frames_in": peer.frames_in,
                    "frames_out": peer.frames_out,
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
            for (user, peer) in link.peers.iter_mut() {
                if now - peer.heard_ms >= QUIET_AFTER_MS && peer.theirs.reset() {
                    stopped.push(*user);
                }
            }
        }
        for user in stopped {
            self.speaking(user, false);
        }
    }
}
