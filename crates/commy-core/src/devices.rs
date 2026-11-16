use crate::ids::{ChannelId, UserId};
use crate::user::Presence;
use std::collections::HashMap;

pub const MAX_DEVICES_PER_USER: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub node: String,
    pub voice: Option<ChannelId>,
    pub since: Option<i64>,
    pub left: Option<i64>,
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
    pub fn note(&mut self, user: UserId, node: String, voice: Option<ChannelId>, since: Option<i64>) -> bool {
        let before_now = self.of(user).iter().find(|d| d.node == node).cloned();
        let heard_late = before_now.as_ref().is_some_and(|d| repeats_a_join_it_left(d, since));
        let takes_seat = voice.is_some() && !heard_late && self.voice_holder(user).is_none_or(|held| held.node == node || since > held.since);
        let voice = voice.filter(|_| takes_seat);
        let left = before_now.as_ref().and_then(|d| if voice.is_some() { d.left } else { last_join(d) });
        let device = Device { node, since: voice.and(since), voice, left };
        let list = self.by_user.entry(user).or_default();
        let before = list.clone();
        if takes_seat {
            list.iter_mut().filter(|d| d.node != device.node).for_each(unseat);
        }
        if let Some(i) = list.iter().position(|d| d.node == device.node) {
            list.remove(i);
        } else if list.len() >= MAX_DEVICES_PER_USER {
            list.remove(0);
        }
        list.push(device);
        *list != before
    }

    pub fn join_stamp(&self, user: UserId, now_ms: i64) -> i64 {
        let latest = self.of(user).iter().filter_map(last_join).max();
        latest.map_or(now_ms, |since| now_ms.max(since.saturating_add(1)))
    }

    pub fn voice_holder(&self, user: UserId) -> Option<&Device> {
        self.of(user).iter().find(|d| d.voice.is_some())
    }

    pub fn stand_up(&mut self, user: UserId) -> bool {
        let Some(list) = self.by_user.get_mut(&user) else { return false };
        let seated = list.iter().any(|d| d.voice.is_some());
        list.iter_mut().for_each(clear_seat);
        seated
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
        self.voice_holder(user).filter(|d| d.voice == Some(channel)).map(|d| d.node.as_str())
    }
}

fn unseat(device: &mut Device) {
    device.left = last_join(device);
    clear_seat(device);
}

fn clear_seat(device: &mut Device) {
    device.voice = None;
    device.since = None;
}

fn last_join(device: &Device) -> Option<i64> {
    device.since.max(device.left)
}

