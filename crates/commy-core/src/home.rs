use crate::identity::{IdentityKey, Signature};
use crate::ids::UserId;
use crate::server_key::PublishedKey;
use serde::{Deserialize, Serialize};

pub const CARD_PURPOSE: &str = "home-card";
pub const CARD_DAYS: i64 = 30;
pub const RENEW_DAYS: i64 = 7;
pub const MAX_BLOCKED_HOMES: usize = 200;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const ORIGIN_PREFIX: &str = "https://";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeCard {
    pub home: String,
    pub user: UserId,
    pub identity: IdentityKey,
    pub key: IdentityKey,
    pub issued: i64,
    pub expires: i64,
    pub sig: Signature,
}

pub fn card_payload(user: UserId, identity: &IdentityKey, issued: i64, expires: i64) -> String {
    format!("{user}\n{}\n{issued}\n{expires}", identity.to_hex())
}

impl HomeCard {
    pub fn domain(&self) -> &str {
        self.home.strip_prefix(ORIGIN_PREFIX).unwrap_or(&self.home)
    }

    fn payload(&self) -> String {
        card_payload(self.user, &self.identity, self.issued, self.expires)
    }

    fn sensible_dates(&self) -> bool {
        self.issued > 0 && self.expires > self.issued && self.expires - self.issued <= (CARD_DAYS + 1) * DAY_MS
    }

    pub fn holds_together(&self, user: UserId, identity: Option<&IdentityKey>) -> bool {
        crate::sync_host::is_sync_host(&self.home)
            && self.user == user
            && identity == Some(&self.identity)
            && self.sensible_dates()
            && crate::server_key::verify(&self.key, CARD_PURPOSE, &self.home, self.payload().as_bytes(), &self.sig)
    }

    pub fn signed_by(&self, published: &PublishedKey) -> bool {
        published.is_well_formed() && published.public == self.key
    }

    pub fn current_at(&self, now_ms: i64) -> bool {
        self.issued <= now_ms + DAY_MS && now_ms < self.expires
    }

    pub fn due_for_renewal(&self, now_ms: i64) -> bool {
        self.expires - now_ms < (CARD_DAYS - RENEW_DAYS) * DAY_MS
    }
}

pub fn request_proof(identity: &crate::identity::Identity, home: &str, user: UserId, at: i64) -> String {
    let message = format!("commy-home-request/v1\n{home}\n{user}\n{}\n{at}", identity.public().to_hex());
    identity.sign(message.as_bytes()).to_hex()
}

pub fn wants_card(current: Option<&HomeCard>, home: &str, now_ms: i64) -> bool {
    current.is_none_or(|card| card.home != home || card.due_for_renewal(now_ms) || !card.current_at(now_ms))
}

pub fn fresh_card_fits(card: &HomeCard, home: &str, user: UserId, identity: &IdentityKey, now_ms: i64) -> bool {
    card.holds_together(user, Some(identity)) && card.home == home && card.expires > now_ms + RENEW_DAYS * DAY_MS
}

pub fn proven_home(profile: &crate::user::UserProfile) -> Option<&HomeCard> {
    profile.home.as_ref().filter(|card| card.holds_together(profile.id, profile.identity.as_ref()))
}

pub fn clean_home_domain(input: &str) -> Option<String> {
    let origin = crate::sync_host::clean_sync_host(input)?;
    Some(origin.trim_start_matches(ORIGIN_PREFIX).to_string())
}

pub fn covers(blocked: &str, domain: &str) -> bool {
    let domain = domain.trim_end_matches('.');
    domain.eq_ignore_ascii_case(blocked)
        || (domain.len() > blocked.len()
            && domain.as_bytes()[domain.len() - blocked.len() - 1] == b'.'
            && domain[domain.len() - blocked.len()..].eq_ignore_ascii_case(blocked))
}

