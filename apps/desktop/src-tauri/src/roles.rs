//! Roles a peer that was offline may have missed. Gossip has no backfill,
//! so on each new neighbor members allowed to manage roles re-announce
//! the latest role deletes and the role changes of the last
//! [`ANNOUNCE_DAYS`] days: definitions, and who was given or lost a role.
//! Every other member re-announces the self-assignable roles it took or
//! dropped in that time. The store keeps the latest change to each (see
//! `commy_core::RoleMark`), so an older announcement changes nothing.
//! Limit: a member offline for more than [`ANNOUNCE_DAYS`] days may miss
//! a role change until someone changes it again.

use chrono::{DateTime, Utc};
use commy_core::{Permissions, ServerEvent, ServerState, UserId};

/// Days of role changes re-announced.
pub const ANNOUNCE_DAYS: i64 = 30;

/// Most role events one member sends per new neighbor, so a large server
/// does not flood the topic. The newest changes go first.
pub const MAX_ANNOUNCED: usize = 100;

/// Of those, most role deletes, the latest first.
pub const MAX_ANNOUNCED_DELETES: usize = 20;

/// The role events `me` re-announces in `state`'s server at `now`, see
/// the module docs.
pub fn announcements(state: &ServerState, me: UserId, now: DateTime<Utc>) -> Vec<ServerEvent> {
    let server = state.server.id;
    let since = (now - chrono::Duration::days(ANNOUNCE_DAYS)).timestamp_millis();
    let manages = state.server.permissions_for(me).allows(Permissions::MANAGE_ROLES);
    let mut events = Vec::new();
    // (is a grant or removal, time, event)
    let mut changes = Vec::new();
    if manages {
        let deletes = state.server.deleted_roles.iter().rev().take(MAX_ANNOUNCED_DELETES);
        events.extend(deletes.map(|&role| ServerEvent::RoleDelete { server, role }));
        // A create of a role a peer has is an update there.
        for role in state.server.roles.iter().filter(|r| r.updated_at >= since) {
            changes.push((false, role.updated_at, ServerEvent::RoleCreate { server, role: role.clone() }));
        }
    }
    let pickable = |role| state.server.roles.iter().any(|r| r.id == role && r.self_assignable);
    for mark in state.role_marks().filter(|m| m.at >= since && (manages || (m.user == me && pickable(m.role)))) {
        changes.push((true, mark.at, mark.event(server)));
    }
    changes.sort_by_key(|(_, at, _)| std::cmp::Reverse(*at));
    changes.truncate(MAX_ANNOUNCED - events.len());
    // Definitions first, since a grant of a role not known yet is ignored.
    changes.sort_by_key(|(mark, _, _)| *mark);
    events.extend(changes.into_iter().map(|(_, _, event)| event));
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::server::Role;
    use commy_core::{Identity, Message, RoleId, Server, TombstoneProof, UserProfile};

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
            updated_at: Utc::now().timestamp_millis(),
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

    fn create(state: &mut ServerState, role: &Role) {
        let server = state.server.id;
        state.apply(ServerEvent::RoleCreate { server, role: role.clone() }).unwrap();
    }

    fn change(state: &mut ServerState, user: UserId, role: RoleId, held: bool) {
        state.apply(state.role_change(user, role, held, Utc::now())).unwrap();
    }

    /// What `from` announces as `me`, applied to `to`.
    fn announce(from: &ServerState, me: UserId, to: &mut ServerState) -> Vec<Result<bool, commy_core::StoreError>> {
        announcements(from, me, Utc::now()).into_iter().map(|e| to.apply(e)).collect()
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
            create(&mut state, role);
        }
        change(&mut state, admin, mods.id, true);
        change(&mut state, member, pick.id, true);

        announce(&state, owner, &mut offline);
        assert_eq!(offline.server.roles, state.server.roles);
        assert_eq!(offline.server.members, state.server.members);
        assert!(offline.server.permissions_for(admin).allows(Permissions::MANAGE_MESSAGES));
        // Idempotent: a second round changes nothing.
        assert!(announce(&state, owner, &mut offline).iter().all(|r| *r == Ok(false)));

        // Without Manage Roles a member announces only the roles it picked.
        let mark = *state.role_mark(member, pick.id).unwrap();
        assert_eq!(announcements(&state, member, Utc::now()), vec![mark.event(sid)]);
        assert!(announcements(&state, admin, Utc::now()).is_empty());
    }

    #[test]
    fn a_stale_managers_announcements_do_not_restore_a_removed_role() {
        let (mut state, owner, admin, member) = server();
        let admins = role("admins", 1, Permissions::MANAGE_ROLES, false);
        let mods = role("mods", 2, Permissions::MANAGE_MESSAGES, false);
        for role in [&admins, &mods] {
            create(&mut state, role);
        }
        change(&mut state, admin, admins.id, true);
        change(&mut state, member, mods.id, true);
        // The admin misses the member losing the role.
        let mut stale = state.clone();
        change(&mut state, member, mods.id, false);

        announce(&stale, admin, &mut state);
        assert!(!state.server.permissions_for(member).allows(Permissions::MANAGE_MESSAGES));
        // A peer that missed the removal learns it from the owner.
        announce(&state, owner, &mut stale);
        assert!(!stale.server.permissions_for(member).allows(Permissions::MANAGE_MESSAGES));
        assert_eq!(stale.server.members, state.server.members);
    }

    #[test]
    fn a_peer_that_missed_a_permission_change_learns_it() {
        let (mut state, owner, _, member) = server();
        let sid = state.server.id;
        let mods = Role { updated_at: 1, ..role("mods", 1, Permissions::NONE, false) };
        create(&mut state, &mods);
        change(&mut state, member, mods.id, true);
        let mut offline = state.clone();
        let granted = Role { permissions: Permissions::MANAGE_MESSAGES, updated_at: Utc::now().timestamp_millis(), ..mods };
        state.apply(ServerEvent::RoleUpdate { server: sid, role: granted }).unwrap();
        // The everyone role changes too.
        let everyone = Role { permissions: Permissions::all(), updated_at: Utc::now().timestamp_millis(), ..state.server.roles[0].clone() };
        state.apply(ServerEvent::RoleUpdate { server: sid, role: everyone }).unwrap();

        announce(&state, owner, &mut offline);
        assert_eq!(offline.server.roles, state.server.roles);
        assert!(offline.server.permissions_for(member).allows(Permissions::MANAGE_MESSAGES));
    }

    #[test]
    fn a_deleted_role_does_not_come_back() {
        let (mut state, owner, admin, member) = server();
        let admins = role("admins", 1, Permissions::MANAGE_ROLES, false);
        let gone = role("gone", 2, Permissions::MANAGE_MESSAGES, false);
        for role in [&admins, &gone] {
            create(&mut state, role);
        }
        change(&mut state, admin, admins.id, true);
        change(&mut state, member, gone.id, true);
        // The admin misses the delete.
        let mut stale = state.clone();
        let sid = state.server.id;
        state.apply(ServerEvent::RoleDelete { server: sid, role: gone.id }).unwrap();

        // What the admin announces brings nothing back here...
        announce(&stale, admin, &mut state);
        assert!(!state.server.roles.iter().any(|r| r.id == gone.id));
        assert!(!state.server.permissions_for(member).allows(Permissions::MANAGE_MESSAGES));
        // ...and what the owner announces deletes it there too.
        announce(&state, owner, &mut stale);
        assert_eq!(stale.server.roles, state.server.roles);
        assert_eq!(stale.server.members, state.server.members);
    }

    #[test]
    fn only_recent_changes_go_out_and_never_too_many() {
        let (mut state, owner, _, _) = server();
        let sid = state.server.id;
        let now = Utc::now();
        let old = (now - chrono::Duration::days(ANNOUNCE_DAYS + 1)).timestamp_millis();
        let quiet = Role { updated_at: old, ..role("quiet", 1, Permissions::NONE, false) };
        create(&mut state, &quiet);
        let busy = role("busy", 2, Permissions::MANAGE_MESSAGES, true);
        create(&mut state, &busy);
        for i in 0..3000 {
            let profile = UserProfile::new(format!("m{i}"));
            let user = profile.id;
            state.apply(ServerEvent::MemberJoin { server: sid, profile }).unwrap();
            state.apply(ServerEvent::RoleAssign { server: sid, user, role: quiet.id, at: old }).unwrap();
            state.apply(ServerEvent::RoleAssign { server: sid, user, role: busy.id, at: now.timestamp_millis() - i }).unwrap();
        }
        for _ in 0..50 {
            let gone = role("gone", 3, Permissions::NONE, false);
            create(&mut state, &gone);
            state.apply(ServerEvent::RoleDelete { server: sid, role: gone.id }).unwrap();
        }

        let events = announcements(&state, owner, now);
        assert_eq!(events.len(), MAX_ANNOUNCED);
        let deletes = events.iter().filter(|e| matches!(e, ServerEvent::RoleDelete { .. })).count();
        assert_eq!(deletes, MAX_ANNOUNCED_DELETES);
        assert_eq!(events[deletes], ServerEvent::RoleCreate { server: sid, role: busy.clone() });
        // The newest grants, and nothing older than the window.
        let grants: Vec<i64> = events
            .iter()
            .filter_map(|e| match e {
                ServerEvent::RoleAssign { role, at, .. } if *role == busy.id => Some(*at),
                _ => None,
            })
            .collect();
        assert_eq!(grants.len(), MAX_ANNOUNCED - deletes - 1);
        assert!(grants.iter().all(|at| *at > now.timestamp_millis() - MAX_ANNOUNCED as i64));
        assert!(!events.iter().any(|e| matches!(e, ServerEvent::RoleCreate { role, .. } if role.id == quiet.id)));
    }

    #[test]
    fn a_moderators_delete_applies_once_the_missed_role_arrives() {
        let (mut state, owner, admin, member) = server();
        let sid = state.server.id;
        let channel = state.server.channels[0].id;
        let keys: Vec<(UserId, Identity)> = [owner, admin, member].map(|u| (u, Identity::generate())).into();
        for (user, key) in &keys {
            let mut profile = state.profiles[user].clone();
            profile.identity = Some(key.public());
            state.apply(ServerEvent::ProfileUpdate { profile }).unwrap();
        }
        let key = |user: UserId| &keys.iter().find(|(u, _)| *u == user).unwrap().1;
        let mut spam = Message::new(channel, member, "spam");
        spam.sign(sid, key(member));
        state.apply(ServerEvent::MessageCreate { message: spam.clone() }).unwrap();
        // The peer goes offline before the admin becomes a moderator.
        let mut offline = state.clone();
        let mods = role("mods", 1, Permissions::MANAGE_MESSAGES, false);
        create(&mut state, &mods);
        change(&mut state, admin, mods.id, true);
        let delete = TombstoneProof::sign(sid, channel, spam.id, admin, Utc::now(), key(admin)).into_event();
        assert_eq!(state.apply(delete.clone()), Ok(true));
        assert!(!state.has_message(spam.id));

        // The delete reaches the peer first: kept, not applied.
        assert_eq!(offline.apply(delete), Ok(true));
        assert!(offline.has_message(spam.id));
        announce(&state, owner, &mut offline);
        assert!(!offline.has_message(spam.id));
    }
}
