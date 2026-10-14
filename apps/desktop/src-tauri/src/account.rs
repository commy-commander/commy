//! Commy account sign-in. An account on the account server (commy.chat by
//! default) is required to create or join servers, which keeps throwaway
//! spam installs out. Sign-in uses the OAuth device flow (RFC 8628): the
//! app shows a code, the browser approves it, and the app receives a
//! bearer token. The app then links its iroh node id to the account by
//! signing the server's challenge with its node key, so only the holder of
//! a node key can claim that node id.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::invite_link::{normalize_origin, DEFAULT_ACCOUNT_SERVER};
use crate::state::App;

const FILE: &str = "account.json";

/// Saved sign-in. The token is a bearer secret, kept only in this
/// install's data folder.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountState {
    /// Account server chosen in Settings; None means the default.
    #[serde(default)]
    pub server: Option<String>,
    /// Server the token below belongs to.
    #[serde(default)]
    pub signed_in_server: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub user: Option<AccountUser>,
    #[serde(default)]
    pub device_linked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccountUser {
    pub id: String,
    pub name: String,
    pub email: String,
}

static CHOSEN_SERVER: Mutex<Option<String>> = Mutex::new(None);

/// The account server: `COMMY_AUTH_URL`, else the Settings choice, else commy.chat.
pub fn effective_server() -> String {
    if let Some(env) = std::env::var("COMMY_AUTH_URL").ok().and_then(|s| normalize_origin(&s)) {
        return env;
    }
    CHOSEN_SERVER.lock().unwrap().clone().unwrap_or_else(|| DEFAULT_ACCOUNT_SERVER.to_string())
}

fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

pub fn load(dir: &Path) -> AccountState {
    let state: AccountState = std::fs::read_to_string(path(dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    *CHOSEN_SERVER.lock().unwrap() = state.server.clone();
    state
}

fn save(dir: &Path, state: &AccountState) {
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let tmp = path(dir).with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, path(dir));
        }
    }
}

/// Signed in to the server currently in use. A token for a different
/// server (the setting or COMMY_AUTH_URL changed) does not count.
pub fn is_signed_in(state: &AccountState) -> bool {
    state.token.is_some() && state.user.is_some() && state.signed_in_server.as_deref() == Some(effective_server().as_str())
}

pub fn require_account(app: &App) -> Result<(), String> {
    if is_signed_in(&app.account.lock().unwrap()) {
        Ok(())
    } else {
        Err(format!("Sign in to your Commy account on {} first", host_of(&effective_server())))
    }
}

pub fn host_of(origin: &str) -> String {
    origin.split("://").nth(1).unwrap_or(origin).to_string()
}

/// The exact message the account server asks us to sign is fetched from
/// it, but it must have this shape: it names the server, so a signature
/// for one account server cannot be replayed on another.
pub fn link_message_ok(message: &str, server: &str, node_id: &str, challenge: &str) -> bool {
    message == format!("commy-device-link:v1\n{server}\n{node_id}\n{challenge}")
}

/// Node id (hex public key) and a signature over `message`, from the node secret.
pub fn sign_with_node_key(secret: &[u8; 32], message: &[u8]) -> (String, String) {
    let key = SigningKey::from_bytes(secret);
    let node_id = data_encoding::HEXLOWER.encode(key.verifying_key().as_bytes());
    let signature = data_encoding::HEXLOWER.encode(&key.sign(message).to_bytes());
    (node_id, signature)
}

// ---------- Discovery ----------

#[derive(Debug, Clone, Deserialize)]
struct Discovery {
    issuer: String,
    #[serde(rename = "deviceAuthorization")]
    device: DeviceEndpoints,
    endpoints: Endpoints,
}

