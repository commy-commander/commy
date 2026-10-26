use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinSet;

use super::protocol::{Control, DeckStatus, DenyReason, Directory, Reply, Request, SoundChoice, SoundboardList, PROTOCOL_VERSION};

pub const MAX_LINE_BYTES: u64 = 4096;
pub const MAX_UPLOAD_LINE_BYTES: u64 = (commy_view::pictures::MAX_SOUND_BYTES as u64).div_ceil(3) * 4 + MAX_LINE_BYTES;
pub const MAX_CONNECTIONS: usize = 4;
pub const HELLO_WAIT: Duration = Duration::from_secs(5);
pub const PAIRING_WAIT: Duration = Duration::from_secs(120);
pub const PLAY_COOLDOWN: Duration = Duration::from_millis(1500);
const ACCEPT_RETRY: Duration = Duration::from_millis(250);

pub fn loopback(port: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, port))
}

pub struct PairingAsk {
    pub request: u64,
    pub answer: oneshot::Receiver<bool>,
}

pub trait Backend: Send + Sync + 'static {
    fn is_paired(&self, token: &str) -> bool;
    fn ask_to_pair(&self, name: &str) -> Option<PairingAsk>;
    fn withdraw_pairing(&self, request: u64);
    fn finish_pairing(&self, name: &str) -> String;
    fn soundboards(&self) -> SoundboardList;
    fn play(&self, server: Option<String>, sound: String) -> impl Future<Output = Result<(), String>> + Send;
    fn add_sound(&self, upload: SoundUpload) -> impl Future<Output = Result<SoundChoice, String>> + Send;
    fn status(&self) -> DeckStatus;
    fn directory(&self) -> impl Future<Output = Directory> + Send;
    fn control(&self, control: Control) -> impl Future<Output = Result<(), String>> + Send;
}

#[derive(Default)]
struct Holds {
    talk: bool,
    mute: bool,
}

impl Holds {
    fn note(&mut self, control: &Control) {
        match control {
            Control::Talk { down } => self.talk = *down,
            Control::HoldMute { down } => self.mute = *down,
            _ => {}
        }
    }

    fn releases(&self) -> Vec<Control> {
        let talk = self.talk.then_some(Control::Talk { down: false });
        let mute = self.mute.then_some(Control::HoldMute { down: false });
        talk.into_iter().chain(mute).collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SoundUpload {
    pub server: String,
    pub name: String,
    pub emoji: Option<String>,
    pub data: String,
}

pub async fn bind(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(loopback(port)).await
}

pub async fn listen<B: Backend>(listener: TcpListener, backend: Arc<B>) {
    let mut connections = JoinSet::new();
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                tracing::debug!("stream deck accept failed: {e}");
                tokio::time::sleep(ACCEPT_RETRY).await;
                continue;
            }
        };
        while connections.try_join_next().is_some() {}
        if !peer.ip().is_loopback() || connections.len() >= MAX_CONNECTIONS {
            continue;
        }
        let _ = stream.set_nodelay(true);
        connections.spawn(serve(stream, backend.clone()));
    }
}

async fn serve<B: Backend>(stream: TcpStream, backend: Arc<B>) {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let Ok(Some(Request::Hello { version, name, token })) =
        tokio::time::timeout(HELLO_WAIT, read_request(&mut reader, MAX_LINE_BYTES)).await
    else {
        return;
    };
    if version != PROTOCOL_VERSION {
        let _ = send(&mut write, &Reply::Denied { reason: DenyReason::Unsupported }).await;
        return;
    }
    let token = match token.filter(|t| backend.is_paired(t)) {
        Some(token) => token,
        None => match pair(&*backend, name.as_deref().unwrap_or_default(), &mut reader, &mut write).await {
            Some(token) => token,
            None => return,
        },
    };
    if send(&mut write, &Reply::Ready { version: PROTOCOL_VERSION }).await.is_err() {
        return;
    }
    let mut holds = Holds::default();
    talk_to_deck(&*backend, &token, &mut reader, &mut write, &mut holds).await;
    for release in holds.releases() {
        let _ = backend.control(release).await;
    }
}

