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
pub const MAX_BOOTSTRAP: usize = 6;
pub const MAX_INVITE_NEIGHBORS: usize = 4;
const MAX_DIRECT_ADDRESSES: usize = 4;

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

    pub fn via_relay(node_id: [u8; 32], relay_url: Option<String>) -> Self {
        Self { node_id, info: AddrInfo { relay_url, direct_addresses: BTreeSet::new() } }
    }

    fn trimmed(mut self) -> Self {
        self.info.direct_addresses = std::mem::take(&mut self.info.direct_addresses).into_iter().take(MAX_DIRECT_ADDRESSES).collect();
        self
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

    pub fn reaching(&self, inviter: Option<NodeAddr>, neighbors: impl IntoIterator<Item = NodeAddr>) -> Self {
        let known = |addr: NodeAddr| self.bootstrap.iter().find(|held| held.node_id == addr.node_id && addr.info.direct_addresses.is_empty()).cloned().unwrap_or(addr);
        let neighbors = neighbors.into_iter().take(MAX_INVITE_NEIGHBORS).map(known);
        let mut bootstrap: Vec<NodeAddr> = Vec::with_capacity(MAX_BOOTSTRAP);
        for addr in inviter.into_iter().chain(neighbors).chain(self.bootstrap.iter().cloned()) {
            if bootstrap.len() == MAX_BOOTSTRAP {
                break;
            }
            if !bootstrap.iter().any(|held| held.node_id == addr.node_id) {
                bootstrap.push(addr.trimmed());
            }
        }
        Self { bootstrap, ..self.clone() }
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

    fn node_at(byte: u8, ips: usize) -> NodeAddr {
        let direct_addresses = (0..ips).map(|i| SocketAddr::from(([192, 0, 2, byte], 7000 + i as u16))).collect();
        NodeAddr { node_id: [byte; 32], info: AddrInfo { relay_url: Some("https://relay.commy.chat./".into()), direct_addresses } }
    }

    #[test]
    fn a_member_invite_leads_with_the_inviter_and_live_neighbors_then_keeps_the_owner() {
        let owner = node_at(1, 2);
        let mut ticket = sample();
        ticket.bootstrap = vec![owner.clone()];
        let (member, friend) = (node_at(2, 1), node_at(3, 1));
        let shared = ticket.reaching(Some(member.clone()), [friend.clone()]);
        assert_eq!(shared.bootstrap, vec![member, friend, owner]);
        assert_eq!((shared.server_id, shared.topic, shared.owner), (ticket.server_id, ticket.topic, ticket.owner));
        assert_eq!(ServerTicket::decode(&shared.encode()).unwrap(), shared);
    }

    #[test]
    fn a_neighbor_known_only_by_id_keeps_the_addresses_the_invite_already_had() {
        let owner = node_at(1, 2);
        let mut ticket = sample();
        ticket.bootstrap = vec![owner.clone()];
        let shared = ticket.reaching(Some(node_at(2, 1)), [NodeAddr::via_relay(owner.node_id, None)]);
        assert_eq!(shared.bootstrap, vec![node_at(2, 1), owner]);
    }

    #[test]
    fn the_owner_inviting_is_listed_once_with_fresh_addresses() {
        let mut ticket = sample();
        ticket.bootstrap = vec![node_at(1, 1)];
        let fresh = node_at(1, 3);
        assert_eq!(ticket.reaching(Some(fresh.clone()), []).bootstrap, vec![fresh]);
    }

    #[test]
    fn invites_stay_small_however_many_devices_are_around() {
        let mut ticket = sample();
        ticket.bootstrap = (10..20).map(|b| node_at(b, 9)).collect();
        let shared = ticket.reaching(Some(node_at(1, 9)), (2..10).map(|b| node_at(b, 9)));
        assert_eq!(shared.bootstrap.len(), MAX_BOOTSTRAP);
        assert_eq!(shared.bootstrap[0].node_id, [1; 32]);
        assert_eq!(shared.bootstrap.iter().filter(|a| (2..10).contains(&a.node_id[0])).count(), MAX_INVITE_NEIGHBORS);
        assert_eq!(shared.bootstrap[MAX_BOOTSTRAP - 1].node_id, [10; 32], "the invite this one grew from still counts");
        assert!(shared.bootstrap.iter().all(|a| a.info.direct_addresses.len() <= MAX_DIRECT_ADDRESSES));
        assert!(shared.encode().len() < 2400, "{} characters", shared.encode().len());
    }
}
