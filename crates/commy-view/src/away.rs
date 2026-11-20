use std::collections::{BTreeMap, HashMap};

use commy_core::home::{proven_home, HomeCard};
use commy_core::sync_host::{clean_sync_host, socket_url};
use commy_core::{Avatar, ServerState, UserProfile};
use commy_sync::endpoint::accepted_url;
use commy_sync::wake::RingTags;
use serde::Deserialize;

pub const DISCOVERY_PATH: &str = "/.well-known/commy-auth.json";
pub const LOOK_AGAIN_MS: i64 = 6 * 60 * 60 * 1000;
pub const RETRY_MS: i64 = 10 * 60 * 1000;
pub const FIRST_RETRY_MS: i64 = 15 * 1000;
pub const MAX_AWAY_HOMES: usize = 64;

pub fn home_of(profile: &UserProfile) -> Option<&str> {
    proven_home(profile).map(|card| card.home.as_str())
}

fn origin_form(origin: &str) -> String {
    origin.trim().trim_end_matches('/').to_ascii_lowercase()
}

pub fn same_home(a: &str, b: &str) -> bool {
    origin_form(a) == origin_form(b)
}

pub fn newest_home<'a>(profile: &'a UserProfile, copies: impl IntoIterator<Item = &'a UserProfile>) -> Option<HomeCard> {
    std::iter::once(profile)
        .chain(copies)
        .filter_map(|copy| copy.home.as_ref())
        .filter(|card| card.holds_together(profile.id, profile.identity.as_ref()))
        .max_by_key(|card| card.issued)
        .cloned()
}

pub fn away_home(mine: &str, profile: &UserProfile) -> Option<String> {
    home_of(profile).filter(|theirs| !same_home(mine, theirs)).map(str::to_string)
}

pub fn shows_picture(profile: &UserProfile, blob: &str) -> bool {
    [profile.avatar.as_deref(), profile.banner.as_deref()]
        .into_iter()
        .flatten()
        .any(|raw| matches!(Avatar::parse(raw), Some(Avatar::Image { blob: shown }) if shown == blob))
}

pub fn picture_home<'a>(profiles: impl IntoIterator<Item = &'a UserProfile>, mine: &str, blob: &str) -> Option<String> {
    profiles.into_iter().filter(|profile| shows_picture(profile, blob)).find_map(|profile| away_home(mine, profile))
}

pub fn discovery_url(home: &str) -> String {
    format!("{}{DISCOVERY_PATH}", home.trim_end_matches('/'))
}

pub fn cursor_key(stream_hex: &str, url: &str) -> String {
    let host = url.split_once("://").map_or(url, |(_, rest)| rest).split('/').next().unwrap_or_default();
    format!("{stream_hex}@{host}")
}

#[derive(Deserialize)]
struct Endpoints {
    #[serde(default)]
    sync: Option<String>,
}

#[derive(Deserialize)]
struct HomeDoc {
    issuer: String,
    endpoints: Endpoints,
}

