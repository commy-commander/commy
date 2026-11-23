use crate::lock_or_recover::LockOrRecover;
use serde::{de::IgnoredAny, Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
#[cfg(desktop)]
use tauri_plugin_updater::UpdaterExt;

pub const VERSION: Option<&str> = option_env!("COMMY_VERSION");
pub const RELEASES: &str = match option_env!("COMMY_RELEASES_URL") {
    Some(url) => url,
    None => "https://releases.commy.chat",
};
const DOWNLOAD_PAGE: &str = "https://commy.chat/download";
const MANIFEST: &str = "latest.json";
const STABLE_CHANNEL: &str = "stable";
const TEST_CHANNEL: &str = "rc";
const NIGHTLY_CHANNEL: &str = "nightly";
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
const FIRST_CHECK: Duration = Duration::from_secs(30);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(mobile, allow(dead_code))]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Preview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Track {
    Stable,
    Preview,
    Test,
}

impl Track {
    pub fn pick(test_build: bool, channel: UpdateChannel) -> Self {
        match (test_build, channel) {
            (true, _) => Track::Test,
            (false, UpdateChannel::Stable) => Track::Stable,
            (false, UpdateChannel::Preview) => Track::Preview,
        }
    }

    pub fn feeds(self) -> &'static [&'static str] {
        match self {
            Track::Stable => &[STABLE_CHANNEL],
            Track::Preview => &[NIGHTLY_CHANNEL, STABLE_CHANNEL],
            Track::Test => &[TEST_CHANNEL],
        }
    }

    pub fn takes(self, remote: &semver::Version) -> bool {
        let is_final = remote.pre.is_empty();
        match self {
            Track::Stable => is_final,
            Track::Preview => is_final || is_nightly(remote),
            Track::Test => !is_final,
        }
    }
}

pub fn is_nightly(version: &semver::Version) -> bool {
    version.pre.as_str().split('.').next() == Some(NIGHTLY_CHANNEL)
}

pub fn platform_key() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateStatus {
    pub current: String,
    pub can_install: bool,
    pub can_choose_channel: bool,
    pub reason: Option<String>,
    pub phase: Phase,
    pub version: Option<String>,
    pub progress: Option<u8>,
    pub checked_at: Option<i64>,
    pub error: Option<String>,
    pub page: Option<String>,
}

#[derive(Default)]
pub struct Updates {
    status: Mutex<UpdateStatus>,
    busy: AtomicBool,
    #[cfg(desktop)]
    pending: Mutex<Option<(tauri_plugin_updater::Update, Vec<u8>)>>,
}

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub platforms: HashMap<String, IgnoredAny>,
}

impl Manifest {
    pub fn serves(&self, platform: &str) -> bool {
        let installer_prefix = format!("{platform}-");
        self.platforms.keys().any(|key| key == platform || key.starts_with(&installer_prefix))
    }
}

pub fn parse_version(tag: &str) -> Option<semver::Version> {
    semver::Version::parse(tag.trim().trim_start_matches('v')).ok()
}

pub fn manifest_url(releases: &str, feed: &str) -> String {
    format!("{}/{feed}/{MANIFEST}", releases.trim_end_matches('/'))
}

pub fn is_offered(remote: &semver::Version, current: &semver::Version, track: Track) -> bool {
    remote > current && track.takes(remote)
}

pub fn offered_version(manifest: &Manifest, current: &semver::Version, track: Track) -> Option<semver::Version> {
    parse_version(&manifest.version).filter(|remote| is_offered(remote, current, track))
}

pub fn best_offer<'a>(feeds: &'a [(String, Manifest)], current: &semver::Version, track: Track, platform: &str) -> Option<(&'a str, semver::Version)> {
    feeds
        .iter()
        .filter(|(_, manifest)| manifest.serves(platform))
        .filter_map(|(url, manifest)| offered_version(manifest, current, track).map(|version| (url.as_str(), version)))
        .max_by(|a, b| a.1.cmp(&b.1))
}

fn install_blocker(handle: &AppHandle) -> Option<&'static str> {
    if cfg!(mobile) {
        return Some("store");
    }
    #[cfg(windows)]
    {
        let installed = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("uninstall.exe").exists()))
            .unwrap_or(false);
        if !installed {
            return Some("portable");
        }
    }
    #[cfg(target_os = "linux")]
    {
        if handle.env().appimage.is_none() {
            return Some("package");
        }
    }
    let _ = handle;
    None
}

