//! Shareable invite links. `https://<server>/i/<CODE>#<KEY>` (and the
//! `commy://i/<host>/<CODE>#<KEY>` deep link its landing page opens) map a
//! short code to a server ticket stored on that account server, so a link
//! always resolves on the server that issued it. The ticket is sealed with
//! KEY, which only travels in the link's fragment: browsers and this app never
//! send a fragment to a server, so the account server cannot tell which Commy
//! server an invite opens or who is in it. Links without a key are from
//! older versions and still open until they expire.
//!
//! The creator picks how long a link lasts ([`Expiry`]). The expiry is sealed
//! inside the invite too, so the app refuses an expired link even if the
//! account server kept or extended its row.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{DateTime, SecondsFormat, Utc};
use commy_net::sealed::{self, Purpose, SealKey};
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

/// How long a new invite link lasts. `Never` is only offered for a server
/// marked public.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expiry {
    Hour,
    Day,
    Week,
    Month,
    Never,
}

impl Expiry {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "1h" => Expiry::Hour,
            "1d" => Expiry::Day,
            "7d" => Expiry::Week,
            "30d" => Expiry::Month,
            "never" => Expiry::Never,
            _ => return None,
        })
    }

    /// The account server's name for it.
    pub fn as_str(self) -> &'static str {
        match self {
            Expiry::Hour => "1h",
            Expiry::Day => "1d",
            Expiry::Week => "7d",
            Expiry::Month => "30d",
            Expiry::Never => "never",
        }
    }

    pub fn length(self) -> Option<chrono::Duration> {
        match self {
            Expiry::Hour => Some(chrono::Duration::hours(1)),
            Expiry::Day => Some(chrono::Duration::days(1)),
            Expiry::Week => Some(chrono::Duration::days(7)),
            Expiry::Month => Some(chrono::Duration::days(30)),
            Expiry::Never => None,
        }
    }
}

/// A link made this run is shared again while at least this much of its
/// life is left, so it never runs out right after being copied.
const REUSE_MIN_LEFT: f64 = 0.85;

static PENDING: Mutex<Option<String>> = Mutex::new(None);
/// Links this run created, by ticket and expiry, so sharing twice gives the
/// same link.
static CREATED: Mutex<Option<HashMap<String, (CreatedInvite, Instant)>>> = Mutex::new(None);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteLink {
    /// scheme://host[:port] of the server that issued the code
    pub origin: String,
    pub code: String,
    /// The key that opens the sealed ticket. None for links from older versions.
    pub key: Option<String>,
}