async fn talk_to_deck<B: Backend, R, W>(backend: &B, token: &str, reader: &mut R, write: &mut W, holds: &mut Holds)
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut last_play: Option<Instant> = None;
    while let Some(request) = read_request(reader, MAX_UPLOAD_LINE_BYTES).await {
        if !backend.is_paired(token) {
            let _ = send(write, &Reply::Denied { reason: DenyReason::Removed }).await;
            return;
        }
        let reply = match request {
            Request::Hello { .. } => return,
            Request::List => Reply::Soundboards(backend.soundboards()),
            Request::Play { id, server, sound } => {
                if last_play.is_some_and(|at| at.elapsed() < PLAY_COOLDOWN) {
                    Reply::Failed { id, message: "wait a moment between sounds".into() }
                } else {
                    last_play = Some(Instant::now());
                    match backend.play(server, sound).await {
                        Ok(()) => Reply::Played { id },
                        Err(message) => Reply::Failed { id, message },
                    }
                }
            }
            Request::AddSound { id, server, name, emoji, data } => {
                match backend.add_sound(SoundUpload { server: server.clone(), name, emoji, data }).await {
                    Ok(sound) => Reply::Added { id, server, sound },
                    Err(message) => Reply::Failed { id, message },
                }
            }
            Request::Status => Reply::Status(backend.status()),
            Request::Directory => Reply::Directory(backend.directory().await),
            Request::Control { id, control } => {
                holds.note(&control);
                match backend.control(control).await {
                    Ok(()) => Reply::Done { id, status: backend.status() },
                    Err(message) => Reply::Failed { id, message },
                }
            }
        };
        if send(write, &reply).await.is_err() {
            return;
        }
    }
}

async fn pair<B: Backend, R, W>(backend: &B, name: &str, reader: &mut R, write: &mut W) -> Option<String>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let name = super::protocol::deck_name(name);
    let Some(PairingAsk { request, answer }) = backend.ask_to_pair(&name) else {
        let _ = send(write, &Reply::Denied { reason: DenyReason::Busy }).await;
        return None;
    };
    if send(write, &Reply::Waiting).await.is_err() {
        backend.withdraw_pairing(request);
        return None;
    }
    let allowed = tokio::select! {
        answer = answer => answer.unwrap_or(false),
        _ = tokio::time::sleep(PAIRING_WAIT) => false,
        _ = reader.fill_buf() => false,
    };
    if !allowed {
        backend.withdraw_pairing(request);
        let _ = send(write, &Reply::Denied { reason: DenyReason::Declined }).await;
        return None;
    }
    let token = backend.finish_pairing(&name);
    send(write, &Reply::Paired { token: token.clone() }).await.ok()?;
    Some(token)
}

async fn read_request<R: AsyncBufRead + Unpin>(reader: &mut R, limit: u64) -> Option<Request> {
    let mut line = String::new();
    let read = reader.take(limit).read_line(&mut line).await.ok()?;
    if read == 0 || !line.ends_with('\n') {
        return None;
    }
    Request::parse(&line)
}

