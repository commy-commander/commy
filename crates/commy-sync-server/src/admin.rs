use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{DefaultBodyLimit, Path as Segment, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode, Uri};
use axum::middleware::{from_fn, Next};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Router};
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use sha2::Sha256;

use crate::certificate::{self, Facts};
use crate::config::AdminSettings;
use crate::gateway::{self, Overview};
use crate::ramp::Tier;
use crate::operator::{self, Command};
use crate::reports::describe;
use crate::store::Totals;
use crate::Service;

const COOKIE: &str = "commy_admin";
const SESSION_SECONDS: i64 = 12 * 60 * 60;
const MAX_FAILED_SIGN_INS: u32 = 10;
const SIGN_IN_WINDOW: Duration = Duration::from_secs(15 * 60);
const FAILED_SIGN_IN_PAUSE: Duration = Duration::from_millis(400);
const RELEASE_FRESH: Duration = Duration::from_secs(60 * 60);
const RELEASE_RETRY: Duration = Duration::from_secs(5 * 60);
const FORM_LIMIT: usize = 128 * 1024;
const CERT_FILE: &str = "custom.crt";
const KEY_FILE: &str = "custom.key";
const CADDY_SNIPPET: &str = "tls.caddy";

pub struct Admin {
    service: Arc<Service>,
    settings: AdminSettings,
    secret: [u8; 32],
    tries: Mutex<Tries>,
    newest: tokio::sync::Mutex<Option<(Instant, Option<String>)>>,
    http: reqwest::Client,
}

struct Tries {
    failed: u32,
    since: Instant,
}

impl Admin {
    pub fn new(service: Arc<Service>, settings: AdminSettings) -> Arc<Self> {
        let http = reqwest::Client::builder().timeout(Duration::from_secs(10)).build().unwrap_or_default();
        Arc::new(Self {
            service,
            settings,
            secret: rand::random(),
            tries: Mutex::new(Tries { failed: 0, since: Instant::now() }),
            newest: tokio::sync::Mutex::new(None),
            http,
        })
    }

