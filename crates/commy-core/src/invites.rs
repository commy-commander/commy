use crate::ids::UserId;
use serde::{Deserialize, Serialize};

pub const MAX_TRACKED_INVITES: usize = 500;
pub const MAX_INVITE_CODE_CHARS: usize = 32;
pub const MAX_INVITE_ORIGIN_CHARS: usize = 200;
pub const REVOKE_KEY_CHARS: usize = 43;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackedInvite {
    pub origin: String,
    pub code: String,
    pub creator: UserId,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub revoke_key: String,
}

pub fn invite_id(origin: &str, code: &str) -> String {
    format!("{origin}/i/{code}")
}

fn plain_text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && value.chars().all(|c| c.is_ascii_graphic())
}

impl TrackedInvite {
    pub fn id(&self) -> String {
        invite_id(&self.origin, &self.code)
    }

    pub fn valid(&self) -> bool {
        plain_text(&self.origin, MAX_INVITE_ORIGIN_CHARS)
            && plain_text(&self.code, MAX_INVITE_CODE_CHARS)
            && self.revoke_key.len() == REVOKE_KEY_CHARS
            && self.revoke_key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            && self.expires_at.is_none_or(|t| t > self.created_at)
    }

    pub fn live(&self, now_ms: i64) -> bool {
        self.expires_at.is_none_or(|t| t > now_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invite(code: &str, expires_at: Option<i64>) -> TrackedInvite {
        TrackedInvite {
            origin: "https://commy.chat".into(),
            code: code.into(),
            creator: UserId::new(),
            created_at: 1_000,
            expires_at,
            revoke_key: "k".repeat(REVOKE_KEY_CHARS),
        }
    }

    #[test]
    fn invites_are_named_by_origin_and_code() {
        assert_eq!(invite("AB23CD45", None).id(), "https://commy.chat/i/AB23CD45");
    }

    #[test]
    fn only_tidy_invites_are_kept() {
        assert!(invite("AB23CD45", Some(2_000)).valid());
        assert!(invite("pixel-pioneers", None).valid());
        assert!(!invite("", None).valid());
        assert!(!invite(&"a".repeat(MAX_INVITE_CODE_CHARS + 1), None).valid());
        assert!(!invite("has space", None).valid());
        assert!(!invite("AB23CD45", Some(500)).valid(), "expires before it was made");
        let mut short_key = invite("AB23CD45", None);
        short_key.revoke_key = "short".into();
        assert!(!short_key.valid());
        let mut odd_key = invite("AB23CD45", None);
        odd_key.revoke_key = "+".repeat(REVOKE_KEY_CHARS);
        assert!(!odd_key.valid());
    }

    #[test]
    fn an_invite_without_an_expiry_stays_live() {
        assert!(invite("A", None).live(i64::MAX));
        assert!(invite("A", Some(2_000)).live(1_999));
        assert!(!invite("A", Some(2_000)).live(2_000));
    }

    mod in_a_server {
        use super::*;
        use crate::action::SignedEvent;
        use crate::event::ServerEvent;
        use crate::permissions::Permissions;
        use crate::server::Role;
        use crate::store::{ServerState, StoreError};
        use crate::test_support::{fixture, identity_for, join};

        fn now() -> i64 {
            chrono::Utc::now().timestamp_millis()
        }

        fn made_by(creator: UserId, code: &str, expires_at: Option<i64>) -> TrackedInvite {
            TrackedInvite { creator, created_at: now() - 1_000, expires_at, ..invite(code, None) }
        }

        fn grant_invites(state: &mut ServerState, user: UserId) {
            let sid = state.server.id;
            let role = Role {
                id: crate::ids::RoleId::new(),
                name: "inviters".into(),
                color: None,
                permissions: Permissions::CREATE_INVITES,
                position: 1,
                hoist: false,
                self_assignable: false,
                tag: None,
                banner: None,
            };
            let rid = role.id;
            state.apply(ServerEvent::RoleCreate { server: sid, role }).unwrap();
            state.apply(ServerEvent::RoleAssign { server: sid, user, role: rid }).unwrap();
        }

        #[test]
        fn inviters_track_their_own_links_and_managers_or_the_maker_revoke_them() {
            let (mut state, _, owner) = fixture();
            let sid = state.server.id;
            let maker = join(&mut state, "maker");
            let plain = join(&mut state, "plain");
            let link = made_by(maker, "AB23CD45", Some(now() + 3_600_000));
            let t0 = now();
            let track = |by| SignedEvent::sign(sid, by, t0, ServerEvent::InviteTrack { server: sid, invite: link.clone() }, &identity_for(by));
            assert_eq!(state.apply_signed(&track(maker)), Err(StoreError::NotAllowed), "needs create invites");
            grant_invites(&mut state, maker);
            assert_eq!(state.apply_signed(&track(owner)), Err(StoreError::NotAllowed), "a link names its maker");
            assert_eq!(state.apply_signed(&track(maker)), Ok(true));
            assert_eq!(state.tracked_invites(now()).len(), 1);
            assert!(state.holds_signed_action(&format!("invite:{}", link.id())), "relayed to members who missed it");

            let revoke = |by, at| {
                let event = ServerEvent::InviteRevoke { server: sid, origin: link.origin.clone(), code: link.code.clone(), creator: maker };
                SignedEvent::sign(sid, by, at, event, &identity_for(by))
            };
            assert_eq!(state.apply_signed(&revoke(plain, t0 + 1)), Err(StoreError::NotAllowed));
            let gone = revoke(owner, t0 + 1);
            assert_eq!(state.apply_signed(&gone), Ok(true));
            assert!(state.tracked_invites(now()).is_empty());
            assert_eq!(state.apply_signed(&track(maker)), Ok(false), "an older copy relayed again stays revoked");

            let mut other = ServerState::from_snapshot(state.snapshot());
            assert!(other.tracked_invites(now()).is_empty());
            let second = made_by(maker, "ZZ23CD45", None);
            let mine = SignedEvent::sign(sid, maker, t0 + 2, ServerEvent::InviteTrack { server: sid, invite: second.clone() }, &identity_for(maker));
            assert_eq!(other.apply_signed(&mine), Ok(true));
            let by_maker = ServerEvent::InviteRevoke { server: sid, origin: second.origin.clone(), code: second.code.clone(), creator: maker };
            let by_maker = SignedEvent::sign(sid, maker, t0 + 3, by_maker, &identity_for(maker));
            assert_eq!(other.apply_signed(&by_maker), Ok(true), "the maker can revoke their own");
            let log: Vec<crate::audit::AuditKind> = other.audit_log().iter().map(|e| e.kind).collect();
            assert!(log.ends_with(&[crate::audit::AuditKind::InviteCreate, crate::audit::AuditKind::InviteRevoke]));
        }

        #[test]
        fn expired_links_are_refused_and_pruned_and_kept_across_restarts() {
            let (mut state, _, owner) = fixture();
            let sid = state.server.id;
            let track = |link: &TrackedInvite| ServerEvent::InviteTrack { server: sid, invite: link.clone() };
            let stale = made_by(owner, "OLD23CD4", Some(now() - 10));
            assert_eq!(state.apply(track(&stale)), Ok(false));
            let soon = made_by(owner, "SOON3CD4", Some(now() + 60_000));
            let forever = made_by(owner, "pixel-pioneers", None);
            assert_eq!(state.apply(track(&soon)), Ok(true));
            assert_eq!(state.apply(track(&forever)), Ok(true));
            let restored = ServerState::from_snapshot(state.snapshot());
            assert_eq!(restored.tracked_invites(now()).len(), 2);
            assert_eq!(restored.tracked_invite(&forever.origin, &forever.code), Some(&forever));
            state.prune_expired(chrono::Utc::now() + chrono::Duration::minutes(5));
            assert_eq!(state.tracked_invites(now() + 300_000), vec![&forever]);
            let mut broken = made_by(owner, "BAD", None);
            broken.revoke_key = "x".into();
            assert_eq!(state.apply(track(&broken)), Ok(false));
        }

        #[test]
        fn the_invite_background_is_a_managers_image() {
            let (mut state, _, owner) = fixture();
            let sid = state.server.id;
            let user = join(&mut state, "user");
            let image = format!("{}@{}", "b".repeat(52), "0".repeat(64));
            let set = ServerEvent::InviteBackgroundSet { server: sid, background: Some(image.clone()) };
            assert!(!crate::action::authorized(&state.server, user, &set));
            assert!(crate::action::authorized(&state.server, owner, &set));
            assert_eq!(state.apply(set), Ok(true));
            assert_eq!(state.invite_background(), Some(image.as_str()));
            assert_eq!(state.apply(ServerEvent::InviteBackgroundSet { server: sid, background: Some("not an image".into()) }), Ok(false));
            assert!(crate::action::snapshot_omits("invite_background"));
            assert_eq!(ServerState::from_snapshot(state.snapshot()).invite_background(), Some(image.as_str()));
        }
    }
}
