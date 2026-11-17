use commy_core::sync_host::clean_sync_host;
use commy_core::{ServerEvent, ServerId, ServerState, UserId};
use std::collections::HashMap;
use commy_sync::wire::{Fit, Greeting, PROTOCOL};
use serde::Serialize;

use crate::act::Act;

pub const SYNC_HOST_INVALID: &str = "Use the address of a Commy sync server on a public domain, like sync.yourcommunity.org";
pub const SYNC_HOST_OWNER_ONLY: &str = "Only the server's owner can change where it syncs";
pub const SYNC_HOST_UNCHANGED: &str = "This server already syncs there";
pub const HOST_UNREACHABLE: &str = "Commy could not reach a sync server there. Check the address and that the server is running.";
pub const HOST_TOO_OLD: &str = "That sync server needs updating before it can sync this server.";
pub const HOST_TAKEN: &str = "Someone else already holds this server's erase keys on that sync server, so Commy won't move it there.";
pub const HOST_SLOW: &str = "That sync server did not answer in time. Try again in a moment.";
pub const KEYS_NOT_READY: &str = "This server's keys are not ready yet. Try again in a moment.";

pub const ROOM_RELAY_DOWN: &str = "This server's voice relay isn't answering, so voice here is off for now. Try again in a few minutes.";
pub const ROOM_RELAY_MISSING: &str = "This server's sync server runs no voice relay, so voice here isn't available. Ask the server's owner to run one.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomRelay {
    Ours,
    Theirs { url: String, public_fallback: bool },
    Public,
    Unavailable(&'static str),
}

pub const HOST_REMOVED: &str = "This server's sync server removed it, so new messages here no longer reach anyone.";

#[derive(Debug, Clone, Default)]
pub struct HostRemovals {
    held: HashMap<ServerId, String>,
}

impl HostRemovals {
    pub fn note(&mut self, server: ServerId, socket: &str) -> bool {
        self.held.insert(server, socket.to_string()).as_deref() != Some(socket)
    }

    pub fn holds(&self, server: ServerId, socket: &str) -> bool {
        self.held.get(&server).is_some_and(|held| held == socket)
    }

    pub fn forget(&mut self, server: ServerId) {
        self.held.remove(&server);
    }
}

pub fn refuse_when_removed(removed: bool) -> Result<(), String> {
    if removed { Err(HOST_REMOVED.into()) } else { Ok(()) }
}

pub const HOST_NEEDS_NEWER_APP: &str = "This server's sync server needs a newer Commy. Update Commy to keep chatting here.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostRelease {
    pub version: Option<String>,
    pub fit: &'static str,
    pub text: String,
}

pub fn host_release(greeting: &Greeting) -> HostRelease {
    host_release_for(greeting, PROTOCOL)
}

fn host_release_for(greeting: &Greeting, app_protocol: u32) -> HostRelease {
    let version = greeting.version.clone().filter(|v| v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+')));
    let runs = match &version {
        Some(v) if v.starts_with(|c: char| c.is_ascii_digit()) => format!("Commy {v}"),
        Some(_) => "a development build of Commy".to_string(),
        None => "an earlier Commy".to_string(),
    };
    let (fit, text) = match greeting.fit(app_protocol) {
        Fit::Current => ("current", format!("It runs {runs}.")),
        Fit::HostBehind => {
            ("host_behind", format!("It runs {runs}, which is older than this app. Ask whoever runs it to update it, or run setup.sh again if it's yours."))
        }
        Fit::AppBehind => ("app_behind", HOST_NEEDS_NEWER_APP.to_string()),
    };
    HostRelease { version, fit, text }
}

pub fn room_relay(community_socket: &str, default_socket: &str, greeting: Option<&Greeting>) -> RoomRelay {
    let Some(host) = socket_host(community_socket) else { return RoomRelay::Ours };
    if socket_host(default_socket).as_deref() == Some(host.as_str()) {
        return RoomRelay::Ours;
    }
    let Some(greeting) = greeting else { return RoomRelay::Unavailable(ROOM_RELAY_DOWN) };
    let relay = greeting.relay.as_deref().map(|r| r.trim().trim_end_matches('/'));
    let relay_host = relay.and_then(|r| r.strip_prefix("https://")).filter(|rest| !rest.contains(['/', '?', '#', '@']));
    match relay_host {
        Some(relay_host) if relay_host.to_ascii_lowercase() == host => RoomRelay::Theirs { url: format!("https://{relay_host}"), public_fallback: greeting.public_fallback },
        _ if greeting.public_fallback => RoomRelay::Public,
        _ => RoomRelay::Unavailable(ROOM_RELAY_MISSING),
    }
}

fn socket_host(socket: &str) -> Option<String> {
    let rest = socket.trim().split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty() && !host.contains('@')).then(|| host.to_ascii_lowercase())
}

