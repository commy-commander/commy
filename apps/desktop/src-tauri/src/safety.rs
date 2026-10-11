//! Local safety settings: blocked users, who may DM you, message
//! requests, a simple DM spam filter, and reports sent to server owners.
//! Everything here is per device; nothing is shared with peers except
//! the report a user chooses to send.

use std::collections::{HashMap, HashSet, VecDeque};

use commy_core::UserId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DmPolicy {
    /// Anyone; strangers land in Message Requests.
    #[default]
    Everyone,
    /// Only people who share a server with you.
    ServerMembers,
    /// Only people you already talk to.
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

/// A report received as server owner.
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
    /// Copy of the reported message so it survives deletion.
    pub excerpt: String,
    pub created_at: String,
}

pub const REPORT_REASONS: &[&str] = &["spam", "harassment", "inappropriate", "impersonation", "other"];
const MAX_REPORTS: usize = 200;
const MAX_NOTE: usize = 500;
const MAX_EXCERPT: usize = 300;

/// Reports travel as a DM whose content starts with this marker.
pub const REPORT_PREFIX: &str = "[[commy-report]]";

impl Report {
    pub fn encode(&self) -> String {
        format!("{REPORT_PREFIX}{}", serde_json::to_string(self).unwrap_or_default())
    }

    /// Parses and sanity checks a report DM. The sender must be the
    /// reporter it claims to be.
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
    /// People whose DMs go straight to your inbox.
    #[serde(default)]
    pub accepted: HashSet<UserId>,
    /// Message requests flagged as likely spam.
    #[serde(default)]
    pub spam: HashSet<UserId>,
    #[serde(default = "default_true")]
    pub spam_filter: bool,
    #[serde(default)]
    pub reports: Vec<Report>,
    #[serde(default)]
    pub friends: HashSet<UserId>,
    /// Friend requests we sent and are waiting on.
    #[serde(default)]
    pub friend_out: HashSet<UserId>,
    /// Friend requests waiting for our answer.
    #[serde(default)]
    pub friend_in: HashSet<UserId>,
}

/// Friend requests and answers travel as DMs starting with this marker.
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

/// What a friend signal from `from` means for us, applied to our lists.
/// Returns what to send back, if anything.
pub fn apply_friend_signal(s: &mut SafetySettings, from: UserId, signal: FriendSignal) -> Option<FriendSignal> {
    if s.is_blocked(from) {
        return None;
    }
    match signal {
        FriendSignal::Request => {
            if s.friends.contains(&from) {
                // They lost our acceptance; confirm again.
                return Some(FriendSignal::Accept);
            }
            if s.friend_out.remove(&from) {
                // We both asked: that is a yes.
                s.friends.insert(from);
                return Some(FriendSignal::Accept);
            }
            s.friend_in.insert(from);
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
            s.friends.remove(&from);
            s.friend_in.remove(&from);
            s.friend_out.remove(&from);
            None
        }
    }
}

fn default_true() -> bool {
    true
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
            self.friends.remove(&user);
            self.friend_in.remove(&user);
            self.friend_out.remove(&user);
        } else if !blocked && had {
            self.blocked.retain(|u| *u != user);
        }
        had != blocked
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

/// What we know about a DM's sender when it arrives.
pub struct Sender {
    pub blocked: bool,
    pub accepted: bool,
    pub shares_server: bool,
}

const RATE_WINDOW_SECS: i64 = 60;
const RATE_LIMIT: usize = 5;
const REPEAT_WINDOW_SECS: i64 = 600;
const REPEAT_LIMIT: usize = 3;
const LINK_LIMIT: usize = 3;

/// Remembers recent DMs from people you have not accepted, for rate
/// limiting and repeat detection. In memory only.
#[derive(Default)]
pub struct DmFilter {
    recent: HashMap<UserId, VecDeque<(i64, u64)>>,
}

fn content_hash(content: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    content.trim().to_lowercase().hash(&mut h);
    h.finish()
}

// "host.tld/path" without a scheme, the usual shape of invite and shortener links.
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
        Sender { blocked: false, accepted: false, shares_server: true }
    }

    #[test]
    fn blocked_is_dropped_accepted_goes_to_inbox() {
        let mut f = DmFilter::default();
        let u = UserId::new();
        let blocked = Sender { blocked: true, accepted: true, shares_server: true };
        assert_eq!(f.classify(DmPolicy::Everyone, true, &blocked, u, "hi", 0), Verdict::Drop);
        let friend = Sender { blocked: false, accepted: true, shares_server: false };
        assert_eq!(f.classify(DmPolicy::Accepted, true, &friend, u, "hi", 0), Verdict::Inbox);
    }

    #[test]
    fn policy_limits_strangers() {
        let mut f = DmFilter::default();
        let u = UserId::new();
        assert_eq!(f.classify(DmPolicy::Everyone, true, &stranger(), u, "hi", 0), Verdict::Request);
        assert_eq!(f.classify(DmPolicy::Accepted, true, &stranger(), u, "hi", 0), Verdict::Drop);
        let outsider = Sender { blocked: false, accepted: false, shares_server: false };
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
        // A minute later the window has passed.
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
        assert_eq!(apply_friend_signal(&mut s, a, FriendSignal::Request), None);
        assert!(s.friend_in.contains(&a));
        // Accepting is done locally; their Accept to our request also lands.
        let b = UserId::new();
        s.friend_out.insert(b);
        apply_friend_signal(&mut s, b, FriendSignal::Accept);
        assert!(s.friends.contains(&b) && !s.friend_out.contains(&b));
        // Crossed requests become friends at once.
        let c = UserId::new();
        s.friend_out.insert(c);
        assert_eq!(apply_friend_signal(&mut s, c, FriendSignal::Request), Some(FriendSignal::Accept));
        assert!(s.friends.contains(&c));
        apply_friend_signal(&mut s, c, FriendSignal::Remove);
        assert!(!s.friends.contains(&c));
        // Blocked people cannot send requests.
        let d = UserId::new();
        s.set_blocked(d, true);
        apply_friend_signal(&mut s, d, FriendSignal::Request);
        assert!(!s.friend_in.contains(&d));
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
        assert!(SafetySettings::default().spam_filter, "new installs filter spam");
    }
}
