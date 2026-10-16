//! Roles and memberships a peer that was offline may have missed. Gossip
//! has no backfill, so on each new neighbor members re-announce the
//! changes of the last [`ANNOUNCE_DAYS`] days (see
//! `ServerState::changes_since`), a page of at most [`MAX_ANNOUNCED`]
//! events at a time: each round goes on where the last one stopped, and
//! starts over once all went out, so more than a page takes several new
//! neighbors. Members allowed to manage roles
//! announce role definitions, who was given or lost a role, and the
//! latest role deletes; every other member the self-assignable roles it
//! took or dropped. Members who may kick or ban announce joins (with the
//! member's profile), leaves and bans; each member announces its own join
//! to new neighbors anyway. The store keeps the latest change to each
//! (see `commy_core::RoleMark` and `commy_core::MemberMark`), so an older
//! announcement changes nothing.
//! Limit: a member offline for more than [`ANNOUNCE_DAYS`] days may miss
//! a change made before then until it is made again; it still learns of
//! each member who comes online from that member's own join.

use chrono::{DateTime, Utc};
use commy_core::{ChangeKey, Permissions, ServerEvent, ServerId, ServerState, UserId};
use std::collections::HashMap;

/// Days of changes re-announced.
pub const ANNOUNCE_DAYS: i64 = 30;

/// Most events one member sends per new neighbor, so a large server does
/// not flood the topic.
pub const MAX_ANNOUNCED: usize = 100;

/// Of those, most role deletes, the latest first. They carry no time, so
/// they are not paged.
pub const MAX_ANNOUNCED_DELETES: usize = 20;

/// Where each server's next round of announcements goes on from.
#[derive(Debug, Default)]
pub struct Cursors(HashMap<ServerId, ChangeKey>);

impl Cursors {
    /// The next round `me` announces in `state`'s server at `now`; the
    /// one after goes on past it.
    pub fn next_round(&mut self, state: &ServerState, me: UserId, now: DateTime<Utc>) -> Vec<ServerEvent> {
        let server = state.server.id;
        let (events, next) = announcements(state, me, now, self.0.get(&server).copied());
        match next {
            Some(key) => self.0.insert(server, key),
            None => self.0.remove(&server),
        };
        events
    }

    pub fn forget(&mut self, server: ServerId) {
        self.0.remove(&server);
    }
}

