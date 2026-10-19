use crate::Bot;
use chrono::Utc;
use commy_core::catchup::{digests, items, BucketItems};
use commy_core::{
    BucketDigest, BucketKey, Identity, IdentityKey, Message, MessageId, ServerId, ServerState, TombstoneProof, UserId,
    UserProfile,
};
use commy_net::catchup::{check_peer, collect, profiles_of, ApplyReport, CatchupStore, PeerCheck};
use commy_net::WirePayload;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone, PartialEq)]
pub enum Outgoing {
    Send(ServerId, Box<WirePayload>),
    Changed(ServerId),
}

type Session = (ServerId, [u8; 32]);

#[derive(Default)]
struct Sessions {
    running: HashSet<Session>,
    finished: HashMap<Session, Instant>,
}

pub struct BotStore {
    bot: Arc<Mutex<Bot>>,
    out: UnboundedSender<Outgoing>,
    sessions: Mutex<Sessions>,
}

impl BotStore {
    pub fn new(bot: Arc<Mutex<Bot>>, out: UnboundedSender<Outgoing>) -> Self {
        Self { bot, out, sessions: Mutex::default() }
    }

    pub fn bot(&self) -> MutexGuard<'_, Bot> {
        self.bot.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn begin(&self, server: ServerId, peer: [u8; 32], unless_within: Option<Duration>) -> bool {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let recent = sessions.finished.get(&(server, peer)).zip(unless_within).is_some_and(|(at, within)| at.elapsed() < within);
        !recent && sessions.running.insert((server, peer))
    }

    pub fn finish(&self, server: ServerId, peer: [u8; 32]) {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions.running.remove(&(server, peer));
        sessions.finished.insert((server, peer), Instant::now());
    }

    fn with_state<T>(&self, server: ServerId, f: impl FnOnce(&ServerState) -> T) -> Option<T> {
        self.bot().replica(server)?.state.as_ref().map(f)
    }

    fn send(&self, server: ServerId, payloads: Vec<WirePayload>, changed: bool) {
        for payload in payloads {
            let _ = self.out.send(Outgoing::Send(server, Box::new(payload)));
        }
        if changed {
            let _ = self.out.send(Outgoing::Changed(server));
        }
    }
}

impl CatchupStore for BotStore {
    fn local_identity(&self, server: ServerId) -> Option<(UserId, Identity)> {
        let bot = self.bot();
        let me = bot.profile().id;
        let state = bot.replica(server)?.state.as_ref()?;
        state.server.is_current_member(me).then(|| (me, bot.identity().clone()))
    }

    fn verify_peer(&self, server: ServerId, user: UserId, key: &IdentityKey) -> PeerCheck {
        self.with_state(server, |s| check_peer(s, user, key)).unwrap_or(PeerCheck::Denied)
    }

    fn digests(&self, server: ServerId) -> Vec<BucketDigest> {
        self.with_state(server, |s| digests(s, Utc::now())).unwrap_or_default()
    }

    fn items(&self, server: ServerId, keys: &[BucketKey]) -> Vec<BucketItems> {
        self.with_state(server, |s| items(s, keys, Utc::now())).unwrap_or_default()
    }

    fn get(&self, server: ServerId, ids: &[MessageId]) -> (Vec<Message>, Vec<TombstoneProof>) {
        self.with_state(server, |s| collect(s, ids)).unwrap_or_default()
    }

    fn profiles(&self, server: ServerId, users: &[UserId]) -> Vec<UserProfile> {
        self.with_state(server, |s| profiles_of(s, users)).unwrap_or_default()
    }

    fn apply_profiles(&self, server: ServerId, profiles: Vec<UserProfile>) {
        let out = self.bot().on_profiles(server, profiles, Instant::now(), Utc::now());
        self.send(server, out, true);
    }

    fn apply(&self, server: ServerId, messages: Vec<Message>, tombstones: Vec<TombstoneProof>) -> ApplyReport {
        let (report, out) = self.bot().on_caught_up(server, messages, tombstones, Instant::now(), Utc::now());
        let changed = !report.applied.is_empty() || !out.is_empty();
        self.send(server, out, changed);
        report
    }
}
