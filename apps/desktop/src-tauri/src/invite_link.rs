//! Shareable invite links. `https://<server>/i/<CODE>` (and the
//! `commy://i/<host>/<CODE>` deep link its landing page opens) map a short
//! code to a server ticket stored on that account server, so a link always
//! resolves on the server that issued it.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use commy_net::ServerTicket;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::App;

/// commy.chat, unless the build sets COMMY_DEFAULT_ACCOUNT_SERVER (test builds
/// from pre-release tags use https://staging.commy.chat).
pub const DEFAULT_ACCOUNT_SERVER: &str = match option_env!("COMMY_DEFAULT_ACCOUNT_SERVER") {
    Some(server) => server,
    None => "https://commy.chat",
};
pub const SCHEME: &str = "commy";
const CODE_ALPHABET: &str = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const CODE_LENGTH: usize = 8;
const MAX_LINK_LEN: usize = 2048;
const PORT_FILE: &str = "link.port";
const HANDOFF_PREFIX: &str = "commy-link-v1 ";

static PENDING: Mutex<Option<String>> = Mutex::new(None);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteLink {
    /// scheme://host[:port] of the server that issued the code
    pub origin: String,
    pub code: String,
}

impl InviteLink {
    pub fn url(&self) -> String {
        format!("{}/i/{}", self.origin, self.code)
    }

    pub fn host(&self) -> &str {
        self.origin.split("://").nth(1).unwrap_or(&self.origin)
    }
}

/// The account server this install uses: `COMMY_AUTH_URL`, the Settings
/// choice, or commy.chat.
pub fn account_server() -> String {
    crate::account::effective_server()
}

fn is_loopback(host: &str) -> bool {
    host == "localhost" || host == "127.0.0.1" || host == "[::1]" || host.ends_with(".localhost")
}

fn origin_of(url: &Url) -> Option<String> {
    let host = url.host_str()?.to_ascii_lowercase();
    match url.scheme() {
        "https" => {}
        // Plain http only for a dev stack on this machine
        "http" if is_loopback(&host) => {}
        _ => return None,
    }
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    Some(match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    })
}

/// Lowercased `scheme://host[:port]` of a server URL, https only.
pub fn normalize_origin(input: &str) -> Option<String> {
    origin_of(&Url::parse(input.trim()).ok()?)
}