    fn mac(&self, words: &[&[u8]]) -> Hmac<Sha256> {
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&self.secret).expect("HMAC takes any key length");
        for word in words {
            mac.update(&(word.len() as u64).to_le_bytes());
            mac.update(word);
        }
        mac
    }

    fn password_matches(&self, typed: &str) -> bool {
        let expected = self.mac(&[b"password", self.settings.password.as_bytes()]).finalize().into_bytes();
        self.mac(&[b"password", typed.as_bytes()]).verify_slice(&expected).is_ok()
    }

    fn session_token(&self, expires: i64) -> String {
        let tag = self.mac(&[b"session", expires.to_string().as_bytes()]).finalize().into_bytes();
        format!("{expires}.{}", data_encoding::HEXLOWER.encode(&tag))
    }

    fn signed_in(&self, headers: &HeaderMap) -> bool {
        let Some(token) = cookie(headers, COOKIE) else { return false };
        let Some((expires, tag)) = token.split_once('.') else { return false };
        let (Ok(expires), Ok(tag)) = (expires.parse::<i64>(), data_encoding::HEXLOWER.decode(tag.as_bytes())) else { return false };
        expires > now_s() && self.mac(&[b"session", expires.to_string().as_bytes()]).verify_slice(&tag).is_ok()
    }

    fn locked_out(&self) -> bool {
        let mut tries = self.tries.lock().unwrap_or_else(|e| e.into_inner());
        if tries.since.elapsed() >= SIGN_IN_WINDOW {
            *tries = Tries { failed: 0, since: Instant::now() };
        }
        tries.failed >= MAX_FAILED_SIGN_INS
    }

    fn failed_sign_in(&self) {
        let mut tries = self.tries.lock().unwrap_or_else(|e| e.into_inner());
        tries.failed += 1;
    }

    fn allowed(&self, headers: &HeaderMap) -> Result<(), Response> {
        if !same_origin(headers) {
            return Err((StatusCode::FORBIDDEN, "This form must be sent from the admin page itself.").into_response());
        }
        if !self.signed_in(headers) {
            return Err(Redirect::to("/").into_response());
        }
        Ok(())
    }

    async fn newest_release(&self) -> Option<String> {
        let mut cached = self.newest.lock().await;
        if let Some((at, newest)) = cached.as_ref() {
            let fresh = if newest.is_some() { RELEASE_FRESH } else { RELEASE_RETRY };
            if at.elapsed() < fresh {
                return newest.clone();
            }
        }
        let newest = async {
            let body: serde_json::Value = self.http.get(&self.settings.releases).send().await.ok()?.error_for_status().ok()?.json().await.ok()?;
            let version = body.get("version")?.as_str()?.trim().trim_start_matches('v');
            (!version.is_empty() && version.len() <= 40).then(|| version.to_string())
        }
        .await;
        *cached = Some((Instant::now(), newest.clone()));
        newest
    }

    async fn relay_answers(&self) -> bool {
        match self.http.get(&self.settings.relay_probe).timeout(Duration::from_secs(4)).send().await {
            Ok(response) => response.status().is_success(),
            Err(_) => false,
        }
    }

    async fn served_certificate(&self) -> Result<Facts, String> {
        let domain = self.settings.domain.as_deref().ok_or("SYNC_DOMAIN is not set")?;
        certificate::served(&self.settings.caddy_tls, domain).await
    }

    fn tls_file(&self, name: &str) -> PathBuf {
        self.settings.custom_tls.join(name)
    }

    fn uses_own_certificate(&self) -> bool {
        self.tls_file(CADDY_SNIPPET).is_file() && self.tls_file(CERT_FILE).is_file()
    }

    async fn reload_caddy(&self) -> Result<(), String> {
        let caddyfile = tokio::fs::read(&self.settings.caddyfile).await.map_err(|e| format!("could not read the Caddyfile: {e}"))?;
        let url = format!("{}/load", self.settings.caddy_admin.trim_end_matches('/'));
        let response = self
            .http
            .post(url)
            .header(header::CONTENT_TYPE, "text/caddyfile")
            .body(caddyfile)
            .send()
            .await
            .map_err(|e| format!("could not reach Caddy: {e}"))?;
        if response.status().is_success() {
            return Ok(());
        }
        let said: String = response.text().await.unwrap_or_default().chars().take(300).collect();
        Err(format!("Caddy refused it: {}", said.trim()))
    }

    async fn use_own_certificate(&self, certificate: &str, key: &str) -> Result<(), String> {
        let dir = &self.settings.custom_tls;
        let snippet = format!("tls {} {}\n", self.tls_file(CERT_FILE).display(), self.tls_file(KEY_FILE).display());
        let written = write_file(dir, CERT_FILE, certificate.trim().as_bytes(), 0o644)
            .and_then(|_| write_file(dir, KEY_FILE, key.trim().as_bytes(), 0o640))
            .and_then(|_| write_file(dir, CADDY_SNIPPET, snippet.as_bytes(), 0o644));
        let result = match written {
            Ok(()) => self.reload_caddy().await,
            Err(e) => Err(format!("could not save the certificate in {}: {e}", dir.display())),
        };
        if result.is_err() {
            self.forget_own_certificate();
            let _ = self.reload_caddy().await;
        }
        result
    }

    fn forget_own_certificate(&self) {
        for name in [CADDY_SNIPPET, CERT_FILE, KEY_FILE] {
            let _ = std::fs::remove_file(self.tls_file(name));
        }
    }

    async fn page(&self, note: Option<&str>, problem: Option<&str>) -> String {
        let service = &self.service;
        let (database, totals, relay, served, newest, reports, overview) = tokio::join!(
            service.store.ping(),
            service.store.totals(),
            self.relay_answers(),
            self.served_certificate(),
            self.newest_release(),
            service.store.reports(true),
            gateway::overview(service),
        );
        let facts = Dashboard {
            domain: self.settings.domain.as_deref(),
            version: service.greeting.version.as_deref(),
            newest: newest.as_deref(),
            database,
            relay,
            relay_url: service.greeting.relay.as_deref(),
            public_relay_fallback: service.greeting.public_fallback,
            connections: service.stats.connections(),
            protocol: service.greeting.protocol,
            oldest_protocol: service.greeting.oldest_protocol,
            totals: totals.ok(),
            retention_days: service.retention_days,
            served,
            own_certificate: self.uses_own_certificate(),
            operator: &service.desk.operator,
            contact: service.desk.contact.as_ref().map(|c| c.shown()),
            emails_reports: service.desk.emails_reports(),
            reports: reports.map(|list| list.iter().map(|r| (r.id.clone(), r.report.community_name.clone(), r.report.reason.clone(), r.received, r.report.messages.len())).collect()),
            gateway: overview.ok().filter(|o| o.enabled),
            now: now_s(),
        };
        facts.render(note, problem)
    }
}

pub fn router(admin: Arc<Admin>) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/sign-in", post(sign_in))
        .route("/sign-out", post(sign_out))
        .route("/certificate", post(own_certificate))
        .route("/certificate/automatic", post(automatic_certificate))
        .route("/reports/{id}", get(report))
        .route("/reports/{id}/drop", post(drop_report))
        .route("/reports/{id}/dismiss", post(dismiss_report))
        .route("/gateway/cut", post(gateway_cut))
        .route("/gateway/restore", post(gateway_restore))
        .route("/gateway/tier", post(gateway_tier))
        .layer(DefaultBodyLimit::max(FORM_LIMIT))
        .layer(from_fn(guard))
        .with_state(admin)
}

