use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

pub const VERSION: Option<&str> = option_env!("COMMY_VERSION");
pub const REPO: &str = match option_env!("COMMY_UPDATE_REPO") {
    Some(repo) => repo,
    None => "commy-commander/commy",
};
const MANIFEST: &str = "latest.json";
const FIRST_CHECK: Duration = Duration::from_secs(30);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
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
    pending: Mutex<Option<(tauri_plugin_updater::Update, Vec<u8>)>>,
}

#[derive(Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

pub fn parse_version(tag: &str) -> Option<semver::Version> {
    semver::Version::parse(tag.trim().trim_start_matches('v')).ok()
}

pub fn pick_release<'a>(
    releases: &'a [Release],
    current: &semver::Version,
    test_build: bool,
) -> Option<(&'a Release, semver::Version, &'a str)> {
    releases
        .iter()
        .filter(|r| !r.draft && r.prerelease == test_build)
        .filter_map(|r| {
            let version = parse_version(&r.tag_name)?;
            let manifest = r.assets.iter().find(|a| a.name == MANIFEST)?;
            Some((r, version, manifest.browser_download_url.as_str()))
        })
        .filter(|(_, version, _)| version > current)
        .max_by(|a, b| a.1.cmp(&b.1))
}

fn install_blocker(handle: &AppHandle) -> Option<&'static str> {
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

fn current_version() -> String {
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
            if handle.state::<crate::App>().voice.lock().unwrap().auto_update {
                check(&handle).await;
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
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
    let releases = fetch_releases().await?;
    let Some((release, version, manifest)) = pick_release(&releases, &current, crate::instance::TEST_BUILD) else {
        return Ok(());
    };
    let found = version.to_string();
    let page = release.html_url.clone();
    let can_install = handle.state::<Updates>().status.lock().unwrap().can_install;
    if !can_install {
        set(handle, |s| {
            s.phase = Phase::Available;
            s.version = Some(found);
            s.page = Some(page);
        });
        return Ok(());
    }
    let url = manifest.parse().map_err(|e| format!("bad manifest link: {e}"))?;
    let newer_than = current.clone();
    let update = handle
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .version_comparator(move |_, remote| remote.version > newer_than)
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

async fn fetch_releases() -> Result<Vec<Release>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=30");
    let response = reqwest::Client::new()
        .get(url)
        .header("User-Agent", format!("Commy/{}", current_version()))
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| "Could not reach the update server.".to_string())?;
    if !response.status().is_success() {
        return Err(format!("The update server answered {}.", response.status().as_u16()));
    }
    response.json().await.map_err(|e| format!("unreadable release list: {e}"))
}

pub fn restart(handle: &AppHandle) -> Result<(), String> {
    let phase = handle.state::<Updates>().status.lock().unwrap().phase;
    if phase != Phase::Ready {
        return Err("No update is ready yet.".into());
    }
    let pending = handle.state::<Updates>().pending.lock().unwrap().take();
    if let Some((update, bytes)) = pending {
        update.install(&bytes).map_err(|e| e.to_string())?;
    }
    handle.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool, manifest: bool) -> Release {
        Release {
            tag_name: tag.into(),
            draft: false,
            prerelease,
            html_url: format!("https://example.test/{tag}"),
            assets: if manifest {
                vec![Asset { name: MANIFEST.into(), browser_download_url: format!("https://example.test/{tag}/latest.json") }]
            } else {
                vec![]
            },
        }
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
    fn test_builds_follow_only_pre_releases() {
        let releases = vec![release("v0.3.0", false, true), release("v0.2.0-rc.6", true, true), release("v0.2.0-rc.5", true, true)];
        let current = parse_version("0.2.0-rc.5").unwrap();
        let (picked, version, manifest) = pick_release(&releases, &current, true).unwrap();
        assert_eq!(picked.tag_name, "v0.2.0-rc.6");
        assert_eq!(version.to_string(), "0.2.0-rc.6");
        assert!(manifest.ends_with("/v0.2.0-rc.6/latest.json"));
    }

    #[test]
    fn final_builds_follow_only_finals() {
        let releases = vec![release("v0.3.0-rc.1", true, true), release("v0.2.1", false, true)];
        let current = parse_version("0.2.0").unwrap();
        assert_eq!(pick_release(&releases, &current, false).unwrap().0.tag_name, "v0.2.1");
    }

    #[test]
    fn nothing_newer_means_no_update() {
        let releases = vec![release("v0.2.0", false, true), release("v0.1.9", false, true)];
        assert!(pick_release(&releases, &parse_version("0.2.0").unwrap(), false).is_none());
    }

    #[test]
    fn releases_without_a_manifest_and_drafts_are_skipped() {
        let mut draft = release("v0.4.0", false, true);
        draft.draft = true;
        let releases = vec![draft, release("v0.3.0", false, false), release("v0.2.1", false, true)];
        let current = parse_version("0.2.0").unwrap();
        assert_eq!(pick_release(&releases, &current, false).unwrap().0.tag_name, "v0.2.1");
    }
}
