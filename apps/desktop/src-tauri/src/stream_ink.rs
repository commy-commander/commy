use commy_core::UserId;
pub use commy_net::media::{mark_from, mark_view, InkMarkInput, InkMarkView};
use commy_net::media::{InkMark, MediaPacket};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::mpsc;

pub const INK_PER_SECOND: f64 = 40.0;
pub const INK_BURST: f64 = 80.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InkNews {
    Allowed(bool),
    Mark { from: UserId, mark: InkMark },
}

pub type InkSink = Arc<dyn Fn(InkNews) + Send + Sync>;

pub struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    pub fn new(now: Instant) -> Self {
        Self { tokens: INK_BURST, at: now }
    }

    pub fn take(&mut self, now: Instant) -> bool {
        let gained = now.saturating_duration_since(self.at).as_secs_f64() * INK_PER_SECOND;
        self.tokens = (self.tokens + gained).min(INK_BURST);
        self.at = now;
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

pub struct InkHub {
    owner: UserId,
    inner: Mutex<Hub>,
}

#[derive(Default)]
struct Hub {
    allowed: bool,
    viewers: Vec<(UserId, mpsc::UnboundedSender<MediaPacket>)>,
    blocked: HashSet<UserId>,
    drawers: Vec<UserId>,
    local: Option<InkSink>,
}

impl Hub {
    fn send_all(&mut self, packet: &MediaPacket, skip: Option<UserId>) {
        self.viewers.retain(|(user, tx)| Some(*user) == skip || tx.send(packet.clone()).is_ok());
    }

    fn tell_local(&self, news: InkNews) {
        if let Some(local) = &self.local {
            local(news);
        }
    }
}

impl InkHub {
    pub fn new(owner: UserId, allowed: bool) -> Self {
        Self { owner, inner: Mutex::new(Hub { allowed, ..Hub::default() }) }
    }

    pub fn set_local(&self, sink: InkSink) {
        self.inner.lock().unwrap().local = Some(sink);
    }

    pub fn allowed(&self) -> bool {
        self.inner.lock().unwrap().allowed
    }

    pub fn drawers(&self) -> Vec<UserId> {
        self.inner.lock().unwrap().drawers.clone()
    }

    pub fn set_allowed(&self, on: bool) {
        let mut hub = self.inner.lock().unwrap();
        if hub.allowed == on {
            return;
        }
        hub.allowed = on;
        let blocked = hub.blocked.clone();
        hub.viewers.retain(|(user, tx)| tx.send(MediaPacket::InkAllowed { on: on && !blocked.contains(user) }).is_ok());
        if !on {
            hub.drawers.clear();
            hub.tell_local(InkNews::Mark { from: self.owner, mark: InkMark::Clear });
        }
    }

    pub fn join(&self, user: UserId, tx: mpsc::UnboundedSender<MediaPacket>) {
        let mut hub = self.inner.lock().unwrap();
        let on = hub.allowed && !hub.blocked.contains(&user);
        if tx.send(MediaPacket::InkAllowed { on }).is_ok() {
            hub.viewers.retain(|(_, tx)| !tx.is_closed());
            hub.viewers.push((user, tx));
        }
    }

    pub fn draw(&self, user: UserId, mark: InkMark, bucket: &mut Bucket, now: Instant) -> bool {
        let mut hub = self.inner.lock().unwrap();
        if !hub.allowed || user == self.owner || hub.blocked.contains(&user) || !mark.is_valid() || !bucket.take(now) {
            return false;
        }
        if !hub.drawers.contains(&user) {
            hub.drawers.push(user);
        }
        hub.send_all(&MediaPacket::InkFrom { user, mark: mark.clone() }, Some(user));
        hub.tell_local(InkNews::Mark { from: user, mark });
        true
    }

    pub fn clear_all(&self) {
        let mut hub = self.inner.lock().unwrap();
        hub.send_all(&MediaPacket::InkFrom { user: self.owner, mark: InkMark::Clear }, None);
        hub.tell_local(InkNews::Mark { from: self.owner, mark: InkMark::Clear });
    }

    pub fn block(&self, user: UserId) {
        let mut hub = self.inner.lock().unwrap();
        if user == self.owner || !hub.blocked.insert(user) {
            return;
        }
        hub.drawers.retain(|u| *u != user);
        hub.send_all(&MediaPacket::InkFrom { user, mark: InkMark::Clear }, Some(user));
        for (viewer, tx) in &hub.viewers {
            if *viewer == user {
                let _ = tx.send(MediaPacket::InkAllowed { on: false });
            }
        }
        hub.tell_local(InkNews::Mark { from: user, mark: InkMark::Clear });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn viewer() -> (mpsc::UnboundedSender<MediaPacket>, mpsc::UnboundedReceiver<MediaPacket>) {
        mpsc::unbounded_channel()
    }

    fn drain(rx: &mut mpsc::UnboundedReceiver<MediaPacket>) -> Vec<MediaPacket> {
        let mut got = Vec::new();
        while let Ok(packet) = rx.try_recv() {
            got.push(packet);
        }
        got
    }

    fn point() -> InkMark {
        InkMark::Point { x: 10, y: 20 }
    }

    #[test]
    fn viewers_learn_whether_drawing_is_allowed_when_they_join_and_when_it_changes() {
        let hub = InkHub::new(UserId::new(), false);
        let (tx, mut rx) = viewer();
        hub.join(UserId::new(), tx);
        hub.set_allowed(true);
        hub.set_allowed(true);
        assert_eq!(drain(&mut rx), [MediaPacket::InkAllowed { on: false }, MediaPacket::InkAllowed { on: true }]);
    }

    #[test]
    fn marks_reach_other_viewers_and_the_streamer_but_not_the_one_drawing() {
        let owner = UserId::new();
        let hub = InkHub::new(owner, true);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let local = seen.clone();
        hub.set_local(Arc::new(move |news| local.lock().unwrap().push(news)));
        let (sam, lee) = (UserId::new(), UserId::new());
        let (sam_tx, mut sam_rx) = viewer();
        let (lee_tx, mut lee_rx) = viewer();
        hub.join(sam, sam_tx);
        hub.join(lee, lee_tx);
        drain(&mut sam_rx);
        drain(&mut lee_rx);
        let now = Instant::now();
        assert!(hub.draw(sam, point(), &mut Bucket::new(now), now));
        assert!(drain(&mut sam_rx).is_empty());
        assert_eq!(drain(&mut lee_rx), [MediaPacket::InkFrom { user: sam, mark: point() }]);
        assert_eq!(*seen.lock().unwrap(), [InkNews::Mark { from: sam, mark: point() }]);
        assert_eq!(hub.drawers(), [sam]);
    }

    #[test]
    fn nothing_is_drawn_while_drawing_is_off_or_for_bad_marks() {
        let owner = UserId::new();
        let hub = InkHub::new(owner, false);
        let now = Instant::now();
        let mut bucket = Bucket::new(now);
        let sam = UserId::new();
        assert!(!hub.draw(sam, point(), &mut bucket, now));
        hub.set_allowed(true);
        assert!(!hub.draw(sam, InkMark::Line { stroke: 1, points: vec![] }, &mut bucket, now));
        assert!(!hub.draw(owner, point(), &mut bucket, now));
        assert!(hub.draw(sam, point(), &mut bucket, now));
    }

    #[test]
    fn a_blocked_viewer_is_wiped_and_told_to_stop() {
        let owner = UserId::new();
        let hub = InkHub::new(owner, true);
        let (sam, lee) = (UserId::new(), UserId::new());
        let (sam_tx, mut sam_rx) = viewer();
        let (lee_tx, mut lee_rx) = viewer();
        hub.join(sam, sam_tx);
        hub.join(lee, lee_tx);
        let now = Instant::now();
        let mut bucket = Bucket::new(now);
        hub.draw(sam, point(), &mut bucket, now);
        drain(&mut sam_rx);
        drain(&mut lee_rx);
        hub.block(sam);
        assert_eq!(drain(&mut sam_rx), [MediaPacket::InkAllowed { on: false }]);
        assert_eq!(drain(&mut lee_rx), [MediaPacket::InkFrom { user: sam, mark: InkMark::Clear }]);
        assert!(!hub.draw(sam, point(), &mut bucket, now));
        assert!(hub.drawers().is_empty());
        hub.set_allowed(false);
        hub.set_allowed(true);
        assert_eq!(drain(&mut sam_rx), [MediaPacket::InkAllowed { on: false }, MediaPacket::InkAllowed { on: false }]);
    }

    #[test]
    fn turning_drawing_off_clears_the_streamers_screen() {
        let owner = UserId::new();
        let hub = InkHub::new(owner, true);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let local = seen.clone();
        hub.set_local(Arc::new(move |news| local.lock().unwrap().push(news)));
        hub.set_allowed(false);
        hub.clear_all();
        let clear = InkNews::Mark { from: owner, mark: InkMark::Clear };
        assert_eq!(*seen.lock().unwrap(), [clear.clone(), clear]);
    }

    #[test]
    fn the_bucket_limits_a_flood_and_refills_over_time() {
        let start = Instant::now();
        let mut bucket = Bucket::new(start);
        let allowed = (0..200).filter(|_| bucket.take(start)).count();
        assert_eq!(allowed, INK_BURST as usize);
        assert!(!bucket.take(start));
        assert!(bucket.take(start + Duration::from_millis(100)));
    }
}
