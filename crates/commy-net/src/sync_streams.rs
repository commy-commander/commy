use chrono::{DateTime, Datelike, Months, Utc};
use commy_core::{Identity, IdentityKey, UserId};
use commy_sync::{StreamKeys, StreamSecret};

use crate::dm::DmEnvelope;
use crate::sealed::SealKey;

const ACCOUNT_LOG: &str = "commy-account-log";
const DM_PAIR: &str = "commy-dm-stream/v1";
const DM_EPOCH: &str = "commy-dm-epoch";

pub fn account_stream(key: &SealKey) -> StreamKeys {
    StreamSecret::derive(key.bytes(), ACCOUNT_LOG, b"").keys()
}

pub fn dm_epoch(at: DateTime<Utc>) -> String {
    format!("{:04}-{:02}", at.year(), at.month())
}

pub fn recent_dm_epochs(now: DateTime<Utc>, months: u32) -> Vec<String> {
    (0..months).filter_map(|back| now.checked_sub_months(Months::new(back))).map(dm_epoch).collect()
}

pub fn dm_stream(me: &Identity, partner: &IdentityKey, epoch: &str) -> Option<StreamKeys> {
    let pair = me.pairwise_secret(partner, DM_PAIR)?;
    Some(StreamSecret::derive(&pair, DM_EPOCH, epoch.as_bytes()).keys())
}

pub fn encode_dm(envelope: &DmEnvelope) -> Vec<u8> {
    postcard::to_stdvec(envelope).expect("postcard encodes DM envelopes")
}

#[derive(Debug, Clone, PartialEq)]
pub enum DmEntry {
    FromPartner(DmEnvelope),
    FromMe(DmEnvelope),
}

pub fn decode_dm(plain: &[u8], me: UserId, me_key: &IdentityKey, partner: UserId, partner_key: &IdentityKey) -> Option<DmEntry> {
    let envelope = DmEnvelope::decode(plain)?;
    if !envelope.signed_by_sender() {
        return None;
    }
    let message = &envelope.message;
    let signer = envelope.sender.identity?;
    if message.from == partner && message.to == me && signer == *partner_key {
        return Some(DmEntry::FromPartner(envelope));
    }
    if message.from == me && message.to == partner && signer == *me_key {
        return Some(DmEntry::FromMe(envelope));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{DmMessage, UserProfile};

    fn person(name: &str) -> (UserProfile, Identity) {
        let identity = Identity::generate();
        let mut profile = UserProfile::new(name);
        profile.identity = Some(identity.public());
        (profile, identity)
    }

    fn signed(from: &(UserProfile, Identity), to: &UserProfile, text: &str) -> DmEnvelope {
        let mut message = DmMessage::new(from.0.id, to.id, text);
        message.sign(&"ab".repeat(32), &from.1);
        DmEnvelope::new(message, from.0.clone())
    }

    #[test]
    fn both_people_and_all_their_devices_reach_the_same_stream() {
        let (alice, alice_id) = person("alice");
        let (bob, bob_id) = person("bob");
        let from_alice = dm_stream(&alice_id, &bob.identity.unwrap(), "2026-10").unwrap();
        let from_bob = dm_stream(&bob_id, &alice.identity.unwrap(), "2026-10").unwrap();
        assert_eq!(from_alice.id, from_bob.id);
        let alices_phone = Identity::from_bytes(&alice_id.to_bytes());
        assert_eq!(dm_stream(&alices_phone, &bob.identity.unwrap(), "2026-10").unwrap().id, from_alice.id);
        assert_ne!(dm_stream(&alice_id, &bob.identity.unwrap(), "2026-11").unwrap().id, from_alice.id, "each month has its own stream");
        let (carol, _) = person("carol");
        assert_ne!(dm_stream(&alice_id, &carol.identity.unwrap(), "2026-10").unwrap().id, from_alice.id);
        let sealed = from_alice.seal(b"hi").unwrap();
        assert_eq!(from_bob.open(&sealed).unwrap(), b"hi");
    }

    #[test]
    fn the_account_log_follows_the_entry_key() {
        let key = SealKey::generate();
        assert_eq!(account_stream(&key).id, account_stream(&SealKey::from_text(&key.to_text()).unwrap()).id);
        assert_ne!(account_stream(&key).id, account_stream(&SealKey::generate()).id);
    }

    #[test]
    fn epochs_are_calendar_months_counting_back() {
        let now = DateTime::parse_from_rfc3339("2026-01-15T10:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(dm_epoch(now), "2026-01");
        assert_eq!(recent_dm_epochs(now, 3), vec!["2026-01", "2025-12", "2025-11"]);
        let end = DateTime::parse_from_rfc3339("2026-03-31T23:59:59Z").unwrap().with_timezone(&Utc);
        assert_eq!(recent_dm_epochs(end, 2), vec!["2026-03", "2026-02"]);
    }

    #[test]
    fn entries_are_sorted_by_who_wrote_them_and_forgeries_are_dropped() {
        let alice = person("alice");
        let bob = person("bob");
        let (a, ak, b, bk) = (alice.0.id, alice.1.public(), bob.0.id, bob.1.public());
        let hello = signed(&alice, &bob.0, "hello bob");
        let plain = encode_dm(&hello);
        assert_eq!(decode_dm(&plain, b, &bk, a, &ak), Some(DmEntry::FromPartner(hello.clone())));
        assert_eq!(decode_dm(&plain, a, &ak, b, &bk), Some(DmEntry::FromMe(hello.clone())));

        let mallory = person("mallory");
        let mut forged = signed(&mallory, &bob.0, "i am alice");
        forged.message.from = a;
        assert_eq!(decode_dm(&encode_dm(&forged), b, &bk, a, &ak), None);
        let wrong_key = signed(&(alice.0.clone(), mallory.1), &bob.0, "x");
        assert_eq!(decode_dm(&encode_dm(&wrong_key), b, &bk, a, &ak), None);
        let mut edited = hello.clone();
        edited.message.content.push('!');
        assert_eq!(decode_dm(&encode_dm(&edited), b, &bk, a, &ak), None);
        let carol = person("carol");
        let elsewhere = signed(&alice, &carol.0, "not for bob");
        assert_eq!(decode_dm(&encode_dm(&elsewhere), b, &bk, a, &ak), None);
        assert_eq!(decode_dm(b"junk", b, &bk, a, &ak), None);
    }
}
