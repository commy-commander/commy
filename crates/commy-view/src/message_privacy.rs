use commy_core::day_chain;
use commy_core::{Permissions, ServerEvent, ServerState, UserId};
use serde::Serialize;

use crate::act::Act;

pub const TURN_ON_NEEDS: &str = "you need permission to manage this server to change message privacy";
pub const TURN_OFF_NEEDS: &str = "only the server owner can turn daily keys off";
pub const UNKNOWN_DAYS: &str = "choose 1, 7, 30 or 90 days";

#[derive(Debug, Clone, Serialize)]
pub struct DayKeysView {
    pub on: bool,
    pub chosen: bool,
    pub since: Option<i64>,
    pub history_days: u32,
    pub history_choices: Vec<u32>,
    pub waiting: usize,
    pub can_turn_on: bool,
    pub can_turn_off: bool,
}

pub fn view(state: &ServerState, viewer: Option<UserId>) -> Option<DayKeysView> {
    let viewer = viewer.filter(|v| state.server.is_current_member(*v))?;
    let server = &state.server;
    let on = day_chain::is_on(server);
    Some(DayKeysView {
        on,
        chosen: server.day_keys.is_some(),
        since: server.day_keys.filter(|keys| keys.on).map(|keys| keys.since),
        history_days: day_chain::history_days(server),
        history_choices: day_chain::HISTORY_DAYS.to_vec(),
        waiting: day_chain::waiting_on(server).len(),
        can_turn_on: server.permissions_for(viewer).allows(Permissions::MANAGE_SERVER),
        can_turn_off: viewer == server.owner,
    })
}

pub fn set_day_keys(state: &ServerState, me: UserId, on: bool, history_days: u32, now_ms: i64) -> Act<ServerEvent> {
    let server = &state.server;
    if !day_chain::HISTORY_DAYS.contains(&history_days) {
        return Err(UNKNOWN_DAYS.into());
    }
    if !server.is_current_member(me) || !server.permissions_for(me).allows(Permissions::MANAGE_SERVER) {
        return Err(TURN_ON_NEEDS.into());
    }
    if !on && me != server.owner {
        return Err(TURN_OFF_NEEDS.into());
    }
    Ok(day_chain::switch_event(server, on, history_days, now_ms))
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Server, UserProfile};

    fn owned() -> (ServerState, UserId) {
        let owner = UserId::new();
        (ServerState::new(Server::new("test", owner)), owner)
    }

    #[test]
    fn owners_turn_daily_keys_on_and_off_and_pick_how_far_back_new_members_read() {
        let (mut state, owner) = owned();
        let shown = view(&state, Some(owner)).unwrap();
        assert!(!shown.on && !shown.chosen && shown.can_turn_on && shown.can_turn_off);
        assert_eq!(shown.history_days, day_chain::DEFAULT_HISTORY_DAYS);
        let event = set_day_keys(&state, owner, true, 7, 1_000).unwrap();
        state.apply(event).unwrap();
        let shown = view(&state, Some(owner)).unwrap();
        assert!(shown.on && shown.chosen);
        assert_eq!((shown.history_days, shown.since), (7, Some(1_000)));
        let event = set_day_keys(&state, owner, true, 90, 5_000).unwrap();
        assert!(matches!(event, ServerEvent::DayKeysSet { on: true, since: 1_000, history_days: 90, .. }));
        assert_eq!(set_day_keys(&state, owner, true, 3, 5_000).unwrap_err(), UNKNOWN_DAYS);
        assert!(matches!(set_day_keys(&state, owner, false, 30, 6_000), Ok(ServerEvent::DayKeysSet { on: false, .. })));
    }

    #[test]
    fn people_who_do_not_manage_the_server_cannot_change_it_and_outsiders_see_nothing() {
        let (mut state, _) = owned();
        let profile = UserProfile::new("member");
        let member = profile.id;
        state.apply(ServerEvent::MemberJoin { server: state.server.id, profile }).unwrap();
        let shown = view(&state, Some(member)).unwrap();
        assert!(!shown.can_turn_on && !shown.can_turn_off);
        assert_eq!(shown.waiting, 2);
        assert_eq!(set_day_keys(&state, member, true, 30, 1).unwrap_err(), TURN_ON_NEEDS);
        assert!(view(&state, Some(UserId::new())).is_none());
        assert!(view(&state, None).is_none());
    }
}
