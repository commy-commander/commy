//! Direct message transport. DMs bypass server gossip entirely: the
//! sender opens a QUIC connection with the `commy/dm/1` ALPN, writes
//! message envelopes, and waits for a per-message ack so delivery is
//! confirmed. Both peers must be online; offline inboxes would need
//! hosted infrastructure, which Commy does not have by design.

use commy_core::{DmMessage, UserProfile};
use serde::{Deserialize, Serialize};

pub const DM_ALPN: &[u8] = b"commy/dm/1";

/// One message plus the sender's profile, so the receiver can render
/// the conversation even without a shared server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DmEnvelope {
    pub message: DmMessage,
    pub sender: UserProfile,
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
    use commy_core::UserId;

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
