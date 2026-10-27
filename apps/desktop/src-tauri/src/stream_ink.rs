use commy_core::UserId;
use commy_net::media::{InkMark, MediaPacket, MAX_INK_POINTS};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::mpsc;

pub const INK_PER_SECOND: f64 = 40.0;
pub const INK_BURST: f64 = 80.0;
const SCALE: f32 = u16::MAX as f32;

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

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InkMarkView {
    pub kind: &'static str,
    pub stroke: u32,
    pub points: Vec<[f32; 2]>,
}

fn to_fraction(v: u16) -> f32 {
    v as f32 / SCALE
}

fn from_fraction(v: f32) -> u16 {
    if !v.is_finite() {
        return 0;
    }
    (v.clamp(0.0, 1.0) * SCALE).round() as u16
}

pub fn mark_view(mark: &InkMark) -> InkMarkView {
    match mark {
        InkMark::Point { x, y } => InkMarkView { kind: "point", stroke: 0, points: vec![[to_fraction(*x), to_fraction(*y)]] },
        InkMark::Line { stroke, points } => InkMarkView {
            kind: "line",
            stroke: *stroke,
            points: points.iter().map(|[x, y]| [to_fraction(*x), to_fraction(*y)]).collect(),
        },
        InkMark::Clear => InkMarkView { kind: "clear", stroke: 0, points: Vec::new() },
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct InkMarkInput {
    pub kind: String,
    #[serde(default)]
    pub stroke: u32,
    #[serde(default)]
    pub points: Vec<[f32; 2]>,
}

pub fn mark_from(input: &InkMarkInput) -> Option<InkMark> {
    let points: Vec<[u16; 2]> = input.points.iter().map(|[x, y]| [from_fraction(*x), from_fraction(*y)]).collect();
    let mark = match input.kind.as_str() {
        "point" => {
            let [x, y] = *points.first()?;
            InkMark::Point { x, y }
        }
        "line" if points.len() <= MAX_INK_POINTS => InkMark::Line { stroke: input.stroke, points },
        "clear" => InkMark::Clear,
        _ => return None,
    };
    mark.is_valid().then_some(mark)
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

    #[test]
    fn marks_convert_between_fractions_and_the_wire() {
        let input = |kind: &str, points: Vec<[f32; 2]>| InkMarkInput { kind: kind.into(), stroke: 3, points };
        assert_eq!(mark_from(&input("point", vec![[0.0, 1.0]])), Some(InkMark::Point { x: 0, y: u16::MAX }));
        assert_eq!(mark_from(&input("point", vec![[-2.0, f32::NAN]])), Some(InkMark::Point { x: 0, y: 0 }));
        assert_eq!(mark_from(&input("point", vec![])), None);
        assert_eq!(mark_from(&input("line", vec![[0.5, 0.5]; MAX_INK_POINTS + 1])), None);
        assert_eq!(mark_from(&input("line", vec![])), None);
        assert_eq!(mark_from(&input("clear", vec![])), Some(InkMark::Clear));
        assert_eq!(mark_from(&input("scribble", vec![])), None);
        let line = mark_from(&input("line", vec![[0.25, 0.75]])).unwrap();
        let view = mark_view(&line);
        assert_eq!(view.kind, "line");
        assert_eq!(view.stroke, 3);
        assert!((view.points[0][0] - 0.25).abs() < 0.001 && (view.points[0][1] - 0.75).abs() < 0.001);
    }
}
