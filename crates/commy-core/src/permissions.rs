use serde::{Deserialize, Serialize};
use std::ops::{BitAnd, BitOr, BitOrAssign, Not};

/// Permission bitset, one bit per capability.
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
    /// Add and remove custom emojis and soundboard sounds.
    pub const MANAGE_EXPRESSIONS: Permissions = Permissions(1 << 19);
    pub const MANAGE_EVENTS: Permissions = Permissions(1 << 20);

    /// Sensible defaults for the implicit everyone role.
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
            | Self::CREATE_INVITES
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

    /// Administrator implies every permission.
    pub fn allows(self, other: Permissions) -> bool {
        self.contains(Self::ADMINISTRATOR) || self.contains(other)
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
        assert!(p.allows(Permissions::MANAGE_SERVER));
        assert!(!p.contains(Permissions::BAN_MEMBERS));
    }

    #[test]
    fn default_member_cannot_moderate() {
        let p = Permissions::default_member();
        assert!(p.allows(Permissions::SEND_MESSAGES));
        assert!(!p.allows(Permissions::KICK_MEMBERS));
        assert!(!p.allows(Permissions::MANAGE_SERVER));
    }
}
