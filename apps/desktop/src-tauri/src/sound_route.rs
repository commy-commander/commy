use commy_media::sound_route::SoundChoice;
#[cfg(any(target_os = "ios", target_os = "android", test))]
use commy_media::sound_route::{SoundPlace, SoundRoute};
use serde::{Deserialize, Serialize};

use crate::state::{App, VoiceSettings};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PhoneSound {
    #[default]
    Speaker,
    Phone,
}

impl PhoneSound {
    fn kept_for(choice: &SoundChoice) -> Option<Self> {
        match choice {
            SoundChoice::Speaker => Some(Self::Speaker),
            SoundChoice::Phone => Some(Self::Phone),
            SoundChoice::Headset(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SoundChoiceView {
    id: String,
    name: String,
    place: &'static str,
    on: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SoundRouteView {
    now: &'static str,
    name: String,
    choices: Vec<SoundChoiceView>,
}

#[cfg(any(target_os = "ios", target_os = "android", test))]
fn built_in_choice(route: &SoundRoute, place: SoundPlace) -> SoundChoiceView {
    SoundChoiceView {
        id: place.word().to_string(),
        name: String::new(),
        place: place.word(),
        on: route.now.place == place,
    }
}

#[cfg(any(target_os = "ios", target_os = "android", test))]
pub fn route_view(route: &SoundRoute, built_in: &[SoundPlace]) -> SoundRouteView {
    let headsets = route.headsets.iter().map(|headset| SoundChoiceView {
        id: headset.id.clone(),
        name: headset.name.clone(),
        place: headset.place.word(),
        on: route.plays_through(headset),
    });
    let built_in = built_in.iter().map(|place| built_in_choice(route, *place));
    SoundRouteView {
        now: route.now.place.word(),
        name: route.now.name.clone(),
        choices: headsets.chain(built_in).collect(),
    }
}

#[cfg(target_os = "ios")]
fn current() -> Option<SoundRouteView> {
    Some(route_view(&commy_media::sound_route(), &[SoundPlace::Speaker, SoundPlace::Phone]))
}

#[cfg(target_os = "android")]
fn current() -> Option<SoundRouteView> {
    Some(route_view(&commy_media::sound_route(), &[SoundPlace::Speaker]))
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn current() -> Option<SoundRouteView> {
    None
}

#[cfg(any(target_os = "ios", target_os = "android"))]
fn steer(choice: &SoundChoice) -> anyhow::Result<()> {
    commy_media::choose_sound(choice)
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn steer(_choice: &SoundChoice) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(target_os = "ios")]
pub fn prepare(handle: &tauri::AppHandle, voice: &VoiceSettings) {
    use tauri::Emitter;
    commy_media::want_speaker(voice.phone_sound == PhoneSound::Speaker);
    let emitter = handle.clone();
    commy_media::on_sound_route_change(move || {
        let _ = emitter.emit("sound-route", current());
    });
}

#[cfg(target_os = "android")]
pub fn prepare(handle: &tauri::AppHandle, _voice: &VoiceSettings) {
    use tauri::Emitter;
    let emitter = handle.clone();
    commy_media::on_sound_route_change(move || {
        let _ = emitter.emit("sound-route", current());
    });
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub fn prepare(_handle: &tauri::AppHandle, _voice: &VoiceSettings) {}

#[tauri::command]
pub fn sound_route() -> Option<SoundRouteView> {
    current()
}

#[tauri::command]
pub fn choose_sound(app: tauri::State<'_, App>, choice: String) -> Result<Option<SoundRouteView>, String> {
    let choice = SoundChoice::from_id(&choice);
    if let Some(kept) = PhoneSound::kept_for(&choice) {
        app.update_voice(|voice| voice.phone_sound = kept);
    }
    steer(&choice).map_err(|e| e.to_string())?;
    Ok(current())
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_media::sound_route::SoundPort;

    fn port(id: &str, name: &str, place: SoundPlace) -> SoundPort {
        SoundPort { id: id.into(), name: name.into(), place }
    }

    fn picked(view: &SoundRouteView) -> Vec<&str> {
        view.choices.iter().filter(|choice| choice.on).map(|choice| choice.id.as_str()).collect()
    }

    #[test]
    fn the_speaker_and_the_phone_are_always_offered_after_any_headsets() {
        let route = SoundRoute {
            now: port("air-tacl", "AirPods Pro", SoundPlace::Bluetooth),
            headsets: vec![port("air-tsco", "AirPods Pro", SoundPlace::Bluetooth)],
        };
        let view = route_view(&route, &[SoundPlace::Speaker, SoundPlace::Phone]);
        let ids: Vec<&str> = view.choices.iter().map(|choice| choice.id.as_str()).collect();
        assert_eq!(ids, ["air-tsco", "speaker", "phone"]);
        assert_eq!(view.now, "bluetooth");
        assert_eq!(view.name, "AirPods Pro");
        assert_eq!(picked(&view), ["air-tsco"]);
    }

    #[test]
    fn the_built_in_place_playing_now_is_the_one_picked() {
        let both = [SoundPlace::Speaker, SoundPlace::Phone];
        let speaker = route_view(&SoundRoute { now: port("Speaker", "Speaker", SoundPlace::Speaker), headsets: vec![] }, &both);
        assert_eq!(picked(&speaker), ["speaker"]);
        let phone = route_view(&SoundRoute { now: port("Receiver", "Receiver", SoundPlace::Phone), headsets: vec![] }, &both);
        assert_eq!(picked(&phone), ["phone"]);
        assert_eq!(phone.now, "phone");
    }

    #[test]
    fn a_phone_without_an_earpiece_choice_offers_its_headsets_and_the_speaker() {
        let route = SoundRoute { now: port("40", "", SoundPlace::Wired), headsets: vec![port("40", "", SoundPlace::Wired)] };
        let view = route_view(&route, &[SoundPlace::Speaker]);
        let ids: Vec<&str> = view.choices.iter().map(|choice| choice.id.as_str()).collect();
        assert_eq!(ids, ["40", "speaker"]);
        assert_eq!(picked(&view), ["40"]);
        let bare = route_view(&SoundRoute { now: port("speaker", "", SoundPlace::Speaker), headsets: vec![] }, &[SoundPlace::Speaker]);
        assert_eq!(bare.choices.len(), 1);
    }

    #[test]
    fn only_the_speaker_and_the_phone_are_remembered() {
        assert_eq!(PhoneSound::kept_for(&SoundChoice::Speaker), Some(PhoneSound::Speaker));
        assert_eq!(PhoneSound::kept_for(&SoundChoice::Phone), Some(PhoneSound::Phone));
        assert_eq!(PhoneSound::kept_for(&SoundChoice::Headset("air".into())), None);
    }

    #[test]
    fn voice_settings_saved_before_this_choice_existed_start_on_the_speaker() {
        let saved = serde_json::json!({ "noise_cancellation": true, "muted": false, "deafened": false });
        let voice: VoiceSettings = serde_json::from_value(saved).unwrap();
        assert_eq!(voice.phone_sound, PhoneSound::Speaker);
        let phone = serde_json::to_value(PhoneSound::Phone).unwrap();
        assert_eq!(phone, serde_json::json!("phone"));
    }
}