pub async fn serve(listener: tokio::net::TcpListener, service: Arc<Service>, settings: AdminSettings) -> std::io::Result<()> {
    axum::serve(listener, router(Admin::new(service, settings))).await
}

async fn guard(request: Request, next: Next) -> Response {
    let local = request.headers().get(header::HOST).and_then(|v| v.to_str().ok()).is_some_and(loopback_host);
    let mut response = if local {
        next.run(request).await
    } else {
        (StatusCode::MISDIRECTED_REQUEST, "Open the admin page through an SSH tunnel, at http://localhost").into_response()
    };
    let headers = response.headers_mut();
    for (name, value) in [
        (header::CACHE_CONTROL, "no-store"),
        (header::CONTENT_SECURITY_POLICY, "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::REFERRER_POLICY, "same-origin"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}

#[derive(Deserialize)]
struct SignIn {
    password: String,
}

#[derive(Deserialize)]
struct Upload {
    certificate: String,
    key: String,
}

async fn home(State(admin): State<Arc<Admin>>, headers: HeaderMap, uri: Uri) -> Response {
    if !admin.signed_in(&headers) {
        return Html(sign_in_page(None)).into_response();
    }
    let note = uri.query().and_then(|q| q.strip_prefix("done=")).and_then(done_note);
    Html(admin.page(note, None).await).into_response()
}

async fn sign_in(State(admin): State<Arc<Admin>>, headers: HeaderMap, Form(form): Form<SignIn>) -> Response {
    if !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, "This form must be sent from the admin page itself.").into_response();
    }
    if admin.locked_out() {
        return (StatusCode::TOO_MANY_REQUESTS, Html(sign_in_page(Some("Too many wrong passwords. Wait 15 minutes and try again.")))).into_response();
    }
    if !admin.password_matches(&form.password) {
        admin.failed_sign_in();
        tokio::time::sleep(FAILED_SIGN_IN_PAUSE).await;
        return (StatusCode::UNAUTHORIZED, Html(sign_in_page(Some("That password is not right.")))).into_response();
    }
    let token = admin.session_token(now_s() + SESSION_SECONDS);
    let cookie = format!("{COOKIE}={token}; Path=/; Max-Age={SESSION_SECONDS}; HttpOnly; SameSite=Strict");
    ([(header::SET_COOKIE, cookie)], Redirect::to("/")).into_response()
}

async fn sign_out(headers: HeaderMap) -> Response {
    if !same_origin(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let cookie = format!("{COOKIE}=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict");
    ([(header::SET_COOKIE, cookie)], Redirect::to("/")).into_response()
}

async fn own_certificate(State(admin): State<Arc<Admin>>, headers: HeaderMap, Form(form): Form<Upload>) -> Response {
    if let Err(refused) = admin.allowed(&headers) {
        return refused;
    }
    let checked = certificate::check_pair(&form.certificate, &form.key, admin.settings.domain.as_deref(), now_s());
    let result = match checked {
        Ok(_) => admin.use_own_certificate(&form.certificate, &form.key).await,
        Err(problem) => Err(problem),
    };
    match result {
        Ok(()) => Redirect::to("/?done=certificate").into_response(),
        Err(problem) => (StatusCode::BAD_REQUEST, Html(admin.page(None, Some(&format!("Your certificate was not used: {problem}."))).await)).into_response(),
    }
}

async fn automatic_certificate(State(admin): State<Arc<Admin>>, headers: HeaderMap) -> Response {
    if let Err(refused) = admin.allowed(&headers) {
        return refused;
    }
    let _ = std::fs::remove_file(admin.tls_file(CADDY_SNIPPET));
    match admin.reload_caddy().await {
        Ok(()) => {
            admin.forget_own_certificate();
            Redirect::to("/?done=automatic").into_response()
        }
        Err(problem) => (StatusCode::BAD_GATEWAY, Html(admin.page(None, Some(&format!("Could not switch back: {problem}."))).await)).into_response(),
    }
}

async fn report(State(admin): State<Arc<Admin>>, headers: HeaderMap, Segment(id): Segment<String>) -> Response {
    if !admin.signed_in(&headers) {
        return Redirect::to("/").into_response();
    }
    if !report_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match admin.service.store.report(&id).await {
        Ok(Some(stored)) => Html(report_page(&id, &describe(&stored), stored.state.as_str())).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::SERVICE_UNAVAILABLE, format!("The database did not answer: {e}")).into_response(),
    }
}

async fn drop_report(State(admin): State<Arc<Admin>>, headers: HeaderMap, Segment(id): Segment<String>) -> Response {
    settle(&admin, &headers, id, Command::Drop, "dropped").await
}

async fn dismiss_report(State(admin): State<Arc<Admin>>, headers: HeaderMap, Segment(id): Segment<String>) -> Response {
    settle(&admin, &headers, id, Command::Dismiss, "dismissed").await
}

async fn settle(admin: &Admin, headers: &HeaderMap, id: String, command: fn(String) -> Command, done: &str) -> Response {
    if let Err(refused) = admin.allowed(headers) {
        return refused;
    }
    if !report_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match operator::run(&admin.service.store, command(id)).await {
        Ok(_) => {
            crate::learn_dropped(&admin.service).await;
            Redirect::to(&format!("/?done={done}")).into_response()
        }
        Err(e) => (StatusCode::SERVICE_UNAVAILABLE, format!("The database did not answer: {e}")).into_response(),
    }
}

#[derive(Deserialize)]
struct GatewayForm {
    origin: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    tier: String,
}

async fn gateway_cut(State(admin): State<Arc<Admin>>, headers: HeaderMap, Form(form): Form<GatewayForm>) -> Response {
    let reason = form.reason.trim();
    let reason = if reason.is_empty() { "cut off from the admin page" } else { reason };
    let command = operator::origin_of(&form.origin).map(|origin| Command::GatewayCut { origin, reason: reason.to_string() });
    change_gateway(&admin, &headers, command, "gateway-cut").await
}

async fn gateway_restore(State(admin): State<Arc<Admin>>, headers: HeaderMap, Form(form): Form<GatewayForm>) -> Response {
    let command = operator::origin_of(&form.origin).map(Command::GatewayRestore);
    change_gateway(&admin, &headers, command, "gateway-restored").await
}

async fn gateway_tier(State(admin): State<Arc<Admin>>, headers: HeaderMap, Form(form): Form<GatewayForm>) -> Response {
    let tier = match form.tier.as_str() {
        "auto" => Ok(None),
        text => Tier::parse(text).map(Some).ok_or("unknown tier"),
    };
    let command = operator::origin_of(&form.origin).and_then(|origin| tier.map(|tier| Command::GatewayTier { origin, tier }));
    change_gateway(&admin, &headers, command, "gateway-tier").await
}

async fn change_gateway(admin: &Admin, headers: &HeaderMap, command: Result<Command, &'static str>, done: &str) -> Response {
    if let Err(refused) = admin.allowed(headers) {
        return refused;
    }
    let Ok(command) = command else {
        return (StatusCode::BAD_REQUEST, Html(admin.page(None, Some("That server address is not one the gateway knows.")).await)).into_response();
    };
    match operator::run(&admin.service.store, command).await {
        Ok(_) => Redirect::to(&format!("/?done={done}")).into_response(),
        Err(e) => (StatusCode::SERVICE_UNAVAILABLE, format!("The database did not answer: {e}")).into_response(),
    }
}

fn done_note(key: &str) -> Option<&'static str> {
    Some(match key {
        "certificate" => "Your certificate is in use. The relay switches to it within a minute.",
        "automatic" => "Back to automatic certificates. Caddy fetches one if it needs to.",
        "dropped" => "That community is removed from this server.",
        "dismissed" => "Report dismissed.",
        "gateway-cut" => "That server is cut off. Its pings are refused from now on.",
        "gateway-restored" => "That server may send pings again, starting over at the new server caps.",
        "gateway-tier" => "That server's caps are changed.",
        _ => return None,
    })
}

