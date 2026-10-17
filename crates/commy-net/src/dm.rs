use commy_core::{DmMessage, UserProfile};
use serde::{Deserialize, Serialize};

pub const DM_ALPN: &[u8] = b"commy/dm/2";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DmEnvelope {
    pub message: DmMessage,
    pub sender: UserProfile,
}

impl DmEnvelope {
    pub fn signed_by_sender(&self) -> bool {
        self.message.from == self.sender.id
            && self
                .sender
                .identity
                .is_some_and(|key| self.message.verify(&key))
    }

    pub fn sent_from(&self, node: &[u8; 32]) -> bool {
        data_encoding::HEXLOWER_PERMISSIVE
            .decode(self.message.from_node.trim().as_bytes())
            .is_ok_and(|bytes| bytes == node)
    }
}

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

    const NODE: [u8; 32] = [3; 32];

    fn signed_envelope(content: &str) -> (DmEnvelope, Identity) {
        let sender = keyed_profile("sender");
        let identity = crate::test_support::identity_for(sender.id);
        let mut message = DmMessage::new(sender.id, UserId::new(), content);
        message.sign(&data_encoding::HEXLOWER.encode(&NODE), &identity);
        (DmEnvelope { message, sender }, identity)
    }

    #[test]
    fn a_dm_names_the_node_it_is_sent_from() {
        let (envelope, identity) = signed_envelope("from my device");
        assert!(envelope.sent_from(&NODE));
        assert!(!envelope.sent_from(&[4; 32]));
        let mut upper = envelope.clone();
        upper.message.from_node = upper.message.from_node.to_uppercase();
        assert!(upper.sent_from(&NODE));
        let mut unnamed = envelope.clone();
        unnamed.message.from_node.clear();
        assert!(!unnamed.sent_from(&NODE));

        let mut moved = envelope.clone();
        moved.message.from_node = data_encoding::HEXLOWER.encode(&[4; 32]);
        assert!(moved.sent_from(&[4; 32]));
        assert!(!moved.signed_by_sender());
        moved.message.sign(&data_encoding::HEXLOWER.encode(&[4; 32]), &identity);
        assert!(moved.signed_by_sender());
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