pub(crate) fn current_version() -> String {
    VERSION.map(str::to_string).unwrap_or_else(|| format!("{}-dev", env!("CARGO_PKG_VERSION")))
}

pub fn init(handle: &AppHandle) {
    let reason = if VERSION.is_none() { Some("dev") } else { install_blocker(handle) };
    let updates = handle.state::<Updates>();
    let mut status = updates.status.lock_or_recover();
    status.current = current_version();
    status.can_install = reason.is_none();
    status.can_choose_channel = !crate::instance::TEST_BUILD && cfg!(desktop);
    status.reason = reason.map(str::to_string);
}

fn chosen_channel(handle: &AppHandle) -> UpdateChannel {
    handle.state::<crate::App>().voice.lock_or_recover().update_channel
}

pub fn channel_changed(handle: &AppHandle) {
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        let auto = handle.state::<crate::App>().voice.lock_or_recover().auto_update;
        run(&handle, Mode::when_auto_update(auto)).await;
    });
}

pub fn status(handle: &AppHandle) -> UpdateStatus {
    handle.state::<Updates>().status.lock_or_recover().clone()
}

fn set(handle: &AppHandle, change: impl FnOnce(&mut UpdateStatus)) {
    let status = {
        let updates = handle.state::<Updates>();
        let mut status = updates.status.lock_or_recover();
        change(&mut status);
        status.clone()
    };
    let _ = handle.emit("update-status", &status);
}

