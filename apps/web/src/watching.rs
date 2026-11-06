use std::collections::HashMap;

use commy_core::{ChannelId, UserId};
use commy_net::media::{InkMark, InkMarkInput};
use commy_net::{MediaHello, MediaPacket};
use tokio::sync::mpsc;
use serde_json::{json, Value};

pub const STREAM_SPEAKER: &str = "stream:";

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Feed {
    Screen,
    Camera,
}

impl Feed {
    pub fn hello(self, me: UserId, channel: ChannelId) -> MediaHello {
        match self {
            Feed::Screen => MediaHello::ScreenSubscribe { user: me, channel },
            Feed::Camera => MediaHello::CameraSubscribe { user: me, channel },
        }
    }

    pub fn event(self) -> &'static str {
        match self {
            Feed::Screen => "video-frame",
            Feed::Camera => "camera-frame",
        }
    }

    pub fn off(self) -> &'static str {
        match self {
            Feed::Screen => "that user is not sharing",
            Feed::Camera => "that camera is off",
        }
    }
}

pub type Hangup = Box<dyn FnOnce()>;

struct Watch {
    generation: u64,
    hangup: Option<Hangup>,
    out: Option<mpsc::UnboundedSender<MediaPacket>>,
    ink_allowed: bool,
}

#[derive(Default)]
pub struct Watching {
    feeds: HashMap<(Feed, UserId), Watch>,
    generation: u64,
}

impl Watching {
    pub fn begin(&mut self, feed: Feed, user: UserId) -> Option<u64> {
        if self.feeds.contains_key(&(feed, user)) {
            return None;
        }
        self.generation += 1;
        self.feeds.insert((feed, user), Watch { generation: self.generation, hangup: None, out: None, ink_allowed: false });
        Some(self.generation)
    }

    pub fn connected(&mut self, feed: Feed, user: UserId, generation: u64, hangup: Hangup, out: mpsc::UnboundedSender<MediaPacket>) -> bool {
        match self.feeds.get_mut(&(feed, user)).filter(|w| w.generation == generation) {
            Some(watch) => {
                watch.hangup = Some(hangup);
                watch.out = Some(out);
                true
            }
            None => {
                hangup();
                false
            }
        }
    }

    pub fn allow_ink(&mut self, sharer: UserId, generation: u64, on: bool) {
        if let Some(watch) = self.feeds.get_mut(&(Feed::Screen, sharer)).filter(|w| w.generation == generation) {
            watch.ink_allowed = on;
        }
    }

    pub fn send_ink(&self, sharer: UserId, input: &InkMarkInput) -> Result<(), &'static str> {
        let watch = self.feeds.get(&(Feed::Screen, sharer)).ok_or("you are not watching that stream")?;
        if !watch.ink_allowed {
            return Err("drawing is not allowed on this stream");
        }
        let mark: InkMark = commy_net::media::mark_from(input).ok_or("that drawing could not be sent")?;
        let out = watch.out.as_ref().ok_or("you are not watching that stream")?;
        out.send(MediaPacket::Ink { mark }).map_err(|_| "the stream ended")
    }

    pub fn current(&self, feed: Feed, user: UserId, generation: u64) -> bool {
        self.feeds.get(&(feed, user)).is_some_and(|w| w.generation == generation)
    }

    pub fn end(&mut self, feed: Feed, user: UserId, generation: Option<u64>) {
        let Some(watch) = self.feeds.get(&(feed, user)) else { return };
        if generation.is_some_and(|g| g != watch.generation) {
            return;
        }
        if let Some(hangup) = self.feeds.remove(&(feed, user)).and_then(|watch| watch.hangup) {
            hangup();
        }
    }

    pub fn end_all(&mut self) {
        for hangup in self.feeds.drain().filter_map(|(_, watch)| watch.hangup) {
            hangup();
        }
    }
}

pub fn frame_payload(user: UserId, seq: Option<u64>, width: u32, height: u32, jpeg: &[u8]) -> Value {
    let mut payload = json!({
        "user": user.to_string(),
        "width": width,
        "height": height,
        "jpeg": data_encoding::BASE64.encode(jpeg),
    });
    if let Some(seq) = seq {
        payload["seq"] = json!(seq);
    }
    payload
}

