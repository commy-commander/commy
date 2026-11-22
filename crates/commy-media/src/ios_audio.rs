use crate::sound_route::{add_headset, headset_arrived, steer, SoundChoice, SoundPlace, SoundPort, SoundRoute, Steer};
use anyhow::{anyhow, Result};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_avf_audio::{
    AVAudioSession, AVAudioSessionCategoryOptions, AVAudioSessionCategoryPlayAndRecord, AVAudioSessionModeDefault,
    AVAudioSessionPort, AVAudioSessionPortBluetoothA2DP, AVAudioSessionPortBluetoothHFP, AVAudioSessionPortBluetoothLE,
    AVAudioSessionPortBuiltInMic, AVAudioSessionPortBuiltInReceiver, AVAudioSessionPortBuiltInSpeaker,
    AVAudioSessionPortCarAudio, AVAudioSessionPortDescription, AVAudioSessionPortHeadphones,
    AVAudioSessionPortHeadsetMic, AVAudioSessionPortLineOut, AVAudioSessionPortOverride, AVAudioSessionPortUSBAudio,
    AVAudioSessionRecordPermission, AVAudioSessionRouteChangeNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSString};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};
use tokio::sync::oneshot;

type RouteHeard = Arc<dyn Fn() + Send + Sync>;

static SPEAKER_WANTED: AtomicBool = AtomicBool::new(true);
static HEADSETS_SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
static ROUTE_HEARD: Mutex<Option<RouteHeard>> = Mutex::new(None);
static ROUTE_WATCH: Once = Once::new();

fn voice_options(speaker: bool) -> AVAudioSessionCategoryOptions {
    let headsets = AVAudioSessionCategoryOptions::AllowBluetoothHFP | AVAudioSessionCategoryOptions::AllowBluetoothA2DP;
    if speaker {
        headsets | AVAudioSessionCategoryOptions::DefaultToSpeaker
    } else {
        headsets
    }
}

fn session() -> Retained<AVAudioSession> {
    // SAFETY: the shared audio session is created by AVFAudio and lives for the whole process.
    unsafe { AVAudioSession::sharedInstance() }
}

fn set_voice_category(session: &AVAudioSession, speaker: bool) -> Result<()> {
    // SAFETY: the category and mode constants are provided by AVFAudio for the life of the process, and the setter only reads the values passed in.
    unsafe {
        let category = AVAudioSessionCategoryPlayAndRecord.ok_or_else(|| anyhow!("play and record category missing"))?;
        let mode = AVAudioSessionModeDefault.ok_or_else(|| anyhow!("default audio mode missing"))?;
        session
            .setCategory_mode_options_error(category, mode, voice_options(speaker))
            .map_err(|e| anyhow!("could not set the audio session for voice: {}", e.localizedDescription()))
    }
}

fn in_voice_category(session: &AVAudioSession) -> bool {
    // SAFETY: reading the category of the shared session has no side effects, and the constant lives for the process.
    unsafe { AVAudioSessionCategoryPlayAndRecord.is_some_and(|voice| session.category().isEqualToString(voice)) }
}

fn align_voice_category(session: &AVAudioSession, speaker: bool) -> Result<()> {
    // SAFETY: reading the options of the shared session has no side effects.
    let options = unsafe { session.categoryOptions() };
    if in_voice_category(session) && options == voice_options(speaker) {
        return Ok(());
    }
    set_voice_category(session, speaker)
}

fn override_output(session: &AVAudioSession, port: AVAudioSessionPortOverride) {
    // SAFETY: the override only changes where the shared session plays, and AVFAudio checks the category itself.
    if let Err(e) = unsafe { session.overrideOutputAudioPort_error(port) } {
        tracing::warn!("could not move the sound: {}", e.localizedDescription());
    }
}

fn prefer_input(session: &AVAudioSession, port: Option<&AVAudioSessionPortDescription>) {
    // SAFETY: the port, when given, came from this session's own list of inputs.
    if let Err(e) = unsafe { session.setPreferredInput_error(port) } {
        tracing::warn!("could not pick the microphone for the sound route: {}", e.localizedDescription());
    }
}

