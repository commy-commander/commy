use crate::lock_or_recover::LockOrRecover;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::AppHandle;

use crate::account::{complete_sign_in, discover, host_of, room_for_another_account, sign_in_server, Discovery};
use crate::state::App;

const CHALLENGE_LIFETIME: Duration = Duration::from_secs(9 * 60);
const MAX_EMAIL: usize = 254;
const MAX_PASSWORD: usize = 128;
const MIN_NEW_PASSWORD: usize = 8;
const MAX_NAME: usize = 64;
const SIGN_UP_RETURN: &str = "/account?app=1";
const MAX_CODE: usize = 40;
const TOTP_DIGITS: usize = 6;
const ACCESS_COOKIE: &str = "commy_access";
const SHEET_PROVIDERS: [&str; 2] = ["google", "twitch"];
const MAX_SUSPENDED_NOTE: usize = 240;
const SUSPENDED_FALLBACK: &str = "This account is suspended for breaking the Terms. If you think this is a mistake, write to this account server's contact.";
const SHEET_RETURN_SCHEME: &str = "commy";
const APP_LINK_SCHEME: &str = match option_env!("COMMY_LINK_SCHEME") {
    Some(scheme) => scheme,
    None => SHEET_RETURN_SCHEME,
};
const SHEET_RETURN_HOST: &str = "signed-in";
const MAX_HANDOFF: usize = 64;
const IOS_PASSKEY_SITES: [&str; 1] = ["commy.chat"];
#[cfg(desktop)]
const BROWSER_WAIT: Duration = Duration::from_secs(10 * 60);

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);
static BROWSER_RETURN: Mutex<Option<tokio::sync::oneshot::Sender<Option<String>>>> = Mutex::new(None);

#[derive(Clone)]
struct Pending {
    server: String,
    started: Instant,
    challenge: Option<String>,
    apple: Option<AppleAttempt>,
    access: Option<String>,
}

impl Pending {
    fn new(server: &str) -> Self {
        Pending { server: server.to_string(), started: Instant::now(), challenge: None, apple: None, access: None }
    }

    fn fresh_for(&self, server: &str) -> bool {
        self.server == server && self.started.elapsed() < CHALLENGE_LIFETIME
    }
}

#[derive(Clone)]
struct AppleAttempt {
    token: String,
    nonce: String,
    given_name: String,
    family_name: String,
}

fn pending_for(server: &str) -> Pending {
    PENDING.lock_or_recover().clone().filter(|p| p.fresh_for(server)).unwrap_or_else(|| Pending::new(server))
}

fn keep(pending: Pending) {
    *PENDING.lock_or_recover() = Some(pending);
}

pub fn forget() {
    PENDING.lock_or_recover().take();
}

#[derive(Serialize)]
pub struct InAppOptions {
    password: bool,
    apple: bool,
    google: bool,
    twitch: bool,
    passkey: bool,
    passkey_in_browser: bool,
    browser_sheet: bool,
    sign_up: bool,
    captcha_url: Option<String>,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum SignInStep {
    Done,
    Code,
    NewAccount,
    ConfirmAdult,
    AccessCode,
    CheckEmail,
    Canceled,
}

#[derive(Debug, PartialEq)]
enum Reply {
    Token(String),
    TwoFactor,
    NoAccount,
    NeedsAdultConfirmation,
    NeedsAccessCode,
    CheckEmail,
}

struct Answer {
    status: u16,
    body: Value,
    token: Option<String>,
    cookies: Vec<String>,
}

fn offers_password(doc: &Discovery) -> bool {
    doc.providers.email_password && doc.endpoints.sign_in_email.is_some() && doc.endpoints.two_factor_verify.is_some()
}

fn offers_apple(doc: &Discovery) -> bool {
    cfg!(target_os = "ios") && doc.providers.apple_app && doc.endpoints.sign_in_social.is_some()
}

fn offers_sheet(doc: &Discovery, provider: &str) -> bool {
    let offered = match provider {
        "google" => doc.providers.google,
        "twitch" => doc.providers.twitch,
        _ => false,
    };
    offered && doc.endpoints.app_sign_in_start.is_some() && doc.endpoints.app_sign_in_finish.is_some()
}

fn offers_passkey(doc: &Discovery, server: &str) -> bool {
    let linked_site = !cfg!(target_os = "ios") || IOS_PASSKEY_SITES.contains(&host_of(server).as_str());
    cfg!(mobile) && linked_site && doc.providers.passkey && doc.endpoints.passkey_options.is_some() && doc.endpoints.passkey_verify.is_some()
}

fn captcha_url(doc: &Discovery) -> Option<String> {
    doc.turnstile_site_key.as_ref().and(doc.endpoints.captcha.clone())
}

fn cookie_pairs<'a>(set_cookies: impl Iterator<Item = &'a str>) -> Vec<String> {
    set_cookies
        .filter_map(|header| {
            let mut parts = header.split(';').map(str::trim);
            let pair = parts.next()?;
            let removed = parts.any(|a| a.eq_ignore_ascii_case("max-age=0"));
            let (name, value) = pair.split_once('=')?;
            (!removed && !value.is_empty()).then(|| format!("{name}={value}"))
        })
        .collect()
}

fn suspended_note(message: &str) -> String {
    let note: String = message.chars().filter(|c| !c.is_control()).take(MAX_SUSPENDED_NOTE).collect();
    if note.trim().is_empty() {
        SUSPENDED_FALLBACK.into()
    } else {
        note
    }
}

