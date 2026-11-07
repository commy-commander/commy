use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateStatus {
    pub current: String,
    pub can_install: bool,
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
}

pub fn parse_version(tag: &str) -> Option<semver::Version> {
    semver::Version::parse(tag.trim().trim_start_matches('v')).ok()
}

pub fn channel(test_build: bool) -> &'static str {
    if test_build {
        TEST_CHANNEL
    } else {
        STABLE_CHANNEL
    }
}

pub fn manifest_url(releases: &str, test_build: bool) -> String {
    format!("{}/{}/{MANIFEST}", releases.trim_end_matches('/'), channel(test_build))
}

pub fn is_offered(remote: &semver::Version, current: &semver::Version, test_build: bool) -> bool {
    let is_pre_release = !remote.pre.is_empty();
    remote > current && is_pre_release == test_build
}

pub fn offered_version(manifest: &Manifest, current: &semver::Version, test_build: bool) -> Option<semver::Version> {
    parse_version(&manifest.version).filter(|remote| is_offered(remote, current, test_build))
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
    let mut status = updates.status.lock().unwrap();
    status.current = current_version();
    status.can_install = reason.is_none();
    status.reason = reason.map(str::to_string);
}

pub fn status(handle: &AppHandle) -> UpdateStatus {
    handle.state::<Updates>().status.lock().unwrap().clone()
}

fn set(handle: &AppHandle, change: impl FnOnce(&mut UpdateStatus)) {
    let status = {
        let updates = handle.state::<Updates>();
        let mut status = updates.status.lock().unwrap();
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
            let auto = handle.state::<crate::App>().voice.lock().unwrap().auto_update;
            if auto && check_due(&status(&handle), chrono::Utc::now().timestamp()) {
                check(&handle).await;
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

pub async fn check(handle: &AppHandle) -> UpdateStatus {
    if VERSION.is_none() {
        return status(handle);
    }
    let updates = handle.state::<Updates>();
    let phase = updates.status.lock().unwrap().phase;
    if phase == Phase::Ready || updates.busy.swap(true, Ordering::SeqCst) {
        return status(handle);
    }
    set(handle, |s| {
        s.phase = Phase::Checking;
        s.error = None;
        s.progress = None;
    });
    let result = run_check(handle).await;
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

async fn run_check(handle: &AppHandle) -> Result<(), String> {
    let Some(current) = VERSION.and_then(parse_version) else {
        return Err("This build does not update itself.".into());
    };
    let test_build = crate::instance::TEST_BUILD;
    let manifest = manifest_url(RELEASES, test_build);
    let Some(version) = offered_version(&fetch_manifest(&manifest).await?, &current, test_build) else {
        return Ok(());
    };
    let found = version.to_string();
    let page = DOWNLOAD_PAGE.to_string();
    let can_install = handle.state::<Updates>().status.lock().unwrap().can_install;
    if !can_install {
        set(handle, |s| {
            s.phase = Phase::Available;
            s.version = Some(found);
            s.page = Some(page);
        });
        return Ok(());
    }
    install(handle, &manifest, current, found, page).await
}

#[cfg(mobile)]
async fn install(_handle: &AppHandle, _manifest: &str, _current: semver::Version, _found: String, _page: String) -> Result<(), String> {
    Ok(())
}

#[cfg(desktop)]
async fn install(handle: &AppHandle, manifest: &str, current: semver::Version, found: String, page: String) -> Result<(), String> {
    let url = manifest.parse().map_err(|e| format!("bad manifest link: {e}"))?;
    let newer_than = current;
    let test_build = crate::instance::TEST_BUILD;
    let update = handle
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .timeout(FETCH_TIMEOUT)
        .version_comparator(move |_, remote| is_offered(&remote.version, &newer_than, test_build))
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
        *handle.state::<Updates>().pending.lock().unwrap() = Some((update, bytes));
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
    let phase = handle.state::<Updates>().status.lock().unwrap().phase;
    if phase != Phase::Ready {
        return Err("No update is ready yet.".into());
    }
    #[cfg(desktop)]
    {
        let pending = handle.state::<Updates>().pending.lock().unwrap().take();
        if let Some((update, bytes)) = pending {
            update.install(&bytes).map_err(|e| e.to_string())?;
        }
    }
    crate::relaunch::restart(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offered(remote: &str, current: &str, test_build: bool) -> Option<String> {
        let manifest = Manifest { version: remote.into() };
        offered_version(&manifest, &parse_version(current).unwrap(), test_build).map(|v| v.to_string())
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
        assert_eq!(manifest_url("https://releases.commy.chat", false), "https://releases.commy.chat/stable/latest.json");
        assert_eq!(manifest_url("https://releases.commy.chat", true), "https://releases.commy.chat/rc/latest.json");
    }

    #[test]
    fn a_trailing_slash_on_the_releases_url_is_ignored() {
        assert_eq!(manifest_url("https://mirror.example/", false), "https://mirror.example/stable/latest.json");
    }

    #[test]
    fn test_builds_take_newer_pre_releases_only() {
        assert_eq!(offered("0.2.0-rc.6", "0.2.0-rc.5", true).as_deref(), Some("0.2.0-rc.6"));
        assert_eq!(offered("0.3.0", "0.2.0-rc.5", true), None);
    }

    #[test]
    fn final_builds_take_newer_finals_only() {
        assert_eq!(offered("0.2.1", "0.2.0", false).as_deref(), Some("0.2.1"));
        assert_eq!(offered("0.3.0-rc.1", "0.2.0", false), None);
    }

    #[test]
    fn nothing_newer_means_no_update() {
        assert_eq!(offered("0.2.0", "0.2.0", false), None);
        assert_eq!(offered("0.1.9", "0.2.0", false), None);
    }

    #[test]
    fn an_unreadable_manifest_version_offers_nothing() {
        assert_eq!(offered("latest", "0.2.0", false), None);
    }

    #[test]
    fn the_signed_manifest_parses_for_its_version() {
        let body = r#"{"version":"0.2.1","notes":"Commy 0.2.1","pub_date":"2026-11-06T20:00:00Z","platforms":{"windows-x86_64":{"url":"https://releases.commy.chat/v0.2.1/Commy_0.2.1_x64-setup.exe","signature":"sig"}}}"#;
        let manifest: Manifest = serde_json::from_str(body).unwrap();
        assert_eq!(offered_version(&manifest, &parse_version("0.2.0").unwrap(), false).unwrap().to_string(), "0.2.1");
    }
}
