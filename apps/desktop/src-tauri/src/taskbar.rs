use std::path::Path;
use std::sync::atomic::{AtomicPtr, Ordering};

use image::RgbaImage;
use tauri::{AppHandle, Manager};

use crate::state::App;

static OUR_BIG_ICON: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

pub fn show(handle: &AppHandle, chosen: Option<&RgbaImage>) {
    let Some(base) = handle.state::<App>().base_dir().cloned() else { return };
    let file = crate::app_icon::save_taskbar_icon(&base, chosen);
    let main = handle.clone();
    let _ = handle.run_on_main_thread(move || {
        crate::toast::ensure_start_menu_shortcut(file.as_deref());
        let Some(window) = main.get_webview_window("main") else { return };
        let Ok(hwnd) = window.hwnd() else { return };
        let hwnd = hwnd.0 as windows_sys::Win32::Foundation::HWND;
        set_big_icon(hwnd, file.as_deref());
        if let Err(e) = set_relaunch_icon(hwnd, file.as_deref()) {
            tracing::warn!("could not give the taskbar button its icon: {e}");
        }
    });
}

fn set_big_icon(hwnd: windows_sys::Win32::Foundation::HWND, file: Option<&Path>) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DestroyIcon, GetSystemMetrics, LoadImageW, SendMessageW, ICON_BIG, IMAGE_ICON, LR_LOADFROMFILE, SM_CXICON,
        SM_CYICON, WM_SETICON,
    };
    let icon = file.map_or(std::ptr::null_mut(), |path| {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        unsafe {
            LoadImageW(
                std::ptr::null_mut(),
                wide.as_ptr(),
                IMAGE_ICON,
                GetSystemMetrics(SM_CXICON),
                GetSystemMetrics(SM_CYICON),
                LR_LOADFROMFILE,
            )
        }
    });
    unsafe { SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, icon as isize) };
    let previous = OUR_BIG_ICON.swap(icon, Ordering::SeqCst);
    if !previous.is_null() {
        unsafe { DestroyIcon(previous) };
    }
}

fn set_relaunch_icon(hwnd: windows_sys::Win32::Foundation::HWND, file: Option<&Path>) -> windows::core::Result<()> {
    use windows::core::PROPVARIANT;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Storage::EnhancedStorage::{
        PKEY_AppUserModel_ID, PKEY_AppUserModel_RelaunchCommand, PKEY_AppUserModel_RelaunchDisplayNameResource,
        PKEY_AppUserModel_RelaunchIconResource,
    };
    use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};

    let Ok(exe) = std::env::current_exe() else { return Ok(()) };
    let icon = format!("{},0", file.unwrap_or(&exe).display());
    let command = format!("\"{}\"", exe.display());
    unsafe {
        let store: IPropertyStore = SHGetPropertyStoreForWindow(HWND(hwnd))?;
        store.SetValue(&PKEY_AppUserModel_ID, &PROPVARIANT::from(crate::toast::APP_ID))?;
        store.SetValue(&PKEY_AppUserModel_RelaunchCommand, &PROPVARIANT::from(command.as_str()))?;
        store.SetValue(&PKEY_AppUserModel_RelaunchDisplayNameResource, &PROPVARIANT::from("Commy"))?;
        store.SetValue(&PKEY_AppUserModel_RelaunchIconResource, &PROPVARIANT::from(icon.as_str()))?;
        store.Commit()?;
    }
    Ok(())
}
