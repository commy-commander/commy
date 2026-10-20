use serde::{Deserialize, Serialize};
use std::ops::{BitAnd, BitOr, BitOrAssign, Not};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct Permissions(pub u64);

impl Permissions {
    pub const NONE: Permissions = Permissions(0);
    pub const VIEW_CHANNELS: Permissions = Permissions(1 << 0);
    pub const SEND_MESSAGES: Permissions = Permissions(1 << 1);
    pub const MANAGE_MESSAGES: Permissions = Permissions(1 << 2);
    pub const MANAGE_CHANNELS: Permissions = Permissions(1 << 3);
    pub const MANAGE_SERVER: Permissions = Permissions(1 << 4);
    pub const KICK_MEMBERS: Permissions = Permissions(1 << 5);
    pub const BAN_MEMBERS: Permissions = Permissions(1 << 6);
    pub const CONNECT_VOICE: Permissions = Permissions(1 << 7);
    pub const SPEAK: Permissions = Permissions(1 << 8);
    pub const MUTE_MEMBERS: Permissions = Permissions(1 << 9);
    pub const DEAFEN_MEMBERS: Permissions = Permissions(1 << 10);
    pub const STREAM: Permissions = Permissions(1 << 11);
    pub const ATTACH_FILES: Permissions = Permissions(1 << 12);
    pub const ADD_REACTIONS: Permissions = Permissions(1 << 13);
    pub const USE_SOUNDBOARD: Permissions = Permissions(1 << 14);
    pub const USE_EXTERNAL_EMOJIS: Permissions = Permissions(1 << 15);
    pub const CREATE_INVITES: Permissions = Permissions(1 << 16);
    pub const MANAGE_ROLES: Permissions = Permissions(1 << 17);
    pub const ADMINISTRATOR: Permissions = Permissions(1 << 18);
    pub const MANAGE_EXPRESSIONS: Permissions = Permissions(1 << 19);
    pub const MANAGE_EVENTS: Permissions = Permissions(1 << 20);
    pub const PIN_MESSAGES: Permissions = Permissions(1 << 21);
    pub const POST_ANNOUNCEMENTS: Permissions = Permissions(1 << 22);
    pub const VIEW_AUDIT_LOG: Permissions = Permissions(1 << 23);
    pub const ADD_EMOJIS: Permissions = Permissions(1 << 24);
    pub const ADD_STICKERS: Permissions = Permissions(1 << 25);
    pub const ADD_EMOTICONS: Permissions = Permissions(1 << 26);
    pub const MENTION_EVERYONE: Permissions = Permissions(1 << 27);
    pub const TIME_OUT_MEMBERS: Permissions = Permissions(1 << 28);
    pub const BYPASS_SLOW_MODE: Permissions = Permissions(1 << 29);
    pub const CHANGE_NICKNAME: Permissions = Permissions(1 << 31);
    pub const MANAGE_NICKNAMES: Permissions = Permissions(1 << 32);

    pub fn default_member() -> Permissions {
        Self::VIEW_CHANNELS
            | Self::SEND_MESSAGES
            | Self::CONNECT_VOICE
            | Self::SPEAK
            | Self::STREAM
            | Self::ATTACH_FILES
            | Self::ADD_REACTIONS
            | Self::USE_SOUNDBOARD
            | Self::USE_EXTERNAL_EMOJIS
            | Self::ADD_EMOJIS
            | Self::ADD_STICKERS
            | Self::ADD_EMOTICONS
            | Self::CHANGE_NICKNAME
    }

    pub fn all() -> Permissions {
        Permissions(u64::MAX)
    }

    pub fn contains(self, other: Permissions) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn insert(&mut self, other: Permissions) {
        self.0 |= other.0;
    }

    pub fn remove(&mut self, other: Permissions) {
        self.0 &= !other.0;
    }

    pub fn allows(self, other: Permissions) -> bool {
        self.contains(Self::ADMINISTRATOR) || self.contains(other)
    }

    pub fn bypasses_slow_mode(self) -> bool {
        self.allows(Self::BYPASS_SLOW_MODE) || self.allows(Self::MANAGE_MESSAGES) || self.allows(Self::MANAGE_CHANNELS)
    }

    pub fn can_pin(self) -> bool {
        self.allows(Self::PIN_MESSAGES) || self.allows(Self::MANAGE_MESSAGES)
    }

    pub fn can_announce(self) -> bool {
        self.allows(Self::POST_ANNOUNCEMENTS) || self.allows(Self::MANAGE_CHANNELS)
    }

    pub fn can_add(self, kind: Permissions) -> bool {
        self.allows(kind) || self.allows(Self::MANAGE_EXPRESSIONS)
    }

    pub fn can_view_audit_log(self) -> bool {
        self.allows(Self::VIEW_AUDIT_LOG) || self.allows(Self::MANAGE_SERVER)
    }