fn report_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|pair| pair.trim().strip_prefix(name).and_then(|rest| rest.strip_prefix('=')))
}

fn loopback_host(host: &str) -> bool {
    let name = match host.strip_prefix('[') {
        Some(rest) => rest.split_once(']').map_or(rest, |(name, _)| name),
        None => host.rsplit_once(':').map_or(host, |(name, _)| name),
    };
    matches!(name.to_ascii_lowercase().as_str(), "localhost" | "127.0.0.1" | "::1")
}

fn same_origin(headers: &HeaderMap) -> bool {
    let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    if let Some(site) = text("sec-fetch-site") {
        return site == "same-origin" || site == "none";
    }
    match text("origin") {
        Some(origin) => text("host").is_some_and(|host| origin == format!("http://{host}") || origin == format!("https://{host}")),
        None => true,
    }
}

fn write_file(dir: &Path, name: &str, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    let next = dir.join(format!(".{name}.next"));
    std::fs::write(&next, bytes)?;
    set_mode(&next, mode)?;
    std::fs::rename(&next, dir.join(name))
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

fn now_s() -> i64 {
    crate::now_ms() / 1000
}

struct Dashboard<'a> {
    domain: Option<&'a str>,
    version: Option<&'a str>,
    newest: Option<&'a str>,
    database: bool,
    relay: bool,
    relay_url: Option<&'a str>,
    public_relay_fallback: bool,
    connections: usize,
    protocol: u32,
    oldest_protocol: u32,
    totals: Option<Totals>,
    retention_days: u32,
    served: Result<Facts, String>,
    own_certificate: bool,
    operator: &'a str,
    contact: Option<&'a str>,
    emails_reports: bool,
    reports: Result<Vec<(String, String, String, i64, usize)>, crate::store::StoreError>,
    gateway: Option<Overview>,
    now: i64,
}