pub fn sync_url_of(home: &str, doc: &str) -> Option<String> {
    let doc: HomeDoc = serde_json::from_str(doc).ok()?;
    if !same_home(&doc.issuer, home) {
        return None;
    }
    accepted_url(home, doc.endpoints.sync.as_deref())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Known {
    url: Option<String>,
    at: i64,
    misses: u32,
}

fn retry_after(misses: u32) -> i64 {
    FIRST_RETRY_MS.saturating_mul(1 << misses.saturating_sub(1).min(10)).min(RETRY_MS)
}

#[derive(Debug, Clone, Default)]
pub struct HomeBook {
    known: HashMap<String, Known>,
    asking: HashMap<String, i64>,
}

impl HomeBook {
    pub fn url(&self, home: &str) -> Option<&str> {
        self.known.get(&origin_form(home)).and_then(|known| known.url.as_deref())
    }

    fn fresh(&self, home: &str, now_ms: i64) -> bool {
        self.known.get(home).is_some_and(|known| {
            let wait = if known.url.is_some() { LOOK_AGAIN_MS } else { retry_after(known.misses) };
            now_ms - known.at < wait
        })
    }

    fn being_asked(&self, home: &str, now_ms: i64) -> bool {
        self.asking.get(home).is_some_and(|since| now_ms - since < RETRY_MS)
    }

    pub fn to_ask<'a>(&mut self, homes: impl IntoIterator<Item = &'a str>, now_ms: i64) -> Vec<String> {
        let mut due: Vec<String> = homes
            .into_iter()
            .map(origin_form)
            .filter(|home| !self.fresh(home, now_ms) && !self.being_asked(home, now_ms))
            .collect();
        due.sort();
        due.dedup();
        due.truncate(MAX_AWAY_HOMES);
        for home in &due {
            self.asking.insert(home.clone(), now_ms);
        }
        due
    }

    pub fn learn(&mut self, home: &str, url: Option<String>, now_ms: i64) -> bool {
        let home = origin_form(home);
        self.asking.remove(&home);
        let url = url.or_else(|| self.url(&home).map(str::to_string));
        let changed = self.url(&home) != url.as_deref();
        if self.known.len() >= MAX_AWAY_HOMES * 4 && !self.known.contains_key(&home) {
            self.known.retain(|_, known| now_ms - known.at < LOOK_AGAIN_MS);
        }
        let misses = match url {
            Some(_) => 0,
            None => self.known.get(&home).map_or(0, |known| known.misses).saturating_add(1),
        };
        self.known.insert(home, Known { url, at: now_ms, misses });
        changed
    }
}

pub type WhoLists = [Vec<String>; 4];

fn is_person(who: &str) -> bool {
    who.parse::<commy_core::UserId>().is_ok()
}

pub fn whos_by_home(lists: [&[String]; 4], homes: &BTreeMap<String, String>) -> BTreeMap<Option<String>, WhoLists> {
    let mut every: Vec<Option<String>> = vec![None];
    every.extend(homes.values().cloned().map(Some));
    every.sort();
    every.dedup();
    every.truncate(MAX_AWAY_HOMES + 1);
    let mut split: BTreeMap<Option<String>, WhoLists> = BTreeMap::new();
    for (at, list) in lists.iter().enumerate() {
        for who in list.iter() {
            let targets: Vec<Option<String>> = match homes.get(who) {
                Some(home) => vec![Some(home.clone())],
                None if is_person(who) => vec![None],
                None => every.clone(),
            };
            for target in targets.into_iter().filter(|target| every.contains(target)) {
                split.entry(target).or_default()[at].push(who.clone());
            }
        }
    }
    split
}

pub fn tags_by_home(split: BTreeMap<Option<String>, WhoLists>, tag: impl Fn(&str) -> [u8; 32]) -> BTreeMap<Option<String>, RingTags> {
    let tagged = |whos: &[String]| -> Vec<[u8; 32]> { whos.iter().map(|who| tag(who)).collect() };
    split
        .into_iter()
        .map(|(home, [broad, direct, hush, quiet])| (home, RingTags { broad: tagged(&broad), direct: tagged(&direct), hush: tagged(&hush), quiet: tagged(&quiet) }))
        .filter(|(_, tags)| !tags.is_empty())
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    At(String),
    Unknown(String),
}

pub fn owners_away_home(state: &ServerState, mine: &str) -> Option<String> {
    away_home(mine, state.profiles.get(&state.server.owner)?)
}

pub fn owners_home_unproven(state: &ServerState) -> bool {
    state.profiles.get(&state.server.owner).is_none_or(|owner| home_of(owner).is_none())
}

pub fn held_place(placed: Option<Option<String>>, recorded_host: Option<&str>, account_url: &str) -> Option<String> {
    match placed {
        Some(Some(socket)) => Some(socket),
        Some(None) => Some(account_url.to_string()),
        None => recorded_host.and_then(socket_url),
    }
}

pub fn community_place(chosen: Option<&str>, owners_home: Option<&str>, held: Option<&str>, account_url: &str, known: impl Fn(&str) -> Option<String>) -> Option<Place> {
    match (chosen, owners_home) {
        (Some(origin), _) => socket_url(origin).map(Place::At),
        (None, None) => Some(Place::At(held.unwrap_or(account_url).to_string())),
        (None, Some(home)) => Some(known(home).map_or_else(|| Place::Unknown(home.to_string()), Place::At)),
    }
}

