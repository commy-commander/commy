use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::account_folder;
use crate::invite_link::{normalize_origin, DEFAULT_ACCOUNT_SERVER};
use crate::state::App;

const FILE: &str = "account.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountState {
    #[serde(default)]
    pub server: Option<String>,
    #[serde(default)]
    pub signed_in_server: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub user: Option<AccountUser>,
    #[serde(default)]
    pub device_linked: bool,
    #[serde(default)]
    pub features: Option<ServerFeatures>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerFeatures {
    #[serde(default)]
    pub gifs: bool,
    #[serde(default)]
    pub games: bool,
    #[serde(default)]
    pub emojis: bool,
    #[serde(default)]
    pub media: bool,
    #[serde(default)]
    pub vault: bool,
    #[serde(default)]
    pub stickers: bool,
}

impl AccountState {
    pub fn credentials(&self, kind: &str) -> Option<(String, String)> {
        if !self.server_offers(kind) {
            return None;
        }
        Some((self.signed_in_server.clone()?, self.token.clone()?))
    }

    pub fn clear_session(&mut self) {
        self.token = None;
        self.user = None;
        self.signed_in_server = None;
        self.features = None;
    }

    pub fn server_offers(&self, kind: &str) -> bool {
        is_signed_in(self)
            && match (self.features, kind) {
                (Some(f), "media") => f.media,
                (None, "media") => false,
                (None, _) => true,
                (Some(f), "gifs") => f.gifs,
                (Some(f), "games") => f.games,
                (Some(f), "emojis") => f.emojis,
                (Some(f), "stickers") => f.stickers,
                _ => false,
            }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccountUser {
    pub id: String,
    pub name: String,
    pub email: String,
}

static CHOSEN_SERVER: Mutex<Option<String>> = Mutex::new(None);

static ENV_SERVER: LazyLock<Option<String>> =
    LazyLock::new(|| std::env::var("COMMY_AUTH_URL").ok().and_then(|s| normalize_origin(&s)));

pub fn effective_server() -> String {
    if let Some(env) = ENV_SERVER.clone() {
        return env;
    }
    CHOSEN_SERVER.lock().unwrap().clone().unwrap_or_else(|| DEFAULT_ACCOUNT_SERVER.to_string())
}

fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

pub fn load(dir: &Path) -> AccountState {
    let state: AccountState = commy_core::at_rest::read_to_string(&path(dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    *CHOSEN_SERVER.lock().unwrap() = state.server.clone();
    state
}

fn store(app: &App, state: &AccountState) {
    let Some(dir) = app.base_dir() else { return };
    if let Err(e) = crate::persist::write_secret_json(&path(dir), state) {
        tracing::warn!("could not save the account: {e}");
    }
}

pub fn is_signed_in(state: &AccountState) -> bool {
    state.token.is_some() && state.user.is_some() && state.signed_in_server.as_deref() == Some(effective_server().as_str())
}

fn signed_in_folder(app: &App) -> Option<(PathBuf, String, String)> {
    let state = app.account.lock().unwrap();
    if !is_signed_in(&state) {
        return None;
    }
    Some((app.base_dir()?.clone(), state.signed_in_server.clone()?, state.user.as_ref()?.id.clone()))
}

pub fn ready_folder(app: &App) -> Option<PathBuf> {
    let (base, server, account) = signed_in_folder(app)?;
    match account_folder::choose(&base, &server, &account) {
        account_folder::Opening::Ready(dir) => Some(dir),
        account_folder::Opening::AskAboutUnclaimed => None,
    }
}

static MAY_CLAIM: Mutex<Option<String>> = Mutex::new(None);

fn may_claim(unclaimed_node: Option<&str>, linked_devices: &[String]) -> bool {
    unclaimed_node.is_some_and(|node| linked_devices.iter().any(|device| device.eq_ignore_ascii_case(node)))
}

fn claim_allowed(account: &str) -> bool {
    MAY_CLAIM.lock().unwrap().as_deref() == Some(account)
}

fn unclaimed_profile(app: &App) -> Option<account_folder::UnclaimedProfile> {
    if app.data_dir().is_some() {
        return None;
    }
    let (base, server, account) = signed_in_folder(app)?;
    if !claim_allowed(&account) {
        return None;
    }
    match account_folder::choose(&base, &server, &account) {
        account_folder::Opening::AskAboutUnclaimed => account_folder::unclaimed(&base),
        account_folder::Opening::Ready(_) => None,
    }
}

fn restart_signed_in_as_someone_else(handle: &AppHandle) -> ! {
    if let Some(base) = handle.state::<App>().base_dir() {
        account_folder::show_window_on_next_start(base);
    }
    handle.restart()
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

pub fn link_message_ok(message: &str, server: &str, node_id: &str, challenge: &str) -> bool {
    message == format!("commy-device-link:v1\n{server}\n{node_id}\n{challenge}")
}

pub fn sign_with_node_key(secret: &[u8; 32], message: &[u8]) -> (String, String) {
    let key = SigningKey::from_bytes(secret);
    let signature = data_encoding::HEXLOWER.encode(&key.sign(message).to_bytes());
    (node_id_of(&key), signature)
}

fn node_id_of(key: &SigningKey) -> String {
    data_encoding::HEXLOWER.encode(key.verifying_key().as_bytes())
}

#[derive(Debug, Clone, Deserialize)]
struct Discovery {
    issuer: String,
    #[serde(rename = "deviceAuthorization")]
    device: DeviceEndpoints,
    endpoints: Endpoints,
    #[serde(default)]
    features: Option<ServerFeatures>,
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

#[derive(Deserialize)]
struct SearchResponse<T> {
    results: Vec<T>,
}

pub async fn server_search<T: serde::de::DeserializeOwned>(app: &App, kind: &str, query: &str) -> Result<Option<Vec<T>>, String> {
    let found: Option<SearchResponse<T>> = server_search_with(app, kind, &[("q", query)]).await?;
    Ok(found.map(|body| body.results))
}

pub async fn server_search_with<T: serde::de::DeserializeOwned>(
    app: &App,
    kind: &str,
    query: &[(&str, &str)],
) -> Result<Option<T>, String> {
    let credentials = app.account.lock().unwrap().credentials(kind);
    let Some((server, token)) = credentials else { return Ok(None) };
    let host = host_of(&server);
    let res = app
        .http
        .get(format!("{server}/api/auth/commy/{kind}/search"))
        .query(query)
        .bearer_auth(token)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|_| format!("Could not reach {host}"))?;
    match res.status().as_u16() {
        200 => {}
        404 => return Ok(None),
        401 => return Err(format!("Sign in to {host} again to search")),
        429 => return Err("Too many searches, try again in a minute".into()),
        _ => return Err(format!("{host} could not search right now")),
    }
    let body: T = res.json().await.map_err(|_| format!("{host} sent an unexpected reply"))?;
    Ok(Some(body))
}

const MAX_MEDIA_BYTES: usize = 15 * 1024 * 1024;

fn media_response(status: u16, content_type: &str, body: Vec<u8>) -> tauri::http::Response<Vec<u8>> {
    tauri::http::Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .header("Cache-Control", "private, max-age=86400")
        .header("X-Content-Type-Options", "nosniff")
        .body(body)
        .unwrap_or_default()
}

pub fn media_target(request_uri: &str) -> Option<String> {
    let uri = reqwest::Url::parse(request_uri).ok()?;
    let target = uri.query_pairs().find(|(k, _)| k == "u")?.1.into_owned();
    target.starts_with("https://").then_some(target)
}

pub async fn media_image(app: &App, request_uri: &str) -> tauri::http::Response<Vec<u8>> {
    let Some(target) = media_target(request_uri) else {
        return media_response(400, "text/plain", Vec::new());
    };
    match fetch_media(app, &target).await {
        Ok((content_type, body)) => media_response(200, &content_type, body),
        Err(status) => media_response(status, "text/plain", Vec::new()),
    }
}

pub async fn fetch_media(app: &App, target: &str) -> Result<(String, Vec<u8>), u16> {
    let credentials = app.account.lock().unwrap().credentials("media");
    let Some((server, token)) = credentials else { return Err(404) };
    let res = app
        .http
        .get(format!("{server}/api/auth/commy/media"))
        .query(&[("u", target)])
        .bearer_auth(token)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| 502u16)?;
    if !res.status().is_success() {
        return Err(res.status().as_u16());
    }
    let content_type = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if !content_type.starts_with("image/") || res.content_length().unwrap_or(0) as usize > MAX_MEDIA_BYTES {
        return Err(502);
    }
    match res.bytes().await {
        Ok(body) if body.len() <= MAX_MEDIA_BYTES => Ok((content_type, body.to_vec())),
        _ => Err(502),
    }
}

#[derive(Serialize)]
pub struct AccountView {
    signed_in: bool,
    user: Option<AccountUser>,
    server: String,
    server_host: String,
    is_default_server: bool,
    default_server_host: String,
    server_from_env: bool,
    device_linked: bool,
    pending_code: Option<String>,
    server_gifs: bool,
    server_games: bool,
    server_emojis: bool,
    server_stickers: bool,
    server_media: bool,
    unclaimed_profile: Option<account_folder::UnclaimedProfile>,
    needs_account_folder: bool,
}

fn view(app: &App) -> AccountView {
    let unclaimed_profile = unclaimed_profile(app);
    let pending_code = app.sign_in.lock().unwrap().as_ref().map(|p| p.user_code.clone());
    let state = app.account.lock().unwrap();
    let server = effective_server();
    let signed_in = is_signed_in(&state);
    AccountView {
        signed_in,
        user: if signed_in { state.user.clone() } else { None },
        server_host: host_of(&server),
        is_default_server: server == DEFAULT_ACCOUNT_SERVER,
        default_server_host: host_of(DEFAULT_ACCOUNT_SERVER),
        server_from_env: ENV_SERVER.is_some(),
        server,
        device_linked: state.device_linked,
        pending_code,
        server_gifs: state.server_offers("gifs"),
        server_games: state.server_offers("games"),
        server_emojis: state.server_offers("emojis"),
        server_stickers: state.server_offers("stickers"),
        server_media: state.server_offers("media"),
        unclaimed_profile,
        needs_account_folder: signed_in && app.data_dir().is_none(),
    }
}

#[tauri::command]
pub fn account_status(app: tauri::State<'_, App>) -> AccountView {
    view(&app)
}

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
        store(&app, &state);
    }
    *CHOSEN_SERVER.lock().unwrap() = server;
    Ok(view(&app))
}

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
    match finish_sign_in(&handle, &server, &doc, &token).await {
        Ok(()) => {
            let _ = handle.emit("account-changed", ());
            crate::entry_list::schedule(&handle);
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

async fn finish_sign_in(handle: &AppHandle, server: &str, doc: &Discovery, token: &str) -> Result<(), String> {
    let app = handle.state::<App>();
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
    let base = app.base_dir().cloned().ok_or("This install has no data folder")?;
    let opening = account_folder::choose(&base, server, &user.id);
    let linking = match &opening {
        account_folder::Opening::Ready(dir) => Some(dir.clone()),
        account_folder::Opening::AskAboutUnclaimed => None,
    };
    let next = after_sign_in(opening, app.data_dir().map(|d| d.as_path()));
    app.igdb.lock().await.resolved.clear();
    let device_linked = match &linking {
        Some(dir) => link_device(&app, dir, server, doc, token).await.map_err(|e| {
            tracing::warn!("device link failed: {e}");
            e
        }),
        None => Ok(()),
    };
    {
        let mut state = app.account.lock().unwrap();
        state.signed_in_server = Some(server.to_string());
        state.token = Some(token.to_string());
        state.user = Some(user);
        state.device_linked = device_linked.is_ok() && linking.is_some();
        state.features = doc.features;
        store(&app, &state);
    }
    match next {
        AfterSignIn::Restart => restart_signed_in_as_someone_else(handle),
        AfterSignIn::Open(dir) => crate::open_account(handle, dir),
        AfterSignIn::KeepOpen | AfterSignIn::Ask => {}
    }
    device_linked.map(|_| ())
}

#[derive(Debug, PartialEq)]
enum AfterSignIn {
    Open(PathBuf),
    KeepOpen,
    Ask,
    Restart,
}

fn after_sign_in(opening: account_folder::Opening, open: Option<&Path>) -> AfterSignIn {
    match (opening, open) {
        (account_folder::Opening::Ready(dir), Some(open)) if dir == open => AfterSignIn::KeepOpen,
        (account_folder::Opening::Ready(dir), None) => AfterSignIn::Open(dir),
        (account_folder::Opening::AskAboutUnclaimed, None) => AfterSignIn::Ask,
        _ => AfterSignIn::Restart,
    }
}

async fn link_device(app: &App, dir: &Path, server: &str, doc: &Discovery, token: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|_| "Could not create this account's folder".to_string())?;
    let secret = crate::persist::load_or_create_node_key(dir).ok_or("This install has no node key yet")?;
    let node_id = node_id_of(&SigningKey::from_bytes(&secret));
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
    let res = app
        .http
        .post(&doc.endpoints.register_device)
        .bearer_auth(token)
        .json(&serde_json::json!({ "nodeId": node_id, "challenge": challenge.challenge, "signature": signature }))
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
    if let (Some(token), Some(server)) = (token, server) {
        if let Ok(doc) = discover(&app.http, &server).await {
            let _ = app.http.post(&doc.endpoints.sign_out).bearer_auth(token).send().await;
        }
    }
    {
        let mut state = app.account.lock().unwrap();
        state.clear_session();
        state.device_linked = false;
        store(&app, &state);
    }
    if app.data_dir().is_some() {
        restart_signed_in_as_someone_else(&handle);
    }
    let _ = handle.emit("account-changed", ());
    Ok(view(&app))
}

#[tauri::command]
pub async fn account_resolve_unclaimed(app: tauri::State<'_, App>, handle: AppHandle) -> Result<AccountView, String> {
    if app.data_dir().is_some() {
        return Ok(view(&app));
    }
    let (base, server, account) = signed_in_folder(&app).ok_or("Sign in first")?;
    if account_folder::choose(&base, &server, &account) == account_folder::Opening::AskAboutUnclaimed {
        let unclaimed_node =
            account_folder::unclaimed_node_secret(&base).map(|secret| node_id_of(&SigningKey::from_bytes(&secret)));
        let session = crate::entry_list::session(&app).ok_or("Sign in first")?;
        let linked = crate::entry_list::account_devices(&app, &session)
            .await
            .map_err(|e| format!("Could not check this computer with your account server. {e}"))?;
        if may_claim(unclaimed_node.as_deref(), &linked) {
            *MAY_CLAIM.lock().unwrap() = Some(account);
            return Ok(view(&app));
        }
    }
    open_folder(&app, &handle, &base, &server, &account, false).await
}

#[tauri::command]
pub async fn account_claim_profile(app: tauri::State<'_, App>, handle: AppHandle, keep: bool) -> Result<AccountView, String> {
    if app.data_dir().is_some() {
        return Ok(view(&app));
    }
    let (base, server, account) = signed_in_folder(&app).ok_or("Sign in first")?;
    if keep && !claim_allowed(&account) {
        return Err("That profile was never used with this account on this computer, so it stays put.".into());
    }
    open_folder(&app, &handle, &base, &server, &account, keep).await
}

async fn open_folder(
    app: &App,
    handle: &AppHandle,
    base: &Path,
    server: &str,
    account: &str,
    keep: bool,
) -> Result<AccountView, String> {
    let dir = account_folder::claim(base, server, account, keep)
        .map_err(|e| format!("Could not set up this account's folder: {e}"))?;
    *MAY_CLAIM.lock().unwrap() = None;
    let token = app.account.lock().unwrap().token.clone().ok_or("Sign in first")?;
    let linked = match discover(&app.http, server).await {
        Ok(doc) => link_device(app, &dir, server, &doc, &token).await.is_ok(),
        Err(_) => false,
    };
    {
        let mut state = app.account.lock().unwrap();
        state.device_linked = linked;
        store(app, &state);
    }
    crate::open_account(handle, dir);
    let _ = handle.emit("account-changed", ());
    Ok(view(app))
}

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
    let valid = res.status().is_success() && res.json::<Option<SessionResponse>>().await.ok().flatten().is_some();
    if valid {
        let changed = {
            let mut state = app.account.lock().unwrap();
            let changed = state.features != doc.features;
            if changed {
                state.features = doc.features;
                store(&app, &state);
            }
            changed
        };
        if changed {
            let _ = handle.emit("account-changed", ());
        }
        return;
    }
    {
        let mut state = app.account.lock().unwrap();
        state.clear_session();
        store(&app, &state);
    }
    let _ = handle.emit("account-changed", ());
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
            features: None,
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

    #[test]
    fn searches_follow_the_server_feature_list() {
        let mut state = AccountState {
            token: Some("t".into()),
            user: Some(AccountUser { id: "u".into(), name: "n".into(), email: "e".into() }),
            signed_in_server: Some(effective_server()),
            ..Default::default()
        };
        assert!(state.server_offers("gifs") && state.server_offers("games"));
        assert!(!state.server_offers("media"));
        state.features = Some(ServerFeatures { gifs: true, games: false, emojis: true, media: true, vault: false, stickers: true });
        assert!(state.server_offers("gifs"));
        assert!(state.server_offers("stickers"));
        assert!(state.server_offers("emojis"));
        assert!(!state.server_offers("games"));
        assert!(state.server_offers("media"));
        state.token = None;
        assert!(!state.server_offers("gifs"));
    }

    #[test]
    fn discovery_features_are_optional() {
        let base = serde_json::json!({
            "issuer": "https://chat.example.org",
            "deviceAuthorization": { "clientId": "c", "deviceCodeEndpoint": "x", "tokenEndpoint": "x" },
            "endpoints": { "session": "x", "signOut": "x", "linkChallenge": "x", "registerDevice": "x" },
        });
        let old: Discovery = serde_json::from_value(base.clone()).unwrap();
        assert_eq!(old.features, None);
        let mut with = base;
        with["features"] = serde_json::json!({ "gifs": true, "games": false });
        let new: Discovery = serde_json::from_value(with).unwrap();
        assert_eq!(new.features, Some(ServerFeatures { gifs: true, games: false, emojis: false, media: false, vault: false, stickers: false }));
    }

    #[test]
    fn signing_in_as_someone_else_never_keeps_the_open_account() {
        use account_folder::Opening;
        let a = PathBuf::from("accounts").join("a");
        let b = PathBuf::from("accounts").join("b");
        assert_eq!(after_sign_in(Opening::Ready(a.clone()), Some(&a)), AfterSignIn::KeepOpen);
        assert_eq!(after_sign_in(Opening::Ready(b.clone()), Some(&a)), AfterSignIn::Restart);
        assert_eq!(after_sign_in(Opening::AskAboutUnclaimed, Some(&a)), AfterSignIn::Restart);
        assert_eq!(after_sign_in(Opening::Ready(b.clone()), None), AfterSignIn::Open(b));
        assert_eq!(after_sign_in(Opening::AskAboutUnclaimed, None), AfterSignIn::Ask);
    }

    #[test]
    fn only_an_account_already_linked_to_the_old_profile_may_take_it() {
        let linked = vec!["AB12".to_string(), "cd34".to_string()];
        assert!(may_claim(Some("ab12"), &linked));
        assert!(may_claim(Some("CD34"), &linked));
        assert!(!may_claim(Some("ef56"), &linked));
        assert!(!may_claim(None, &linked));
        assert!(!may_claim(Some("ab12"), &[]));
    }

    #[test]
    fn media_requests_name_one_https_image() {
        let gif = "https://static.klipy.com/ii/a/b.gif";
        let encoded = "https%3A%2F%2Fstatic.klipy.com%2Fii%2Fa%2Fb.gif";
        assert_eq!(media_target(&format!("commy-media://localhost/?u={encoded}")).as_deref(), Some(gif));
        assert_eq!(media_target(&format!("http://commy-media.localhost/?u={encoded}")).as_deref(), Some(gif));
        assert_eq!(media_target("commy-media://localhost/?u=file%3A%2F%2F%2Fetc%2Fpasswd"), None);
        assert_eq!(media_target("commy-media://localhost/"), None);
    }
}
