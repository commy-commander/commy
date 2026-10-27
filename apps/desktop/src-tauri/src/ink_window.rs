#![cfg_attr(mobile, allow(dead_code))]

use commy_media::screen::SourceKind;
#[cfg(desktop)]
use commy_media::screen::SourceRect;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::AppHandle;
#[cfg(desktop)]
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "stream-ink";
const FOLLOW_EVERY: std::time::Duration = std::time::Duration::from_millis(750);

static TARGET: Mutex<Option<(SourceKind, u32)>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);

#[cfg(mobile)]
pub fn show(_app: &AppHandle, _source: Option<(SourceKind, u32)>) {}

#[cfg(desktop)]
pub fn show(app: &AppHandle, source: Option<(SourceKind, u32)>) {
    let Some(source) = source else { return hide(app) };
    if *TARGET.lock().unwrap() == Some(source) {
        return;
    }
    *TARGET.lock().unwrap() = Some(source);
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut last: Option<SourceRect> = None;
        let mut first = true;
        while GENERATION.load(Ordering::SeqCst) == generation {
            let rect = tauri::async_runtime::spawn_blocking(move || commy_media::screen::source_rect(source.0, source.1))
                .await
                .ok()
                .flatten();
            if first || rect != last {
                first = false;
                last = rect;
                let app = app.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || place(&app, rect)).await;
            }
            tokio::time::sleep(FOLLOW_EVERY).await;
        }
    });
}

pub fn hide(app: &AppHandle) {
    *TARGET.lock().unwrap() = None;
    GENERATION.fetch_add(1, Ordering::SeqCst);
    #[cfg(desktop)]
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = app.emit_to(LABEL, "stream-ink-reset", ());
        let _ = window.hide();
    }
    #[cfg(mobile)]
    let _ = app;
}

#[cfg(desktop)]
fn ensure_window(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    if let Some(window) = app.get_webview_window(LABEL) {
        return Ok(window);
    }
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("ink.html".into()))
        .title("Commy Drawings")
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        .focusable(false)
        .content_protected(true)
        .visible(false)
        .build()
}

#[cfg(desktop)]
fn place(app: &AppHandle, rect: Option<SourceRect>) {
    let Some(rect) = rect else {
        if let Some(window) = app.get_webview_window(LABEL) {
            let _ = window.hide();
        }
        return;
    };
    let window = match ensure_window(app) {
        Ok(window) => window,
        Err(e) => {
            tracing::warn!("could not create the drawing window: {e}");
            return;
        }
    };
    #[cfg(target_os = "macos")]
    {
        let _ = window.set_size(tauri::LogicalSize::new(rect.width as f64, rect.height as f64));
        let _ = window.set_position(tauri::LogicalPosition::new(rect.x as f64, rect.y as f64));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window.set_size(tauri::PhysicalSize::new(rect.width, rect.height));
        let _ = window.set_position(tauri::PhysicalPosition::new(rect.x, rect.y));
    }
    let _ = window.set_ignore_cursor_events(true);
    show_without_focus(app, &window);
}

#[cfg(windows)]
fn show_without_focus(app: &AppHandle, window: &tauri::WebviewWindow) {
    let Ok(hwnd) = window.hwnd() else { return };
    let hwnd = hwnd.0 as isize;
    let _ = app.run_on_main_thread(move || {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE, HWND_TOPMOST, SWP_NOACTIVATE,
            SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_SHOWNOACTIVATE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
            WS_EX_TRANSPARENT,
        };
        let hwnd = hwnd as windows_sys::Win32::Foundation::HWND;
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let wanted = (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT) as isize;
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | wanted);
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
    });
}

#[cfg(all(desktop, not(windows)))]
fn show_without_focus(_app: &AppHandle, window: &tauri::WebviewWindow) {
    let _ = window.show();
}
