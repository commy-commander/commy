use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::routing::post;
use commy_sync_server::config::Config;
use commy_sync_server::ramp::{Day, ServerRecord};
use commy_sync_server::store::Store;
use commy_sync_server::Service;
use reqwest::header::{COOKIE, HOST, LOCATION, ORIGIN, SET_COOKIE};
use reqwest::StatusCode;

const PASSWORD: &str = "correct horse battery staple";

struct Caddy {
    url: String,
    loads: Arc<Mutex<Vec<String>>>,
}

async fn caddy(accepts: bool) -> Caddy {
    let loads = Arc::new(Mutex::new(Vec::new()));
    let seen = loads.clone();
    let app = axum::Router::new().route(
        "/load",
        post(move |body: String| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(body);
                if accepts {
                    (axum::http::StatusCode::OK, String::new())
                } else {
                    (axum::http::StatusCode::BAD_REQUEST, "loading new config: bad certificate".to_string())
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });
    Caddy { url, loads }
}

struct Page {
    url: String,
    tls: PathBuf,
    http: reqwest::Client,
    service: Arc<Service>,
}

impl Page {
    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.http.get(format!("{}{path}", self.url)).header(HOST, "localhost:8790")
    }

    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.http.post(format!("{}{path}", self.url)).header(HOST, "localhost:8790").header(ORIGIN, "http://localhost:8790")
    }

    async fn sign_in(&self) -> String {
        let response = self.post("/sign-in").form(&[("password", PASSWORD)]).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let cookie = response.headers()[SET_COOKIE].to_str().unwrap().to_string();
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
        cookie.split(';').next().unwrap().to_string()
    }
}

async fn open(caddy: &Caddy) -> Page {
    open_with(caddy, &[]).await
}

async fn open_with(caddy: &Caddy, extra: &[(&str, &str)]) -> Page {
    let tls = std::env::temp_dir().join(format!("commy-admin-{}", rand_suffix()));
    std::fs::create_dir_all(&tls).unwrap();
    let caddyfile = tls.join("Caddyfile");
    std::fs::write(&caddyfile, "{$SYNC_DOMAIN} {\n\timport /custom-tls/*.caddy\n}\n").unwrap();
    let env = [
        ("SYNC_ADMIN_PASSWORD", PASSWORD.to_string()),
        ("SYNC_DOMAIN", "sync.example.org".to_string()),
        ("SYNC_CUSTOM_TLS", tls.display().to_string()),
        ("SYNC_CADDYFILE", caddyfile.display().to_string()),
        ("SYNC_CADDY_ADMIN", caddy.url.clone()),
        ("SYNC_CADDY_TLS", "127.0.0.1:9".to_string()),
        ("SYNC_RELAY_PROBE", "http://127.0.0.1:9/generate_204".to_string()),
        ("SYNC_RELEASES_URL", "http://127.0.0.1:9/latest.json".to_string()),
        ("COMMY_VERSION", "0.2.5".to_string()),
    ];
    let extra: Vec<(&str, String)> = extra.iter().map(|(k, v)| (*k, v.to_string())).collect();
    let env: Vec<(&str, String)> = env.into_iter().chain(extra).collect();
    let config = Config::from_env(|key| env.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone())).unwrap();
    let service = Service::new(Store::memory(), &config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(commy_sync_server::admin::serve(listener, service.clone(), config.admin.clone().unwrap()));
    let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    Page { url, tls, http, service }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

fn rand_suffix() -> String {
    format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos())
}

fn pair(name: &str) -> (String, String) {
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec![name.to_string()]).unwrap().self_signed(&key).unwrap();
    (cert.pem(), key.serialize_pem())
}