fn repeats_a_join_it_left(device: &Device, since: Option<i64>) -> bool {
    device.voice.is_none() && since.zip(device.left).is_some_and(|(since, left)| since <= left)
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
        assert!(devices.note(user, node(1), None, None));
        assert!(devices.note(user, node(2), None, None));
        assert!(!devices.note(user, node(2), None, None), "the same news twice changes nothing");
        assert!(devices.note(user, node(1), Some(room), Some(5)));
        let order: Vec<&str> = devices.of(user).iter().map(|d| d.node.as_str()).collect();
        assert_eq!(order, [node(2), node(1)]);
        assert_eq!(devices.in_voice(user, room), Some(node(1).as_str()));
        assert!(devices.has(user, &node(2).to_uppercase()));
    }

    #[test]
    fn the_oldest_device_makes_room_past_the_cap() {
        let (mut devices, user) = (Devices::default(), UserId::new());
        for n in 0..=MAX_DEVICES_PER_USER as u8 {
            devices.note(user, node(n), None, None);
        }
        assert_eq!(devices.of(user).len(), MAX_DEVICES_PER_USER);
        assert!(!devices.has(user, &node(0)));
        assert!(devices.has(user, &node(MAX_DEVICES_PER_USER as u8)));
    }

    #[test]
    fn a_later_join_takes_the_seat_and_the_earlier_device_stands_up() {
        let (mut devices, user, room, other) = (Devices::default(), UserId::new(), ChannelId::new(), ChannelId::new());
        devices.note(user, node(1), Some(room), Some(1_000));
        devices.note(user, node(2), Some(other), Some(2_000));
        assert_eq!(devices.in_voice(user, other), Some(node(2).as_str()));
        assert_eq!(devices.in_voice(user, room), None, "one voice device per account");
        devices.note(user, node(2), None, None);
        assert!(devices.voice_holder(user).is_none(), "the old device is not brought back when the new one leaves");
    }

    #[test]
    fn an_older_join_heard_late_does_not_take_the_seat() {
        let (mut devices, user, room, other) = (Devices::default(), UserId::new(), ChannelId::new(), ChannelId::new());
        devices.note(user, node(2), Some(other), Some(2_000));
        assert!(devices.note(user, node(1), Some(room), Some(1_000)), "the device is still noted");
        assert_eq!(devices.voice_holder(user).map(|d| d.node.as_str()), Some(node(2).as_str()));
        assert_eq!(devices.of(user).iter().find(|d| d.node == node(1)).and_then(|d| d.voice), None);
        devices.note(user, node(2), Some(room), Some(2_000));
        assert_eq!(devices.in_voice(user, room), Some(node(2).as_str()), "the device in voice may move rooms");
    }

    #[test]
    fn a_join_heard_after_its_device_left_does_not_seat_it_again() {
        let (mut devices, user, room) = (Devices::default(), UserId::new(), ChannelId::new());
        devices.note(user, node(1), Some(room), Some(1_000));
        devices.note(user, node(1), None, None);
        assert!(!devices.note(user, node(1), Some(room), Some(1_000)), "a beat from before the leave arrived late");
        assert!(devices.voice_holder(user).is_none());
        assert!(devices.note(user, node(1), Some(room), Some(devices.join_stamp(user, 500))));
        assert_eq!(devices.in_voice(user, room), Some(node(1).as_str()), "joining again is a new join");
    }

    #[test]
    fn a_device_stood_up_by_a_later_join_is_not_seated_again_by_its_old_join() {
        let (mut devices, user, room) = (Devices::default(), UserId::new(), ChannelId::new());
        devices.note(user, node(1), Some(room), Some(1_000));
        devices.note(user, node(2), Some(room), Some(2_000));
        devices.note(user, node(2), None, None);
        devices.note(user, node(1), Some(room), Some(1_000));
        assert!(devices.voice_holder(user).is_none());
    }

    #[test]
    fn a_seat_ended_by_silence_comes_back_when_the_same_join_is_heard_again() {
        let (mut devices, user, room) = (Devices::default(), UserId::new(), ChannelId::new());
        devices.note(user, node(1), Some(room), Some(1_000));
        devices.stand_up(user);
        assert!(devices.note(user, node(1), Some(room), Some(1_000)));
        assert_eq!(devices.in_voice(user, room), Some(node(1).as_str()));
    }

    #[test]
    fn a_join_is_stamped_after_the_seat_it_takes_over_whatever_the_clock_says() {
        let (mut devices, user, room) = (Devices::default(), UserId::new(), ChannelId::new());
        assert_eq!(devices.join_stamp(user, 500), 500);
        devices.note(user, node(1), Some(room), Some(9_000));
        assert_eq!(devices.join_stamp(user, 5_000), 9_001, "this device's clock is behind the one in voice");
        assert_eq!(devices.join_stamp(user, 12_000), 12_000);
        assert!(devices.note(user, node(2), Some(room), Some(devices.join_stamp(user, 5_000))));
        assert_eq!(devices.in_voice(user, room), Some(node(2).as_str()));
    }

    #[test]
    fn a_tie_keeps_the_device_already_in_voice_as_the_devices_themselves_do() {
        let (mut devices, user, room) = (Devices::default(), UserId::new(), ChannelId::new());
        devices.note(user, node(1), Some(room), Some(7));
        devices.note(user, node(2), Some(room), Some(7));
        assert_eq!(devices.in_voice(user, room), Some(node(1).as_str()));
    }

    #[test]
    fn standing_up_clears_every_seat_of_a_member() {
        let (mut devices, user, room) = (Devices::default(), UserId::new(), ChannelId::new());
        devices.note(user, node(1), Some(room), Some(1));
        assert!(devices.stand_up(user));
        assert!(!devices.stand_up(user));
        assert!(devices.voice_holder(user).is_none());
        assert!(devices.has(user, &node(1)), "the device itself is still known");
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