pub fn clean_blocked_homes(homes: &[String]) -> Option<Vec<String>> {
    if homes.len() > MAX_BLOCKED_HOMES {
        return None;
    }
    let mut cleaned: Vec<String> = homes.iter().map(|home| clean_home_domain(home)).collect::<Option<_>>()?;
    cleaned.sort();
    cleaned.dedup();
    Some(cleaned)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::identity::Identity;

    pub(crate) fn card_from(server: &Identity, home: &str, user: UserId, identity: IdentityKey, issued: i64) -> HomeCard {
        let expires = issued + CARD_DAYS * DAY_MS;
        let payload = card_payload(user, &identity, issued, expires);
        let sig = crate::server_key::sign(server, CARD_PURPOSE, home, payload.as_bytes());
        HomeCard { home: home.into(), user, identity, key: server.public(), issued, expires, sig }
    }

    const NOW: i64 = 1_800_000_000_000;

    #[test]
    fn a_card_is_asked_for_when_missing_moved_or_aging() {
        let server = Identity::from_bytes(&[1; 32]);
        let person = Identity::from_bytes(&[2; 32]).public();
        let user = UserId::new();
        assert!(wants_card(None, "https://commy.chat", NOW));
        let fresh = card_from(&server, "https://commy.chat", user, person, NOW);
        assert!(!wants_card(Some(&fresh), "https://commy.chat", NOW + DAY_MS));
        assert!(wants_card(Some(&fresh), "https://friends.org", NOW));
        assert!(wants_card(Some(&fresh), "https://commy.chat", NOW + (RENEW_DAYS + 1) * DAY_MS));
        assert!(fresh_card_fits(&fresh, "https://commy.chat", user, &person, NOW));
        assert!(!fresh_card_fits(&fresh, "https://friends.org", user, &person, NOW));
        assert!(!fresh_card_fits(&fresh, "https://commy.chat", user, &person, NOW + (CARD_DAYS - RENEW_DAYS) * DAY_MS));
    }

    #[test]
    fn the_request_proof_matches_what_the_account_server_checks() {
        let identity = Identity::from_bytes(&[9; 32]);
        let user: UserId = "6f1c1d6e-2b7a-4c55-9d4e-0d8a1b2c3d4e".parse().unwrap();
        let proof = request_proof(&identity, "https://commy.chat", user, 5);
        let message = format!("commy-home-request/v1\nhttps://commy.chat\n{user}\n{}\n5", identity.public().to_hex());
        assert!(identity.public().verify(message.as_bytes(), &crate::identity::Signature::from_hex(&proof).unwrap()));
    }

    #[test]
    fn a_card_holds_only_for_its_person_and_key() {
        let server = Identity::from_bytes(&[1; 32]);
        let person = Identity::from_bytes(&[2; 32]).public();
        let user = UserId::new();
        let card = card_from(&server, "https://commy.chat", user, person, NOW);
        assert!(card.holds_together(user, Some(&person)));
        assert!(!card.holds_together(UserId::new(), Some(&person)));
        assert!(!card.holds_together(user, Some(&Identity::from_bytes(&[9; 32]).public())));
        assert!(!card.holds_together(user, None));
        assert_eq!(card.domain(), "commy.chat");
    }

    #[test]
    fn a_card_moved_to_another_home_or_stretched_stops_holding() {
        let server = Identity::from_bytes(&[1; 32]);
        let person = Identity::from_bytes(&[2; 32]).public();
        let user = UserId::new();
        let card = card_from(&server, "https://bad.example", user, person, NOW);
        let claimed = HomeCard { home: "https://commy.chat".into(), ..card.clone() };
        assert!(!claimed.holds_together(user, Some(&person)));
        let stretched = HomeCard { expires: card.expires + 365 * DAY_MS, ..card.clone() };
        assert!(!stretched.holds_together(user, Some(&person)));
        let messy = card_from(&server, "https://Bad.example/", user, person, NOW);
        assert!(!messy.holds_together(user, Some(&person)));
    }

    #[test]
    fn a_card_is_matched_to_the_key_its_home_publishes() {
        let server = Identity::from_bytes(&[1; 32]);
        let person = Identity::from_bytes(&[2; 32]).public();
        let card = card_from(&server, "https://commy.chat", UserId::new(), person, NOW);
        assert!(card.signed_by(&PublishedKey::of(server.public())));
        assert!(!card.signed_by(&PublishedKey::of(Identity::from_bytes(&[3; 32]).public())));
    }

    #[test]
    fn a_card_ages_out_and_asks_for_renewal_a_week_in() {
        let server = Identity::from_bytes(&[1; 32]);
        let card = card_from(&server, "https://commy.chat", UserId::new(), server.public(), NOW);
        assert!(card.current_at(NOW));
        assert!(!card.due_for_renewal(NOW + DAY_MS));
        assert!(card.due_for_renewal(NOW + (RENEW_DAYS + 1) * DAY_MS));
        assert!(!card.current_at(card.expires));
    }

    #[test]
    fn a_blocked_domain_covers_its_subdomains_only() {
        assert!(covers("bad.example", "bad.example"));
        assert!(covers("bad.example", "sync.bad.example"));
        assert!(covers("bad.example", "Sync.Bad.Example."));
        assert!(!covers("bad.example", "notbad.example"));
        assert!(!covers("bad.example", "bad.example.org"));
        assert!(!covers("sync.bad.example", "bad.example"));
    }

    #[test]
    fn blocked_homes_are_cleaned_sorted_and_capped() {
        let homes = vec!["https://Bad.example/".to_string(), "spam.example".into(), "bad.example".into()];
        assert_eq!(clean_blocked_homes(&homes), Some(vec!["bad.example".to_string(), "spam.example".into()]));
        assert_eq!(clean_blocked_homes(&["localhost".to_string()]), None);
        assert_eq!(clean_blocked_homes(&vec!["a.example".to_string(); MAX_BLOCKED_HOMES + 1]), None);
    }
}