pub fn normalize_code(input: &str) -> Option<String> {
    let code = input.trim().to_ascii_uppercase();
    (code.len() == CODE_LENGTH && code.chars().all(|c| CODE_ALPHABET.contains(c))).then_some(code)
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = input.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Parses `https://host/i/CODE` or `commy://i/host/CODE`. Anything else,
/// including other paths, credentials or non-https servers, is rejected.
pub fn parse(input: &str) -> Option<InviteLink> {
    let input = input.trim();
    if input.len() > MAX_LINK_LEN {
        return None;
    }
    let url = Url::parse(input).ok()?;
    let mut segments: Vec<&str> = url.path_segments()?.collect();
    if segments.last() == Some(&"") {
        segments.pop();
    }

    let (origin, code) = if url.scheme() == SCHEME {
        if url.host_str() != Some("i") || segments.len() != 2 {
            return None;
        }
        let host = percent_decode(segments[0])?.to_ascii_lowercase();
        let host_ok = !host.is_empty()
            && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
        if !host_ok {
            return None;
        }
        (normalize_origin(&format!("https://{host}"))?, segments[1])
    } else {
        if segments.len() != 2 || segments[0] != "i" || url.query().is_some() {
            return None;
        }
        (origin_of(&url)?, segments[1])
    };
    Some(InviteLink { origin, code: normalize_code(code)? })
}

pub fn looks_like_link(input: &str) -> bool {
    parse(input).is_some()
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedInvite {
    pub link: String,
    pub host: String,
    pub server_name: String,
    pub ticket: String,
    pub expires_at: String,
    /// False when the link points somewhere other than this install's account server
    pub trusted: bool,
    pub account_server_host: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolveResponse {
    ticket: String,
    expires_at: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateResponse {
    url: String,
}

#[derive(Deserialize)]
struct ErrorResponse {
    message: Option<String>,
}

async fn error_message(res: reqwest::Response) -> String {
    let status = res.status();
    match res.json::<ErrorResponse>().await {
        Ok(ErrorResponse { message: Some(m) }) => clip(&m, 200),
        _ => format!("the invite server answered {status}"),
    }
}

fn clip(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

fn host_of(origin: &str) -> String {
    origin.split("://").nth(1).unwrap_or(origin).to_string()
}

#[tauri::command]
pub async fn resolve_invite_link(app: tauri::State<'_, App>, link: String) -> Result<ResolvedInvite, String> {
    let parsed = parse(&link).ok_or("That is not a Commy invite link")?;
    let mut url = Url::parse(&format!("{}/api/auth/commy/invites/resolve", parsed.origin)).map_err(|e| e.to_string())?;
    url.query_pairs_mut().append_pair("code", &parsed.code);
    let res = app
        .http
        .get(url)
        .send()
        .await
        .map_err(|_| format!("Could not reach {}", parsed.host()))?;
    if !res.status().is_success() {
        return Err(error_message(res).await);
    }
    let body: ResolveResponse = res.json().await.map_err(|_| "The invite server sent an invalid reply".to_string())?;
    // The server name comes from the ticket itself, not from the web server
    let ticket = ServerTicket::decode(body.ticket.trim()).map_err(|_| "The invite server sent an invalid invite".to_string())?;
    let account = account_server();
    Ok(ResolvedInvite {
        link: parsed.url(),
        host: parsed.host().to_string(),
        server_name: clip(&ticket.server_name, 100),
        ticket: ticket.encode(),
        expires_at: clip(&body.expires_at, 40),
        trusted: parsed.origin == account,
        account_server_host: host_of(&account),
    })
}

#[tauri::command]
pub async fn create_invite_link(app: tauri::State<'_, App>, server_id: String) -> Result<String, String> {
    let id: commy_core::ServerId = server_id.parse().map_err(|_| "unknown server".to_string())?;
    let raw = app
        .invites
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or("no invite available; you may be offline")?;
    let ticket = ServerTicket::decode(&raw).map_err(|e| e.to_string())?;
    let origin = account_server();
    let res = app
        .http
        .post(format!("{origin}/api/auth/commy/invites/create"))
        .json(&serde_json::json!({ "ticket": raw, "serverName": ticket.server_name }))
        .send()
        .await
        .map_err(|_| format!("Could not reach {}", host_of(&origin)))?;
    if !res.status().is_success() {
        return Err(error_message(res).await);
    }
    let body: CreateResponse = res.json().await.map_err(|_| "The invite server sent an invalid reply".to_string())?;
    // Only hand out a link that points back at our own account server
    match parse(&body.url) {
        Some(link) if link.origin == origin => Ok(link.url()),
        _ => Err("The invite server sent an unexpected link".into()),
    }
}

#[derive(Serialize)]
pub struct AccountServerInfo {
    pub origin: String,
    pub host: String,
    pub is_default: bool,
}

#[tauri::command]
pub fn account_server_info() -> AccountServerInfo {
    let origin = account_server();
    AccountServerInfo { host: host_of(&origin), is_default: origin == DEFAULT_ACCOUNT_SERVER, origin }
}

/// A link the app was launched with, handed to the UI once it is ready.
#[tauri::command]
pub fn take_pending_invite_link() -> Option<String> {
    PENDING.lock().unwrap().take()
}

// ---------- Launch handling ----------

/// The `commy://` link in this process's arguments, if it was opened by one.
pub fn link_from_args() -> Option<String> {
    std::env::args().skip(1).find(|a| a.starts_with("commy://") && looks_like_link(a))
}

/// Where Tauri keeps app data, computed before Tauri starts so a second
/// launch can find the running instance. Matches `app_data_dir()`.
pub fn default_data_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("COMMY_DATA_DIR") {
        return Some(PathBuf::from(dir));
    }
    #[cfg(windows)]
    {
        std::env::var("APPDATA").ok().map(|d| PathBuf::from(d).join(crate::toast::APP_ID))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join("Library/Application Support").join(crate::toast::APP_ID))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

/// Passes a link to an already running instance. True when it took it.
pub fn forward_to_running_instance(data_dir: &Path, link: &str) -> bool {
    let Some(port) = std::fs::read_to_string(data_dir.join(PORT_FILE))
        .ok()
        .and_then(|s| s.trim().parse::<u16>().ok())
    else {
        return false;
    };
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    if stream.write_all(format!("{HANDOFF_PREFIX}{link}\n").as_bytes()).is_err() {
        return false;
    }
    let mut ack = String::new();
    BufReader::new(stream).read_line(&mut ack).is_ok() && ack.trim() == "ok"
}

fn deliver(handle: &AppHandle, link: String) {
    *PENDING.lock().unwrap() = Some(link.clone());
    if let Some(window) = handle.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    let _ = handle.emit("invite-link", link);
}

/// macOS delivers clicked commy:// links as an Apple Event, both at launch
/// and while running, instead of in argv.
pub fn on_run_event(handle: &AppHandle, event: &tauri::RunEvent) {
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "android"))]
    if let tauri::RunEvent::Opened { urls } = event {
        for url in urls {
            let link = url.to_string();
            if looks_like_link(&link) {
                deliver(handle, link);
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "android")))]
    let _ = (handle, event);
}

/// Accepts links from later launches on a loopback port recorded in the
/// data dir, and queues the link this process was started with.
pub fn start(handle: &AppHandle, data_dir: &Path, launch_link: Option<String>) {
    if let Some(link) = launch_link {
        *PENDING.lock().unwrap() = Some(link);
    }
    let listener = match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("invite link handoff unavailable: {e}");
            return;
        }
    };
    let Ok(addr) = listener.local_addr() else { return };
    let _ = std::fs::create_dir_all(data_dir);
    if let Err(e) = std::fs::write(data_dir.join(PORT_FILE), addr.port().to_string()) {
        tracing::warn!("could not record the invite link port: {e}");
        return;
    }
    let handle = handle.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut line = String::new();
            let mut reader = BufReader::new(stream.take_limit());
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let link = line.trim().strip_prefix(HANDOFF_PREFIX).map(str::to_string);
            let mut stream = reader.into_inner().into_inner();
            match link {
                Some(link) if looks_like_link(&link) => {
                    let _ = stream.write_all(b"ok\n");
                    deliver(&handle, link);
                }
                _ => {
                    let _ = stream.write_all(b"no\n");
                }
            }
        }
    });
}

