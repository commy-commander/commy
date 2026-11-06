use std::rc::Rc;

use commy_audio::codec::VoiceDecoder;
use commy_core::{ServerId, UserId};
use commy_net::media::{mark_view, InkMarkInput};
use commy_net::MediaPacket;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use wasm_bindgen_futures::spawn_local;

use super::{parse, Args, Session};
use crate::watching::{frame_payload, newest_frame_with_all_audio, Feed, STREAM_SPEAKER};

impl Session {
    pub(super) async fn watch_command(self: &Rc<Self>, cmd: &str, args: Args) -> Result<Value, String> {
        match cmd {
            "watch_screen" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                let user: UserId = parse(args.user_id.as_ref(), "user")?;
                self.watch(Feed::Screen, server, user).await.map(|_| Value::Null)
            }
            "watch_camera" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                let user: UserId = parse(args.user_id.as_ref(), "user")?;
                self.watch(Feed::Camera, server, user).await.map(|_| Value::Null)
            }
            "stop_watching" => {
                let user: UserId = parse(args.user_id.as_ref(), "user")?;
                self.watching.borrow_mut().end(Feed::Screen, user, None);
                Ok(Value::Null)
            }
            "stop_watching_camera" => {
                let user: UserId = parse(args.user_id.as_ref(), "user")?;
                self.watching.borrow_mut().end(Feed::Camera, user, None);
                Ok(Value::Null)
            }
            "send_stream_ink" => {
                let user: UserId = parse(args.user_id.as_ref(), "user")?;
                let mark: InkMarkInput = serde_json::from_value(args.mark.clone().ok_or("that drawing could not be sent")?).map_err(|_| "that drawing could not be sent")?;
                self.watching.borrow().send_ink(user, &mark).map(|_| Value::Null).map_err(str::to_string)
            }
            _ => Err(format!("{cmd} is not available in the web app yet")),
        }
    }

    pub(super) fn stop_all_watching(&self) {
        self.watching.borrow_mut().end_all();
    }

    async fn watch(self: &Rc<Self>, feed: Feed, server: ServerId, user: UserId) -> Result<(), String> {
        let (me, _) = self.me_and_key().ok_or("sign in first")?;
        let (channel, theirs) = {
            let client = self.client.borrow();
            let servers = client.servers.lock().unwrap();
            let state = servers.get(&server).ok_or("unknown server")?;
            let shares = match feed {
                Feed::Screen => &state.screen_shares,
                Feed::Camera => &state.cameras,
            };
            let channel = *shares.get(&user).ok_or(feed.off())?;
            (channel, state.voice_node(user).ok_or("that member has no reachable identity")?)
        };
        let Some(generation) = self.watching.borrow_mut().begin(feed, user) else { return Ok(()) };
        let dialer = {
            let node = self.node.lock().await;
            node.as_ref().map(|node| {
                node.reach_via_relays(std::slice::from_ref(&theirs));
                node.media_dialer()
            })
        };
        let dialed = match dialer {
            Some(dialer) => dialer.connect(&theirs, feed.hello(me.id, channel)).await.map_err(|e| e.to_string()),
            None => Err("network is not ready yet".to_string()),
        };
        let media = match dialed {
            Ok(media) => media,
            Err(e) => {
                self.watching.borrow_mut().end(feed, user, Some(generation));
                return Err(e);
            }
        };
        let commy_net::node::MediaChannel { incoming, outgoing, connection, .. } = media;
        let hangup: crate::watching::Hangup = Box::new(move || connection.close(0u32.into(), b"stopped watching"));
        if !self.watching.borrow_mut().connected(feed, user, generation, hangup, outgoing.clone()) {
            return Ok(());
        }
        if feed == Feed::Screen {
            let _ = outgoing.send(MediaPacket::StreamAudioWanted);
            let _ = outgoing.send(MediaPacket::InkReady);
        }
        let this = self.clone();
        spawn_local(async move {
            this.show_frames(feed, user, generation, incoming, outgoing).await;
            this.watching.borrow_mut().end(feed, user, Some(generation));
        });
        Ok(())
    }

    async fn show_frames(&self, feed: Feed, user: UserId, generation: u64, mut incoming: mpsc::UnboundedReceiver<MediaPacket>, _keep: mpsc::UnboundedSender<MediaPacket>) {
        let mut last_seq: Option<u64> = None;
        let mut decoder = VoiceDecoder::new();
        let speaker = format!("{STREAM_SPEAKER}{user}");
        while let Some(first) = incoming.recv().await {
            if !self.watching.borrow().current(feed, user, generation) {
                return;
            }
            let mut batch = vec![first];
            while let Ok(next) = incoming.try_recv() {
                batch.push(next);
            }
            for packet in newest_frame_with_all_audio(batch) {
                match packet {
                    MediaPacket::Video { seq, width, height, jpeg } => {
                        if last_seq.is_some_and(|last| seq <= last) {
                            continue;
                        }
                        last_seq = Some(seq);
                        let seq = (feed == Feed::Screen).then_some(seq);
                        self.emit(feed.event(), frame_payload(user, seq, width, height, &jpeg));
                    }
                    MediaPacket::StreamAudio { data, .. } if feed == Feed::Screen => {
                        let pcm = decoder.decode(&data);
                        if !self.voice_deafened() {
                            self.host.play_voice(&speaker, &pcm);
                        }
                    }
                    MediaPacket::InkAllowed { on } if feed == Feed::Screen => {
                        self.watching.borrow_mut().allow_ink(user, generation, on);
                        self.emit("stream-ink-allowed", json!({ "sharer": user.to_string(), "on": on }));
                    }
                    MediaPacket::InkFrom { user: from, mark } if feed == Feed::Screen && mark.is_valid() => {
                        if self.dms.borrow().safety.is_blocked(from) {
                            continue;
                        }
                        let name = {
                            let client = self.client.borrow();
                            let servers = client.servers.lock().unwrap();
                            crate::dm_book::known_name(&self.dms.borrow().store, &servers, from)
                        };
                        let view = json!({ "sharer": user.to_string(), "own": false, "from": from.to_string(), "name": name, "mark": mark_view(&mark) });
                        self.emit("stream-ink", view);
                    }
                    _ => {}
                }
            }
        }
    }
}