#[cfg(test)]
mod blocking {
    use super::tests::card_from;
    use crate::event::ServerEvent;
    use crate::identity::Identity;
    use crate::permissions::Permissions;
    use crate::server::Role;
    use crate::store::{ServerState, StoreError};
    use crate::test_support::{fixture, keyed_profile, post, signed_event};
    use crate::user::UserProfile;
    use crate::ids::RoleId;
    use chrono::Utc;

    fn from_home(name: &str, home: &str) -> UserProfile {
        let mut profile = keyed_profile(name);
        let server = Identity::from_bytes(&[home.len() as u8; 32]);
        profile.home = Some(card_from(&server, home, profile.id, profile.identity.unwrap(), Utc::now().timestamp_millis()));
        profile
    }

    fn arrive(state: &mut ServerState, profile: &UserProfile) -> Result<bool, StoreError> {
        let event = ServerEvent::MemberJoin { server: state.server.id, profile: profile.clone() };
        state.apply_signed(&signed_event(state, profile.id, Utc::now(), event))
    }

    fn block(state: &mut ServerState, by: crate::ids::UserId, homes: &[&str], require_home: bool) -> Result<bool, StoreError> {
        let event = ServerEvent::HomesBlocked { server: state.server.id, homes: homes.iter().map(|h| h.to_string()).collect(), require_home };
        state.apply_signed(&signed_event(state, by, Utc::now(), event))
    }

    #[test]
    fn people_from_a_blocked_home_are_removed_and_kept_out() {
        let (mut state, channel, owner) = fixture();
        let spammer = from_home("spammer", "https://spam.example");
        let friend = from_home("friend", "https://commy.chat");
        assert_eq!(arrive(&mut state, &spammer), Ok(true));
        assert_eq!(arrive(&mut state, &friend), Ok(true));
        assert_eq!(block(&mut state, owner, &["Spam.example"], false), Ok(true));
        assert_eq!(state.server.blocked_homes, vec!["spam.example".to_string()]);
        assert!(!state.server.is_member(spammer.id));
        assert!(state.server.is_member(friend.id));
        assert_eq!(arrive(&mut state, &spammer), Err(StoreError::NotAllowed));
        assert!(!state.server.is_member(spammer.id));
        let spam = post(&state, channel, spammer.id, "buy now");
        assert_eq!(state.apply_unsigned(ServerEvent::MessageCreate { message: spam }), Err(StoreError::NotAllowed));
        let hello = post(&state, channel, friend.id, "hello");
        assert_eq!(state.apply_unsigned(ServerEvent::MessageCreate { message: hello }), Ok(true));
    }