trait TakeLimit {
    fn take_limit(self) -> std::io::Take<TcpStream>;
}

impl TakeLimit for TcpStream {
    fn take_limit(self) -> std::io::Take<TcpStream> {
        std::io::Read::take(self, (MAX_LINK_LEN + HANDOFF_PREFIX.len() + 2) as u64)
    }
}

/// Registers the `commy://` scheme for this user so invite pages can open
/// the app. Unpackaged builds have no installer to do it.
#[cfg(windows)]
pub fn register_scheme() {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE,
        REG_SZ,
    };
    let Ok(exe) = std::env::current_exe() else { return };
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let write = |path: &str, values: &[(&str, &str)]| unsafe {
        let path = wide(path);
        let mut key: HKEY = 0;
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
            tracing::warn!("could not register the commy:// link handler: {status}");
            return;
        }
        for (name, value) in values {
            let name = wide(name);
            let value = wide(value);
            RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, value.as_ptr().cast(), (value.len() * 2) as u32);
        }
        RegCloseKey(key);
    };
    write("Software\\Classes\\commy", &[("", "URL:Commy invite"), ("URL Protocol", "")]);
    write(
        "Software\\Classes\\commy\\shell\\open\\command",
        &[("", &format!("\"{}\" \"%1\"", exe.display()))],
    );
}

