use std::path::{Path, PathBuf};

use crate::release::Problem;

pub const NO_AUTOSTART_ARG: &str = "--no-start-with-computer";

pub fn setup_file_path(version: &str) -> PathBuf {
    std::env::temp_dir().join(format!("Commy_{}_x64-setup.exe", safe_version(version)))
}

fn safe_version(version: &str) -> String {
    version.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-')).collect()
}

pub fn save(path: &Path, bytes: &[u8]) -> Result<(), Problem> {
    std::fs::write(path, bytes).map_err(|e| Problem::Disk(e.to_string()))
}

pub fn quiet_setup_args(desktop_shortcut: bool) -> Vec<&'static str> {
    let mut args = vec!["/S"];
    if !desktop_shortcut {
        args.push("/NS");
    }
    args
}

pub fn run_quietly(path: &Path, desktop_shortcut: bool) -> Result<(), Problem> {
    let status = std::process::Command::new(path)
        .args(quiet_setup_args(desktop_shortcut))
        .status()
        .map_err(|e| Problem::Setup(e.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(Problem::Setup(format!("it ended with code {}", status.code().unwrap_or(-1))))
    }
}

pub fn run_with_its_own_window(path: &Path) -> Result<(), Problem> {
    std::process::Command::new(path).spawn().map(drop).map_err(|e| Problem::Setup(e.to_string()))
}

pub fn launch_args(start_with_computer: bool) -> Vec<&'static str> {
    if start_with_computer {
        Vec::new()
    } else {
        vec![NO_AUTOSTART_ARG]
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn unquote(value: &str) -> &str {
    value.trim().trim_matches('"')
}

pub fn open_commy(start_with_computer: bool) -> Result<(), Problem> {
    let app = installed_app().ok_or_else(|| Problem::Setup("Commy was installed but could not be found".into()))?;
    std::process::Command::new(app)
        .args(launch_args(start_with_computer))
        .spawn()
        .map(drop)
        .map_err(|e| Problem::Setup(e.to_string()))
}

#[cfg(windows)]
pub fn installed_app() -> Option<PathBuf> {
    let icon = windows_registry::current_user_string(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Commy", "DisplayIcon")?;
    let path = PathBuf::from(unquote(&icon));
    path.exists().then_some(path)
}

#[cfg(not(windows))]
pub fn installed_app() -> Option<PathBuf> {
    None
}

#[cfg(windows)]
pub fn window_can_open() -> bool {
    const RUNTIME: &str = r"Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
    let machine_wide = [format!(r"SOFTWARE\WOW6432Node\{RUNTIME}"), format!(r"SOFTWARE\{RUNTIME}")];
    let found = machine_wide.iter().filter_map(|key| windows_registry::local_machine_string(key, "pv")).chain(
        windows_registry::current_user_string(&format!(r"Software\{RUNTIME}"), "pv"),
    );
    found.into_iter().any(|version| webview_version_usable(&version))
}

#[cfg(not(windows))]
pub fn window_can_open() -> bool {
    true
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn webview_version_usable(version: &str) -> bool {
    let version = version.trim();
    !version.is_empty() && version != "0.0.0.0"
}

#[cfg(windows)]
pub fn tell(text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (text, title) = (wide(text), wide("Commy Setup"));
    // SAFETY: both strings are NUL terminated UTF-16 buffers that live until MessageBoxW returns.
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONINFORMATION);
    }
}

#[cfg(not(windows))]
pub fn tell(text: &str) {
    eprintln!("{text}");
}

#[cfg(windows)]
mod windows_registry {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

    pub fn current_user_string(key: &str, value: &str) -> Option<String> {
        read_string(HKEY_CURRENT_USER, key, value)
    }

    pub fn local_machine_string(key: &str, value: &str) -> Option<String> {
        read_string(HKEY_LOCAL_MACHINE, key, value)
    }

    fn read_string(root: HKEY, key: &str, value: &str) -> Option<String> {
        let key = wide(key);
        let value = wide(value);
        let mut buffer = vec![0u16; 1024];
        let mut size = (buffer.len() * 2) as u32;
        // SAFETY: key and value are NUL terminated, and size holds the buffer's length in bytes.
        let status = unsafe {
            RegGetValueW(root, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut size)
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let len = (size as usize / 2).saturating_sub(1);
        Some(String::from_utf16_lossy(&buffer[..len.min(buffer.len())]))
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_silently_with_or_without_a_desktop_shortcut() {
        assert_eq!(quiet_setup_args(true), vec!["/S"]);
        assert_eq!(quiet_setup_args(false), vec!["/S", "/NS"]);
    }

    #[test]
    fn tells_commy_not_to_start_with_the_computer_only_when_asked() {
        assert!(launch_args(true).is_empty());
        assert_eq!(launch_args(false), vec![NO_AUTOSTART_ARG]);
    }

    #[test]
    fn reads_the_installed_path_from_a_quoted_registry_value() {
        assert_eq!(unquote("\"C:\\Users\\x\\AppData\\Local\\Commy\\Commy.exe\""), "C:\\Users\\x\\AppData\\Local\\Commy\\Commy.exe");
    }

    #[test]
    fn keeps_odd_characters_out_of_the_setup_file_name() {
        let path = setup_file_path("1.0.2/../../x");
        assert!(path.ends_with("Commy_1.0.2....x_x64-setup.exe"));
    }

    #[test]
    fn treats_an_empty_webview_version_as_missing() {
        assert!(webview_version_usable("131.0.2903.70"));
        assert!(!webview_version_usable("0.0.0.0"));
        assert!(!webview_version_usable(" "));
    }
}
