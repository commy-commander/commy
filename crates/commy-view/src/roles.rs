use commy_core::server::{valid_hex_color, valid_role_tag, Role, MAX_ROLE_TAG_CHARS};
use commy_core::{AuditKind, Permissions, RoleId, ServerEvent, ServerState, UserId};
use serde::Serialize;

use crate::act::Act;
use crate::manage::need;
use crate::permissions_from_names;

pub const MAX_ROLE_NAME_CHARS: usize = 50;

#[derive(Default)]
pub struct RoleForm {
    pub name: String,
    pub color: Option<String>,
    pub permissions: Option<Vec<String>>,
    pub tag: Option<String>,
    pub self_assignable: Option<bool>,
    pub hoist: Option<bool>,
    pub mentionable: Option<bool>,
}

fn role_name(raw: &str) -> Act<String> {
    let name = raw.trim().to_string();
    if name.is_empty() || name.chars().count() > MAX_ROLE_NAME_CHARS {
        return Err("role names must be 1 to 50 characters".into());
    }
    Ok(name)
}

fn role_tag(raw: Option<&str>) -> Act<Option<String>> {
    let tag = raw.map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
    if tag.as_deref().is_some_and(|t| !valid_role_tag(t)) {
        return Err(format!("tags can be up to {MAX_ROLE_TAG_CHARS} characters"));
    }
    Ok(tag)
}

fn role_color(raw: Option<&str>) -> Option<String> {
    raw.map(|c| c.trim().trim_start_matches('#').to_lowercase()).filter(|c| !c.is_empty())
}

pub fn create_role(state: &ServerState, perms: Permissions, form: RoleForm) -> Act<(RoleId, ServerEvent)> {
    let name = role_name(&form.name)?;
    let color = role_color(form.color.as_deref()).filter(|c| valid_hex_color(c));
    let tag = role_tag(form.tag.as_deref())?;
    need(perms, Permissions::MANAGE_ROLES, "manage roles")?;
    let position = state.server.roles.iter().map(|r| r.position).max().unwrap_or(0) + 1;
    let role = Role {
        id: RoleId::new(),
        name,
        color,
        permissions: permissions_from_names(&form.permissions.unwrap_or_default()),
        position,
        hoist: form.hoist.unwrap_or(false),
        self_assignable: form.self_assignable.unwrap_or(false),
        tag,
        banner: None,
        mentionable: form.mentionable.unwrap_or(false),
    };
    Ok((role.id, ServerEvent::RoleCreate { server: state.server.id, role }))
}

pub fn update_role(state: &ServerState, perms: Permissions, role: RoleId, form: RoleForm) -> Act<ServerEvent> {
    let tag = role_tag(form.tag.as_deref())?;
    let color = role_color(form.color.as_deref());
    if color.as_deref().is_some_and(|c| !valid_hex_color(c)) {
        return Err("colors are six hex digits".into());
    }
    need(perms, Permissions::MANAGE_ROLES, "edit roles")?;
    let existing = state.server.roles.iter().find(|r| r.id == role).ok_or("unknown role")?;
    let role = Role {
        name: role_name(&form.name)?,
        color,
        self_assignable: form.self_assignable.unwrap_or(existing.self_assignable),
        hoist: form.hoist.unwrap_or(existing.hoist),
        tag,
        permissions: form.permissions.map_or(existing.permissions, |names| permissions_from_names(&names)),
        mentionable: form.mentionable.unwrap_or(existing.mentionable),
        ..existing.clone()
    };
    Ok(ServerEvent::RoleUpdate { server: state.server.id, role })
}

pub fn delete_role(state: &ServerState, perms: Permissions, role: RoleId) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_ROLES, "manage roles")?;
    Ok(ServerEvent::RoleDelete { server: state.server.id, role })
}

pub fn member_role(state: &ServerState, perms: Permissions, user: UserId, role: RoleId, grant: bool) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_ROLES, "manage roles")?;
    Ok(assignment(state, user, role, grant))
}

pub fn own_role(state: &ServerState, me: UserId, role: RoleId, held: bool) -> Act<ServerEvent> {
    if !state.server.roles.iter().any(|r| r.id == role && r.self_assignable) {
        return Err("that role cannot be picked".into());
    }
    Ok(assignment(state, me, role, held))
}

fn assignment(state: &ServerState, user: UserId, role: RoleId, grant: bool) -> ServerEvent {
    let server = state.server.id;
    if grant {
        ServerEvent::RoleAssign { server, user, role }
    } else {
        ServerEvent::RoleUnassign { server, user, role }
    }
}

