use serde::{Deserialize, Serialize};

use crate::stream::StreamId;

pub const REASONS: &[&str] = &["spam", "harassment", "inappropriate", "impersonation", "other"];
pub const MAX_PICKED_MESSAGES: usize = 10;
pub const MAX_MESSAGE_CHARS: usize = 2000;
pub const MAX_NOTE_CHARS: usize = 500;
pub const MAX_NAME_CHARS: usize = 100;
pub const MAX_ID_CHARS: usize = 64;
pub const MAX_REPORTED_STREAMS: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickedMessage {
    pub id: String,
    pub channel: String,
    pub author: Person,
    pub at: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostReport {
    pub community: String,
    pub community_name: String,
    pub owner: Person,
    pub reporter: Person,
    pub reported: Person,
    pub reason: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub messages: Vec<PickedMessage>,
    #[serde(default)]
    pub streams: Vec<StreamId>,
    pub sent_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReportProblem {
    #[error("pick a reason for the report")]
    Reason,
    #[error("a report needs the community and the people in it")]
    Missing,
    #[error("a field of the report is too long")]
    TooLong,
    #[error("a report carries at most {MAX_PICKED_MESSAGES} messages")]
    TooManyMessages,
    #[error("a report lists too many streams")]
    TooManyStreams,
}

fn chars(text: &str) -> usize {
    text.chars().count()
}

fn person_fits(person: &Person) -> bool {
    !person.id.is_empty() && chars(&person.id) <= MAX_ID_CHARS && chars(&person.name) <= MAX_NAME_CHARS
}

fn message_fits(message: &PickedMessage) -> bool {
    chars(&message.id) <= MAX_ID_CHARS
        && chars(&message.channel) <= MAX_NAME_CHARS
        && chars(&message.at) <= MAX_ID_CHARS
        && chars(&message.text) <= MAX_MESSAGE_CHARS
        && person_fits(&message.author)
}

impl HostReport {
    pub fn check(&self) -> Result<(), ReportProblem> {
        if !REASONS.contains(&self.reason.as_str()) {
            return Err(ReportProblem::Reason);
        }
        if self.community.is_empty() || self.reporter.id.is_empty() || self.reported.id.is_empty() || self.owner.id.is_empty() {
            return Err(ReportProblem::Missing);
        }
        if self.messages.len() > MAX_PICKED_MESSAGES {
            return Err(ReportProblem::TooManyMessages);
        }
        if self.streams.len() > MAX_REPORTED_STREAMS {
            return Err(ReportProblem::TooManyStreams);
        }
        let people_fit = [&self.owner, &self.reporter, &self.reported].into_iter().all(person_fits);
        let fields_fit = chars(&self.community) <= MAX_ID_CHARS
            && chars(&self.community_name) <= MAX_NAME_CHARS
            && chars(&self.note) <= MAX_NOTE_CHARS
            && chars(&self.sent_at) <= MAX_ID_CHARS;
        if !people_fit || !fields_fit || !self.messages.iter().all(message_fits) {
            return Err(ReportProblem::TooLong);
        }
        Ok(())
    }
}

pub fn clip(text: &str, most: usize) -> String {
    text.chars().take(most).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub id: String,
    pub operator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact: Option<String>,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::stream::StreamSecret;

    pub(crate) fn sample() -> HostReport {
        let person = |id: &str, name: &str| Person { id: id.into(), name: name.into() };
        HostReport {
            community: "c1".into(),
            community_name: "Night Owls".into(),
            owner: person("u-owner", "Owl"),
            reporter: person("u-me", "Ferris"),
            reported: person("u-them", "Nova"),
            reason: "harassment".into(),
            note: "Keeps going after people in general.".into(),
            messages: vec![PickedMessage {
                id: "m1".into(),
                channel: "general".into(),
                author: person("u-them", "Nova"),
                at: "2026-11-15T04:00:00Z".into(),
                text: "you again".into(),
            }],
            streams: vec![StreamSecret::random().keys().id],
            sent_at: "2026-11-15T04:01:00Z".into(),
        }
    }

    #[test]
    fn a_sound_report_passes_and_round_trips() {
        let report = sample();
        assert_eq!(report.check(), Ok(()));
        let json = serde_json::to_string(&report).unwrap();
        assert_eq!(serde_json::from_str::<HostReport>(&json).unwrap(), report);
    }

    #[test]
    fn reports_outside_the_limits_are_refused() {
        let mut report = sample();
        report.reason = "boring".into();
        assert_eq!(report.check(), Err(ReportProblem::Reason));
        let mut report = sample();
        report.reported.id.clear();
        assert_eq!(report.check(), Err(ReportProblem::Missing));
        let mut report = sample();
        report.messages = vec![report.messages[0].clone(); MAX_PICKED_MESSAGES + 1];
        assert_eq!(report.check(), Err(ReportProblem::TooManyMessages));
        let mut report = sample();
        report.messages[0].text = "x".repeat(MAX_MESSAGE_CHARS + 1);
        assert_eq!(report.check(), Err(ReportProblem::TooLong));
        let mut report = sample();
        report.note = "é".repeat(MAX_NOTE_CHARS);
        assert_eq!(report.check(), Ok(()), "limits count characters, not bytes");
        let mut report = sample();
        report.streams = vec![report.streams[0]; MAX_REPORTED_STREAMS + 1];
        assert_eq!(report.check(), Err(ReportProblem::TooManyStreams));
    }

    #[test]
    fn clipping_keeps_whole_characters() {
        assert_eq!(clip("héllo", 2), "hé");
        assert_eq!(clip("hi", 10), "hi");
    }
}
