use commy_core::template::{text_channel_name, MAX_RETENTION_DAYS};
use commy_core::server::Role;
use commy_core::{Channel, ChannelId, ChannelKind, Permissions, RoleId, ServerEvent, ServerState};

use crate::act::Act;

pub const MAX_SERVER_NAME_BYTES: usize = 100;
pub const MAX_CHANNEL_NAME_BYTES: usize = 50;
pub const MAX_CHANNEL_MOVES: usize = 200;
pub const SERVER_IMAGE_INVALID: &str = "server images must be a JPEG or PNG under 1 MB";
pub const CHANNEL_BANNER_INVALID: &str = "channel banners must be a JPEG or PNG under 1 MB";
pub const ROLE_BANNER_INVALID: &str = "role banners must be a JPEG or PNG under 1 MB";

pub struct ChannelMove {
    pub channel: ChannelId,
    pub parent: Option<ChannelId>,
    pub position: u32,
}

pub fn need(perms: Permissions, needed: Permissions, action: &str) -> Act<()> {
    if perms.allows(needed) {
        Ok(())
    } else {
        Err(format!("you do not have permission to {action}"))
    }
}

pub fn server_name(raw: &str) -> Act<String> {
    let name = raw.trim();
    if name.is_empty() || name.len() > MAX_SERVER_NAME_BYTES {
        return Err("server name must be 1 to 100 characters".into());
    }
    Ok(name.to_string())
}

pub fn channel_name(name: String) -> Act<String> {
    if name.is_empty() || name.len() > MAX_CHANNEL_NAME_BYTES {
        return Err("channel name must be 1 to 50 characters".into());
    }
    Ok(name)
}

pub fn channel_kind(raw: &str) -> Act<ChannelKind> {
    match raw {
        "text" => Ok(ChannelKind::Text),
        "voice" => Ok(ChannelKind::Voice),
        "announcement" => Ok(ChannelKind::Announcement),
        "forum" => Ok(ChannelKind::Forum),
        "stage" => Ok(ChannelKind::Stage),
        "drive" => Ok(ChannelKind::Drive),
        _ => Err("channel kind must be text, voice, stage, announcement, forum or drive".into()),
    }
}

pub fn rename(state: &ServerState, perms: Permissions, name: &str) -> Act<ServerEvent> {
    let name = server_name(name)?;
    need(perms, Permissions::MANAGE_SERVER, "rename the server")?;
    Ok(ServerEvent::ServerRename { server: state.server.id, name })
}

pub fn create_channel(state: &ServerState, perms: Permissions, name: &str, kind: &str, parent: Option<ChannelId>) -> Act<ServerEvent> {
    let name = channel_name(text_channel_name(name))?;
    let kind = channel_kind(kind)?;
    need(perms, Permissions::MANAGE_CHANNELS, "create channels")?;
    if state.server.channels.iter().any(|c| c.name == name && c.kind == kind) {
        return Err("a channel with that name already exists".into());
    }
    let mut channel = Channel::new(name, kind, state.server.channels.len() as u32);
    channel.parent = parent.filter(|p| state.server.channel(*p).is_some_and(|c| c.kind == ChannelKind::Category));
    Ok(ServerEvent::ChannelCreate { server: state.server.id, channel })
}

pub fn create_category(state: &ServerState, perms: Permissions, name: &str) -> Act<ServerEvent> {
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > MAX_CHANNEL_NAME_BYTES {
        return Err("category names must be 1 to 50 characters".into());
    }
    need(perms, Permissions::MANAGE_CHANNELS, "create categories")?;
    let position = state.server.channels.iter().map(|c| c.position).max().unwrap_or(0) + 1;
    Ok(ServerEvent::ChannelCreate { server: state.server.id, channel: Channel::new(name, ChannelKind::Category, position) })
}

pub fn update_channel(state: &ServerState, perms: Permissions, channel: ChannelId, name: &str, retention_days: Option<u32>) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_CHANNELS, "edit channels")?;
    let kind = state.server.channel(channel).ok_or("unknown channel")?.kind;
    let name = channel_name(if kind.holds_messages() { text_channel_name(name) } else { name.trim().to_string() })?;
    let retention_days = retention_days.filter(|d| *d > 0).map(|d| d.min(MAX_RETENTION_DAYS));
    Ok(ServerEvent::ChannelUpdate { server: state.server.id, channel, name, retention_days })
}

pub fn channel_kind_set(state: &ServerState, perms: Permissions, channel: ChannelId, kind: &str) -> Act<ServerEvent> {
    let kind = match kind {
        "text" => ChannelKind::Text,
        "announcement" => ChannelKind::Announcement,
        _ => return Err("channel kind must be text or announcement".into()),
    };
    need(perms, Permissions::MANAGE_CHANNELS, "edit channels")?;
    let current = state.server.channel(channel).ok_or("unknown channel")?;
    if !matches!(current.kind, ChannelKind::Text | ChannelKind::Announcement) {
        return Err("only text channels can become announcement channels".into());
    }
    Ok(ServerEvent::ChannelKindSet { server: state.server.id, channel, kind })
}