fn read_reply(answer: &Answer) -> Result<Reply, String> {
    if (200..300).contains(&answer.status) {
        if answer.body.get("twoFactorRedirect").and_then(Value::as_bool) == Some(true) {
            return Ok(Reply::TwoFactor);
        }
        return answer.token.clone().map(Reply::Token).ok_or_else(|| "The account server did not accept the sign-in".into());
    }
    let code = answer.body.get("code").and_then(Value::as_str).unwrap_or_default();
    let message = answer.body.get("message").and_then(Value::as_str).unwrap_or_default();
    match code {
        "EMAIL_NOT_VERIFIED" => return Ok(Reply::CheckEmail),
        "ACCESS_CODE_REQUIRED" => return Ok(Reply::NeedsAccessCode),
        "ADULT_CONFIRMATION_REQUIRED" => return Ok(Reply::NeedsAdultConfirmation),
        "OAUTH_LINK_ERROR" if message == "signup disabled" => return Ok(Reply::NoAccount),
        "OAUTH_LINK_ERROR" if message == "account not linked" => {
            return Err("An account already uses this email. Sign in with your email and password, then confirm your email if asked.".into())
        }
        "SIGNUP_DISABLED" | "EMAIL_PASSWORD_SIGN_UP_DISABLED" => return Err("This account server isn't taking new accounts right now.".into()),
        "USER_ALREADY_EXISTS" | "USER_ALREADY_EXISTS_USE_ANOTHER_EMAIL" => return Err("An account already uses this email. Sign in instead.".into()),
        "PASSWORD_TOO_SHORT" => return Err("Use a password with at least 8 characters.".into()),
        "PASSWORD_TOO_LONG" => return Err("Use a password with 128 characters or fewer.".into()),
        "ACCOUNT_SUSPENDED" => return Err(suspended_note(message)),
        "THROWAWAY_EMAIL" => return Err("Use an email address you keep. Throwaway inboxes can't make accounts.".into()),
        "ACCOUNT_NOT_LINKED" => {
            return Err("An account already uses this email. Sign in with your email and password, then confirm your email if asked.".into())
        }
        "PASSKEY_NOT_FOUND" => return Err("That passkey isn't on a Commy account here.".into()),
        "AUTHENTICATION_FAILED" => return Err("The passkey didn't work. Try again.".into()),
        "CHALLENGE_NOT_FOUND" => return Err("That sign-in timed out. Start again.".into()),
        "INVALID_EMAIL_OR_PASSWORD" | "INVALID_EMAIL" | "INVALID_PASSWORD" => return Err("That email and password don't match.".into()),
        "WRONG_ACCESS_CODE" => return Err("That access code isn't right.".into()),
        "INVALID_CODE" | "INVALID_BACKUP_CODE" => return Err("That code isn't right. Try the newest one.".into()),
        "INVALID_TWO_FACTOR_COOKIE"
        | "TOO_MANY_ATTEMPTS_REQUEST_NEW_CODE"
        | "STATE_MISMATCH"
        | "STATE_NOT_FOUND"
        | "PLEASE_RESTART_THE_PROCESS"
        | "NO_SESSION"
        | "HANDOFF_NOT_FOUND" => return Err("That sign-in timed out. Start again.".into()),
        "VERIFICATION_FAILED" | "MISSING_RESPONSE" => return Err("The quick safety check didn't go through. Try it again.".into()),
        _ => {}
    }
    if answer.status == 429 {
        return Err("Too many tries. Wait a minute, then try again.".into());
    }
    let shown: String = code.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(40).collect();
    if shown.is_empty() {
        return Err("The account server refused the sign-in. Try again, or use your browser instead.".into());
    }
    Err(format!("The account server refused the sign-in ({shown}). Try again, or use your browser instead."))
}

async fn post(app: &App, server: &str, url: &str, body: Value, cookie: Option<&str>, captcha: Option<&str>) -> Result<Answer, String> {
    let mut request = app.http.post(url).header(reqwest::header::ORIGIN, server).json(&body);
    if let Some(cookie) = cookie.filter(|c| !c.is_empty()) {
        request = request.header(reqwest::header::COOKIE, cookie);
    }
    if let Some(captcha) = captcha {
        request = request.header("x-captcha-response", captcha);
    }
    let res = request.send().await.map_err(|_| format!("Could not reach {}. Check your connection.", host_of(server)))?;
    let status = res.status().as_u16();
    let headers = res.headers().clone();
    let token = headers.get("set-auth-token").and_then(|v| v.to_str().ok()).filter(|t| !t.is_empty()).map(str::to_string);
    let cookies = cookie_pairs(headers.get_all(reqwest::header::SET_COOKIE).iter().filter_map(|v| v.to_str().ok()));
    let body: Value = res.json().await.unwrap_or(Value::Null);
    let token = token.or_else(|| body.get("token").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string));
    Ok(Answer { status, body, token, cookies })
}

async fn current_server(app: &App) -> Result<(String, Discovery), String> {
    room_for_another_account(app)?;
    let server = sign_in_server();
    let doc = discover(&app.http, &server).await?;
    Ok((server, doc))
}

async fn settle(handle: &AppHandle, server: &str, doc: &Discovery, mut pending: Pending, answer: Answer) -> Result<SignInStep, String> {
    match read_reply(&answer)? {
        Reply::Token(token) => {
            forget();
            complete_sign_in(handle, server, doc, &token).await?;
            Ok(SignInStep::Done)
        }
        Reply::TwoFactor => {
            pending.challenge = Some(answer.cookies.join("; "));
            keep(pending);
            Ok(SignInStep::Code)
        }
        Reply::NoAccount => {
            keep(pending);
            Ok(SignInStep::NewAccount)
        }
        Reply::NeedsAdultConfirmation => {
            keep(pending);
            Ok(SignInStep::ConfirmAdult)
        }
        Reply::NeedsAccessCode => {
            pending.access = None;
            keep(pending);
            Ok(SignInStep::AccessCode)
        }
        Reply::CheckEmail => {
            forget();
            Ok(SignInStep::CheckEmail)
        }
    }
}

#[tauri::command]
pub async fn account_in_app_options(app: tauri::State<'_, App>) -> Result<InAppOptions, String> {
    let server = sign_in_server();
    let doc = discover(&app.http, &server).await?;
    Ok(InAppOptions {
        password: offers_password(&doc),
        apple: offers_apple(&doc),
        google: offers_sheet(&doc, "google"),
        twitch: offers_sheet(&doc, "twitch"),
        passkey: offers_passkey(&doc, &server),
        passkey_in_browser: cfg!(desktop) && doc.providers.passkey,
        browser_sheet: cfg!(desktop),
        sign_up: sign_up_url(&doc).is_some(),
        captcha_url: captcha_url(&doc),
    })
}