impl Dashboard<'_> {
    fn render(&self, note: Option<&str>, problem: Option<&str>) -> String {
        let mut body = String::new();
        if let Some(note) = note {
            body += &format!("<p class=\"note\">{}</p>", esc(note));
        }
        if let Some(problem) = problem {
            body += &format!("<p class=\"note bad\">{}</p>", esc(problem));
        }
        body += &self.release();
        body += &self.health();
        body += &self.certificate();
        body += &self.reports();
        body += &self.gateway();
        body += &self.settings();
        let title = self.domain.unwrap_or("Commy sync server");
        layout(
            title,
            &format!(
                "<header><div><h1>{}</h1><p>Commy sync server admin</p></div><form method=\"post\" action=\"/sign-out\"><button class=\"quiet\">Sign out</button></form></header>{body}",
                esc(title)
            ),
        )
    }

    fn release(&self) -> String {
        let running = match self.version {
            Some(version) => format!("Release {}", esc(version)),
            None => "A development build".into(),
        };
        let (state, advice) = match (self.version.and_then(release_parts), self.newest) {
            (_, None) => ("unknown", "Could not check for a newer release just now.".to_string()),
            (Some(current), Some(newest)) if release_parts(newest).is_some_and(|n| n > current) => {
                ("warn", format!("Release {} is out. Run setup.sh again on this server to update.", esc(newest)))
            }
            (Some(_), Some(_)) => ("good", "Up to date.".into()),
            (None, Some(newest)) => ("unknown", format!("The newest release is {}.", esc(newest))),
        };
        card("Version", &format!("{}{}", row("Running", &running, None), row("Updates", &advice, Some(state))))
    }

    fn health(&self) -> String {
        let yes_no = |ok: bool, good: &str, bad: &str| (if ok { good } else { bad }).to_string();
        let mut rows = row("Database", &yes_no(self.database, "Answering", "Not answering"), Some(if self.database { "good" } else { "bad" }));
        rows += &row("Relay", &yes_no(self.relay, "Answering", "Not answering"), Some(if self.relay { "good" } else { "bad" }));
        rows += &row("Connected apps", &self.connections.to_string(), None);
        match &self.totals {
            Some(t) => {
                rows += &row("Streams", &t.streams.to_string(), None);
                rows += &row("Stored entries", &t.entries.to_string(), None);
                rows += &row("Database size", &size(t.bytes), None);
            }
            None => rows += &row("Storage", "Could not read totals", Some("bad")),
        }
        rows += &row("Sync protocol", &format!("{} (apps back to {} can connect)", self.protocol, self.oldest_protocol), None);
        card("Health", &rows)
    }

    fn certificate(&self) -> String {
        let source = if self.own_certificate { "Your own certificate" } else { "Automatic (renewed by Caddy)" };
        let mut rows = row("Source", source, None);
        match &self.served {
            Ok(facts) => {
                let (state, left) = expiry(facts.not_after - self.now, self.own_certificate);
                rows += &row("Issued by", &esc(&facts.issuer), None);
                rows += &row("Expires", &format!("{} ({left})", day(facts.not_after)), Some(state));
                rows += &row("Covers", &esc(&facts.names.join(", ")), None);
            }
            Err(problem) => rows += &row("Served", &esc(problem), Some("bad")),
        }
        let mut form = String::from(
            "<details><summary>Use your own certificate</summary><form method=\"post\" action=\"/certificate\">\
             <label>Certificate (PEM, with any intermediates after it)<textarea name=\"certificate\" rows=\"6\" required spellcheck=\"false\" placeholder=\"-----BEGIN CERTIFICATE-----\"></textarea></label>\
             <label>Private key (PEM)<textarea name=\"key\" rows=\"4\" required spellcheck=\"false\" placeholder=\"-----BEGIN PRIVATE KEY-----\"></textarea></label>\
             <p class=\"hint\">It must cover this server's domain. You'll need to upload a new one before it expires.</p>\
             <button>Use this certificate</button></form></details>",
        );
        if self.own_certificate {
            form += "<form method=\"post\" action=\"/certificate/automatic\"><button class=\"quiet\">Go back to automatic certificates</button></form>";
        }
        card("Certificate", &format!("{rows}{form}"))
    }

    fn reports(&self) -> String {
        let list = match &self.reports {
            Err(e) => format!("<p class=\"hint\">Could not read reports: {}</p>", esc(&e.to_string())),
            Ok(open) if open.is_empty() => "<p class=\"hint\">No open reports.</p>".into(),
            Ok(open) => {
                let items: String = open
                    .iter()
                    .map(|(id, community, reason, received, messages)| {
                        format!(
                            "<li><a href=\"/reports/{}\"><strong>{}</strong><span>{} · {} · {} messages</span></a></li>",
                            esc(id),
                            esc(community),
                            esc(reason),
                            day(received / 1000),
                            messages
                        )
                    })
                    .collect();
                format!("<ul class=\"reports\">{items}</ul>")
            }
        };
        card("Reports", &list)
    }

    fn gateway(&self) -> String {
        let Some(overview) = &self.gateway else {
            return String::new();
        };
        let percent = (overview.used.saturating_mul(100)).checked_div(overview.budget).unwrap_or(100);
        let state = if overview.used >= overview.budget {
            "bad"
        } else if percent >= overview.alert_percent {
            "warn"
        } else {
            "good"
        };
        let mut rows = row("Pings today", &format!("{} of {} ({percent}%)", overview.used, overview.budget), Some(state));
        rows += &row("Refused today", &overview.refused.to_string(), None);
        rows += &row("Servers", &overview.servers.len().to_string(), None);
        let blocklist = match (&overview.blocklist_from, overview.blocked) {
            (None, _) => ("Not used".to_string(), None),
            (Some(from), None) => (format!("Not loaded yet from {}", esc(from)), Some("warn")),
            (Some(from), Some(count)) => (format!("{count} blocked, from {}", esc(from)), Some("good")),
        };
        rows += &row("Server blocklist", &blocklist.0, blocklist.1);
        if overview.servers.is_empty() {
            rows += "<p class=\"hint\">No server has sent a ping yet.</p>";
            return card("Push gateway", &rows);
        }
        let items: String = overview.servers.iter().map(|server| self.gateway_server(server)).collect();
        rows += &format!("<ul class=\"servers\">{items}</ul>");
        rows += "<p class=\"hint\">A server that keeps sending far past its cap is cut off on its own. Restoring one starts it over at the new server caps.</p>";
        card("Push gateway", &rows)
    }

    fn gateway_server(&self, server: &gateway::ServerRow) -> String {
        let origin = esc(&server.record.origin);
        let pinned = if server.record.pinned.is_some() { ", set by you" } else { "" };
        let mut status = format!(
            "{} tier{pinned} · {} of {} pings · {} of {} new phones · {} refused",
            server.tier.as_str(),
            server.today.pushes,
            server.caps.pushes,
            server.today.new_phones,
            server.caps.new_phones,
            server.today.refused
        );
        let action = match &server.record.cut_off {
            Some(cut) => {
                let by = if cut.automatic { "automatically" } else { "by you" };
                status = format!("Cut off {by} on {}: {} · {status}", day(cut.at / 1000), esc(&cut.reason));
                format!("<form method=\"post\" action=\"/gateway/restore\"><input type=\"hidden\" name=\"origin\" value=\"{origin}\"><button class=\"quiet\">Restore</button></form>")
            }
            None => format!(
                "<form method=\"post\" action=\"/gateway/cut\"><input type=\"hidden\" name=\"origin\" value=\"{origin}\"><input name=\"reason\" maxlength=\"200\" placeholder=\"Reason\" aria-label=\"Reason\"><button class=\"danger\">Cut off</button></form>"
            ),
        };
        let chosen = server.record.pinned.map_or("auto", Tier::as_str);
        let options: String = ["auto", "new", "settled", "trusted"]
            .iter()
            .map(|tier| {
                let label = if *tier == "auto" { "Earned" } else { tier };
                let selected = if *tier == chosen { " selected" } else { "" };
                format!("<option value=\"{tier}\"{selected}>{label}</option>")
            })
            .collect();
        let tier = format!(
            "<form method=\"post\" action=\"/gateway/tier\"><input type=\"hidden\" name=\"origin\" value=\"{origin}\"><select name=\"tier\" aria-label=\"Tier\">{options}</select><button class=\"quiet\">Set tier</button></form>"
        );
        let state = if server.record.cut_off.is_some() { "bad" } else if server.today.pushes >= server.caps.pushes { "warn" } else { "good" };
        format!("<li><div><i class=\"dot {state}\"></i><strong>{origin}</strong><span>{status}</span></div><div class=\"actions\">{action}{tier}</div></li>")
    }

    fn settings(&self) -> String {
        let mut rows = row("Operator name", &esc(self.operator), None);
        let contact = match (self.contact, self.emails_reports) {
            (Some(contact), true) => format!("{} (reports are also emailed there)", esc(contact)),
            (Some(contact), false) => esc(contact),
            (None, _) => "Not set".into(),
        };
        rows += &row("Abuse contact", &contact, None);
        rows += &row("Relay", &esc(self.relay_url.unwrap_or("Not set")), None);
        let fallback = if self.public_relay_fallback { "Voice uses the free public relays" } else { "Voice pauses" };
        rows += &row("If the relay is down", fallback, None);
        let retention = if self.retention_days == 0 { "Forever".to_string() } else { format!("{} days", self.retention_days) };
        rows += &row("Keeps messages for", &retention, None);
        rows += "<p class=\"hint\">To change these, edit .env next to setup.sh and run setup.sh again.</p>";
        card("Settings", &rows)
    }
}

