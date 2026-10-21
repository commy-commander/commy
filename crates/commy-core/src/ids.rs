use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use uuid::Uuid;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl std::str::FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(s)?))
            }
        }
    };
}

id_type!(UserId);
id_type!(ServerId);
id_type!(ChannelId);
id_type!(MessageId);
id_type!(RoleId);
id_type!(TagId);

pub const CALL_SERVER: ServerId = ServerId(Uuid::nil());

const MESSAGE_ID_DOMAIN: &[u8] = b"commy-message-id/v1";

impl MessageId {
    pub fn for_author(author: UserId) -> Self {
        let tail: [u8; 8] = rand::random();
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&author_tag(author, &tail));
        bytes[8..].copy_from_slice(&tail);
        Self(Uuid::from_bytes(bytes))
    }

    pub fn is_bound_to(&self, author: UserId) -> bool {
        let bytes = self.0.as_bytes();
        bytes[..8] == author_tag(author, &bytes[8..])
    }
}

fn author_tag(author: UserId, tail: &[u8]) -> [u8; 8] {
    let digest = Sha256::new()
        .chain_update(MESSAGE_ID_DOMAIN)
        .chain_update(author.0.as_bytes())
        .chain_update(tail)
        .finalize();
    let mut out = [0u8; 8];
    out.copy_from_slice(&digest[..8]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        assert_ne!(UserId::new(), UserId::new());
    }

    #[test]
    fn id_roundtrips_through_string() {
        let id = ChannelId::new();
        let parsed: ChannelId = id.to_string().parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn message_ids_are_bound_to_their_author() {
        let (author, other) = (UserId::new(), UserId::new());
        let id = MessageId::for_author(author);
        assert!(id.is_bound_to(author));
        assert!(!id.is_bound_to(other));
        assert_ne!(id, MessageId::for_author(author));
        assert!(!MessageId::new().is_bound_to(author));
        let mut bytes = *id.0.as_bytes();
        bytes[15] ^= 1;
        assert!(
            !MessageId(Uuid::from_bytes(bytes)).is_bound_to(author),
            "the tail is covered"
        );
    }

    #[test]
    fn bound_id_layout_follows_the_spec() {
        let author = UserId(Uuid::from_bytes([7; 16]));
        let id = MessageId::for_author(author);
        let bytes = id.0.as_bytes();
        let mut input = b"commy-message-id/v1".to_vec();
        input.extend_from_slice(&[7; 16]);
        input.extend_from_slice(&bytes[8..]);
        assert_eq!(bytes[..8], Sha256::digest(&input)[..8]);
        let parsed: MessageId = id.to_string().parse().unwrap();
        assert!(parsed.is_bound_to(author), "survives the string form");
    }

    #[test]
    fn id_serializes_as_plain_string() {
        let id = MessageId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{}\"", id));
    }
}
