use anyhow::{bail, Context, Result};
use commy_core::identity::{Identity, IdentityKey, Signature};
use commy_core::ServerId;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

use crate::media::{read_frame, write_frame};
use crate::node::accept_error;

pub const MIRROR_ALPN: &[u8] = b"commy/mirror/1";

pub const MIRROR_MAX_AGE_MS: i64 = 5 * 60 * 1000;

pub const MAX_SNAPSHOT_BYTES: usize = crate::media::MAX_MEDIA_FRAME as usize - 1024;

const SIGNING_DOMAIN: &[u8] = b"commy-mirror/v1";
const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);
const DIAL_TIMEOUT: Duration = Duration::from_secs(20);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(120);
const REPLY_DELIVERY_GRACE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MirrorRequest {
    pub account: IdentityKey,
    pub requester: [u8; 32],
    pub responder: [u8; 32],
    pub at_ms: i64,
    pub servers: Vec<ServerId>,
    pub sig: Signature,
}

impl MirrorRequest {
    pub fn new(identity: &Identity, requester: [u8; 32], responder: [u8; 32], at_ms: i64, servers: Vec<ServerId>) -> Self {
        let account = identity.public();
        let sig = identity.sign(&signed_bytes(&account, &requester, &responder, at_ms, &servers));
        Self { account, requester, responder, at_ms, servers, sig }
    }

    pub fn is_from(&self, own: &IdentityKey, now_ms: i64) -> bool {
        self.account == *own
            && (now_ms - self.at_ms).abs() <= MIRROR_MAX_AGE_MS
            && own.verify(
                &signed_bytes(&self.account, &self.requester, &self.responder, self.at_ms, &self.servers),
                &self.sig,
            )
    }
}

fn signed_bytes(account: &IdentityKey, requester: &[u8; 32], responder: &[u8; 32], at_ms: i64, servers: &[ServerId]) -> Vec<u8> {
    let body = postcard::to_allocvec(&(account, requester, responder, at_ms, servers))
        .expect("mirror request serialization cannot fail");
    [SIGNING_DOMAIN, &body].concat()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MirrorFrame {
    Server { server: ServerId, snapshot: Vec<u8> },
    Refused,
    End,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MirroredServer {
    pub server: ServerId,
    pub snapshot: Vec<u8>,
}

#[derive(Debug)]
pub struct MirrorAsk {
    pub peer: [u8; 32],
    pub request: MirrorRequest,
    pub reply: oneshot::Sender<Option<Vec<MirroredServer>>>,
}

#[derive(Debug)]
pub(crate) struct MirrorAcceptor {
    pub(crate) tx: mpsc::UnboundedSender<MirrorAsk>,
    pub(crate) local: [u8; 32],
}

impl ProtocolHandler for MirrorAcceptor {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        self.answer(connection).await.map_err(accept_error)
    }
}

impl MirrorAcceptor {
    async fn answer(&self, connection: Connection) -> Result<()> {
        let peer = *connection.remote_id().as_bytes();
        let (mut send, mut recv) = connection.accept_bi().await?;
        let request: MirrorRequest = read_frame(&mut recv)
            .await
            .map_err(|e| anyhow::anyhow!("bad mirror request: {e}"))?
            .context("peer closed before asking")?;
        let addressed = request.requester == peer && request.responder == self.local;
        let servers = if addressed { self.ask(peer, request).await } else { None };
        match servers {
            Some(servers) => {
                for item in servers {
                    let frame = MirrorFrame::Server { server: item.server, snapshot: item.snapshot };
                    write_frame(&mut send, &frame).await.map_err(|e| anyhow::anyhow!("mirror send failed: {e}"))?;
                }
                write_frame(&mut send, &MirrorFrame::End).await
            }
            None => write_frame(&mut send, &MirrorFrame::Refused).await,
        }
        .map_err(|e| anyhow::anyhow!("mirror reply failed: {e}"))?;
        send.finish()?;
        let _ = n0_future::time::timeout(REPLY_DELIVERY_GRACE, connection.closed()).await;
        Ok(())
    }

    async fn ask(&self, peer: [u8; 32], request: MirrorRequest) -> Option<Vec<MirroredServer>> {
        let (reply, answer) = oneshot::channel();
        self.tx.send(MirrorAsk { peer, request, reply }).ok()?;
        n0_future::time::timeout(ANSWER_TIMEOUT, answer).await.ok()?.ok()?
    }
}

pub async fn fetch(endpoint: &iroh::Endpoint, node_hex: &str, request: &MirrorRequest) -> Result<Vec<MirroredServer>> {
    let node_id = crate::node::parse_node_hex(node_hex)?;
    let connection = n0_future::time::timeout(DIAL_TIMEOUT, endpoint.connect(node_id, MIRROR_ALPN))
        .await
        .context("device did not answer in time")?
        .context("device is unreachable")?;
    let (mut send, mut recv) = connection.open_bi().await?;
    write_frame(&mut send, request)
        .await
        .map_err(|e| anyhow::anyhow!("mirror request failed: {e}"))?;
    let received = n0_future::time::timeout(TRANSFER_TIMEOUT, receive(&mut recv))
        .await
        .context("device stopped sending")?;
    connection.close(0u32.into(), b"done");
    received
}

async fn receive(recv: &mut iroh::endpoint::RecvStream) -> Result<Vec<MirroredServer>> {
    let mut servers = Vec::new();
    loop {
        let frame = read_frame::<_, MirrorFrame>(recv)
            .await
            .map_err(|e| anyhow::anyhow!("mirror frame failed: {e}"))?
            .context("device closed early")?;
        match frame {
            MirrorFrame::Server { server, snapshot } => servers.push(MirroredServer { server, snapshot }),
            MirrorFrame::End => return Ok(servers),
            MirrorFrame::Refused => bail!("device refused"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn request(identity: &Identity, at_ms: i64) -> MirrorRequest {
        MirrorRequest::new(identity, [1; 32], [2; 32], at_ms, vec![ServerId::new()])
    }

    #[test]
    fn accepts_a_fresh_request_signed_by_the_same_account() {
        let identity = Identity::generate();
        assert!(request(&identity, NOW).is_from(&identity.public(), NOW + 1000));
    }

    #[test]
    fn rejects_another_account() {
        let identity = Identity::generate();
        let other = Identity::generate();
        assert!(!request(&other, NOW).is_from(&identity.public(), NOW));
    }

    #[test]
    fn rejects_an_old_request() {
        let identity = Identity::generate();
        assert!(!request(&identity, NOW).is_from(&identity.public(), NOW + MIRROR_MAX_AGE_MS + 1));
    }

    #[test]
    fn rejects_a_request_retargeted_to_another_device() {
        let identity = Identity::generate();
        let mut moved = request(&identity, NOW);
        moved.requester = [9; 32];
        assert!(!moved.is_from(&identity.public(), NOW));
        let mut widened = request(&identity, NOW);
        widened.servers.push(ServerId::new());
        assert!(!widened.is_from(&identity.public(), NOW));
    }

    #[tokio::test]
    async fn frames_round_trip_over_a_stream() {
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);
        let server = ServerId::new();
        let frame = MirrorFrame::Server { server, snapshot: vec![7; 40_000] };
        write_frame(&mut a, &frame).await.unwrap();
        let got: MirrorFrame = read_frame(&mut b).await.unwrap().unwrap();
        assert_eq!(got, frame);
    }
}
