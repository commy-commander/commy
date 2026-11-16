const IGNORED_EXES: &[&str] = &[
    "commy-desktop.exe",
    "commy.exe",
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
    "pickerhost.exe",
    "openwith.exe",
    "dllhost.exe",
    "credentialuibroker.exe",
    "runtimebroker.exe",
    "sihost.exe",
    "shellhost.exe",
    "ctfmon.exe",
    "logonui.exe",
    "smartscreen.exe",
    "werfault.exe",
    "screenclippinghost.exe",
    "searchui.exe",
];

const GAME_LIBRARIES: &[&str] = &[
    "\\steamapps\\common\\",
    "\\epic games\\",
    "\\riot games\\",
    "\\xboxgames\\",
    "\\gog galaxy\\games\\",
    "\\gog games\\",
    "\\ea games\\",
    "\\ubisoft game launcher\\games\\",
    "\\battle.net\\games\\",
];

const OTHER_GAME_PATHS: &[&str] = &["\\minecraft launcher\\"];

const LIBRARY_TOOL_FOLDERS: &[&str] = &[
    "launcher",
    "epic online services",
    "riot client",
    "steamworks shared",
    "directx",
    "directxredist",
];

const BUILD_WORDS: &[&str] = &["shipping", "win64", "win32", "wingdk", "x64", "x86", "dx11", "dx12", "d3d11", "d3d12", "vulkan"];

const GAME_PART_WORDS: &[&str] = &["launcher", "client"];

const GLUED_BUILD_WORDS: &[&str] = &["Shipping"];

const GLUED_GAME_PART_WORDS: &[&str] = &["Launcher", "Client"];

const ENGINE_DESCRIPTIONS: &[&str] = &["bootstrappackagedgame", "unrealgame", "ue4game", "ue5game", "unity", "unityplayer"];

const MAX_LEN: usize = 64;

pub fn is_own_exe(exe_path: &str) -> bool {
    let Ok(own) = std::env::current_exe() else { return false };
    let normalize = |p: &str| p.to_ascii_lowercase().replace('/', "\\");
    normalize(exe_path) == normalize(&own.to_string_lossy())
}

pub fn describe(exe_path: &str, description: Option<&str>) -> Option<String> {
    let lower = exe_path.to_ascii_lowercase().replace('/', "\\");
    let exe = lower.rsplit('\\').next().unwrap_or(&lower);
    if exe.is_empty() || IGNORED_EXES.contains(&exe) || is_own_exe(exe_path) {
        return None;
    }
    let folder = library_folder(exe_path);
    let in_game_path = folder.is_some() || OTHER_GAME_PATHS.iter().any(|m| lower.contains(m));
    let friendly = friendly_name(description, folder.as_deref(), in_game_path, exe_path);
    if friendly.is_empty() {
        return None;
    }
    let verb = if in_game_path { "Playing" } else { "Using" };
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

fn library_folder(exe_path: &str) -> Option<String> {
    let path = exe_path.replace('/', "\\");
    let lower = path.to_ascii_lowercase();
    let start = GAME_LIBRARIES.iter().find_map(|m| lower.find(m).map(|at| at + m.len()))?;
    let folder = path[start..].split('\\').next()?.trim();
    let is_file = folder.len() == path.len() - start;
    if folder.is_empty() || is_file || LIBRARY_TOOL_FOLDERS.contains(&folder.to_ascii_lowercase().as_str()) {
        return None;
    }
    Some(folder.to_string())
}

fn friendly_name(description: Option<&str>, folder: Option<&str>, game: bool, exe_path: &str) -> String {
    let description = description.map(str::trim).filter(|d| !d.is_empty());
    if let Some(text) = description.filter(|d| is_tidy(d, game)) {
        return text.to_string();
    }
    if let Some(folder) = folder {
        return folder.to_string();
    }
    let raw = description.map(str::to_string).unwrap_or_else(|| title_from_exe(exe_path));
    let tidy = tidy_name(&raw, game);
    if tidy.is_empty() { raw } else { tidy }
}

fn is_tidy(name: &str, game: bool) -> bool {
    let squashed: String = name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect();
    !ENGINE_DESCRIPTIONS.contains(&squashed.as_str()) && tidy_name(name, game) == name
}

fn tidy_name(name: &str, game: bool) -> String {
    let separators = ['_', '-', ' '];
    let trailing = |word: &str| {
        let lower = word.to_ascii_lowercase();
        BUILD_WORDS.contains(&lower.as_str()) || (game && GAME_PART_WORDS.contains(&lower.as_str()))
    };
    let mut text = name.trim_matches(separators);
    while let Some((head, word)) = text.rsplit_once(separators) {
        let head = head.trim_end_matches(separators);
        if head.is_empty() || !trailing(word) {
            break;
        }
        text = head;
    }
    let glued = GLUED_BUILD_WORDS.iter().chain(if game { GLUED_GAME_PART_WORDS } else { &[] });
    for suffix in glued {
        let Some(stem) = text.strip_suffix(suffix) else { continue };
        let joined = stem.chars().last().is_some_and(|c| c.is_lowercase() || c.is_ascii_digit());
        let word_start = stem.rfind(separators).map_or(0, |at| at + 1);
        if joined && stem.len() - word_start > 1 {
            text = stem;
            break;
        }
    }
    text.to_string()
}

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

#[cfg(windows)]
pub fn foreground_pid() -> Option<u32> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
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

#[cfg(windows)]
pub fn process_running(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    const STILL_ACTIVE: u32 = 259;
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(process, &mut code);
        CloseHandle(process);
        ok != 0 && code == STILL_ACTIVE
    }
}

