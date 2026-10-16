//! Roles a peer that was offline may have missed. Gossip has no backfill,
//! so on each new neighbor members allowed to manage roles re-announce
//! every role and who holds it, and each member the self-assignable roles
//! it took. Deleted roles are remembered here, as a server keeps its
//! deleted channels, so a late create cannot bring one back, and those
//! allowed to delete them re-announce the deletes.

use commy_core::{Permissions, RoleId, ServerEvent, ServerId, ServerState, UserId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

pub const DELETED_ROLES_FILE: &str = "deleted-roles.json";

/// Roles deleted per server, kept in `<data>/deleted-roles.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeletedRoles(HashMap<ServerId, Vec<RoleId>>);

impl DeletedRoles {
    pub fn load(dir: &Path) -> Self {
        crate::persist::read_json(&dir.join(DELETED_ROLES_FILE)).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        crate::persist::write_json(&dir.join(DELETED_ROLES_FILE), self)
    }

    /// Remembers a deleted role. True when it was new.
    pub fn note(&mut self, server: ServerId, role: RoleId) -> bool {
        let list = self.0.entry(server).or_default();
        if list.contains(&role) {
            return false;
        }
        list.push(role);
        true
    }

    pub fn of(&self, server: ServerId) -> Vec<RoleId> {
        self.0.get(&server).cloned().unwrap_or_default()
    }

    /// True for a create of a role deleted here, which is ignored.
    pub fn revives(&self, server: ServerId, event: &ServerEvent) -> bool {
        matches!(event, ServerEvent::RoleCreate { role, .. } if self.0.get(&server).is_some_and(|l| l.contains(&role.id)))
    }

    /// Everything kept for a server we left. True when anything was.
    pub fn forget_server(&mut self, server: ServerId) -> bool {
        self.0.remove(&server).is_some()
    }
}

/// Whether a delete of `role` is one to remember: any role but the
/// everyone role, known here or not, since a delete may beat its create.
pub fn remembers_delete(state: &ServerState, role: RoleId) -> bool {
    !state.server.roles.iter().any(|r| r.id == role && r.position == 0)
}

/// The role events `me` re-announces in `state`'s server, see the module
/// docs. Peers ignore what they hold already.
pub fn announcements(state: &ServerState, me: UserId, deleted: &[RoleId]) -> Vec<ServerEvent> {
    let server = state.server.id;
    let manages = state.server.permissions_for(me).allows(Permissions::MANAGE_ROLES);
    let mut events = Vec::new();
    if manages {
        for &role in deleted {
            events.push(ServerEvent::RoleDelete { server, role });
        }
        // The everyone role comes with the server.
        for role in state.server.roles.iter().filter(|r| r.position > 0) {
            events.push(ServerEvent::RoleCreate { server, role: role.clone() });
        }
        // Create is ignored by peers that have the role, so these go too.
        for role in state.server.roles.iter().filter(|r| r.self_assignable || r.tag.is_some() || r.banner.is_some()) {
            events.push(ServerEvent::RoleUpdate { server, role: role.clone() });
        }
    }
    let pickable = |role: &RoleId| state.server.roles.iter().any(|r| r.id == *role && r.self_assignable);
    for member in &state.server.members {
        for role in member.roles.iter().filter(|role| manages || (member.user == me && pickable(role))) {
            events.push(ServerEvent::RoleAssign { server, user: member.user, role: *role });
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::server::Role;
    use commy_core::{Server, StoreError, UserProfile};

    fn role(name: &str, position: u32, permissions: Permissions, self_assignable: bool) -> Role {
        Role {
            id: RoleId::new(),
            name: name.into(),
            color: None,
            permissions,
            position,
            hoist: false,
            self_assignable,
            tag: None,
            banner: None,
        }
    }

    /// An owned server with two more members.
    fn server() -> (ServerState, UserId, UserId, UserId) {
        let owner = UserProfile::new("owner");
        let mut state = ServerState::new(Server::new("s", owner.id));
        state.profiles.insert(owner.id, owner.clone());
        let sid = state.server.id;
        let [admin, member] = ["admin", "member"].map(|name| {
            let profile = UserProfile::new(name);
            let id = profile.id;
            state.apply(ServerEvent::MemberJoin { server: sid, profile }).unwrap();
            id
        });
        (state, owner.id, admin, member)
    }

    fn apply_all(state: &mut ServerState, events: Vec<ServerEvent>, deleted: &DeletedRoles) -> Vec<Result<bool, StoreError>> {
        let sid = state.server.id;
        events.into_iter().filter(|e| !deleted.revives(sid, e)).map(|e| state.apply(e)).collect()
    }

    #[test]
    fn a_peer_that_missed_role_assignments_learns_them() {
        let (mut state, owner, admin, member) = server();
        let sid = state.server.id;
        // The member goes offline here.
        let mut offline = state.clone();
        let mods = role("mods", 1, Permissions::MANAGE_MESSAGES, false);
        let pick = role("pick", 2, Permissions::NONE, true);
        for role in [&mods, &pick] {
            state.apply(ServerEvent::RoleCreate { server: sid, role: role.clone() }).unwrap();
        }
        state.apply(ServerEvent::RoleAssign { server: sid, user: admin, role: mods.id }).unwrap();
        state.apply(ServerEvent::RoleAssign { server: sid, user: member, role: pick.id }).unwrap();

        let none = DeletedRoles::default();
        apply_all(&mut offline, announcements(&state, owner, &[]), &none);
        assert_eq!(offline.server.roles, state.server.roles);
        assert_eq!(offline.server.members, state.server.members);
        assert!(offline.server.permissions_for(admin).allows(Permissions::MANAGE_MESSAGES));
        // Idempotent: a second round changes nothing.
        let again = apply_all(&mut offline, announcements(&state, owner, &[]), &none);
        assert!(again.iter().all(|r| *r == Ok(false)));

        // Without Manage Roles a member announces only the roles it picked.
        assert_eq!(announcements(&state, member, &[]), vec![ServerEvent::RoleAssign { server: sid, user: member, role: pick.id }]);
        assert!(announcements(&state, admin, &[]).is_empty());
    }

    #[test]
    fn a_deleted_role_does_not_come_back() {
        let (mut state, owner, admin, member) = server();
        let sid = state.server.id;
        let admins = role("admins", 1, Permissions::MANAGE_ROLES, false);
        let gone = role("gone", 2, Permissions::MANAGE_MESSAGES, false);
        for role in [&admins, &gone] {
            state.apply(ServerEvent::RoleCreate { server: sid, role: role.clone() }).unwrap();
        }
        state.apply(ServerEvent::RoleAssign { server: sid, user: admin, role: admins.id }).unwrap();
        state.apply(ServerEvent::RoleAssign { server: sid, user: member, role: gone.id }).unwrap();
        // The admin misses the delete.
        let mut stale = state.clone();
        assert!(remembers_delete(&state, gone.id));
        assert!(!remembers_delete(&state, state.server.roles[0].id), "the everyone role stays");
        state.apply(ServerEvent::RoleDelete { server: sid, role: gone.id }).unwrap();
        let mut deleted = DeletedRoles::default();
        assert!(deleted.note(sid, gone.id) && !deleted.note(sid, gone.id));

        // What the admin announces brings nothing back here...
        apply_all(&mut state, announcements(&stale, admin, &[]), &deleted);
        assert!(!state.server.roles.iter().any(|r| r.id == gone.id));
        assert!(!state.server.permissions_for(member).allows(Permissions::MANAGE_MESSAGES));
        // ...and what the owner announces deletes it there too.
        apply_all(&mut stale, announcements(&state, owner, &deleted.of(sid)), &DeletedRoles::default());
        assert_eq!(stale.server.roles, state.server.roles);
        assert_eq!(stale.server.members, state.server.members);

        // Remembered across restarts, and forgotten with the server.
        let dir = std::env::temp_dir().join(format!("commy-roles-{:x}", std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos()));
        deleted.save(&dir).unwrap();
        let mut loaded = DeletedRoles::load(&dir);
        assert_eq!(loaded, deleted);
        assert!(loaded.forget_server(sid) && loaded.of(sid).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
