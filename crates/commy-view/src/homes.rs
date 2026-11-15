use commy_core::event::ServerEvent;
use commy_core::home::{clean_blocked_homes, proven_home};
use commy_core::server_blocklist::{official_key, Envelope, ServerBlocklist};
use commy_core::server_key::PublishedKey;
use std::collections::HashMap;
use commy_core::permissions::Permissions;
use commy_core::{ServerState, UserId, UserProfile};
use serde::Serialize;

pub const OFFICIAL_HOMES: &[&str] = &["https://commy.chat"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HomeTag {
    pub home: String,
    pub shown: String,
    pub key: String,
    pub lookalike: bool,
    pub current: bool,
}

pub fn tag_of(profile: &UserProfile, now_ms: i64) -> Option<HomeTag> {
    let card = proven_home(profile)?;
    let domain = card.domain();
    Some(HomeTag {
        home: card.home.clone(),
        shown: crate::links::unicode_host(domain),
        key: commy_core::server_key::key_id(&card.key),
        lookalike: crate::links::imitates_home(domain, OFFICIAL_HOMES),
        current: card.current_at(now_ms),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlockedHomesView {
    pub homes: Vec<String>,
    pub require_home: bool,
    pub can_edit: bool,
}

pub fn blocked_homes_view(state: &ServerState, viewer: Option<UserId>) -> BlockedHomesView {
    let can_edit = viewer.is_some_and(|me| state.server.permissions_for(me).contains(Permissions::BAN_MEMBERS));
    BlockedHomesView { homes: state.server.blocked_homes.clone(), require_home: state.server.require_home, can_edit }
}

pub fn set_blocked_homes(state: &ServerState, perms: Permissions, me: UserId, homes: &[String], require_home: bool) -> Result<ServerEvent, String> {
    if !perms.contains(Permissions::BAN_MEMBERS) {
        return Err("Only people who can ban members can block servers".into());
    }
    let cleaned = clean_blocked_homes(homes).ok_or("Use a server address like example.org (up to 200)")?;
    if let Some(own) = state.profiles.get(&me).and_then(proven_home) {
        if cleaned.iter().any(|blocked| commy_core::home::covers(blocked, own.domain())) {
            return Err("That would block your own home server".into());
        }
    }
    Ok(ServerEvent::HomesBlocked { server: state.server.id, homes: cleaned, require_home })
}

pub fn members_from(state: &ServerState, domain: &str) -> usize {
    state
        .server
        .members
        .iter()
        .filter_map(|m| state.profiles.get(&m.user))
        .filter_map(proven_home)
        .filter(|card| commy_core::home::covers(domain, card.domain()))
        .count()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HomeStatus {
    pub home: String,
    pub key: Option<String>,
    pub checked: bool,
    pub blocked: bool,
}

#[derive(Debug, Clone, Default)]
pub struct HomeGuard {
    list: Option<ServerBlocklist>,
    resolved: HashMap<String, Option<PublishedKey>>,
}

pub const LIST_KEY_CHANGED: &str = "the server blocklist key does not match the one built into Commy";
pub const LIST_KEY_BAD: &str = "the server blocklist key is not an ed25519 key";
pub const LIST_SIGNATURE_BAD: &str = "the server blocklist signature does not check out";

impl HomeGuard {
    pub fn take_list(&mut self, origin: &str, list_key: &PublishedKey, envelope: &Envelope) -> Result<bool, &'static str> {
        if !list_key.is_well_formed() {
            return Err(LIST_KEY_BAD);
        }
        if official_key(origin).is_some_and(|pinned| pinned != list_key.public) {
            return Err(LIST_KEY_CHANGED);
        }
        let list = envelope.open(origin, &list_key.public).ok_or(LIST_SIGNATURE_BAD)?;
        if self.list.as_ref().is_some_and(|held| !list.newer_than(held)) {
            return Ok(false);
        }
        self.list = Some(list);
        Ok(true)
    }

    pub fn list(&self) -> Option<&ServerBlocklist> {
        self.list.as_ref()
    }

    pub fn blocks_domain(&self, domain: &str) -> bool {
        self.list.as_ref().is_some_and(|list| list.blocks(domain))
    }

    pub fn blocks_origin(&self, origin: &str) -> bool {
        self.list.as_ref().is_some_and(|list| list.blocks_origin(origin))
    }

    pub fn remember_key(&mut self, home: &str, key: Option<PublishedKey>) {
        if self.resolved.len() >= MAX_REMEMBERED_HOMES && !self.resolved.contains_key(home) {
            self.resolved.clear();
        }
        self.resolved.insert(home.to_string(), key.filter(PublishedKey::is_well_formed));
    }

    pub fn needs_lookup(&self, home: &str) -> bool {
        self.listed_key(home).is_none() && !self.resolved.contains_key(home)
    }

    fn listed_key(&self, home: &str) -> Option<&PublishedKey> {
        self.list.as_ref().and_then(|list| list.known_key(home))
    }

    pub fn key_of(&self, home: &str) -> Option<&PublishedKey> {
        self.listed_key(home).or_else(|| self.resolved.get(home).and_then(Option::as_ref))
    }

    pub fn status(&self, home: &str) -> HomeStatus {
        let domain = commy_core::home::clean_home_domain(home).unwrap_or_default();
        HomeStatus {
            home: home.to_string(),
            key: self.key_of(home).map(|key| key.id.clone()),
            checked: !self.needs_lookup(home),
            blocked: !domain.is_empty() && self.blocks_domain(&domain),
        }
    }

    pub fn hides(&self, profile: &UserProfile) -> bool {
        proven_home(profile).is_some_and(|card| self.blocks_domain(card.domain()))
    }
}

const MAX_REMEMBERED_HOMES: usize = 2000;

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::home::{card_payload, HomeCard, CARD_PURPOSE};
    use commy_core::{Identity, Server};

    fn keyed_profile(name: &str) -> UserProfile {
        UserProfile { identity: Some(Identity::generate().public()), ..UserProfile::new(name) }
    }

    const DAY_MS: i64 = 24 * 60 * 60 * 1000;

    fn carded(name: &str, home: &str, issued: i64) -> UserProfile {
        let mut profile = keyed_profile(name);
        let server = Identity::from_bytes(&[home.len() as u8; 32]);
        let identity = profile.identity.unwrap();
        let expires = issued + 30 * DAY_MS;
        let sig = commy_core::server_key::sign(&server, CARD_PURPOSE, home, card_payload(profile.id, &identity, issued, expires).as_bytes());
        profile.home = Some(HomeCard { home: home.into(), user: profile.id, identity, key: server.public(), issued, expires, sig });
        profile
    }

    #[test]
    fn a_proven_card_becomes_a_tag_with_its_key() {
        let profile = carded("a", "https://commy.chat", 1_000);
        let tag = tag_of(&profile, 2_000).unwrap();
        assert_eq!(tag.home, "https://commy.chat");
        assert_eq!(tag.shown, "commy.chat");
        assert!(!tag.lookalike && tag.current);
        assert_eq!(tag.key.len(), 16);
        assert!(!tag_of(&profile, 1_000 + 31 * DAY_MS).unwrap().current);
    }

    #[test]
    fn a_look_alike_home_is_flagged_and_a_bad_card_shows_nothing() {
        assert!(tag_of(&carded("b", "https://cornmy.chat", 1_000), 2_000).unwrap().lookalike);
        let mut forged = carded("c", "https://friends.org", 1_000);
        forged.home.as_mut().unwrap().home = "https://commy.chat".into();
        assert_eq!(tag_of(&forged, 2_000), None);
        assert_eq!(tag_of(&keyed_profile("d"), 2_000), None);
    }

    #[test]
    fn owners_cannot_block_their_own_home() {
        let me = carded("owner", "https://accounts.friends.org", chrono::Utc::now().timestamp_millis());
        let mut state = ServerState::new(Server::new("t", me.id));
        state.profiles.insert(me.id, me.clone());
        assert!(set_blocked_homes(&state, Permissions::all(), me.id, &["spam.org".into()], false).is_ok());
        assert!(set_blocked_homes(&state, Permissions::all(), me.id, &["friends.org".into()], false).is_err());
        assert!(set_blocked_homes(&state, Permissions::all(), me.id, &["localhost".into()], false).is_err());
        assert!(set_blocked_homes(&state, Permissions::default(), keyed_profile("stranger").id, &["spam.org".into()], false).is_err());
        assert_eq!(members_from(&state, "friends.org"), 1);
    }

    #[test]
    fn the_settings_view_says_who_may_edit() {
        let owner = UserId::new();
        let state = ServerState::new(Server::new("t", owner));
        assert!(blocked_homes_view(&state, Some(owner)).can_edit);
        assert!(!blocked_homes_view(&state, None).can_edit);
    }

    fn sealed(signer: &Identity, origin: &str, issued: i64, domains: &[&str]) -> Envelope {
        let list = ServerBlocklist {
            v: commy_core::server_blocklist::LIST_VERSION,
            issued,
            expires: issued + 7 * DAY_MS,
            domains: domains.iter().map(|d| commy_core::server_blocklist::BlockedServer { domain: d.to_string(), since: 0 }).collect(),
            homes: vec![commy_core::server_blocklist::KnownHome { home: origin.into(), key: PublishedKey::of(Identity::from_bytes(&[2; 32]).public()) }],
        };
        let text = serde_json::to_string(&list).unwrap();
        let sig = commy_core::server_key::sign(signer, commy_core::server_blocklist::LIST_PURPOSE, origin, text.as_bytes());
        Envelope { list: text, key: commy_core::server_key::key_id(&signer.public()), sig }
    }

    #[test]
    fn the_guard_keeps_the_newest_signed_list() {
        let signer = Identity::from_bytes(&[1; 32]);
        let key = PublishedKey::of(signer.public());
        let origin = "https://staging.example.org";
        let mut guard = HomeGuard::default();
        assert!(!guard.blocks_domain("spam.org"));
        assert_eq!(guard.take_list(origin, &key, &sealed(&signer, origin, 10, &["spam.org"])), Ok(true));
        assert!(guard.blocks_domain("relay.spam.org"));
        assert!(guard.blocks_origin("wss://sync.spam.org/sync"));
        assert_eq!(guard.take_list(origin, &key, &sealed(&signer, origin, 5, &[])), Ok(false), "an older list never replaces a newer one");
        assert!(guard.blocks_domain("spam.org"));
        let stranger = Identity::from_bytes(&[3; 32]);
        assert_eq!(guard.take_list(origin, &key, &sealed(&stranger, origin, 20, &[])), Err(LIST_SIGNATURE_BAD));
        let renamed = PublishedKey { id: "0000000000000000".into(), ..key.clone() };
        assert_eq!(guard.take_list(origin, &renamed, &sealed(&signer, origin, 20, &[])), Err(LIST_KEY_BAD));
    }

    #[test]
    fn listed_homes_need_no_lookup_and_others_are_remembered() {
        let signer = Identity::from_bytes(&[1; 32]);
        let origin = "https://staging.example.org";
        let mut guard = HomeGuard::default();
        guard.take_list(origin, &PublishedKey::of(signer.public()), &sealed(&signer, origin, 10, &["spam.org"])).unwrap();
        assert!(!guard.needs_lookup(origin));
        assert!(guard.status(origin).key.is_some());
        assert!(guard.needs_lookup("https://friends.org"));
        guard.remember_key("https://friends.org", None);
        let status = guard.status("https://friends.org");
        assert!(status.checked && status.key.is_none() && !status.blocked);
        assert!(guard.status("https://spam.org").blocked);
    }

    #[test]
    fn the_guard_hides_people_from_blocked_homes() {
        let signer = Identity::from_bytes(&[1; 32]);
        let origin = "https://staging.example.org";
        let mut guard = HomeGuard::default();
        guard.take_list(origin, &PublishedKey::of(signer.public()), &sealed(&signer, origin, 10, &["spam.org"])).unwrap();
        assert!(guard.hides(&carded("s", "https://spam.org", 1_000)));
        assert!(!guard.hides(&carded("f", "https://friends.org", 1_000)));
        assert!(!guard.hides(&keyed_profile("bare")));
    }

}