pub fn set_sync_host(state: &ServerState, me: UserId, address: Option<&str>) -> Act<ServerEvent> {
    if state.server.owner != me || state.server.group {
        return Err(SYNC_HOST_OWNER_ONLY.into());
    }
    let url = match address.map(str::trim).filter(|a| !a.is_empty()) {
        None => None,
        Some(address) => Some(clean_sync_host(address).ok_or(SYNC_HOST_INVALID)?),
    };
    if url == state.server.sync_host {
        return Err(SYNC_HOST_UNCHANGED.into());
    }
    Ok(ServerEvent::SyncHostSet { server: state.server.id, url })
}

pub fn shown_host(state: &ServerState) -> Option<String> {
    state.server.sync_host.as_deref().and_then(commy_core::sync_host::display_host).map(str::to_string)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_removal_holds_only_for_the_sync_server_that_removed_it() {
        let mut removals = HostRemovals::default();
        let server = ServerId::new();
        assert!(!removals.holds(server, "wss://sync.example.org/sync"));
        assert!(removals.note(server, "wss://sync.example.org/sync"), "first word of it is news");
        assert!(!removals.note(server, "wss://sync.example.org/sync"), "hearing it again is not");
        assert!(removals.holds(server, "wss://sync.example.org/sync"));
        assert!(!removals.holds(server, "wss://other.example.org/sync"), "moving elsewhere lifts it");
        assert!(!removals.holds(ServerId::new(), "wss://sync.example.org/sync"));
        removals.forget(server);
        assert!(!removals.holds(server, "wss://sync.example.org/sync"));
    }

    #[test]
    fn posting_is_refused_in_a_removed_server() {
        assert_eq!(refuse_when_removed(false), Ok(()));
        assert_eq!(refuse_when_removed(true), Err(HOST_REMOVED.to_string()));
    }

    #[test]
    fn a_community_uses_its_own_hosts_relay_only() {
        let with = |relay: &str| Greeting { relay: Some(relay.to_string()), ..Greeting::of(PROTOCOL) };
        let ours = "wss://commy.chat/sync";
        let theirs = "wss://Sync.Example.org/sync";
        let own = |url: &str| RoomRelay::Theirs { url: url.to_string(), public_fallback: false };
        assert_eq!(room_relay(theirs, ours, Some(&with("https://sync.example.org"))), own("https://sync.example.org"));
        assert_eq!(room_relay(theirs, ours, Some(&with("https://sync.example.org/"))), own("https://sync.example.org"));
        assert_eq!(room_relay(ours, ours, Some(&with("https://commy.chat"))), RoomRelay::Ours, "communities on our host keep our relay");
        assert_eq!(room_relay(ours, ours, None), RoomRelay::Ours);
    }

    #[test]
    fn a_community_elsewhere_never_borrows_our_relay() {
        let with = |relay: &str| Greeting { relay: Some(relay.to_string()), ..Greeting::of(PROTOCOL) };
        let ours = "wss://commy.chat/sync";
        let theirs = "wss://sync.example.org/sync";
        let missing = RoomRelay::Unavailable(ROOM_RELAY_MISSING);
        assert_eq!(room_relay(theirs, ours, Some(&with("https://tracker.example.net"))), missing, "another domain is ignored");
        assert_eq!(room_relay(theirs, ours, Some(&with("http://sync.example.org"))), missing);
        assert_eq!(room_relay(theirs, ours, Some(&with("https://sync.example.org/elsewhere"))), missing);
        assert_eq!(room_relay(theirs, ours, Some(&Greeting::of(PROTOCOL))), missing, "no relay announced");
        assert_eq!(room_relay(theirs, ours, None), RoomRelay::Unavailable(ROOM_RELAY_DOWN), "a host we cannot reach");
    }

    #[test]
    fn an_operator_can_let_voice_fall_back_to_the_public_relays() {
        let public = |relay: Option<&str>| Greeting { relay: relay.map(str::to_string), public_fallback: true, ..Greeting::of(PROTOCOL) };
        let ours = "wss://commy.chat/sync";
        let theirs = "wss://sync.example.org/sync";
        assert_eq!(
            room_relay(theirs, ours, Some(&public(Some("https://sync.example.org")))),
            RoomRelay::Theirs { url: "https://sync.example.org".into(), public_fallback: true }
        );
        assert_eq!(room_relay(theirs, ours, Some(&public(None))), RoomRelay::Public, "no relay of its own, so the public ones");
        assert_eq!(room_relay(ours, ours, Some(&public(None))), RoomRelay::Ours, "our own communities are unchanged");
    }

    use super::*;
    use commy_core::{Server, UserProfile};

    #[test]
    fn only_the_owner_picks_a_clean_sync_host() {
        let owner = UserId::new();
        let mut state = ServerState::new(Server::new("s", owner));
        let friend = UserProfile::new("friend");
        let sid = state.server.id;
        let friend_id = friend.id;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: friend }).unwrap();
        let friend = friend_id;
        let event = set_sync_host(&state, owner, Some(" Sync.Example.org ")).unwrap();
        assert!(matches!(event, ServerEvent::SyncHostSet { url: Some(ref u), .. } if u == "https://sync.example.org"));
        assert_eq!(set_sync_host(&state, friend, Some("sync.example.org")).unwrap_err(), SYNC_HOST_OWNER_ONLY);
        assert_eq!(set_sync_host(&state, owner, Some("http://sync.example.org")).unwrap_err(), SYNC_HOST_INVALID);
        assert_eq!(set_sync_host(&state, owner, Some("  ")).unwrap_err(), SYNC_HOST_UNCHANGED, "blank means the default it already has");
        state.apply(event).unwrap();
        assert_eq!(shown_host(&state).as_deref(), Some("sync.example.org"));
        assert_eq!(set_sync_host(&state, owner, Some("https://sync.example.org/")).unwrap_err(), SYNC_HOST_UNCHANGED);
        assert!(matches!(set_sync_host(&state, owner, None), Ok(ServerEvent::SyncHostSet { url: None, .. })));
    }

    #[test]
    fn the_hosts_release_reads_as_plain_words() {
        let current = Greeting { version: Some("0.2.5".into()), ..Greeting::of(5) };
        assert_eq!(host_release_for(&current, 5), HostRelease { version: Some("0.2.5".into()), fit: "current", text: "It runs Commy 0.2.5.".into() });
        let behind = host_release_for(&Greeting::of(4), 5);
        assert_eq!(behind.fit, "host_behind");
        assert!(behind.text.starts_with("It runs an earlier Commy, which is older than this app."));
        let ahead = Greeting { oldest_protocol: 6, ..Greeting::of(7) };
        assert_eq!(host_release_for(&ahead, 5).text, HOST_NEEDS_NEWER_APP);
        let odd = Greeting { version: Some("<b>hi</b>".into()), ..Greeting::of(5) };
        assert_eq!(host_release_for(&odd, 5).version, None, "a version with markup is not shown");
        let dev = Greeting { version: Some("main".into()), ..Greeting::of(5) };
        assert_eq!(host_release_for(&dev, 5).text, "It runs a development build of Commy.");
    }
}
