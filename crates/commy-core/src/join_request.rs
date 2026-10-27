use serde::{Deserialize, Serialize};

use crate::ids::UserId;
use crate::server::Server;
use crate::user::UserProfile;

pub const MAX_JOIN_REQUESTS: usize = 100;
pub const MAX_TURNED_AWAY: usize = 500;
pub const MAX_NOTE_CHARS: usize = 300;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinRequest {
    pub profile: UserProfile,
    pub note: String,
    pub at: i64,
}

pub fn clean_note(raw: &str) -> Option<String> {
    let note = raw.trim();
    let fits = note.chars().count() <= MAX_NOTE_CHARS && !note.chars().any(|c| c.is_control() && c != '\n');
    fits.then(|| note.to_string())
}

pub fn may_ask(server: &Server, user: UserId) -> bool {
    server.join_approval && !server.is_member(user) && !server.banned.contains(&user) && user != server.owner
}

pub fn request_of(server: &Server, user: UserId) -> Option<&JoinRequest> {
    server.join_requests.iter().find(|r| r.profile.id == user)
}

pub(crate) fn record(server: &mut Server, request: JoinRequest) -> bool {
    if server.join_requests.iter().any(|r| *r == request) {
        return false;
    }
    let user = request.profile.id;
    server.join_requests.retain(|r| r.profile.id != user);
    server.join_requests.push(request);
    server.join_requests.sort_by_key(|r| r.at);
    let extra = server.join_requests.len().saturating_sub(MAX_JOIN_REQUESTS);
    server.join_requests.drain(..extra);
    true
}

pub(crate) fn turn_away(server: &mut Server, user: UserId) -> bool {
    let before = server.join_requests.len();
    server.join_requests.retain(|r| r.profile.id != user);
    let removed = server.join_requests.len() != before;
    if server.turned_away.contains(&user) {
        return removed;
    }
    server.turned_away.push(user);
    let extra = server.turned_away.len().saturating_sub(MAX_TURNED_AWAY);
    server.turned_away.drain(..extra);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(profile: &UserProfile, at: i64) -> JoinRequest {
        JoinRequest { profile: profile.clone(), note: "hi".into(), at }
    }

    #[test]
    fn notes_are_short_plain_text() {
        assert_eq!(clean_note("  hello\nthere  "), Some("hello\nthere".into()));
        assert_eq!(clean_note(""), Some(String::new()));
        assert_eq!(clean_note(&"a".repeat(MAX_NOTE_CHARS + 1)), None);
        assert_eq!(clean_note("bell\u{7}"), None);
    }

    #[test]
    fn only_strangers_may_ask_and_only_when_the_server_asks_for_it() {
        let owner = UserId::new();
        let mut server = Server::new("Rockets", owner);
        let stranger = UserId::new();
        assert!(!may_ask(&server, stranger));
        server.join_approval = true;
        assert!(may_ask(&server, stranger));
        assert!(!may_ask(&server, owner));
        server.banned.push(stranger);
        assert!(!may_ask(&server, stranger));
    }

    #[test]
    fn asking_again_replaces_the_old_request_and_the_queue_keeps_the_newest() {
        let mut server = Server::new("Rockets", UserId::new());
        let first = UserProfile::new("first");
        assert!(record(&mut server, request(&first, 1)));
        assert!(!record(&mut server, request(&first, 1)));
        assert!(record(&mut server, request(&first, 5)));
        assert_eq!(server.join_requests.len(), 1);
        for at in 0..MAX_JOIN_REQUESTS as i64 {
            record(&mut server, request(&UserProfile::new("later"), 10 + at));
        }
        assert_eq!(server.join_requests.len(), MAX_JOIN_REQUESTS);
        assert!(request_of(&server, first.id).is_none());
    }

    #[test]
    fn turning_someone_away_drops_their_request_and_remembers_them() {
        let mut server = Server::new("Rockets", UserId::new());
        let asker = UserProfile::new("asker");
        record(&mut server, request(&asker, 1));
        assert!(turn_away(&mut server, asker.id));
        assert!(server.join_requests.is_empty());
        assert_eq!(server.turned_away, vec![asker.id]);
        assert!(!turn_away(&mut server, asker.id));
    }
}
