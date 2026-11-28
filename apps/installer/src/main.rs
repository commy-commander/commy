#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod release;
mod setup;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{Emitter, Manager};

use release::{Problem, Release};

const PROGRESS_EVENT: &str = "install-progress";
const PROGRESS_EVERY: Duration = Duration::from_millis(80);

#[derive(Default)]
struct Installer {
    release: Mutex<Option<Release>>,
    cancelled: Arc<AtomicBool>,
    busy: AtomicBool,
}

#[derive(Serialize)]
struct ReleaseInfo {
    version: String,
    bytes: Option<u64>,
    starts_with_computer: bool,
}

#[derive(Serialize, Clone)]
#[serde(tag = "step", rename_all = "lowercase")]
enum Progress {
    Download { received: u64, total: Option<u64> },
    Check,
    Closing,
    Install,
    Done,
    Cancelled,
    Failed { message: String },
}

#[tauri::command]
async fn release_info(state: tauri::State<'_, Installer>) -> Result<ReleaseInfo, String> {
    let client = release::client();
    let release = release::latest(&client).await.map_err(|p| p.message())?;
    let bytes = release::size(&client, &release).await;
    let version = release.version.clone();
    *state.release.lock().unwrap_or_else(|e| e.into_inner()) = Some(release);
    Ok(ReleaseInfo { version, bytes, starts_with_computer: setup::starts_with_computer_now() })
}

#[tauri::command]
fn start_install(app: tauri::AppHandle, state: tauri::State<'_, Installer>, desktop_shortcut: bool) {
    if state.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    state.cancelled.store(false, Ordering::SeqCst);
    let known = state.release.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let cancelled = state.cancelled.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = install(&app, known, &cancelled, desktop_shortcut).await;
        let last = match outcome {
            Ok(()) => Progress::Done,
            Err(Problem::Cancelled) => Progress::Cancelled,
            Err(problem) => Progress::Failed { message: problem.message() },
        };
        app.state::<Installer>().busy.store(false, Ordering::SeqCst);
        let _ = app.emit(PROGRESS_EVENT, last);
    });
}

async fn install(app: &tauri::AppHandle, known: Option<Release>, cancelled: &AtomicBool, desktop_shortcut: bool) -> Result<(), Problem> {
    let client = release::client();
    let release = match known {
        Some(release) => release,
        None => release::latest(&client).await?,
    };
    let mut last_sent: Option<Instant> = None;
    let bytes = release::download(&client, &release, cancelled, |received, total| {
        let finished = total.is_some_and(|total| received >= total);
        if finished || last_sent.is_none_or(|at| at.elapsed() >= PROGRESS_EVERY) {
            last_sent = Some(Instant::now());
            let _ = app.emit(PROGRESS_EVENT, Progress::Download { received, total });
        }
    })
    .await?;
    let _ = app.emit(PROGRESS_EVENT, Progress::Check);
    release::check_signature(&bytes, &release.signature)?;
    if cancelled.load(Ordering::SeqCst) {
        return Err(Problem::Cancelled);
    }
    let path = setup::setup_file_path(&release.version);
    setup::save(&path, &bytes)?;
    drop(bytes);
    let ran = tauri::async_runtime::spawn_blocking({
        let path = path.clone();
        let app = app.clone();
        move || {
            setup::close_running_commy(|| {
                let _ = app.emit(PROGRESS_EVENT, Progress::Closing);
            })?;
            let _ = app.emit(PROGRESS_EVENT, Progress::Install);
            setup::run_quietly(&path, desktop_shortcut)
        }
    })
    .await
    .map_err(|e| Problem::Setup(e.to_string()))?;
    let _ = std::fs::remove_file(&path);
    ran
}

#[tauri::command]
fn cancel_install(state: tauri::State<'_, Installer>) {
    state.cancelled.store(true, Ordering::SeqCst);
}

#[tauri::command]
fn open_commy(app: tauri::AppHandle, start_with_computer: bool) -> Result<(), String> {
    setup::open_commy(start_with_computer).map_err(|p| p.message())?;
    app.exit(0);
    Ok(())
}

fn install_without_window() {
    setup::tell("Commy Setup needs a part of Windows that is missing here, so the standard Commy installer will open instead. It adds that part for you.");
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => return setup::tell(&format!("Commy Setup could not start: {e}")),
    };
    let outcome = runtime.block_on(async {
        let client = release::client();
        let release = release::latest(&client).await?;
        let bytes = release::download(&client, &release, &AtomicBool::new(false), |_, _| {}).await?;
        release::check_signature(&bytes, &release.signature)?;
        let path = setup::setup_file_path(&release.version);
        setup::save(&path, &bytes)?;
        setup::run_with_its_own_window(&path)
    });
    if let Err(problem) = outcome {
        setup::tell(&problem.message());
    }
}

fn main() {
    if !setup::window_can_open() {
        install_without_window();
        return;
    }
    let ran = tauri::Builder::default()
        .manage(Installer::default())
        .invoke_handler(tauri::generate_handler![release_info, start_install, cancel_install, open_commy])
        .run(tauri::generate_context!());
    if let Err(e) = ran {
        setup::tell(&format!("Commy Setup could not open its window: {e}"));
    }
}