#[derive(Debug, Clone, Deserialize)]
struct DeviceEndpoints {
    #[serde(rename = "clientId")]
    client_id: String,
    #[serde(rename = "deviceCodeEndpoint")]
    device_code: String,
    #[serde(rename = "tokenEndpoint")]
    token: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Endpoints {
    session: String,
    #[serde(rename = "signOut")]
    sign_out: String,
    #[serde(rename = "linkChallenge")]
    link_challenge: String,
    #[serde(rename = "registerDevice")]
    register_device: String,
}

impl Discovery {
    /// Every endpoint must live on the configured server, so a tampered
    /// discovery document cannot send the token somewhere else.
    fn same_origin(&self, server: &str) -> bool {
        let ok = |url: &str| url == server || url.starts_with(&format!("{server}/"));
        ok(&self.issuer)
            && [
                &self.device.device_code,
                &self.device.token,
                &self.endpoints.session,
                &self.endpoints.sign_out,
                &self.endpoints.link_challenge,
                &self.endpoints.register_device,
            ]
            .iter()
            .all(|u| ok(u))
    }
}

async fn discover(http: &reqwest::Client, server: &str) -> Result<Discovery, String> {
    let host = host_of(server);
    let res = http
        .get(format!("{server}/.well-known/commy-auth.json"))
        .send()
        .await
        .map_err(|_| format!("Could not reach {host}. Check your connection or the account server in Settings."))?;
    if !res.status().is_success() {
        return Err(format!("{host} is not a Commy account server"));
    }
    let doc: Discovery = res.json().await.map_err(|_| format!("{host} sent an invalid sign-in description"))?;
    if !doc.same_origin(server) {
        return Err(format!("{host} points sign-in at a different address; refusing for safety"));
    }
    Ok(doc)
}

// ---------- Searches run by the account server ----------

#[derive(Deserialize)]
struct SearchResponse<T> {
    results: Vec<T>,
}

/// Runs a GIF or game search on the account server, which holds the
/// service keys so the app never carries them. `Ok(None)` when nobody is
/// signed in or the server has that search turned off.
pub async fn server_search<T: serde::de::DeserializeOwned>(app: &App, kind: &str, query: &str) -> Result<Option<Vec<T>>, String> {
    let (server, token) = {
        let state = app.account.lock().unwrap();
        if !is_signed_in(&state) {
            return Ok(None);
        }
        match (state.signed_in_server.clone(), state.token.clone()) {
            (Some(server), Some(token)) => (server, token),
            _ => return Ok(None),
        }
    };
    let host = host_of(&server);
    let res = app
        .http
        .get(format!("{server}/api/auth/commy/{kind}/search"))
        .query(&[("q", query)])
        .bearer_auth(token)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|_| format!("Could not reach {host}"))?;
    match res.status().as_u16() {
        200 => {}
        // Older account servers and servers without the key
        404 => return Ok(None),
        401 => return Err(format!("Sign in to {host} again to search")),
        429 => return Err("Too many searches, try again in a minute".into()),
        _ => return Err(format!("{host} could not search right now")),
    }
    let body: SearchResponse<T> = res.json().await.map_err(|_| format!("{host} sent an unexpected reply"))?;
    Ok(Some(body.results))
}

// ---------- Commands ----------

#[derive(Serialize)]
pub struct AccountView {
    signed_in: bool,
    user: Option<AccountUser>,
    server: String,
    server_host: String,
    is_default_server: bool,
    /// Set by COMMY_AUTH_URL, so the setting cannot change it.
    server_from_env: bool,
    device_linked: bool,
    pending_code: Option<String>,
}

fn view(app: &App) -> AccountView {
    let state = app.account.lock().unwrap().clone();
    let server = effective_server();
    AccountView {
        signed_in: is_signed_in(&state),
        user: if is_signed_in(&state) { state.user.clone() } else { None },
        server_host: host_of(&server),
        is_default_server: server == DEFAULT_ACCOUNT_SERVER,
        server_from_env: std::env::var("COMMY_AUTH_URL").ok().and_then(|s| normalize_origin(&s)).is_some(),
        server,
        device_linked: state.device_linked,
        pending_code: app.sign_in.lock().unwrap().as_ref().map(|p| p.user_code.clone()),
    }
}

#[tauri::command]
pub fn account_status(app: tauri::State<'_, App>) -> AccountView {
    view(&app)
}

