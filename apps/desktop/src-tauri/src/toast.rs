use tauri::AppHandle;

#[cfg(desktop)]
pub const APP_ID: &str = "chat.commy.desktop";

pub fn valid_target(target: &str) -> bool {
    let ok = |s: &str| !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
    match target.split(':').collect::<Vec<_>>().as_slice() {
        ["channel", server, channel] => ok(server) && ok(channel),
        ["dm", user] => ok(user),
        _ => false,
    }
}

#[cfg(windows)]
pub fn register(icon_png: Option<&std::path::Path>) {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };
    use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;

    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    unsafe {
        let id = wide(APP_ID);
        SetCurrentProcessExplicitAppUserModelID(id.as_ptr());

        let path = wide(&format!("Software\\Classes\\AppUserModelId\\{APP_ID}"));
        let mut key: HKEY = std::ptr::null_mut();
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        );
        if status != 0 {
            tracing::warn!("could not register notification app id: {status}");
            return;
        }
        let mut set = |name: &str, value: &str| {
            let name = wide(name);
            let value = wide(value);
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            );
        };
        set("DisplayName", "Commy");
        if let Some(icon) = icon_png {
            set("IconUri", &icon.display().to_string());
        }
        RegCloseKey(key);
    }
}

#[cfg(windows)]
pub fn ensure_start_menu_shortcut(icon: Option<&std::path::Path>) {
    if let Err(e) = write_shortcut(icon) {
        tracing::warn!("could not create the Start menu shortcut for notifications: {e}");
    }
}

#[cfg(windows)]
fn write_shortcut(icon: Option<&std::path::Path>) -> windows::core::Result<()> {
    use windows::core::{Interface, HSTRING};
    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    let Ok(exe) = std::env::current_exe() else { return Ok(()) };
    let Some(appdata) = std::env::var_os("APPDATA") else { return Ok(()) };
    let programs = std::path::PathBuf::from(appdata).join("Microsoft\\Windows\\Start Menu\\Programs");
    let _ = std::fs::create_dir_all(&programs);
    let lnk = programs.join("Commy.lnk");
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&HSTRING::from(exe.as_os_str()))?;
        if let Some(dir) = exe.parent() {
            link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
        }
        link.SetIconLocation(&HSTRING::from(icon.unwrap_or(&exe).as_os_str()), 0)?;
        link.SetDescription(&HSTRING::from("Commy"))?;
        let store: IPropertyStore = link.cast()?;
        store.SetValue(&PKEY_AppUserModel_ID, &windows::core::PROPVARIANT::from(APP_ID))?;
        store.Commit()?;
        let file: IPersistFile = link.cast()?;
        file.Save(&HSTRING::from(lnk.as_os_str()), true)?;
    }
    tell_shell_about(&lnk);
    Ok(())
}

#[cfg(windows)]
fn tell_shell_about(path: &std::path::Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{SHChangeNotify, SHCNE_UPDATEITEM, SHCNF_PATHW};
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    unsafe { SHChangeNotify(SHCNE_UPDATEITEM as i32, SHCNF_PATHW, wide.as_ptr().cast(), std::ptr::null()) };
}

#[cfg(not(windows))]
pub fn ensure_start_menu_shortcut(_icon: Option<&std::path::Path>) {}

#[cfg(not(windows))]
pub fn register(_icon_png: Option<&std::path::Path>) {}

#[cfg(windows)]
pub fn show(handle: &AppHandle, title: &str, body: &str, target: Option<String>) -> Result<(), String> {
    use tauri::Emitter;
    use tauri_winrt_notification::Toast;
    let clicked = handle.clone();
    Toast::new(APP_ID)
        .title(title)
        .text1(body)
        .sound(None)
        .on_activated(move |_| {
            crate::tray::show_main_window(&clicked);
            if let Some(target) = &target {
                let _ = clicked.emit("notification-clicked", target);
            }
            Ok(())
        })
        .show()
        .map_err(|e| e.to_string())
}

#[cfg(not(windows))]
pub fn show(handle: &AppHandle, title: &str, body: &str, _target: Option<String>) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    handle
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_channel_and_dm_targets() {
        let id = "7da1b7e3-9818-4075-a087-1c4587540659";
        assert!(valid_target(&format!("channel:{id}:{id}")));
        assert!(valid_target(&format!("dm:{id}")));
    }

    #[test]
    fn rejects_anything_else() {
        for bad in ["", "channel:x", "dm:", "dm:abc def", "open:calc.exe", "channel:a:b:c", "dm:../../x"] {
            assert!(!valid_target(bad), "{bad}");
        }
    }
}
