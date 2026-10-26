use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::Utc;
use commy_net::invite::{BackgroundResponse, CreateRequest, CreateResponse, ResolveResponse};
use commy_net::invite::clip;
use commy_net::sealed::{Purpose, SealKey};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::account::host_of;
use crate::state::App;

pub use commy_net::invite::{
    looks_like_link, normalize_origin, parse, CreatedInvite, Expiry, InviteLink, InviteView, ResolvedInvite,
    MAX_INVITE_BACKGROUND_BYTES, MAX_LINK_LEN,
};

pub const DEFAULT_ACCOUNT_SERVER: &str = match option_env!("COMMY_DEFAULT_ACCOUNT_SERVER") {
    Some(server) => server,
    None => "https://commy.chat",
};
const PORT_FILE: &str = "link.port";
const HANDOFF_PREFIX: &str = "commy-link-v1 ";
const HANDOFF_LIMIT: u64 = (MAX_LINK_LEN + HANDOFF_PREFIX.len() + 2) as u64;

const REUSE_MIN_LIFE_LEFT: f64 = 0.85;

static PENDING: Mutex<Option<String>> = Mutex::new(None);
static CREATED: Mutex<BTreeMap<String, (CreatedInvite, Instant)>> = Mutex::new(BTreeMap::new());

#[derive(Deserialize)]
struct ErrorResponse {
    message: Option<String>,
}

pub(crate) async fn error_message(res: reqwest::Response) -> String {
    let status = res.status();
    match res.json::<ErrorResponse>().await {
        Ok(ErrorResponse { message: Some(m) }) => clip(&m, 200),
        _ => format!("the invite server answered {status}"),
    }
}

pub(crate) async fn account_call<T: serde::de::DeserializeOwned>(
    request: reqwest::RequestBuilder,
    origin: &str,
) -> Result<T, String> {
    let res = request.send().await.map_err(|_| format!("Could not reach {}", host_of(origin)))?;
    if !res.status().is_success() {
        return Err(error_message(res).await);
    }
    res.json().await.map_err(|_| "The invite server sent an invalid reply".to_string())
}

#[tauri::command]
pub async fn resolve_invite_link(app: tauri::State<'_, App>, link: String) -> Result<ResolvedInvite, String> {
    resolve(&app, &link).await
}

pub(crate) async fn resolve(app: &App, link: &str) -> Result<ResolvedInvite, String> {
    let parsed = parse(link).ok_or("That is not a Commy invite link")?;
    let body: ResolveResponse = account_call(app.http.get(parsed.endpoint(commy_net::invite::RESOLVE_PATH)), &parsed.origin).await?;
    let opened = commy_net::invite::open_reply(&parsed, &body, Utc::now())?;
    app.doorbell.lock().unwrap().note_invite_origin(opened.ticket.server_id, parsed.origin.clone());
    if let Some((id, secret)) = opened.community.clone() {
        crate::community_sync::remember_invite_key(opened.ticket.server_id, id, secret);
    }
    let background = match (body.has_background, parsed.key.as_deref()) {
        (true, Some(key)) => fetch_background(app, &parsed, key).await,
        _ => None,
    };
    Ok(commy_net::invite::resolved(&parsed, &opened, &crate::account::effective_server(), background))
}

async fn fetch_background(app: &App, link: &InviteLink, key: &str) -> Option<String> {
    let body: BackgroundResponse = account_call(app.http.get(link.endpoint(commy_net::invite::BACKGROUND_PATH)), &link.origin).await.ok()?;
    let bytes = commy_net::invite::open_background(&body.background, key).filter(|b| crate::is_jpeg_or_png(b))?;
    Some(crate::picture_data_url(&bytes))
}

#[tauri::command]
pub async fn create_invite_link(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    server_id: String,
    expires: Option<String>,
    name: Option<String>,
) -> Result<CreatedInvite, String> {
    let expiry = commy_net::invite::expiry_choice(expires.as_deref())?;
    let name = commy_net::invite::name_choice(name.as_deref())?;
    create(&app, &handle, &server_id, expiry, name).await
}

