use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use commy_core::channel_key::{ChannelSecret, KeyScope, Keyring};
use commy_core::{Identity, IdentityKey, ServerId, ServerState, UserId};
use commy_sync::{StreamKeys, StreamSecret};

use crate::community_streams;
use crate::dm::DmEnvelope;
use crate::sync_streams::encode_dm;

const INBOX: &str = "commy-community-inbox/v1";
const INBOX_SEAL: &str = "commy-inbox-seal/v1";
const NONCE: [u8; 12] = [0; 12];

pub fn inbox_for(secret: &ChannelSecret, server: ServerId, user: UserId) -> StreamKeys {
    let place = postcard::to_stdvec(&(server, user)).expect("ids always encode");
    StreamSecret::derive(secret.bytes(), INBOX, &place).keys()
}

pub fn inboxes(state: &ServerState, user: UserId, ring: &Keyring, now_ms: i64) -> Vec<StreamKeys> {
    if !state.server.is_current_member(user) || state.is_deleted() {
        return Vec::new();
    }
    community_streams::epochs(state, KeyScope::Community, ring, now_ms)
        .into_iter()
        .map(|(_, secret)| inbox_for(&secret, state.server.id, user))
        .collect()
}

pub fn writable(state: &ServerState, me: UserId, to: UserId, ring: &Keyring) -> Option<StreamKeys> {
    if !state.server.is_current_member(me) || !state.server.is_current_member(to) || me == to || state.is_deleted() {
        return None;
    }
    let theirs = state.community_keys().grant_for(to)?.key;
    ring.secret(theirs).map(|secret| inbox_for(secret, state.server.id, to))
}

fn cipher(secret: [u8; 32]) -> Aes256Gcm {
    Aes256Gcm::new_from_slice(&secret).expect("32 byte key")
}

pub fn seal(to: &IdentityKey, envelope: &DmEnvelope) -> Option<Vec<u8>> {
    let once = Identity::generate();
    let secret = once.pairwise_secret(to, INBOX_SEAL)?;
    let body = cipher(secret).encrypt(&Nonce::from(NONCE), Payload { msg: &encode_dm(envelope), aad: &to.0 }).ok()?;
    let mut out = once.public().0.to_vec();
    out.extend(body);
    Some(out)
}

pub fn open(me: &Identity, sealed: &[u8]) -> Option<DmEnvelope> {
    let (once, body) = sealed.split_at_checked(32)?;
    let once = IdentityKey(once.try_into().ok()?);
    let secret = me.pairwise_secret(&once, INBOX_SEAL)?;
    let plain = cipher(secret).decrypt(&Nonce::from(NONCE), Payload { msg: body, aad: &me.public().0 }).ok()?;
    DmEnvelope::decode(&plain).filter(DmEnvelope::signed_by_sender)
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::channel_key::community_key_task;
    use commy_core::test_support::{fixture, identity_for, join};
    use commy_core::{DmMessage, Server, ServerEvent, UserProfile};

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
    fn only_the_person_it_is_for_can_open_a_first_message() {
        let alice = person("alice");
        let (bob, bob_id) = person("bob");
        let (_, carol_id) = person("carol");
        let envelope = signed(&alice, &bob, "hello from the raid");
        let sealed = seal(&bob.identity.unwrap(), &envelope).unwrap();
        assert_eq!(open(&bob_id, &sealed), Some(envelope.clone()));
        assert_eq!(open(&carol_id, &sealed), None, "other members see only noise");
        assert_ne!(seal(&bob.identity.unwrap(), &envelope).unwrap(), sealed, "each one is sealed afresh");
        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert_eq!(open(&bob_id, &tampered), None);
        assert_eq!(open(&bob_id, &sealed[..20]), None);
        let mut forged = envelope;
        forged.message.content = "changed".into();
        assert_eq!(open(&bob_id, &seal(&bob.identity.unwrap(), &forged).unwrap()), None, "the sender's signature must hold");
    }

    #[test]
    fn each_member_has_an_inbox_per_server_that_only_members_can_name() {
        let secret = ChannelSecret::from_bytes([3; 32]);
        let other = ChannelSecret::from_bytes([4; 32]);
        let (server, elsewhere) = (ServerId::new(), ServerId::new());
        let (alice, bob) = (UserId::new(), UserId::new());
        let inbox = inbox_for(&secret, server, alice);
        assert_eq!(inbox_for(&secret, server, alice).id, inbox.id);
        assert_ne!(inbox_for(&secret, server, bob).id, inbox.id);
        assert_ne!(inbox_for(&secret, elsewhere, alice).id, inbox.id);
        assert_ne!(inbox_for(&other, server, alice).id, inbox.id, "a new community key moves the inbox");
        let state = commy_core::ServerState::new(Server::new("Moonbase", alice));
        assert!(writable(&state, alice, alice, &Keyring::default()).is_none());
        assert!(writable(&state, alice, bob, &Keyring::default()).is_none(), "only to a current member");
        let mut state = state;
        state.apply(ServerEvent::MemberJoin { server: state.server.id, profile: UserProfile { id: bob, ..UserProfile::new("bob") } }).unwrap();
        assert!(writable(&state, alice, bob, &Keyring::default()).is_none(), "and only with the community key");
    }

    fn settle(state: &mut ServerState, me: UserId) -> Keyring {
        loop {
            let mut ring = Keyring::unlock_community(state, me, &identity_for(me));
            let Some(task) = community_key_task(state, me, &ring) else { return ring };
            for event in ring.task_events(state, KeyScope::Community, task, chrono::Utc::now().timestamp_millis()) {
                state.apply(event).unwrap();
            }
        }
    }

    #[test]
    fn a_member_writes_where_the_other_member_reads() {
        let (mut state, _, owner) = fixture();
        let member = join(&mut state, "member");
        settle(&mut state, owner);
        let now = chrono::Utc::now().timestamp_millis();
        let owners = Keyring::unlock_community(&state, owner, &identity_for(owner));
        let members = Keyring::unlock_community(&state, member, &identity_for(member));
        let written = writable(&state, owner, member, &owners).expect("both are members holding the key");
        let read = inboxes(&state, member, &members, now);
        assert_eq!(read.first().map(|keys| keys.id), Some(written.id));
        assert!(!inboxes(&state, member, &members, now).iter().any(|keys| keys.id == writable(&state, member, owner, &members).unwrap().id));
    }

    #[test]
    fn a_first_message_goes_where_the_recipient_reads_even_before_they_hold_the_newest_key() {
        let (mut state, _, owner) = fixture();
        let member = join(&mut state, "member");
        settle(&mut state, owner);
        let now = chrono::Utc::now().timestamp_millis();
        let mut owners = Keyring::unlock_community(&state, owner, &identity_for(owner));
        let held: Vec<_> = state.community_keys().heads();
        let record = owners.rotate(state.server.id, KeyScope::Community, &held, now + 1);
        let newest = record.id;
        state.apply(ServerEvent::CommunityKeyAdd { server: state.server.id, key: record }).unwrap();
        for event in owners.grant_events(&state, KeyScope::Community, newest, &[owner]) {
            state.apply(event).unwrap();
        }
        let owners = Keyring::unlock_community(&state, owner, &identity_for(owner));
        let members = Keyring::unlock_community(&state, member, &identity_for(member));
        assert!(owners.holds(newest) && !members.holds(newest), "the member has not been given the newest key yet");
        let written = writable(&state, owner, member, &owners).expect("the owner still holds the member's key");
        let read = inboxes(&state, member, &members, now);
        assert_eq!(read.first().map(|keys| keys.id), Some(written.id));
    }
}
