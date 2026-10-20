use crate::ids::UserId;
use crate::message::{Attachment, Message};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const POLL_MIME: &str = "application/vnd.commy.poll+json";
pub const MIN_OPTIONS: usize = 2;
pub const MAX_OPTIONS: usize = 10;
pub const MAX_QUESTION_CHARS: usize = 300;
pub const MAX_OPTION_CHARS: usize = 100;
pub const MAX_DURATION_HOURS: u32 = 24 * 7 * 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Poll {
    pub question: String,
    pub options: Vec<String>,
    #[serde(default)]
    pub multi: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closes_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollTally {
    pub counts: Vec<usize>,
    pub voters: usize,
    pub closed: bool,
}

pub type Ballots = BTreeMap<UserId, Vec<u8>>;

impl Poll {
    pub fn tidy(mut self) -> Option<Self> {
        self.question = squash(&self.question);
        self.options = self.options.iter().map(|o| squash(o)).filter(|o| !o.is_empty()).collect();
        let sized = |text: &str, max: usize| !text.is_empty() && text.chars().count() <= max;
        if !sized(&self.question, MAX_QUESTION_CHARS) {
            return None;
        }
        if !(MIN_OPTIONS..=MAX_OPTIONS).contains(&self.options.len()) {
            return None;
        }
        if !self.options.iter().all(|o| sized(o, MAX_OPTION_CHARS)) {
            return None;
        }
        Some(self)
    }

    pub fn into_attachment(self) -> Attachment {
        Attachment {
            blob: String::new(),
            filename: serde_json::to_string(&self).expect("plain strings encode"),
            size_bytes: 0,
            mime_type: POLL_MIME.to_string(),
            provider: None,
        }
    }

    pub fn is_open_at(&self, now: DateTime<Utc>) -> bool {
        self.closes_at.is_none_or(|t| now < t)
    }

    pub fn normalize_choices(&self, choices: &[u8]) -> Option<Vec<u8>> {
        let mut picked: Vec<u8> = choices.to_vec();
        picked.sort_unstable();
        picked.dedup();
        if picked.iter().any(|&c| c as usize >= self.options.len()) {
            return None;
        }
        if !self.multi && picked.len() > 1 {
            return None;
        }
        Some(picked)
    }

    pub fn tally(&self, ballots: &Ballots, closed: bool) -> PollTally {
        let mut counts = vec![0usize; self.options.len()];
        let mut voters = 0;
        for choices in ballots.values().filter(|c| !c.is_empty()) {
            voters += 1;
            for &choice in choices {
                if let Some(slot) = counts.get_mut(choice as usize) {
                    *slot += 1;
                }
            }
        }
        PollTally { counts, voters, closed }
    }
}

impl Attachment {
    pub fn is_poll(&self) -> bool {
        self.mime_type == POLL_MIME
    }

    pub fn poll(&self) -> Option<Poll> {
        if !self.is_poll() {
            return None;
        }
        serde_json::from_str::<Poll>(&self.filename).ok()?.tidy()
    }
}

impl Message {
    pub fn poll(&self) -> Option<Poll> {
        self.attachments.iter().find_map(Attachment::poll)
    }
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ChannelId;

    fn sample() -> Poll {
        Poll {
            question: "Raid night?".into(),
            options: vec!["Friday".into(), "Saturday".into(), "Sunday".into()],
            multi: false,
            closes_at: None,
        }
    }

    #[test]
    fn tidy_squashes_whitespace_and_drops_empty_options() {
        let poll = Poll {
            question: "  Raid   night? ".into(),
            options: vec![" Friday ".into(), "".into(), "Sat\n\nurday".into()],
            ..sample()
        };
        let tidy = poll.tidy().unwrap();
        assert_eq!(tidy.question, "Raid night?");
        assert_eq!(tidy.options, vec!["Friday", "Sat urday"]);
    }

    #[test]
    fn tidy_enforces_the_limits() {
        assert!(Poll { question: "".into(), ..sample() }.tidy().is_none());
        assert!(Poll { question: "q".repeat(MAX_QUESTION_CHARS + 1), ..sample() }.tidy().is_none());
        assert!(Poll { question: "q".repeat(MAX_QUESTION_CHARS), ..sample() }.tidy().is_some());
        assert!(Poll { options: vec!["one".into()], ..sample() }.tidy().is_none());
        assert!(Poll { options: (0..=MAX_OPTIONS).map(|i| i.to_string()).collect(), ..sample() }.tidy().is_none());
        assert!(Poll { options: (0..MAX_OPTIONS).map(|i| i.to_string()).collect(), ..sample() }.tidy().is_some());
        assert!(Poll { options: vec!["a".into(), "o".repeat(MAX_OPTION_CHARS + 1)], ..sample() }.tidy().is_none());
    }

    #[test]
    fn a_poll_rides_on_a_message_as_an_attachment_without_a_file() {
        let att = sample().into_attachment();
        assert!(att.is_poll());
        assert!(!att.carries_a_file());
        assert!(!att.is_link_preview());
        assert_eq!(att.poll(), Some(sample()));
        let mut message = Message::new(ChannelId::new(), UserId::new(), "");
        assert_eq!(message.poll(), None);
        message.attachments.push(att);
        assert_eq!(message.poll(), Some(sample()));
        assert_eq!(message.files().count(), 0, "a poll is not a file to download");
    }

    #[test]
    fn broken_or_oversized_poll_json_reads_as_no_poll() {
        let mut att = sample().into_attachment();
        att.filename = "{not json".into();
        assert_eq!(att.poll(), None);
        let mut att = Poll { options: vec!["only".into()], ..sample() }.into_attachment();
        assert_eq!(att.poll(), None);
        att.mime_type = "text/plain".into();
        assert_eq!(att.poll(), None);
        assert!(att.carries_a_file());
    }

    #[test]
    fn poll_json_stays_small_and_old_fields_default() {
        let json = serde_json::to_string(&sample()).unwrap();
        assert!(!json.contains("closes_at"));
        let bare: Poll = serde_json::from_str(r#"{"question":"q","options":["a","b"]}"#).unwrap();
        assert!(!bare.multi);
        assert_eq!(bare.closes_at, None);
    }

    #[test]
    fn choices_are_sorted_deduplicated_and_bounded() {
        let poll = Poll { multi: true, ..sample() };
        assert_eq!(poll.normalize_choices(&[2, 0, 2]), Some(vec![0, 2]));
        assert_eq!(poll.normalize_choices(&[]), Some(vec![]));
        assert_eq!(poll.normalize_choices(&[3]), None);
        let single = sample();
        assert_eq!(single.normalize_choices(&[1]), Some(vec![1]));
        assert_eq!(single.normalize_choices(&[1, 1]), Some(vec![1]));
        assert_eq!(single.normalize_choices(&[0, 1]), None);
    }

    #[test]
    fn tally_counts_each_voter_once_and_each_choice_per_option() {
        let poll = Poll { multi: true, ..sample() };
        let (a, b, c) = (UserId::new(), UserId::new(), UserId::new());
        let ballots: Ballots = [(a, vec![0, 2]), (b, vec![2]), (c, vec![])].into_iter().collect();
        let tally = poll.tally(&ballots, false);
        assert_eq!(tally, PollTally { counts: vec![1, 0, 2], voters: 2, closed: false });
        assert_eq!(poll.tally(&Ballots::new(), true), PollTally { counts: vec![0, 0, 0], voters: 0, closed: true });
    }

    #[test]
    fn a_poll_is_open_until_it_closes() {
        let now = Utc::now();
        assert!(sample().is_open_at(now));
        let timed = Poll { closes_at: Some(now + chrono::Duration::hours(1)), ..sample() };
        assert!(timed.is_open_at(now));
        assert!(!timed.is_open_at(now + chrono::Duration::hours(1)));
        assert!(!timed.is_open_at(now + chrono::Duration::hours(2)));
    }
}
