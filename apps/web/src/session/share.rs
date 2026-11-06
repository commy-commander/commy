use std::collections::HashSet;
use std::rc::Rc;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use commy_core::{ChannelId, ServerEvent, ServerId};
use commy_net::node::MediaChannel;
use commy_net::MediaPacket;
use iroh::endpoint::Connection;
use serde_json::{json, Value};
use wasm_bindgen_futures::spawn_local;

use super::{parse, Args, Session};
use crate::sharing::{check_frame, feed_of, Broadcast, CAMERA_OFF, NOT_SHARING};
use crate::watching::Feed;

const SHARE_BEAT: Duration = Duration::from_secs(8);

pub(super) type ShareGate = Arc<StdMutex<HashSet<(Feed, ChannelId)>>>;

#[derive(Default)]
pub(super) struct Shares {
    screen: Option<Broadcast<Connection>>,
    camera: Option<Broadcast<Connection>>,
    generation: u64,
}

impl Shares {
    fn slot(&mut self, feed: Feed) -> &mut Option<Broadcast<Connection>> {
        match feed {
            Feed::Screen => &mut self.screen,
            Feed::Camera => &mut self.camera,
        }
    }
}

fn announce(feed: Feed, user: commy_core::UserId, channel: ChannelId, active: bool) -> ServerEvent {
    match feed {
        Feed::Screen => ServerEvent::ScreenShare { user, channel, active },
        Feed::Camera => ServerEvent::CameraState { user, channel, active },
    }
}

fn close_all(keeps: Vec<Connection>) {
    for connection in keeps {
        connection.close(0u32.into(), b"stopped sharing");
    }
}

impl Session {
    pub(super) async fn share_command(self: &Rc<Self>, cmd: &str, args: Args) -> Result<Value, String> {
        match cmd {
            "start_screen_share" => self.start_share(Feed::Screen, parse(args.server_id.as_ref(), "server")?).await,
            "stop_screen_share" => self.stop_share(Feed::Screen).await,
            "start_camera" => self.start_share(Feed::Camera, parse(args.server_id.as_ref(), "server")?).await,
            "stop_camera" => self.stop_share(Feed::Camera).await,
            "push_camera_frame" => {
                let jpeg = data_encoding::BASE64
                    .decode(args.jpeg.as_deref().unwrap_or("").as_bytes())
                    .map_err(|_| "camera frame is not valid base64".to_string())?;
                self.push_frame(Feed::Camera, args.width.unwrap_or(0), args.height.unwrap_or(0), jpeg).map(|n| json!(n))
            }
            "own_share_frame" => {
                let shares = self.shares.borrow();
                let share = shares.screen.as_ref().ok_or(NOT_SHARING)?;
                let frame = share.latest();
                Ok(json!({
                    "width": frame.map_or(0, |f| f.width),
                    "height": frame.map_or(0, |f| f.height),
                    "jpeg": frame.map(|f| data_encoding::BASE64.encode(&f.jpeg)),
                    "viewers": share.viewers(),
                    "frames": share.frames(),
                    "audio": false,
                    "ink": false,
                    "drawers": [],
                }))
            }
            _ => Err(format!("{cmd} is not available in the web app yet")),
        }
    }

    pub fn screen_frame(&self, width: u32, height: u32, jpeg: Vec<u8>) -> Result<usize, String> {
        self.push_frame(Feed::Screen, width, height, jpeg)
    }

    fn push_frame(&self, feed: Feed, width: u32, height: u32, jpeg: Vec<u8>) -> Result<usize, String> {
        check_frame(feed, &jpeg)?;
        let mut shares = self.shares.borrow_mut();
        let share = shares.slot(feed).as_mut().ok_or(match feed {
            Feed::Screen => NOT_SHARING,
            Feed::Camera => CAMERA_OFF,
        })?;
        Ok(share.push(width, height, jpeg))
    }

