use crate::ids::{ChannelId, UserId};
use crate::user::Presence;
use std::collections::HashMap;

pub const MAX_DEVICES_PER_USER: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub node: String,
    pub voice: Option<ChannelId>,
}

#[derive(Debug, Clone, Default)]
pub struct Devices {
    by_user: HashMap<UserId, Vec<Device>>,
}

pub fn node_key(node: &str) -> Option<String> {
    let valid = node.len() == 64 && node.bytes().all(|b| b.is_ascii_hexdigit());
    valid.then(|| node.to_ascii_lowercase())
}

impl Devices {
    pub fn note(&mut self, user: UserId, node: String, voice: Option<ChannelId>) -> bool {
        let list = self.by_user.entry(user).or_default();
        let device = Device { node, voice };
        let at = list.iter().position(|d| d.node == device.node);
        if at.is_some_and(|i| i + 1 == list.len() && list[i] == device) {
            return false;
        }
        if let Some(i) = at {
            list.remove(i);
        } else if list.len() >= MAX_DEVICES_PER_USER {
            list.remove(0);
        }
        list.push(device);
        true
    }

    pub fn forget(&mut self, user: UserId) {
        self.by_user.remove(&user);
    }

    pub fn users(&self) -> usize {
        self.by_user.len()
    }

    pub fn of(&self, user: UserId) -> &[Device] {
        self.by_user.get(&user).map_or(&[], Vec::as_slice)
    }

    pub fn has(&self, user: UserId, node: &str) -> bool {
        self.of(user).iter().any(|d| d.node.eq_ignore_ascii_case(node))
    }

    pub fn in_voice(&self, user: UserId, channel: ChannelId) -> Option<&str> {
        self.of(user).iter().rev().find(|d| d.voice == Some(channel)).map(|d| d.node.as_str())
    }
}

fn liveliness(presence: Presence) -> u8 {
    match presence.as_seen_by_others() {
        Presence::DoNotDisturb => 3,
        Presence::Online => 2,
        Presence::Idle => 1,
        Presence::Invisible | Presence::Offline => 0,
    }
}

pub fn combined_presence(devices: impl IntoIterator<Item = Presence>) -> Presence {
    devices.into_iter().map(Presence::as_seen_by_others).max_by_key(|p| liveliness(*p)).unwrap_or(Presence::Offline)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(n: u8) -> String {
        format!("{n:02x}").repeat(32)
    }

    #[test]
    fn a_device_is_noted_once_and_moves_to_the_back_when_it_changes() {
        let (mut devices, user, room) = (Devices::default(), UserId::new(), ChannelId::new());
        assert!(devices.note(user, node(1), None));
        assert!(devices.note(user, node(2), None));
        assert!(!devices.note(user, node(2), None), "the same news twice changes nothing");
        assert!(devices.note(user, node(1), Some(room)));
        let order: Vec<&str> = devices.of(user).iter().map(|d| d.node.as_str()).collect();
        assert_eq!(order, [node(2), node(1)]);
        assert_eq!(devices.in_voice(user, room), Some(node(1).as_str()));
        assert!(devices.has(user, &node(2).to_uppercase()));
    }

    #[test]
    fn the_oldest_device_makes_room_past_the_cap() {
        let (mut devices, user) = (Devices::default(), UserId::new());
        for n in 0..=MAX_DEVICES_PER_USER as u8 {
            devices.note(user, node(n), None);
        }
        assert_eq!(devices.of(user).len(), MAX_DEVICES_PER_USER);
        assert!(!devices.has(user, &node(0)));
        assert!(devices.has(user, &node(MAX_DEVICES_PER_USER as u8)));
    }

    #[test]
    fn the_latest_device_in_a_room_is_the_one_heard_there() {
        let (mut devices, user, room) = (Devices::default(), UserId::new(), ChannelId::new());
        devices.note(user, node(1), Some(room));
        devices.note(user, node(2), Some(room));
        assert_eq!(devices.in_voice(user, room), Some(node(2).as_str()));
        devices.note(user, node(2), None);
        assert_eq!(devices.in_voice(user, room), Some(node(1).as_str()));
        assert_eq!(devices.in_voice(user, ChannelId::new()), None);
    }

    #[test]
    fn only_full_length_hex_is_a_node() {
        assert_eq!(node_key(&node(171).to_uppercase()), Some(node(171)));
        assert_eq!(node_key("ab"), None);
        assert_eq!(node_key(&"zz".repeat(32)), None);
    }

    #[test]
    fn the_most_present_device_speaks_for_the_person() {
        use Presence::*;
        assert_eq!(combined_presence([Offline, Online]), Online);
        assert_eq!(combined_presence([Idle, Online]), Online);
        assert_eq!(combined_presence([Online, DoNotDisturb]), DoNotDisturb);
        assert_eq!(combined_presence([Invisible, Idle]), Idle);
        assert_eq!(combined_presence([Invisible]), Offline);
        assert_eq!(combined_presence([]), Offline);
    }
}