    #[test]
    fn a_blocked_home_covers_its_subdomains() {
        let (mut state, _, owner) = fixture();
        block(&mut state, owner, &["spam.example"], false).unwrap();
        assert_eq!(arrive(&mut state, &from_home("sub", "https://accounts.spam.example")), Err(StoreError::NotAllowed));
        assert_eq!(arrive(&mut state, &from_home("near", "https://notspam.example")), Ok(true));
    }

    #[test]
    fn requiring_a_home_keeps_out_people_without_a_card() {
        let (mut state, _, owner) = fixture();
        let bare = keyed_profile("bare");
        assert_eq!(arrive(&mut state, &bare), Ok(true));
        assert_eq!(block(&mut state, owner, &[], true), Ok(true));
        assert!(!state.server.is_member(bare.id));
        assert!(state.server.is_member(owner), "the owner never needs a card");
        assert_eq!(arrive(&mut state, &bare), Err(StoreError::NotAllowed));
        let unsigned = ServerEvent::MemberJoin { server: state.server.id, profile: UserProfile { identity: None, ..bare.clone() } };
        assert_eq!(state.apply_unsigned(unsigned), Err(StoreError::NotAllowed));
        assert_eq!(arrive(&mut state, &from_home("carded", "https://commy.chat")), Ok(true));
    }

    #[test]
    fn a_forged_card_counts_as_no_card() {
        let (mut state, _, owner) = fixture();
        block(&mut state, owner, &[], true).unwrap();
        let mut forged = from_home("forged", "https://spam.example");
        if let Some(card) = forged.home.as_mut() {
            card.home = "https://commy.chat".into();
        }
        assert_eq!(arrive(&mut state, &forged), Err(StoreError::NotAllowed));
        assert!(!state.server.is_member(forged.id));
    }

    #[test]
    fn a_member_cannot_switch_to_a_blocked_home_card() {
        let (mut state, _, owner) = fixture();
        let friend = from_home("friend", "https://commy.chat");
        arrive(&mut state, &friend).unwrap();
        block(&mut state, owner, &["spam.example"], false).unwrap();
        let server = Identity::from_bytes(&[2; 32]);
        let moved = UserProfile {
            home: Some(card_from(&server, "https://spam.example", friend.id, friend.identity.unwrap(), Utc::now().timestamp_millis())),
            ..friend.clone()
        };
        let update = ServerEvent::ProfileUpdate { profile: moved };
        assert_eq!(state.apply_signed(&signed_event(&state, friend.id, Utc::now(), update)), Err(StoreError::NotAllowed));
        assert_eq!(state.profiles[&friend.id].home, friend.home);
    }

    #[test]
    fn only_people_who_can_ban_set_blocked_homes() {
        let (mut state, _, owner) = fixture();
        let member = from_home("member", "https://commy.chat");
        arrive(&mut state, &member).unwrap();
        assert_eq!(block(&mut state, member.id, &["spam.example"], false), Err(StoreError::NotAllowed));
        let role = Role { permissions: Permissions::BAN_MEMBERS, position: 1, ..state.server.roles[0].clone() };
        let role = Role { id: RoleId::new(), name: "mods".into(), ..role };
        let id = role.id;
        state.apply_signed(&signed_event(&state, owner, Utc::now(), ServerEvent::RoleCreate { server: state.server.id, role })).unwrap();
        let assign = ServerEvent::RoleAssign { server: state.server.id, user: member.id, role: id };
        state.apply_signed(&signed_event(&state, owner, Utc::now(), assign)).unwrap();
        assert_eq!(block(&mut state, member.id, &["spam.example"], false), Ok(true));
        assert_eq!(block(&mut state, owner, &["localhost"], false), Ok(false), "an unusable address changes nothing");
        assert_eq!(state.server.blocked_homes, vec!["spam.example".to_string()]);
    }

    #[test]
    fn people_from_a_blocked_home_may_still_leave() {
        let (mut state, _, _) = fixture();
        let spammer = from_home("spammer", "https://spam.example");
        arrive(&mut state, &spammer).unwrap();
        state.server.blocked_homes = vec!["spam.example".into()];
        let leave = ServerEvent::MemberLeave { server: state.server.id, user: spammer.id };
        let later = Utc::now() + chrono::Duration::seconds(1);
        assert_eq!(state.apply_signed(&signed_event(&state, spammer.id, later, leave)), Ok(true));
    }
}
