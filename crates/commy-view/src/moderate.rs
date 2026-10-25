use chrono::{DateTime, Duration, Utc};
use commy_core::server::{clean_nickname, valid_hex_color, valid_rules, MAX_ACCESS_ROLES, MAX_NICKNAME_CHARS, MAX_SLOW_MODE, MAX_TIMEOUT_SECS, MAX_USER_LIMIT, SERVER_AFK_MINUTES};
use commy_core::{ChannelId, ChannelKind, Permissions, RoleId, ServerEvent, ServerState, UserId};

use crate::act::Act;
use crate::manage::need;

pub fn kick(state: &ServerState, perms: Permissions, user: UserId) -> Act<ServerEvent> {
    need(perms, Permissions::KICK_MEMBERS, "kick members")?;
    if user == state.server.owner {
        return Err("the owner cannot be kicked".into());
    }
    Ok(ServerEvent::MemberLeave { server: state.server.id, user })
}

pub fn ban(state: &ServerState, perms: Permissions, me: UserId, user: UserId) -> Act<ServerEvent> {
    need(perms, Permissions::BAN_MEMBERS, "ban members")?;
    if user == state.server.owner || user == me {
        return Err("you cannot ban that member".into());
    }
    Ok(ServerEvent::MemberBan { server: state.server.id, user })
}

pub fn unban(state: &ServerState, perms: Permissions, user: UserId) -> Act<ServerEvent> {
    need(perms, Permissions::BAN_MEMBERS, "unban members")?;
    Ok(ServerEvent::MemberUnban { server: state.server.id, user })
}

pub fn timeout(state: &ServerState, perms: Permissions, me: UserId, user: UserId, seconds: Option<u32>, at: DateTime<Utc>) -> Act<ServerEvent> {
    need(perms, Permissions::TIME_OUT_MEMBERS, "time out members")?;
    if user == state.server.owner || user == me {
        return Err("you cannot time out that member".into());
    }
    if !state.server.is_member(user) {
        return Err("that member is no longer in the server".into());
    }
    let until = match seconds {
        Some(secs) if i64::from(secs) > MAX_TIMEOUT_SECS => return Err("a timeout can last four weeks at most".into()),
        Some(0) => return Err("choose how long the timeout lasts".into()),
        Some(secs) => Some(at + Duration::seconds(i64::from(secs))),
        None => None,
    };
    Ok(ServerEvent::MemberTimeout { server: state.server.id, user, by: me, at, until })
}

pub fn nickname(state: &ServerState, perms: Permissions, me: UserId, user: UserId, nickname: Option<&str>) -> Act<ServerEvent> {
    let nickname = match nickname.map(str::trim).filter(|n| !n.is_empty()) {
        None => None,
        Some(text) => Some(clean_nickname(text).ok_or_else(|| format!("nicknames can be up to {MAX_NICKNAME_CHARS} characters"))?),
    };
    if !state.server.is_member(user) {
        return Err("that person is not a member of this server".into());
    }
    let own = user == me && perms.allows(Permissions::CHANGE_NICKNAME);
    if !own {
        need(perms, Permissions::MANAGE_NICKNAMES, "change nicknames")?;
        if user == state.server.owner {
            return Err("only the server owner can change their own nickname".into());
        }
    }
    Ok(ServerEvent::MemberNickname { server: state.server.id, user, nickname })
}

fn named_as_shown(state: &ServerState, confirm_name: &str, action: &str) -> Act<()> {
    if confirm_name.trim() == state.server.name.trim() {
        Ok(())
    } else {
        Err(format!("type the server name exactly as it is shown to {action}"))
    }
}

pub fn transfer(state: &ServerState, me: UserId, user: UserId, confirm_name: &str) -> Act<ServerEvent> {
    if state.server.owner != me {
        return Err("only the owner can hand over this server".into());
    }
    if user == me || !state.server.is_member(user) {
        return Err("pick another member of this server".into());
    }
    named_as_shown(state, confirm_name, "hand it over")?;
    Ok(ServerEvent::OwnerTransfer { server: state.server.id, user })
}

pub fn delete(state: &ServerState, me: UserId, confirm_name: &str) -> Act<ServerEvent> {
    if state.server.owner != me {
        return Err("only the owner can delete this server".into());
    }
    named_as_shown(state, confirm_name, "delete it")?;
    Ok(ServerEvent::ServerDelete { server: state.server.id })
}