pub fn role_access_permission(access: &str) -> Act<(Permissions, &'static str)> {
    match access {
        "files" => Ok((Permissions::ATTACH_FILES, "change who can send files")),
        "emojis" => Ok((Permissions::ADD_EMOJIS, "change who can add emojis")),
        "stickers" => Ok((Permissions::ADD_STICKERS, "change who can add stickers")),
        "emoticons" => Ok((Permissions::ADD_EMOTICONS, "change who can add emoticons")),
        "embeds" => Ok((Permissions::NO_EMBED_LINKS, "change who can embed links")),
        "history" => Ok((Permissions::NO_READ_MESSAGE_HISTORY, "change who can read message history")),
        "threads" => Ok((Permissions::NO_CREATE_THREADS, "change who can start threads")),
        "polls" => Ok((Permissions::NO_SEND_POLLS, "change who can create polls")),
        "events" => Ok((Permissions::NO_CREATE_EVENTS, "change who can create events")),
        _ => Err("unknown setting".into()),
    }
}

pub fn role_access_updates(roles: &[Role], permission: Permissions, restricted: bool, allowed: &[RoleId]) -> Vec<Role> {
    let inverted = permission.is_restriction();
    roles
        .iter()
        .filter_map(|r| {
            let mut role = r.clone();
            if inverted {
                role.permissions.remove(permission);
                if restricted && !allowed.contains(&r.id) {
                    role.permissions.insert(permission);
                }
            } else if r.is_everyone() {
                role.permissions.remove(permission);
                if !restricted {
                    role.permissions.insert(permission);
                }
            } else if restricted {
                if allowed.contains(&r.id) {
                    role.permissions.insert(permission);
                } else {
                    role.permissions.remove(permission);
                }
            }
            (role.permissions != r.permissions).then_some(role)
        })
        .collect()
}

pub fn role_access(state: &ServerState, perms: Permissions, access: &str, restricted: bool, allowed: &[RoleId]) -> Act<Vec<ServerEvent>> {
    let (permission, action) = role_access_permission(access)?;
    need(perms, Permissions::MANAGE_ROLES, action)?;
    let server = state.server.id;
    Ok(role_access_updates(&state.server.roles, permission, restricted, allowed).into_iter().map(|role| ServerEvent::RoleUpdate { server, role }).collect())
}

#[derive(Debug, Serialize)]
pub struct AuditEntryView {
    pub at: i64,
    pub by: String,
    pub by_name: String,
    pub kind: AuditKind,
    pub target: Option<String>,
    pub label: Option<String>,
    pub detail: Option<String>,
}