pub fn delete_channel(state: &ServerState, perms: Permissions, channel: ChannelId) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_CHANNELS, "delete channels")?;
    if !state.server.channels.iter().any(|c| c.kind == ChannelKind::Text && c.id != channel) {
        return Err("a server needs at least one text channel".into());
    }
    Ok(ServerEvent::ChannelDelete { server: state.server.id, channel })
}

pub fn move_channels(state: &ServerState, perms: Permissions, moves: Vec<ChannelMove>) -> Act<Vec<ServerEvent>> {
    need(perms, Permissions::MANAGE_CHANNELS, "move channels")?;
    let server = state.server.id;
    Ok(moves
        .into_iter()
        .take(MAX_CHANNEL_MOVES)
        .map(|m| ServerEvent::ChannelLayout { server, channel: m.channel, parent: m.parent, position: m.position })
        .collect())
}

pub fn server_images(state: &ServerState, perms: Permissions, which: &str, image: Option<String>) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_SERVER, "change the server's images")?;
    let server = state.server.id;
    match which {
        "icon" => Ok(ServerEvent::ServerImages { server, icon: image, banner: state.server.banner.clone() }),
        "banner" => Ok(ServerEvent::ServerImages { server, icon: state.server.icon.clone(), banner: image }),
        "invite" => Ok(ServerEvent::InviteBackgroundSet { server, background: image }),
        _ => Err("unknown server image".into()),
    }
}

pub fn channel_banner(state: &ServerState, perms: Permissions, channel: ChannelId, banner: Option<String>) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_CHANNELS, "edit channels")?;
    state.server.channel(channel).ok_or("unknown channel")?;
    Ok(ServerEvent::ChannelBanner { server: state.server.id, channel, banner })
}