/// Changes the account server (Settings). Signing out is required first.
#[tauri::command]
pub fn set_account_server(app: tauri::State<'_, App>, server: Option<String>) -> Result<AccountView, String> {
    if is_signed_in(&app.account.lock().unwrap()) {
        return Err("Sign out before changing the account server".into());
    }
    let server = match server.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        None => None,
        Some(s) => {
            let with_scheme = if s.contains("://") { s } else { format!("https://{s}") };
            Some(normalize_origin(&with_scheme).ok_or("Enter an address like https://chat.example.com")?)
        }
    }
    .filter(|s| s != DEFAULT_ACCOUNT_SERVER);
    {
        let mut state = app.account.lock().unwrap();
        state.server = server.clone();
        if let Some(dir) = app.data_dir() {
            save(dir, &state);
        }
    }
    *CHOSEN_SERVER.lock().unwrap() = server;
    Ok(view(&app))
}

/// A device-flow sign-in in progress.
pub struct PendingSignIn {
    pub user_code: String,
    cancel: tokio::sync::watch::Sender<bool>,
}

#[derive(Serialize)]
pub struct SignInStart {
    user_code: String,
    verification_url: String,
    expires_in: u64,
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    #[serde(default = "default_expiry")]
    expires_in: u64,
    #[serde(default = "default_interval")]
    interval: u64,
}

fn default_expiry() -> u64 {
    600
}
fn default_interval() -> u64 {
    5
}

/// Starts sign-in: gets a code, opens the approval page, and polls for
/// the token in the background. Emits "account-changed" when done and
/// "sign-in-failed" with a message if it fails.
#[tauri::command]
pub async fn account_begin_sign_in(app: tauri::State<'_, App>, handle: AppHandle) -> Result<SignInStart, String> {
    let server = effective_server();
    let doc = discover(&app.http, &server).await?;
    let res = app
        .http
        .post(&doc.device.device_code)
        .json(&serde_json::json!({ "client_id": doc.device.client_id }))
        .send()
        .await
        .map_err(|_| format!("Could not reach {}", host_of(&server)))?;
    if !res.status().is_success() {
        return Err("The account server would not start a sign-in. Try again in a minute.".into());
    }
    let code: DeviceCodeResponse = res.json().await.map_err(|_| "Unexpected reply from the account server".to_string())?;
    let verification_url = code.verification_uri_complete.clone().unwrap_or_else(|| code.verification_uri.clone());
    // Only ever open a page on the account server itself.
    if !(verification_url.starts_with(&format!("{server}/"))) {
        return Err("The account server sent an unexpected approval address".into());
    }
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    if let Some(old) = app.sign_in.lock().unwrap().replace(PendingSignIn { user_code: code.user_code.clone(), cancel: cancel_tx }) {
        let _ = old.cancel.send(true);
    }
    let _ = open::that(&verification_url);
    let start = SignInStart { user_code: code.user_code.clone(), verification_url, expires_in: code.expires_in };
    tauri::async_runtime::spawn(poll_for_token(handle.clone(), server, doc, code, cancel_rx));
    Ok(start)
}

#[tauri::command]
pub fn account_cancel_sign_in(app: tauri::State<'_, App>) {
    if let Some(pending) = app.sign_in.lock().unwrap().take() {
        let _ = pending.cancel.send(true);
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    error: Option<String>,
}

async fn poll_for_token(
    handle: AppHandle,
    server: String,
    doc: Discovery,
    code: DeviceCodeResponse,
    mut cancel: tokio::sync::watch::Receiver<bool>,
) {
    let app = handle.state::<App>();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(code.expires_in.min(1800));
    let mut interval = code.interval.clamp(1, 30);
    let fail = |message: &str| {
        let _ = handle.emit("sign-in-failed", message);
    };
    let token = loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(interval)) => {}
            _ = cancel.changed() => return,
        }
        if tokio::time::Instant::now() > deadline {
            fail("The sign-in code expired. Start again.");
            break None;
        }
        let res = app
            .http
            .post(&doc.device.token)
            .json(&serde_json::json!({
                "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
                "device_code": code.device_code,
                "client_id": doc.device.client_id,
            }))
            .send()
            .await;
        let Ok(res) = res else { continue };
        let body: TokenResponse = match res.json().await {
            Ok(b) => b,
            Err(_) => continue,
        };
        if let Some(token) = body.access_token {
            break Some(token);
        }
        match body.error.as_deref() {
            Some("authorization_pending") | None => {}
            Some("slow_down") => interval = (interval + 5).min(30),
            Some("access_denied") => {
                fail("Sign-in was declined in the browser.");
                break None;
            }
            Some("expired_token") => {
                fail("The sign-in code expired. Start again.");
                break None;
            }
            Some(_) => {
                fail("The account server refused the sign-in.");
                break None;
            }
        }
    };
    app.sign_in.lock().unwrap().take();
    let Some(token) = token else { return };
    match finish_sign_in(&app, &server, &doc, &token).await {
        Ok(()) => {
            let _ = handle.emit("account-changed", ());
        }
        Err(e) => fail(&e),
    }
}

