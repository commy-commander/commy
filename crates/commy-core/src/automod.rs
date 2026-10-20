use crate::ids::{RoleId, UserId};
use crate::message::Message;
use crate::permissions::Permissions;
use crate::server::Server;
use chrono::Duration;
use serde::{Deserialize, Serialize};

pub const MAX_BLOCKED_TERMS: usize = 100;
pub const MAX_TERM_BYTES: usize = 40;
pub const MAX_TERMS_BYTES: usize = 1500;
pub const MAX_EXEMPT_ROLES: usize = 20;
pub const MENTION_LIMIT_RANGE: std::ops::RangeInclusive<u8> = 1..=50;
pub const BURST_MESSAGES_RANGE: std::ops::RangeInclusive<u8> = 2..=30;
pub const BURST_SECONDS_RANGE: std::ops::RangeInclusive<u16> = 5..=300;

const PREFIX_WILDCARD: char = '*';
const LINK_MARKERS: [&str; 3] = ["http://", "https://", "www."];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Burst {
    pub messages: u8,
    pub seconds: u16,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoMod {
    #[serde(default)]
    pub blocked_terms: Vec<String>,
    #[serde(default)]
    pub block_links: bool,
    #[serde(default)]
    pub max_mentions: Option<u8>,
    #[serde(default)]
    pub burst: Option<Burst>,
    #[serde(default)]
    pub exempt_roles: Vec<RoleId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hold {
    BlockedTerm,
    Link,
    Mentions,
    Burst,
}

impl Hold {
    pub fn describe(self) -> &'static str {
        match self {
            Hold::BlockedTerm => "blocked word",
            Hold::Link => "link",
            Hold::Mentions => "too many mentions",
            Hold::Burst => "posting too fast",
        }
    }

    pub fn refusal(self) -> &'static str {
        match self {
            Hold::BlockedTerm => "this server blocks one of the words in that message",
            Hold::Link => "this server does not allow links in messages",
            Hold::Mentions => "that message mentions too many people for this server",
            Hold::Burst => "you are posting too fast for this server; wait a few seconds",
        }
    }
}

impl AutoMod {
    pub fn is_off(&self) -> bool {
        self.blocked_terms.is_empty() && !self.block_links && self.max_mentions.is_none() && self.burst.is_none()
    }

    pub fn tidied(mut self) -> Self {
        let mut seen = std::collections::HashSet::new();
        self.blocked_terms = self
            .blocked_terms
            .iter()
            .filter_map(|term| normalize_term(term))
            .filter(|term| seen.insert(term.clone()))
            .collect();
        let mut roles = std::collections::HashSet::new();
        self.exempt_roles.retain(|r| roles.insert(*r));
        self
    }

    pub fn valid(&self) -> bool {
        let terms_fit = self.blocked_terms.len() <= MAX_BLOCKED_TERMS
            && self.blocked_terms.iter().map(String::len).sum::<usize>() <= MAX_TERMS_BYTES
            && self
                .blocked_terms
                .iter()
                .all(|t| t.len() <= MAX_TERM_BYTES && normalize_term(t).as_deref() == Some(t.as_str()));
        let mentions_fit = self.max_mentions.is_none_or(|n| MENTION_LIMIT_RANGE.contains(&n));
        let burst_fits = self
            .burst
            .is_none_or(|b| BURST_MESSAGES_RANGE.contains(&b.messages) && BURST_SECONDS_RANGE.contains(&b.seconds));
        terms_fit && mentions_fit && burst_fits && self.exempt_roles.len() <= MAX_EXEMPT_ROLES
    }

    pub fn exempt(&self, server: &Server, user: UserId) -> bool {
        moderates(server, user)
            || server
                .member(user)
                .is_some_and(|m| m.roles.iter().any(|r| self.exempt_roles.contains(r)))
    }

    pub fn content_hold(&self, content: &str) -> Option<Hold> {
        if !self.blocked_terms.is_empty() && self.has_blocked_term(content) {
            return Some(Hold::BlockedTerm);
        }
        if self.block_links && has_link(content) {
            return Some(Hold::Link);
        }
        if self.max_mentions.is_some_and(|limit| mention_count(content) > usize::from(limit)) {
            return Some(Hold::Mentions);
        }
        None
    }

    pub fn hold(&self, server: &Server, channel_messages: &[Message], message: &Message) -> Option<Hold> {
        if self.is_off() || self.exempt(server, message.author) {
            return None;
        }
        self.content_hold(&message.content).or_else(|| {
            self.burst
                .filter(|burst| over_burst(*burst, channel_messages, message))
                .map(|_| Hold::Burst)
        })
    }