/// The events `me` announces in `state`'s server at `now`, going on after
/// `after`, see the module docs. Returns them with where the next round
/// goes on from: None once the last went out, so it starts over.
pub fn announcements(
    state: &ServerState,
    me: UserId,
    now: DateTime<Utc>,
    after: Option<ChangeKey>,
) -> (Vec<ServerEvent>, Option<ChangeKey>) {
    let server = state.server.id;
    let since = (now - chrono::Duration::days(ANNOUNCE_DAYS)).timestamp_millis();
    let perms = state.server.permissions_for(me);
    let manages = perms.allows(Permissions::MANAGE_ROLES);
    let removes = perms.allows(Permissions::KICK_MEMBERS) || perms.allows(Permissions::BAN_MEMBERS);
    let pickable = |role| state.server.roles.iter().any(|r| r.id == role && r.self_assignable);
    let ours = |event: &ServerEvent| match event {
        ServerEvent::RoleCreate { .. } => manages,
        ServerEvent::RoleAssign { user, role, .. } | ServerEvent::RoleUnassign { user, role, .. } => {
            manages || (*user == me && pickable(*role))
        }
        // Our own join goes out with the profile we announce.
        ServerEvent::MemberJoin { profile, .. } => removes && profile.id != me,
        ServerEvent::MemberLeave { .. } | ServerEvent::MemberBan { .. } => removes,
        _ => false,
    };
    let mut events = Vec::new();
    if manages {
        let deletes = state.server.deleted_roles.iter().rev().take(MAX_ANNOUNCED_DELETES);
        events.extend(deletes.map(|&role| ServerEvent::RoleDelete { server, role }));
    }
    let listed = |after| state.changes_since(since, after).into_iter().filter(|(_, e)| ours(e)).collect::<Vec<_>>();
    let mut changes = listed(after);
    if changes.is_empty() && after.is_some() {
        changes = listed(None);
    }
    // Deletes leave room for at least one change.
    let room = MAX_ANNOUNCED - events.len();
    let next = (changes.len() > room).then(|| changes[room - 1].0);
    events.extend(changes.into_iter().take(room).map(|(_, event)| event));
    (events, next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::server::Role;
    use commy_core::{Identity, Membership, Message, RoleId, Server, TombstoneProof, UserProfile};

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
            state.apply(ServerEvent::MemberJoin { server: sid, profile, at: 0 }).unwrap();
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

    /// The first round `me` announces in `state`.
    fn first(state: &ServerState, me: UserId) -> Vec<ServerEvent> {
        announcements(state, me, Utc::now(), None).0
    }

    /// What `from` announces as `me` first, applied to `to`.
    fn announce(from: &ServerState, me: UserId, to: &mut ServerState) -> Vec<Result<bool, commy_core::StoreError>> {
        first(from, me).into_iter().map(|e| to.apply(e)).collect()
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
        assert_eq!(definitions(&offline), definitions(&state));
        assert_eq!(standing(&offline), standing(&state));
        assert!(offline.server.permissions_for(admin).allows(Permissions::MANAGE_MESSAGES));
        // Idempotent: a second round changes nothing.
        assert!(announce(&state, owner, &mut offline).iter().all(|r| *r == Ok(false)));

        // Without Manage Roles a member announces only the roles it picked.
        let mark = *state.role_mark(member, pick.id).unwrap();
        assert_eq!(first(&state, member), vec![mark.event(sid)]);
        assert!(first(&state, admin).is_empty());
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
        assert_eq!(standing(&stale), standing(&state));
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
        assert_eq!(definitions(&offline), definitions(&state));
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
        assert_eq!(definitions(&stale), definitions(&state));
        assert_eq!(standing(&stale), standing(&state));
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
            state.apply(ServerEvent::MemberJoin { server: sid, profile, at: 0 }).unwrap();
            state.apply(ServerEvent::RoleAssign { server: sid, user, role: quiet.id, at: old }).unwrap();
            state.apply(ServerEvent::RoleAssign { server: sid, user, role: busy.id, at: now.timestamp_millis() - i }).unwrap();
        }
        for _ in 0..50 {
            let gone = role("gone", 3, Permissions::NONE, false);
            create(&mut state, &gone);
            state.apply(ServerEvent::RoleDelete { server: sid, role: gone.id }).unwrap();
        }

        let (events, next) = announcements(&state, owner, now, None);
        assert!(next.is_some(), "more to come");
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

    /// Role definitions, in no particular order: they arrive in any.
    fn definitions(state: &ServerState) -> Vec<Role> {
        let mut roles = state.server.roles.clone();
        roles.sort_by_key(|r| r.id);
        roles
    }

    /// Members with their roles, in no particular order.
    fn standing(state: &ServerState) -> Vec<(UserId, Vec<RoleId>)> {
        let mut list: Vec<(UserId, Vec<RoleId>)> = state
            .server
            .members
            .iter()
            .map(|m| {
                let mut roles = m.roles.clone();
                roles.sort();
                (m.user, roles)
            })
            .collect();
        list.sort();
        list
    }

    #[test]
    fn every_change_reaches_a_peer_over_successive_rounds() {
        let (mut state, owner, admin, _) = server();
        let sid = state.server.id;
        let mods = role("mods", 1, Permissions::MANAGE_MESSAGES, false);
        create(&mut state, &mods);
        // The peer goes offline before any of what follows.
        let mut offline = state.clone();
        let base = Utc::now().timestamp_millis() - 3_600_000;
        // The oldest change makes the admin a moderator; well over a page
        // of joins, grants and leaves follow.
        let grant = ServerEvent::RoleAssign { server: sid, user: admin, role: mods.id, at: base };
        state.apply(grant.clone()).unwrap();
        for i in 0..120 {
            let profile = UserProfile::new(format!("m{i}"));
            let user = profile.id;
            state.apply(ServerEvent::MemberJoin { server: sid, profile, at: base + 10 + i }).unwrap();
            state.apply(ServerEvent::RoleAssign { server: sid, user, role: mods.id, at: base + 1_000 + i }).unwrap();
            if i % 10 == 0 {
                state.apply(ServerEvent::MemberLeave { server: sid, user, at: base + 2_000 + i }).unwrap();
            }
        }
        assert!(!first(&state, owner).contains(&grant), "past the first page");

        let mut cursors = Cursors::default();
        let now = Utc::now();
        let mut rounds = 0;
        while standing(&offline) != standing(&state) && rounds < 10 {
            let events = cursors.next_round(&state, owner, now);
            assert!(!events.is_empty() && events.len() <= MAX_ANNOUNCED);
            for event in events {
                let _ = offline.apply(event);
            }
            rounds += 1;
        }
        assert_eq!(standing(&offline), standing(&state));
        assert!(rounds > 2, "{rounds} rounds");
        assert!(offline.server.permissions_for(admin).allows(Permissions::MANAGE_MESSAGES));
        let left: Vec<UserId> = state.member_marks().filter(|m| m.state == Membership::Left).map(|m| m.user).collect();
        assert_eq!(left.len(), 12);
        assert!(left.iter().all(|u| offline.member_mark(*u).is_some_and(|m| m.state == Membership::Left)));
        // Once all went out, it starts over.
        let again = (0..rounds + 1).flat_map(|_| cursors.next_round(&state, owner, now)).collect::<Vec<_>>();
        assert!(again.contains(&grant));
    }

    #[test]
    fn a_peer_that_missed_a_join_shows_their_messages_afterwards() {
        let (mut state, owner, _, _) = server();
        let sid = state.server.id;
        let channel = state.server.channels[0].id;
        // The peer goes offline before someone joins and talks.
        let mut offline = state.clone();
        let key = Identity::generate();
        let mut newcomer = UserProfile::new("newcomer");
        newcomer.identity = Some(key.public());
        let joined = Utc::now().timestamp_millis() - 60_000;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: newcomer.clone(), at: joined }).unwrap();
        let mut hello = Message::new(channel, newcomer.id, "hello");
        hello.sign(sid, &key);
        state.apply(ServerEvent::MessageCreate { message: hello.clone() }).unwrap();

        // Catch-up relays the message and their profile: neither is taken
        // from someone not known as a member.
        let unread = crate::catchup::Unread::default();
        let (report, held, _) = crate::catchup::apply_page(&mut offline, &unread, vec![hello.clone()], vec![]);
        assert_eq!((report.held, held.len()), (vec![hello.id], 1));
        assert!(commy_net::catchup::apply_profiles(&mut offline, vec![newcomer.clone()]).is_empty());

        // The owner announces the join; the message is taken from then on.
        announce(&state, owner, &mut offline);
        assert!(offline.server.is_member(newcomer.id));
        assert_eq!(offline.identity_of(newcomer.id), Some(key.public()));
        let (report, _, _) = crate::catchup::apply_page(&mut offline, &unread, vec![hello.clone()], vec![]);
        assert_eq!(report.applied, vec![hello.id]);
    }

    #[test]
    fn a_stale_manager_does_not_restore_a_kicked_member() {
        let (mut state, owner, admin, member) = server();
        let sid = state.server.id;
        let kickers = role("kickers", 1, Permissions::KICK_MEMBERS, false);
        create(&mut state, &kickers);
        change(&mut state, admin, kickers.id, true);
        let kicked = UserProfile::new("kicked");
        let now = Utc::now();
        let join = ServerEvent::MemberJoin { server: sid, profile: kicked.clone(), at: now.timestamp_millis() - 60_000 };
        state.apply(join.clone()).unwrap();
        // The admin misses the kick, and announces the join again.
        let mut stale = state.clone();
        let kick = ServerEvent::MemberLeave { server: sid, user: kicked.id, at: state.member_change_time(kicked.id, now) };
        state.apply(kick.clone()).unwrap();
        assert!(first(&stale, admin).contains(&join));
        announce(&stale, admin, &mut state);
        assert!(!state.server.is_member(kicked.id));
        // The owner's announcements take them out there too.
        assert!(first(&state, owner).contains(&kick));
        announce(&state, owner, &mut stale);
        assert!(!stale.server.is_member(kicked.id));

        // Who may not kick or ban announces no memberships, and nobody
        // their own join here: it goes out with the profile they announce.
        let membership = |e: &ServerEvent| {
            matches!(e, ServerEvent::MemberJoin { .. } | ServerEvent::MemberLeave { .. } | ServerEvent::MemberBan { .. })
        };
        assert!(!first(&state, member).iter().any(membership));
        let rejoin = ServerEvent::MemberJoin { server: sid, profile: state.profiles[&admin].clone(), at: now.timestamp_millis() - 1 };
        state.apply(rejoin.clone()).unwrap();
        assert!(first(&state, owner).contains(&rejoin));
        assert!(!first(&state, admin).contains(&rejoin));
    }
}