#[derive(Deserialize)]
struct SessionResponse {
    user: SessionUser,
}

#[derive(Deserialize)]
struct SessionUser {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    email: String,
}

#[derive(Deserialize)]
struct ChallengeResponse {
    challenge: String,
    message: String,
}

async fn finish_sign_in(app: &App, server: &str, doc: &Discovery, token: &str) -> Result<(), String> {
    let res = app
        .http
        .get(&doc.endpoints.session)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| "Could not load your account".to_string())?;
    let session: SessionResponse = res.json().await.map_err(|_| "The account server did not accept the sign-in".to_string())?;
    let user = AccountUser {
        id: session.user.id,
        name: crate::safety::truncate(&session.user.name, 64),
        email: crate::safety::truncate(&session.user.email, 120),
    };
    // Game lookups cached under the previous account may now resolve differently
    app.igdb.lock().await.resolved.clear();
    let device_linked = link_device(app, server, doc, token).await.map_err(|e| {
        tracing::warn!("device link failed: {e}");
        e
    });
    {
        let mut state = app.account.lock().unwrap();
        state.signed_in_server = Some(server.to_string());
        state.token = Some(token.to_string());
        state.user = Some(user);
        state.device_linked = device_linked.is_ok();
        if let Some(dir) = app.data_dir() {
            save(dir, &state);
        }
    }
    device_linked.map(|_| ())
}

/// Proves this install holds its node key and links the node id.
async fn link_device(app: &App, server: &str, doc: &Discovery, token: &str) -> Result<(), String> {
    let secret = app
        .data_dir()
        .and_then(|d| crate::persist::load_or_create_node_key(d))
        .ok_or("This install has no node key yet")?;
    let (node_id, _) = sign_with_node_key(&secret, b"");
    let res = app
        .http
        .post(&doc.endpoints.link_challenge)
        .bearer_auth(token)
        .json(&serde_json::json!({ "nodeId": node_id }))
        .send()
        .await
        .map_err(|_| "Could not link this device".to_string())?;
    let challenge: ChallengeResponse = res.json().await.map_err(|_| "Could not link this device".to_string())?;
    if !link_message_ok(&challenge.message, server, &node_id, &challenge.challenge) {
        return Err("The account server asked to sign an unexpected message".into());
    }
    let (_, signature) = sign_with_node_key(&secret, challenge.message.as_bytes());
    let name = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "Commy desktop".into());
    let res = app
        .http
        .post(&doc.endpoints.register_device)
        .bearer_auth(token)
        .json(&serde_json::json!({ "nodeId": node_id, "name": name, "challenge": challenge.challenge, "signature": signature }))
        .send()
        .await
        .map_err(|_| "Could not link this device".to_string())?;
    if res.status().is_success() {
        Ok(())
    } else {
        Err("The account server could not verify this device".into())
    }
}

#[tauri::command]
pub async fn account_sign_out(app: tauri::State<'_, App>, handle: AppHandle) -> Result<AccountView, String> {
    let (token, server) = {
        let state = app.account.lock().unwrap();
        (state.token.clone(), state.signed_in_server.clone())
    };
    // Best effort: end the session on the server too.
    if let (Some(token), Some(server)) = (token, server) {
        if let Ok(doc) = discover(&app.http, &server).await {
            let _ = app.http.post(&doc.endpoints.sign_out).bearer_auth(token).send().await;
        }
    }
    {
        let mut state = app.account.lock().unwrap();
        state.token = None;
        state.user = None;
        state.signed_in_server = None;
        state.device_linked = false;
        if let Some(dir) = app.data_dir() {
            save(dir, &state);
        }
    }
    let _ = handle.emit("account-changed", ());
    Ok(view(&app))
}