pub fn start(handle: &AppHandle) {
    if VERSION.is_none() {
        return;
    }
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            let auto = handle.state::<crate::App>().voice.lock_or_recover().auto_update;
            if check_due(&status(&handle), chrono::Utc::now().timestamp()) {
                run(&handle, Mode::when_auto_update(auto)).await;
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

pub fn check_due(status: &UpdateStatus, now: i64) -> bool {
    let fresh_for = (CHECK_EVERY - FIRST_CHECK).as_secs() as i64;
    match status.phase {
        Phase::Checking | Phase::Downloading | Phase::Ready => false,
        Phase::Failed => true,
        _ => status.checked_at.is_none_or(|at| now - at >= fresh_for),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    Install,
    LookOnly,
}

impl Mode {
    pub fn when_auto_update(auto: bool) -> Self {
        if auto {
            Mode::Install
        } else {
            Mode::LookOnly
        }
    }
}

pub async fn check(handle: &AppHandle) -> UpdateStatus {
    run(handle, Mode::Install).await
}

pub async fn look(handle: &AppHandle) -> UpdateStatus {
    run(handle, Mode::LookOnly).await
}

async fn run(handle: &AppHandle, mode: Mode) -> UpdateStatus {
    if VERSION.is_none() {
        return status(handle);
    }
    let updates = handle.state::<Updates>();
    let phase = updates.status.lock_or_recover().phase;
    if phase == Phase::Ready || updates.busy.swap(true, Ordering::SeqCst) {
        return status(handle);
    }
    set(handle, |s| {
        s.phase = Phase::Checking;
        s.error = None;
        s.progress = None;
    });
    let result = run_check(handle, mode).await;
    updates.busy.store(false, Ordering::SeqCst);
    let now = chrono::Utc::now().timestamp();
    match result {
        Ok(()) => set(handle, |s| {
            if s.phase == Phase::Checking {
                s.phase = Phase::UpToDate;
                s.version = None;
                s.page = None;
            }
            s.checked_at = Some(now);
        }),
        Err(e) => {
            tracing::warn!("update check failed: {e}");
            set(handle, |s| {
                s.phase = Phase::Failed;
                s.error = Some(e);
                s.progress = None;
                s.checked_at = Some(now);
            });
        }
    }
    status(handle)
}

async fn run_check(handle: &AppHandle, mode: Mode) -> Result<(), String> {
    let Some(current) = VERSION.and_then(parse_version) else {
        return Err("This build does not update itself.".into());
    };
    let track = Track::pick(crate::instance::TEST_BUILD, chosen_channel(handle));
    let feeds = fetch_feeds(track).await?;
    let Some((manifest, version)) = best_offer(&feeds, &current, track, &platform_key()) else {
        return Ok(());
    };
    let manifest = manifest.to_string();
    let found = version.to_string();
    let page = DOWNLOAD_PAGE.to_string();
    let can_install = handle.state::<Updates>().status.lock_or_recover().can_install;
    if !can_install || mode == Mode::LookOnly {
        set(handle, |s| {
            s.phase = Phase::Available;
            s.version = Some(found);
            s.page = Some(page);
        });
        return Ok(());
    }
    install(handle, &manifest, current, track, found, page).await
}

async fn fetch_feeds(track: Track) -> Result<Vec<(String, Manifest)>, String> {
    let mut feeds = Vec::new();
    let mut first_error = None;
    for feed in track.feeds() {
        let url = manifest_url(RELEASES, feed);
        match fetch_manifest(&url).await {
            Ok(manifest) => feeds.push((url, manifest)),
            Err(e) => {
                tracing::info!("update feed {feed} unavailable: {e}");
                first_error.get_or_insert(e);
            }
        }
    }
    match first_error {
        Some(e) if feeds.is_empty() => Err(e),
        _ => Ok(feeds),
    }
}

#[cfg(mobile)]
async fn install(_handle: &AppHandle, _manifest: &str, _current: semver::Version, _track: Track, _found: String, _page: String) -> Result<(), String> {
    Ok(())
}

#[cfg(desktop)]
async fn install(handle: &AppHandle, manifest: &str, current: semver::Version, track: Track, found: String, page: String) -> Result<(), String> {
    let url = manifest.parse().map_err(|e| format!("bad manifest link: {e}"))?;
    let newer_than = current;
    let leaving = handle.clone();
    let update = handle
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .timeout(FETCH_TIMEOUT)
        .on_before_exit(move || leave_for_installer(&leaving))
        .version_comparator(move |_, remote| is_offered(&remote.version, &newer_than, track))
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    let Some(update) = update else { return Ok(()) };
    set(handle, |s| {
        s.phase = Phase::Downloading;
        s.version = Some(found.clone());
        s.page = Some(page);
        s.progress = Some(0);
    });
    let mut received = 0usize;
    let mut shown = 0u8;
    let progress_handle = handle.clone();
    let bytes = update
        .download(
            move |chunk, total| {
                received += chunk;
                if let Some(total) = total.filter(|t| *t > 0) {
                    let percent = ((received as u64 * 100) / total).min(100) as u8;
                    if percent != shown {
                        shown = percent;
                        set(&progress_handle, |s| s.progress = Some(percent));
                    }
                }
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    if cfg!(windows) {
        *handle.state::<Updates>().pending.lock_or_recover() = Some((update, bytes));
    } else {
        update.install(&bytes).map_err(|e| e.to_string())?;
    }
    tracing::info!("update {found} ready");
    set(handle, |s| {
        s.phase = Phase::Ready;
        s.progress = None;
    });
    Ok(())
}

#[cfg(desktop)]
fn leave_for_installer(handle: &AppHandle) {
    crate::leave_cleanly(handle);
    crate::invite_link::step_aside();
    handle.cleanup_before_exit();
}

async fn fetch_manifest(url: &str) -> Result<Manifest, String> {
    let response = reqwest::Client::new()
        .get(url)
        .header("User-Agent", format!("Commy/{}", current_version()))
        .header("Cache-Control", "no-cache")
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .map_err(|_| "Could not reach the update server.".to_string())?;
    if !response.status().is_success() {
        return Err(format!("The update server answered {}.", response.status().as_u16()));
    }
    response.json().await.map_err(|e| format!("unreadable update manifest: {e}"))
}

pub fn restart(handle: &AppHandle) -> Result<(), String> {
    let phase = handle.state::<Updates>().status.lock_or_recover().phase;
    if phase != Phase::Ready {
        return Err("No update is ready yet.".into());
    }
    #[cfg(desktop)]
    {
        let pending = handle.state::<Updates>().pending.lock_or_recover().take();
        if let Some((update, bytes)) = pending {
            update.install(&bytes).map_err(|e| e.to_string())?;
        }
    }
    crate::relaunch::restart(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(version: &str, platforms: &[&str]) -> Manifest {
        let platforms = platforms.iter().map(|key| (key.to_string(), IgnoredAny)).collect();
        Manifest { version: version.into(), platforms }
    }

    fn offered(remote: &str, current: &str, track: Track) -> Option<String> {
        offered_version(&manifest(remote, &[]), &parse_version(current).unwrap(), track).map(|v| v.to_string())
    }

    fn best(feeds: &[(&str, &str, &[&str])], current: &str, track: Track) -> Option<(String, String)> {
        let feeds: Vec<(String, Manifest)> = feeds.iter().map(|(url, version, platforms)| (url.to_string(), manifest(version, platforms))).collect();
        best_offer(&feeds, &parse_version(current).unwrap(), track, "windows-x86_64").map(|(url, version)| (url.to_string(), version.to_string()))
    }

    #[test]
    fn a_check_at_launch_stands_in_for_the_first_timed_check() {
        let now = 1_000_000;
        let checked = |phase, ago: i64| UpdateStatus { phase, checked_at: Some(now - ago), ..UpdateStatus::default() };
        assert!(check_due(&UpdateStatus::default(), now));
        assert!(!check_due(&checked(Phase::UpToDate, FIRST_CHECK.as_secs() as i64), now));
        assert!(check_due(&checked(Phase::UpToDate, CHECK_EVERY.as_secs() as i64), now));
        assert!(check_due(&checked(Phase::Failed, 1), now));
        assert!(!check_due(&checked(Phase::Ready, CHECK_EVERY.as_secs() as i64), now));
        assert!(!check_due(&UpdateStatus { phase: Phase::Downloading, ..UpdateStatus::default() }, now));
    }

    #[test]
    fn turning_auto_update_off_still_looks_for_new_versions() {
        assert_eq!(Mode::when_auto_update(true), Mode::Install);
        assert_eq!(Mode::when_auto_update(false), Mode::LookOnly);
    }

    #[test]
    fn updates_must_carry_the_version_they_were_signed_for() {
        let config: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["plugins"]["updater"]["requireSignedVersion"], serde_json::Value::Bool(true));
    }

    #[test]
    fn windows_updates_install_without_showing_the_installer() {
        let config: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["plugins"]["updater"]["windows"]["installMode"], "quiet");
    }

    #[test]
    fn versions_parse_with_or_without_v() {
        assert_eq!(parse_version("v0.2.0-rc.1").unwrap().to_string(), "0.2.0-rc.1");
        assert_eq!(parse_version("0.2.0").unwrap().to_string(), "0.2.0");
        assert!(parse_version("latest").is_none());
    }

    #[test]
    fn pre_releases_order_before_their_final() {
        let rc = parse_version("0.2.0-rc.9").unwrap();
        assert!(rc < parse_version("0.2.0").unwrap());
        assert!(parse_version("0.2.0-rc.10").unwrap() > rc);
    }

    #[test]
    fn each_build_reads_its_own_channel() {
        assert_eq!(Track::pick(false, UpdateChannel::Stable).feeds(), &["stable"]);
        assert_eq!(Track::pick(false, UpdateChannel::Preview).feeds(), &["nightly", "stable"]);
        assert_eq!(Track::pick(true, UpdateChannel::Preview).feeds(), &["rc"]);
        assert_eq!(manifest_url("https://releases.commy.chat", "nightly"), "https://releases.commy.chat/nightly/latest.json");
    }

    #[test]
    fn a_trailing_slash_on_the_releases_url_is_ignored() {
        assert_eq!(manifest_url("https://mirror.example/", "stable"), "https://mirror.example/stable/latest.json");
    }

    #[test]
    fn test_builds_take_newer_pre_releases_only() {
        assert_eq!(offered("0.2.0-rc.6", "0.2.0-rc.5", Track::Test).as_deref(), Some("0.2.0-rc.6"));
        assert_eq!(offered("0.3.0", "0.2.0-rc.5", Track::Test), None);
    }

    #[test]
    fn final_builds_take_newer_finals_only() {
        assert_eq!(offered("0.2.1", "0.2.0", Track::Stable).as_deref(), Some("0.2.1"));
        assert_eq!(offered("0.3.0-rc.1", "0.2.0", Track::Stable), None);
        assert_eq!(offered("1.0.1-nightly.20261118.42", "1.0.0", Track::Stable), None);
    }

    #[test]
    fn nothing_newer_means_no_update() {
        assert_eq!(offered("0.2.0", "0.2.0", Track::Stable), None);
        assert_eq!(offered("0.1.9", "0.2.0", Track::Stable), None);
    }

    #[test]
    fn an_unreadable_manifest_version_offers_nothing() {
        assert_eq!(offered("latest", "0.2.0", Track::Stable), None);
    }

    #[test]
    fn the_signed_manifest_parses_for_its_version() {
        let body = r#"{"version":"0.2.1","notes":"Commy 0.2.1","pub_date":"2026-11-06T20:00:00Z","platforms":{"windows-x86_64":{"url":"https://releases.commy.chat/v0.2.1/Commy_0.2.1_x64-setup.exe","signature":"sig"}}}"#;
        let manifest: Manifest = serde_json::from_str(body).unwrap();
        assert_eq!(offered_version(&manifest, &parse_version("0.2.0").unwrap(), Track::Stable).unwrap().to_string(), "0.2.1");
        assert!(manifest.serves("windows-x86_64"));
        assert!(!manifest.serves("darwin-aarch64"));
    }

    #[test]
    fn installer_specific_platform_keys_still_serve_their_platform() {
        assert!(manifest("1.0.0", &["linux-x86_64-appimage"]).serves("linux-x86_64"));
        assert!(!manifest("1.0.0", &["linux-x86_64-appimage"]).serves("linux-x86"));
    }

    #[test]
    fn preview_takes_nightlies_and_finals_but_not_release_candidates() {
        assert_eq!(offered("1.0.1-nightly.20261118.42", "1.0.0", Track::Preview).as_deref(), Some("1.0.1-nightly.20261118.42"));
        assert_eq!(offered("1.0.1-nightly.20261119.43", "1.0.1-nightly.20261118.42", Track::Preview).as_deref(), Some("1.0.1-nightly.20261119.43"));
        assert_eq!(offered("1.0.1", "1.0.1-nightly.20261118.42", Track::Preview).as_deref(), Some("1.0.1"));
        assert_eq!(offered("1.0.1-rc.1", "1.0.0", Track::Preview), None);
    }

    #[test]
    fn preview_picks_the_newer_of_the_nightly_and_stable_feeds() {
        let windows: &[&str] = &["windows-x86_64"];
        let nightly = ("nightly", "1.0.1-nightly.20261118.42", windows);
        assert_eq!(best(&[nightly, ("stable", "1.0.0", windows)], "1.0.0", Track::Preview), Some(("nightly".into(), "1.0.1-nightly.20261118.42".into())));
        assert_eq!(best(&[nightly, ("stable", "1.0.1", windows)], "1.0.0", Track::Preview), Some(("stable".into(), "1.0.1".into())));
    }

    #[test]
    fn a_nightly_without_this_platform_leaves_preview_on_stable() {
        let mac_only: &[&str] = &["darwin-aarch64"];
        let windows: &[&str] = &["windows-x86_64"];
        assert_eq!(best(&[("nightly", "1.0.1-nightly.20261118.42", mac_only), ("stable", "1.0.0", windows)], "0.2.4", Track::Preview), Some(("stable".into(), "1.0.0".into())));
    }

    #[test]
    fn switching_back_to_stable_waits_for_a_newer_stable_instead_of_going_back() {
        let windows: &[&str] = &["windows-x86_64"];
        let current = "1.0.1-nightly.20261118.42";
        assert_eq!(best(&[("stable", "1.0.0", windows)], current, Track::Stable), None);
        assert_eq!(best(&[("stable", "1.0.1", windows)], current, Track::Stable), Some(("stable".into(), "1.0.1".into())));
    }

    #[test]
    fn the_channel_setting_reads_old_settings_as_stable() {
        assert_eq!(serde_json::from_str::<UpdateChannel>(r#""preview""#).unwrap(), UpdateChannel::Preview);
        assert_eq!(UpdateChannel::default(), UpdateChannel::Stable);
    }
}