fn is_port(kind: &NSString, port: Option<&'static AVAudioSessionPort>) -> bool {
    port.is_some_and(|port| kind.isEqualToString(port))
}

fn place_of(port: &AVAudioSessionPortDescription) -> SoundPlace {
    // SAFETY: the port type constants live for the process, and reading a port's type has no side effects.
    unsafe {
        let kind = port.portType();
        if is_port(&kind, AVAudioSessionPortBuiltInSpeaker) {
            SoundPlace::Speaker
        } else if is_port(&kind, AVAudioSessionPortBuiltInReceiver) || is_port(&kind, AVAudioSessionPortBuiltInMic) {
            SoundPlace::Phone
        } else if [AVAudioSessionPortBluetoothHFP, AVAudioSessionPortBluetoothA2DP, AVAudioSessionPortBluetoothLE]
            .into_iter()
            .any(|bluetooth| is_port(&kind, bluetooth))
        {
            SoundPlace::Bluetooth
        } else if [
            AVAudioSessionPortHeadphones,
            AVAudioSessionPortHeadsetMic,
            AVAudioSessionPortUSBAudio,
            AVAudioSessionPortLineOut,
        ]
        .into_iter()
        .any(|wired| is_port(&kind, wired))
        {
            SoundPlace::Wired
        } else if is_port(&kind, AVAudioSessionPortCarAudio) {
            SoundPlace::Car
        } else {
            SoundPlace::Other
        }
    }
}

fn sound_port(port: &AVAudioSessionPortDescription) -> SoundPort {
    // SAFETY: reading a port's name and id has no side effects.
    let (id, name) = unsafe { (port.UID().to_string(), port.portName().to_string()) };
    SoundPort { id, name, place: place_of(port) }
}

fn inputs(session: &AVAudioSession) -> Vec<Retained<AVAudioSessionPortDescription>> {
    // SAFETY: listing the session's inputs has no side effects.
    unsafe { session.availableInputs() }.map(|list| list.to_vec()).unwrap_or_default()
}

fn playing_now(session: &AVAudioSession) -> SoundPort {
    // SAFETY: reading the current route has no side effects.
    let first = unsafe { session.currentRoute().outputs() }.firstObject();
    first.map(|port| sound_port(&port)).unwrap_or(SoundPort {
        id: String::new(),
        name: String::new(),
        place: SoundPlace::Other,
    })
}

fn route_of(session: &AVAudioSession) -> SoundRoute {
    let now = playing_now(session);
    let mut headsets = Vec::new();
    for input in inputs(session) {
        add_headset(&mut headsets, sound_port(&input));
    }
    add_headset(&mut headsets, now.clone());
    SoundRoute { now, headsets }
}

fn note_headsets(route: &SoundRoute) -> bool {
    let ids: Vec<String> = route.headsets.iter().map(|headset| headset.id.clone()).collect();
    let mut seen = HEADSETS_SEEN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let arrived = headset_arrived(&seen, &ids);
    *seen = ids;
    arrived
}

fn steer_output(session: &AVAudioSession) {
    let route = route_of(session);
    let arrived = note_headsets(&route);
    if !in_voice_category(session) {
        return;
    }
    match steer(route.now.place, SPEAKER_WANTED.load(Ordering::Relaxed), arrived) {
        Steer::Leave => {}
        Steer::ToSpeaker => override_output(session, AVAudioSessionPortOverride::Speaker),
        Steer::ToPhone => {
            override_output(session, AVAudioSessionPortOverride::None);
            if let Err(e) = align_voice_category(session, false) {
                tracing::warn!("{e}");
            }
        }
        Steer::FollowNewHeadset => {
            prefer_input(session, None);
            override_output(session, AVAudioSessionPortOverride::None);
        }
    }
}

