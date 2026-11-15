use std::collections::{HashMap, HashSet, VecDeque};

use chrono::{DateTime, Utc};
use commy_core::{ChannelId, MessageId, ServerId, ServerState, UserId, UserProfile};
use commy_core::DmStore;

use crate::keys::KeyHolder;
use commy_sync::report::{clip, HostReport, Person, PickedMessage, MAX_MESSAGE_CHARS, MAX_NAME_CHARS, MAX_PICKED_MESSAGES, MAX_REPORTED_STREAMS};
use commy_sync::StreamId;
use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

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

pub const REPORT_REASONS: &[&str] = commy_sync::report::REASONS;
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

pub struct ReportForm {
    pub reported: UserId,
    pub channel: Option<ChannelId>,
    pub messages: Vec<MessageId>,
    pub reason: String,
    pub note: String,
}

pub struct OutgoingReport {
    pub owner: Option<(UserId, Report)>,
    pub host: HostReport,
}

fn picked_ids(form: &ReportForm) -> Result<Vec<MessageId>, String> {
    let mut picked: Vec<MessageId> = Vec::new();
    for id in &form.messages {
        if !picked.contains(id) {
            picked.push(*id);
        }
    }
    if picked.len() > MAX_PICKED_MESSAGES {
        return Err(format!("pick at most {MAX_PICKED_MESSAGES} messages"));
    }
    if !picked.is_empty() && form.channel.is_none() {
        return Err("that message is no longer available".into());
    }
    Ok(picked)
}

fn person(id: UserId, name: &str) -> Person {
    Person { id: id.to_string(), name: clip(name, MAX_NAME_CHARS) }
}

fn opened_messages(state: &ServerState, keys: &KeyHolder, channel: Option<ChannelId>, picked: &[MessageId]) -> Result<Vec<commy_core::Message>, String> {
    let Some(channel) = channel else { return Ok(Vec::new()) };
    let stored = state.messages(channel);
    picked
        .iter()
        .map(|id| stored.iter().find(|m| m.id == *id).and_then(|m| keys.open(state, m)).ok_or_else(|| "that message is no longer available".to_string()))
        .collect()
}

pub fn report(
    state: &ServerState,
    keys: &KeyHolder,
    me: &UserProfile,
    form: ReportForm,
    reported_name: String,
    streams: Vec<StreamId>,
    now: DateTime<Utc>,
) -> Result<OutgoingReport, String> {
    if !REPORT_REASONS.contains(&form.reason.as_str()) {
        return Err("pick a reason for the report".into());
    }
    if form.reported == me.id {
        return Err("you cannot report yourself".into());
    }
    let picked = picked_ids(&form)?;
    let opened = opened_messages(state, keys, form.channel, &picked)?;
    let owner = state.server.owner;
    let name_of = |user: UserId| if user == form.reported { reported_name.clone() } else { crate::display_name(state, user) };
    let channel_name = form.channel.and_then(|c| state.server.channels.iter().find(|ch| ch.id == c)).map(|ch| ch.name.clone()).unwrap_or_default();
    let note = truncate(form.note.trim(), MAX_NOTE);
    let shown = opened.iter().find(|m| m.author == form.reported).or(opened.first());
    let to_owner = (owner != me.id && form.reported != owner).then(|| {
        let report = Report {
            id: MessageId::new().to_string(),
            server: state.server.id.to_string(),
            channel: form.channel.map(|c| c.to_string()),
            message: shown.map(|m| m.id.to_string()),
            reporter: me.id.to_string(),
            reporter_name: me.visible_name().to_string(),
            reported: form.reported.to_string(),
            reported_name: reported_name.clone(),
            reason: form.reason.clone(),
            note: note.clone(),
            excerpt: truncate(shown.map(|m| m.content.as_str()).unwrap_or_default(), MAX_EXCERPT),
            created_at: now.to_rfc3339(),
        };
        (owner, report)
    });
    let host = HostReport {
        community: state.server.id.to_string(),
        community_name: clip(&state.server.name, MAX_NAME_CHARS),
        owner: person(owner, &name_of(owner)),
        reporter: person(me.id, me.visible_name()),
        reported: person(form.reported, &reported_name),
        reason: form.reason,
        note,
        messages: opened
            .iter()
            .map(|m| PickedMessage {
                id: m.id.to_string(),
                channel: clip(&channel_name, MAX_NAME_CHARS),
                author: person(m.author, &name_of(m.author)),
                at: m.created_at.to_rfc3339(),
                text: clip(&m.content, MAX_MESSAGE_CHARS),
            })
            .collect(),
        streams: streams.into_iter().take(MAX_REPORTED_STREAMS).collect(),
        sent_at: now.to_rfc3339(),
    };
    Ok(OutgoingReport { owner: to_owner, host })
}