fn expiry(seconds_left: i64, own: bool) -> (&'static str, String) {
    let hours = seconds_left.div_euclid(3600);
    let days = seconds_left.div_euclid(86_400);
    let warn_below = if own { 14 * 86_400 } else { 86_400 };
    let state = if seconds_left <= 0 {
        "bad"
    } else if seconds_left < warn_below {
        "warn"
    } else {
        "good"
    };
    let left = match seconds_left {
        s if s <= 0 => "expired".to_string(),
        s if s < 3600 => "less than an hour left".to_string(),
        s if s < 2 * 86_400 => format!("{hours} hours left"),
        _ => format!("{days} days left"),
    };
    (state, left)
}

fn card(title: &str, inner: &str) -> String {
    format!("<section><h2>{title}</h2>{inner}</section>")
}

fn row(label: &str, value: &str, state: Option<&str>) -> String {
    let dot = state.map(|s| format!("<i class=\"dot {s}\"></i>")).unwrap_or_default();
    format!("<div class=\"row\"><span>{label}</span><span>{dot}{value}</span></div>")
}

fn sign_in_page(problem: Option<&str>) -> String {
    let problem = problem.map(|p| format!("<p class=\"note bad\">{}</p>", esc(p))).unwrap_or_default();
    layout(
        "Sign in",
        &format!(
            "<section class=\"narrow\"><h1>Commy sync server</h1>{problem}<form method=\"post\" action=\"/sign-in\">\
             <label>Admin password<input type=\"password\" name=\"password\" autocomplete=\"current-password\" required autofocus></label>\
             <button>Sign in</button></form>\
             <p class=\"hint\">The password is SYNC_ADMIN_PASSWORD in the .env file next to setup.sh.</p></section>"
        ),
    )
}