impl InviteLink {
    pub fn url(&self) -> String {
        match &self.key {
            Some(key) => format!("{}/i/{}#{key}", self.origin, self.code),
            None => format!("{}/i/{}", self.origin, self.code),
        }
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

fn valid_key(fragment: &str) -> Option<String> {
    SealKey::from_text(fragment).ok().map(|_| fragment.to_string())
}

/// Parses `https://host/i/CODE#KEY` or `commy://i/host/CODE#KEY` (KEY is
/// optional for older links). Anything else, including other paths,
/// credentials, non-https servers or a malformed key, is rejected.
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
    let key = match url.fragment() {
        None => None,
        Some(fragment) => Some(valid_key(fragment)?),
    };
    Some(InviteLink { origin, code: normalize_code(code)?, key })
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
    /// RFC 3339; None when the link never expires.
    pub expires_at: Option<String>,
    /// False when the link points somewhere other than this install's account server
    pub trusted: bool,
    pub account_server_host: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolveResponse {
    ticket: String,
    /// Null when the row never expires.
    expires_at: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateResponse {
    url: String,
    expires_at: Option<String>,
}

/// A new invite link and when it stops working (RFC 3339, None for never).
#[derive(Debug, Clone, Serialize)]
pub struct CreatedInvite {
    pub url: String,
    pub expires_at: Option<String>,
}

fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.trim()).ok().map(|t| t.with_timezone(&Utc))
}

/// When an invite stops working: the earlier of what the account server says
/// and what is sealed inside. A sealed expiry that has passed is refused, so a
/// row the server kept or extended cannot bring the link back.
fn effective_expiry(
    server: Option<DateTime<Utc>>,
    sealed: Option<SealedExpiry>,
    now: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, String> {
    let sealed = match sealed {
        Some(SealedExpiry::At(t)) if t <= now => return Err("This invite has expired. Ask for a new link.".into()),
        Some(SealedExpiry::At(t)) => Some(t),
        Some(SealedExpiry::Never) | None => None,
    };
    Ok(match (server, sealed) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    })
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
    resolve(&app, &link).await
}

/// Looks up an invite link on the server that issued it and opens its ticket.
pub(crate) async fn resolve(app: &App, link: &str) -> Result<ResolvedInvite, String> {
    let parsed = parse(link).ok_or("That is not a Commy invite link")?;
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
    let (raw, sealed_expiry) = open_ticket(&body.ticket, parsed.key.as_deref())?;
    let ticket = ServerTicket::decode(&raw).map_err(|_| "The invite server sent an invalid invite".to_string())?;
    app.doorbell.lock().unwrap().note_invite_origin(ticket.server_id, parsed.origin.clone());
    let server_expiry = body.expires_at.as_deref().and_then(parse_time);
    let expires_at = effective_expiry(server_expiry, sealed_expiry, Utc::now())?;
    let account = account_server();
    Ok(ResolvedInvite {
        link: parsed.url(),
        host: parsed.host().to_string(),
        server_name: clip(&ticket.server_name, 100),
        ticket: ticket.encode(),
        expires_at: expires_at.map(|t| t.to_rfc3339_opts(SecondsFormat::Secs, true)),
        trusted: parsed.origin == account,
        account_server_host: host_of(&account),
    })
}

/// What an invite link seals: the ticket, the server name for the
/// website's invite page, and when it expires (unix seconds), or that it
/// never does. Invites sealed before expiries were chosen carry neither.
#[derive(Serialize, Deserialize)]
struct SealedInvite {
    #[serde(rename = "t")]
    ticket: String,
    #[serde(rename = "n")]
    name: String,
    #[serde(rename = "e", default, skip_serializing_if = "Option::is_none")]
    expires: Option<i64>,
    #[serde(rename = "p", default, skip_serializing_if = "std::ops::Not::not")]
    permanent: bool,
}

/// The expiry sealed inside an invite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SealedExpiry {
    At(DateTime<Utc>),
    Never,
}

fn seal_ticket(key: &SealKey, raw: &str, name: &str, expiry: SealedExpiry) -> String {
    let (expires, permanent) = match expiry {
        SealedExpiry::At(t) => (Some(t.timestamp()), false),
        SealedExpiry::Never => (None, true),
    };
    let invite = SealedInvite { ticket: raw.to_string(), name: name.to_string(), expires, permanent };
    let payload = serde_json::to_vec(&invite).expect("invite serialization cannot fail");
    key.seal(Purpose::Invite, &payload)
}

/// The ticket a resolved invite holds, and the expiry sealed with it (None
/// for invites from before expiries were sealed, and for older keyless
/// links). A link with a key only accepts a sealed ticket that key opens.
fn open_ticket(stored: &str, key: Option<&str>) -> Result<(String, Option<SealedExpiry>), String> {
    let stored = stored.trim();
    match key {
        Some(key) => {
            let key = SealKey::from_text(key).map_err(|_| "This invite link is damaged".to_string())?;
            let bytes = key
                .open(Purpose::Invite, stored)
                .map_err(|_| "This invite link is damaged or was changed".to_string())?;
            let invite: SealedInvite =
                serde_json::from_slice(&bytes).map_err(|_| "The invite server sent an invalid invite".to_string())?;
            let expiry = match (invite.permanent, invite.expires) {
                (true, _) => Some(SealedExpiry::Never),
                (false, Some(secs)) => Some(SealedExpiry::At(
                    DateTime::from_timestamp(secs, 0).ok_or("The invite server sent an invalid invite")?,
                )),
                (false, None) => None,
            };
            Ok((invite.ticket, expiry))
        }
        None if sealed::is_sealed(stored) => Err("This invite link is missing its key. Ask for a new link.".into()),
        None => Ok((stored.to_string(), None)),
    }
}

