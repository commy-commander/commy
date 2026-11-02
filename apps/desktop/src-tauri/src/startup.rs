use commy_net::voice_gate::StateLink;
use crate::state::App;
use crate::updater::{self, Phase, UpdateStatus};
use commy_core::ServerId;
use commy_net::RelayLink;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

pub const TICK: Duration = Duration::from_millis(250);
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
pub const UPDATE_PATIENCE: Duration = Duration::from_secs(25);
pub const SYNC_PATIENCE: Duration = Duration::from_secs(15);
pub const SKIP_AFTER: Duration = Duration::from_secs(4);
pub const RESTART_PAUSE: Duration = Duration::from_millis(1200);
pub const OFFLINE_RECHECK: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Working,
    Done,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Step {
    pub state: StepState,
    pub detail: String,
    pub progress: Option<u8>,
}

impl Step {
    fn new(state: StepState, detail: impl Into<String>) -> Self {
        Step { state, detail: detail.into(), progress: None }
    }

    fn working(detail: impl Into<String>) -> Self {
        Step::new(StepState::Working, detail)
    }

    fn done(detail: impl Into<String>) -> Self {
        Step::new(StepState::Done, detail)
    }

    fn skipped(detail: impl Into<String>) -> Self {
        Step::new(StepState::Skipped, detail)
    }

    fn failed(detail: impl Into<String>) -> Self {
        Step::new(StepState::Failed, detail)
    }

    pub fn settled(&self) -> bool {
        self.state != StepState::Working
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Unknown,
    Reachable,
    Unreachable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Unknown,
    Used,
    NotUsed,
    Unreachable,
}

pub struct ConnectionSignals {
    pub account_server: Reach,
    pub relay: Option<RelayLink>,
}

pub fn connection_step(signals: &ConnectionSignals) -> (Step, bool) {
    let relay_up = signals.relay == Some(RelayLink::Connected);
    match signals.account_server {
        Reach::Reachable => (Step::done("You are online"), false),
        _ if relay_up => (Step::done("You are online"), false),
        Reach::Unreachable => (Step::failed("Offline. Opening what is saved on this device."), true),
        Reach::Unknown => (Step::working("Checking your connection"), false),
    }
}

pub struct UpdateSignals<'a> {
    pub auto: bool,
    pub status: &'a UpdateStatus,
    pub offline: bool,
    pub restarting: bool,
    pub waited: Duration,
}

fn named(version: &Option<String>) -> String {
    version.as_deref().map(|v| format!("Commy {v}")).unwrap_or_else(|| "the new version".into())
}

pub fn update_step(signals: &UpdateSignals) -> Step {
    let status = signals.status;
    match status.reason.as_deref() {
        Some("dev") => return Step::skipped("Development builds do not update themselves"),
        Some("store") => return Step::skipped("Updates come from the app store"),
        _ => {}
    }
    if signals.restarting {
        return Step::working(format!("Restarting into {}", named(&status.version)));
    }
    if !signals.auto {
        return Step::skipped("Automatic updates are off");
    }
    let still_checking = || {
        if signals.offline {
            Step::skipped("Checks again once you are online")
        } else if signals.waited >= UPDATE_PATIENCE {
            Step::skipped("Still checking. Commy finishes in the background.")
        } else {
            Step::working("Looking for a new version")
        }
    };
    match status.phase {
        Phase::Idle | Phase::Checking => still_checking(),
        Phase::UpToDate => Step::done("You have the latest version"),
        Phase::Available => match status.reason.as_deref() {
            Some("portable") => Step::done(format!("{} is out. Download it from the website.", named(&status.version))),
            Some("package") => Step::done(format!("{} is out. Your package manager brings it.", named(&status.version))),
            _ => Step::done(format!("{} is out", named(&status.version))),
        },
        Phase::Downloading => Step {
            progress: status.progress,
            ..Step::working(format!("Downloading {}", named(&status.version)))
        },
        Phase::Ready => Step::working(format!("Installing {}", named(&status.version))),
        Phase::Failed if signals.offline => Step::skipped("Checks again once you are online"),
        Phase::Failed => Step::failed("Could not check for updates. Commy tries again later."),
    }
}

pub struct SyncSignals {
    pub signed_in: bool,
    pub offline: bool,
    pub service: Service,
    pub account_current: bool,
    pub servers: Vec<StateLink>,
    pub waited: Duration,
}

fn server_count(count: usize) -> String {
    if count == 1 {
        "1 server".into()
    } else {
        format!("{count} servers")
    }
}

