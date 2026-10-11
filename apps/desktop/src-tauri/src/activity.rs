//! Activity presence: what game or app is in the foreground, shown to
//! peers as "Playing X" or "Using X" unless the user turns it off.

/// Processes that are part of the desktop itself, never an activity.
const IGNORED_EXES: &[&str] = &[
    "commy-desktop.exe",
    "explorer.exe",
    "searchhost.exe",
    "searchapp.exe",
    "startmenuexperiencehost.exe",
    "shellexperiencehost.exe",
    "applicationframehost.exe",
    "textinputhost.exe",
    "lockapp.exe",
    "dwm.exe",
    "msedgewebview2.exe",
    "rundll32.exe",
    "consent.exe",
];

/// Install folders that mean the foreground app is a game.
const GAME_PATH_MARKERS: &[&str] = &[
    "\\steamapps\\common\\",
    "\\epic games\\",
    "\\riot games\\",
    "\\xboxgames\\",
    "\\gog galaxy\\games\\",
    "\\gog games\\",
    "\\ea games\\",
    "\\ubisoft game launcher\\games\\",
    "\\battle.net\\games\\",
    "\\minecraft launcher\\",
];

/// Longest activity text sent to peers.
const MAX_LEN: usize = 64;

/// Turns a foreground process into activity text. `description` is the
/// exe's FileDescription (its friendly name) when available.
pub fn describe(exe_path: &str, description: Option<&str>) -> Option<String> {
    let lower = exe_path.to_ascii_lowercase().replace('/', "\\");
    let exe = lower.rsplit('\\').next().unwrap_or(&lower);
    if exe.is_empty() || IGNORED_EXES.contains(&exe) {
        return None;
    }
    let friendly = description
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| title_from_exe(exe_path));
    if friendly.is_empty() {
        return None;
    }
    let verb = if GAME_PATH_MARKERS.iter().any(|m| lower.contains(m)) {
        "Playing"
    } else {
        "Using"
    };
    let mut text = format!("{verb} {friendly}");
    if text.len() > MAX_LEN {
        let mut cut = MAX_LEN;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
    Some(text)
}

/// "C:\\Games\\hollow_knight.exe" becomes "Hollow Knight".
fn title_from_exe(exe_path: &str) -> String {
    let file = exe_path.rsplit(['\\', '/']).next().unwrap_or(exe_path);
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    stem.split(['_', '-', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Process id of the foreground window.
#[cfg(windows)]
pub fn foreground_pid() -> Option<u32> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd == 0 {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        (pid != 0).then_some(pid)
    }
}

#[cfg(not(windows))]
pub fn foreground_pid() -> Option<u32> {
    None
}

/// The foreground window's process, as (exe path, friendly name).
#[cfg(windows)]
pub fn foreground_app() -> Option<(String, Option<String>)> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd == 0 {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return None;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process == 0 {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
        CloseHandle(process);
        if ok == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        let description = file_description(&buf[..len as usize]);
        Some((path, description))
    }
}

#[cfg(not(windows))]
pub fn foreground_app() -> Option<(String, Option<String>)> {
    None
}

/// Reads the FileDescription string from an exe's version resource.
#[cfg(windows)]
fn file_description(path_utf16: &[u16]) -> Option<String> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let mut path: Vec<u16> = path_utf16.to_vec();
    path.push(0);
    unsafe {
        let size = GetFileVersionInfoSizeW(path.as_ptr(), std::ptr::null_mut());
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(path.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 {
            return None;
        }
        // First language and code page listed by the resource.
        let mut ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        let query = wide("\\VarFileInfo\\Translation");
        if VerQueryValueW(data.as_ptr().cast(), query.as_ptr(), &mut ptr, &mut len) == 0 || len < 4 {
            return None;
        }
        let lang = *(ptr as *const u16);
        let codepage = *(ptr as *const u16).add(1);
        let key = wide(&format!("\\StringFileInfo\\{lang:04x}{codepage:04x}\\FileDescription"));
        if VerQueryValueW(data.as_ptr().cast(), key.as_ptr(), &mut ptr, &mut len) == 0 || len == 0 {
            return None;
        }
        let text = std::slice::from_raw_parts(ptr as *const u16, len as usize);
        let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
        let s = String::from_utf16_lossy(&text[..end]).trim().to_string();
        (!s.is_empty()).then_some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn games_are_playing() {
        assert_eq!(
            describe("C:\\Program Files (x86)\\Steam\\steamapps\\common\\Hades\\Hades.exe", Some("Hades")),
            Some("Playing Hades".to_string())
        );
        assert_eq!(
            describe("D:\\Riot Games\\VALORANT\\live\\VALORANT.exe", Some("VALORANT")),
            Some("Playing VALORANT".to_string())
        );
    }

    #[test]
    fn other_apps_are_using() {
        assert_eq!(
            describe("C:\\Users\\r\\AppData\\Local\\Programs\\Microsoft VS Code\\Code.exe", Some("Visual Studio Code")),
            Some("Using Visual Studio Code".to_string())
        );
    }

    #[test]
    fn desktop_shell_and_commy_are_ignored() {
        assert_eq!(describe("C:\\Windows\\explorer.exe", Some("Windows Explorer")), None);
        assert_eq!(describe("C:\\dev\\commy\\target\\debug\\commy-desktop.exe", None), None);
        assert_eq!(describe("C:\\Windows\\SystemApps\\SearchHost.exe", None), None);
    }

    #[test]
    fn falls_back_to_a_tidy_exe_name() {
        assert_eq!(
            describe("C:\\Games\\hollow_knight.exe", None),
            Some("Using Hollow Knight".to_string())
        );
        assert_eq!(describe("C:\\x\\tool.exe", Some("   ")), Some("Using Tool".to_string()));
    }

    #[test]
    fn long_names_are_truncated_on_char_boundaries() {
        let name = "é".repeat(60);
        let text = describe("C:\\x\\a.exe", Some(&name)).unwrap();
        assert!(text.len() <= MAX_LEN);
        assert!(text.starts_with("Using "));
    }
}
