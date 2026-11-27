use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use curve25519_dalek::montgomery::MontgomeryPoint;
use data_encoding::BASE64URL_NOPAD;
use serde::{Deserialize, Serialize};
use std::fmt;

use crate::report::{HostReport, ReportProblem, MAX_ID_CHARS, MAX_REPORTED_STREAMS, REASONS};
use crate::stream::{hkdf, StreamId};
use crate::wire::Bytes;

pub const REPORT_KEY_PREFIX: &str = "commy-report-key1.";
pub const REPORT_SECRET_PREFIX: &str = "commy-report-secret1.";
pub const MAX_SEALED_REPORT_BYTES: usize = 128 * 1024;
const SEAL_SALT: &[u8] = b"commy-report-seal/v1";
const HEADER_DOMAIN: &str = "commy-report-header/v1";

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ReportKey([u8; 32]);

#[derive(Clone, PartialEq, Eq)]
pub struct ReportSecret([u8; 32]);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OpenProblem {
    #[error("this report was sealed to a different report key")]
    OtherKey,
    #[error("the report key does not open this report")]
    WrongKey,
    #[error("the sealed report does not match what was sent with it")]
    Mismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedReport {
    pub community: String,
    pub reason: String,
    #[serde(default)]
    pub streams: Vec<StreamId>,
    pub sent_at: String,
    pub key: Bytes<32>,
    pub ephemeral: Bytes<32>,
    pub nonce: Bytes<12>,
    pub sealed: String,
}

fn decode_text<const N: usize>(text: &str, prefix: &str) -> Option<[u8; N]> {
    let body = text.trim().strip_prefix(prefix)?;
    BASE64URL_NOPAD.decode(body.as_bytes()).ok()?.try_into().ok()
}

fn cipher(shared: &[u8; 32], ephemeral: &[u8; 32], recipient: &[u8; 32]) -> Aes256Gcm {
    let mut info = Vec::with_capacity(64);
    info.extend_from_slice(ephemeral);
    info.extend_from_slice(recipient);
    Aes256Gcm::new_from_slice(&hkdf(shared, SEAL_SALT, &info)).expect("32 byte key")
}

impl ReportKey {
    pub fn from_text(text: &str) -> Option<Self> {
        decode_text(text, REPORT_KEY_PREFIX).map(Self)
    }

    pub fn to_text(&self) -> String {
        format!("{REPORT_KEY_PREFIX}{}", BASE64URL_NOPAD.encode(&self.0))
    }

    pub fn seal(&self, report: &HostReport) -> Option<SealedReport> {
        let ephemeral_secret: [u8; 32] = rand::random();
        let ephemeral = MontgomeryPoint::mul_base_clamped(ephemeral_secret).to_bytes();
        let shared = MontgomeryPoint(self.0).mul_clamped(ephemeral_secret).to_bytes();
        if shared == [0; 32] {
            return None;
        }
        let nonce: [u8; 12] = rand::random();
        let mut sealed = SealedReport {
            community: report.community.clone(),
            reason: report.reason.clone(),
            streams: report.streams.clone(),
            sent_at: report.sent_at.clone(),
            key: Bytes(self.0),
            ephemeral: Bytes(ephemeral),
            nonce: Bytes(nonce),
            sealed: String::new(),
        };
        let plain = serde_json::to_vec(report).ok()?;
        let body = cipher(&shared, &ephemeral, &self.0).encrypt(&Nonce::from(nonce), Payload { msg: &plain, aad: &sealed.header() }).ok()?;
        sealed.sealed = BASE64URL_NOPAD.encode(&body);
        Some(sealed)
    }
}

impl fmt::Debug for ReportKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ReportKey({})", self.to_text())
    }
}

impl ReportSecret {
    pub fn generate() -> Self {
        Self(rand::random())
    }

    pub fn from_text(text: &str) -> Option<Self> {
        decode_text(text, REPORT_SECRET_PREFIX).map(Self)
    }

    pub fn to_text(&self) -> String {
        format!("{REPORT_SECRET_PREFIX}{}", BASE64URL_NOPAD.encode(&self.0))
    }

    pub fn public(&self) -> ReportKey {
        ReportKey(MontgomeryPoint::mul_base_clamped(self.0).to_bytes())
    }

    pub fn open(&self, sealed: &SealedReport) -> Result<HostReport, OpenProblem> {
        let mine = self.public();
        if sealed.key.0 != mine.0 {
            return Err(OpenProblem::OtherKey);
        }
        let shared = MontgomeryPoint(sealed.ephemeral.0).mul_clamped(self.0).to_bytes();
        let body = BASE64URL_NOPAD.decode(sealed.sealed.as_bytes()).map_err(|_| OpenProblem::WrongKey)?;
        let plain = cipher(&shared, &sealed.ephemeral.0, &mine.0)
            .decrypt(&Nonce::from(sealed.nonce.0), Payload { msg: &body, aad: &sealed.header() })
            .map_err(|_| OpenProblem::WrongKey)?;
        let report: HostReport = serde_json::from_slice(&plain).map_err(|_| OpenProblem::Mismatch)?;
        sealed.matches(&report).then_some(report).ok_or(OpenProblem::Mismatch)
    }
}

impl fmt::Debug for ReportSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ReportSecret(..)")
    }
}

impl SealedReport {
    fn header(&self) -> Vec<u8> {
        serde_json::to_vec(&(HEADER_DOMAIN, &self.community, &self.reason, &self.streams, &self.sent_at, &self.key)).expect("plain fields encode")
    }