fn sync_progress(signals: &SyncSignals) -> Step {
    let lost = signals.servers.iter().filter(|link| **link == StateLink::Lost).count();
    let behind = signals.servers.iter().filter(|link| **link == StateLink::Behind).count();
    if lost > 0 {
        Step::working("Reconnecting to the sync service")
    } else if behind > 0 {
        Step::working(format!("Catching up {}", server_count(behind)))
    } else if !signals.account_current {
        Step::working("Loading your account")
    } else {
        Step::done("Everything is up to date")
    }
}

pub fn sync_step(signals: &SyncSignals) -> Step {
    if !signals.signed_in {
        return Step::skipped("Starts after you sign in");
    }
    if signals.offline {
        return Step::skipped("Your servers catch up once you are online");
    }
    let step = match signals.service {
        Service::Unknown => Step::working("Connecting to the sync service"),
        Service::Unreachable => Step::failed("Could not reach the sync service. Commy keeps trying."),
        Service::NotUsed => Step::skipped("Your servers catch up from other members"),
        Service::Used => sync_progress(signals),
    };
    if !step.settled() && signals.waited >= SYNC_PATIENCE {
        return Step::skipped("Still catching up. Commy finishes in the background.");
    }
    step
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartupView {
    pub connection: Step,
    pub update: Step,
    pub sync: Step,
    pub offline: bool,
    pub restarting: bool,
    pub ready: bool,
    pub can_skip: bool,
}

pub fn view_of(connection: Step, offline: bool, update: Step, sync: Step, restarting: bool, waited: Duration) -> StartupView {
    let ready = !restarting && connection.settled() && update.settled() && sync.settled();
    StartupView { connection, update, sync, offline, restarting, ready, can_skip: !restarting && !ready && waited >= SKIP_AFTER }
}

pub fn wants_update_at_launch(auto: bool, status: &UpdateStatus) -> bool {
    auto && updater::VERSION.is_some() && status.reason.as_deref() != Some("store")
}

static STARTED: OnceLock<Instant> = OnceLock::new();
static PROBE: Mutex<Reach> = Mutex::new(Reach::Unknown);
static SESSION: Mutex<Service> = Mutex::new(Service::Unknown);
static HANDED_OFF: AtomicBool = AtomicBool::new(false);
static RESTARTING: AtomicBool = AtomicBool::new(false);
static LAST: Mutex<Option<StartupView>> = Mutex::new(None);

fn waited() -> Duration {
    STARTED.get_or_init(Instant::now).elapsed()
}

pub fn note_session(service: Service) {
    *SESSION.lock().unwrap() = service;
}

pub fn handed_off() -> bool {
    HANDED_OFF.load(Ordering::SeqCst)
}

async fn probe(app: &App) -> Reach {
    let url = format!("{}/.well-known/commy-auth.json", crate::account::effective_server());
    match app.http.get(url).timeout(PROBE_TIMEOUT).send().await {
        Ok(_) => Reach::Reachable,
        Err(e) => {
            tracing::info!("startup connection check could not reach the account server: {e}");
            Reach::Unreachable
        }
    }
}

async fn relay(app: &App) -> Option<RelayLink> {
    app.net.lock().await.as_ref().map(|node| node.connection_checker().relay())
}

fn sync_signals(app: &App, offline: bool) -> SyncSignals {
    let signed_in = crate::account::is_signed_in(&app.account.lock().unwrap());
    let servers: Vec<ServerId> = app.servers.lock().unwrap().keys().copied().collect();
    SyncSignals {
        signed_in,
        offline,
        service: *SESSION.lock().unwrap(),
        account_current: !crate::account_log::is_live(app) || crate::account_log::caught_up(),
        servers: servers.into_iter().map(|sid| crate::voice_gate::state_link(app, sid)).collect(),
        waited: waited(),
    }
}

async fn view(handle: &AppHandle) -> StartupView {
    let app = handle.state::<App>();
    let account_server = *PROBE.lock().unwrap();
    let signals = ConnectionSignals { account_server, relay: relay(&app).await };
    let (connection, offline) = connection_step(&signals);
    let restarting = RESTARTING.load(Ordering::SeqCst);
    let status = updater::status(handle);
    let auto = app.voice.lock().unwrap().auto_update;
    let update = update_step(&UpdateSignals { auto, status: &status, offline, restarting, waited: waited() });
    let sync = sync_step(&sync_signals(&app, offline));
    view_of(connection, offline, update, sync, restarting, waited())
}

pub async fn current(handle: &AppHandle) -> StartupView {
    let fresh = view(handle).await;
    *LAST.lock().unwrap() = Some(fresh.clone());
    fresh
}

async fn publish(handle: &AppHandle) -> StartupView {
    let fresh = view(handle).await;
    let changed = LAST.lock().unwrap().replace(fresh.clone()).as_ref() != Some(&fresh);
    if changed {
        let _ = handle.emit("startup-status", &fresh);
    }
    fresh
}

pub fn hand_off() {
    HANDED_OFF.store(true, Ordering::SeqCst);
}

pub fn start(handle: &AppHandle) {
    STARTED.get_or_init(Instant::now);
    let probing = handle.clone();
    tauri::async_runtime::spawn(async move {
        let reach = probe(&probing.state::<App>()).await;
        *PROBE.lock().unwrap() = reach;
    });
    tauri::async_runtime::spawn(update_at_launch(handle.clone()));
    tauri::async_runtime::spawn(run(handle.clone()));
}

async fn update_at_launch(handle: AppHandle) {
    let auto = handle.state::<App>().voice.lock().unwrap().auto_update;
    if !wants_update_at_launch(auto, &updater::status(&handle)) {
        return;
    }
    let status = updater::check(&handle).await;
    if status.phase != Phase::Ready || handed_off() {
        return;
    }
    RESTARTING.store(true, Ordering::SeqCst);
    publish(&handle).await;
    tokio::time::sleep(RESTART_PAUSE).await;
    tracing::info!("restarting into the update found at launch");
    if let Err(e) = updater::restart(&handle) {
        tracing::warn!("could not restart into the update: {e}");
        RESTARTING.store(false, Ordering::SeqCst);
        publish(&handle).await;
    }
}

async fn run(handle: AppHandle) {
    loop {
        let fresh = publish(&handle).await;
        if fresh.ready {
            hand_off();
        }
        if handed_off() {
            if !fresh.offline {
                return;
            }
            tokio::time::sleep(OFFLINE_RECHECK).await;
            let reach = probe(&handle.state::<App>()).await;
            if reach == Reach::Reachable {
                crate::account::recheck_session_now();
                crate::community_sync::nudge();
            }
            *PROBE.lock().unwrap() = reach;
            continue;
        }
        tokio::time::sleep(TICK).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(phase: Phase) -> UpdateStatus {
        UpdateStatus { current: "0.2.0".into(), can_install: true, phase, version: Some("0.2.1".into()), ..UpdateStatus::default() }
    }

    fn update(status: &UpdateStatus) -> UpdateSignals<'_> {
        UpdateSignals { auto: true, status, offline: false, restarting: false, waited: Duration::from_secs(1) }
    }

    fn synced() -> SyncSignals {
        SyncSignals {
            signed_in: true,
            offline: false,
            service: Service::Used,
            account_current: true,
            servers: vec![StateLink::Current, StateLink::NotUsed],
            waited: Duration::from_secs(2),
        }
    }

    #[test]
    fn reaching_the_account_server_or_the_relay_means_online() {
        let (step, offline) = connection_step(&ConnectionSignals { account_server: Reach::Reachable, relay: None });
        assert_eq!((step.state, offline), (StepState::Done, false));
        let (step, offline) = connection_step(&ConnectionSignals { account_server: Reach::Unreachable, relay: Some(RelayLink::Connected) });
        assert_eq!((step.state, offline), (StepState::Done, false));
    }

    #[test]
    fn nothing_reachable_means_offline_and_the_app_still_opens() {
        let (step, offline) = connection_step(&ConnectionSignals { account_server: Reach::Unreachable, relay: Some(RelayLink::Unreachable) });
        assert_eq!(step.state, StepState::Failed);
        assert!(offline);
        assert!(step.settled());
        assert!(step.detail.contains("saved on this device"));
    }

    #[test]
    fn the_connection_check_waits_for_an_answer() {
        let (step, offline) = connection_step(&ConnectionSignals { account_server: Reach::Unknown, relay: Some(RelayLink::Unreachable) });
        assert_eq!(step.state, StepState::Working);
        assert!(!offline);
    }

    #[test]
    fn an_update_downloads_with_progress_then_installs() {
        let mut downloading = status(Phase::Downloading);
        downloading.progress = Some(42);
        let step = update_step(&update(&downloading));
        assert_eq!(step.state, StepState::Working);
        assert_eq!(step.progress, Some(42));
        assert_eq!(step.detail, "Downloading Commy 0.2.1");
        assert_eq!(update_step(&update(&status(Phase::Ready))).detail, "Installing Commy 0.2.1");
        let ready = status(Phase::Ready);
        let restarting = update_step(&UpdateSignals { restarting: true, ..update(&ready) });
        assert_eq!(restarting.detail, "Restarting into Commy 0.2.1");
    }

    #[test]
    fn the_update_step_settles_when_current_or_off() {
        assert_eq!(update_step(&update(&status(Phase::UpToDate))).state, StepState::Done);
        let checking = status(Phase::Checking);
        assert_eq!(update_step(&UpdateSignals { auto: false, ..update(&checking) }).state, StepState::Skipped);
        let dev = UpdateStatus { reason: Some("dev".into()), ..status(Phase::Idle) };
        assert_eq!(update_step(&update(&dev)).state, StepState::Skipped);
    }

    #[test]
    fn offline_or_a_slow_check_does_not_hold_the_app() {
        let checking = status(Phase::Checking);
        assert_eq!(update_step(&update(&checking)).state, StepState::Working);
        assert_eq!(update_step(&UpdateSignals { offline: true, ..update(&checking) }).state, StepState::Skipped);
        assert_eq!(update_step(&UpdateSignals { waited: UPDATE_PATIENCE, ..update(&checking) }).state, StepState::Skipped);
        let failed = status(Phase::Failed);
        assert_eq!(update_step(&update(&failed)).state, StepState::Failed);
        assert_eq!(update_step(&UpdateSignals { offline: true, ..update(&failed) }).state, StepState::Skipped);
    }

    #[test]
    fn a_download_is_never_cut_short_by_patience() {
        let downloading = status(Phase::Downloading);
        assert_eq!(update_step(&UpdateSignals { waited: UPDATE_PATIENCE * 4, ..update(&downloading) }).state, StepState::Working);
    }

    #[test]
    fn copies_that_cannot_install_say_where_the_update_comes_from() {
        let portable = UpdateStatus { reason: Some("portable".into()), can_install: false, ..status(Phase::Available) };
        assert_eq!(update_step(&update(&portable)).detail, "Commy 0.2.1 is out. Download it from the website.");
        let store = UpdateStatus { reason: Some("store".into()), ..status(Phase::Idle) };
        assert_eq!(update_step(&update(&store)).state, StepState::Skipped);
    }

    #[test]
    fn only_auto_update_on_a_released_build_checks_at_launch() {
        let idle = status(Phase::Idle);
        assert_eq!(wants_update_at_launch(true, &idle), updater::VERSION.is_some());
        assert!(!wants_update_at_launch(false, &idle));
        let store = UpdateStatus { reason: Some("store".into()), ..idle };
        assert!(!wants_update_at_launch(true, &store));
    }

    #[test]
    fn sync_is_done_when_every_server_is_current() {
        assert_eq!(sync_step(&synced()).state, StepState::Done);
    }

    #[test]
    fn sync_reports_what_it_is_waiting_for() {
        let behind = SyncSignals { servers: vec![StateLink::Behind, StateLink::Behind, StateLink::Current], ..synced() };
        assert_eq!(sync_step(&behind).detail, "Catching up 2 servers");
        let lost = SyncSignals { servers: vec![StateLink::Lost, StateLink::Behind], ..synced() };
        assert_eq!(sync_step(&lost).detail, "Reconnecting to the sync service");
        let account = SyncSignals { account_current: false, ..synced() };
        assert_eq!(sync_step(&account).detail, "Loading your account");
        let unknown = SyncSignals { service: Service::Unknown, ..synced() };
        assert_eq!(sync_step(&unknown).state, StepState::Working);
    }

    #[test]
    fn sync_steps_aside_when_signed_out_offline_unused_or_slow() {
        assert_eq!(sync_step(&SyncSignals { signed_in: false, ..synced() }).state, StepState::Skipped);
        assert_eq!(sync_step(&SyncSignals { offline: true, ..synced() }).state, StepState::Skipped);
        assert_eq!(sync_step(&SyncSignals { service: Service::NotUsed, ..synced() }).state, StepState::Skipped);
        assert_eq!(sync_step(&SyncSignals { service: Service::Unreachable, ..synced() }).state, StepState::Failed);
        let slow = SyncSignals { servers: vec![StateLink::Behind], waited: SYNC_PATIENCE, ..synced() };
        assert_eq!(sync_step(&slow).state, StepState::Skipped);
    }

    #[test]
    fn the_app_opens_once_every_step_settles_and_never_while_restarting() {
        let open = view_of(Step::done("a"), false, Step::done("b"), Step::skipped("c"), false, Duration::from_secs(1));
        assert!(open.ready);
        assert!(!open.can_skip);
        let busy = view_of(Step::done("a"), false, Step::working("b"), Step::done("c"), false, SKIP_AFTER);
        assert!(!busy.ready);
        assert!(busy.can_skip);
        let restarting = view_of(Step::done("a"), false, Step::done("b"), Step::done("c"), true, SKIP_AFTER);
        assert!(!restarting.ready);
        assert!(!restarting.can_skip);
    }

    #[test]
    fn the_view_reaches_the_app_in_plain_words() {
        let view = view_of(Step::failed("off"), true, Step::skipped("u"), Step::skipped("s"), false, Duration::ZERO);
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["connection"]["state"], "failed");
        assert_eq!(json["offline"], true);
        assert_eq!(json["ready"], true);
        assert_eq!(json["update"]["progress"], serde_json::Value::Null);
    }
}
