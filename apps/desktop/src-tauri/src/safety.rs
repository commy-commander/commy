use std::collections::{HashMap, HashSet, VecDeque};

use chrono::{DateTime, Utc};
use commy_core::{MessageId, UserId};
use serde::{Deserialize, Serialize};

use crate::state::default_true;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DmPolicy {
    #[default]
    Everyone,
    ServerMembers,
    Accepted,
}

impl DmPolicy {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "everyone" => Some(Self::Everyone),
            "server_members" => Some(Self::ServerMembers),
            "accepted" => Some(Self::Accepted),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProfilePolicy {
    #[default]
    Everyone,
    SharedServers,
    Friends,
}

impl ProfilePolicy {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "everyone" => Some(Self::Everyone),
            "shared_servers" => Some(Self::SharedServers),
            "friends" => Some(Self::Friends),
            _ => None,
        }
    }

    pub fn shows_details_to(self, friend: bool, shares_server: bool) -> bool {
        match self {
            Self::Everyone => true,
            Self::SharedServers => friend || shares_server,
            Self::Friends => friend,
        }
    }

    pub fn shows_details_to_servers(self) -> bool {
        self != Self::Friends
    }
}

pub const MAX_FRIEND_NOTE_CHARS: usize = 200;

pub fn clean_friend_note(raw: &str) -> Option<String> {
    let text: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let text: String = text.chars().filter(|c| !c.is_control()).take(MAX_FRIEND_NOTE_CHARS).collect();
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub id: String,
    pub server: String,
    pub channel: Option<String>,
    pub message: Option<String>,
    pub reporter: String,
    pub reporter_name: String,
    pub reported: String,
    pub reported_name: String,
    pub reason: String,
    pub note: String,
    pub excerpt: String,
    pub created_at: String,
}

pub const REPORT_REASONS: &[&str] = &["spam", "harassment", "inappropriate", "impersonation", "other"];
const MAX_REPORTS: usize = 200;
const MAX_NOTE: usize = 500;
const MAX_EXCERPT: usize = 300;

pub const REPORT_PREFIX: &str = "[[commy-report]]";

impl Report {
    pub fn encode(&self) -> String {
        format!("{REPORT_PREFIX}{}", serde_json::to_string(self).unwrap_or_default())
    }

    pub fn decode(content: &str, sender: UserId) -> Option<Self> {
        let mut report: Report = serde_json::from_str(content.strip_prefix(REPORT_PREFIX)?).ok()?;
        if report.reporter != sender.to_string() || !REPORT_REASONS.contains(&report.reason.as_str()) {
            return None;
        }
        report.note = truncate(&report.note, MAX_NOTE);
        report.excerpt = truncate(&report.excerpt, MAX_EXCERPT);
        report.reporter_name = truncate(&report.reporter_name, 64);
        report.reported_name = truncate(&report.reported_name, 64);
        Some(report)
    }
}