pub fn role_banner(state: &ServerState, perms: Permissions, role: RoleId, banner: Option<String>) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_ROLES, "edit roles")?;
    let existing = state.server.roles.iter().find(|r| r.id == role).cloned().ok_or("unknown role")?;
    Ok(ServerEvent::RoleUpdate { server: state.server.id, role: Role { banner, ..existing } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Server, UserId};

    fn hangout() -> (ServerState, UserId, UserId) {
        let owner = UserId::new();
        let member = UserId::new();
        let state = ServerState::new(Server::new("Hangout", owner));
        (state, owner, member)
    }

    #[test]
    fn only_the_right_people_change_server_channel_and_role_pictures() {
        let (state, owner, member) = hangout();
        let (mine, theirs) = (state.server.permissions_for(owner), state.server.permissions_for(member));
        let picture = Some("pic".to_string());
        assert!(server_images(&state, theirs, "icon", picture.clone()).is_err());
        assert!(matches!(server_images(&state, mine, "icon", picture.clone()), Ok(ServerEvent::ServerImages { icon: Some(_), banner: None, .. })));
        assert!(matches!(server_images(&state, mine, "banner", picture.clone()), Ok(ServerEvent::ServerImages { icon: None, banner: Some(_), .. })));
        assert!(matches!(server_images(&state, mine, "invite", None), Ok(ServerEvent::InviteBackgroundSet { background: None, .. })));
        assert_eq!(server_images(&state, mine, "wallpaper", None).unwrap_err(), "unknown server image");
        let channel = state.server.channels[0].id;
        assert!(channel_banner(&state, theirs, channel, picture.clone()).is_err());
        assert!(matches!(channel_banner(&state, mine, channel, picture.clone()), Ok(ServerEvent::ChannelBanner { banner: Some(_), .. })));
        assert_eq!(channel_banner(&state, mine, ChannelId::new(), None).unwrap_err(), "unknown channel");
        let role = state.server.roles[0].clone();
        assert!(role_banner(&state, theirs, role.id, picture.clone()).is_err());
        let Ok(ServerEvent::RoleUpdate { role: changed, .. }) = role_banner(&state, mine, role.id, picture) else { panic!() };
        assert_eq!((changed.banner.as_deref(), changed.name), (Some("pic"), role.name));
        assert_eq!(role_banner(&state, mine, RoleId::new(), None).unwrap_err(), "unknown role");
    }

    #[test]
    fn server_names_are_trimmed_and_bounded() {
        assert_eq!(server_name("  Friends ").unwrap(), "Friends");
        assert!(server_name("   ").is_err());
        assert!(server_name(&"a".repeat(101)).is_err());
        assert!(server_name(&"a".repeat(100)).is_ok());
    }

    #[test]
    fn only_managers_rename_servers_and_edit_channels() {
        let (state, owner, member) = hangout();
        let theirs = state.server.permissions_for(member);
        let mine = state.server.permissions_for(owner);
        assert!(rename(&state, theirs, "New name").is_err());
        assert!(matches!(rename(&state, mine, "New name"), Ok(ServerEvent::ServerRename { name, .. }) if name == "New name"));
        assert!(create_channel(&state, theirs, "memes", "text", None).is_err());
        assert!(create_category(&state, theirs, "Games").is_err());
    }

    #[test]
    fn new_text_channels_are_lowercase_dashed_and_unique_per_kind() {
        let (state, owner, _) = hangout();
        let perms = state.server.permissions_for(owner);
        let Ok(ServerEvent::ChannelCreate { channel, .. }) = create_channel(&state, perms, "Cat Pictures", "text", None) else { panic!() };
        assert_eq!((channel.name.as_str(), channel.kind), ("cat-pictures", ChannelKind::Text));
        assert_eq!(create_channel(&state, perms, "general", "text", None).unwrap_err(), "a channel with that name already exists");
        assert!(create_channel(&state, perms, "general", "voice", None).is_ok());
        assert!(create_channel(&state, perms, "x", "dungeon", None).is_err());
    }

    #[test]
    fn a_new_channel_only_sits_under_a_category() {
        let (mut state, owner, _) = hangout();
        let perms = state.server.permissions_for(owner);
        let text = state.server.channels[0].id;
        let event = create_category(&state, perms, "Games").unwrap();
        let ServerEvent::ChannelCreate { channel: category, .. } = &event else { panic!() };
        let category = category.id;
        state.apply(event).unwrap();
        let Ok(ServerEvent::ChannelCreate { channel, .. }) = create_channel(&state, perms, "chess", "text", Some(category)) else { panic!() };
        assert_eq!(channel.parent, Some(category));
        let Ok(ServerEvent::ChannelCreate { channel, .. }) = create_channel(&state, perms, "go", "text", Some(text)) else { panic!() };
        assert_eq!(channel.parent, None);
    }

    #[test]
    fn the_last_text_channel_stays() {
        let (state, owner, _) = hangout();
        let perms = state.server.permissions_for(owner);
        let texts: Vec<ChannelId> = state.server.channels.iter().filter(|c| c.kind == ChannelKind::Text).map(|c| c.id).collect();
        let last = *texts.last().unwrap();
        if texts.len() == 1 {
            assert_eq!(delete_channel(&state, perms, last).unwrap_err(), "a server needs at least one text channel");
        }
        let voice = state.server.channels.iter().find(|c| c.kind == ChannelKind::Voice).map(|c| c.id).unwrap();
        assert!(delete_channel(&state, perms, voice).is_ok());
    }

    #[test]
    fn retention_is_capped_and_zero_means_keep_forever() {
        let (state, owner, _) = hangout();
        let perms = state.server.permissions_for(owner);
        let text = state.server.channels[0].id;
        let Ok(ServerEvent::ChannelUpdate { retention_days, name, .. }) = update_channel(&state, perms, text, "Chat Room", Some(9999)) else { panic!() };
        assert_eq!((name.as_str(), retention_days), ("chat-room", Some(MAX_RETENTION_DAYS)));
        let Ok(ServerEvent::ChannelUpdate { retention_days, .. }) = update_channel(&state, perms, text, "chat", Some(0)) else { panic!() };
        assert_eq!(retention_days, None);
    }

    #[test]
    fn only_text_channels_turn_into_announcements_and_back() {
        let (state, owner, member) = hangout();
        let perms = state.server.permissions_for(owner);
        let text = state.server.channels.iter().find(|c| c.kind == ChannelKind::Text).map(|c| c.id).unwrap();
        let voice = state.server.channels.iter().find(|c| c.kind == ChannelKind::Voice).map(|c| c.id).unwrap();
        assert!(matches!(channel_kind_set(&state, perms, text, "announcement"), Ok(ServerEvent::ChannelKindSet { kind: ChannelKind::Announcement, .. })));
        assert!(channel_kind_set(&state, perms, voice, "announcement").is_err());
        assert!(channel_kind_set(&state, perms, text, "voice").is_err());
        assert!(channel_kind_set(&state, state.server.permissions_for(member), text, "text").is_err());
    }

    #[test]
    fn channel_moves_are_capped() {
        let (state, owner, _) = hangout();
        let perms = state.server.permissions_for(owner);
        let text = state.server.channels[0].id;
        let moves = (0..300).map(|i| ChannelMove { channel: text, parent: None, position: i }).collect();
        assert_eq!(move_channels(&state, perms, moves).unwrap().len(), MAX_CHANNEL_MOVES);
    }
}