#[tokio::test]
async fn the_admin_page_needs_its_password_and_a_local_address() {
    let caddy = caddy(true).await;
    let page = open(&caddy).await;
    let first = page.get("/").send().await.unwrap();
    assert_eq!(first.headers()["cache-control"], "no-store");
    assert!(first.headers()["content-security-policy"].to_str().unwrap().contains("default-src 'none'"));
    assert!(first.text().await.unwrap().contains("Admin password"));

    let wrong = page.post("/sign-in").form(&[("password", "guess")]).send().await.unwrap();
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    let elsewhere = page.http.post(format!("{}/sign-in", page.url)).header(HOST, "localhost:8790").header(ORIGIN, "https://evil.example");
    assert_eq!(elsewhere.form(&[("password", PASSWORD)]).send().await.unwrap().status(), StatusCode::FORBIDDEN);
    let rebound = page.http.get(format!("{}/", page.url)).header(HOST, "evil.example:8790").send().await.unwrap();
    assert_eq!(rebound.status(), StatusCode::MISDIRECTED_REQUEST);

    let cookie = page.sign_in().await;
    let dashboard = page.get("/").header(COOKIE, &cookie).send().await.unwrap().text().await.unwrap();
    for shown in ["sync.example.org", "Release 0.2.5", "Could not check for a newer release", "Not answering", "No open reports", "Automatic"] {
        assert!(dashboard.contains(shown), "the dashboard shows {shown}");
    }
    assert!(!dashboard.contains(PASSWORD));
    let forged = page.get("/").header(COOKIE, "commy_admin=99999999999.00").send().await.unwrap().text().await.unwrap();
    assert!(forged.contains("Admin password"), "a made-up session is not signed in");
    let unsigned = page.post("/certificate/automatic").send().await.unwrap();
    assert_eq!(unsigned.status(), StatusCode::SEE_OTHER, "forms need a session");
    assert!(caddy.loads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_operator_can_use_their_own_certificate_and_go_back() {
    let caddy = caddy(true).await;
    let page = open(&caddy).await;
    let cookie = page.sign_in().await;

    let (other, other_key) = pair("other.example.org");
    let refused = page.post("/certificate").header(COOKIE, &cookie).form(&[("certificate", other.as_str()), ("key", other_key.as_str())]).send().await.unwrap();
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert!(refused.text().await.unwrap().contains("this certificate is for other.example.org, not sync.example.org"));
    assert!(!page.tls.join("custom.crt").exists());

    let (cert, key) = pair("sync.example.org");
    let used = page.post("/certificate").header(COOKIE, &cookie).form(&[("certificate", cert.as_str()), ("key", key.as_str())]).send().await.unwrap();
    assert_eq!(used.headers()[LOCATION], "/?done=certificate");
    assert_eq!(std::fs::read_to_string(page.tls.join("custom.crt")).unwrap(), cert.trim());
    let snippet = std::fs::read_to_string(page.tls.join("tls.caddy")).unwrap();
    assert_eq!(snippet, format!("tls {0}/custom.crt {0}/custom.key\n", page.tls.display()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(page.tls.join("custom.key")).unwrap().permissions().mode() & 0o777, 0o640);
    }
    assert_eq!(caddy.loads.lock().unwrap().len(), 1);
    assert!(caddy.loads.lock().unwrap()[0].contains("import /custom-tls/*.caddy"));
    let shown = page.get("/?done=certificate").header(COOKIE, &cookie).send().await.unwrap().text().await.unwrap();
    assert!(shown.contains("Your certificate is in use") && shown.contains("Your own certificate"));

    let back = page.post("/certificate/automatic").header(COOKIE, &cookie).send().await.unwrap();
    assert_eq!(back.headers()[LOCATION], "/?done=automatic");
    for name in ["custom.crt", "custom.key", "tls.caddy"] {
        assert!(!page.tls.join(name).exists(), "{name} is removed");
    }
    assert_eq!(caddy.loads.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_certificate_caddy_refuses_is_rolled_back() {
    let caddy = caddy(false).await;
    let page = open(&caddy).await;
    let cookie = page.sign_in().await;
    let (cert, key) = pair("sync.example.org");
    let refused = page.post("/certificate").header(COOKIE, &cookie).form(&[("certificate", cert.as_str()), ("key", key.as_str())]).send().await.unwrap();
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert!(refused.text().await.unwrap().contains("Caddy refused it: loading new config: bad certificate"));
    for name in ["custom.crt", "custom.key", "tls.caddy"] {
        assert!(!page.tls.join(name).exists(), "{name} is rolled back");
    }
    assert_eq!(caddy.loads.lock().unwrap().len(), 2, "Caddy is reloaded again without it");
}

#[tokio::test]
async fn the_push_gateway_card_shows_each_server_and_can_cut_one_off() {
    let caddy = caddy(true).await;
    let plain = open(&caddy).await;
    let cookie = plain.sign_in().await;
    let without = plain.get("/").header(COOKIE, &cookie).send().await.unwrap().text().await.unwrap();
    assert!(!without.contains("Push gateway"), "no card while the gateway is off");

    let page = open_with(&caddy, &[("SYNC_PUSH_GATEWAY", "on"), ("SYNC_GATEWAY_DAILY_BUDGET", "1000")]).await;
    let cookie = page.sign_in().await;
    let origin = "https://busy.example.org";
    let store = &page.service.store;
    store.put_gateway_server(&ServerRecord::first(origin, now_ms())).await.unwrap();
    let today = commy_sync::wake::wake_day(now_ms());
    store.count_gateway(origin, today, Day { pushes: 800, refused: 12, new_phones: 20 }).await.unwrap();
    let dashboard = page.get("/").header(COOKIE, &cookie).send().await.unwrap().text().await.unwrap();
    for shown in ["Push gateway", "800 of 1000 (80%)", "busy.example.org", "new tier", "800 of 500 pings", "20 of 25 new phones", "12 refused", "Cut off"] {
        assert!(dashboard.contains(shown), "the gateway card shows {shown}");
    }

    let cut = page.post("/gateway/cut").header(COOKIE, &cookie).form(&[("origin", origin), ("reason", "sent spam")]).send().await.unwrap();
    assert_eq!(cut.headers()[LOCATION], "/?done=gateway-cut");
    let record = store.gateway_server(origin).await.unwrap().unwrap();
    assert_eq!(record.cut_off.as_ref().map(|c| (c.reason.as_str(), c.automatic)), Some(("sent spam", false)));
    let shown = page.get("/?done=gateway-cut").header(COOKIE, &cookie).send().await.unwrap().text().await.unwrap();
    assert!(shown.contains("Cut off by you") && shown.contains("Restore"));

    let pinned = page.post("/gateway/tier").header(COOKIE, &cookie).form(&[("origin", origin), ("tier", "trusted")]).send().await.unwrap();
    assert_eq!(pinned.headers()[LOCATION], "/?done=gateway-tier");
    assert_eq!(store.gateway_server(origin).await.unwrap().unwrap().pinned, Some(commy_sync_server::ramp::Tier::Trusted));
    let restored = page.post("/gateway/restore").header(COOKIE, &cookie).form(&[("origin", origin)]).send().await.unwrap();
    assert_eq!(restored.headers()[LOCATION], "/?done=gateway-restored");
    assert_eq!(store.gateway_server(origin).await.unwrap().unwrap().cut_off, None);

    let bad = page.post("/gateway/cut").header(COOKIE, &cookie).form(&[("origin", "http://10.0.0.1"), ("reason", "x")]).send().await.unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    let unsigned = page.post("/gateway/cut").form(&[("origin", origin), ("reason", "x")]).send().await.unwrap();
    assert_eq!(unsigned.status(), StatusCode::SEE_OTHER, "forms need a session");
}
