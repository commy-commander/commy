//! Direct message transport. DMs bypass server gossip entirely: the
//! sender opens a QUIC connection with the `commy/dm/2` ALPN, writes
//! message envelopes, and waits for a per-message ack so delivery is
//! confirmed. Both peers must be online; offline inboxes would need
//! hosted infrastructure, which Commy does not have by design. Each
//! message is signed with the sender's identity key.

use commy_core::{DmMessage, UserProfile};
use serde::{Deserialize, Serialize};

pub const DM_ALPN: &[u8] = b"commy/dm/2";

/// One message plus the sender's profile, so the receiver can render
/// the conversation even without a shared server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DmEnvelope {
    pub message: DmMessage,
    pub sender: UserProfile,
}

impl DmEnvelope {
    /// True when the message comes from the enclosed profile and is
    /// signed by that profile's identity key. Receivers should also
    /// check the key against the one they pinned for this contact.
    pub fn signed_by_sender(&self) -> bool {
        self.message.from == self.sender.id
            && self
                .sender
                .identity
                .is_some_and(|key| self.message.verify(&key))
    }
}

/// Receiver's reply on the same stream confirming the message landed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DmAck {
    pub accepted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{read_frame, write_frame};
    use crate::test_support::keyed_profile;
    use commy_core::{Identity, UserId};

    /// Envelope from a keyed sender, signed by them.
    fn signed_envelope(content: &str) -> (DmEnvelope, Identity) {
        let sender = keyed_profile("sender");
        let identity = crate::test_support::identity_for(sender.id);
        let mut message = DmMessage::new(sender.id, UserId::new(), content);
        message.sign(&identity);
        (DmEnvelope { message, sender }, identity)
    }

    #[tokio::test]
    async fn signed_envelope_roundtrip() {
        let (mut a, mut b) = tokio::io::duplex(16 * 1024);
        let (envelope, _) = signed_envelope("signed and sealed");
        assert!(envelope.signed_by_sender());
        write_frame(&mut a, &envelope).await.unwrap();
        let got: DmEnvelope = read_frame(&mut b).await.unwrap().unwrap();
        assert_eq!(got, envelope);
        assert!(got.message.sig.is_some());
        assert!(got.signed_by_sender(), "signature survives the wire");
    }

    #[test]
    fn tampered_or_misattributed_envelopes_fail() {
        let (envelope, identity) = signed_envelope("original");

        let mut edited = envelope.clone();
        edited.message.content.push('!');
        assert!(!edited.signed_by_sender());

        let mut unsigned = envelope.clone();
        unsigned.message.sig = None;
        assert!(!unsigned.signed_by_sender());

        let mut keyless = envelope.clone();
        keyless.sender.identity = None;
        assert!(!keyless.signed_by_sender());

        let mut other_key = envelope.clone();
        other_key.sender.identity = Some(Identity::generate().public());
        assert!(!other_key.signed_by_sender());

        // Someone else's profile around a correctly signed message.
        let mut impostor = envelope.clone();
        impostor.sender = keyed_profile("impostor");
        impostor.sender.identity = Some(identity.public());
        assert!(!impostor.signed_by_sender());
    }

    #[test]
    fn alpn_is_bumped_for_signed_messages() {
        assert_eq!(DM_ALPN, b"commy/dm/2");
    }

    #[tokio::test]
    async fn envelope_and_ack_roundtrip() {
        let (mut a, mut b) = tokio::io::duplex(16 * 1024);
        let envelope = DmEnvelope {
            message: DmMessage::new(UserId::new(), UserId::new(), "psst"),
            sender: UserProfile::new("whisperer"),
        };
        write_frame(&mut a, &envelope).await.unwrap();
        let got: DmEnvelope = read_frame(&mut b).await.unwrap().unwrap();
        assert_eq!(got, envelope);

        write_frame(&mut b, &DmAck { accepted: true }).await.unwrap();
        let ack: DmAck = read_frame(&mut a).await.unwrap().unwrap();
        assert!(ack.accepted);
    }
}
