use commy_core::join_request::{self, JoinRequest};
use commy_core::{Permissions, ServerEvent, ServerState, UserId, UserProfile};
use serde::Serialize;

use crate::act::Act;
use crate::manage::need;

pub const NOT_ASKING: &str = "That person is no longer waiting to join";
pub const NOTE_TOO_LONG: &str = "Keep your note to 300 characters";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JoinRequestView {
    pub id: String,
    pub name: String,
    pub username: String,
    pub avatar: Option<String>,
    pub avatar_provider: Option<String>,
    pub note: String,
    pub at: i64,
}

fn request_view(request: &JoinRequest) -> JoinRequestView {
    let profile = &request.profile;
    JoinRequestView {
        id: profile.id.to_string(),
        name: profile.display_name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| profile.username.clone()),
        username: profile.username.clone(),
        avatar: profile.avatar.as_deref().and_then(commy_core::Avatar::parse).map(|a| a.encode()),
        avatar_provider: profile.node.clone(),
        note: request.note.clone(),
        at: request.at,
    }
}

pub fn queue(state: &ServerState, viewer: Option<UserId>) -> Vec<JoinRequestView> {
    let answers = viewer.is_some_and(|v| may_answer(state, v));
    if !answers {
        return Vec::new();
    }
    let mut list: Vec<JoinRequestView> = state.server.join_requests.iter().map(request_view).collect();
    list.sort_by_key(|r| r.at);
    list
}

pub fn may_answer(state: &ServerState, user: UserId) -> bool {
    state.server.is_current_member(user) && state.server.permissions_for(user).allows(Permissions::KICK_MEMBERS)
}

pub fn set_approval(state: &ServerState, perms: Permissions, on: bool, at: i64) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_SERVER, "change who can join")?;
    Ok(ServerEvent::JoinApprovalSet { server: state.server.id, on, at })
}

pub fn answer(state: &ServerState, perms: Permissions, user: UserId, admit: bool) -> Act<ServerEvent> {
    need(perms, Permissions::KICK_MEMBERS, "answer join requests")?;
    if join_request::request_of(&state.server, user).is_none() {
        return Err(NOT_ASKING.into());
    }
    Ok(ServerEvent::JoinAnswer { server: state.server.id, user, admit })
}

pub fn ask(server: commy_core::ServerId, me: &UserProfile, note: &str, at: i64) -> Act<ServerEvent> {
    let note = join_request::clean_note(note).ok_or(NOTE_TOO_LONG)?;
    Ok(ServerEvent::JoinAsk { server, profile: me.as_seen_by_others(), note, at })
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::Server;

    fn fixture() -> (ServerState, UserId) {
        let owner = UserProfile::new("owner");
        let mut state = ServerState::new(Server::new("test", owner.id));
        state.profiles.insert(owner.id, owner.clone());
        (state, owner.id)
    }

    fn join(state: &mut ServerState, name: &str) -> UserId {
        let profile = UserProfile::new(name);
        let id = profile.id;
        state.apply(ServerEvent::MemberJoin { server: state.server.id, profile }).unwrap();
        id
    }

    fn keyed_profile(name: &str) -> UserProfile {
        UserProfile::new(name)
    }

    #[test]
    fn moderators_see_the_queue_oldest_first_and_others_see_nothing() {
        let (mut state, owner) = fixture();
        let sid = state.server.id;
        let friend = join(&mut state, "friend");
        state.apply(ServerEvent::JoinApprovalSet { server: sid, on: true, at: 1 }).unwrap();
        let late = keyed_profile("late");
        let early = keyed_profile("early");
        state.apply(ask(sid, &late, "second", 20).unwrap()).unwrap();
        state.apply(ask(sid, &early, "first", 10).unwrap()).unwrap();
        let list = queue(&state, Some(owner));
        assert_eq!(list.iter().map(|r| r.note.as_str()).collect::<Vec<_>>(), ["first", "second"]);
        assert_eq!(list[0].name, "early");
        assert!(queue(&state, Some(friend)).is_empty());
        assert!(queue(&state, None).is_empty());
    }

    #[test]
    fn answering_needs_the_kick_permission_and_a_waiting_person() {
        let (mut state, _) = fixture();
        let sid = state.server.id;
        state.apply(ServerEvent::JoinApprovalSet { server: sid, on: true, at: 1 }).unwrap();
        let asker = keyed_profile("asker");
        state.apply(ask(sid, &asker, "", 1).unwrap()).unwrap();
        assert!(matches!(answer(&state, Permissions::all(), asker.id, true), Ok(ServerEvent::JoinAnswer { admit: true, .. })));
        assert!(answer(&state, Permissions::NONE, asker.id, true).is_err());
        assert_eq!(answer(&state, Permissions::all(), UserId::new(), false), Err(NOT_ASKING.into()));
        assert!(set_approval(&state, Permissions::NONE, false, 1).is_err());
        assert_eq!(ask(sid, &asker, &"x".repeat(301), 1), Err(NOTE_TOO_LONG.into()));
    }
}