pub fn rules(state: &ServerState, perms: Permissions, rules: Option<&str>) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_SERVER, "change the rules")?;
    let rules = rules.map(str::trim).filter(|r| !r.is_empty()).map(str::to_string);
    if rules.as_deref().is_some_and(|r| !valid_rules(r)) {
        return Err("rules are too long; keep them under about 3000 characters".into());
    }
    Ok(ServerEvent::RulesSet { server: state.server.id, rules })
}

pub fn accept_rules(state: &ServerState, me: UserId) -> Act<ServerEvent> {
    Ok(ServerEvent::RulesAccept { server: state.server.id, user: me })
}

pub fn afk_channel(state: &ServerState, perms: Permissions, channel: Option<ChannelId>) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_SERVER, "change the AFK channel")?;
    Ok(ServerEvent::AfkChannelSet { server: state.server.id, channel })
}

pub fn afk_timeout(state: &ServerState, perms: Permissions, minutes: Option<u32>) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_SERVER, "change the AFK timeout")?;
    let minutes = minutes.filter(|m| *m > 0);
    if minutes.is_some_and(|m| !SERVER_AFK_MINUTES.contains(&m)) {
        return Err("pick 1, 5, 15, 30 or 60 minutes".into());
    }
    Ok(ServerEvent::AfkTimeoutSet { server: state.server.id, minutes })
}

pub fn public(state: &ServerState, perms: Permissions, public: bool) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_SERVER, "make this server public")?;
    Ok(ServerEvent::PublicSet { server: state.server.id, public })
}

pub fn channel_limit(state: &ServerState, perms: Permissions, channel: ChannelId, limit: u32) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_CHANNELS, "change user limits")?;
    if limit > MAX_USER_LIMIT {
        return Err(format!("the limit can be at most {MAX_USER_LIMIT}"));
    }
    Ok(ServerEvent::ChannelLimit { server: state.server.id, channel, user_limit: limit })
}

pub fn slow_mode(state: &ServerState, perms: Permissions, channel: ChannelId, seconds: u32) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_CHANNELS, "change slow mode")?;
    if seconds > MAX_SLOW_MODE {
        return Err("slow mode can be at most six hours".into());
    }
    Ok(ServerEvent::ChannelSlowMode { server: state.server.id, channel, seconds })
}

pub fn channel_color(state: &ServerState, perms: Permissions, channel: ChannelId, color: Option<&str>) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_CHANNELS, "edit channels")?;
    state.server.channel(channel).ok_or("unknown channel")?;
    let color = color.map(|c| c.trim().trim_start_matches('#').to_ascii_lowercase());
    if color.as_deref().is_some_and(|c| !valid_hex_color(c)) {
        return Err("channel colour must be six hex digits".into());
    }
    Ok(ServerEvent::ChannelColor { server: state.server.id, channel, color })
}

pub fn sensitive(state: &ServerState, perms: Permissions, channel: ChannelId, sensitive: bool) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_CHANNELS, "edit channels")?;
    if !state.server.channel(channel).ok_or("unknown channel")?.kind.holds_messages() {
        return Err("only text and forum channels can be age-restricted".into());
    }
    Ok(ServerEvent::ChannelSensitiveSet { server: state.server.id, channel, sensitive })
}

