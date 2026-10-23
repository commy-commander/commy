use commy_core::{Identity, IdentityKey};
use commy_sync::{StreamKeys, StreamSecret};
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::sealed::{Purpose, SealKey};

const MAILBOX: &str = "commy-key-handoff/v1";
const MAILBOX_ROOT: &[u8; 32] = b"commy-key-handoff-mailbox-root!!";
const PAIR: &str = "commy-key-handoff-pair/v1";
const CODE: &[u8] = b"commy-device-code/v1";

pub const FRESH_MS: i64 = 10 * 60 * 1000;
pub const TAIL: u32 = 64;
const MAX_NAME: usize = 64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "snake_case")]
pub enum Note {
    Ask { node: IdentityKey, name: String },
    Give { to: IdentityKey, from: IdentityKey, key: String },
    #[serde(other)]
    Unknown,
}

impl Note {
    pub fn parse(plain: &[u8]) -> Note {
        serde_json::from_slice(plain).unwrap_or(Note::Unknown)
    }

    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("handoff notes encode")
    }
}

pub fn mailbox(account: &str) -> StreamKeys {
    StreamSecret::derive(MAILBOX_ROOT, MAILBOX, account.as_bytes()).keys()
}

pub fn ask(node: &Identity, name: &str) -> Note {
    Note::Ask { node: node.public(), name: clean_name(name) }
}

pub fn clean_name(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect()
}

pub fn give(to: &IdentityKey, entry_key: &SealKey) -> Option<Note> {
    let from = Identity::generate();
    let pair = SealKey::from_bytes(from.pairwise_secret(to, PAIR)?);
    let key = pair.seal(Purpose::KeyHandoff, entry_key.to_text().as_bytes());
    Some(Note::Give { to: *to, from: from.public(), key })
}

pub fn open_give(note: &Note, me: &Identity) -> Option<SealKey> {
    let Note::Give { to, from, key } = note else { return None };
    if *to != me.public() {
        return None;
    }
    let pair = SealKey::from_bytes(me.pairwise_secret(from, PAIR)?);
    let text = pair.open(Purpose::KeyHandoff, key).ok()?;
    SealKey::from_text(std::str::from_utf8(&text).ok()?).ok()
}

pub fn is_fresh(at: i64, now: i64) -> bool {
    now - at < FRESH_MS
}

pub fn code(node: &IdentityKey) -> String {
    let mut hash = sha2::Sha256::new();
    hash.update(CODE);
    hash.update(node.0);
    let digest = hash.finalize();
    let number = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000;
    format!("{:03} {:03}", number / 1000, number % 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_asking_device_opens_the_key() {
        let asker = Identity::generate();
        let entry_key = SealKey::generate();
        let Note::Ask { node, name } = Note::parse(&ask(&asker, "Web browser\n").encode()) else { panic!("an ask") };
        assert_eq!(name, "Web browser");
        let given = Note::parse(&give(&node, &entry_key).unwrap().encode());
        assert_eq!(open_give(&given, &asker), Some(entry_key.clone()));
        assert_eq!(open_give(&given, &Identity::generate()), None);
        let Note::Give { from, key, .. } = given.clone() else { panic!("a give") };
        let stranger = Identity::generate();
        let readdressed = Note::Give { to: stranger.public(), from, key };
        assert_eq!(open_give(&readdressed, &stranger), None);
        assert!(!format!("{given:?}").contains(&entry_key.to_text()));
    }

    #[test]
    fn every_device_of_one_account_meets_in_one_mailbox() {
        assert_eq!(mailbox("acct-1").id, mailbox("acct-1").id);
        assert_ne!(mailbox("acct-1").id, mailbox("acct-2").id);
    }

    #[test]
    fn both_screens_show_the_same_short_code() {
        let node = Identity::generate().public();
        let shown = code(&node);
        assert_eq!(shown, code(&node));
        assert_eq!(shown.len(), 7);
        assert!(shown.chars().all(|c| c.is_ascii_digit() || c == ' '));
        assert_ne!(shown, code(&Identity::generate().public()));
    }

    #[test]
    fn old_asks_go_stale_and_unknown_notes_are_skipped() {
        assert!(is_fresh(1_000, 1_000 + FRESH_MS - 1));
        assert!(!is_fresh(1_000, 1_000 + FRESH_MS));
        assert_eq!(Note::parse(br#"{"k":"later"}"#), Note::Unknown);
        assert_eq!(Note::parse(b"junk"), Note::Unknown);
    }
}