pub fn received_report(servers: &HashMap<ServerId, ServerState>, me: UserId, sender: UserId, content: &str) -> Option<Report> {
    let report = Report::decode(content, sender)?;
    let server = servers.get(&report.server.parse::<ServerId>().ok()?)?;
    (server.server.owner == me && server.server.members.iter().any(|m| m.user == sender)).then_some(report)
}

pub use crate::truncate;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    None,
    Friend,
    Outgoing,
    Incoming,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationPiece {
    pub relation: Relation,
    pub at: i64,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub profile: Option<UserProfile>,
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

    pub fn relation(&self, user: UserId) -> Relation {
        if self.is_blocked(user) {
            Relation::Blocked
        } else if self.friends.contains(&user) {
            Relation::Friend
        } else if self.friend_out.contains(&user) {
            Relation::Outgoing
        } else if self.friend_in.contains(&user) {
            Relation::Incoming
        } else {
            Relation::None
        }
    }

    pub fn set_relation(&mut self, user: UserId, relation: Relation, note: Option<String>) -> bool {
        let before = (self.relation(user), self.friend_notes.get(&user).cloned());
        let note = note.filter(|_| relation == Relation::Incoming);
        self.set_blocked(user, relation == Relation::Blocked);
        self.forget_friend(user);
        match relation {
            Relation::Friend => {
                self.friends.insert(user);
            }
            Relation::Outgoing => {
                self.friend_out.insert(user);
            }
            Relation::Incoming => {
                self.friend_in.insert(user);
            }
            Relation::None | Relation::Blocked => {}
        }
        if let Some(note) = note.as_deref().and_then(clean_friend_note) {
            self.friend_notes.insert(user, note);
        }
        before != (self.relation(user), self.friend_notes.get(&user).cloned())
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

pub const MAX_STRANGER_REQUESTS: usize = 50;

pub fn open_stranger_requests(dms: &DmStore, me: UserId, known: impl Fn(UserId) -> bool) -> usize {
    dms.partners().into_iter().filter(|partner| !dms.has_sent_to(me, *partner) && !known(*partner)).count()
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

    #[test]
    fn a_relation_from_another_device_replaces_the_one_here() {
        let mut s = SafetySettings::default();
        let user = UserId::new();
        assert_eq!(s.relation(user), Relation::None);
        assert!(s.set_relation(user, Relation::Incoming, Some("hi from the raid".into())));
        assert_eq!(s.relation(user), Relation::Incoming);
        assert_eq!(s.friend_notes.get(&user).map(String::as_str), Some("hi from the raid"));
        assert!(s.set_relation(user, Relation::Friend, None));
        assert!(s.friends.contains(&user) && !s.friend_in.contains(&user) && s.friend_notes.is_empty());
        assert!(!s.set_relation(user, Relation::Friend, None));
        assert!(s.set_relation(user, Relation::Blocked, None));
        assert!(s.is_blocked(user) && s.friends.is_empty());
        assert!(s.set_relation(user, Relation::None, None));
        assert!(!s.is_blocked(user));
        assert_eq!(s.relation(user), Relation::None);
    }

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
    fn reports_go_to_the_owner_from_members_about_others() {
        let owner = UserProfile::new("owner");
        let (member, other) = (UserProfile::new("member"), UserProfile::new("other"));
        let mut state = ServerState::new(commy_core::Server::new("Hangout", owner.id));
        let sid = state.server.id;
        for p in [&member, &other] {
            state.apply(commy_core::ServerEvent::MemberJoin { server: sid, profile: p.clone() }).unwrap();
        }
        let keys = KeyHolder::new(None);
        let form = |reported: UserId, reason: &str| ReportForm { reported, channel: None, messages: Vec::new(), reason: reason.into(), note: "  rude  ".into() };
        let send = |me: &UserProfile, form: ReportForm| super::report(&state, &keys, me, form, "other".into(), Vec::new(), Utc::now());
        let made = send(&member, form(other.id, "spam")).unwrap();
        let (to, made) = made.owner.unwrap();
        assert_eq!((to, made.note.as_str(), made.reported.as_str()), (owner.id, "rude", other.id.to_string().as_str()));
        assert_eq!(send(&member, form(other.id, "nope")).err().unwrap(), "pick a reason for the report");
        assert_eq!(send(&member, form(member.id, "spam")).err().unwrap(), "you cannot report yourself");
        let missing = ReportForm { channel: Some(ChannelId::new()), messages: vec![MessageId::new()], ..form(other.id, "spam") };
        assert_eq!(send(&member, missing).err().unwrap(), "that message is no longer available");
        let servers = HashMap::from([(sid, state.clone())]);
        assert_eq!(received_report(&servers, owner.id, member.id, &made.encode()), Some(made.clone()));
        assert_eq!(received_report(&servers, member.id, member.id, &made.encode()), None, "only the owner takes reports");
        assert_eq!(received_report(&servers, owner.id, other.id, &made.encode()), None, "the sender must be the reporter");
    }

    #[test]
    fn every_report_also_reaches_the_sync_operator_even_about_the_owner() {
        let owner = UserProfile::new("owner");
        let member = UserProfile::new("member");
        let mut state = ServerState::new(commy_core::Server::new("Hangout", owner.id));
        let sid = state.server.id;
        state.apply(commy_core::ServerEvent::MemberJoin { server: sid, profile: member.clone() }).unwrap();
        let keys = KeyHolder::new(None);
        let stream = commy_sync::StreamSecret::random().keys().id;
        let form = |reported: UserId| ReportForm { reported, channel: None, messages: Vec::new(), reason: "inappropriate".into(), note: String::new() };
        let about_owner = super::report(&state, &keys, &member, form(owner.id), "owner".into(), vec![stream], Utc::now()).unwrap();
        assert!(about_owner.owner.is_none(), "the owner is not asked to judge themselves");
        assert_eq!(about_owner.host.reported.id, owner.id.to_string());
        assert_eq!(about_owner.host.owner.id, owner.id.to_string());
        assert_eq!(about_owner.host.community, sid.to_string());
        assert_eq!(about_owner.host.community_name, "Hangout");
        assert_eq!(about_owner.host.streams, vec![stream]);
        assert_eq!(about_owner.host.check(), Ok(()));
        let by_owner = super::report(&state, &keys, &owner, form(member.id), "member".into(), Vec::new(), Utc::now()).unwrap();
        assert!(by_owner.owner.is_none(), "an owner's report goes only to the operator");
        assert_eq!(by_owner.host.reporter.id, owner.id.to_string());
    }

    #[test]
    fn picked_messages_travel_as_the_reporter_saw_them() {
        let owner = UserProfile::new("owner");
        let (member_identity, other_identity) = (commy_core::Identity::generate(), commy_core::Identity::generate());
        let (mut member, mut other) = (UserProfile::new("member"), UserProfile::new("other"));
        member.identity = Some(member_identity.public());
        other.identity = Some(other_identity.public());
        let mut state = ServerState::new(commy_core::Server::new("Hangout", owner.id));
        let sid = state.server.id;
        for p in [&member, &other] {
            state.apply(commy_core::ServerEvent::MemberJoin { server: sid, profile: p.clone() }).unwrap();
        }
        let channel = state.server.channels.iter().find(|c| c.kind.holds_messages()).unwrap().clone();
        let said = |author: &UserProfile, identity: &commy_core::Identity, text: &str| {
            let mut message = commy_core::Message::new(channel.id, author.id, text.to_string());
            message.sign(sid, identity);
            message
        };
        let (context, insult) = (said(&member, &member_identity, "please stop"), said(&other, &other_identity, &"x".repeat(MAX_MESSAGE_CHARS + 50)));
        for m in [&context, &insult] {
            state.apply(commy_core::ServerEvent::MessageCreate { message: m.clone() }).unwrap();
        }
        let keys = KeyHolder::new(None);
        let form = ReportForm {
            reported: other.id,
            channel: Some(channel.id),
            messages: vec![context.id, insult.id, context.id],
            reason: "harassment".into(),
            note: String::new(),
        };
        let made = super::report(&state, &keys, &member, form, "other".into(), Vec::new(), Utc::now()).unwrap();
        let picked = &made.host.messages;
        assert_eq!(picked.len(), 2, "a message picked twice is sent once");
        assert_eq!((picked[0].text.as_str(), picked[0].author.name.as_str()), ("please stop", "member"));
        assert_eq!(picked[1].text.chars().count(), MAX_MESSAGE_CHARS);
        assert_eq!(picked[1].channel, channel.name);
        let (_, to_owner) = made.owner.unwrap();
        assert_eq!(to_owner.message, Some(insult.id.to_string()), "the owner sees the reported person's own message");
        assert_eq!(to_owner.excerpt.chars().count(), MAX_EXCERPT);
        let too_many = ReportForm { reported: other.id, channel: Some(channel.id), messages: (0..=MAX_PICKED_MESSAGES).map(|_| MessageId::new()).collect(), reason: "spam".into(), note: String::new() };
        assert!(super::report(&state, &keys, &member, too_many, String::new(), Vec::new(), Utc::now()).err().unwrap().starts_with("pick at most"));
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
