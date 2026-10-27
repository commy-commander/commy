use commy_core::action::authorized;
use commy_core::drive::{self, DriveFile, DriveItem};
use commy_core::user::Presence;
use commy_core::{ChannelId, Permissions, ServerEvent, ServerState, UserId};
use commy_core::ids::DriveId;
use serde::Serialize;

use crate::act::Act;

pub const NOT_A_DRIVE: &str = "That channel is not a drive";
pub const NO_SUCH_ITEM: &str = "That file or folder is no longer in the drive";
pub const BAD_NAME: &str = "Names can be up to 80 characters, without slashes";
pub const DRIVE_FULL: &str = "This server's drives are full. Remove something first";
pub const ALREADY: &str = "Nothing to change";
pub const TOO_DEEP: &str = "Folders can only go 6 deep";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DriveItemView {
    pub id: String,
    pub parent: Option<String>,
    pub name: String,
    pub folder: bool,
    pub size: Option<u64>,
    pub mime: Option<String>,
    pub blob: Option<String>,
    pub by: String,
    pub by_name: String,
    pub at: i64,
    pub keepers: usize,
    pub keepers_online: usize,
    pub sources: Vec<String>,
    pub kept_here: bool,
    pub can_edit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DriveView {
    pub channel: String,
    pub items: Vec<DriveItemView>,
    pub can_add: bool,
    pub can_upload: bool,
    pub max_items: usize,
    pub used_items: usize,
}

fn drive_channel(state: &ServerState, channel: ChannelId, me: UserId) -> Act<()> {
    let found = state.server.channel(channel).filter(|c| c.is_drive()).ok_or(NOT_A_DRIVE)?;
    if !state.server.can_view(found.id, me) {
        return Err(NOT_A_DRIVE.into());
    }
    Ok(())
}

fn online(state: &ServerState, user: UserId) -> bool {
    state.profiles.get(&user).is_some_and(|p| !matches!(p.presence, Presence::Offline | Presence::Invisible))
}

fn item_view(state: &ServerState, item: &DriveItem, me: UserId, node: Option<&str>, perms: Permissions) -> DriveItemView {
    let mut sources: Vec<(bool, String)> = item.holders.iter().filter(|(user, _)| **user != me).map(|(user, node)| (online(state, *user), node.clone())).collect();
    sources.sort_by_key(|(on, _)| !*on);
    DriveItemView {
        id: item.id.to_string(),
        parent: item.parent.map(|p| p.to_string()),
        name: item.name.clone(),
        folder: item.is_folder(),
        size: item.file.as_ref().map(|f| f.size),
        mime: item.file.as_ref().map(|f| f.mime.clone()),
        blob: item.file.as_ref().map(|f| f.blob.clone()),
        by: item.by.to_string(),
        by_name: crate::display_name(state, item.by),
        at: item.at,
        keepers: item.holders.len(),
        keepers_online: item.holders.keys().filter(|u| **u == me || online(state, **u)).count(),
        sources: sources.into_iter().map(|(_, node)| node).collect(),
        kept_here: node.is_some_and(|n| item.holders.get(&me).is_some_and(|h| h.eq_ignore_ascii_case(n))),
        can_edit: item.by == me || perms.allows(Permissions::MANAGE_MESSAGES),
    }
}

pub fn view(state: &ServerState, channel: ChannelId, me: UserId, node: Option<&str>) -> Act<DriveView> {
    drive_channel(state, channel, me)?;
    let perms = state.server.permissions_for(me);
    let member = state.server.is_current_member(me);
    let can_add = member && state.server.can_post_in(channel, me) && !state.server.is_timed_out(me, chrono::Utc::now());
    let mut items: Vec<DriveItemView> =
        state.server.drive.iter().filter(|i| i.channel == channel).map(|i| item_view(state, i, me, node, perms)).collect();
    items.sort_by(|a, b| b.folder.cmp(&a.folder).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(DriveView {
        channel: channel.to_string(),
        items,
        can_add,
        can_upload: can_add && perms.allows(Permissions::ATTACH_FILES),
        max_items: drive::MAX_DRIVE_ITEMS,
        used_items: state.server.drive.len(),
    })
}

fn room_for(state: &ServerState, channel: ChannelId, parent: Option<DriveId>) -> Act<()> {
    if state.server.drive.len() >= drive::MAX_DRIVE_ITEMS {
        return Err(DRIVE_FULL.into());
    }
    if !drive::parent_fits(&state.server.drive, channel, parent) {
        return Err(NO_SUCH_ITEM.into());
    }
    if drive::depth(&state.server.drive, parent) >= drive::MAX_DRIVE_DEPTH {
        return Err(TOO_DEEP.into());
    }
    Ok(())
}

fn allowed(state: &ServerState, me: UserId, event: ServerEvent, action: &str) -> Act<ServerEvent> {
    if authorized(&state.server, me, &event) {
        Ok(event)
    } else {
        Err(format!("you do not have permission to {action}"))
    }
}

pub fn new_folder(state: &ServerState, me: UserId, channel: ChannelId, parent: Option<DriveId>, name: &str, now: i64) -> Act<ServerEvent> {
    drive_channel(state, channel, me)?;
    room_for(state, channel, parent)?;
    let item = DriveItem::folder(channel, parent, name, me, now).ok_or(BAD_NAME)?;
    allowed(state, me, ServerEvent::DriveAdd { server: state.server.id, item }, "add folders here")
}

#[allow(clippy::too_many_arguments)]
pub fn add_file(state: &ServerState, me: UserId, channel: ChannelId, parent: Option<DriveId>, name: &str, file: DriveFile, node: &str, now: i64) -> Act<ServerEvent> {
    drive_channel(state, channel, me)?;
    room_for(state, channel, parent)?;
    if !file.is_valid() {
        return Err("files up to 4 GB can go in a drive".into());
    }
    let item = DriveItem::file(channel, parent, name, file, me, node, now).ok_or(BAD_NAME)?;
    allowed(state, me, ServerEvent::DriveAdd { server: state.server.id, item }, "upload files here")
}

fn existing(state: &ServerState, item: DriveId) -> Act<&DriveItem> {
    drive::find(&state.server.drive, item).ok_or_else(|| NO_SUCH_ITEM.to_string())
}

pub fn edit(state: &ServerState, me: UserId, item: DriveId, name: &str, parent: Option<DriveId>) -> Act<ServerEvent> {
    let found = existing(state, item)?;
    let name = name.trim();
    if !drive::is_name(name) {
        return Err(BAD_NAME.into());
    }
    if found.parent != parent {
        if !drive::parent_fits(&state.server.drive, found.channel, parent) {
            return Err(NO_SUCH_ITEM.into());
        }
        let mut trial = state.server.drive.clone();
        if !drive::edit(&mut trial, item, name, parent) {
            return Err("A folder can't go inside itself, or more than 6 deep".into());
        }
    }
    let event = ServerEvent::DriveEdit { server: state.server.id, item, name: name.to_string(), parent };
    allowed(state, me, event, "change this")
}

pub fn remove(state: &ServerState, me: UserId, item: DriveId) -> Act<ServerEvent> {
    existing(state, item)?;
    allowed(state, me, ServerEvent::DriveRemove { server: state.server.id, item }, "remove this")
}

pub fn keep(state: &ServerState, me: UserId, item: DriveId, node: Option<&str>) -> Act<ServerEvent> {
    let found = existing(state, item)?;
    if found.is_folder() {
        return Err("Only files can be kept".into());
    }
    let held = found.holders.get(&me);
    if node.is_some_and(|n| held.is_some_and(|h| h.eq_ignore_ascii_case(n))) || (node.is_none() && held.is_none()) {
        return Err(ALREADY.into());
    }
    let event = ServerEvent::DriveHold { server: state.server.id, item, user: me, node: node.map(str::to_ascii_lowercase) };
    allowed(state, me, event, "keep this file")
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::server::{Channel, ChannelKind};
    use commy_core::{Server, UserProfile};

    fn node(n: u8) -> String {
        format!("{n:02x}").repeat(32)
    }

    fn fixture() -> (ServerState, UserId, ChannelId) {
        let owner = UserProfile::new("owner");
        let mut server = Server::new("test", owner.id);
        let drive = Channel::new("files", ChannelKind::Drive, 9);
        let id = drive.id;
        server.channels.push(drive);
        let mut state = ServerState::new(server);
        state.profiles.insert(owner.id, owner.clone());
        (state, owner.id, id)
    }

    fn join(state: &mut ServerState, name: &str) -> UserId {
        let profile = UserProfile::new(name);
        let id = profile.id;
        state.apply(ServerEvent::MemberJoin { server: state.server.id, profile }).unwrap();
        id
    }

    #[test]
    fn members_build_folders_and_upload_files() {
        let (mut state, owner, channel) = fixture();
        let friend = join(&mut state, "friend");
        let folder = new_folder(&state, friend, channel, None, " Maps ", 1).unwrap();
        let ServerEvent::DriveAdd { item: ref made, .. } = folder else { panic!() };
        let maps = made.id;
        assert_eq!(made.name, "Maps");
        state.apply(folder).unwrap();
        let file = DriveFile { blob: node(1), size: 5_000_000_000 / 2, mime: "application/zip".into() };
        state.apply(add_file(&state, friend, channel, Some(maps), "map.zip", file, &node(9), 2).unwrap()).unwrap();
        let shown = view(&state, channel, friend, Some(&node(9))).unwrap();
        assert_eq!(shown.items.iter().map(|i| (i.name.as_str(), i.folder)).collect::<Vec<_>>(), vec![("Maps", true), ("map.zip", false)]);
        let zip = &shown.items[1];
        assert!(zip.kept_here && zip.can_edit && zip.sources.is_empty());
        assert_eq!((zip.keepers, zip.parent.as_deref()), (1, Some(maps.to_string().as_str())));
        let seen_by_owner = view(&state, channel, owner, Some(&node(4))).unwrap();
        assert_eq!(seen_by_owner.items[1].sources, vec![node(9)]);
        assert!(!seen_by_owner.items[1].kept_here && seen_by_owner.items[1].can_edit, "managers can tidy the drive");
        assert_eq!(new_folder(&state, friend, channel, None, "a/b", 3).unwrap_err(), BAD_NAME);
        assert_eq!(new_folder(&state, friend, ChannelId::new(), None, "x", 3).unwrap_err(), NOT_A_DRIVE);
    }

    #[test]
    fn only_the_uploader_or_a_moderator_changes_an_item() {
        let (mut state, owner, channel) = fixture();
        let (a, b) = (join(&mut state, "a"), join(&mut state, "b"));
        let event = new_folder(&state, a, channel, None, "Clips", 1).unwrap();
        let ServerEvent::DriveAdd { item: ref made, .. } = event else { panic!() };
        let id = made.id;
        state.apply(event).unwrap();
        assert!(edit(&state, b, id, "Mine now", None).is_err());
        assert!(remove(&state, b, id).is_err());
        assert!(edit(&state, a, id, "Best clips", None).is_ok());
        assert!(remove(&state, owner, id).is_ok());
        assert_eq!(edit(&state, a, id, "", None).unwrap_err(), BAD_NAME);
        assert!(edit(&state, a, id, "Clips", Some(id)).is_err());
    }

    #[test]
    fn downloaders_keep_files_for_others() {
        let (mut state, owner, channel) = fixture();
        let friend = join(&mut state, "friend");
        let file = DriveFile { blob: node(1), size: 10, mime: "text/plain".into() };
        let event = add_file(&state, owner, channel, None, "notes.txt", file, &node(2), 1).unwrap();
        let ServerEvent::DriveAdd { item: ref made, .. } = event else { panic!() };
        let id = made.id;
        state.apply(event).unwrap();
        state.apply(keep(&state, friend, id, Some(&node(3).to_uppercase())).unwrap()).unwrap();
        let shown = view(&state, channel, friend, Some(&node(3))).unwrap();
        assert!(shown.items[0].kept_here);
        assert_eq!(shown.items[0].keepers, 2);
        assert_eq!(keep(&state, friend, id, Some(&node(3))).unwrap_err(), ALREADY);
        state.apply(keep(&state, friend, id, None).unwrap()).unwrap();
        assert_eq!(keep(&state, friend, id, None).unwrap_err(), ALREADY);
        assert_eq!(view(&state, channel, friend, Some(&node(3))).unwrap().items[0].keepers, 1);
        let folder = new_folder(&state, owner, channel, None, "f", 2).unwrap();
        let ServerEvent::DriveAdd { item: ref f, .. } = folder else { panic!() };
        let fid = f.id;
        state.apply(folder).unwrap();
        assert!(keep(&state, friend, fid, Some(&node(3))).is_err());
    }
}