    pub fn can_mention_everyone(self) -> bool {
        self.allows(Self::MENTION_EVERYONE)
    }

    pub fn can_change_own_nickname(self) -> bool {
        self.allows(Self::CHANGE_NICKNAME) || self.allows(Self::MANAGE_NICKNAMES)
    }
}

impl BitOr for Permissions {
    type Output = Permissions;
    fn bitor(self, rhs: Permissions) -> Permissions {
        Permissions(self.0 | rhs.0)
    }
}

impl BitOrAssign for Permissions {
    fn bitor_assign(&mut self, rhs: Permissions) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for Permissions {
    type Output = Permissions;
    fn bitand(self, rhs: Permissions) -> Permissions {
        Permissions(self.0 & rhs.0)
    }
}

impl Not for Permissions {
    type Output = Permissions;
    fn not(self) -> Permissions {
        Permissions(!self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_and_insert() {
        let mut p = Permissions::NONE;
        assert!(!p.contains(Permissions::SEND_MESSAGES));
        p.insert(Permissions::SEND_MESSAGES | Permissions::SPEAK);
        assert!(p.contains(Permissions::SEND_MESSAGES));
        assert!(p.contains(Permissions::SPEAK));
        p.remove(Permissions::SPEAK);
        assert!(!p.contains(Permissions::SPEAK));
    }

    #[test]
    fn administrator_allows_everything() {
        let p = Permissions::ADMINISTRATOR;
        assert!(p.allows(Permissions::BAN_MEMBERS));
        assert!(p.allows(Permissions::TIME_OUT_MEMBERS));
        assert!(p.allows(Permissions::MANAGE_SERVER));
        assert!(!p.contains(Permissions::BAN_MEMBERS));
    }

    #[test]
    fn default_member_cannot_moderate() {
        let p = Permissions::default_member();
        assert!(p.allows(Permissions::SEND_MESSAGES));
        assert!(!p.allows(Permissions::KICK_MEMBERS));
        assert!(!p.allows(Permissions::TIME_OUT_MEMBERS));
        assert!(!p.allows(Permissions::MANAGE_SERVER));
        assert!(!p.allows(Permissions::CREATE_INVITES));
        assert!(!p.can_pin());
        assert!(!p.can_announce());
        assert!(!p.can_view_audit_log());
        assert!(!p.can_mention_everyone());
        assert!(p.can_change_own_nickname());
        assert!(!p.allows(Permissions::MANAGE_NICKNAMES));
    }

    #[test]
    fn managing_nicknames_covers_your_own_too() {
        assert!(Permissions::CHANGE_NICKNAME.can_change_own_nickname());
        assert!(Permissions::MANAGE_NICKNAMES.can_change_own_nickname());
        assert!(Permissions::ADMINISTRATOR.can_change_own_nickname());
        assert!(!Permissions::SEND_MESSAGES.can_change_own_nickname());
        assert_ne!(Permissions::CHANGE_NICKNAME, Permissions::MANAGE_NICKNAMES);
    }

    #[test]
    fn mentioning_everyone_is_its_own_permission() {
        assert!(Permissions::MENTION_EVERYONE.can_mention_everyone());
        assert!(Permissions::ADMINISTRATOR.can_mention_everyone());
        assert!(!Permissions::MANAGE_MESSAGES.can_mention_everyone());
        assert!(!Permissions::MANAGE_SERVER.can_mention_everyone());
    }

    #[test]
    fn moderation_permissions_cover_pins_and_announcements() {
        assert!(Permissions::PIN_MESSAGES.can_pin());
        assert!(Permissions::MANAGE_MESSAGES.can_pin());
        assert!(!Permissions::MANAGE_CHANNELS.can_pin());
        assert!(Permissions::POST_ANNOUNCEMENTS.can_announce());
        assert!(Permissions::MANAGE_CHANNELS.can_announce());
        assert!(!Permissions::MANAGE_MESSAGES.can_announce());
        assert!(Permissions::ADMINISTRATOR.can_pin() && Permissions::ADMINISTRATOR.can_announce());
        assert!(Permissions::VIEW_AUDIT_LOG.can_view_audit_log());
        assert!(Permissions::MANAGE_SERVER.can_view_audit_log());
        assert!(Permissions::ADMINISTRATOR.can_view_audit_log());
        assert!(!Permissions::MANAGE_ROLES.can_view_audit_log());
    }

    #[test]
    fn members_add_expressions_by_default_and_managers_always_can() {
        let member = Permissions::default_member();
        for kind in [Permissions::ADD_EMOJIS, Permissions::ADD_STICKERS, Permissions::ADD_EMOTICONS] {
            assert!(member.can_add(kind));
            assert!(!Permissions::SEND_MESSAGES.can_add(kind));
            assert!(Permissions::MANAGE_EXPRESSIONS.can_add(kind));
        }
    }
}
