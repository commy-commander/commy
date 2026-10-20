use crate::compat::{self, Embed};
use crate::handler::Act;
use crate::storage::DataDir;
use crate::{ActError, BotStore};
use chrono::Utc;
use commy_core::{ChannelId, ServerId};
use data_encoding::HEXLOWER;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

pub const DEFAULT_PORT: u16 = 47621;
pub const MAX_BODY_BYTES: usize = 32 * 1024;
pub const PER_MINUTE: usize = 30;
pub const MAX_WEBHOOKS: usize = 50;
pub const MAX_NAME_CHARS: usize = 40;
pub const MAX_CONNECTIONS: usize = 32;
const MINUTE: Duration = Duration::from_secs(60);
const HEADER_WAIT: Duration = Duration::from_secs(10);

pub fn default_addr() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, DEFAULT_PORT))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Webhook {
    pub id: String,
    pub secret: String,
    pub name: String,
    pub server: ServerId,
    pub channel: ChannelId,
}

impl Webhook {
    pub fn new(name: &str, server: ServerId, channel: ChannelId) -> Self {
        Self {
            id: HEXLOWER.encode(&rand::random::<[u8; 6]>()),
            secret: HEXLOWER.encode(&rand::random::<[u8; 24]>()),
            name: name.trim().chars().filter(|c| !c.is_control()).take(MAX_NAME_CHARS).collect(),
            server,
            channel,
        }
    }

    pub fn path(&self) -> String {
        format!("/webhooks/{}/{}", self.id, self.secret)
    }

    fn opens(&self, id: &str, secret: &str) -> bool {
        let (a, b) = (self.secret.as_bytes(), secret.as_bytes());
        self.id == id && a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Post {
    pub content: Option<String>,
    pub embeds: Vec<Embed>,
}

pub struct Webhooks {
    store: Arc<BotStore>,
    data: DataDir,
    recent: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl Webhooks {
    pub fn new(store: Arc<BotStore>, data: DataDir) -> Self {
        Self { store, data, recent: Mutex::default() }
    }

    pub fn post(&self, id: &str, secret: &str, body: &[u8]) -> (StatusCode, String) {
        let hooks = match self.data.webhooks() {
            Ok(hooks) => hooks,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
        };
        let Some(hook) = hooks.into_iter().find(|h| h.opens(id, secret)) else {
            return (StatusCode::NOT_FOUND, "unknown webhook".into());
        };
        let post: Post = match serde_json::from_slice(body) {
            Ok(post) => post,
            Err(e) => return (StatusCode::BAD_REQUEST, format!("expected JSON with content or embeds: {e}")),
        };
        if !self.allow(&hook.id) {
            return (StatusCode::TOO_MANY_REQUESTS, format!("at most {PER_MINUTE} posts a minute per webhook"));
        }
        let result = {
            let mut bot = self.store.bot();
            let content = match bot.replica(hook.server).and_then(|r| r.state.as_ref()) {
                Some(state) => compat::compose(post.content.as_deref(), &post.embeds, state),
                None => return (StatusCode::SERVICE_UNAVAILABLE, ActError::NotReady.to_string()),
            };
            bot.perform(hook.server, Act::Send { channel: hook.channel, content, reply_to: None }, Utc::now())
        };
        match result {
            Ok(payload) => {
                self.store.publish(hook.server, payload);
                (StatusCode::NO_CONTENT, String::new())
            }
            Err(e) => (status_for(&e), e.to_string()),
        }
    }

    fn allow(&self, id: &str) -> bool {
        let mut recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
        let times = recent.entry(id.to_string()).or_default();
        let now = Instant::now();
        while times.front().is_some_and(|t| now.duration_since(*t) >= MINUTE) {
            times.pop_front();
        }
        if times.len() >= PER_MINUTE {
            return false;
        }
        times.push_back(now);
        true
    }

    async fn answer(&self, request: Request<Incoming>) -> Response<Full<Bytes>> {
        let parts: Vec<String> = request.uri().path().trim_matches('/').split('/').map(str::to_string).collect();
        let [root, id, secret] = parts.as_slice() else { return reply(StatusCode::NOT_FOUND, "not found".into()) };
        if root != "webhooks" {
            return reply(StatusCode::NOT_FOUND, "not found".into());
        }
        if request.method() != Method::POST {
            return reply(StatusCode::METHOD_NOT_ALLOWED, "use POST".into());
        }
        let body = match Limited::new(request.into_body(), MAX_BODY_BYTES).collect().await {
            Ok(body) => body.to_bytes(),
            Err(_) => return reply(StatusCode::PAYLOAD_TOO_LARGE, format!("bodies are at most {MAX_BODY_BYTES} bytes")),
        };
        let (status, text) = self.post(id, secret, &body);
        reply(status, text)
    }
}

fn status_for(error: &ActError) -> StatusCode {
    match error {
        ActError::NotReady => StatusCode::SERVICE_UNAVAILABLE,
        ActError::Empty | ActError::TooLong | ActError::Held(_) => StatusCode::UNPROCESSABLE_ENTITY,
        ActError::UnknownChannel | ActError::UnknownServer => StatusCode::GONE,
        _ => StatusCode::FORBIDDEN,
    }
}

fn reply(status: StatusCode, text: String) -> Response<Full<Bytes>> {
    let body = if text.is_empty() { String::new() } else { serde_json::json!({"error": text}).to_string() };
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() = status;
    response
}

pub async fn listen(listener: TcpListener, hooks: Arc<Webhooks>) {
    let mut connections = JoinSet::new();
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(e) => {
                tracing::debug!("webhook accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            }
        };
        while connections.try_join_next().is_some() {}
        if connections.len() >= MAX_CONNECTIONS {
            continue;
        }
        let hooks = hooks.clone();
        connections.spawn(async move {
            let service = service_fn(move |request| {
                let hooks = hooks.clone();
                async move { Ok::<_, Infallible>(hooks.answer(request).await) }
            });
            let connection = http1::Builder::new()
                .timer(hyper_util::rt::TokioTimer::new())
                .header_read_timeout(HEADER_WAIT)
                .keep_alive(false)
                .serve_connection(TokioIo::new(stream), service);
            if let Err(e) = connection.await {
                tracing::debug!("webhook connection ended: {e}");
            }
        });
    }
}
