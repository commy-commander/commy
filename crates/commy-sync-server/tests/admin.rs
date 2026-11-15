use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::routing::post;
use commy_sync_server::config::Config;
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
    let config = Config::from_env(|key| env.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone())).unwrap();
    let service = Service::new(Store::memory(), &config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(commy_sync_server::admin::serve(listener, service, config.admin.clone().unwrap()));
    let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    Page { url, tls, http }
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