    async fn start_share(self: &Rc<Self>, feed: Feed, server: ServerId) -> Result<Value, String> {
        let (channel, _) = self.voice_here(server).ok_or("join a voice channel first")?;
        let payload = self.client.borrow().act(server, |_, me, _, _| Ok(announce(feed, me.user, channel, true)))?;
        let generation = {
            let mut shares = self.shares.borrow_mut();
            shares.generation += 1;
            let generation = shares.generation;
            if let Some(old) = shares.slot(feed).replace(Broadcast::new(server, channel, generation)) {
                close_all(old.into_keeps());
            }
            generation
        };
        self.share_gate.lock().unwrap().insert((feed, channel));
        self.send_voice(server, &payload).await;
        self.emit("server-event", json!(server.to_string()));
        if feed == Feed::Screen {
            spawn_local(self.clone().keep_announcing(generation));
        }
        Ok(Value::Null)
    }

    async fn keep_announcing(self: Rc<Self>, generation: u64) {
        loop {
            n0_future::time::sleep(SHARE_BEAT).await;
            let live = self.shares.borrow().screen.as_ref().filter(|s| s.generation == generation).map(|s| (s.server, s.channel));
            let Some((server, channel)) = live else { return };
            let payload = self.client.borrow().act(server, |_, me, _, _| Ok(announce(Feed::Screen, me.user, channel, true)));
            if let Ok(payload) = payload {
                self.send_voice(server, &payload).await;
            }
        }
    }

    async fn stop_share(self: &Rc<Self>, feed: Feed) -> Result<Value, String> {
        let Some((server, payload)) = self.take_share(feed) else { return Ok(Value::Null) };
        if let Some(payload) = payload {
            self.send_voice(server, &payload).await;
        }
        self.emit("server-event", json!(server.to_string()));
        Ok(Value::Null)
    }

    fn take_share(&self, feed: Feed) -> Option<(ServerId, Option<commy_net::WirePayload>)> {
        let share = self.shares.borrow_mut().slot(feed).take()?;
        let (server, channel) = (share.server, share.channel);
        self.share_gate.lock().unwrap().remove(&(feed, channel));
        close_all(share.into_keeps());
        let payload = self.client.borrow().act(server, |_, me, _, _| Ok(announce(feed, me.user, channel, false))).ok();
        Some((server, payload))
    }

    pub(super) fn end_shares(&self) {
        for feed in [Feed::Screen, Feed::Camera] {
            if let Some((server, Some(payload))) = self.take_share(feed) {
                self.publish(server, &payload);
                self.emit("server-event", json!(server.to_string()));
            }
        }
    }

    pub(super) fn add_share_viewer(self: &Rc<Self>, media: MediaChannel) {
        let Some((feed, user, channel)) = feed_of(&media.hello) else { return };
        let MediaChannel { peer, mut incoming, outgoing, connection, .. } = media;
        let node = data_encoding::HEXLOWER.encode(&peer);
        let server = self.shares.borrow_mut().slot(feed).as_ref().filter(|s| s.channel == channel).map(|s| (s.server, s.generation));
        let Some((server, generation)) = server else {
            connection.close(0u32.into(), b"not sharing");
            return;
        };
        let allowed = {
            let client = self.client.borrow();
            let servers = client.servers.lock().unwrap();
            servers.get(&server).is_some_and(|state| state.media_peer_ok(user, &node, channel, false))
        };
        if !allowed {
            connection.close(0u32.into(), b"not allowed");
            return;
        }
        if feed == Feed::Screen {
            let _ = outgoing.send(MediaPacket::InkAllowed { on: false });
        }
        let (viewer, replaced) = {
            let mut shares = self.shares.borrow_mut();
            let Some(share) = shares.slot(feed).as_mut() else { return };
            share.add_viewer(user, outgoing, connection)
        };
        if let Some(old) = replaced {
            old.close(0u32.into(), b"replaced");
        }
        let this = self.clone();
        spawn_local(async move {
            while incoming.recv().await.is_some() {}
            let mut shares = this.shares.borrow_mut();
            if let Some(share) = shares.slot(feed).as_mut().filter(|s| s.generation == generation) {
                share.drop_viewer(viewer);
            }
        });
    }
}