pub fn channel_access(state: &ServerState, perms: Permissions, channel: ChannelId, view: Vec<RoleId>, send: Vec<RoleId>) -> Act<ServerEvent> {
    if view.len() > MAX_ACCESS_ROLES || send.len() > MAX_ACCESS_ROLES {
        return Err(format!("pick at most {MAX_ACCESS_ROLES} roles"));
    }
    need(perms, Permissions::MANAGE_CHANNELS, "edit channels")?;
    if state.server.channel(channel).ok_or("unknown channel")?.kind == ChannelKind::Category {
        return Err("categories do not hold messages to lock".into());
    }
    if [&view, &send].into_iter().flatten().any(|r| !state.server.roles.iter().any(|known| known.id == *r && !known.is_everyone())) {
        return Err("that role no longer exists".into());
    }
    Ok(ServerEvent::ChannelAccessSet { server: state.server.id, channel, view, send })
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Server, UserProfile};

    fn hangout() -> (ServerState, UserId, UserId) {
        let owner = UserId::new();
        let mut state = ServerState::new(Server::new("Hangout", owner));
        let member = UserProfile::new("friend");
        let id = member.id;
        state.apply(ServerEvent::MemberJoin { server: state.server.id, profile: member }).unwrap();
        (state, owner, id)
    }

    #[test]
    fn nobody_removes_the_owner_or_themselves() {
        let (state, owner, member) = hangout();
        let mine = state.server.permissions_for(owner);
        let theirs = state.server.permissions_for(member);
        assert_eq!(kick(&state, mine, owner).unwrap_err(), "the owner cannot be kicked");
        assert!(matches!(kick(&state, mine, member), Ok(ServerEvent::MemberLeave { user, .. }) if user == member));
        assert!(kick(&state, theirs, owner).is_err());
        assert!(ban(&state, mine, owner, owner).is_err());
        assert!(ban(&state, theirs, member, owner).is_err());
        assert!(matches!(ban(&state, mine, owner, member), Ok(ServerEvent::MemberBan { .. })));
        assert!(unban(&state, theirs, member).is_err());
    }

    #[test]
    fn timeouts_last_up_to_four_weeks() {
        let (state, owner, member) = hangout();
        let perms = state.server.permissions_for(owner);
        let now = Utc::now();
        let Ok(ServerEvent::MemberTimeout { until, by, .. }) = timeout(&state, perms, owner, member, Some(60), now) else { panic!() };
        assert_eq!((until, by), (Some(now + Duration::seconds(60)), owner));
        assert!(timeout(&state, perms, owner, member, Some(0), now).is_err());
        assert!(timeout(&state, perms, owner, member, Some(MAX_TIMEOUT_SECS as u32 + 1), now).is_err());
        assert!(matches!(timeout(&state, perms, owner, member, None, now), Ok(ServerEvent::MemberTimeout { until: None, .. })));
        assert!(timeout(&state, perms, owner, UserId::new(), Some(60), now).is_err());
        assert!(timeout(&state, state.server.permissions_for(member), member, owner, Some(60), now).is_err());
    }

    #[test]
    fn people_rename_themselves_and_managers_rename_others() {
        let (state, owner, member) = hangout();
        let theirs = state.server.permissions_for(member);
        assert!(matches!(nickname(&state, theirs, member, member, Some(" Pal ")), Ok(ServerEvent::MemberNickname { nickname: Some(n), .. }) if n == "Pal"));
        assert!(nickname(&state, theirs, member, owner, Some("Boss")).is_err());
        assert!(nickname(&state, state.server.permissions_for(owner), owner, member, None).is_ok());
        assert!(nickname(&state, theirs, member, member, Some(&"x".repeat(MAX_NICKNAME_CHARS + 1))).is_err());
    }

    #[test]
    fn deleting_or_handing_over_needs_the_owner_and_the_exact_name() {
        let (state, owner, member) = hangout();
        assert!(delete(&state, member, "Hangout").is_err());
        assert!(delete(&state, owner, "hangout").is_err());
        assert!(matches!(delete(&state, owner, " Hangout "), Ok(ServerEvent::ServerDelete { .. })));
        assert!(transfer(&state, owner, owner, "Hangout").is_err());
        assert!(transfer(&state, owner, UserId::new(), "Hangout").is_err());
        assert!(transfer(&state, owner, member, "Hang").is_err());
        assert!(matches!(transfer(&state, owner, member, "Hangout"), Ok(ServerEvent::OwnerTransfer { user, .. }) if user == member));
    }

    #[test]
    fn server_settings_check_their_bounds() {
        let (state, owner, member) = hangout();
        let perms = state.server.permissions_for(owner);
        assert!(matches!(rules(&state, perms, Some("  ")), Ok(ServerEvent::RulesSet { rules: None, .. })));
        assert!(rules(&state, state.server.permissions_for(member), Some("Be kind")).is_err());
        assert!(afk_timeout(&state, perms, Some(7)).is_err());
        assert!(matches!(afk_timeout(&state, perms, Some(0)), Ok(ServerEvent::AfkTimeoutSet { minutes: None, .. })));
        let text = state.server.channels.iter().find(|c| c.kind == ChannelKind::Text).unwrap().id;
        let voice = state.server.channels.iter().find(|c| c.kind == ChannelKind::Voice).unwrap().id;
        assert!(channel_limit(&state, perms, voice, MAX_USER_LIMIT + 1).is_err());
        assert!(slow_mode(&state, perms, text, MAX_SLOW_MODE + 1).is_err());
        assert!(matches!(channel_color(&state, perms, text, Some("#FF4D4D")), Ok(ServerEvent::ChannelColor { color: Some(c), .. }) if c == "ff4d4d"));
        assert!(channel_color(&state, perms, text, Some("red")).is_err());
        assert!(sensitive(&state, perms, voice, true).is_err());
        assert!(sensitive(&state, perms, text, true).is_ok());
        assert!(channel_access(&state, perms, text, vec![RoleId::new()], vec![]).is_err());
        assert!(channel_access(&state, perms, text, vec![], vec![]).is_ok());
    }
}