pub(crate) fn signed_in_token(app: &App) -> Option<String> {
    let state = app.account.lock().unwrap();
    crate::account::is_signed_in(&state).then(|| state.token.clone()).flatten()
}

async fn sealed_background(app: &App, server: commy_core::ServerId, key: &SealKey) -> Option<String> {
    let image = app.servers.lock().unwrap().get(&server)?.invite_background()?.to_string();
    match crate::server_image_bytes(app, &image).await {
        Ok(bytes) if bytes.len() <= MAX_INVITE_BACKGROUND_BYTES => Some(key.seal(Purpose::InviteBackground, &bytes)),
        Ok(_) => None,
        Err(e) => {
            tracing::debug!("invite made without its background: {e}");
            None
        }
    }
}

pub(crate) async fn create(
    app: &App,
    handle: &AppHandle,
    server_id: &str,
    expiry: Expiry,
    name: Option<String>,
) -> Result<CreatedInvite, String> {
    let id: commy_core::ServerId = server_id.parse().map_err(|_| "unknown server".to_string())?;
    crate::require_invite_permission(app, id)?;
    if name.is_some() {
        crate::require_manage_server(app, id, "pick a custom invite name")?;
    }
    if expiry == Expiry::Never && !app.servers.lock().unwrap().get(&id).is_some_and(|s| s.server.public) {
        return Err("Only links to a public server can last forever".into());
    }
    let ticket = app.vouched_ticket(id).ok_or("no invite available; you may be offline")?;
    if ticket.owner.is_none() {
        return Err("your account key is still being confirmed; try again in a moment".into());
    }
    let raw = ticket.encode();
    let origin = crate::account::effective_server();
    let cache_key = format!("{origin} {} {raw}", expiry.as_str());
    let cached = if name.is_some() { None } else { CREATED.lock().unwrap().get(&cache_key).cloned() };
    let cached = cached.filter(|(created, _)| {
        let servers = app.servers.lock().unwrap();
        let tracked = |l: InviteLink| servers.get(&id).is_some_and(|s| s.tracked_invite(&l.origin, &l.code).is_some());
        parse(&created.url).is_some_and(tracked)
    });
    if let Some((created, at)) = cached {
        let fresh = match expiry.length() {
            Some(length) => at.elapsed().as_secs_f64() < length.num_seconds() as f64 * (1.0 - REUSE_MIN_LIFE_LEFT),
            None => created.expires_at.is_none(),
        };
        if fresh {
            return Ok(created);
        }
    }
    let revoke_key = commy_net::invite::new_revoke_key();
    let (key, key_text) = commy_net::invite::link_key(&revoke_key, name.is_some());
    let background = sealed_background(app, id, &key).await;
    if crate::community_sync::invite_key(app, id).is_none() {
        crate::channel_lock::tend_scope(app, handle, id, commy_core::channel_key::KeyScope::Community).await;
    }
    let community = crate::community_sync::invite_key(app, id);
    let needs_account = expiry == Expiry::Never || name.is_some() || background.is_some();
    let token = needs_account.then(|| signed_in_token(app)).flatten();
    if name.is_some() && token.is_none() {
        return Err("Sign in to pick a custom invite name".into());
    }
    let background = background.filter(|_| token.is_some());
    let request = CreateRequest {
        raw_ticket: &raw,
        server_name: &ticket.server_name,
        expiry,
        custom: name.as_deref(),
        community: community.as_ref().map(|(id, secret)| (*id, secret)),
        background: background.as_deref(),
        revoke_key: &revoke_key,
    };
    let (body, sealed_at) = commy_net::invite::create_body(&key, &request, Utc::now());
    let mut request = app.http.post(format!("{origin}{}", commy_net::invite::CREATE_PATH)).json(&body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let reply: CreateResponse = account_call(request, &origin).await?;
    let made = commy_net::invite::created(&origin, name.as_deref(), key_text, reply, sealed_at)?;
    if name.is_none() {
        CREATED.lock().unwrap().insert(cache_key, (made.invite.clone(), Instant::now()));
    }
    track(app, handle, id, origin, &made, revoke_key).await;
    Ok(made.invite)
}

async fn track(app: &App, handle: &AppHandle, server: commy_core::ServerId, origin: String, made: &commy_net::invite::Created, revoke_key: String) {
    let Some(creator) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { return };
    let event = commy_net::invite::track_event(server, creator, origin, made, revoke_key, Utc::now().timestamp_millis());
    if let Err(e) = crate::apply_and_broadcast(app, handle, server, event).await {
        tracing::warn!("could not list the new invite for the server's admins: {e}");
    }
}

#[tauri::command]
pub fn list_invites(app: tauri::State<'_, App>, server_id: String) -> Result<Vec<InviteView>, String> {
    let id: commy_core::ServerId = server_id.parse().map_err(|_| "unknown server".to_string())?;
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id).ok_or("no profile yet")?;
    let servers = app.servers.lock().unwrap();
    let state = servers.get(&id).ok_or("unknown server")?;
    Ok(commy_net::invite::invite_views(state, me, Utc::now().timestamp_millis()))
}

