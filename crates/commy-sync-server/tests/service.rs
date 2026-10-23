use std::sync::Arc;
use std::time::Duration;

use commy_sync::stream::{StreamSecret, APPEND_NONCE_BYTES};
use commy_sync::wake::{wake_day, WakeForget, WakeRegister, WakeRing, WakeTag, MAX_RING_TAGS};
use commy_sync::wire::{Blob, Bytes, ClientFrame, ErrorCode, ServerFrame, MAX_STREAMS_PER_CONNECTION};
use commy_sync::{SyncClient, SyncEvent};
use commy_sync_server::config::Config;
use commy_sync_server::limits::Rate;
use commy_sync_server::store::Store;
use commy_sync_server::wake::Pusher;
use commy_sync_server::{serve, Service};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_tungstenite::tungstenite::Message;

const WAIT: Duration = Duration::from_secs(10);

async fn start() -> (String, Arc<Service>) {
    let mut config = Config::from_env(|_| None).unwrap();
    config.limits.appends = Rate { burst: 1_000.0, per_second: 1_000.0 };
    config.limits.stream_appends = config.limits.appends;
    start_with(config).await
}

async fn start_with(config: Config) -> (String, Arc<Service>) {
    let service = Service::new(Store::memory(), &config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(serve(listener, service.clone()));
    (format!("ws://{address}/sync"), service)
}

async fn next_entry(events: &mut UnboundedReceiver<SyncEvent>) -> (u64, Vec<u8>) {
    loop {
        match tokio::time::timeout(WAIT, events.recv()).await.expect("an entry in time").expect("client running") {
            SyncEvent::Entry { seq, plain, .. } => return (seq, plain),
            _ => continue,
        }
    }
}

async fn caught_up(events: &mut UnboundedReceiver<SyncEvent>) -> Vec<(u64, Vec<u8>)> {
    let mut seen = Vec::new();
    loop {
        match tokio::time::timeout(WAIT, events.recv()).await.expect("caught up in time").expect("client running") {
            SyncEvent::Entry { seq, plain, .. } => seen.push((seq, plain)),
            SyncEvent::CaughtUp { .. } => return seen,
            _ => continue,
        }
    }
}

#[tokio::test]
async fn entries_reach_every_follower_in_order() {
    let (url, _) = start().await;
    let keys = StreamSecret::random().keys();
    let (alice, mut alice_events) = SyncClient::spawn(url.clone());
    let (bob, mut bob_events) = SyncClient::spawn(url.clone());
    alice.follow(keys.clone(), 0);
    bob.follow(keys.clone(), 0);
    assert!(caught_up(&mut alice_events).await.is_empty());
    assert!(caught_up(&mut bob_events).await.is_empty());

    assert_eq!(alice.append(keys.id, b"hello bob".to_vec()).await.unwrap(), 1);
    assert_eq!(bob.append(keys.id, b"hi alice".to_vec()).await.unwrap(), 2);
    assert_eq!(next_entry(&mut bob_events).await, (1, b"hello bob".to_vec()));
    assert_eq!(next_entry(&mut bob_events).await, (2, b"hi alice".to_vec()));
    assert_eq!(next_entry(&mut alice_events).await, (1, b"hello bob".to_vec()), "the author hears its own entry too");
    assert_eq!(next_entry(&mut alice_events).await, (2, b"hi alice".to_vec()));
}

#[tokio::test]
async fn a_newcomer_reads_history_and_a_returning_device_reads_only_what_it_missed() {
    let (url, _) = start().await;
    let keys = StreamSecret::random().keys();
    let (writer, _events) = SyncClient::spawn(url.clone());
    writer.follow(keys.clone(), 0);
    for i in 0..250u32 {
        writer.append(keys.id, i.to_le_bytes().to_vec()).await.unwrap();
    }

    let (newcomer, mut events) = SyncClient::spawn(url.clone());
    newcomer.follow(keys.clone(), 0);
    let history = caught_up(&mut events).await;
    assert_eq!(history.len(), 250, "history spans more than one page");
    assert_eq!(history.first().unwrap(), &(1, 0u32.to_le_bytes().to_vec()));
    assert_eq!(history.last().unwrap(), &(250, 249u32.to_le_bytes().to_vec()));

    let (returning, mut events) = SyncClient::spawn(url.clone());
    returning.follow(keys.clone(), 248);
    let missed = caught_up(&mut events).await;
    assert_eq!(missed.iter().map(|(seq, _)| *seq).collect::<Vec<_>>(), vec![249, 250]);
}

#[tokio::test]
async fn a_device_ahead_of_a_reset_stream_reads_it_from_the_start() {
    let (url, _) = start().await;
    let keys = StreamSecret::random().keys();
    let (writer, _events) = SyncClient::spawn(url.clone());
    writer.follow(keys.clone(), 0);
    writer.append(keys.id, b"after a reset".to_vec()).await.unwrap();
    let (stale, mut events) = SyncClient::spawn(url.clone());
    stale.follow(keys.clone(), 900);
    assert_eq!(caught_up(&mut events).await, vec![(1, b"after a reset".to_vec())]);
}

#[tokio::test]
async fn streams_do_not_leak_into_each_other() {
    let (url, _) = start().await;
    let ours = StreamSecret::random().keys();
    let theirs = StreamSecret::random().keys();
    let (us, mut our_events) = SyncClient::spawn(url.clone());
    us.follow(ours.clone(), 0);
    caught_up(&mut our_events).await;
    let (them, _their_events) = SyncClient::spawn(url.clone());
    them.follow(theirs.clone(), 0);
    them.append(theirs.id, b"private".to_vec()).await.unwrap();
    us.append(ours.id, b"ours".to_vec()).await.unwrap();
    assert_eq!(next_entry(&mut our_events).await, (1, b"ours".to_vec()));
}

#[tokio::test]
async fn appending_to_an_unfollowed_stream_fails_locally() {
    let (url, _) = start().await;
    let (client, _events) = SyncClient::spawn(url);
    let stray = StreamSecret::random().keys();
    assert_eq!(client.append(stray.id, b"x".to_vec()).await, Err(commy_sync::AppendError::NotFollowed));
}

type Raw = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn raw(url: &str) -> Raw {
    let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    assert_eq!(receive(&mut socket).await, ServerFrame::Hello { protocol: 1 });
    socket
}

async fn send(socket: &mut Raw, frame: &ClientFrame) {
    socket.send(Message::Text(serde_json::to_string(frame).unwrap().into())).await.unwrap();
}

async fn receive(socket: &mut Raw) -> ServerFrame {
    loop {
        let message = tokio::time::timeout(WAIT, socket.next()).await.expect("a frame in time").unwrap().unwrap();
        if let Message::Text(text) = message {
            return serde_json::from_str(&text).unwrap();
        }
    }
}

async fn open_raw(socket: &mut Raw, keys: &commy_sync::StreamKeys) {
    send(socket, &ClientFrame::open(keys, 0)).await;
    assert_eq!(receive(socket).await, ServerFrame::Opened { stream: keys.id, head: 0 });
    assert_eq!(receive(socket).await, ServerFrame::Entries { stream: keys.id, entries: vec![], caught_up: true });
}

#[tokio::test]
async fn the_service_refuses_what_it_cannot_check() {
    let (url, _) = start().await;
    let keys = StreamSecret::random().keys();
    let other = StreamSecret::random().keys();
    let mut socket = raw(&url).await;

    send(&mut socket, &ClientFrame::Open { stream: keys.id, write: Bytes(other.write_public()), read: Bytes(keys.read_token()), after: 0, tail: None }).await;
    assert_eq!(receive(&mut socket).await, ServerFrame::stream_error(ErrorCode::BadStream, keys.id));

    let nonce = [1u8; APPEND_NONCE_BYTES];
    send(&mut socket, &ClientFrame::append(&keys, nonce, keys.seal(b"x").unwrap())).await;
    assert!(matches!(receive(&mut socket).await, ServerFrame::Error { code: ErrorCode::NotOpen, nonce: Some(_), .. }));

    open_raw(&mut socket, &keys).await;
    send(&mut socket, &ClientFrame::append(&other, nonce, keys.seal(b"x").unwrap())).await;
    let refused = receive(&mut socket).await;
    assert!(matches!(refused, ServerFrame::Error { code: ErrorCode::NotOpen, .. }), "{refused:?}");
    let forged = ClientFrame::Append { stream: keys.id, nonce: Bytes(nonce), body: Blob(keys.seal(b"x").unwrap()), sig: Bytes([7; 64]) };
    send(&mut socket, &forged).await;
    assert!(matches!(receive(&mut socket).await, ServerFrame::Error { code: ErrorCode::BadSignature, .. }));
    send(&mut socket, &ClientFrame::append(&keys, nonce, vec![0; 700])).await;
    assert!(matches!(receive(&mut socket).await, ServerFrame::Error { code: ErrorCode::TooLarge, .. }), "bodies must be padded");

    socket.send(Message::Text("{\"t\":\"what\"}".into())).await.unwrap();
    assert_eq!(receive(&mut socket).await, ServerFrame::error(ErrorCode::BadFrame));
    send(&mut socket, &ClientFrame::Ping).await;
    assert_eq!(receive(&mut socket).await, ServerFrame::Pong);
}

#[tokio::test]
async fn a_retried_append_is_stored_once() {
    let (url, _) = start().await;
    let keys = StreamSecret::random().keys();
    let mut socket = raw(&url).await;
    open_raw(&mut socket, &keys).await;
    let frame = ClientFrame::append(&keys, [4; APPEND_NONCE_BYTES], keys.seal(b"once").unwrap());
    send(&mut socket, &frame).await;
    let mut appended = Vec::new();
    let mut entries = Vec::new();
    for _ in 0..2 {
        match receive(&mut socket).await {
            ServerFrame::Appended { seq, .. } => appended.push(seq),
            ServerFrame::Entries { entries: e, .. } => entries.extend(e),
            other => panic!("{other:?}"),
        }
    }
    send(&mut socket, &frame).await;
    match receive(&mut socket).await {
        ServerFrame::Appended { seq, .. } => appended.push(seq),
        other => panic!("{other:?}"),
    }
    assert_eq!(appended, vec![1, 1]);
    assert_eq!(entries.len(), 1, "the retry is not broadcast again");
}

#[tokio::test]
async fn one_connection_follows_a_bounded_number_of_streams() {
    let (url, _) = start().await;
    let mut socket = raw(&url).await;
    for _ in 0..MAX_STREAMS_PER_CONNECTION {
        open_raw(&mut socket, &StreamSecret::random().keys()).await;
    }
    let one_more = StreamSecret::random().keys();
    send(&mut socket, &ClientFrame::open(&one_more, 0)).await;
    assert_eq!(receive(&mut socket).await, ServerFrame::stream_error(ErrorCode::TooManyStreams, one_more.id));
}

#[tokio::test]
async fn closing_a_connection_releases_its_streams() {
    let (url, service) = start().await;
    let mut socket = raw(&url).await;
    open_raw(&mut socket, &StreamSecret::random().keys()).await;
    assert_eq!(service.hub.live_streams(), 1);
    socket.close(None).await.unwrap();
    drop(socket);
    let deadline = tokio::time::Instant::now() + WAIT;
    while (service.hub.live_streams() > 0 || service.stats.connections() > 0) && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(service.hub.live_streams(), 0);
    assert_eq!(service.stats.connections(), 0);
}

#[tokio::test]
async fn health_answers_without_a_websocket() {
    let (url, _) = start().await;
    let address = url.trim_start_matches("ws://").trim_end_matches("/sync").to_string();
    let mut tcp = tokio::net::TcpStream::connect(&address).await.unwrap();
    tcp.write_all(b"GET /sync/healthz HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await.unwrap();
    let mut reply = String::new();
    tcp.read_to_string(&mut reply).await.unwrap();
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.contains(r#""ok":true"#));
}

#[tokio::test]
async fn a_rate_limited_append_is_retried_until_it_lands() {
    let mut config = Config::from_env(|_| None).unwrap();
    config.limits.appends = Rate { burst: 2.0, per_second: 4.0 };
    let (url, _) = start_with(config).await;
    let keys = StreamSecret::random().keys();
    let (client, _events) = SyncClient::spawn(url);
    client.follow(keys.clone(), 0);
    let started = tokio::time::Instant::now();
    let mut seqs = Vec::new();
    for i in 0..5u8 {
        seqs.push(client.append(keys.id, vec![i]).await.unwrap());
    }
    assert_eq!(seqs, vec![1, 2, 3, 4, 5]);
    assert!(started.elapsed() >= Duration::from_millis(500), "the limit held the later appends back");
}

#[tokio::test]
async fn a_nudge_reconnects_at_once_instead_of_waiting_out_the_back_off() {
    let parked = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = parked.local_addr().unwrap();
    drop(parked);
    let (client, mut events) = SyncClient::spawn(format!("ws://{address}/sync"));
    client.follow(StreamSecret::random().keys(), 0);
    tokio::time::sleep(Duration::from_secs(13)).await;

    let config = Config::from_env(|_| None).unwrap();
    let listener = tokio::net::TcpListener::bind(address).await.unwrap();
    tokio::spawn(serve(listener, Service::new(Store::memory(), &config)));
    client.nudge();
    let connected = async {
        while let Some(event) = events.recv().await {
            if event == SyncEvent::Connected {
                return;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(3), connected).await.expect("connected right after the nudge");
}

async fn post(address: &str, path: &str, body: &serde_json::Value) -> u16 {
    let body = body.to_string();
    let mut tcp = tokio::net::TcpStream::connect(address).await.unwrap();
    let request =
        format!("POST {path} HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    tcp.write_all(request.as_bytes()).await.unwrap();
    let mut reply = String::new();
    tcp.read_to_string(&mut reply).await.unwrap();
    reply[9..12].parse().unwrap()
}

#[tokio::test]
async fn a_ring_wakes_the_rung_persons_phones_once_per_gap() {
    let config = Config::from_env(|_| None).unwrap();
    let (pushed, mut pushes) = tokio::sync::mpsc::unbounded_channel();
    let service = Service::with_pusher(Store::memory(), &config, Pusher::Recording(pushed));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    tokio::spawn(serve(listener, service));

    let keys = StreamSecret::random().keys();
    let today = wake_day(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64);
    let bob = Bytes(keys.wake_tag("bob", today));
    let alice = Bytes(keys.wake_tag("alice", today));
    let register = |tag: &WakeTag, push: &str, until: i64| serde_json::to_value(WakeRegister { tag: tag.clone(), push: push.into(), until }).unwrap();

    assert_eq!(post(&address, "/sync/wake/register", &register(&bob, "fcm:bob-phone", today + 1)).await, 204);
    assert_eq!(post(&address, "/sync/wake/register", &register(&bob, "fcm:bob-tablet", today)).await, 204);
    assert_eq!(post(&address, "/sync/wake/register", &register(&alice, "fcm:alice-phone", today)).await, 204);
    assert_eq!(post(&address, "/sync/wake/register", &register(&bob, "apns:x", today)).await, 400);
    assert_eq!(post(&address, "/sync/wake/register", &register(&bob, "fcm:late", today + 3)).await, 400);

    let ring = serde_json::to_value(WakeRing { tags: vec![bob.clone()] }).unwrap();
    assert_eq!(post(&address, "/sync/wake/ring", &ring).await, 204);
    let mut woken = vec![tokio::time::timeout(WAIT, pushes.recv()).await.unwrap().unwrap(), tokio::time::timeout(WAIT, pushes.recv()).await.unwrap().unwrap()];
    woken.sort();
    assert_eq!(woken, ["fcm:bob-phone", "fcm:bob-tablet"], "only the rung person's devices wake");

    assert_eq!(post(&address, "/sync/wake/ring", &ring).await, 204);
    assert!(tokio::time::timeout(Duration::from_millis(500), pushes.recv()).await.is_err(), "a second ring within the gap waits for the gap to end");

    let forget = serde_json::to_value(WakeForget { tag: alice.clone(), push: "fcm:alice-phone".into() }).unwrap();
    assert_eq!(post(&address, "/sync/wake/forget", &forget).await, 204);
    let ring_alice = serde_json::to_value(WakeRing { tags: vec![alice.clone()] }).unwrap();
    assert_eq!(post(&address, "/sync/wake/ring", &ring_alice).await, 204);
    assert!(tokio::time::timeout(Duration::from_millis(500), pushes.recv()).await.is_err(), "a forgotten tag wakes nobody");

    let too_many = serde_json::to_value(WakeRing { tags: vec![bob; MAX_RING_TAGS + 1] }).unwrap();
    assert_eq!(post(&address, "/sync/wake/ring", &too_many).await, 400);
}

#[tokio::test]
async fn wake_ups_are_off_without_a_push_service() {
    let (url, _) = start().await;
    let address = url.trim_start_matches("ws://").trim_end_matches("/sync").to_string();
    let tag = Bytes([1; 32]);
    let register = serde_json::to_value(WakeRegister { tag: tag.clone(), push: "fcm:x".into(), until: 0 }).unwrap();
    assert_eq!(post(&address, "/sync/wake/register", &register).await, 404);
    assert_eq!(post(&address, "/sync/wake/ring", &serde_json::to_value(WakeRing { tags: vec![tag] }).unwrap()).await, 204);
}

async fn request(address: &str, method: &str, path: &str, body: &[u8]) -> (u16, Vec<u8>) {
    let mut tcp = tokio::net::TcpStream::connect(address).await.unwrap();
    let head = format!("{method} {path} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    tcp.write_all(head.as_bytes()).await.unwrap();
    tcp.write_all(body).await.unwrap();
    let mut reply = Vec::new();
    tcp.read_to_end(&mut reply).await.unwrap();
    let status = std::str::from_utf8(&reply[9..12]).unwrap().parse().unwrap();
    let split = reply.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    (status, reply[split..].to_vec())
}

#[tokio::test]
async fn a_sealed_picture_is_kept_for_whoever_holds_its_reference() {
    let (url, _) = start().await;
    let address = url.trim_start_matches("ws://").trim_end_matches("/sync").to_string();
    let key = commy_sync::PictureKey::for_blob("a-picture-hash");
    let path = format!("/sync/pictures/{}", key.id_hex());
    assert_eq!(request(&address, "GET", &path, b"").await.0, 404);
    let sealed = key.seal(b"pixels").unwrap();
    assert_eq!(request(&address, "PUT", &path, &sealed).await.0, 204);
    let (status, body) = request(&address, "GET", &path, b"").await;
    assert_eq!(status, 200);
    assert_eq!(key.open(&body).unwrap(), b"pixels");
    assert_eq!(request(&address, "PUT", &path, b"not sealed").await.0, 400, "only whole sealed pictures are taken");
    assert_eq!(request(&address, "GET", "/sync/pictures/nothex", b"").await.0, 400);
    let too_big = vec![0u8; commy_sync::picture::MAX_PICTURE_SEALED + 16];
    assert_eq!(request(&address, "PUT", &path, &too_big).await.0, 413);
}