#[tauri::command]
pub async fn account_sign_in_password(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    email: String,
    password: String,
    captcha: Option<String>,
) -> Result<SignInStep, String> {
    let email = email.trim().to_string();
    if email.is_empty() || email.len() > MAX_EMAIL || !email.contains('@') {
        return Err("Enter the email for your account.".into());
    }
    if password.is_empty() || password.len() > MAX_PASSWORD {
        return Err("Enter your password.".into());
    }
    let (server, doc) = current_server(&app).await?;
    let url = doc.endpoints.sign_in_email.clone().filter(|_| offers_password(&doc)).ok_or("This account server needs your browser to sign in.")?;
    if captcha_url(&doc).is_some() && captcha.as_deref().unwrap_or_default().is_empty() {
        return Err("Finish the quick safety check first.".into());
    }
    let answer = post(&app, &server, &url, json!({ "email": email, "password": password }), None, captcha.as_deref()).await?;
    settle(&handle, &server, &doc, Pending::new(&server), answer).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn account_sign_up_email(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    email: String,
    password: String,
    name: String,
    adult_confirmed: bool,
    captcha: Option<String>,
    access_code: Option<String>,
) -> Result<SignInStep, String> {
    let fields = SignUpFields::checked(&email, &password, &name, adult_confirmed)?;
    let (server, doc) = current_server(&app).await?;
    let url = sign_up_url(&doc).ok_or("This account server needs your browser to create an account.")?;
    if captcha_url(&doc).is_some() && captcha.as_deref().unwrap_or_default().is_empty() {
        return Err("Finish the quick safety check first.".into());
    }
    let mut pending = pending_for(&server);
    if let Some(code) = access_code {
        pending.access = Some(check_access_code(&app, &server, &doc, &code).await?);
    }
    let cookie = pending.access.as_ref().map(|t| format!("{ACCESS_COOKIE}={t}"));
    let answer = post(&app, &server, &url, fields.body(), cookie.as_deref(), captcha.as_deref()).await?;
    if waits_for_email(&answer) {
        forget();
        return Ok(SignInStep::CheckEmail);
    }
    settle(&handle, &server, &doc, pending, answer).await
}

#[derive(Debug, PartialEq)]
struct SignUpFields {
    email: String,
    password: String,
    name: String,
}

impl SignUpFields {
    fn checked(email: &str, password: &str, name: &str, adult_confirmed: bool) -> Result<Self, String> {
        let email = email.trim();
        if email.is_empty() || email.len() > MAX_EMAIL || !email.contains('@') {
            return Err("Enter the email you want to use.".into());
        }
        let name = name.trim();
        if name.is_empty() {
            return Err("Enter a display name.".into());
        }
        if name.chars().count() > MAX_NAME {
            return Err("Use a display name with 64 characters or fewer.".into());
        }
        let length = password.chars().count();
        if length < MIN_NEW_PASSWORD {
            return Err("Use a password with at least 8 characters.".into());
        }
        if password.len() > MAX_PASSWORD {
            return Err("Use a password with 128 characters or fewer.".into());
        }
        if !adult_confirmed {
            return Err("Confirm you are 18 or older and agree to the Terms.".into());
        }
        Ok(SignUpFields { email: email.to_string(), password: password.to_string(), name: name.to_string() })
    }

    fn body(&self) -> Value {
        json!({ "email": self.email, "password": self.password, "name": self.name, "adultConfirmed": true, "callbackURL": SIGN_UP_RETURN })
    }
}

fn sign_up_url(doc: &Discovery) -> Option<String> {
    if !offers_password(doc) {
        return None;
    }
    let api = doc.endpoints.sign_in_email.as_deref()?.strip_suffix("/sign-in/email")?;
    Some(format!("{api}/sign-up/email"))
}

fn waits_for_email(answer: &Answer) -> bool {
    (200..300).contains(&answer.status) && answer.token.is_none() && answer.body.get("twoFactorRedirect").is_none()
}

fn code_route(doc: &Discovery, code: &str) -> Option<String> {
    let totp = code.len() == TOTP_DIGITS && code.chars().all(|c| c.is_ascii_digit());
    if totp {
        doc.endpoints.two_factor_verify.clone()
    } else {
        doc.endpoints.two_factor_backup_code.clone()
    }
}

fn tidy_code(code: &str) -> String {
    let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.chars().filter(|c| *c != '-').all(|c| c.is_ascii_digit()) {
        compact.replace('-', "")
    } else {
        compact
    }
}

#[tauri::command]
pub async fn account_sign_in_code(app: tauri::State<'_, App>, handle: AppHandle, code: String) -> Result<SignInStep, String> {
    let code = tidy_code(&code);
    if code.is_empty() || code.len() > MAX_CODE {
        return Err("Enter the code from your authenticator app.".into());
    }
    let (server, doc) = current_server(&app).await?;
    let pending = pending_for(&server);
    let Some(challenge) = pending.challenge.clone() else {
        return Err("That sign-in timed out. Start again.".into());
    };
    let url = code_route(&doc, &code).ok_or("This account server needs your browser to sign in.")?;
    let answer = post(&app, &server, &url, json!({ "code": code }), Some(&challenge), None).await?;
    if !(200..300).contains(&answer.status) {
        let problem = read_reply(&answer).err().unwrap_or_else(|| "That code isn't right.".into());
        if problem.contains("timed out") {
            forget();
        }
        return Err(problem);
    }
    settle(&handle, &server, &doc, pending, answer).await
}

fn new_nonce() -> (String, String) {
    let raw = data_encoding::HEXLOWER.encode(&rand::random::<[u8; 32]>());
    let hashed = data_encoding::HEXLOWER.encode(&Sha256::digest(raw.as_bytes()));
    (raw, hashed)
}

#[cfg(target_os = "ios")]
async fn ask_apple(handle: &AppHandle) -> Result<Option<AppleAttempt>, String> {
    use commy_apple_sign_in::AppleSignInExt;
    let (raw, hashed) = new_nonce();
    let handle = handle.clone();
    let answer = tauri::async_runtime::spawn_blocking(move || handle.apple_sign_in().sign_in(&hashed))
        .await
        .map_err(|_| "Sign in with Apple stopped unexpectedly".to_string())?;
    match answer {
        Ok(credential) => Ok(Some(AppleAttempt {
            token: credential.identity_token,
            nonce: raw,
            given_name: crate::safety::truncate(&credential.given_name, 64),
            family_name: crate::safety::truncate(&credential.family_name, 64),
        })),
        Err(e) if e.contains("canceled") => Ok(None),
        Err(e) => Err(format!("Sign in with Apple didn't finish: {e}")),
    }
}

#[cfg(not(target_os = "ios"))]
async fn ask_apple(_handle: &AppHandle) -> Result<Option<AppleAttempt>, String> {
    let _ = new_nonce();
    Err("Sign in with Apple is only in the iPhone app".into())
}

fn apple_body(attempt: &AppleAttempt, create: bool, adult_confirmed: bool) -> Value {
    let mut id_token = json!({ "token": attempt.token, "nonce": attempt.nonce });
    if !attempt.given_name.is_empty() || !attempt.family_name.is_empty() {
        id_token["user"] = json!({ "name": { "firstName": attempt.given_name, "lastName": attempt.family_name } });
    }
    let mut body = json!({ "provider": "apple", "idToken": id_token });
    if create {
        body["requestSignUp"] = json!(true);
    }
    if create && adult_confirmed {
        body["additionalData"] = json!({ "adultConfirmed": true });
    }
    body
}

async fn check_access_code(app: &App, server: &str, doc: &Discovery, code: &str) -> Result<String, String> {
    let code = code.trim();
    if code.is_empty() || code.len() > 200 {
        return Err("Enter the access code.".into());
    }
    let url = doc.endpoints.access_code.clone().ok_or("This account server does not use an access code.")?;
    let answer = post(app, server, &url, json!({ "code": code }), None, None).await?;
    if !(200..300).contains(&answer.status) {
        return Err(read_reply(&answer).err().unwrap_or_else(|| "That access code isn't right.".into()));
    }
    answer.body.get("token").and_then(Value::as_str).map(str::to_string).ok_or_else(|| "That access code isn't right.".into())
}

#[tauri::command]
pub async fn account_sign_in_apple(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    create: bool,
    adult_confirmed: bool,
    access_code: Option<String>,
) -> Result<SignInStep, String> {
    let (server, doc) = current_server(&app).await?;
    let url = doc.endpoints.sign_in_social.clone().filter(|_| offers_apple(&doc)).ok_or("This account server does not offer Sign in with Apple.")?;
    let mut pending = pending_for(&server);
    let reuse = create.then(|| pending.apple.clone()).flatten();
    let attempt = match reuse {
        Some(attempt) => attempt,
        None => match ask_apple(&handle).await? {
            Some(attempt) => attempt,
            None => return Ok(SignInStep::Canceled),
        },
    };
    pending.apple = Some(attempt.clone());
    if let Some(code) = access_code {
        pending.access = Some(check_access_code(&app, &server, &doc, &code).await?);
    }
    let cookie = pending.access.as_ref().map(|t| format!("{ACCESS_COOKIE}={t}"));
    let answer = post(&app, &server, &url, apple_body(&attempt, create, adult_confirmed), cookie.as_deref(), None).await?;
    let step = settle(&handle, &server, &doc, pending, answer).await?;
    if create && step == SignInStep::NewAccount {
        forget();
        return Err("This account server isn't taking new accounts right now.".into());
    }
    Ok(step)
}

fn new_verifier() -> (String, String) {
    let verifier = data_encoding::BASE64URL_NOPAD.encode(&rand::random::<[u8; 32]>());
    let challenge = data_encoding::BASE64URL_NOPAD.encode(&Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

fn query_value(url: &url::Url, name: &str) -> Option<String> {
    url.query_pairs().find(|(k, _)| k == name).map(|(_, v)| v.into_owned()).filter(|v| !v.is_empty() && v.len() <= MAX_HANDOFF)
}

fn sheet_page(server: &str, answer: &Answer) -> Result<(String, String), String> {
    let link = answer.body.get("url").and_then(Value::as_str).ok_or("The account server did not start the sign-in")?;
    let page = url::Url::parse(link).map_err(|_| "The account server sent a broken sign-in link")?;
    let home = url::Url::parse(server).map_err(|_| "The account server address is not valid")?;
    if page.origin() != home.origin() {
        return Err("The account server sent a sign-in link for another site".into());
    }
    let handoff = query_value(&page, "handoff").ok_or("The account server sent a broken sign-in link")?;
    Ok((page.to_string(), handoff))
}

fn sheet_return_scheme(started: &Value, own: &'static str) -> &'static str {
    let scheme = started.get("returnUrl").and_then(Value::as_str).and_then(|link| url::Url::parse(link).ok()).map(|url| url.scheme().to_string());
    match scheme.as_deref() {
        Some(scheme) if scheme == own => own,
        _ => SHEET_RETURN_SCHEME,
    }
}

fn sheet_code(back: &str, handoff: &str, scheme: &str) -> Result<String, String> {
    let url = url::Url::parse(back).map_err(|_| "The sign-in came back broken. Try again.")?;
    let ours = url.scheme() == scheme && url.host_str() == Some(SHEET_RETURN_HOST);
    if !ours || query_value(&url, "handoff").as_deref() != Some(handoff) {
        return Err("The sign-in came back for another attempt. Try again.".into());
    }
    query_value(&url, "code").ok_or_else(|| "The sign-in came back broken. Try again.".into())
}

fn sheet_body(provider: &str, challenge: &str, adult_confirmed: bool, access: Option<&str>, link_scheme: &str) -> Value {
    let mut body = json!({ "provider": provider, "challenge": challenge });
    if link_scheme != SHEET_RETURN_SCHEME {
        body["returnScheme"] = json!(link_scheme);
    }
    if adult_confirmed {
        body["adultConfirmed"] = json!(true);
    }
    if let Some(token) = access {
        body["accessToken"] = json!(token);
    }
    body
}

fn native_answer(answer: Result<Value, String>, what: &str) -> Result<Option<Value>, String> {
    match answer {
        Ok(value) => Ok(Some(value)),
        Err(e) if e.contains("canceled") => Ok(None),
        Err(e) => Err(format!("{what} didn't finish: {e}")),
    }
}

#[cfg(target_os = "android")]
static NATIVE: std::sync::OnceLock<tauri::plugin::PluginHandle<tauri::Wry>> = std::sync::OnceLock::new();

#[cfg(target_os = "android")]
pub fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("commy-sign-in")
        .setup(|_app, api| {
            let _ = NATIVE.set(api.register_android_plugin("chat.commy.mobile", "SignInPlugin")?);
            Ok(())
        })
        .build()
}

#[cfg(target_os = "android")]
fn run_native(_handle: &AppHandle, command: &'static str, args: Value) -> Result<Value, String> {
    let plugin = NATIVE.get().ok_or("The sign-in helper isn't ready yet")?;
    plugin.run_mobile_plugin::<Value>(command, args).map_err(|e| e.to_string())
}

#[cfg(target_os = "ios")]
fn run_native(handle: &AppHandle, command: &'static str, args: Value) -> Result<Value, String> {
    use commy_apple_sign_in::AppleSignInExt;
    handle.apple_sign_in().run(command, args)
}

#[cfg(not(mobile))]
fn run_native(_handle: &AppHandle, _command: &'static str, _args: Value) -> Result<Value, String> {
    Err("This sign-in is only in the phone apps".into())
}

async fn ask_native(handle: &AppHandle, command: &'static str, args: Value, what: &str) -> Result<Option<Value>, String> {
    let handle = handle.clone();
    let answer = tauri::async_runtime::spawn_blocking(move || run_native(&handle, command, args))
        .await
        .map_err(|_| format!("{what} stopped unexpectedly"))?;
    native_answer(answer, what)
}

#[tauri::command]
pub async fn account_sign_in_sheet(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    provider: String,
    adult_confirmed: bool,
    access_code: Option<String>,
) -> Result<SignInStep, String> {
    let (server, doc) = current_server(&app).await?;
    if !SHEET_PROVIDERS.contains(&provider.as_str()) || !offers_sheet(&doc, &provider) {
        return Err("This account server needs your browser for that sign-in.".into());
    }
    let start = doc.endpoints.app_sign_in_start.clone().ok_or("This account server needs your browser for that sign-in.")?;
    let finish = doc.endpoints.app_sign_in_finish.clone().ok_or("This account server needs your browser for that sign-in.")?;
    let mut pending = pending_for(&server);
    if let Some(code) = access_code {
        pending.access = Some(check_access_code(&app, &server, &doc, &code).await?);
    }
    let (verifier, challenge) = new_verifier();
    let body = sheet_body(&provider, &challenge, adult_confirmed, pending.access.as_deref(), APP_LINK_SCHEME);
    let started = post(&app, &server, &start, body, None, None).await?;
    if !(200..300).contains(&started.status) {
        return Err(read_reply(&started).err().unwrap_or_else(|| "The account server did not start the sign-in".into()));
    }
    let (page, handoff) = sheet_page(&server, &started)?;
    let scheme = sheet_return_scheme(&started.body, APP_LINK_SCHEME);
    let Some(back) = open_sheet(&handle, page, scheme).await? else {
        keep(pending);
        return Ok(SignInStep::Canceled);
    };
    let code = sheet_code(&back, &handoff, scheme)?;
    let answer = post(&app, &server, &finish, json!({ "handoff": handoff, "verifier": verifier, "code": code }), None, None).await?;
    if answer.body.get("code").and_then(Value::as_str) == Some("ACCESS_DENIED") {
        keep(pending);
        return Ok(SignInStep::Canceled);
    }
    settle(&handle, &server, &doc, pending, answer).await
}

#[cfg(mobile)]
async fn open_sheet(handle: &AppHandle, page: String, scheme: &str) -> Result<Option<String>, String> {
    let back = ask_native(handle, "openSheet", json!({ "url": page, "scheme": scheme }), "The sign-in page").await?;
    Ok(back.map(|back| back.get("url").and_then(Value::as_str).unwrap_or_default().to_string()))
}

#[cfg(desktop)]
async fn open_sheet(handle: &AppHandle, page: String, _scheme: &str) -> Result<Option<String>, String> {
    let back = wait_for_browser();
    if let Err(e) = crate::browser::open(handle, &page) {
        account_cancel_browser_sign_in();
        return Err(format!("Could not open your browser: {e}"));
    }
    match tokio::time::timeout(BROWSER_WAIT, back).await {
        Ok(link) => Ok(link.ok().flatten()),
        Err(_) => {
            account_cancel_browser_sign_in();
            Err("The sign-in took too long. Try again.".into())
        }
    }
}

#[cfg(desktop)]
fn wait_for_browser() -> tokio::sync::oneshot::Receiver<Option<String>> {
    let (waiter, back) = tokio::sync::oneshot::channel();
    if let Some(earlier) = BROWSER_RETURN.lock_or_recover().replace(waiter) {
        let _ = earlier.send(None);
    }
    back
}

pub fn is_browser_return(link: &str) -> bool {
    url::Url::parse(link).is_ok_and(|url| url.scheme() == SHEET_RETURN_SCHEME && url.host_str() == Some(SHEET_RETURN_HOST))
}

pub fn browser_came_back(link: &str) {
    if let Some(waiter) = BROWSER_RETURN.lock_or_recover().take() {
        let _ = waiter.send(Some(link.to_string()));
    }
}

#[tauri::command]
pub fn account_cancel_browser_sign_in() {
    if let Some(waiter) = BROWSER_RETURN.lock_or_recover().take() {
        let _ = waiter.send(None);
    }
}

#[tauri::command]
pub async fn account_sign_in_passkey(app: tauri::State<'_, App>, handle: AppHandle, quiet: Option<bool>) -> Result<SignInStep, String> {
    let quiet = quiet.unwrap_or(false);
    let (server, doc) = current_server(&app).await?;
    let (Some(options_url), Some(verify_url)) = (doc.endpoints.passkey_options.clone(), doc.endpoints.passkey_verify.clone()) else {
        return Err("This account server needs your browser for passkeys.".into());
    };
    if !offers_passkey(&doc, &server) {
        return Err("This account server needs your browser for passkeys.".into());
    }
    let res = app
        .http
        .get(&options_url)
        .header(reqwest::header::ORIGIN, &server)
        .send()
        .await
        .map_err(|_| format!("Could not reach {}. Check your connection.", host_of(&server)))?;
    if !res.status().is_success() {
        return Err("The account server refused the passkey sign-in. Try again, or use your browser instead.".into());
    }
    let cookies = cookie_pairs(res.headers().get_all(reqwest::header::SET_COOKIE).iter().filter_map(|v| v.to_str().ok())).join("; ");
    let options: Value = res.json().await.map_err(|_| "The account server sent broken passkey options")?;
    if options.get("challenge").and_then(Value::as_str).unwrap_or_default().is_empty() {
        return Err("The account server sent broken passkey options".into());
    }
    let prompt = ask_native(&handle, "passkey", passkey_prompt(&options, quiet), "The passkey").await;
    let Some(credential) = quiet_passkey_answer(prompt, quiet)? else {
        return Ok(SignInStep::Canceled);
    };
    let response = passkey_response(credential)?;
    let answer = post(&app, &server, &verify_url, json!({ "response": response }), Some(&cookies), None).await?;
    settle(&handle, &server, &doc, Pending::new(&server), answer).await
}

fn passkey_prompt(options: &Value, quiet: bool) -> Value {
    json!({ "request": options.to_string(), "quiet": quiet })
}

fn quiet_passkey_answer(prompt: Result<Option<Value>, String>, quiet: bool) -> Result<Option<Value>, String> {
    match prompt {
        Err(_) if quiet => Ok(None),
        other => other,
    }
}

fn passkey_response(credential: Value) -> Result<Value, String> {
    let parsed = match credential.get("json").and_then(Value::as_str) {
        Some(text) => serde_json::from_str::<Value>(text).map_err(|_| "The passkey answer was broken")?,
        None => credential,
    };
    let fine = parsed.get("id").and_then(Value::as_str).is_some_and(|id| !id.is_empty()) && parsed.get("response").is_some_and(Value::is_object);
    if !fine {
        return Err("The passkey answer was broken".into());
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(status: u16, body: Value) -> Answer {
        Answer { status, body, token: None, cookies: vec![] }
    }

    #[test]
    fn a_good_answer_hands_back_the_token() {
        let mut ok = answer(200, json!({ "redirect": false }));
        ok.token = Some("tok".into());
        assert_eq!(read_reply(&ok), Ok(Reply::Token("tok".into())));
        assert!(read_reply(&answer(200, json!({}))).is_err());
    }

    #[test]
    fn the_server_steps_are_recognised() {
        assert_eq!(read_reply(&answer(200, json!({ "twoFactorRedirect": true }))), Ok(Reply::TwoFactor));
        assert_eq!(read_reply(&answer(401, json!({ "code": "OAUTH_LINK_ERROR", "message": "signup disabled" }))), Ok(Reply::NoAccount));
        assert_eq!(read_reply(&answer(403, json!({ "code": "ACCESS_CODE_REQUIRED" }))), Ok(Reply::NeedsAccessCode));
        assert_eq!(read_reply(&answer(403, json!({ "code": "EMAIL_NOT_VERIFIED" }))), Ok(Reply::CheckEmail));
        assert_eq!(read_reply(&answer(403, json!({ "code": "ADULT_CONFIRMATION_REQUIRED" }))), Ok(Reply::NeedsAdultConfirmation));
    }

    #[test]
    fn refusals_read_like_sentences() {
        let wrong = read_reply(&answer(401, json!({ "code": "INVALID_EMAIL_OR_PASSWORD" }))).unwrap_err();
        assert_eq!(wrong, "That email and password don't match.");
        let linked = read_reply(&answer(401, json!({ "code": "OAUTH_LINK_ERROR", "message": "account not linked" }))).unwrap_err();
        assert!(linked.contains("already uses this email"));
        assert!(read_reply(&answer(429, json!({}))).unwrap_err().contains("Too many"));
        assert!(read_reply(&answer(400, json!({ "code": "MISSING_RESPONSE" }))).unwrap_err().contains("safety check"));
        assert!(read_reply(&answer(500, Value::Null)).unwrap_err().contains("browser"));
    }

    #[test]
    fn a_suspended_account_hears_why_and_whom_to_write_to() {
        let told = read_reply(&answer(403, json!({ "code": "ACCOUNT_SUSPENDED", "message": "This account is suspended. Write to safety@example.org." }))).unwrap_err();
        assert_eq!(told, "This account is suspended. Write to safety@example.org.");
        assert_eq!(read_reply(&answer(403, json!({ "code": "ACCOUNT_SUSPENDED" }))).unwrap_err(), SUSPENDED_FALLBACK);
        let long = read_reply(&answer(403, json!({ "code": "ACCOUNT_SUSPENDED", "message": format!("a\u{7}{}", "b".repeat(500)) }))).unwrap_err();
        assert_eq!(long.chars().count(), MAX_SUSPENDED_NOTE);
        assert!(!long.contains('\u{7}'));
    }

    #[test]
    fn only_live_cookies_are_sent_back() {
        let pairs = cookie_pairs(
            [
                "__Secure-better-auth.two_factor=abc.sig; Max-Age=600; Path=/; HttpOnly; Secure",
                "__Secure-better-auth.session_token=; Max-Age=0; Path=/",
                "broken",
            ]
            .into_iter(),
        );
        assert_eq!(pairs, vec!["__Secure-better-auth.two_factor=abc.sig".to_string()]);
    }

    #[test]
    fn authenticator_and_backup_codes_take_their_own_route() {
        let api = "https://commy.chat/api/auth";
        let doc: Discovery = serde_json::from_value(json!({
            "issuer": "https://commy.chat",
            "deviceAuthorization": { "clientId": "c", "deviceCodeEndpoint": format!("{api}/device/code"), "tokenEndpoint": format!("{api}/device/token") },
            "endpoints": {
                "session": format!("{api}/get-session"), "signOut": format!("{api}/sign-out"),
                "linkChallenge": format!("{api}/l"), "registerDevice": format!("{api}/r"),
                "twoFactorVerify": format!("{api}/two-factor/verify-totp"),
                "twoFactorBackupCode": format!("{api}/two-factor/verify-backup-code"),
            },
        }))
        .unwrap();
        assert_eq!(code_route(&doc, &tidy_code(" 123 456 ")).unwrap(), format!("{api}/two-factor/verify-totp"));
        assert_eq!(code_route(&doc, &tidy_code("AbCd-EfGh")).unwrap(), format!("{api}/two-factor/verify-backup-code"));
        assert!(!offers_password(&doc), "an older server without the email route keeps the browser sign-in");
        assert!(!offers_apple(&doc));
        assert_eq!(captcha_url(&doc), None);
    }

    #[test]
    fn apple_names_go_along_only_when_apple_shares_them() {
        let attempt = AppleAttempt { token: "t".into(), nonce: "n".into(), given_name: String::new(), family_name: String::new() };
        assert_eq!(apple_body(&attempt, false, true), json!({ "provider": "apple", "idToken": { "token": "t", "nonce": "n" } }));
        let named = AppleAttempt { given_name: "Ada".into(), ..attempt };
        let body = apple_body(&named, true, true);
        assert_eq!(body["requestSignUp"], json!(true));
        assert_eq!(body["additionalData"], json!({ "adultConfirmed": true }));
        assert_eq!(apple_body(&named, true, false).get("additionalData"), None);
        assert_eq!(body["idToken"]["user"]["name"]["firstName"], json!("Ada"));
    }

    #[test]
    fn the_sheet_link_must_be_the_account_server() {
        let mut ok = answer(200, json!({ "url": "https://commy.chat/api/auth/commy/app-sign-in/go?handoff=abc" }));
        assert_eq!(sheet_page("https://commy.chat", &ok).unwrap().1, "abc");
        ok.body = json!({ "url": "https://evil.example/go?handoff=abc" });
        assert!(sheet_page("https://commy.chat", &ok).unwrap_err().contains("another site"));
        ok.body = json!({ "url": "https://commy.chat/go" });
        assert!(sheet_page("https://commy.chat", &ok).is_err());
    }

    #[test]
    fn only_our_own_attempt_comes_back_from_the_sheet() {
        assert_eq!(sheet_code("commy://signed-in?handoff=abc&code=xyz", "abc", "commy").unwrap(), "xyz");
        assert!(sheet_code("commy://signed-in?handoff=other&code=xyz", "abc", "commy").is_err());
        assert!(sheet_code("commy://i/commy.chat/AB?handoff=abc&code=xyz", "abc", "commy").is_err());
        assert!(sheet_code("https://signed-in/?handoff=abc&code=xyz", "abc", "commy").is_err());
        assert!(sheet_code("commy://signed-in?handoff=abc", "abc", "commy").is_err());
        assert_eq!(sheet_code("commy-beta://signed-in?handoff=abc&code=xyz", "abc", "commy-beta").unwrap(), "xyz");
        assert!(sheet_code("commy://signed-in?handoff=abc&code=xyz", "abc", "commy-beta").is_err());
    }

    #[test]
    fn the_sheet_waits_on_the_link_the_account_server_will_send() {
        let new_server = json!({ "url": "https://staging.commy.chat/go", "returnUrl": "commy-beta://signed-in" });
        assert_eq!(sheet_return_scheme(&new_server, "commy-beta"), "commy-beta");
        let old_server = json!({ "url": "https://staging.commy.chat/go", "returnUrl": "commy://signed-in" });
        assert_eq!(sheet_return_scheme(&old_server, "commy-beta"), "commy");
        assert_eq!(sheet_return_scheme(&json!({ "url": "https://commy.chat/go" }), "commy-beta"), "commy");
        let stranger = json!({ "returnUrl": "elsewhere://signed-in" });
        assert_eq!(sheet_return_scheme(&stranger, "commy-beta"), "commy");
        assert_eq!(sheet_return_scheme(&new_server, "commy"), "commy");
    }

    #[test]
    fn the_sheet_asks_for_what_the_person_already_confirmed() {
        let (verifier, challenge) = new_verifier();
        assert_eq!(challenge.len(), 43);
        assert_eq!(challenge, data_encoding::BASE64URL_NOPAD.encode(&Sha256::digest(verifier.as_bytes())));
        assert_eq!(sheet_body("google", "c", false, None, "commy"), json!({ "provider": "google", "challenge": "c" }));
        assert_eq!(sheet_body("google", "c", false, None, "commy-beta")["returnScheme"], json!("commy-beta"));
        let body = sheet_body("twitch", "c", true, Some("tok"), "commy");
        assert_eq!(body["adultConfirmed"], json!(true));
        assert_eq!(body["accessToken"], json!("tok"));
    }

    #[test]
    fn a_closed_sheet_or_passkey_prompt_is_not_an_error() {
        assert_eq!(native_answer(Err("canceled".into()), "x"), Ok(None));
        assert!(native_answer(Err("boom".into()), "The passkey").unwrap_err().starts_with("The passkey"));
        let android = json!({ "json": "{\"id\":\"cred\",\"response\":{\"signature\":\"s\"}}" });
        assert_eq!(passkey_response(android).unwrap()["id"], json!("cred"));
        assert_eq!(passkey_response(json!({ "id": "cred", "response": {} })).unwrap()["id"], json!("cred"));
        assert!(passkey_response(json!({ "id": "" , "response": {} })).is_err());
        assert!(passkey_response(json!({ "json": "nope" })).is_err());
    }

    #[test]
    fn sign_up_checks_each_field_before_asking_the_server() {
        let good = SignUpFields::checked(" me@example.com ", "long enough", " Nova ", true).unwrap();
        assert_eq!(good, SignUpFields { email: "me@example.com".into(), password: "long enough".into(), name: "Nova".into() });
        let body = good.body();
        assert_eq!(body["adultConfirmed"], json!(true));
        assert_eq!(body["callbackURL"], json!(SIGN_UP_RETURN));
        assert_eq!(body["name"], json!("Nova"));
        assert!(SignUpFields::checked("nope", "long enough", "Nova", true).unwrap_err().contains("email"));
        assert!(SignUpFields::checked("me@example.com", "short", "Nova", true).unwrap_err().contains("8"));
        assert!(SignUpFields::checked("me@example.com", &"x".repeat(129), "Nova", true).unwrap_err().contains("128"));
        assert!(SignUpFields::checked("me@example.com", "long enough", "  ", true).unwrap_err().contains("display name"));
        assert!(SignUpFields::checked("me@example.com", "long enough", &"n".repeat(65), true).unwrap_err().contains("64"));
        assert!(SignUpFields::checked("me@example.com", "long enough", "Nova", false).unwrap_err().contains("18"));
    }

    #[test]
    fn sign_up_waits_for_the_email_only_when_no_session_came_back() {
        assert!(waits_for_email(&answer(200, json!({ "token": null, "user": {} }))));
        let mut signed_in = answer(200, json!({}));
        signed_in.token = Some("t".into());
        assert!(!waits_for_email(&signed_in));
        assert!(!waits_for_email(&answer(422, json!({ "code": "USER_ALREADY_EXISTS" }))));
        assert!(read_reply(&answer(422, json!({ "code": "USER_ALREADY_EXISTS" }))).unwrap_err().contains("Sign in instead"));
        assert!(read_reply(&answer(400, json!({ "code": "THROWAWAY_EMAIL" }))).unwrap_err().contains("Throwaway"));
    }

    #[test]
    fn only_the_sign_in_return_link_counts_as_the_browser_coming_back() {
        assert!(is_browser_return("commy://signed-in?handoff=h1&code=c1"));
        assert!(!is_browser_return("commy://email-confirmed"));
        assert!(!is_browser_return("commy://i/commy.chat/AB23CD45"));
        assert!(!is_browser_return("https://signed-in/?handoff=h1"));
    }

    #[cfg(desktop)]
    #[test]
    fn the_browser_return_reaches_only_the_newest_waiting_sign_in() {
        let mut earlier = wait_for_browser();
        let mut newest = wait_for_browser();
        assert_eq!(earlier.try_recv().unwrap(), None, "a second sign-in sets the first aside");
        browser_came_back("commy://signed-in?handoff=h2&code=c2");
        assert_eq!(newest.try_recv().unwrap().as_deref(), Some("commy://signed-in?handoff=h2&code=c2"));
        let mut canceled = wait_for_browser();
        account_cancel_browser_sign_in();
        assert_eq!(canceled.try_recv().unwrap(), None);
        browser_came_back("commy://signed-in?handoff=late&code=late");
    }

    #[test]
    fn a_quiet_passkey_prompt_steps_aside_for_the_password() {
        let options = json!({ "challenge": "abc", "rpId": "commy.chat" });
        let prompt = passkey_prompt(&options, true);
        assert_eq!(prompt["quiet"], json!(true));
        assert_eq!(serde_json::from_str::<Value>(prompt["request"].as_str().unwrap()).unwrap(), options);
        assert_eq!(passkey_prompt(&options, false)["quiet"], json!(false));
        assert_eq!(quiet_passkey_answer(Err("No passkey for this account is saved on this phone".into()), true), Ok(None));
        assert!(quiet_passkey_answer(Err("The passkey didn't finish".into()), false).is_err());
        assert_eq!(quiet_passkey_answer(Ok(Some(json!({ "id": "cred" }))), true), Ok(Some(json!({ "id": "cred" }))));
        assert_eq!(quiet_passkey_answer(Ok(None), false), Ok(None));
    }

    #[test]
    fn sheet_and_passkey_refusals_read_like_sentences() {
        assert!(read_reply(&answer(403, json!({ "code": "SIGNUP_DISABLED" }))).unwrap_err().contains("new accounts"));
        assert!(read_reply(&answer(403, json!({ "code": "ACCOUNT_NOT_LINKED" }))).unwrap_err().contains("already uses this email"));
        assert!(read_reply(&answer(401, json!({ "code": "PASSKEY_NOT_FOUND" }))).unwrap_err().contains("passkey"));
        for lost in ["STATE_MISMATCH", "NO_SESSION", "HANDOFF_NOT_FOUND", "PLEASE_RESTART_THE_PROCESS"] {
            assert_eq!(read_reply(&answer(403, json!({ "code": lost }))).unwrap_err(), "That sign-in timed out. Start again.");
        }
        let unknown = read_reply(&answer(403, json!({ "code": "UNABLE_TO_GET_USER_INFO<script>" }))).unwrap_err();
        assert!(unknown.contains("(UNABLE_TO_GET_USER_INFOscript)"), "{unknown}");
    }

    #[test]
    fn the_nonce_apple_sees_is_the_hash_of_the_one_sent_to_the_server() {
        let (raw, hashed) = new_nonce();
        assert_eq!(raw.len(), 64);
        assert_eq!(hashed, data_encoding::HEXLOWER.encode(&Sha256::digest(raw.as_bytes())));
    }
}
