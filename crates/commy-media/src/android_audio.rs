use std::sync::Mutex;

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait};
use oboe::{AudioDeviceDirection, AudioDeviceInfo};

use crate::devices::StreamFault;
use crate::sound_route::{phone_pick, phone_route, pick_holds, AudioPort, PhonePick, SoundChoice, SoundRoute};

type RouteChanged = Box<dyn Fn() + Send + Sync>;

static PICK: Mutex<Option<PhonePick>> = Mutex::new(None);
static OPENED: Mutex<[Option<i32>; 2]> = Mutex::new([None, None]);
static STREAMS: Mutex<[Option<StreamFault>; 2]> = Mutex::new([None, None]);
static HEARD: Mutex<Option<SoundRoute>> = Mutex::new(None);
static ON_CHANGE: Mutex<Option<RouteChanged>> = Mutex::new(None);

fn slot(input: bool) -> usize {
    usize::from(input)
}

fn ports(direction: AudioDeviceDirection) -> Vec<AudioPort> {
    AudioDeviceInfo::request(direction)
        .unwrap_or_default()
        .into_iter()
        .map(|device| AudioPort { id: device.id, kind: device.device_type as i32, name: device.product_name })
        .collect()
}

fn current_pick(outputs: &[AudioPort]) -> Option<PhonePick> {
    let mut pick = PICK.lock().unwrap();
    if pick.as_ref().is_some_and(|picked| !pick_holds(picked, outputs)) {
        tracing::info!("the picked sound place left or a headset arrived, sound follows the system again");
        *pick = None;
    }
    pick.clone()
}

pub fn sound_route() -> SoundRoute {
    let outputs = ports(AudioDeviceDirection::Output);
    let pick = current_pick(&outputs);
    phone_route(&outputs, pick.as_ref())
}

pub fn on_sound_route_change(changed: impl Fn() + Send + Sync + 'static) {
    *ON_CHANGE.lock().unwrap() = Some(Box::new(changed));
}

pub fn choose_sound(choice: &SoundChoice) -> Result<()> {
    let outputs = ports(AudioDeviceDirection::Output);
    let inputs = ports(AudioDeviceDirection::Input);
    let pick = phone_pick(choice, &outputs, &inputs).ok_or_else(|| anyhow!("that sound place is not connected"))?;
    let route = phone_route(&outputs, Some(&pick));
    tracing::info!("sound moves to the {}", route.now.place.word());
    *PICK.lock().unwrap() = Some(pick);
    *HEARD.lock().unwrap() = Some(route);
    for fault in STREAMS.lock().unwrap().iter().flatten() {
        fault.raise();
    }
    Ok(())
}

pub(crate) fn opening(input: bool, fault: &StreamFault) {
    STREAMS.lock().unwrap()[slot(input)] = Some(fault.clone());
}

pub(crate) fn routed_device(host: &cpal::Host, input: bool) -> Option<cpal::Device> {
    let wanted = PICK.lock().unwrap().as_ref().and_then(|pick| if input { pick.input } else { pick.output });
    OPENED.lock().unwrap()[slot(input)] = wanted;
    let wanted = wanted?;
    let all = ports(AudioDeviceDirection::InputOutput);
    let found = all.iter().position(|port| port.id == wanted).and_then(|index| {
        let device = host.devices().ok()?.nth(index)?;
        (device.name().ok()? == all[index].name).then_some(device)
    });
    if found.is_none() {
        tracing::warn!("the picked sound device is not listed, using the system default");
    }
    found
}

pub(crate) fn route_moved(input: bool) -> Option<bool> {
    if !input {
        tell_route_changes();
    }
    let wanted = PICK.lock().unwrap().as_ref().and_then(|pick| if input { pick.input } else { pick.output });
    let opened = OPENED.lock().unwrap()[slot(input)];
    (wanted.is_some() || opened.is_some()).then_some(wanted != opened)
}

fn tell_route_changes() {
    let route = sound_route();
    let mut heard = HEARD.lock().unwrap();
    if heard.as_ref() == Some(&route) {
        return;
    }
    if heard.as_ref().is_some_and(|before| before.now != route.now) {
        tracing::info!("sound plays through the {}", route.now.place.word());
    }
    *heard = Some(route);
    drop(heard);
    if let Some(changed) = ON_CHANGE.lock().unwrap().as_ref() {
        changed();
    }
}