async fn send<W: AsyncWrite + Unpin>(write: &mut W, reply: &Reply) -> std::io::Result<()> {
    write.write_all(reply.to_line().as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_deck::protocol::{SoundChoice, SoundboardServer};
    use std::sync::Mutex;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    #[derive(Default)]
    struct FakeApp {
        tokens: Mutex<Vec<String>>,
        answers: Mutex<Option<oneshot::Sender<bool>>>,
        asked: Mutex<Vec<String>>,
        withdrawn: Mutex<Vec<u64>>,
        played: Mutex<Vec<(Option<String>, String)>>,
        added: Mutex<Vec<SoundUpload>>,
        controls: Mutex<Vec<Control>>,
    }

    impl Backend for FakeApp {
        fn is_paired(&self, token: &str) -> bool {
            self.tokens.lock().unwrap().iter().any(|t| t == token)
        }

        fn ask_to_pair(&self, name: &str) -> Option<PairingAsk> {
            let mut answers = self.answers.lock().unwrap();
            if answers.is_some() {
                return None;
            }
            let (tx, rx) = oneshot::channel();
            *answers = Some(tx);
            self.asked.lock().unwrap().push(name.to_string());
            Some(PairingAsk { request: 1, answer: rx })
        }

        fn withdraw_pairing(&self, request: u64) {
            self.answers.lock().unwrap().take();
            self.withdrawn.lock().unwrap().push(request);
        }

        fn finish_pairing(&self, _name: &str) -> String {
            self.answers.lock().unwrap().take();
            let token = format!("token-{}", self.tokens.lock().unwrap().len());
            self.tokens.lock().unwrap().push(token.clone());
            token
        }

        fn soundboards(&self) -> SoundboardList {
            SoundboardList {
                voice_server: Some("s1".into()),
                defaults: vec![SoundChoice { name: "default:airhorn".into(), label: "Airhorn".into(), emoji: None }],
                servers: vec![SoundboardServer { id: "s1".into(), name: "Friends".into(), can_play: true, can_add: true, sounds: vec![] }],
            }
        }

        async fn play(&self, server: Option<String>, sound: String) -> Result<(), String> {
            if sound == "missing" {
                return Err("that sound no longer exists".into());
            }
            self.played.lock().unwrap().push((server, sound));
            Ok(())
        }

        async fn add_sound(&self, upload: SoundUpload) -> Result<SoundChoice, String> {
            if upload.server != "s1" {
                return Err("you do not have permission to add sounds".into());
            }
            let sound = SoundChoice { name: upload.name.clone(), label: upload.name.clone(), emoji: upload.emoji.clone() };
            self.added.lock().unwrap().push(upload);
            Ok(sound)
        }

        fn status(&self) -> DeckStatus {
            let muted = self.controls.lock().unwrap().iter().filter(|c| **c == Control::ToggleMute).count() % 2 == 1;
            DeckStatus { muted, unread: 3, ..DeckStatus::default() }
        }

        async fn directory(&self) -> Directory {
            Directory { inputs: vec!["Desk mic".into()], ..Directory::default() }
        }

        async fn control(&self, control: Control) -> Result<(), String> {
            if control == Control::ToggleCamera {
                return Err("join a voice channel first".into());
            }
            self.controls.lock().unwrap().push(control);
            Ok(())
        }
    }

    impl FakeApp {
        fn answer(&self, allow: bool) {
            if let Some(tx) = self.answers.lock().unwrap().take() {
                let _ = tx.send(allow);
            }
        }
    }

    struct Client {
        reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
        writer: tokio::net::tcp::OwnedWriteHalf,
    }

    impl Client {
        async fn connect(port: u16) -> Self {
            let stream = TcpStream::connect(loopback(port)).await.unwrap();
            let (read, writer) = stream.into_split();
            Self { reader: BufReader::new(read), writer }
        }

        async fn send(&mut self, line: &str) {
            self.writer.write_all(format!("{line}\n").as_bytes()).await.unwrap();
        }

        async fn recv(&mut self) -> Option<serde_json::Value> {
            let mut line = String::new();
            let read = tokio::time::timeout(Duration::from_secs(5), self.reader.read_line(&mut line)).await.unwrap().unwrap_or(0);
            (read > 0).then(|| serde_json::from_str(&line).unwrap())
        }
    }

    async fn start() -> (u16, Arc<FakeApp>) {
        let listener = bind(0).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = Arc::new(FakeApp::default());
        tokio::spawn(listen(listener, app.clone()));
        (port, app)
    }

    async fn wait_until(check: impl Fn() -> bool) {
        for _ in 0..200 {
            if check() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("condition never held");
    }

    #[test]
    fn listens_on_loopback_only() {
        assert!(loopback(4000).ip().is_loopback());
    }

    #[tokio::test]
    async fn a_browser_request_gets_nothing_back() {
        let (port, app) = start().await;
        let mut client = Client::connect(port).await;
        client.send("GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n").await;
        assert_eq!(client.recv().await, None);
        assert!(app.asked.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_unknown_deck_waits_for_approval_then_plays() {
        let (port, app) = start().await;
        let mut client = Client::connect(port).await;
        client.send(r#"{"type":"hello","version":1,"name":"Desk"}"#).await;
        assert_eq!(client.recv().await.unwrap()["type"], "waiting");
        assert_eq!(*app.asked.lock().unwrap(), vec!["Desk".to_string()]);
        app.answer(true);
        let paired = client.recv().await.unwrap();
        assert_eq!(paired["type"], "paired");
        assert_eq!(paired["token"], "token-0");
        assert_eq!(client.recv().await.unwrap()["type"], "ready");

        client.send(r#"{"type":"list"}"#).await;
        let list = client.recv().await.unwrap();
        assert_eq!(list["type"], "soundboards");
        assert_eq!(list["servers"][0]["name"], "Friends");

        client.send(r#"{"type":"play","id":3,"sound":"default:airhorn"}"#).await;
        assert_eq!(client.recv().await.unwrap(), serde_json::json!({ "type": "played", "id": 3 }));
        assert_eq!(*app.played.lock().unwrap(), vec![(None, "default:airhorn".to_string())]);
    }

    #[tokio::test]
    async fn a_declined_deck_is_told_and_dropped() {
        let (port, app) = start().await;
        let mut client = Client::connect(port).await;
        client.send(r#"{"type":"hello","version":1}"#).await;
        assert_eq!(client.recv().await.unwrap()["type"], "waiting");
        app.answer(false);
        assert_eq!(client.recv().await.unwrap(), serde_json::json!({ "type": "denied", "reason": "declined" }));
        assert_eq!(client.recv().await, None);
        assert!(app.tokens.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_second_pairing_while_one_waits_is_busy() {
        let (port, _app) = start().await;
        let mut first = Client::connect(port).await;
        first.send(r#"{"type":"hello","version":1}"#).await;
        assert_eq!(first.recv().await.unwrap()["type"], "waiting");
        let mut second = Client::connect(port).await;
        second.send(r#"{"type":"hello","version":1}"#).await;
        assert_eq!(second.recv().await.unwrap(), serde_json::json!({ "type": "denied", "reason": "busy" }));
    }

    #[tokio::test]
    async fn hanging_up_while_waiting_withdraws_the_prompt() {
        let (port, app) = start().await;
        let mut client = Client::connect(port).await;
        client.send(r#"{"type":"hello","version":1}"#).await;
        assert_eq!(client.recv().await.unwrap()["type"], "waiting");
        drop(client);
        wait_until(|| app.withdrawn.lock().unwrap().as_slice() == [1]).await;
    }

    #[tokio::test]
    async fn a_paired_token_skips_the_prompt_and_a_wrong_one_does_not() {
        let (port, app) = start().await;
        app.tokens.lock().unwrap().push("known".into());
        let mut client = Client::connect(port).await;
        client.send(r#"{"type":"hello","version":1,"token":"known"}"#).await;
        assert_eq!(client.recv().await.unwrap()["type"], "ready");
        assert!(app.asked.lock().unwrap().is_empty());

        let mut stranger = Client::connect(port).await;
        stranger.send(r#"{"type":"hello","version":1,"token":"guess"}"#).await;
        assert_eq!(stranger.recv().await.unwrap()["type"], "waiting");
    }

    #[tokio::test]
    async fn a_removed_deck_is_cut_off_at_its_next_request() {
        let (port, app) = start().await;
        app.tokens.lock().unwrap().push("known".into());
        let mut client = Client::connect(port).await;
        client.send(r#"{"type":"hello","version":1,"token":"known"}"#).await;
        assert_eq!(client.recv().await.unwrap()["type"], "ready");
        app.tokens.lock().unwrap().clear();
        client.send(r#"{"type":"list"}"#).await;
        assert_eq!(client.recv().await.unwrap(), serde_json::json!({ "type": "denied", "reason": "removed" }));
        assert_eq!(client.recv().await, None);
    }

    #[tokio::test]
    async fn rapid_presses_are_held_back_and_failures_carry_the_reason() {
        let (port, app) = start().await;
        app.tokens.lock().unwrap().push("known".into());
        let mut client = Client::connect(port).await;
        client.send(r#"{"type":"hello","version":1,"token":"known"}"#).await;
        client.recv().await.unwrap();
        client.send(r#"{"type":"play","id":1,"server":"s1","sound":"missing"}"#).await;
        assert_eq!(
            client.recv().await.unwrap(),
            serde_json::json!({ "type": "failed", "id": 1, "message": "that sound no longer exists" })
        );
        client.send(r#"{"type":"play","id":2,"server":"s1","sound":"boom"}"#).await;
        assert_eq!(
            client.recv().await.unwrap(),
            serde_json::json!({ "type": "failed", "id": 2, "message": "wait a moment between sounds" })
        );
        assert!(app.played.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_paired_deck_can_upload_a_full_size_sound() {
        let (port, app) = start().await;
        app.tokens.lock().unwrap().push("known".into());
        let mut client = Client::connect(port).await;
        client.send(r#"{"type":"hello","version":1,"token":"known"}"#).await;
        client.recv().await.unwrap();
        let data = "A".repeat((commy_view::pictures::MAX_SOUND_BYTES / 3 + 1) * 4);
        let request = serde_json::json!({ "type": "add_sound", "id": 4, "server": "s1", "name": "boom", "emoji": "💥", "data": data });
        client.send(&request.to_string()).await;
        assert_eq!(
            client.recv().await.unwrap(),
            serde_json::json!({ "type": "added", "id": 4, "server": "s1", "sound": { "name": "boom", "label": "boom", "emoji": "💥" } })
        );
        assert_eq!(app.added.lock().unwrap()[0].data.len(), data.len());

        client.send(r#"{"type":"add_sound","id":5,"server":"s2","name":"boom","data":"AA=="}"#).await;
        assert_eq!(
            client.recv().await.unwrap(),
            serde_json::json!({ "type": "failed", "id": 5, "message": "you do not have permission to add sounds" })
        );
    }

    async fn paired_client(port: u16, app: &FakeApp) -> Client {
        app.tokens.lock().unwrap().push("known".into());
        let mut client = Client::connect(port).await;
        client.send(r#"{"type":"hello","version":1,"token":"known"}"#).await;
        assert_eq!(client.recv().await.unwrap()["type"], "ready");
        client
    }

    #[tokio::test]
    async fn controls_answer_with_the_new_status() {
        let (port, app) = start().await;
        let mut client = paired_client(port, &app).await;
        client.send(r#"{"type":"control","id":1,"control":{"action":"toggle_mute"}}"#).await;
        let done = client.recv().await.unwrap();
        assert_eq!(done["type"], "done");
        assert_eq!(done["id"], 1);
        assert_eq!(done["status"]["muted"], true);
        client.send(r#"{"type":"control","id":2,"control":{"action":"toggle_camera"}}"#).await;
        assert_eq!(
            client.recv().await.unwrap(),
            serde_json::json!({ "type": "failed", "id": 2, "message": "join a voice channel first" })
        );
        client.send(r#"{"type":"status"}"#).await;
        let status = client.recv().await.unwrap();
        assert_eq!(status["type"], "status");
        assert_eq!(status["unread"], 3);
        client.send(r#"{"type":"directory"}"#).await;
        let directory = client.recv().await.unwrap();
        assert_eq!(directory["type"], "directory");
        assert_eq!(directory["inputs"][0], "Desk mic");
    }

    #[tokio::test]
    async fn hanging_up_lets_go_of_held_talk_and_mute_keys() {
        let (port, app) = start().await;
        let mut client = paired_client(port, &app).await;
        client.send(r#"{"type":"control","id":1,"control":{"action":"talk","down":true}}"#).await;
        client.recv().await.unwrap();
        client.send(r#"{"type":"control","id":2,"control":{"action":"hold_mute","down":true}}"#).await;
        client.recv().await.unwrap();
        drop(client);
        wait_until(|| app.controls.lock().unwrap().len() == 4).await;
        assert_eq!(
            app.controls.lock().unwrap()[2..],
            [Control::Talk { down: false }, Control::HoldMute { down: false }]
        );
    }

    #[tokio::test]
    async fn a_released_key_is_not_released_again_on_hang_up() {
        let (port, app) = start().await;
        let mut client = paired_client(port, &app).await;
        client.send(r#"{"type":"control","id":1,"control":{"action":"talk","down":true}}"#).await;
        client.recv().await.unwrap();
        client.send(r#"{"type":"control","id":2,"control":{"action":"talk","down":false}}"#).await;
        client.recv().await.unwrap();
        drop(client);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(app.controls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn uploads_need_a_paired_deck() {
        let (port, app) = start().await;
        let mut client = Client::connect(port).await;
        client.send(r#"{"type":"add_sound","id":1,"server":"s1","name":"boom","data":"AA=="}"#).await;
        assert_eq!(client.recv().await, None);
        assert!(app.added.lock().unwrap().is_empty());
        assert!(app.asked.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_newer_protocol_is_refused() {
        let (port, app) = start().await;
        let mut client = Client::connect(port).await;
        client.send(r#"{"type":"hello","version":2}"#).await;
        assert_eq!(client.recv().await.unwrap(), serde_json::json!({ "type": "denied", "reason": "unsupported" }));
        assert!(app.asked.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_overlong_line_closes_the_connection() {
        let (port, _app) = start().await;
        let mut client = Client::connect(port).await;
        client.send(&"x".repeat(MAX_LINE_BYTES as usize + 10)).await;
        assert_eq!(client.recv().await, None);
    }

    #[tokio::test]
    async fn an_upload_past_the_sound_limit_closes_the_connection() {
        let (port, app) = start().await;
        app.tokens.lock().unwrap().push("known".into());
        let mut client = Client::connect(port).await;
        client.send(r#"{"type":"hello","version":1,"token":"known"}"#).await;
        client.recv().await.unwrap();
        client.send(&"x".repeat(MAX_UPLOAD_LINE_BYTES as usize + 10)).await;
        assert_eq!(client.recv().await, None);
    }
}
