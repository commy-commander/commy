use crate::act;
use commy_core::{ChannelId, Permissions, ServerState, UserId};

pub const DEFAULT_TIMEOUT_MINUTES: u32 = 10;

pub const MAX_TIMEOUT_MINUTES: u32 = 240;

const NO_TIMEOUT: i64 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleVoice {
    Stay,
    MoveTo(ChannelId),
    Leave,
}

pub fn clamp_timeout_minutes(minutes: u32) -> u32 {
    minutes.min(MAX_TIMEOUT_MINUTES)
}

pub fn voice_timeout_secs(personal_secs: i64, server_minutes: Option<u32>) -> i64 {
    let server = server_minutes.map_or(NO_TIMEOUT, |m| m as i64 * 60);
    match (personal_secs > NO_TIMEOUT, server > NO_TIMEOUT) {
        (true, true) => personal_secs.min(server),
        (true, false) => personal_secs,
        (false, true) => server,
        (false, false) => NO_TIMEOUT,
    }
}

pub fn idle_voice_step(
    state: &ServerState,
    perms: Permissions,
    me: UserId,
    current: ChannelId,
    idle_secs: i64,
    timeout_secs: i64,
) -> IdleVoice {
    if timeout_secs <= NO_TIMEOUT || idle_secs < timeout_secs {
        return IdleVoice::Stay;
    }
    let joinable_afk = state
        .server
        .afk_channel()
        .filter(|afk| *afk == current || act::join_voice(state, perms, me, *afk).is_ok());
    match joinable_afk {
        Some(afk) if afk == current => IdleVoice::Stay,
        Some(afk) => IdleVoice::MoveTo(afk),
        None if state.server.afk_timeout_minutes.is_some() => IdleVoice::Leave,
        None => IdleVoice::Stay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Server, ServerEvent, UserProfile};

    fn hangout() -> (ServerState, UserId, ChannelId, ChannelId) {
        let owner = UserId::new();
        let mut state = ServerState::new(Server::new("Hangout", owner));
        let member = UserProfile::new("friend");
        let id = member.id;
        state.apply(ServerEvent::MemberJoin { server: state.server.id, profile: member }).unwrap();
        let room = |name: &str| state.server.channels.iter().find(|c| c.name == name).unwrap().id;
        let (general, afk) = (room("General"), room("AFK"));
        (state, id, general, afk)
    }

    fn step(state: &ServerState, me: UserId, current: ChannelId, idle_secs: i64, timeout_secs: i64) -> IdleVoice {
        idle_voice_step(state, state.server.permissions_for(me), me, current, idle_secs, timeout_secs)
    }

    #[test]
    fn shorter_voice_timeout_wins() {
        assert_eq!(voice_timeout_secs(600, Some(5)), 300);
        assert_eq!(voice_timeout_secs(120, Some(5)), 120);
        assert_eq!(voice_timeout_secs(0, Some(1)), 60, "server limit applies even with yours off");
        assert_eq!(voice_timeout_secs(600, None), 600);
        assert_eq!(voice_timeout_secs(0, None), 0);
    }

    #[test]
    fn timeout_is_clamped() {
        assert_eq!(clamp_timeout_minutes(0), 0);
        assert_eq!(clamp_timeout_minutes(10), 10);
        assert_eq!(clamp_timeout_minutes(10_000), MAX_TIMEOUT_MINUTES);
    }

    #[test]
    fn an_idle_member_is_moved_to_afk_once_the_timeout_passes() {
        let (state, me, general, afk) = hangout();
        assert_eq!(step(&state, me, general, 599, 600), IdleVoice::Stay);
        assert_eq!(step(&state, me, general, 600, 600), IdleVoice::MoveTo(afk));
        assert_eq!(step(&state, me, general, 15 * 3600, 600), IdleVoice::MoveTo(afk));
    }

    #[test]
    fn a_member_already_in_afk_stays_there() {
        let (state, me, _, afk) = hangout();
        assert_eq!(step(&state, me, afk, 15 * 3600, 600), IdleVoice::Stay);
    }

    #[test]
    fn no_timeout_means_nobody_is_moved() {
        let (state, me, general, _) = hangout();
        assert_eq!(step(&state, me, general, 15 * 3600, 0), IdleVoice::Stay);
    }

    #[test]
    fn without_an_afk_room_only_a_server_limit_disconnects() {
        let (mut state, me, general, _) = hangout();
        let sid = state.server.id;
        state.apply(ServerEvent::AfkChannelSet { server: sid, channel: None }).unwrap();
        assert_eq!(step(&state, me, general, 15 * 3600, 600), IdleVoice::Stay);
        state.apply(ServerEvent::AfkTimeoutSet { server: sid, minutes: Some(15) }).unwrap();
        assert_eq!(step(&state, me, general, 15 * 3600, 600), IdleVoice::Leave);
    }

    #[test]
    fn a_full_afk_room_falls_back_to_the_server_limit() {
        let (mut state, me, general, afk) = hangout();
        let sid = state.server.id;
        let owner = state.server.owner;
        state.apply(ServerEvent::ChannelLimit { server: sid, channel: afk, user_limit: 1 }).unwrap();
        state.apply(ServerEvent::VoiceState { user: owner, channel: Some(afk), muted: false, deafened: false, since: None }).unwrap();
        assert_eq!(step(&state, me, general, 15 * 3600, 600), IdleVoice::Stay);
        state.apply(ServerEvent::AfkTimeoutSet { server: sid, minutes: Some(15) }).unwrap();
        assert_eq!(step(&state, me, general, 15 * 3600, 600), IdleVoice::Leave);
    }
}