fn route_changed() {
    let session = session();
    steer_output(&session);
    tracing::info!("sound plays through the {}", playing_now(&session).place.word());
    let heard = ROUTE_HEARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
    if let Some(heard) = heard {
        heard();
    }
}

fn watch_routes() {
    ROUTE_WATCH.call_once(|| {
        // SAFETY: the notification name lives for the process.
        let Some(name) = (unsafe { AVAudioSessionRouteChangeNotification }) else {
            return;
        };
        let block = RcBlock::new(|_: NonNull<NSNotification>| route_changed());
        // SAFETY: the block only reads and steers the shared audio session, which is safe from any thread, and the observer is kept for the life of the process.
        let observer = unsafe {
            NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
        };
        std::mem::forget(observer);
    });
}

pub fn want_speaker(speaker: bool) {
    SPEAKER_WANTED.store(speaker, Ordering::Relaxed);
}

pub fn on_sound_route_change(heard: impl Fn() + Send + Sync + 'static) {
    *ROUTE_HEARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::new(heard));
}

pub fn sound_route() -> SoundRoute {
    route_of(&session())
}

pub fn choose_sound(choice: &SoundChoice) -> Result<()> {
    let session = session();
    match choice {
        SoundChoice::Speaker => {
            want_speaker(true);
            if in_voice_category(&session) {
                prefer_input(&session, None);
                align_voice_category(&session, true)?;
                override_output(&session, AVAudioSessionPortOverride::Speaker);
            }
        }
        SoundChoice::Phone => {
            want_speaker(false);
            if in_voice_category(&session) {
                align_voice_category(&session, false)?;
                override_output(&session, AVAudioSessionPortOverride::None);
                let built_in = inputs(&session).into_iter().find(|input| place_of(input) == SoundPlace::Phone);
                prefer_input(&session, built_in.as_deref());
            }
        }
        SoundChoice::Headset(id) => {
            override_output(&session, AVAudioSessionPortOverride::None);
            let headset = inputs(&session).into_iter().find(|input| sound_port(input).id == *id);
            prefer_input(&session, headset.as_deref());
        }
    }
    Ok(())
}

pub fn prepare_session_for_voice() -> Result<()> {
    watch_routes();
    let session = session();
    set_voice_category(&session, SPEAKER_WANTED.load(Ordering::Relaxed))?;
    // SAFETY: activating the shared session only changes its own state.
    unsafe { session.setActive_error(true) }
        .map_err(|e| anyhow!("could not start the audio session: {}", e.localizedDescription()))?;
    steer_output(&session);
    tracing::info!("voice sound starts on the {}", playing_now(&session).place.word());
    Ok(())
}

pub fn wake_session() {
    // SAFETY: the shared audio session lives for the whole process, and activating it only changes its own state.
    let woke = unsafe { session().setActive_error(true) };
    if let Err(e) = woke {
        tracing::debug!("the audio session is not back yet: {}", e.localizedDescription());
    }
}

#[allow(deprecated)]
fn known_microphone_answer() -> Option<bool> {
    // SAFETY: reading the record permission of the shared audio session has no side effects.
    let permission = unsafe { session().recordPermission() };
    if permission == AVAudioSessionRecordPermission::Granted {
        Some(true)
    } else if permission == AVAudioSessionRecordPermission::Denied {
        Some(false)
    } else {
        None
    }
}

#[allow(deprecated)]
fn request_microphone(answer: oneshot::Sender<bool>) {
    let answer = Mutex::new(Some(answer));
    let reply = RcBlock::new(move |granted: Bool| {
        if let Some(answer) = answer.lock().unwrap().take() {
            let _ = answer.send(granted.as_bool());
        }
    });
    // SAFETY: AVFAudio copies the block and keeps it until it answers, and the block only sends that answer on a channel.
    unsafe { session().requestRecordPermission(&reply) };
}

pub async fn ask_for_microphone() -> bool {
    if let Some(answer) = known_microphone_answer() {
        return answer;
    }
    let (tx, rx) = oneshot::channel();
    request_microphone(tx);
    rx.await.unwrap_or(false)
}