    fn matches(&self, report: &HostReport) -> bool {
        report.community == self.community && report.reason == self.reason && report.streams == self.streams && report.sent_at == self.sent_at
    }

    pub fn check(&self) -> Result<(), ReportProblem> {
        if !REASONS.contains(&self.reason.as_str()) {
            return Err(ReportProblem::Reason);
        }
        if self.community.is_empty() || self.sealed.is_empty() {
            return Err(ReportProblem::Missing);
        }
        if self.streams.len() > MAX_REPORTED_STREAMS {
            return Err(ReportProblem::TooManyStreams);
        }
        let fits = self.community.chars().count() <= MAX_ID_CHARS
            && self.sent_at.chars().count() <= MAX_ID_CHARS
            && self.sealed.len() <= MAX_SEALED_REPORT_BYTES.div_ceil(3) * 4;
        if !fits {
            return Err(ReportProblem::TooLong);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::tests::sample;
    use crate::report::{PickedMessage, MAX_MESSAGE_CHARS, MAX_NAME_CHARS, MAX_NOTE_CHARS, MAX_PICKED_MESSAGES};

    #[test]
    fn the_operator_opens_what_the_reporter_sealed() {
        let secret = ReportSecret::generate();
        let report = sample();
        let sealed = secret.public().seal(&report).unwrap();
        assert_eq!(sealed.check(), Ok(()));
        assert_eq!(secret.open(&sealed), Ok(report.clone()));
        let json = serde_json::to_string(&sealed).unwrap();
        assert!(!json.contains("Nova") && !json.contains("you again") && !json.contains("Night Owls"), "names and text stay sealed");
        assert!(json.contains(&report.community) && json.contains("harassment"), "the community and reason stay readable for triage");
    }

    #[test]
    fn another_key_cannot_open_it() {
        let report = sample();
        let sealed = ReportSecret::generate().public().seal(&report).unwrap();
        assert_eq!(ReportSecret::generate().open(&sealed), Err(OpenProblem::OtherKey));
        let mut relabelled = sealed.clone();
        let stranger = ReportSecret::generate();
        relabelled.key = Bytes(stranger.public().0);
        assert_eq!(stranger.open(&relabelled), Err(OpenProblem::WrongKey));
    }

    #[test]
    fn the_readable_fields_cannot_be_changed() {
        let secret = ReportSecret::generate();
        let sealed = secret.public().seal(&sample()).unwrap();
        let mut moved = sealed.clone();
        moved.community = "another".into();
        assert_eq!(secret.open(&moved), Err(OpenProblem::WrongKey));
        let mut softened = sealed.clone();
        softened.reason = "spam".into();
        assert_eq!(secret.open(&softened), Err(OpenProblem::WrongKey));
        let mut fewer = sealed;
        fewer.streams.clear();
        assert_eq!(secret.open(&fewer), Err(OpenProblem::WrongKey));
    }

    #[test]
    fn keys_round_trip_as_text_and_refuse_the_wrong_kind() {
        let secret = ReportSecret::generate();
        let key = secret.public();
        assert_eq!(ReportKey::from_text(&key.to_text()), Some(key));
        assert_eq!(ReportSecret::from_text(&format!("  {}\n", secret.to_text())), Some(secret.clone()));
        assert_eq!(ReportKey::from_text(&secret.to_text()), None);
        assert_eq!(ReportSecret::from_text(&key.to_text()), None);
        assert_eq!(ReportKey::from_text("commy-report-key1.short"), None);
        assert!(!format!("{secret:?}").contains(&secret.to_text()));
    }

    #[test]
    fn the_largest_report_still_fits() {
        let mut report = sample();
        let wide = |n: usize| "\u{1F600}".repeat(n);
        report.note = wide(MAX_NOTE_CHARS);
        report.community_name = wide(MAX_NAME_CHARS);
        report.messages = vec![PickedMessage { text: wide(MAX_MESSAGE_CHARS), channel: wide(MAX_NAME_CHARS), ..report.messages[0].clone() }; MAX_PICKED_MESSAGES];
        report.streams = vec![report.streams[0]; MAX_REPORTED_STREAMS];
        assert_eq!(report.check(), Ok(()));
        let sealed = ReportSecret::generate().public().seal(&report).unwrap();
        assert_eq!(sealed.check(), Ok(()));
        let frame = serde_json::to_string(&crate::wire::ClientFrame::SealedReport { report: Box::new(sealed) }).unwrap();
        assert!(frame.len() <= crate::wire::MAX_CLIENT_FRAME_BYTES, "a full sealed report is {} bytes", frame.len());
    }

    #[test]
    fn sealed_reports_outside_the_limits_are_refused() {
        let sealed = ReportSecret::generate().public().seal(&sample()).unwrap();
        assert_eq!(SealedReport { reason: "boring".into(), ..sealed.clone() }.check(), Err(ReportProblem::Reason));
        assert_eq!(SealedReport { community: String::new(), ..sealed.clone() }.check(), Err(ReportProblem::Missing));
        assert_eq!(SealedReport { sealed: "A".repeat(MAX_SEALED_REPORT_BYTES * 2), ..sealed.clone() }.check(), Err(ReportProblem::TooLong));
        assert_eq!(SealedReport { streams: vec![sealed.streams[0]; MAX_REPORTED_STREAMS + 1], ..sealed }.check(), Err(ReportProblem::TooManyStreams));
    }
}
