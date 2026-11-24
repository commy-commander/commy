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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioPort {
    pub id: i32,
    pub kind: i32,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhonePick {
    pub chosen: String,
    pub output: Option<i32>,
    pub input: Option<i32>,
    pub headsets_then: Vec<String>,
}

const BUILTIN_SPEAKER: i32 = 2;
const WIRED_HEADSET: i32 = 3;
const WIRED_HEADPHONES: i32 = 4;
const BLUETOOTH_SCO: i32 = 7;
const BLUETOOTH_A2DP: i32 = 8;
const BUILTIN_MIC: i32 = 15;
const USB_HEADSET: i32 = 22;
const BLE_HEADSET: i32 = 26;

pub fn port_place(kind: i32) -> Option<SoundPlace> {
    match kind {
        BUILTIN_SPEAKER => Some(SoundPlace::Speaker),
        WIRED_HEADSET | WIRED_HEADPHONES | 11 | USB_HEADSET => Some(SoundPlace::Wired),
        BLUETOOTH_SCO | BLUETOOTH_A2DP | 23 | BLE_HEADSET | 27 => Some(SoundPlace::Bluetooth),
        _ => None,
    }
}

fn shown_name(port: &AudioPort) -> String {
    if matches!(port.kind, WIRED_HEADSET | WIRED_HEADPHONES) {
        String::new()
    } else {
        port.name.clone()
    }
}

fn speaker_port() -> SoundPort {
    SoundPort { id: SoundPlace::Speaker.word().to_string(), name: String::new(), place: SoundPlace::Speaker }
}

pub fn phone_headsets(outputs: &[AudioPort]) -> Vec<SoundPort> {
    let mut ranked: Vec<&AudioPort> = outputs
        .iter()
        .filter(|port| port_place(port.kind).is_some_and(|place| !place.built_in()))
        .collect();
    ranked.sort_by_key(|port| !matches!(port.kind, BLUETOOTH_SCO | BLE_HEADSET));
    let mut headsets = Vec::new();
    for port in ranked {
        if let Some(place) = port_place(port.kind) {
            add_headset(&mut headsets, SoundPort { id: port.id.to_string(), name: shown_name(port), place });
        }
    }
    headsets
}

pub fn phone_route(outputs: &[AudioPort], pick: Option<&PhonePick>) -> SoundRoute {
    let headsets = phone_headsets(outputs);
    let now = match pick {
        Some(pick) if pick.chosen == SoundPlace::Speaker.word() => Some(speaker_port()),
        Some(pick) => headsets.iter().find(|headset| headset.id == pick.chosen).cloned(),
        None => None,
    };
    let now = now.or_else(|| headsets.first().cloned()).unwrap_or_else(speaker_port);
    SoundRoute { now, headsets }
}

fn headset_mic(port: &AudioPort, inputs: &[AudioPort]) -> Option<i32> {
    let kinds: &[i32] = match port.kind {
        WIRED_HEADSET => &[WIRED_HEADSET],
        USB_HEADSET => &[USB_HEADSET],
        _ => &[],
    };
    inputs
        .iter()
        .find(|input| kinds.contains(&input.kind) && (port.kind == WIRED_HEADSET || input.name == port.name))
        .map(|input| input.id)
}

pub fn phone_pick(choice: &SoundChoice, outputs: &[AudioPort], inputs: &[AudioPort]) -> Option<PhonePick> {
    let headsets_then = phone_headsets(outputs).into_iter().map(|headset| headset.id).collect();
    let (chosen, output, input) = match choice {
        SoundChoice::Speaker | SoundChoice::Phone => {
            let speaker = outputs.iter().find(|port| port.kind == BUILTIN_SPEAKER)?;
            let mic = inputs.iter().find(|port| port.kind == BUILTIN_MIC).map(|port| port.id);
            (SoundPlace::Speaker.word().to_string(), Some(speaker.id), mic)
        }
        SoundChoice::Headset(id) => {
            let port = outputs.iter().find(|port| port.id.to_string() == *id)?;
            match port_place(port.kind)? {
                SoundPlace::Bluetooth => (id.clone(), None, None),
                _ => (id.clone(), Some(port.id), headset_mic(port, inputs)),
            }
        }
    };
    Some(PhonePick { chosen, output, input, headsets_then })
}

pub fn pick_holds(pick: &PhonePick, outputs: &[AudioPort]) -> bool {
    let headsets: Vec<String> = phone_headsets(outputs).into_iter().map(|headset| headset.id).collect();
    let still_there = pick.chosen == SoundPlace::Speaker.word() || headsets.contains(&pick.chosen);
    let output_there = pick.output.is_none_or(|id| outputs.iter().any(|port| port.id == id));
    still_there && output_there && !headset_arrived(&pick.headsets_then, &headsets)
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

    fn audio(id: i32, kind: i32, name: &str) -> AudioPort {
        AudioPort { id, kind, name: name.into() }
    }

    fn pixel_outputs() -> Vec<AudioPort> {
        vec![
            audio(1, 1, "Pixel 8"),
            audio(2, 2, "Pixel 8"),
            audio(30, 8, "Galaxy Buds"),
            audio(31, 7, "Galaxy Buds"),
            audio(40, 3, "Pixel 8"),
        ]
    }

    fn pixel_inputs() -> Vec<AudioPort> {
        vec![audio(5, 15, "Pixel 8"), audio(32, 7, "Galaxy Buds"), audio(41, 3, "Pixel 8")]
    }

    #[test]
    fn a_phone_lists_each_headset_once_with_the_call_link_of_a_bluetooth_one() {
        let headsets = phone_headsets(&pixel_outputs());
        assert_eq!(headsets, [port("31", "Galaxy Buds", SoundPlace::Bluetooth), port("40", "", SoundPlace::Wired)]);
        assert!(phone_headsets(&[audio(1, 1, "Pixel 8"), audio(2, 2, "Pixel 8")]).is_empty());
    }

    #[test]
    fn with_nothing_picked_a_phone_plays_through_a_headset_or_else_the_speaker() {
        assert_eq!(phone_route(&pixel_outputs(), None).now.id, "31");
        let bare = phone_route(&[audio(2, 2, "Pixel 8")], None);
        assert_eq!(bare.now, port("speaker", "", SoundPlace::Speaker));
        assert!(bare.headsets.is_empty());
    }

    #[test]
    fn picking_the_speaker_sends_sound_to_it_and_listens_on_the_phone_microphone() {
        let pick = phone_pick(&SoundChoice::Speaker, &pixel_outputs(), &pixel_inputs()).unwrap();
        assert_eq!((pick.chosen.as_str(), pick.output, pick.input), ("speaker", Some(2), Some(5)));
        assert_eq!(pick.headsets_then, ["31", "40"]);
        assert_eq!(phone_route(&pixel_outputs(), Some(&pick)).now.place, SoundPlace::Speaker);
    }

    #[test]
    fn picking_a_wired_headset_uses_its_own_microphone_and_a_bluetooth_one_follows_the_call_link() {
        let wired = phone_pick(&SoundChoice::Headset("40".into()), &pixel_outputs(), &pixel_inputs()).unwrap();
        assert_eq!((wired.output, wired.input), (Some(40), Some(41)));
        let buds = phone_pick(&SoundChoice::Headset("31".into()), &pixel_outputs(), &pixel_inputs()).unwrap();
        assert_eq!((buds.output, buds.input), (None, None));
        assert_eq!(phone_route(&pixel_outputs(), Some(&wired)).now.id, "40");
        assert!(phone_pick(&SoundChoice::Headset("99".into()), &pixel_outputs(), &pixel_inputs()).is_none());
    }

    #[test]
    fn a_pick_lapses_when_its_headset_leaves_or_a_new_headset_arrives() {
        let outputs = vec![audio(2, 2, "Pixel 8"), audio(40, 3, "Pixel 8")];
        let speaker = phone_pick(&SoundChoice::Speaker, &outputs, &[]).unwrap();
        assert!(pick_holds(&speaker, &outputs));
        assert!(pick_holds(&speaker, &[audio(2, 2, "Pixel 8")]));
        assert!(!pick_holds(&speaker, &pixel_outputs()));
        let wired = phone_pick(&SoundChoice::Headset("40".into()), &outputs, &[]).unwrap();
        assert!(!pick_holds(&wired, &[audio(2, 2, "Pixel 8")]));
    }
}