pub fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetySettings {
    #[serde(default)]
    pub blocked: Vec<UserId>,
    #[serde(default)]
    pub dm_policy: DmPolicy,
    #[serde(default)]
    pub accepted: HashSet<UserId>,
    #[serde(default)]
    pub spam: HashSet<UserId>,
    #[serde(default = "default_true")]
    pub spam_filter: bool,
    #[serde(default)]
    pub reports: Vec<Report>,
    #[serde(default)]
    pub friends: HashSet<UserId>,
    #[serde(default)]
    pub friend_out: HashSet<UserId>,
    #[serde(default)]
    pub friend_in: HashSet<UserId>,
    #[serde(default)]
    pub friend_signals: HashMap<UserId, SignalSeen>,
    #[serde(default)]
    pub profile_policy: ProfilePolicy,
    #[serde(default)]
    pub friend_notes: HashMap<UserId, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalSeen {
    pub id: MessageId,
    pub at: DateTime<Utc>,
}

pub const FRIEND_PREFIX: &str = "[[commy-friend]]";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FriendSignal {
    Request,
    Accept,
    Decline,
    Remove,
}

impl FriendSignal {
    pub fn encode(self) -> String {
        format!("{FRIEND_PREFIX}{}", serde_json::to_string(&self).unwrap_or_default())
    }

    pub fn decode(content: &str) -> Option<Self> {
        serde_json::from_str(content.strip_prefix(FRIEND_PREFIX)?).ok()
    }
}

pub fn note_friend_signal(s: &mut SafetySettings, from: UserId, id: MessageId, at: DateTime<Utc>) -> bool {
    if let Some(last) = s.friend_signals.get(&from) {
        if last.id == id || at < last.at {
            return false;
        }
    }
    s.friend_signals.insert(from, SignalSeen { id, at });
    true
}

pub fn apply_friend_signal(s: &mut SafetySettings, from: UserId, signal: FriendSignal, note: Option<&str>) -> Option<FriendSignal> {
    if s.is_blocked(from) {
        return None;
    }
    s.friend_notes.remove(&from);
    match signal {
        FriendSignal::Request => {
            if s.friends.contains(&from) {
                return Some(FriendSignal::Accept);
            }
            if s.friend_out.remove(&from) {
                s.friends.insert(from);
                return Some(FriendSignal::Accept);
            }
            s.friend_in.insert(from);
            if let Some(note) = note.and_then(clean_friend_note) {
                s.friend_notes.insert(from, note);
            }
            None
        }
        FriendSignal::Accept => {
            if s.friend_out.remove(&from) || s.friend_in.remove(&from) {
                s.friends.insert(from);
            }
            None
        }
        FriendSignal::Decline => {
            s.friend_out.remove(&from);
            None
        }
        FriendSignal::Remove => {
            s.forget_friend(from);
            None
        }
    }
}

impl Default for SafetySettings {
    fn default() -> Self {
        Self {
            blocked: Vec::new(),
            dm_policy: DmPolicy::default(),
            accepted: HashSet::new(),
            spam: HashSet::new(),
            spam_filter: true,
            reports: Vec::new(),
            friends: HashSet::new(),
            friend_out: HashSet::new(),
            friend_in: HashSet::new(),
            friend_signals: HashMap::new(),
            profile_policy: ProfilePolicy::default(),
            friend_notes: HashMap::new(),
        }
    }
}

impl SafetySettings {
    pub fn is_blocked(&self, user: UserId) -> bool {
        self.blocked.contains(&user)
    }

    pub fn set_blocked(&mut self, user: UserId, blocked: bool) -> bool {
        let had = self.is_blocked(user);
        if blocked && !had {
            self.blocked.push(user);
            self.accepted.remove(&user);
            self.forget_friend(user);
        } else if !blocked && had {
            self.blocked.retain(|u| *u != user);
        }
        had != blocked
    }

    pub fn forget_friend(&mut self, user: UserId) {
        self.friends.remove(&user);
        self.friend_in.remove(&user);
        self.friend_out.remove(&user);
        self.friend_notes.remove(&user);
    }

    pub fn shows_details_to(&self, viewer: UserId, shares_server: bool) -> bool {
        self.profile_policy.shows_details_to(self.friends.contains(&viewer), shares_server)
    }

    pub fn add_report(&mut self, report: Report) -> bool {
        if self.reports.iter().any(|r| r.id == report.id) {
            return false;
        }
        self.reports.insert(0, report);
        self.reports.truncate(MAX_REPORTS);
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Inbox,
    Request,
    Spam,
    Drop,
}

pub struct Sender {
    pub blocked: bool,
    pub accepted: bool,
    pub shares_server: bool,
    pub node: [u8; 32],
}

const RATE_WINDOW_SECS: i64 = 60;
const RATE_LIMIT: usize = 5;
const MAX_TRACKED: usize = 4096;
const REPEAT_WINDOW_SECS: i64 = 600;
const REPEAT_LIMIT: usize = 3;
const LINK_LIMIT: usize = 3;

#[derive(Default)]
pub struct DmFilter {
    recent: HashMap<UserId, VecDeque<(i64, u64)>>,
    recent_nodes: HashMap<[u8; 32], VecDeque<i64>>,
}

fn content_hash(content: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    content.trim().to_lowercase().hash(&mut h);
    h.finish()
}

fn is_bare_link(word: &str) -> bool {
    word.split_once('/').is_some_and(|(host, _)| {
        let tld = host.rsplit('.').next().unwrap_or("");
        host.contains('.')
            && !host.starts_with('.')
            && tld.len() >= 2
            && tld.chars().all(|c| c.is_ascii_alphabetic())
            && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    })
}

pub fn link_count(content: &str) -> usize {
    content
        .split_whitespace()
        .filter(|w| {
            let w = w.to_ascii_lowercase();
            w.starts_with("http://") || w.starts_with("https://") || w.starts_with("www.") || is_bare_link(&w)
        })
        .count()
}

impl DmFilter {
    pub fn classify(&mut self, policy: DmPolicy, spam_filter: bool, sender: &Sender, from: UserId, content: &str, now: i64) -> Verdict {
        if sender.blocked {
            return Verdict::Drop;
        }
        if sender.accepted {
            return Verdict::Inbox;
        }
        match policy {
            DmPolicy::Accepted => return Verdict::Drop,
            DmPolicy::ServerMembers if !sender.shares_server => return Verdict::Drop,
            _ => {}
        }
        if self.recent_nodes.len() >= MAX_TRACKED {
            self.recent_nodes.retain(|_, h| h.back().is_some_and(|t| now - t <= RATE_WINDOW_SECS));
        }
        if self.recent.len() >= MAX_TRACKED {
            self.recent.retain(|_, h| h.back().is_some_and(|(t, _)| now - t <= REPEAT_WINDOW_SECS));
        }
        let by_node = self.recent_nodes.entry(sender.node).or_default();
        while by_node.front().is_some_and(|t| now - t > RATE_WINDOW_SECS) {
            by_node.pop_front();
        }
        if by_node.len() >= RATE_LIMIT {
            return Verdict::Drop;
        }
        by_node.push_back(now);
        let history = self.recent.entry(from).or_default();
        while history.front().is_some_and(|(t, _)| now - t > REPEAT_WINDOW_SECS) {
            history.pop_front();
        }
        let in_last_minute = history.iter().filter(|(t, _)| now - t <= RATE_WINDOW_SECS).count();
        if in_last_minute >= RATE_LIMIT {
            return Verdict::Drop;
        }
        let hash = content_hash(content);
        let repeats = history.iter().filter(|(_, h)| *h == hash).count();
        history.push_back((now, hash));
        if spam_filter && (repeats + 1 >= REPEAT_LIMIT || link_count(content) >= LINK_LIMIT) {
            return Verdict::Spam;
        }
        Verdict::Request
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stranger() -> Sender {
        Sender { blocked: false, accepted: false, shares_server: true, node: [7; 32] }
    }

    #[test]
    fn one_node_is_rate_limited_whatever_ids_it_sends_as() {
        let mut f = DmFilter::default();
        for i in 0..RATE_LIMIT {
            assert_eq!(f.classify(DmPolicy::Everyone, false, &stranger(), UserId::new(), &format!("m{i}"), i as i64), Verdict::Request);
        }
        assert_eq!(f.classify(DmPolicy::Everyone, false, &stranger(), UserId::new(), "again", 10), Verdict::Drop);
        let other = Sender { node: [8; 32], ..stranger() };
        assert_eq!(f.classify(DmPolicy::Everyone, false, &other, UserId::new(), "hello", 10), Verdict::Request);
        assert_eq!(f.classify(DmPolicy::Everyone, false, &stranger(), UserId::new(), "later", 100), Verdict::Request);
        let friend = Sender { accepted: true, ..stranger() };
        assert_eq!(f.classify(DmPolicy::Everyone, false, &friend, UserId::new(), "hi", 100), Verdict::Inbox);
    }

    #[test]
    fn tracked_senders_stay_bounded() {
        let mut f = DmFilter::default();
        for i in 0..(MAX_TRACKED + 10) {
            let node = Sender { node: [(i % 251) as u8; 32], ..stranger() };
            f.classify(DmPolicy::Everyone, false, &node, UserId::new(), "x", i as i64 * 1000);
        }
        assert!(f.recent.len() <= MAX_TRACKED + 1);
        assert!(f.recent_nodes.len() <= MAX_TRACKED + 1);
    }

    #[test]
    fn blocked_is_dropped_accepted_goes_to_inbox() {
        let mut f = DmFilter::default();
        let u = UserId::new();
        let blocked = Sender { blocked: true, accepted: true, shares_server: true, node: [7; 32] };
        assert_eq!(f.classify(DmPolicy::Everyone, true, &blocked, u, "hi", 0), Verdict::Drop);
        let friend = Sender { blocked: false, accepted: true, shares_server: false, node: [7; 32] };
        assert_eq!(f.classify(DmPolicy::Accepted, true, &friend, u, "hi", 0), Verdict::Inbox);
    }

    #[test]
    fn policy_limits_strangers() {
        let mut f = DmFilter::default();
        let u = UserId::new();
        assert_eq!(f.classify(DmPolicy::Everyone, true, &stranger(), u, "hi", 0), Verdict::Request);
        assert_eq!(f.classify(DmPolicy::Accepted, true, &stranger(), u, "hi", 0), Verdict::Drop);
        let outsider = Sender { blocked: false, accepted: false, shares_server: false, node: [7; 32] };
        assert_eq!(f.classify(DmPolicy::ServerMembers, true, &outsider, u, "hi", 0), Verdict::Drop);
        assert_eq!(f.classify(DmPolicy::ServerMembers, true, &stranger(), u, "yo", 0), Verdict::Request);
    }

    #[test]
    fn rate_limits_unaccepted_senders() {
        let mut f = DmFilter::default();
        let u = UserId::new();
        for i in 0..RATE_LIMIT {
            assert_ne!(f.classify(DmPolicy::Everyone, false, &stranger(), u, &format!("m{i}"), i as i64), Verdict::Drop);
        }
        assert_eq!(f.classify(DmPolicy::Everyone, false, &stranger(), u, "again", 10), Verdict::Drop);
        assert_eq!(f.classify(DmPolicy::Everyone, false, &stranger(), u, "later", 100), Verdict::Request);
    }

    #[test]
    fn repeats_and_link_floods_are_spam() {
        let mut f = DmFilter::default();
        let u = UserId::new();
        assert_eq!(f.classify(DmPolicy::Everyone, true, &stranger(), u, "free gift card", 0), Verdict::Request);
        assert_eq!(f.classify(DmPolicy::Everyone, true, &stranger(), u, "FREE gift card ", 70), Verdict::Request);
        assert_eq!(f.classify(DmPolicy::Everyone, true, &stranger(), u, "free gift card", 140), Verdict::Spam);
        let v = UserId::new();
        let links = "https://a.com https://b.com www.c.com";
        assert_eq!(f.classify(DmPolicy::Everyone, true, &stranger(), v, links, 0), Verdict::Spam);
        assert_eq!(f.classify(DmPolicy::Everyone, false, &stranger(), UserId::new(), links, 0), Verdict::Request);
    }

    #[test]
    fn block_toggles_and_clears_acceptance() {
        let mut s = SafetySettings::default();
        let u = UserId::new();
        s.accepted.insert(u);
        assert!(s.set_blocked(u, true));
        assert!(!s.set_blocked(u, true));
        assert!(s.is_blocked(u) && !s.accepted.contains(&u));
        assert!(s.set_blocked(u, false));
        assert!(!s.is_blocked(u));
    }

    fn report(reporter: UserId) -> Report {
        Report {
            id: "r1".into(),
            server: "s".into(),
            channel: None,
            message: None,
            reporter: reporter.to_string(),
            reporter_name: "a".into(),
            reported: UserId::new().to_string(),
            reported_name: "b".into(),
            reason: "spam".into(),
            note: "x".repeat(900),
            excerpt: String::new(),
            created_at: String::new(),
        }
    }

    #[test]
    fn reports_roundtrip_and_check_the_sender() {
        let me = UserId::new();
        let encoded = report(me).encode();
        let decoded = Report::decode(&encoded, me).unwrap();
        assert_eq!(decoded.note.len(), MAX_NOTE);
        assert!(Report::decode(&encoded, UserId::new()).is_none(), "spoofed reporter");
        assert!(Report::decode("hello", me).is_none());
        let mut bad = report(me);
        bad.reason = "because".into();
        assert!(Report::decode(&bad.encode(), me).is_none());
    }

    #[test]
    fn reports_dedupe_and_cap() {
        let mut s = SafetySettings::default();
        let me = UserId::new();
        assert!(s.add_report(report(me)));
        assert!(!s.add_report(report(me)));
        for i in 0..MAX_REPORTS + 10 {
            s.add_report(Report { id: format!("r{i}x"), ..report(me) });
        }
        assert_eq!(s.reports.len(), MAX_REPORTS);
    }

    #[test]
    fn friend_requests_flow() {
        let mut s = SafetySettings::default();
        let a = UserId::new();
        assert_eq!(apply_friend_signal(&mut s, a, FriendSignal::Request, None), None);
        assert!(s.friend_in.contains(&a));
        let b = UserId::new();
        s.friend_out.insert(b);
        apply_friend_signal(&mut s, b, FriendSignal::Accept, None);
        assert!(s.friends.contains(&b) && !s.friend_out.contains(&b));
        let c = UserId::new();
        s.friend_out.insert(c);
        assert_eq!(apply_friend_signal(&mut s, c, FriendSignal::Request, None), Some(FriendSignal::Accept));
        assert!(s.friends.contains(&c));
        apply_friend_signal(&mut s, c, FriendSignal::Remove, None);
        assert!(!s.friends.contains(&c));
        let d = UserId::new();
        s.set_blocked(d, true);
        apply_friend_signal(&mut s, d, FriendSignal::Request, None);
        assert!(!s.friend_in.contains(&d));
    }

    #[test]
    fn a_request_note_stays_with_the_request_and_no_longer() {
        let mut s = SafetySettings::default();
        let a = UserId::new();
        let long = "x".repeat(MAX_FRIEND_NOTE_CHARS + 50);
        assert_eq!(apply_friend_signal(&mut s, a, FriendSignal::Request, Some(&format!("  we met\nat the raid  {long}"))), None);
        let note = s.friend_notes[&a].clone();
        assert!(note.starts_with("we met at the raid x"));
        assert_eq!(note.chars().count(), MAX_FRIEND_NOTE_CHARS);
        apply_friend_signal(&mut s, a, FriendSignal::Remove, None);
        assert!(s.friend_notes.is_empty(), "declined, withdrawn or accepted requests drop their note");

        let b = UserId::new();
        apply_friend_signal(&mut s, b, FriendSignal::Request, Some("hello"));
        s.friend_in.remove(&b);
        s.friends.insert(b);
        apply_friend_signal(&mut s, b, FriendSignal::Accept, Some("ignored on accept"));
        assert!(s.friend_notes.is_empty());

        let c = UserId::new();
        apply_friend_signal(&mut s, c, FriendSignal::Request, Some(" \t\n "));
        assert!(s.friend_in.contains(&c) && !s.friend_notes.contains_key(&c), "blank notes are no note");
        s.set_blocked(c, true);
        assert!(!s.friend_in.contains(&c));
        assert_eq!(clean_friend_note("a\u{7}b  c"), Some("ab c".into()));
    }

    #[test]
    fn profile_policy_decides_who_sees_details() {
        let mut s = SafetySettings::default();
        let friend = UserId::new();
        let member = UserId::new();
        let stranger = UserId::new();
        s.friends.insert(friend);
        for (policy, expect) in [
            (ProfilePolicy::Everyone, [true, true, true]),
            (ProfilePolicy::SharedServers, [true, true, false]),
            (ProfilePolicy::Friends, [true, false, false]),
        ] {
            s.profile_policy = policy;
            assert_eq!(s.shows_details_to(friend, false), expect[0], "{policy:?} friend");
            assert_eq!(s.shows_details_to(member, true), expect[1], "{policy:?} member");
            assert_eq!(s.shows_details_to(stranger, false), expect[2], "{policy:?} stranger");
        }
        assert!(!ProfilePolicy::Friends.shows_details_to_servers());
        assert!(ProfilePolicy::SharedServers.shows_details_to_servers());
        assert_eq!(ProfilePolicy::parse("shared_servers"), Some(ProfilePolicy::SharedServers));
        assert_eq!(ProfilePolicy::parse("nope"), None);
        assert_eq!(serde_json::to_string(&ProfilePolicy::SharedServers).unwrap(), "\"shared_servers\"");
    }

    #[test]
    fn replayed_friend_signals_are_ignored() {
        let mut s = SafetySettings::default();
        let friend = UserId::new();
        let t0 = Utc::now();
        let (decline, accept) = (MessageId::new(), MessageId::new());
        assert!(note_friend_signal(&mut s, friend, decline, t0));
        assert!(!note_friend_signal(&mut s, friend, decline, t0), "the same signal again");
        assert!(note_friend_signal(&mut s, friend, accept, t0 + chrono::Duration::seconds(5)));
        assert!(!note_friend_signal(&mut s, friend, decline, t0), "older than the last one");
        assert!(note_friend_signal(&mut s, UserId::new(), decline, t0));

        let json = serde_json::to_string(&s).unwrap();
        let mut loaded: SafetySettings = serde_json::from_str(&json).unwrap();
        assert!(!note_friend_signal(&mut loaded, friend, accept, t0 + chrono::Duration::seconds(5)));
        let old: SafetySettings = serde_json::from_str("{}").unwrap();
        assert!(old.friend_signals.is_empty());
    }

    #[test]
    fn friend_signals_roundtrip() {
        assert_eq!(FriendSignal::decode(&FriendSignal::Accept.encode()), Some(FriendSignal::Accept));
        assert_eq!(FriendSignal::decode("[[commy-friend]]\"nope\""), None);
        assert_eq!(FriendSignal::decode("hello"), None);
    }

    #[test]
    fn counts_links() {
        assert_eq!(link_count("see https://x.io and www.y.com and chat.gg/abc"), 3);
        assert_eq!(link_count("no links here"), 0);
        assert_eq!(link_count("either/or and 1/2 and ../up and v1.2/"), 0);
    }

    #[test]
    fn settings_default_from_empty_json() {
        let s: SafetySettings = serde_json::from_str("{}").unwrap();
        assert!(s.spam_filter && s.blocked.is_empty() && s.dm_policy == DmPolicy::Everyone);
        assert!(s.profile_policy == ProfilePolicy::Everyone && s.friend_notes.is_empty());
        assert!(SafetySettings::default().spam_filter, "new installs filter spam");
    }
}