/// Checks a saved token is still valid; clears it if the server says no.
pub async fn verify_saved_session(handle: AppHandle) {
    let app = handle.state::<App>();
    let (token, server) = {
        let state = app.account.lock().unwrap();
        if !is_signed_in(&state) {
            return;
        }
        (state.token.clone(), state.signed_in_server.clone())
    };
    let (Some(token), Some(server)) = (token, server) else { return };
    let Ok(doc) = discover(&app.http, &server).await else { return };
    let Ok(res) = app.http.get(&doc.endpoints.session).bearer_auth(&token).send().await else { return };
    // A null session (revoked or expired) signs us out; network trouble does not.
    let valid = res.status().is_success() && res.json::<Option<SessionResponse>>().await.ok().flatten().is_some();
    if !valid {
        {
            let mut state = app.account.lock().unwrap();
            state.token = None;
            state.user = None;
            state.signed_in_server = None;
            if let Some(dir) = app.data_dir() {
                save(dir, &state);
            }
        }
        let _ = handle.emit("account-changed", ());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    #[test]
    fn signatures_verify_against_the_node_id() {
        let secret = [7u8; 32];
        let (node_id, sig) = sign_with_node_key(&secret, b"hello");
        let key_bytes: [u8; 32] = data_encoding::HEXLOWER.decode(node_id.as_bytes()).unwrap().try_into().unwrap();
        let sig_bytes: [u8; 64] = data_encoding::HEXLOWER.decode(sig.as_bytes()).unwrap().try_into().unwrap();
        let key = VerifyingKey::from_bytes(&key_bytes).unwrap();
        assert!(key.verify(b"hello", &Signature::from_bytes(&sig_bytes)).is_ok());
        assert!(key.verify(b"other", &Signature::from_bytes(&sig_bytes)).is_err());
        assert_eq!(node_id.len(), 64);
        assert_eq!(sig.len(), 128);
    }

    #[test]
    fn link_message_must_name_this_server() {
        let ok = "commy-device-link:v1\nhttps://commy.chat\nab\n123.x";
        assert!(link_message_ok(ok, "https://commy.chat", "ab", "123.x"));
        assert!(!link_message_ok(ok, "https://evil.example", "ab", "123.x"));
        assert!(!link_message_ok("sign this bank transfer", "https://commy.chat", "ab", "123.x"));
    }

    fn doc(base: &str) -> Discovery {
        let api = format!("{base}/api/auth");
        Discovery {
            issuer: base.into(),
            device: DeviceEndpoints {
                client_id: "commy-desktop".into(),
                device_code: format!("{api}/device/code"),
                token: format!("{api}/device/token"),
            },
            endpoints: Endpoints {
                session: format!("{api}/get-session"),
                sign_out: format!("{api}/sign-out"),
                link_challenge: format!("{api}/commy/devices/challenge"),
                register_device: format!("{api}/commy/devices/register"),
            },
        }
    }

    #[test]
    fn discovery_must_stay_on_the_server() {
        assert!(doc("https://commy.chat").same_origin("https://commy.chat"));
        assert!(!doc("https://evil.example").same_origin("https://commy.chat"));
        let mut sneaky = doc("https://commy.chat");
        sneaky.device.token = "https://commy.chat.evil.example/token".into();
        assert!(!sneaky.same_origin("https://commy.chat"));
    }

    #[test]
    fn a_token_for_another_server_does_not_count() {
        let mut state = AccountState {
            token: Some("t".into()),
            user: Some(AccountUser { id: "u".into(), name: "n".into(), email: "e".into() }),
            signed_in_server: Some(effective_server()),
            ..Default::default()
        };
        assert!(is_signed_in(&state));
        state.signed_in_server = Some("https://other.example".into());
        assert!(!is_signed_in(&state));
        state.signed_in_server = Some(effective_server());
        state.token = None;
        assert!(!is_signed_in(&state));
    }
}