fn report_page(id: &str, described: &str, state: &str) -> String {
    let actions = if state == "open" {
        format!(
            "<form method=\"post\" action=\"/reports/{id}/dismiss\"><button class=\"quiet\">Dismiss</button></form>\
             <form method=\"post\" action=\"/reports/{id}/drop\"><button class=\"danger\">Remove this community from this server</button></form>\
             <p class=\"hint\">Removing deletes everything this community stored here and refuses it from now on. It can't be undone.</p>"
        )
    } else {
        format!("<p class=\"hint\">This report is {}.</p>", esc(state))
    };
    layout(
        "Report",
        &format!("<p><a href=\"/\">Back</a></p><section><h2>Report {}</h2><pre>{}</pre>{actions}</section>", esc(id), esc(described)),
    )
}

fn layout(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta name=\"color-scheme\" content=\"light dark\"><title>{} · Commy admin</title><style>{STYLE}</style></head><body><main>{body}</main></body></html>",
        esc(title)
    )
}

const STYLE: &str = ":root{--bg:#f6f5fb;--card:#fff;--ink:#1d1b26;--soft:#5d5a6b;--line:#e4e1ee;--accent:#6b4ee6;--good:#1f9d61;--warn:#c88a04;--bad:#d33c4a}\
@media (prefers-color-scheme:dark){:root{--bg:#121118;--card:#1c1a24;--ink:#ecebf3;--soft:#a19db3;--line:#2d2a38;--accent:#9d86ff;--good:#3ccf86;--warn:#f0b43a;--bad:#ff6b78}}\
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--ink);font:15px/1.5 system-ui,-apple-system,Segoe UI,sans-serif}\
main{max-width:760px;margin:0 auto;padding:24px 16px 48px}header{display:flex;justify-content:space-between;align-items:center;gap:12px;margin-bottom:16px}\
h1{font-size:22px;margin:0}h2{font-size:16px;margin:0 0 10px}header p{margin:2px 0 0;color:var(--soft)}\
section{background:var(--card);border:1px solid var(--line);border-radius:12px;padding:16px;margin-bottom:14px}.narrow{max-width:420px;margin:12vh auto 0}\
.row{display:flex;justify-content:space-between;gap:16px;padding:6px 0;border-top:1px solid var(--line)}.row:first-of-type{border-top:0}.row span:first-child{color:var(--soft)}.row span:last-child{text-align:right;overflow-wrap:anywhere}\
.dot{display:inline-block;width:8px;height:8px;border-radius:50%;margin-right:8px;background:var(--soft)}.dot.good{background:var(--good)}.dot.warn{background:var(--warn)}.dot.bad{background:var(--bad)}\
.note{background:var(--card);border:1px solid var(--line);border-left:4px solid var(--good);border-radius:8px;padding:10px 12px}.note.bad{border-left-color:var(--bad)}\
.hint{color:var(--soft);font-size:13px}label{display:block;margin:10px 0;color:var(--soft);font-size:13px}\
input,textarea{display:block;width:100%;margin-top:4px;padding:8px 10px;border:1px solid var(--line);border-radius:8px;background:var(--bg);color:var(--ink);font:13px ui-monospace,monospace}\
button{border:0;border-radius:8px;padding:8px 14px;background:var(--accent);color:#fff;font:inherit;cursor:pointer;margin-top:8px}button.quiet{background:transparent;color:var(--accent);border:1px solid var(--line)}button.danger{background:var(--bad)}\
details summary{cursor:pointer;color:var(--accent);margin-top:10px}pre{white-space:pre-wrap;overflow-wrap:anywhere;font:13px/1.5 ui-monospace,monospace;background:var(--bg);padding:12px;border-radius:8px}\
.reports{list-style:none;margin:0;padding:0}.reports a{display:flex;flex-direction:column;padding:8px 0;border-top:1px solid var(--line);color:var(--ink);text-decoration:none}.reports li:first-child a{border-top:0}.reports span{color:var(--soft);font-size:13px}\
.servers{list-style:none;margin:10px 0 0;padding:0}.servers li{padding:10px 0;border-top:1px solid var(--line)}.servers li>div:first-child{display:flex;flex-wrap:wrap;align-items:baseline;gap:4px 8px}.servers span{color:var(--soft);font-size:13px;overflow-wrap:anywhere}\
.actions{display:flex;flex-wrap:wrap;gap:8px}.actions form{display:flex;gap:6px;align-items:center}.actions input{width:12em;margin:8px 0 0}select{margin-top:8px;padding:7px 8px;border:1px solid var(--line);border-radius:8px;background:var(--bg);color:var(--ink);font:inherit}\
a{color:var(--accent)}";

fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn release_parts(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let parsed = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(parsed)
}

fn size(bytes: u64) -> String {
    let units = ["bytes", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < units.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

fn day(unix_s: i64) -> String {
    let days = unix_s.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_names_reach_the_admin_page() {
        for host in ["localhost:8790", "127.0.0.1:8790", "[::1]:8790", "LOCALHOST", "127.0.0.1"] {
            assert!(loopback_host(host), "{host}");
        }
        for host in ["sync.example.org", "evil.example:8790", "127.0.0.2:8790", "[::2]:8790", "localhost.evil.example"] {
            assert!(!loopback_host(host), "{host}");
        }
    }

    #[test]
    fn forms_must_come_from_the_page_itself() {
        let headers = |pairs: &[(&'static str, &'static str)]| {
            let mut map = HeaderMap::new();
            for (name, value) in pairs {
                map.insert(*name, HeaderValue::from_static(value));
            }
            map
        };
        assert!(same_origin(&headers(&[("host", "localhost:8790"), ("origin", "http://localhost:8790")])));
        assert!(!same_origin(&headers(&[("host", "localhost:8790"), ("origin", "https://evil.example")])));
        assert!(!same_origin(&headers(&[("host", "localhost:8790"), ("sec-fetch-site", "cross-site")])));
        assert!(!same_origin(&headers(&[("host", "localhost:8790"), ("origin", "http://localhost:8790"), ("sec-fetch-site", "same-site")])));
        assert!(same_origin(&headers(&[("host", "localhost:8790"), ("origin", "null"), ("sec-fetch-site", "same-origin")])));
        assert!(!same_origin(&headers(&[("host", "localhost:8790"), ("origin", "null")])));
        assert!(same_origin(&headers(&[("host", "localhost:8790")])));
    }

    #[test]
    fn releases_compare_by_number_and_days_print_as_dates() {
        assert!(release_parts("0.2.10") > release_parts("0.2.9"));
        assert_eq!(release_parts("0.3.0-rc.1"), Some((0, 3, 0)));
        assert_eq!(release_parts("dev"), None);
        assert_eq!(release_parts("1.2"), None);
        assert_eq!(day(0), "1970-01-01");
        assert_eq!(day(1_794_700_800), "2026-11-15");
        assert_eq!(expiry(40 * 86_400, false), ("good", "40 days left".to_string()));
        assert_eq!(expiry(10 * 86_400, true), ("warn", "10 days left".to_string()));
        assert_eq!(expiry(10 * 3600, false), ("warn", "10 hours left".to_string()));
        assert_eq!(expiry(30 * 3600, false).0, "good");
        assert_eq!(expiry(-5, false), ("bad", "expired".to_string()));
        assert_eq!(size(999), "999 bytes");
        assert_eq!(size(16_400_000), "16.4 MB");
        assert_eq!(esc("<a href=\"x\">&'"), "&lt;a href=&quot;x&quot;&gt;&amp;&#39;");
    }
}
