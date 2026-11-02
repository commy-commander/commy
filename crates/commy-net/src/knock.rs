use anyhow::{Context, Result};
use commy_core::action::SignedEvent;
use commy_core::{ServerEvent, ServerId, ServerState, UserId};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

use crate::media::{read_frame, write_frame};
use crate::node::accept_error;

pub const KNOCK_ALPN: &[u8] = b"commy/knock/1";

pub const MAX_RULES_BYTES: usize = 4000;
pub const KNOCK_TIMEOUT: Duration = Duration::from_secs(8);
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);
const REPLY_DELIVERY_GRACE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnockRequest {
    pub server: ServerId,
    pub ask: SignedEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lobby {
    pub name: String,
    pub rules: Option<String>,
    pub members: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KnockReply {
    Open,
    Waiting(Lobby),
    Admitted,
    TurnedAway,
    Unknown,
}

impl KnockReply {
    pub fn decisive(&self) -> bool {
        !matches!(self, KnockReply::Unknown)
    }
}

#[derive(Debug)]
pub struct KnockAsk {
    pub peer: [u8; 32],
    pub request: KnockRequest,
    pub reply: oneshot::Sender<KnockReply>,
}

pub fn asker(request: &KnockRequest) -> Option<UserId> {
    match &request.ask.event {
        ServerEvent::JoinAsk { server, profile, .. } if *server == request.server && profile.id == request.ask.action.by => Some(profile.id),
        _ => None,
    }
}

pub fn lobby(state: &ServerState) -> Lobby {
    let rules = state.server.rules.as_deref().map(|r| clip(r, MAX_RULES_BYTES));
    let members = state.server.members.iter().filter(|m| state.server.is_current_member(m.user)).count();
    Lobby { name: state.server.name.clone(), rules, members: members.try_into().unwrap_or(u32::MAX) }
}

pub fn answer(state: &ServerState, me: UserId, request: &KnockRequest) -> KnockReply {
    let server = &state.server;
    if request.server != server.id || state.is_deleted() || !server.is_current_member(me) {
        return KnockReply::Unknown;
    }
    let Some(user) = asker(request) else { return KnockReply::Unknown };
    if server.is_banned(user) || server.turned_away.contains(&user) {
        return KnockReply::TurnedAway;
    }
    if server.is_member(user) {
        return KnockReply::Admitted;
    }
    if !server.join_approval {
        return KnockReply::Open;
    }
    KnockReply::Waiting(lobby(state))
}

fn clip(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

#[derive(Debug)]
pub(crate) struct KnockAcceptor {
    pub(crate) tx: mpsc::UnboundedSender<KnockAsk>,
}

impl ProtocolHandler for KnockAcceptor {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        self.answer(connection).await.map_err(accept_error)
    }
}

impl KnockAcceptor {
    async fn answer(&self, connection: Connection) -> Result<()> {
        let peer = *connection.remote_id().as_bytes();
        let (mut send, mut recv) = connection.accept_bi().await?;
        let request: KnockRequest = read_frame(&mut recv)
            .await
            .map_err(|e| anyhow::anyhow!("bad knock: {e}"))?
            .context("peer closed before knocking")?;
        let (reply, answer) = oneshot::channel();
        let reply = match self.tx.send(KnockAsk { peer, request, reply }) {
            Ok(()) => n0_future::time::timeout(ANSWER_TIMEOUT, answer).await.ok().and_then(|r| r.ok()).unwrap_or(KnockReply::Unknown),
            Err(_) => KnockReply::Unknown,
        };
        write_frame(&mut send, &reply).await.map_err(|e| anyhow::anyhow!("knock reply failed: {e}"))?;
        send.finish()?;
        let _ = n0_future::time::timeout(REPLY_DELIVERY_GRACE, connection.closed()).await;
        Ok(())
    }
}

pub(crate) async fn knock_one(endpoint: iroh::Endpoint, node: iroh::EndpointId, request: KnockRequest) -> Result<KnockReply> {
    let connection = n0_future::time::timeout(KNOCK_TIMEOUT, endpoint.connect(node, KNOCK_ALPN))
        .await
        .context("device did not answer in time")?
        .context("device is unreachable")?;
    let (mut send, mut recv) = connection.open_bi().await?;
    write_frame(&mut send, &request).await.map_err(|e| anyhow::anyhow!("knock failed: {e}"))?;
    let reply = n0_future::time::timeout(ANSWER_TIMEOUT + KNOCK_TIMEOUT, read_frame::<_, KnockReply>(&mut recv))
        .await
        .context("no answer in time")?
        .map_err(|e| anyhow::anyhow!("knock reply failed: {e}"))?
        .context("device closed before answering")?;
    connection.close(0u32.into(), b"done");
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::test_support::{fixture, identity_for, join, keyed_profile};
    use commy_core::UserProfile;

    fn knock(state: &ServerState, profile: &UserProfile) -> KnockRequest {
        let server = state.server.id;
        let event = ServerEvent::JoinAsk { server, profile: profile.clone(), note: "hi".into(), at: 1 };
        KnockRequest { server, ask: SignedEvent::sign(server, profile.id, 1, event, &identity_for(profile.id)) }
    }

    #[test]
    fn members_tell_a_knocker_whether_to_come_in_wait_or_go() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let friend = join(&mut state, "friend");
        let stranger = keyed_profile("stranger");
        assert_eq!(answer(&state, owner, &knock(&state, &stranger)), KnockReply::Open);
        state.server.rules = Some("Be kind".into());
        state.apply(ServerEvent::JoinApprovalSet { server: sid, on: true, at: 1 }).unwrap();
        let KnockReply::Waiting(lobby) = answer(&state, friend, &knock(&state, &stranger)) else { panic!("expected to wait") };
        assert_eq!((lobby.name.as_str(), lobby.rules.as_deref(), lobby.members), ("test", Some("Be kind"), 2));
        assert_eq!(answer(&state, stranger.id, &knock(&state, &stranger)), KnockReply::Unknown, "only members answer");
        state.apply(ServerEvent::JoinAnswer { server: sid, user: stranger.id, admit: true }).unwrap();
        assert_eq!(answer(&state, owner, &knock(&state, &stranger)), KnockReply::Admitted);
        let other = keyed_profile("other");
        state.apply(ServerEvent::JoinAnswer { server: sid, user: other.id, admit: false }).unwrap();
        assert_eq!(answer(&state, owner, &knock(&state, &other)), KnockReply::TurnedAway);
    }

    #[test]
    fn a_knock_must_be_the_askers_own_and_for_this_server() {
        let (state, _, owner) = fixture();
        let stranger = keyed_profile("stranger");
        let mut moved = knock(&state, &stranger);
        moved.server = ServerId::new();
        assert_eq!(answer(&state, owner, &moved), KnockReply::Unknown);
        let mut borrowed = knock(&state, &stranger);
        borrowed.ask.action.by = UserId::new();
        assert_eq!(asker(&borrowed), None);
    }

    #[test]
    fn long_rules_are_clipped_on_a_character_boundary() {
        let text = "\u{e9}".repeat(MAX_RULES_BYTES);
        let clipped = clip(&text, MAX_RULES_BYTES - 1);
        assert!(clipped.len() <= MAX_RULES_BYTES - 1 && clipped.chars().all(|c| c == '\u{e9}'));
    }

    #[tokio::test]
    async fn knock_frames_round_trip() {
        let (state, _, _) = fixture();
        let request = knock(&state, &keyed_profile("stranger"));
        let (mut a, mut b) = tokio::io::duplex(16 * 1024);
        write_frame(&mut a, &request).await.unwrap();
        let got: KnockRequest = read_frame(&mut b).await.unwrap().unwrap();
        assert_eq!(got, request);
        let reply = KnockReply::Waiting(Lobby { name: "x".into(), rules: None, members: 3 });
        write_frame(&mut b, &reply).await.unwrap();
        assert_eq!(read_frame::<_, KnockReply>(&mut a).await.unwrap(), Some(reply));
    }
}