pub fn newest_frame_with_all_audio(batch: Vec<MediaPacket>) -> Vec<MediaPacket> {
    let newest = batch.iter().rposition(MediaPacket::is_video);
    batch.into_iter().enumerate().filter(|(i, p)| !p.is_video() || Some(*i) == newest).map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(seq: u64) -> MediaPacket {
        MediaPacket::Video { seq, width: 4, height: 3, jpeg: vec![0xff, 0xd8, 0xff] }
    }

    fn audio(seq: u32) -> MediaPacket {
        MediaPacket::StreamAudio { seq, data: vec![1] }
    }

    #[test]
    fn a_backlog_shows_only_its_newest_picture_and_keeps_every_bit_of_sound() {
        let kept = newest_frame_with_all_audio(vec![video(1), audio(1), video(2), audio(2), video(3)]);
        assert_eq!(kept, vec![audio(1), audio(2), video(3)]);
        assert_eq!(newest_frame_with_all_audio(vec![audio(7)]), vec![audio(7)]);
    }

    #[test]
    fn frames_reach_the_page_in_the_shape_the_desktop_sends() {
        let user = UserId::new();
        let payload = frame_payload(user, Some(9), 640, 360, &[0xff, 0xd8, 0xff]);
        assert_eq!(payload["user"], user.to_string());
        assert_eq!(payload["seq"], 9);
        assert_eq!(payload["width"], 640);
        assert_eq!(payload["jpeg"], "/9j/");
        assert!(frame_payload(user, None, 1, 1, &[]).get("seq").is_none());
    }

    #[test]
    fn watching_someone_twice_dials_once_and_a_stopped_watch_ignores_its_late_connection() {
        let mut watching = Watching::default();
        let user = UserId::new();
        let first = watching.begin(Feed::Screen, user).unwrap();
        assert!(watching.begin(Feed::Screen, user).is_none());
        assert!(watching.begin(Feed::Camera, user).is_some());
        assert!(watching.current(Feed::Screen, user, first));
        watching.end(Feed::Screen, user, None);
        assert!(!watching.current(Feed::Screen, user, first));
        let second = watching.begin(Feed::Screen, user).unwrap();
        watching.end(Feed::Screen, user, Some(first));
        assert!(watching.current(Feed::Screen, user, second));
        let hung = std::rc::Rc::new(std::cell::Cell::new(0));
        let count = hung.clone();
        let (out, _rx) = mpsc::unbounded_channel();
        assert!(watching.connected(Feed::Screen, user, second, Box::new(move || count.set(count.get() + 1)), out.clone()));
        let late = hung.clone();
        assert!(!watching.connected(Feed::Screen, user, first, Box::new(move || late.set(late.get() + 10)), out));
        assert_eq!(hung.get(), 10);
        watching.end_all();
        assert!(!watching.current(Feed::Screen, user, second));
        assert_eq!(hung.get(), 11);
    }

    #[test]
    fn drawing_reaches_the_streamer_only_once_they_allow_it() {
        let mut watching = Watching::default();
        let sharer = UserId::new();
        let point = InkMarkInput { kind: "point".into(), stroke: 0, points: vec![[0.5, 0.5]] };
        assert_eq!(watching.send_ink(sharer, &point), Err("you are not watching that stream"));
        let generation = watching.begin(Feed::Screen, sharer).unwrap();
        let (out, mut rx) = mpsc::unbounded_channel();
        assert!(watching.connected(Feed::Screen, sharer, generation, Box::new(|| {}), out));
        assert_eq!(watching.send_ink(sharer, &point), Err("drawing is not allowed on this stream"));
        watching.allow_ink(sharer, generation + 1, true);
        assert_eq!(watching.send_ink(sharer, &point), Err("drawing is not allowed on this stream"));
        watching.allow_ink(sharer, generation, true);
        let scribble = InkMarkInput { kind: "scribble".into(), stroke: 0, points: vec![] };
        assert_eq!(watching.send_ink(sharer, &scribble), Err("that drawing could not be sent"));
        watching.send_ink(sharer, &point).unwrap();
        assert!(matches!(rx.try_recv(), Ok(MediaPacket::Ink { mark: InkMark::Point { .. } })));
    }
}