#[tauri::command]
pub async fn invite_full_link(app: tauri::State<'_, App>, server_id: String, origin: String, code: String) -> Result<String, String> {
    let id: commy_core::ServerId = server_id.parse().map_err(|_| "unknown server".to_string())?;
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id).ok_or("no profile yet")?;
    let invite = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&id).ok_or("unknown server")?;
        commy_net::invite::revocable(state, me, &origin, &code)?
    };
    let link = commy_net::invite::tracked_link(&invite);
    let body: ResolveResponse = account_call(app.http.get(link.endpoint(commy_net::invite::RESOLVE_PATH)), &link.origin).await?;
    commy_net::invite::open_reply(&link, &body, Utc::now()).map_err(|_| commy_net::invite::OLD_LINK.to_string())?;
    Ok(link.url())
}

#[tauri::command]
pub async fn revoke_invite(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    server_id: String,
    origin: String,
    code: String,
) -> Result<(), String> {
    let id: commy_core::ServerId = server_id.parse().map_err(|_| "unknown server".to_string())?;
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id).ok_or("no profile yet")?;
    let invite = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&id).ok_or("unknown server")?;
        commy_net::invite::revocable(state, me, &origin, &code)?
    };
    let request = app.http.post(format!("{}{}", invite.origin, commy_net::invite::REVOKE_PATH)).json(&commy_net::invite::revoke_body(&invite));
    let res = request.send().await.map_err(|_| format!("Could not reach {}", host_of(&invite.origin)))?;
    if !res.status().is_success() && res.status() != reqwest::StatusCode::NOT_FOUND {
        return Err(error_message(res).await);
    }
    CREATED.lock().unwrap().retain(|_, (created, _)| parse(&created.url).is_none_or(|l| l.code != invite.code));
    crate::apply_and_broadcast(&app, &handle, id, commy_net::invite::revoke_event(id, invite)).await
}

#[derive(Serialize)]
pub struct AccountServerInfo {
    pub origin: String,
    pub host: String,
    pub is_default: bool,
}

#[tauri::command]
pub fn account_server_info() -> AccountServerInfo {
    let origin = crate::account::effective_server();
    AccountServerInfo { host: host_of(&origin), is_default: origin == DEFAULT_ACCOUNT_SERVER, origin }
}

#[tauri::command]
pub fn take_pending_invite_link() -> Option<String> {
    PENDING.lock().unwrap().take()
}

pub fn link_from_args() -> Option<String> {
    std::env::args().skip(1).find(|a| a.starts_with("commy://") && looks_like_link(a))
}

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
    crate::tray::show_main_window(handle);
    let _ = handle.emit("invite-link", link);
}

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
            let mut reader = BufReader::new(stream.take(HANDOFF_LIMIT));
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
            let mut reader = BufReader::new(stream.take(HANDOFF_LIMIT));
            reader.read_line(&mut line).unwrap();
            reader.into_inner().into_inner().write_all(b"ok\n").unwrap();
            line
        });
        assert!(forward_to_running_instance(&dir, "commy://i/commy.chat/AB23CD45"));
        assert_eq!(server.join().unwrap(), "commy-link-v1 commy://i/commy.chat/AB23CD45\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