#[tauri::command]
pub async fn create_invite_link(
    app: tauri::State<'_, App>,
    server_id: String,
    expires: Option<String>,
) -> Result<CreatedInvite, String> {
    let expiry = match expires.as_deref() {
        None => Expiry::Week,
        Some(value) => Expiry::parse(value).ok_or("unknown invite expiry")?,
    };
    create(&app, &server_id, expiry).await
}

/// Stores a server's ticket, sealed, on our account server and returns
/// the link that opens it. A link that never expires is only made for a
/// server marked public.
pub(crate) async fn create(app: &App, server_id: &str, expiry: Expiry) -> Result<CreatedInvite, String> {
    let id: commy_core::ServerId = server_id.parse().map_err(|_| "unknown server".to_string())?;
    crate::require_invite_permission(app, id)?;
    if expiry == Expiry::Never && !app.servers.lock().unwrap().get(&id).is_some_and(|s| s.server.public) {
        return Err("Only links to a public server can last forever".into());
    }
    let ticket = app.vouched_ticket(id).ok_or("no invite available; you may be offline")?;
    let raw = ticket.encode();
    let origin = account_server();
    let cache_key = format!("{origin} {} {raw}", expiry.as_str());
    if let Some((created, at)) = CREATED.lock().unwrap().as_ref().and_then(|m| m.get(&cache_key)) {
        let fresh = match expiry.length() {
            Some(length) => at.elapsed().as_secs_f64() < length.num_seconds() as f64 * (1.0 - REUSE_MIN_LEFT),
            // Unless an older account server gave it an expiry after all
            None => created.expires_at.is_none(),
        };
        if fresh {
            return Ok(created.clone());
        }
    }
    let now = Utc::now();
    let sealed_expiry = match expiry.length() {
        Some(length) => SealedExpiry::At(now + length),
        None => SealedExpiry::Never,
    };
    // The account server only ever gets the sealed ticket; the key stays in the link
    let key = SealKey::generate();
    let res = app
        .http
        .post(format!("{origin}/api/auth/commy/invites/create"))
        .json(&serde_json::json!({
            "ticket": seal_ticket(&key, &raw, &ticket.server_name, sealed_expiry),
            "expiresIn": expiry.as_str(),
        }))
        .send()
        .await
        .map_err(|_| format!("Could not reach {}", host_of(&origin)))?;
    if !res.status().is_success() {
        return Err(error_message(res).await);
    }
    let body: CreateResponse = res.json().await.map_err(|_| "The invite server sent an invalid reply".to_string())?;
    // Only hand out a link that points back at our own account server
    match parse(&body.url) {
        Some(link) if link.origin == origin && link.key.is_none() => {
            let url = InviteLink { key: Some(key.to_text()), ..link }.url();
            // An account server too old to know the choice keeps its own
            // default; the link then lasts the shorter of the two.
            let server_expiry = body.expires_at.as_deref().and_then(parse_time);
            let sealed = match sealed_expiry {
                SealedExpiry::At(t) => Some(t),
                SealedExpiry::Never => None,
            };
            let expires_at = match (server_expiry, sealed) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            let created = CreatedInvite {
                url,
                expires_at: expires_at.map(|t| t.to_rfc3339_opts(SecondsFormat::Secs, true)),
            };
            CREATED.lock().unwrap().get_or_insert_with(HashMap::new).insert(cache_key, (created.clone(), Instant::now()));
            Ok(created)
        }
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

    // A fixed test key: the bytes 0 to 31 as unpadded base64url
    fn test_key() -> String {
        data_encoding::BASE64URL_NOPAD.encode(&(0u8..32).collect::<Vec<_>>())
    }

    fn link(origin: &str, code: &str) -> Option<InviteLink> {
        Some(InviteLink { origin: origin.into(), code: code.into(), key: None })
    }

    fn keyed(origin: &str, code: &str) -> Option<InviteLink> {
        Some(InviteLink { origin: origin.into(), code: code.into(), key: Some(test_key()) })
    }

    #[test]
    fn parses_links_with_keys() {
        let key = test_key();
        assert_eq!(parse(&format!("https://commy.chat/i/AB23CD45#{key}")), keyed("https://commy.chat", "AB23CD45"));
        assert_eq!(parse(&format!("commy://i/commy.chat/AB23CD45#{key}")), keyed("https://commy.chat", "AB23CD45"));
        let l = parse(&format!("commy://i/localhost%3A8443/AB23CD45#{key}")).unwrap();
        assert_eq!(l.url(), format!("https://localhost:8443/i/AB23CD45#{key}"));
        assert_eq!(parse(&l.url()), Some(l));
        let too_long = format!("https://commy.chat/i/AB23CD45#{key}A");
        let bad_char = format!("https://commy.chat/i/AB23CD45#{}+", &key[..key.len() - 1]);
        for bad in ["https://commy.chat/i/AB23CD45#", "https://commy.chat/i/AB23CD45#short", &too_long, &bad_char] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn sealed_tickets_need_their_key() {
        let text = test_key();
        let key = SealKey::from_text(&text).unwrap();
        let sealed = seal_ticket(&key, "commyticket", "Pixel Pioneers", SealedExpiry::Never);
        assert!(!sealed.contains("commyticket") && !sealed.contains("Pixel"));
        assert_eq!(open_ticket(&sealed, Some(&text)).unwrap(), ("commyticket".into(), Some(SealedExpiry::Never)));
        assert!(open_ticket(&sealed, None).is_err());
        assert!(open_ticket(&sealed, Some(&SealKey::generate().to_text())).is_err());
        // A keyed link never accepts a plain ticket in place of a sealed one
        assert!(open_ticket("commyticket", Some(&text)).is_err());
        // Older links without a key still open plain tickets
        assert_eq!(open_ticket(" commyticket ", None).unwrap(), ("commyticket".into(), None));
    }

    #[test]
    fn the_sealed_expiry_wins() {
        let text = test_key();
        let key = SealKey::from_text(&text).unwrap();
        let at = DateTime::from_timestamp(1_800_000_000, 0).unwrap();
        let sealed = seal_ticket(&key, "commyticket", "Pixel Pioneers", SealedExpiry::At(at));
        let (_, expiry) = open_ticket(&sealed, Some(&text)).unwrap();
        assert_eq!(expiry, Some(SealedExpiry::At(at)));

        let before = at - chrono::Duration::days(1);
        let later = at + chrono::Duration::days(300);
        // A row the server extended, or claims never expires, still ends on time
        assert_eq!(effective_expiry(Some(later), expiry, before), Ok(Some(at)));
        assert_eq!(effective_expiry(None, expiry, before), Ok(Some(at)));
        // Once the sealed time has passed the link is refused, whatever the server says
        assert!(effective_expiry(Some(later), expiry, at).is_err());
        assert!(effective_expiry(None, expiry, later).is_err());
        // An earlier server expiry is shown as is
        assert_eq!(effective_expiry(Some(before), expiry, before - chrono::Duration::hours(1)), Ok(Some(before)));
        // Links that never expire, and invites from before sealed expiries
        assert_eq!(effective_expiry(None, Some(SealedExpiry::Never), later), Ok(None));
        assert_eq!(effective_expiry(Some(at), None, before), Ok(Some(at)));
    }

    #[test]
    fn invites_sealed_before_expiries_still_open() {
        let text = test_key();
        let key = SealKey::from_text(&text).unwrap();
        let old = key.seal(Purpose::Invite, br#"{"t":"commyticket","n":"Pixel Pioneers"}"#);
        assert_eq!(open_ticket(&old, Some(&text)).unwrap(), ("commyticket".into(), None));
    }

    #[test]
    fn expiry_choices_round_trip() {
        for value in ["1h", "1d", "7d", "30d", "never"] {
            assert_eq!(Expiry::parse(value).unwrap().as_str(), value);
        }
        assert_eq!(Expiry::parse("90d"), None);
        assert_eq!(Expiry::Never.length(), None);
        assert_eq!(Expiry::Month.length(), Some(chrono::Duration::days(30)));
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
