use commy_core::identity::IdentityKey;
use commy_core::{ServerId, UserId};
use data_encoding::BASE32_NOPAD;
use iroh::{EndpointAddr, EndpointId, RelayUrl, TransportAddr};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::net::SocketAddr;
use thiserror::Error;

const PREFIX: &str = "commy";
const TICKET_VERSION: u8 = 2;
const FIRST_TICKET_VERSION: u8 = 1;

#[derive(Debug, Error, PartialEq)]
pub enum TicketError {
    #[error("not a commy invite")]
    BadPrefix,
    #[error("unsupported invite version {0}")]
    BadVersion(u8),
    #[error("invite is corrupt")]
    Corrupt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeAddr {
    pub node_id: [u8; 32],
    pub info: AddrInfo,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddrInfo {
    pub relay_url: Option<String>,
    pub direct_addresses: BTreeSet<SocketAddr>,
}

impl NodeAddr {
    pub fn new(node: EndpointId) -> Self {
        Self { node_id: *node.as_bytes(), info: AddrInfo::default() }
    }

    pub fn node(&self) -> Option<EndpointId> {
        EndpointId::from_bytes(&self.node_id).ok()
    }

    pub fn endpoint_addr(&self) -> Option<EndpointAddr> {
        let relay = self.info.relay_url.as_deref().and_then(relay_url);
        let addrs = relay
            .map(TransportAddr::Relay)
            .into_iter()
            .chain(self.info.direct_addresses.iter().copied().map(TransportAddr::Ip));
        Some(EndpointAddr::from_parts(self.node()?, addrs))
    }
}

fn relay_url(text: &str) -> Option<RelayUrl> {
    let mut url = (*text.parse::<RelayUrl>().ok()?).clone();
    if let Some(host) = url.host_str().and_then(|h| h.strip_suffix('.')).map(str::to_owned) {
        url.set_host(Some(&host)).ok()?;
    }
    Some(url.into())
}

impl From<&EndpointAddr> for NodeAddr {
    fn from(addr: &EndpointAddr) -> Self {
        Self {
            node_id: *addr.id.as_bytes(),
            info: AddrInfo {
                relay_url: addr.relay_urls().next().map(|url| url.to_string()),
                direct_addresses: addr.ip_addrs().copied().collect(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerTicket {
    pub server_id: ServerId,
    pub server_name: String,
    pub topic: [u8; 32],
    pub bootstrap: Vec<NodeAddr>,
    pub owner: Option<(UserId, IdentityKey)>,
}

#[derive(Deserialize)]
struct FirstTicket {
    server_id: ServerId,
    server_name: String,
    topic: [u8; 32],
    bootstrap: Vec<NodeAddr>,
}

impl ServerTicket {
    pub fn new(
        server_id: ServerId,
        server_name: impl Into<String>,
        topic: [u8; 32],
        bootstrap: Vec<NodeAddr>,
    ) -> Self {
        Self {
            server_id,
            server_name: server_name.into(),
            topic,
            bootstrap,
            owner: None,
        }
    }

    pub fn encode(&self) -> String {
        let mut bytes = vec![TICKET_VERSION];
        bytes.extend(postcard::to_stdvec(self).expect("ticket serialization cannot fail"));
        format!(
            "{PREFIX}{}",
            BASE32_NOPAD.encode(&bytes).to_ascii_lowercase()
        )
    }

    pub fn decode(input: &str) -> Result<Self, TicketError> {
        let input = input.trim();
        let encoded = input.strip_prefix(PREFIX).ok_or(TicketError::BadPrefix)?;
        let bytes = BASE32_NOPAD
            .decode(encoded.to_ascii_uppercase().as_bytes())
            .map_err(|_| TicketError::Corrupt)?;
        let (&version, rest) = bytes.split_first().ok_or(TicketError::Corrupt)?;
        match version {
            TICKET_VERSION => postcard::from_bytes(rest).map_err(|_| TicketError::Corrupt),
            FIRST_TICKET_VERSION => {
                let first: FirstTicket = postcard::from_bytes(rest).map_err(|_| TicketError::Corrupt)?;
                Ok(Self::new(first.server_id, first.server_name, first.topic, first.bootstrap))
            }
            other => Err(TicketError::BadVersion(other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    fn sample() -> ServerTicket {
        let node_id = SecretKey::generate().public();
        ServerTicket::new(
            ServerId::new(),
            "Rust Hangout",
            [7u8; 32],
            vec![NodeAddr::new(node_id)],
        )
    }

    #[test]
    fn roundtrip() {
        let ticket = sample();
        let encoded = ticket.encode();
        assert!(encoded.starts_with("commy"));
        let decoded = ServerTicket::decode(&encoded).unwrap();
        assert_eq!(ticket, decoded);
    }

    #[test]
    fn decode_tolerates_surrounding_whitespace() {
        let ticket = sample();
        let padded = format!("  {}\n", ticket.encode());
        assert_eq!(ServerTicket::decode(&padded).unwrap(), ticket);
    }

    #[test]
    fn rejects_wrong_prefix() {
        assert_eq!(
            ServerTicket::decode("otherappabcdef"),
            Err(TicketError::BadPrefix)
        );
    }

    #[test]
    fn rejects_corrupt_payload() {
        let mut encoded = sample().encode();
        encoded.truncate(encoded.len() - 10);
        assert!(matches!(
            ServerTicket::decode(&encoded),
            Err(TicketError::Corrupt)
        ));
    }

    #[test]
    fn carries_the_owner_and_still_reads_first_version_tickets() {
        let mut ticket = sample();
        let owner = UserId::new();
        let key = commy_core::Identity::generate().public();
        ticket.owner = Some((owner, key));
        let decoded = ServerTicket::decode(&ticket.encode()).unwrap();
        assert_eq!(decoded.owner, Some((owner, key)));
        assert_eq!(decoded, ticket);
        let first = FirstTicketOut {
            server_id: ticket.server_id,
            server_name: ticket.server_name.clone(),
            topic: ticket.topic,
            bootstrap: ticket.bootstrap.clone(),
        };
        let mut bytes = vec![FIRST_TICKET_VERSION];
        bytes.extend(postcard::to_stdvec(&first).unwrap());
        let encoded = format!("commy{}", BASE32_NOPAD.encode(&bytes).to_ascii_lowercase());
        let decoded = ServerTicket::decode(&encoded).unwrap();
        assert_eq!(decoded, ServerTicket { owner: None, ..ticket });
    }

    #[test]
    fn reads_invites_made_before_the_network_upgrade() {
        let old = "commyaiigokk7waloir3exsziqmhx4lgkuccnn5xw4ytbonsqobyha4dqobyha4dqobyha4dqobyha4dqobyha4dqobyha4dqobybnz5bzxjjwc3y7uj26tcvtd7p6txsvfywny6kn4xe7p6m3acqlpyqch3ior2ha4z2f4xxezlmmf4s4y3pnvwxsltdnbqxilr2ha2dimzpaeamaaaca7iseaa";
        let ticket = ServerTicket::decode(old).unwrap();
        assert_eq!(ticket.server_name, "Moonbase");
        assert_eq!(ticket.topic, [7u8; 32]);
        let peer = &ticket.bootstrap[0];
        assert_eq!(peer.node(), Some(SecretKey::from_bytes(&[5u8; 32]).public()));
        let addr = peer.endpoint_addr().unwrap();
        let relay: RelayUrl = "https://relay.commy.chat:8443".parse().unwrap();
        assert_eq!(addr.relay_urls().next(), Some(&relay));
        assert_eq!(addr.ip_addrs().copied().collect::<Vec<_>>(), vec!["192.0.2.7:4433".parse().unwrap()]);
        assert_eq!(ServerTicket::decode(&ticket.encode()).unwrap(), ticket);
    }

    #[test]
    fn endpoint_addresses_survive_the_ticket() {
        let key = SecretKey::generate().public();
        let addr = EndpointAddr::from_parts(
            key,
            [
                TransportAddr::Relay("https://relay.commy.chat:8443".parse().unwrap()),
                TransportAddr::Ip("192.0.2.9:7".parse().unwrap()),
            ],
        );
        assert_eq!(NodeAddr::from(&addr).endpoint_addr(), Some(addr));
    }

    #[derive(Serialize)]
    struct FirstTicketOut {
        server_id: ServerId,
        server_name: String,
        topic: [u8; 32],
        bootstrap: Vec<NodeAddr>,
    }

    #[test]
    fn rejects_future_version() {
        let ticket = sample();
        let mut bytes = vec![99u8];
        bytes.extend(postcard::to_stdvec(&ticket).unwrap());
        let encoded = format!("commy{}", BASE32_NOPAD.encode(&bytes).to_ascii_lowercase());
        assert_eq!(
            ServerTicket::decode(&encoded),
            Err(TicketError::BadVersion(99))
        );
    }
}
