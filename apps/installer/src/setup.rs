use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::release::Problem;

pub const AUTOSTART_ARG: &str = "--start-with-computer";
pub const NO_AUTOSTART_ARG: &str = "--no-start-with-computer";
pub const QUIT_REQUEST: &str = "commy-quit-v1";
const PORT_FILE: &str = "link.port";
const DATA_FOLDER: &str = "chat.commy.desktop";
const APP_FILE: &str = "commy-desktop.exe";
#[cfg_attr(not(windows), allow(dead_code))]
const RUN_VALUE: &str = "Commy";
const QUIT_WAIT: Duration = Duration::from_secs(10);

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
    vec![if start_with_computer { AUTOSTART_ARG } else { NO_AUTOSTART_ARG }]
}

pub fn close_running_commy(on_closing: impl FnOnce()) -> Result<(), Problem> {
    let Some(app) = app_to_replace() else { return Ok(()) };
    let open = processes::running(&app);
    if open.is_empty() {
        return Ok(());
    }
    on_closing();
    let asked = data_folder().is_some_and(|folder| ask_to_quit(&folder));
    let left = if asked { processes::wait_for_exit(open, QUIT_WAIT) } else { open };
    processes::end(left);
    if processes::running(&app).is_empty() {
        Ok(())
    } else {
        Err(Problem::StillOpen)
    }
}

pub fn ask_to_quit(data_folder: &Path) -> bool {
    let Some(port) = std::fs::read_to_string(data_folder.join(PORT_FILE)).ok().and_then(|s| s.trim().parse::<u16>().ok()) else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(500)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    if stream.write_all(format!("{QUIT_REQUEST}\n").as_bytes()).is_err() {
        return false;
    }
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer).is_ok() && answer.trim() == "ok"
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn same_file(a: &Path, b: &Path) -> bool {
    let plain = |path: &Path| path.to_string_lossy().replace('/', "\\").to_lowercase();
    plain(a) == plain(b)
}

fn app_to_replace() -> Option<PathBuf> {
    installed_app().or_else(|| local_app_data().map(|folder| folder.join("Commy").join(APP_FILE)))
}

fn data_folder() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|folder| PathBuf::from(folder).join(DATA_FOLDER))
}

fn local_app_data() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
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
pub fn starts_with_computer_now() -> bool {
    installed_app().is_none() || windows_registry::current_user_string(r"Software\Microsoft\Windows\CurrentVersion\Run", RUN_VALUE).is_some()
}

#[cfg(not(windows))]
pub fn starts_with_computer_now() -> bool {
    true
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
mod processes {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    };

    pub struct Running(HANDLE);

    impl Drop for Running {
        fn drop(&mut self) {
            // SAFETY: the handle came from OpenProcess and is closed only here.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    pub fn running(app: &Path) -> Vec<Running> {
        let Some(name) = app.file_name().map(|name| name.to_string_lossy().to_lowercase()) else { return Vec::new() };
        // SAFETY: the snapshot handle is checked and closed below.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Vec::new();
        }
        let mut found = Vec::new();
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        // SAFETY: entry.dwSize is set, and the snapshot stays open while it is walked.
        let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
        while more {
            let end = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
            if String::from_utf16_lossy(&entry.szExeFile[..end]).to_lowercase() == name {
                if let Some(process) = open_if_same(entry.th32ProcessID, app) {
                    found.push(process);
                }
            }
            // SAFETY: as above.
            more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
        }
        // SAFETY: the snapshot handle is valid and closed once.
        unsafe {
            CloseHandle(snapshot);
        }
        found
    }

    fn open_if_same(id: u32, app: &Path) -> Option<Running> {
        // SAFETY: OpenProcess returns null on failure, which is checked.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, 0, id) };
        if handle.is_null() {
            return None;
        }
        let process = Running(handle);
        let mut buffer = vec![0u16; 32768];
        let mut size = buffer.len() as u32;
        // SAFETY: size holds the buffer's length in characters.
        let ok = unsafe { QueryFullProcessImageNameW(process.0, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut size) } != 0;
        let path = PathBuf::from(String::from_utf16_lossy(&buffer[..size as usize]));
        (ok && super::same_file(&path, app)).then_some(process)
    }

    pub fn wait_for_exit(open: Vec<Running>, wait: Duration) -> Vec<Running> {
        let deadline = Instant::now() + wait;
        open.into_iter()
            .filter(|process| {
                let left = deadline.saturating_duration_since(Instant::now()).as_millis() as u32;
                // SAFETY: the handle was opened with SYNCHRONIZE.
                unsafe { WaitForSingleObject(process.0, left) != WAIT_OBJECT_0 }
            })
            .collect()
    }

    pub fn end(open: Vec<Running>) {
        for process in open {
            // SAFETY: the handle was opened with PROCESS_TERMINATE and SYNCHRONIZE.
            unsafe {
                TerminateProcess(process.0, 1);
                WaitForSingleObject(process.0, 5000);
            }
        }
    }
}

#[cfg(not(windows))]
mod processes {
    use std::path::Path;
    use std::time::Duration;

    pub struct Running;

    pub fn running(_app: &Path) -> Vec<Running> {
        Vec::new()
    }

    pub fn wait_for_exit(open: Vec<Running>, _wait: Duration) -> Vec<Running> {
        open
    }

    pub fn end(_open: Vec<Running>) {}
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
    fn always_tells_commy_whether_to_start_with_the_computer() {
        assert_eq!(launch_args(true), vec![AUTOSTART_ARG]);
        assert_eq!(launch_args(false), vec![NO_AUTOSTART_ARG]);
    }

    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("commy-setup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn running_commy(folder: &Path, answer: &'static str) -> std::thread::JoinHandle<String> {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        std::fs::write(folder.join(PORT_FILE), listener.local_addr().unwrap().port().to_string()).unwrap();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            reader.into_inner().write_all(answer.as_bytes()).unwrap();
            line
        })
    }

    #[test]
    fn asks_a_running_commy_to_quit() {
        let folder = folder("quit");
        let commy = running_commy(&folder, "ok\n");
        assert!(ask_to_quit(&folder));
        assert_eq!(commy.join().unwrap(), format!("{QUIT_REQUEST}\n"));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn an_older_commy_that_refuses_to_quit_is_not_waited_for() {
        let folder = folder("refuse");
        let commy = running_commy(&folder, "no\n");
        assert!(!ask_to_quit(&folder));
        commy.join().unwrap();
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn nothing_to_ask_when_commy_is_not_running() {
        let folder = folder("none");
        assert!(!ask_to_quit(&folder));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn matches_the_installed_app_whatever_the_case_or_slashes() {
        let installed = Path::new("C:\\Users\\x\\AppData\\Local\\Commy\\commy-desktop.exe");
        assert!(same_file(Path::new("c:/users/x/appdata/local/commy/Commy-Desktop.exe"), installed));
        assert!(!same_file(Path::new("C:\\Users\\x\\projects\\commy\\target\\commy-desktop.exe"), installed));
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