pub fn remembered_host(chosen: Option<&str>, place: Option<&str>, account_url: &str) -> Option<Option<String>> {
    if let Some(host) = chosen {
        return Some(Some(host.to_string()));
    }
    let socket = place?;
    if socket == account_url {
        return Some(None);
    }
    clean_sync_host(socket).map(Some)
}

pub fn invite_host(chosen: Option<&str>, place: Option<&str>) -> Option<String> {
    chosen.map(str::to_string).or_else(|| place.and_then(clean_sync_host))
}

pub fn needs_pin(state: &ServerState, me: commy_core::UserId) -> bool {
    state.server.owner == me && !state.server.group && state.server.sync_host.is_none() && !state.is_deleted()
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::home::{card_payload, HomeCard, CARD_DAYS, CARD_PURPOSE};
    use commy_core::Identity;

    const DAY_MS: i64 = 24 * 60 * 60 * 1000;
    const NOW: i64 = 1_800_000_000_000;

    fn housed(home: &str) -> UserProfile {
        let person = Identity::generate();
        let server = Identity::generate();
        let mut profile = UserProfile::new("someone");
        profile.identity = Some(person.public());
        let expires = NOW + CARD_DAYS * DAY_MS;
        let payload = card_payload(profile.id, &person.public(), NOW, expires);
        let sig = commy_core::server_key::sign(&server, CARD_PURPOSE, home, payload.as_bytes());
        profile.home = Some(HomeCard { home: home.into(), user: profile.id, identity: person.public(), key: server.public(), issued: NOW, expires, sig });
        profile
    }

    #[test]
    fn a_community_living_away_from_our_home_is_remembered_by_its_place() {
        let home = "wss://dev.commy.chat/sync";
        assert_eq!(remembered_host(None, Some("wss://commy.chat/sync"), home), Some(Some("https://commy.chat".into())), "a fresh device must not look for it on our own home");
        assert_eq!(remembered_host(None, Some(home), home), Some(None));
        assert_eq!(remembered_host(Some("https://sync.example.org"), Some("wss://other.example.org/sync"), home), Some(Some("https://sync.example.org".into())), "a chosen host is kept as chosen");
        assert_eq!(remembered_host(None, None, home), None, "an unplaced community keeps whatever was remembered");
        assert_eq!(remembered_host(None, Some("wss://10.0.0.5/sync"), home), None);
    }

    fn card(profile: &UserProfile, person: &Identity, home: &str, issued: i64) -> HomeCard {
        let server = Identity::generate();
        let expires = issued + CARD_DAYS * DAY_MS;
        let payload = card_payload(profile.id, &person.public(), issued, expires);
        let sig = commy_core::server_key::sign(&server, CARD_PURPOSE, home, payload.as_bytes());
        HomeCard { home: home.into(), user: profile.id, identity: person.public(), key: server.public(), issued, expires, sig }
    }

    #[test]
    fn the_latest_home_card_tells_where_someone_lives_now() {
        let person = Identity::generate();
        let mut old = UserProfile::new("ben");
        old.identity = Some(person.public());
        old.home = Some(card(&old, &person, "https://staging.commy.chat", NOW - DAY_MS));
        let moved = UserProfile { home: Some(card(&old, &person, "https://dev.commy.chat", NOW)), ..old.clone() };
        assert_eq!(newest_home(&old, [&moved]).map(|c| c.home).as_deref(), Some("https://dev.commy.chat"));
        assert_eq!(newest_home(&moved, [&old]).map(|c| c.home).as_deref(), Some("https://dev.commy.chat"), "an older copy never undoes a move");
        let stranger = Identity::generate();
        let forged = UserProfile { home: Some(card(&old, &stranger, "https://evil.example", NOW + DAY_MS)), ..old.clone() };
        assert_eq!(newest_home(&old, [&forged]).map(|c| c.home).as_deref(), Some("https://staging.commy.chat"), "a card for another identity is ignored");
        assert_eq!(newest_home(&UserProfile::new("unhoused"), []), None);
    }

    #[test]
    fn a_partner_is_away_only_when_their_proven_home_differs_from_ours() {
        let friend = housed("https://friends.org");
        assert_eq!(away_home("https://commy.chat", &friend).as_deref(), Some("https://friends.org"));
        assert_eq!(away_home("https://Friends.org/", &friend), None);
        let neighbour = housed("https://commy.chat");
        assert_eq!(away_home("https://commy.chat", &neighbour), None);
        let unhoused = UserProfile::new("older build");
        assert_eq!(away_home("https://commy.chat", &unhoused), None, "no proven home reads as our own home");
        let mut forged = housed("https://friends.org");
        forged.identity = Some(Identity::generate().public());
        assert_eq!(away_home("https://commy.chat", &forged), None, "a card for another identity proves nothing");
    }

    #[test]
    fn a_profile_picture_is_looked_for_on_its_owners_home() {
        let blob = "a".repeat(52);
        let mut friend = housed("https://friends.org");
        friend.banner = Some(format!("blob:{blob}"));
        let neighbour = housed("https://commy.chat");
        assert!(shows_picture(&friend, &blob));
        assert!(!shows_picture(&neighbour, &blob));
        assert_eq!(picture_home([&neighbour, &friend], "https://commy.chat", &blob).as_deref(), Some("https://friends.org"));
        assert_eq!(picture_home([&neighbour], "https://commy.chat", &blob), None);
        assert_eq!(picture_home([&friend], "https://friends.org", &blob), None, "a picture on our own home needs no second look");
    }

    #[test]
    fn a_homes_sync_address_comes_only_from_its_own_discovery_document() {
        let doc = r#"{"issuer":"https://friends.org","endpoints":{"sync":"wss://friends.org/sync"}}"#;
        assert_eq!(sync_url_of("https://friends.org", doc).as_deref(), Some("wss://friends.org/sync"));
        let elsewhere = r#"{"issuer":"https://friends.org","endpoints":{"sync":"wss://commy.chat/sync"}}"#;
        assert_eq!(sync_url_of("https://friends.org", elsewhere), None, "a home cannot point us at someone else's sync");
        let borrowed = r#"{"issuer":"https://commy.chat","endpoints":{"sync":"wss://friends.org/sync"}}"#;
        assert_eq!(sync_url_of("https://friends.org", borrowed), None);
        let plain = r#"{"issuer":"https://friends.org","endpoints":{"sync":"ws://friends.org/sync"}}"#;
        assert_eq!(sync_url_of("https://friends.org", plain), None);
        let none = r#"{"issuer":"https://friends.org","endpoints":{}}"#;
        assert_eq!(sync_url_of("https://friends.org", none), None);
        assert_eq!(sync_url_of("https://friends.org", "not json"), None);
        assert_eq!(discovery_url("https://friends.org/"), "https://friends.org/.well-known/commy-auth.json");
    }

    #[test]
    fn cursors_on_another_home_never_share_a_key_with_ours() {
        assert_eq!(cursor_key("ab12", "wss://friends.org/sync"), "ab12@friends.org");
        assert_ne!(cursor_key("ab12", "wss://friends.org/sync"), cursor_key("ab12", "wss://other.org/sync"));
        assert_ne!(cursor_key("ab12", "wss://friends.org/sync"), "ab12");
    }

    #[test]
    fn each_home_is_asked_once_and_again_only_when_its_answer_ages() {
        let mut book = HomeBook::default();
        assert_eq!(book.to_ask(["https://Friends.org", "https://friends.org"], NOW), vec!["https://friends.org"]);
        assert!(book.to_ask(["https://friends.org"], NOW + 1000).is_empty(), "already being asked");
        assert!(book.learn("https://friends.org", Some("wss://friends.org/sync".into()), NOW + 2000));
        assert_eq!(book.url("https://FRIENDS.org"), Some("wss://friends.org/sync"));
        assert!(!book.learn("https://friends.org", Some("wss://friends.org/sync".into()), NOW + 3000), "the same answer changes nothing");
        assert!(book.to_ask(["https://friends.org"], NOW + LOOK_AGAIN_MS).is_empty());
        assert_eq!(book.to_ask(["https://friends.org"], NOW + 3000 + LOOK_AGAIN_MS), vec!["https://friends.org"]);
        book.learn("https://gone.org", None, NOW);
        assert_eq!(book.url("https://gone.org"), None);
        assert!(book.to_ask(["https://gone.org"], NOW + 1000).is_empty());
        assert_eq!(book.to_ask(["https://gone.org"], NOW + RETRY_MS), vec!["https://gone.org"], "an unreachable home is tried again sooner");
    }

    #[test]
    fn a_failed_lookup_is_tried_again_within_seconds_and_backs_off() {
        let mut book = HomeBook::default();
        let home = "https://staging.commy.chat";
        let mut at = NOW;
        for wait in [FIRST_RETRY_MS, 2 * FIRST_RETRY_MS, 4 * FIRST_RETRY_MS] {
            assert_eq!(book.to_ask([home], at), vec![home]);
            book.learn(home, None, at);
            assert!(book.to_ask([home], at + wait - 1).is_empty());
            at += wait;
        }
        for _ in 0..20 {
            assert_eq!(book.to_ask([home], at), vec![home]);
            book.learn(home, None, at);
            at += RETRY_MS;
        }
        assert!(book.to_ask([home], at - 1).is_empty(), "the wait never grows past the usual retry");
        assert!(book.learn(home, Some("wss://staging.commy.chat/sync".into()), at));
        assert!(!book.learn(home, None, at + LOOK_AGAIN_MS), "a failed look again keeps the address that worked");
        assert_eq!(book.url(home), Some("wss://staging.commy.chat/sync"));
    }

    #[test]
    fn asking_is_capped_so_a_crowd_of_homes_cannot_flood_us() {
        let mut book = HomeBook::default();
        let homes: Vec<String> = (0..MAX_AWAY_HOMES + 10).map(|at| format!("https://home{at}.org")).collect();
        assert_eq!(book.to_ask(homes.iter().map(String::as_str), NOW).len(), MAX_AWAY_HOMES);
    }

    #[test]
    fn a_mention_wakes_only_its_persons_home_and_group_wake_ups_reach_every_home() {
        let (here, there) = (commy_core::UserId::new().to_string(), commy_core::UserId::new().to_string());
        let homes = BTreeMap::from([(there.clone(), "https://friends.org".to_string())]);
        let broad = vec!["all:0".to_string()];
        let direct = vec![here.clone(), there.clone()];
        let hush = vec!["mute@x".to_string()];
        let split = whos_by_home([&broad, &direct, &hush, &[]], &homes);
        let ours = &split[&None];
        assert_eq!(ours[0], broad);
        assert_eq!(ours[1], vec![here.clone()]);
        assert_eq!(ours[2], hush);
        let theirs = &split[&Some("https://friends.org".to_string())];
        assert_eq!(theirs[0], broad, "everyone's phones hear the server-wide ring on their own home");
        assert_eq!(theirs[1], vec![there.clone()]);
        assert_eq!(theirs[2], hush, "hushes travel with the ring so muted phones stay quiet");
        assert_eq!(split.len(), 2);
    }

    #[test]
    fn wake_up_tags_follow_the_home_their_whos_were_split_into() {
        let theirs = Some("https://friends.org".to_string());
        let split = BTreeMap::from([
            (None, [vec!["all:0".to_string()], vec![], vec![], vec![]]),
            (theirs.clone(), [vec![], vec!["bea".to_string()], vec!["mute@x".to_string()], vec![]]),
            (Some("https://quiet.org".to_string()), [vec![], vec![], vec!["mute@x".to_string()], vec![]]),
        ]);
        let tags = tags_by_home(split, |who| [who.len() as u8; 32]);
        assert_eq!(tags[&None].broad, vec![[5u8; 32]]);
        assert_eq!(tags[&theirs].direct, vec![[3u8; 32]]);
        assert_eq!(tags[&theirs].hush, vec![[6u8; 32]]);
        assert!(!tags.contains_key(&Some("https://quiet.org".to_string())), "a home with only hushes gets no ring");
    }

    #[test]
    fn group_wake_ups_reach_a_capped_number_of_homes() {
        let homes: BTreeMap<String, String> = (0..MAX_AWAY_HOMES + 5).map(|at| (commy_core::UserId::new().to_string(), format!("https://home{at}.org"))).collect();
        let split = whos_by_home([&["all:0".to_string()], &[], &[], &[]], &homes);
        assert_eq!(split.len(), MAX_AWAY_HOMES + 1);
    }

    #[test]
    fn a_community_without_its_own_host_lives_on_its_owners_home() {
        let mine = "wss://commy.chat/sync";
        let known = |home: &str| (home == "https://friends.org").then(|| "wss://friends.org/sync".to_string());
        assert_eq!(community_place(None, None, None, mine, known), Some(Place::At(mine.into())), "an owner on our home keeps it on our sync");
        assert_eq!(
            community_place(None, None, Some("wss://staging.commy.chat/sync"), mine, known),
            Some(Place::At("wss://staging.commy.chat/sync".into())),
            "while the owner's card is missing a community stays where it already lives"
        );
        assert_eq!(community_place(None, Some("https://friends.org"), None, mine, known), Some(Place::At("wss://friends.org/sync".into())));
        assert_eq!(
            community_place(None, Some("https://far.org"), None, mine, known),
            Some(Place::Unknown("https://far.org".into())),
            "an owner's home we have not looked up yet is waited for, never guessed"
        );
        assert_eq!(community_place(Some("https://sync.example.org"), Some("https://friends.org"), Some(mine), mine, known), Some(Place::At("wss://sync.example.org/sync".into())), "a chosen host always wins");
    }

    #[test]
    fn a_community_whose_owner_lost_their_card_stays_where_it_lives() {
        let mine = "wss://dev.commy.chat/sync";
        let theirs = "wss://staging.commy.chat/sync";
        assert_eq!(held_place(Some(Some(theirs.into())), Some("https://elsewhere.org"), mine).as_deref(), Some(theirs), "the live place wins over an older record");
        assert_eq!(held_place(Some(None), Some("https://staging.commy.chat"), mine).as_deref(), Some(mine));
        assert_eq!(held_place(None, Some("https://staging.commy.chat"), mine).as_deref(), Some(theirs), "a fresh start uses where the account log last saw it");
        assert_eq!(held_place(None, None, mine), None);
    }

    #[test]
    fn the_owners_proven_home_decides_and_an_unproven_one_reads_as_ours() {
        let owner = housed("https://friends.org");
        let mut state = ServerState::new(commy_core::Server::new("club", owner.id));
        state.profiles.insert(owner.id, owner.clone());
        assert_eq!(owners_away_home(&state, "https://commy.chat").as_deref(), Some("https://friends.org"));
        assert_eq!(owners_away_home(&state, "https://friends.org"), None);
        let older = UserProfile::new("older build");
        let mut legacy = ServerState::new(commy_core::Server::new("club", older.id));
        legacy.profiles.insert(older.id, older);
        assert_eq!(owners_away_home(&legacy, "https://commy.chat"), None);
        assert!(!owners_home_unproven(&state));
        assert!(owners_home_unproven(&legacy));
        let mut unknown = ServerState::new(commy_core::Server::new("club", owner.id));
        assert!(owners_home_unproven(&unknown));
        unknown.profiles.insert(owner.id, UserProfile { home: None, ..owner.clone() });
        assert!(owners_home_unproven(&unknown));
    }

    #[test]
    fn invites_say_where_a_community_lives_even_without_a_chosen_host() {
        assert_eq!(invite_host(Some("https://sync.example.org"), Some("wss://commy.chat/sync")).as_deref(), Some("https://sync.example.org"));
        assert_eq!(invite_host(None, Some("wss://commy.chat/sync")).as_deref(), Some("https://commy.chat"));
        assert_eq!(invite_host(None, Some("wss://commy.chat/elsewhere")), None, "a sync service off the usual path cannot be named in an invite");
        assert_eq!(invite_host(None, None), None);
    }

    #[test]
    fn only_the_owner_pins_a_community_that_has_no_host_yet() {
        let owner = commy_core::UserId::new();
        let mut state = ServerState::new(commy_core::Server::new("club", owner));
        assert!(needs_pin(&state, owner));
        assert!(!needs_pin(&state, commy_core::UserId::new()));
        state.server.sync_host = Some("https://sync.example.org".into());
        assert!(!needs_pin(&state, owner));
        state.server.sync_host = None;
        state.server.group = true;
        assert!(!needs_pin(&state, owner), "group chats cannot choose a host");
    }
}
