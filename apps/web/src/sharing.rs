use commy_core::{ChannelId, ServerId, UserId};
use commy_net::MediaPacket;
use tokio::sync::mpsc::UnboundedSender;

use crate::watching::Feed;

pub const MAX_SCREEN_FRAME_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_CAMERA_FRAME_BYTES: usize = 400 * 1024;
pub const MAX_FRAME_SIDE: u32 = 4096;
pub const NOT_SHARING: &str = "not sharing";
pub const CAMERA_OFF: &str = "camera is off";

pub fn frame_limit(feed: Feed) -> usize {
    match feed {
        Feed::Screen => MAX_SCREEN_FRAME_BYTES,
        Feed::Camera => MAX_CAMERA_FRAME_BYTES,
    }
}

pub fn check_frame(feed: Feed, jpeg: &[u8]) -> Result<(), String> {
    if jpeg.len() > frame_limit(feed) || !jpeg.starts_with(&[0xff, 0xd8, 0xff]) {
        return Err(match feed {
            Feed::Screen => "screen frame must be a JPEG under 2 MB".into(),
            Feed::Camera => "camera frame must be a JPEG under 400 KB".into(),
        });
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub jpeg: Vec<u8>,
}

struct Viewer<K> {
    id: u64,
    user: UserId,
    out: UnboundedSender<MediaPacket>,
    _keep: K,
}

pub struct Broadcast<K> {
    pub server: ServerId,
    pub channel: ChannelId,
    pub generation: u64,
    viewers: Vec<Viewer<K>>,
    next_viewer: u64,
    seq: u64,
    latest: Option<Frame>,
}

impl<K> Broadcast<K> {
    pub fn new(server: ServerId, channel: ChannelId, generation: u64) -> Self {
        Self { server, channel, generation, viewers: Vec::new(), next_viewer: 0, seq: 0, latest: None }
    }

    pub fn add_viewer(&mut self, user: UserId, out: UnboundedSender<MediaPacket>, keep: K) -> (u64, Option<K>) {
        if let Some(frame) = &self.latest {
            let _ = out.send(MediaPacket::Video { seq: self.seq.saturating_sub(1), width: frame.width, height: frame.height, jpeg: frame.jpeg.clone() });
        }
        let old = self.viewers.iter().position(|v| v.user == user).map(|at| self.viewers.remove(at)._keep);
        self.next_viewer += 1;
        self.viewers.push(Viewer { id: self.next_viewer, user, out, _keep: keep });
        (self.next_viewer, old)
    }

    pub fn drop_viewer(&mut self, id: u64) -> Option<K> {
        let at = self.viewers.iter().position(|v| v.id == id)?;
        Some(self.viewers.remove(at)._keep)
    }

    pub fn push(&mut self, width: u32, height: u32, jpeg: Vec<u8>) -> usize {
        let (width, height) = (width.min(MAX_FRAME_SIDE), height.min(MAX_FRAME_SIDE));
        let packet = MediaPacket::Video { seq: self.seq, width, height, jpeg: jpeg.clone() };
        self.seq += 1;
        self.viewers.retain(|v| v.out.send(packet.clone()).is_ok());
        self.latest = Some(Frame { width, height, jpeg });
        self.viewers.len()
    }

    pub fn viewers(&self) -> usize {
        self.viewers.len()
    }

    pub fn frames(&self) -> u64 {
        self.seq
    }

    pub fn latest(&self) -> Option<&Frame> {
        self.latest.as_ref()
    }

    pub fn into_keeps(self) -> Vec<K> {
        self.viewers.into_iter().map(|v| v._keep).collect()
    }
}

pub fn feed_of(hello: &commy_net::MediaHello) -> Option<(Feed, UserId, ChannelId)> {
    match *hello {
        commy_net::MediaHello::ScreenSubscribe { user, channel } => Some((Feed::Screen, user, channel)),
        commy_net::MediaHello::CameraSubscribe { user, channel } => Some((Feed::Camera, user, channel)),
        commy_net::MediaHello::Voice { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    const JPEG: [u8; 4] = [0xff, 0xd8, 0xff, 0xe0];

    fn video(packet: MediaPacket) -> (u64, u32, u32, Vec<u8>) {
        match packet {
            MediaPacket::Video { seq, width, height, jpeg } => (seq, width, height, jpeg),
            other => panic!("not a frame: {other:?}"),
        }
    }

    #[test]
    fn viewers_get_every_frame_and_a_new_viewer_starts_with_the_latest_one() {
        let mut share: Broadcast<()> = Broadcast::new(ServerId::new(), ChannelId::new(), 1);
        let (first_tx, mut first) = mpsc::unbounded_channel();
        share.add_viewer(UserId::new(), first_tx, ());
        assert_eq!(share.push(1280, 720, JPEG.to_vec()), 1);
        assert_eq!(video(first.try_recv().unwrap()), (0, 1280, 720, JPEG.to_vec()));
        let (late_tx, mut late) = mpsc::unbounded_channel();
        share.add_viewer(UserId::new(), late_tx, ());
        assert_eq!(video(late.try_recv().unwrap()).0, 0);
        assert_eq!(share.push(9000, 9000, JPEG.to_vec()), 2);
        assert_eq!(video(late.try_recv().unwrap()), (1, MAX_FRAME_SIDE, MAX_FRAME_SIDE, JPEG.to_vec()));
        assert_eq!(share.frames(), 2);
    }

    #[test]
    fn a_viewer_who_left_or_came_back_is_counted_once() {
        let mut share: Broadcast<u8> = Broadcast::new(ServerId::new(), ChannelId::new(), 1);
        let user = UserId::new();
        let (tx, rx) = mpsc::unbounded_channel();
        let (first, none) = share.add_viewer(user, tx, 1);
        assert_eq!(none, None);
        let (again, _kept) = mpsc::unbounded_channel();
        let (second, replaced) = share.add_viewer(user, again, 2);
        assert_eq!(replaced, Some(1));
        assert_eq!(share.drop_viewer(first), None, "the replaced connection ending leaves the new one watching");
        assert_eq!(share.viewers(), 1);
        drop(rx);
        let (gone_tx, gone) = mpsc::unbounded_channel();
        share.add_viewer(UserId::new(), gone_tx, 3);
        drop(gone);
        assert_eq!(share.push(10, 10, JPEG.to_vec()), 1);
        assert_eq!(share.drop_viewer(second), Some(2));
        assert_eq!(share.viewers(), 0);
    }

    #[test]
    fn only_jpeg_frames_within_the_limit_are_sent() {
        assert!(check_frame(Feed::Camera, &JPEG).is_ok());
        assert!(check_frame(Feed::Camera, b"\x89PNG").is_err());
        let big = [JPEG.to_vec(), vec![0; MAX_CAMERA_FRAME_BYTES]].concat();
        assert!(check_frame(Feed::Camera, &big).is_err());
        assert!(check_frame(Feed::Screen, &big).is_ok());
    }
}
