#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundPlace {
    Speaker,
    Phone,
    Bluetooth,
    Wired,
    Car,
    Other,
}

impl SoundPlace {
    pub fn word(self) -> &'static str {
        match self {
            Self::Speaker => "speaker",
            Self::Phone => "phone",
            Self::Bluetooth => "bluetooth",
            Self::Wired => "wired",
            Self::Car => "car",
            Self::Other => "other",
        }
    }

    pub fn built_in(self) -> bool {
        matches!(self, Self::Speaker | Self::Phone)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundPort {
    pub id: String,
    pub name: String,
    pub place: SoundPlace,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundRoute {
    pub now: SoundPort,
    pub headsets: Vec<SoundPort>,
}

impl SoundRoute {
    pub fn plays_through(&self, port: &SoundPort) -> bool {
        !self.now.place.built_in() && (port.id == self.now.id || port.name == self.now.name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SoundChoice {
    Speaker,
    Phone,
    Headset(String),
}

impl SoundChoice {
    pub fn from_id(id: &str) -> Self {
        match id {
            "speaker" => Self::Speaker,
            "phone" => Self::Phone,
            other => Self::Headset(other.to_string()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Steer {
    Leave,
    ToSpeaker,
    ToPhone,
    FollowNewHeadset,
}

pub fn steer(now: SoundPlace, speaker_wanted: bool, new_headset: bool) -> Steer {
    if new_headset {
        Steer::FollowNewHeadset
    } else if now == SoundPlace::Phone && speaker_wanted {
        Steer::ToSpeaker
    } else if now == SoundPlace::Speaker && !speaker_wanted {
        Steer::ToPhone
    } else {
        Steer::Leave
    }
}

pub fn add_headset(headsets: &mut Vec<SoundPort>, port: SoundPort) {
    let known = headsets.iter().any(|seen| seen.id == port.id || seen.name == port.name);
    if !port.place.built_in() && !known {
        headsets.push(port);
    }
}

pub fn headset_arrived(before: &[String], now: &[String]) -> bool {
    now.iter().any(|id| !before.contains(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(id: &str, name: &str, place: SoundPlace) -> SoundPort {
        SoundPort { id: id.into(), name: name.into(), place }
    }

    #[test]
    fn the_earpiece_turns_back_to_the_speaker_when_the_speaker_is_wanted() {
        assert_eq!(steer(SoundPlace::Phone, true, false), Steer::ToSpeaker);
        assert_eq!(steer(SoundPlace::Phone, false, false), Steer::Leave);
    }

    #[test]
    fn the_speaker_turns_to_the_earpiece_only_when_the_phone_is_wanted() {
        assert_eq!(steer(SoundPlace::Speaker, false, false), Steer::ToPhone);
        assert_eq!(steer(SoundPlace::Speaker, true, false), Steer::Leave);
    }

    #[test]
    fn a_headset_keeps_the_sound_whatever_was_picked_before() {
        for wanted in [true, false] {
            assert_eq!(steer(SoundPlace::Bluetooth, wanted, false), Steer::Leave);
            assert_eq!(steer(SoundPlace::Wired, wanted, false), Steer::Leave);
        }
    }

    #[test]
    fn a_newly_connected_headset_takes_the_sound() {
        assert_eq!(steer(SoundPlace::Phone, false, true), Steer::FollowNewHeadset);
        assert_eq!(steer(SoundPlace::Speaker, true, true), Steer::FollowNewHeadset);
    }

    #[test]
    fn only_a_headset_not_seen_before_counts_as_arriving() {
        let before = vec!["a".to_string()];
        assert!(!headset_arrived(&before, &["a".to_string()]));
        assert!(!headset_arrived(&before, &[]));
        assert!(headset_arrived(&before, &["a".to_string(), "b".to_string()]));
    }

    #[test]
    fn headsets_are_listed_once_and_built_in_ports_are_left_out() {
        let mut headsets = Vec::new();
        add_headset(&mut headsets, port("air-tsco", "AirPods Pro", SoundPlace::Bluetooth));
        add_headset(&mut headsets, port("air-tacl", "AirPods Pro", SoundPlace::Bluetooth));
        add_headset(&mut headsets, port("Speaker", "Speaker", SoundPlace::Speaker));
        add_headset(&mut headsets, port("Receiver", "Receiver", SoundPlace::Phone));
        assert_eq!(headsets, vec![port("air-tsco", "AirPods Pro", SoundPlace::Bluetooth)]);
    }

    #[test]
    fn the_route_knows_which_headset_is_playing() {
        let pods = port("air-tsco", "AirPods Pro", SoundPlace::Bluetooth);
        let playing = SoundRoute { now: port("air-tacl", "AirPods Pro", SoundPlace::Bluetooth), headsets: vec![pods.clone()] };
        assert!(playing.plays_through(&pods));
        let on_speaker = SoundRoute { now: port("Speaker", "Speaker", SoundPlace::Speaker), headsets: vec![pods.clone()] };
        assert!(!on_speaker.plays_through(&pods));
    }

    #[test]
    fn choices_come_back_from_their_ids() {
        assert_eq!(SoundChoice::from_id("speaker"), SoundChoice::Speaker);
        assert_eq!(SoundChoice::from_id("phone"), SoundChoice::Phone);
        assert_eq!(SoundChoice::from_id("air-tsco"), SoundChoice::Headset("air-tsco".into()));
    }
}