pub fn audit_log(state: &ServerState, perms: Permissions) -> Act<Vec<AuditEntryView>> {
    if !perms.can_view_audit_log() {
        return Err("you do not have permission to view the audit log".into());
    }
    let entries = state.audit_log().iter().rev().map(|entry| AuditEntryView {
        at: entry.at,
        by: entry.by.to_string(),
        by_name: commy_core::audit::member_name(state, entry.by).unwrap_or_else(|| "Someone".into()),
        kind: entry.kind,
        target: entry.target.clone(),
        label: entry.label.clone(),
        detail: entry.detail.clone(),
    });
    Ok(entries.collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::server::EVERYONE_ROLE_POSITION;
    use commy_core::Server;

    fn role(permissions: Permissions, position: u32) -> Role {
        Role {
            id: RoleId::new(),
            name: "r".into(),
            color: None,
            permissions,
            position,
            hoist: false,
            self_assignable: false,
            mentionable: false,
            tag: None,
            banner: None,
        }
    }

    fn after(roles: &[Role], updates: Vec<Role>) -> Vec<Permissions> {
        roles.iter().map(|r| updates.iter().find(|u| u.id == r.id).map_or(r.permissions, |u| u.permissions)).collect()
    }

    fn hangout() -> (ServerState, UserId, UserId) {
        let owner = UserId::new();
        let member = UserId::new();
        (ServerState::new(Server::new("Hangout", owner)), owner, member)
    }

    #[test]
    fn restricting_everyone_sets_the_no_bit_on_every_role_except_the_allowed_ones() {
        let roles = [role(Permissions::default_member(), EVERYONE_ROLE_POSITION), role(Permissions::MANAGE_MESSAGES, 1), role(Permissions::NONE, 2)];
        let bit = Permissions::NO_SEND_POLLS;
        let updates = role_access_updates(&roles, bit, true, &[roles[2].id]);
        assert_eq!(updates.len(), 2);
        let perms = after(&roles, updates);
        assert!(perms[0].contains(bit));
        assert!(perms[1].contains(bit));
        assert!(!perms[2].contains(bit));
        assert!(perms[1].allows(Permissions::MANAGE_MESSAGES));
    }

    #[test]
    fn lifting_a_restriction_clears_the_no_bit_everywhere_and_touches_nothing_else() {
        let bit = Permissions::NO_EMBED_LINKS;
        let roles = [role(Permissions::default_member() | bit, EVERYONE_ROLE_POSITION), role(Permissions::MANAGE_MESSAGES | bit, 1), role(Permissions::NONE, 2)];
        let updates = role_access_updates(&roles, bit, false, &[]);
        assert_eq!(updates.len(), 2);
        let perms = after(&roles, updates);
        assert_eq!(perms[0], Permissions::default_member());
        assert_eq!(perms[1], Permissions::MANAGE_MESSAGES);
        assert_eq!(perms[2], Permissions::NONE);
    }

    #[test]
    fn positive_permissions_keep_the_grant_model() {
        let roles = [role(Permissions::default_member(), EVERYONE_ROLE_POSITION), role(Permissions::NONE, 1)];
        let updates = role_access_updates(&roles, Permissions::ATTACH_FILES, true, &[roles[1].id]);
        let perms = after(&roles, updates);
        assert!(!perms[0].contains(Permissions::ATTACH_FILES));
        assert!(perms[1].contains(Permissions::ATTACH_FILES));
    }

    #[test]
    fn new_roles_sit_on_top_and_drop_bad_colors() {
        let (state, owner, member) = hangout();
        let form = || RoleForm { name: "  Mods ".into(), color: Some("#ZZZZZZ".into()), permissions: Some(vec!["kick_members".into()]), ..RoleForm::default() };
        assert!(create_role(&state, state.server.permissions_for(member), form()).is_err());
        let (id, event) = create_role(&state, state.server.permissions_for(owner), form()).unwrap();
        let ServerEvent::RoleCreate { role, .. } = event else { panic!() };
        assert_eq!((role.id, role.name.as_str(), role.color), (id, "Mods", None));
        assert!(role.position > state.server.roles.iter().map(|r| r.position).max().unwrap());
        assert!(create_role(&state, state.server.permissions_for(owner), RoleForm { name: " ".into(), ..RoleForm::default() }).is_err());
    }

    #[test]
    fn editing_a_role_keeps_what_the_form_leaves_out() {
        let (mut state, owner, _) = hangout();
        let perms = state.server.permissions_for(owner);
        let (id, event) = create_role(&state, perms, RoleForm { name: "Mods".into(), hoist: Some(true), permissions: Some(vec!["kick_members".into()]), ..RoleForm::default() }).unwrap();
        state.apply(event).unwrap();
        let ServerEvent::RoleUpdate { role, .. } = update_role(&state, perms, id, RoleForm { name: "Helpers".into(), color: Some("#FF4D4D".into()), ..RoleForm::default() }).unwrap() else { panic!() };
        assert_eq!((role.name.as_str(), role.color.as_deref(), role.hoist), ("Helpers", Some("ff4d4d"), true));
        assert_eq!(role.permissions, Permissions::KICK_MEMBERS);
        assert!(update_role(&state, perms, id, RoleForm { name: "x".into(), color: Some("blue".into()), ..RoleForm::default() }).is_err());
        assert!(update_role(&state, perms, RoleId::new(), RoleForm { name: "x".into(), ..RoleForm::default() }).is_err());
    }

    #[test]
    fn people_only_pick_roles_marked_self_assignable() {
        let (mut state, owner, member) = hangout();
        let perms = state.server.permissions_for(owner);
        let (open, event) = create_role(&state, perms, RoleForm { name: "Gamers".into(), self_assignable: Some(true), ..RoleForm::default() }).unwrap();
        state.apply(event).unwrap();
        let (closed, event) = create_role(&state, perms, RoleForm { name: "Mods".into(), ..RoleForm::default() }).unwrap();
        state.apply(event).unwrap();
        assert!(matches!(own_role(&state, member, open, true), Ok(ServerEvent::RoleAssign { user, .. }) if user == member));
        assert!(matches!(own_role(&state, member, open, false), Ok(ServerEvent::RoleUnassign { .. })));
        assert!(own_role(&state, member, closed, true).is_err());
        assert!(member_role(&state, state.server.permissions_for(member), member, closed, true).is_err());
        assert!(member_role(&state, perms, member, closed, true).is_ok());
    }

    #[test]
    fn the_audit_log_is_for_those_allowed_to_see_it() {
        let (state, owner, member) = hangout();
        assert!(audit_log(&state, state.server.permissions_for(member)).is_err());
        assert!(audit_log(&state, state.server.permissions_for(owner)).is_ok());
        assert!(role_access(&state, state.server.permissions_for(owner), "dungeon", true, &[]).is_err());
    }
}