#[cfg(not(windows))]
pub fn process_running(_pid: u32) -> bool {
    false
}

pub fn game_name(activity: &str) -> Option<&str> {
    activity.strip_prefix("Playing ")
}

pub fn shown_activity(front: Option<String>, held_game: Option<&str>) -> Option<String> {
    if front.as_deref().is_some_and(|text| game_name(text).is_some()) {
        return front;
    }
    held_game.map(|game| format!("Playing {game}")).or(front)
}

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
        if hwnd.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return None;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
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
        assert_eq!(describe("C:\\Users\\x\\CommyBench\\B\\Commy.exe", Some("Commy")), None);
        let own = std::env::current_exe().unwrap().to_string_lossy().to_string();
        assert!(is_own_exe(&own));
        assert!(is_own_exe(&own.to_ascii_uppercase()));
        assert_eq!(describe(&own, Some("Some Name")), None);
        assert_eq!(describe("C:\\Windows\\SystemApps\\SearchHost.exe", None), None);
    }

    #[test]
    fn system_dialogs_and_shell_helpers_are_ignored() {
        assert_eq!(describe("C:\\Windows\\System32\\PickerHost.exe", Some("File Picker UI Host")), None);
        assert_eq!(describe("C:\\Windows\\System32\\OpenWith.exe", Some("Pick an app")), None);
        assert_eq!(describe("C:\\Windows\\System32\\DllHost.exe", Some("COM Surrogate")), None);
        assert_eq!(
            describe("C:\\Windows\\System32\\CredentialUIBroker.exe", Some("Credential Manager UI Host")),
            None
        );
        assert_eq!(
            describe("C:\\Windows\\SystemApps\\ShellExperienceHost_cw5n1h2txyewy\\ShellExperienceHost.exe", None),
            None
        );
    }

    #[test]
    fn engine_build_names_become_the_game_name() {
        assert_eq!(
            describe(
                "D:\\SteamLibrary\\steamapps\\common\\Wardogs\\WardogsLauncher\\Binaries\\Win64\\WardogsLauncher-Win64-Shipping.exe",
                Some("WardogsLauncher Shipping")
            ),
            Some("Playing Wardogs".to_string())
        );
        assert_eq!(
            describe("C:\\Games\\WardogsLauncher-Win64-Shipping.exe", Some("WardogsLauncher Shipping")),
            Some("Using WardogsLauncher".to_string())
        );
        assert_eq!(
            describe(
                "C:\\Program Files\\Epic Games\\Fortnite\\FortniteGame\\Binaries\\Win64\\FortniteClient-Win64-Shipping.exe",
                None
            ),
            Some("Playing Fortnite".to_string())
        );
        assert_eq!(
            describe(
                "C:\\Program Files (x86)\\Steam\\steamapps\\common\\Deep Rock Galactic\\FSD\\Binaries\\Win64\\FSD-Win64-Shipping.exe",
                Some("BootstrapPackagedGame")
            ),
            Some("Playing Deep Rock Galactic".to_string())
        );
    }

    #[test]
    fn a_tidy_description_beats_the_folder_name() {
        assert_eq!(
            describe(
                "C:\\Program Files (x86)\\Steam\\steamapps\\common\\Counter-Strike Global Offensive\\game\\bin\\win64\\cs2.exe",
                Some("Counter-Strike 2")
            ),
            Some("Playing Counter-Strike 2".to_string())
        );
    }

    #[test]
    fn store_launchers_are_not_games() {
        assert_eq!(
            describe(
                "C:\\Program Files (x86)\\Epic Games\\Launcher\\Portal\\Binaries\\Win64\\EpicGamesLauncher.exe",
                Some("Epic Games Launcher")
            ),
            Some("Using Epic Games Launcher".to_string())
        );
        assert_eq!(
            describe("C:\\Riot Games\\Riot Client\\RiotClientServices.exe", Some("Riot Client")),
            Some("Using Riot Client".to_string())
        );
    }

    #[test]
    fn tidy_names_keep_real_words() {
        assert_eq!(tidy_name("WardogsLauncher Shipping", true), "Wardogs");
        assert_eq!(tidy_name("WardogsLauncher Shipping", false), "WardogsLauncher");
        assert_eq!(tidy_name("Epic Games Launcher", false), "Epic Games Launcher");
        assert_eq!(tidy_name("Launcher", true), "Launcher");
        assert_eq!(tidy_name("Hollow Knight", true), "Hollow Knight");
        assert_eq!(tidy_name("RocketLeague", true), "RocketLeague");
        assert_eq!(tidy_name("Cyberpunk2077", true), "Cyberpunk2077");
        assert_eq!(tidy_name("League of Legends Client", true), "League of Legends");
        assert_eq!(tidy_name("Tool-Win64-Shipping", false), "Tool");
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
    fn a_running_game_stays_shown_while_another_app_is_in_front() {
        assert_eq!(
            shown_activity(Some("Using Firefox".to_string()), Some("Hades")),
            Some("Playing Hades".to_string())
        );
        assert_eq!(shown_activity(None, Some("Hades")), Some("Playing Hades".to_string()));
    }

    #[test]
    fn the_game_in_front_wins_over_a_held_game() {
        assert_eq!(
            shown_activity(Some("Playing VALORANT".to_string()), Some("Hades")),
            Some("Playing VALORANT".to_string())
        );
    }

    #[test]
    fn without_a_running_game_the_app_in_front_is_shown() {
        assert_eq!(
            shown_activity(Some("Using Firefox".to_string()), None),
            Some("Using Firefox".to_string())
        );
        assert_eq!(shown_activity(None, None), None);
    }

    #[test]
    fn this_process_counts_as_running() {
        assert_eq!(process_running(std::process::id()), cfg!(windows));
    }

    #[test]
    fn long_names_are_truncated_on_char_boundaries() {
        let name = "é".repeat(60);
        let text = describe("C:\\x\\a.exe", Some(&name)).unwrap();
        assert!(text.len() <= MAX_LEN);
        assert!(text.starts_with("Using "));
    }
}
