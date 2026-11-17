use crate::identity::{IdentityKey, Signature};
use crate::server_key::PublishedKey;
use serde::Deserialize;

pub const PROOF_PURPOSE: &str = "server-proof";
pub const PROOF_PATH: &str = "/api/auth/commy/server-proof";
pub const OFFICIAL_SERVER_KEYS: &[(&str, &str)] = &[("https://commy.chat", "d04a2158c0185ca33ce6a9dfed5d646eb5e564dc613df690033eaeb3860d12e2")];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ServerProof {
    pub origin: String,
    pub nonce: String,
    pub key: PublishedKey,
    pub sig: Signature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Official,
    Unproven,
    Impostor,
}

pub fn new_nonce() -> String {
    crate::identity::to_hex(&rand::random::<[u8; 32]>())
}

pub fn is_official_origin(origin: &str) -> bool {
    OFFICIAL_SERVER_KEYS.iter().any(|(pinned, _)| *pinned == origin)
}

pub fn holds_official_key(origin: &str, key: &IdentityKey) -> bool {
    holds_pinned_key(OFFICIAL_SERVER_KEYS, origin, key)
}

pub fn holds_pinned_key(pinned: &[(&str, &str)], origin: &str, key: &IdentityKey) -> bool {
    pinned.iter().any(|(o, hex)| *o == origin && IdentityKey::from_hex(hex).as_ref() == Some(key))
}

pub fn judge(origin: &str, nonce: &str, proof: Option<&ServerProof>) -> Verdict {
    judge_against(OFFICIAL_SERVER_KEYS, origin, nonce, proof)
}

pub fn judge_against(pinned: &[(&str, &str)], origin: &str, nonce: &str, proof: Option<&ServerProof>) -> Verdict {
    let keys: Vec<IdentityKey> = pinned.iter().filter(|(o, _)| *o == origin).filter_map(|(_, hex)| IdentityKey::from_hex(hex)).collect();
    if keys.is_empty() {
        return Verdict::Unproven;
    }
    let Some(proof) = proof else { return Verdict::Unproven };
    let answers_this_check = proof.origin == origin && proof.nonce == nonce;
    let pinned_key = keys.contains(&proof.key.public);
    let signed = proof.key.verifies(PROOF_PURPOSE, origin, nonce.as_bytes(), &proof.sig);
    if answers_this_check && pinned_key && signed {
        Verdict::Official
    } else {
        Verdict::Impostor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;
    use crate::server_key::sign;

    const ORIGIN: &str = "https://commy.chat";

    fn pinned_hex(key: &Identity) -> String {
        key.public().to_hex()
    }

    fn proof_from(key: &Identity, origin: &str, nonce: &str) -> ServerProof {
        ServerProof {
            origin: origin.into(),
            nonce: nonce.into(),
            key: PublishedKey::of(key.public()),
            sig: sign(key, PROOF_PURPOSE, origin, nonce.as_bytes()),
        }
    }

    #[test]
    fn official_keys_are_well_formed() {
        for (origin, hex) in OFFICIAL_SERVER_KEYS {
            assert!(crate::sync_host::is_sync_host(origin), "{origin}");
            assert!(IdentityKey::from_hex(hex).is_some(), "{origin}");
        }
        assert!(is_official_origin(ORIGIN));
        assert!(!is_official_origin("https://staging.commy.chat"));
    }

    #[test]
    fn only_the_pinned_key_signing_this_nonce_counts_as_official() {
        let official = Identity::from_bytes(&[11; 32]);
        let hex = pinned_hex(&official);
        let pinned = [(ORIGIN, hex.as_str())];
        let nonce = new_nonce();
        let good = proof_from(&official, ORIGIN, &nonce);
        assert_eq!(judge_against(&pinned, ORIGIN, &nonce, Some(&good)), Verdict::Official);

        let stranger = Identity::from_bytes(&[12; 32]);
        let own_key = proof_from(&stranger, ORIGIN, &nonce);
        assert_eq!(judge_against(&pinned, ORIGIN, &nonce, Some(&own_key)), Verdict::Impostor);

        let copied_key = ServerProof { key: PublishedKey::of(official.public()), ..own_key };
        assert_eq!(judge_against(&pinned, ORIGIN, &nonce, Some(&copied_key)), Verdict::Impostor);

        let replayed = proof_from(&official, ORIGIN, &new_nonce());
        assert_eq!(judge_against(&pinned, ORIGIN, &nonce, Some(&replayed)), Verdict::Impostor);

        let other_origin = proof_from(&official, "https://cornmy.chat", &nonce);
        assert_eq!(judge_against(&pinned, ORIGIN, &nonce, Some(&other_origin)), Verdict::Impostor);
        let relabelled = ServerProof { origin: ORIGIN.into(), ..other_origin };
        assert_eq!(judge_against(&pinned, ORIGIN, &nonce, Some(&relabelled)), Verdict::Impostor);
    }

    #[test]
    fn only_the_pinned_key_for_that_origin_is_official() {
        let official = Identity::from_bytes(&[15; 32]);
        let hex = pinned_hex(&official);
        let pinned = [(ORIGIN, hex.as_str())];
        assert!(holds_pinned_key(&pinned, ORIGIN, &official.public()));
        assert!(!holds_pinned_key(&pinned, "https://cornmy.chat", &official.public()));
        assert!(!holds_pinned_key(&pinned, ORIGIN, &Identity::from_bytes(&[16; 32]).public()));
        assert!(!holds_official_key(ORIGIN, &official.public()));
    }

    #[test]
    fn no_answer_or_no_pinned_key_proves_nothing() {
        let official = Identity::from_bytes(&[13; 32]);
        let hex = pinned_hex(&official);
        let pinned = [(ORIGIN, hex.as_str())];
        let nonce = new_nonce();
        assert_eq!(judge_against(&pinned, ORIGIN, &nonce, None), Verdict::Unproven);
        let elsewhere = proof_from(&official, "https://friends.org", &nonce);
        assert_eq!(judge_against(&pinned, "https://friends.org", &nonce, Some(&elsewhere)), Verdict::Unproven);
    }

    #[test]
    fn a_proof_reads_the_auth_server_reply() {
        let official = Identity::from_bytes(&[14; 32]);
        let nonce = "ab".repeat(32);
        let sig = sign(&official, PROOF_PURPOSE, ORIGIN, nonce.as_bytes());
        let published = PublishedKey::of(official.public());
        let reply = serde_json::json!({
            "origin": ORIGIN,
            "nonce": nonce,
            "key": { "alg": "ed25519", "id": published.id, "public": official.public().to_hex() },
            "sig": sig.to_hex(),
        });
        let proof: ServerProof = serde_json::from_value(reply).unwrap();
        let hex = pinned_hex(&official);
        assert_eq!(judge_against(&[(ORIGIN, hex.as_str())], ORIGIN, &nonce, Some(&proof)), Verdict::Official);
    }
}
