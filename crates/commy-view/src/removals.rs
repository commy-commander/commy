use commy_core::{ServerState, UserId};
use serde::Serialize;

use crate::display_name;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BanView {
    pub id: String,
    pub name: String,
    pub by: Option<String>,
    pub by_name: Option<String>,
    pub at: i64,
    pub until: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KickView {
    pub id: String,
    pub name: String,
    pub by: String,
    pub by_name: String,
    pub at: i64,
}

pub fn bans(state: &ServerState, now_ms: i64) -> Vec<BanView> {
    let mut list: Vec<BanView> = state
        .server
        .banned
        .iter()
        .filter(|user| state.server.banned_at(**user, now_ms))
        .map(|user| {
            let record = state.server.bans.get(user).copied().unwrap_or_default();
            BanView {
                id: user.to_string(),
                name: display_name(state, *user),
                by: record.by.map(|by| by.to_string()),
                by_name: record.by.map(|by| display_name(state, by)),
                at: record.at,
                until: record.until,
            }
        })
        .collect();
    list.sort_by_key(|b| std::cmp::Reverse(b.at));
    list
}

pub fn kicked(state: &ServerState) -> Vec<KickView> {
    let still_out = |user: UserId| !state.server.is_member(user) && !state.server.banned.contains(&user);
    state
        .server
        .kicked
        .iter()
        .filter(|k| still_out(k.user))
        .map(|k| KickView { id: k.user.to_string(), name: display_name(state, k.user), by: k.by.to_string(), by_name: display_name(state, k.by), at: k.at })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::removals::Kick;
    use commy_core::{Server, ServerEvent, UserProfile};

    fn hangout() -> (ServerState, UserId, UserId) {
        let owner = UserId::new();
        let mut state = ServerState::new(Server::new("Hangout", owner));
        let profile = UserProfile::new("friend");
        let id = profile.id;
        state.apply(ServerEvent::MemberJoin { server: state.server.id, profile }).unwrap();
        (state, owner, id)
    }

    #[test]
    fn bans_show_their_end_and_drop_off_once_it_passes() {
        let (mut state, _, friend) = hangout();
        let sid = state.server.id;
        state.apply(ServerEvent::MemberBanUntil { server: sid, user: friend, at: 1_000, until: Some(5_000) }).unwrap();
        let list = bans(&state, 2_000);
        assert_eq!((list.len(), list[0].name.as_str(), list[0].until), (1, "friend", Some(5_000)));
        assert!(bans(&state, 5_000).is_empty());
    }

    #[test]
    fn the_kick_list_leaves_out_people_who_came_back_or_were_banned() {
        let (mut state, owner, friend) = hangout();
        let sid = state.server.id;
        let other = UserProfile::new("other");
        let other_id = other.id;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: other }).unwrap();
        state.server.kicked = vec![Kick { user: friend, by: owner, at: 2 }, Kick { user: other_id, by: owner, at: 1 }];
        assert!(kicked(&state).is_empty(), "both are still members");
        state.apply(ServerEvent::MemberLeave { server: sid, user: friend }).unwrap();
        state.apply(ServerEvent::MemberLeave { server: sid, user: other_id }).unwrap();
        let names: Vec<String> = kicked(&state).into_iter().map(|k| k.name).collect();
        assert_eq!(names, vec!["friend".to_string(), "other".to_string()]);
        state.apply(ServerEvent::MemberBan { server: sid, user: other_id }).unwrap();
        assert_eq!(kicked(&state).len(), 1);
    }
}