#[cfg(not(windows))]
pub fn register_scheme() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(origin: &str, code: &str) -> Option<InviteLink> {
        Some(InviteLink { origin: origin.into(), code: code.into() })
    }

    #[test]
    fn parses_https_links() {
        assert_eq!(parse("https://commy.chat/i/AB23CD45"), link("https://commy.chat", "AB23CD45"));
        assert_eq!(parse("  https://Chat.Example.org/i/ab23cd45/ "), link("https://chat.example.org", "AB23CD45"));
        assert_eq!(parse("https://example.org:8443/i/AB23CD45"), link("https://example.org:8443", "AB23CD45"));
        assert_eq!(parse("http://localhost:3000/i/AB23CD45"), link("http://localhost:3000", "AB23CD45"));
    }

    #[test]
    fn parses_deep_links() {
        assert_eq!(parse("commy://i/commy.chat/AB23CD45"), link("https://commy.chat", "AB23CD45"));
        assert_eq!(parse("commy://i/localhost%3A8443/AB23CD45"), link("https://localhost:8443", "AB23CD45"));
        assert_eq!(parse("commy://i/Example.ORG/ab23cd45/"), link("https://example.org", "AB23CD45"));
    }

    #[test]
    fn rejects_everything_else() {
        for bad in [
            "",
            "commyabcdefghijklmnopqrstuvwxyz234567abcdefgh",
            "http://example.org/i/AB23CD45",
            "ftp://example.org/i/AB23CD45",
            "https://example.org/invite/AB23CD45",
            "https://example.org/i/AB23CD45/extra",
            "https://example.org/i/AB23CD45?next=evil",
            "https://user:pw@example.org/i/AB23CD45",
            "https://example.org/i/AB23CD4O",
            "https://example.org/i/AB23",
            "commy://x/example.org/AB23CD45",
            "commy://i/evil.com%2Fpath/AB23CD45",
            "commy://i/a%40evil.com/AB23CD45",
            "commy://i/example.org/AB23CD45/more",
            "commy://i//AB23CD45",
            "commy://i/%ZZ/AB23CD45",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        assert_eq!(parse(&format!("https://example.org/i/{}", "A".repeat(MAX_LINK_LEN))), None);
    }

    #[test]
    fn link_round_trips_through_its_url() {
        let l = parse("commy://i/localhost%3A8443/AB23CD45").unwrap();
        assert_eq!(l.url(), "https://localhost:8443/i/AB23CD45");
        assert_eq!(l.host(), "localhost:8443");
        assert_eq!(parse(&l.url()), Some(l));
    }

    #[test]
    fn origins_are_normalized() {
        assert_eq!(normalize_origin("https://Commy.Chat/"), Some("https://commy.chat".into()));
        assert_eq!(normalize_origin("https://commy.chat/some/path"), Some("https://commy.chat".into()));
        assert_eq!(normalize_origin("http://commy.chat"), None);
        assert_eq!(normalize_origin("http://127.0.0.1:3001"), Some("http://127.0.0.1:3001".into()));
        assert_eq!(normalize_origin("not a url"), None);
    }

    #[test]
    fn handoff_reaches_a_listener() {
        let dir = std::env::temp_dir().join(format!("commy-link-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!forward_to_running_instance(&dir, "commy://i/commy.chat/AB23CD45"));

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        std::fs::write(dir.join(PORT_FILE), listener.local_addr().unwrap().port().to_string()).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            let mut reader = BufReader::new(stream.take_limit());
            reader.read_line(&mut line).unwrap();
            reader.into_inner().into_inner().write_all(b"ok\n").unwrap();
            line
        });
        assert!(forward_to_running_instance(&dir, "commy://i/commy.chat/AB23CD45"));
        assert_eq!(server.join().unwrap(), "commy-link-v1 commy://i/commy.chat/AB23CD45\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