    fn has_blocked_term(&self, content: &str) -> bool {
        let words = words(content);
        self.blocked_terms.iter().any(|term| {
            let pattern: Vec<&str> = term.split(' ').collect();
            words.windows(pattern.len()).any(|window| window.iter().zip(&pattern).all(|(w, p)| word_matches(w, p)))
        })
    }
}

pub fn moderates(server: &Server, user: UserId) -> bool {
    let perms = server.permissions_for(user);
    user == server.owner || perms.allows(Permissions::MANAGE_MESSAGES) || perms.allows(Permissions::MANAGE_SERVER)
}

pub fn normalize_term(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let wildcard = raw.ends_with(PREFIX_WILDCARD);
    let mut term = words(raw).join(" ");
    if term.is_empty() {
        return None;
    }
    if wildcard {
        term.push(PREFIX_WILDCARD);
    }
    Some(term)
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn word_matches(word: &str, pattern: &str) -> bool {
    match pattern.strip_suffix(PREFIX_WILDCARD) {
        Some(prefix) => word.starts_with(prefix),
        None => word == pattern,
    }
}

fn has_link(content: &str) -> bool {
    let lower = content.to_lowercase();
    LINK_MARKERS.iter().any(|marker| lower.contains(marker))
}

pub fn mention_count(content: &str) -> usize {
    let mut previous = None;
    let mut count = 0;
    let mut chars = content.chars().peekable();
    while let Some(c) = chars.next() {
        let at_word_start = previous.is_none_or(char::is_whitespace);
        if c == '@' && at_word_start && chars.peek().is_some_and(|next| next.is_alphanumeric()) {
            count += 1;
        }
        previous = Some(c);
    }
    count
}

fn over_burst(burst: Burst, channel_messages: &[Message], message: &Message) -> bool {
    let window_start = message.created_at - Duration::seconds(i64::from(burst.seconds));
    let end = channel_messages
        .iter()
        .rposition(|m| m.id == message.id)
        .unwrap_or_else(|| channel_messages.partition_point(|m| m.created_at <= message.created_at));
    let earlier = channel_messages[..end]
        .iter()
        .rev()
        .take_while(|m| m.created_at > window_start)
        .filter(|m| m.author == message.author)
        .count();
    earlier >= usize::from(burst.messages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ServerEvent;
    use crate::server::Role;
    use crate::test_support::{fixture, join, join_moderator, post, post_at};
    use chrono::Utc;

    fn terms(list: &[&str]) -> AutoMod {
        AutoMod { blocked_terms: list.iter().map(|s| s.to_string()).collect(), ..AutoMod::default() }.tidied()
    }

    #[test]
    fn terms_match_whole_words_ignoring_case_and_punctuation() {
        let rules = terms(&["Ass", "free nitro", "spam*"]);
        assert_eq!(rules.blocked_terms, vec!["ass", "free nitro", "spam*"]);
        assert_eq!(rules.content_hold("what an ASS!"), Some(Hold::BlockedTerm));
        assert_eq!(rules.content_hold("a classy pass"), None);
        assert_eq!(rules.content_hold("get FREE... nitro now"), Some(Hold::BlockedTerm));
        assert_eq!(rules.content_hold("free stuff, nitro later"), None);
        assert_eq!(rules.content_hold("total spammer"), Some(Hold::BlockedTerm));
        assert_eq!(rules.content_hold("antispam"), None);
        assert_eq!(rules.content_hold("hello"), None);
    }

    #[test]
    fn tidying_drops_blanks_and_duplicates() {
        let rules = terms(&["  ", "Bad", "bad", "!!", "two   words", "pre*"]);
        assert_eq!(rules.blocked_terms, vec!["bad", "two words", "pre*"]);
        assert!(rules.valid());
        assert_eq!(normalize_term("*"), None);
    }

    #[test]
    fn links_and_mentions() {
        let rules = AutoMod { block_links: true, max_mentions: Some(2), ..AutoMod::default() };
        assert_eq!(rules.content_hold("see https://example.com"), Some(Hold::Link));
        assert_eq!(rules.content_hold("WWW.example.com"), Some(Hold::Link));
        assert_eq!(rules.content_hold("plain text"), None);
        assert_eq!(rules.content_hold("@a @b hi"), None);
        assert_eq!(rules.content_hold("@a @b @everyone"), Some(Hold::Mentions));
        assert_eq!(mention_count("mail me at a@b.com, @ alone, @@x"), 0);
        assert_eq!(mention_count("@Some Name and\n@other"), 2);
    }

    #[test]
    fn validation_bounds_every_setting() {
        assert!(AutoMod::default().valid());
        assert!(AutoMod::default().is_off());
        let too_many = AutoMod { blocked_terms: (0..=MAX_BLOCKED_TERMS).map(|i| format!("w{i}")).collect(), ..AutoMod::default() };
        assert!(!too_many.valid());
        let too_long = AutoMod { blocked_terms: vec!["a".repeat(MAX_TERM_BYTES + 1)], ..AutoMod::default() };
        assert!(!too_long.valid());
        let raw = AutoMod { blocked_terms: vec!["Not Tidy".into()], ..AutoMod::default() };
        assert!(!raw.valid());
        assert!(raw.tidied().valid());
        assert!(!AutoMod { max_mentions: Some(0), ..AutoMod::default() }.valid());
        assert!(!AutoMod { burst: Some(Burst { messages: 1, seconds: 10 }), ..AutoMod::default() }.valid());
        assert!(!AutoMod { burst: Some(Burst { messages: 5, seconds: 1 }), ..AutoMod::default() }.valid());
        assert!(AutoMod { burst: Some(Burst { messages: 5, seconds: 10 }), ..AutoMod::default() }.valid());
        let roles = AutoMod { exempt_roles: (0..=MAX_EXEMPT_ROLES).map(|_| RoleId::new()).collect(), ..AutoMod::default() };
        assert!(!roles.valid());
    }

    #[test]
    fn owners_moderators_and_exempt_roles_are_never_held() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let member = join(&mut state, "member");
        let moderator = join_moderator(&mut state, "mod");
        let trusted = join(&mut state, "trusted");
        let role = Role { id: RoleId::new(), name: "trusted".into(), color: None, permissions: Permissions::NONE, position: 1, hoist: false, self_assignable: false, tag: None, banner: None, mentionable: false };
        let rid = role.id;
        state.apply(ServerEvent::RoleCreate { server: sid, role }).unwrap();
        state.apply(ServerEvent::RoleAssign { server: sid, user: trusted, role: rid }).unwrap();
        let rules = AutoMod { exempt_roles: vec![rid], ..terms(&["bad"]) };
        let held = |author| {
            let message = post(&state, channel, author, "bad");
            rules.hold(&state.server, &[message.clone()], &message)
        };
        assert_eq!(held(member), Some(Hold::BlockedTerm));
        assert_eq!(held(owner), None);
        assert_eq!(held(moderator), None);
        assert_eq!(held(trusted), None);
    }

    #[test]
    fn a_burst_holds_only_the_messages_past_the_limit() {
        let (mut state, channel, _) = fixture();
        let member = join(&mut state, "member");
        let other = join(&mut state, "other");
        let rules = AutoMod { burst: Some(Burst { messages: 3, seconds: 10 }), ..AutoMod::default() };
        let start = Utc::now() - Duration::minutes(5);
        let at = |s: i64| start + Duration::seconds(s);
        let list = vec![
            post_at(&state, channel, member, "1", at(0)),
            post_at(&state, channel, other, "x", at(1)),
            post_at(&state, channel, member, "2", at(1)),
            post_at(&state, channel, member, "3", at(2)),
            post_at(&state, channel, member, "4", at(3)),
            post_at(&state, channel, member, "5", at(9)),
            post_at(&state, channel, member, "6", at(30)),
        ];
        let verdicts: Vec<Option<Hold>> = list.iter().map(|m| rules.hold(&state.server, &list, m)).collect();
        assert_eq!(
            verdicts,
            vec![None, None, None, None, Some(Hold::Burst), Some(Hold::Burst), None]
        );
    }

    #[test]
    fn same_instant_messages_count_in_list_order() {
        let (mut state, channel, _) = fixture();
        let member = join(&mut state, "member");
        let rules = AutoMod { burst: Some(Burst { messages: 2, seconds: 5 }), ..AutoMod::default() };
        let now = Utc::now();
        let list: Vec<Message> = (0..3).map(|i| post_at(&state, channel, member, &i.to_string(), now)).collect();
        let held: Vec<bool> = list.iter().map(|m| rules.hold(&state.server, &list, m).is_some()).collect();
        assert_eq!(held, vec![false, false, true]);
        let unsent = post_at(&state, channel, member, "next", now);
        assert_eq!(rules.hold(&state.server, &list, &unsent), Some(Hold::Burst), "a message not stored yet counts what is");
    }
}
