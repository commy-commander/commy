//! Settling our key. A key made on a signed-in device is held back while
//! the account may still replace it (see `App::hold_key`). Once it is
//! final, what we signed meanwhile is signed again with it before
//! anything may go out, and only then is it announced and every server
//! catches up with its neighbors.

use crate::catchup::{every_neighbor, AppAccess};
use crate::outbox;
use crate::state::App;
use commy_core::{ServerId, UserProfile};
use std::future::Future;
use tauri::Manager;

/// The network side of putting a settled key to use.
pub trait KeyNet: AppAccess {
    /// Announces our profile in every server we are in.
    fn announce(&self, profile: &UserProfile) -> impl Future<Output = ()> + Send;
    /// Each server's current gossip neighbors; none while offline.
    fn neighbors(&self) -> impl Future<Output = Vec<(ServerId, Vec<[u8; 32]>)>> + Send;
    /// Sends what waited for a server's neighbors.
    fn flush(&self, server: ServerId) -> impl Future<Output = ()> + Send;
    /// Starts a catch-up session with one neighbor.
    fn start_session(&self, server: ServerId, peer: [u8; 32]);
}

impl KeyNet for tauri::AppHandle {
    async fn announce(&self, profile: &UserProfile) {
        crate::announce_profile(&self.state::<App>(), self, profile).await;
    }

    async fn neighbors(&self) -> Vec<(ServerId, Vec<[u8; 32]>)> {
        let app = self.state::<App>();
        let servers: Vec<ServerId> = app.servers.lock().unwrap().keys().copied().collect();
        let net = app.net.lock().await;
        let Some(node) = net.as_ref() else { return Vec::new() };
        servers.into_iter().map(|sid| (sid, node.neighbors(sid))).collect()
    }

    async fn flush(&self, server: ServerId) {
        outbox::flush_queued(&self.state::<App>(), server).await;
    }

    fn start_session(&self, server: ServerId, peer: [u8; 32]) {
        crate::catchup::spawn_session(self.clone(), server, peer, None, true);
    }
}

/// Our sole hold on [`App::signing`], taken to swap our key: no send
/// signs with it meanwhile, and every one that did is applied or queued.
pub type KeySwap<'a> = tokio::sync::RwLockWriteGuard<'a, ()>;

/// Our key is final: if it was held back, it is settled (see
/// [`settle_loaded_key`]) and put to use (see [`use_settled_key`]).
pub async fn settle_key<N: KeyNet>(net: &N) {
    if !net.app().settle_pending() {
        return;
    }
    let settled = {
        let swap = net.app().signing.write().await;
        settle_loaded_key(net.app(), &swap)
    };
    match settled {
        Ok(true) => use_settled_key(net).await,
        // Another task settled it meanwhile and puts it to use.
        Ok(false) => {}
        Err(e) => tracing::warn!("could not load our identity key, so it stays held back: {e}"),
    }
}

/// Makes the key in the key file final without letting anything signed
/// with the key it replaces go out. While sends are still held, it loads
/// the key and signs again with it what we signed meanwhile (see
/// [`outbox::resign_held`]); only then does the key settle. True when it
/// was held back until now. A key that cannot be loaded settles nothing:
/// it stays held back for a later try.
pub fn settle_loaded_key(app: &App, _swap: &KeySwap<'_>) -> std::io::Result<bool> {
    settle_after_resigning(app, App::settle_key)
}

/// [`settle_loaded_key`] with its last step passed in.
fn settle_after_resigning(app: &App, settle: impl FnOnce(&App) -> bool) -> std::io::Result<bool> {
    app.reload_identity()?;
    outbox::resign_held(app);
    Ok(settle(app))
}

/// Our key just settled, with what we signed while it was held signed
/// with it: announces it, retries waiting DMs, and catches up every
/// server with each of its current neighbors after sending what waited
/// for them, since all of it was held back until now.
pub async fn use_settled_key<N: KeyNet>(net: &N) {
    let app = net.app();
    let profile = app.profile.lock().unwrap().clone();
    if let Some(profile) = profile {
        net.announce(&profile).await;
        net.emit("profile-changed", serde_json::to_value(&profile).unwrap_or_default());
    }
    outbox::wake_all_dms(app);
    let neighbors = net.neighbors().await;
    for (server, peers) in &neighbors {
        if !peers.is_empty() {
            net.flush(*server).await;
        }
    }
    for (server, peer) in every_neighbor(neighbors) {
        net.start_session(server, peer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catchup::TestHandle;
    use commy_core::{ChannelId, Identity, IdentityKey, Message, Server, ServerEvent, ServerState, TombstoneProof};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos();
        let dir = std::env::temp_dir().join("commy-settle-tests").join(format!("{tag}-{nanos:x}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Puts `identity` in the key file, as the server list sync does.
    fn write_key(dir: &Path, identity: &Identity) {
        let hex = data_encoding::HEXLOWER.encode(&identity.to_bytes());
        std::fs::write(dir.join(commy_core::identity::IDENTITY_FILE), hex).unwrap();
    }

    /// A device holding back the key it signs with, in a server it owns
    /// with one other member, before the account's key arrives.
    struct Device {
        app: Arc<App>,
        dir: PathBuf,
        sid: ServerId,
        channel: ChannelId,
        held: Identity,
        account: Identity,
    }

    impl Device {
        fn new(tag: &str) -> Self {
            let dir = temp_dir(tag);
            let held = Identity::generate();
            write_key(&dir, &held);
            let app = Arc::new(App::default());
            app.data_dir.set(dir.clone()).unwrap();
            let me = UserProfile::new("me");
            *app.profile.lock().unwrap() = Some(me.clone());
            let server = Server::new("s", me.id);
            let (sid, channel) = (server.id, server.channels[0].id);
            let mut state = ServerState::new(server);
            state.profiles.insert(me.id, me);
            let other = Identity::generate();
            let mut member = UserProfile::new("member");
            member.identity = Some(other.public());
            state.apply(ServerEvent::MemberJoin { server: sid, profile: member }).unwrap();
            app.servers.lock().unwrap().insert(sid, state);
            app.reload_identity().unwrap();
            app.mark_key_unsettled();
            app.hold_key(Instant::now() + Duration::from_secs(120));
            Self { app, dir, sid, channel, held, account: Identity::generate() }
        }

        fn me(&self) -> UserProfile {
            self.app.profile.lock().unwrap().clone().unwrap()
        }

        /// One of our messages, signed with the key we hold.
        fn post(&self, content: &str) -> Message {
            let mut message = Message::new(self.channel, self.me().id, content);
            message.sign(self.sid, &self.held);
            message
        }

        /// Applies one of our events here and sends it as the app does,
        /// with a neighbor connected; true when it went out.
        fn send(&self, event: ServerEvent) -> bool {
            self.app.servers.lock().unwrap().get_mut(&self.sid).unwrap().apply(event.clone()).unwrap();
            let delivered = outbox::may_send(&self.app, 1, &event);
            self.app.outbox.lock().unwrap().note_local(self.sid, &event, delivered);
            delivered
        }

        /// The account's key reaches the key file.
        fn account_key_arrives(&self) {
            write_key(&self.dir, &self.account);
        }
    }

    impl Drop for Device {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    /// How much of ours is signed with another key than `key`: messages
    /// and delete proofs kept here, and queued edits and deletes.
    fn stale(app: &App, key: &IdentityKey) -> usize {
        let me = app.profile.lock().unwrap().as_ref().unwrap().id;
        let servers = app.servers.lock().unwrap();
        let mut outbox = app.outbox.lock().unwrap().clone();
        let mut count = 0;
        for state in servers.values() {
            let sid = state.server.id;
            let snapshot = state.snapshot();
            let messages = snapshot.messages.iter().flat_map(|(_, list)| list.iter());
            count += messages.filter(|m| m.author == me && !m.verify(sid, key)).count();
            count += snapshot.tombstone_proofs.iter().filter(|p| p.by == me && !p.verify(sid, key)).count();
            for event in outbox.take_queued(sid) {
                let bad = match event {
                    ServerEvent::MessageEdit { message, content, edited_at, sig, .. } => {
                        state.message(message).is_some_and(|m| !m.verify_edit(sid, &content, edited_at, &sig, key))
                    }
                    ServerEvent::MessageDelete { channel, message, by, at, sig } => {
                        by == me && !TombstoneProof { channel, message, by, at, sig }.verify(sid, key)
                    }
                    _ => false,
                };
                count += usize::from(bad);
            }
        }
        count
    }

    /// Our work while the key was held: a message sent before, edited
    /// meanwhile; one still pending; one deleted, its proof kept.
    fn work_while_held(device: &Device) {
        let sent = device.post("sent before");
        device.app.hold_key(Instant::now());
        assert!(device.send(ServerEvent::MessageCreate { message: sent.clone() }), "the key had settled then");
        device.app.hold_key(Instant::now() + Duration::from_secs(120));
        assert!(!device.send(sent.edit_event(device.sid, "edited meanwhile", chrono::Utc::now(), &device.held)));
        assert!(!device.send(ServerEvent::MessageCreate { message: device.post("pending") }));
        let gone = device.post("deleted meanwhile");
        device.app.servers.lock().unwrap().get_mut(&device.sid).unwrap().apply(ServerEvent::MessageCreate { message: gone.clone() }).unwrap();
        let me = device.me().id;
        let proof = TombstoneProof::sign(device.sid, device.channel, gone.id, me, chrono::Utc::now(), &device.held);
        assert!(!device.send(proof.into_event()));
    }

    #[derive(Debug, Clone, PartialEq)]
    enum Call {
        Announce(Option<IdentityKey>),
        Neighbors,
        Flush(ServerId),
        Session(ServerId, [u8; 32]),
    }

    /// The network as settling uses it, recording each call. Every call
    /// first checks that the key settled and nothing of ours is signed
    /// with another key, so nothing stale could go out through it.
    struct FakeNet {
        handle: TestHandle,
        neighbors: Vec<(ServerId, Vec<[u8; 32]>)>,
        settled: IdentityKey,
        calls: Mutex<Vec<Call>>,
    }

    impl FakeNet {
        fn record(&self, call: Call) {
            assert!(self.app().key_settled(), "{call:?} before the key settled");
            assert_eq!(stale(self.app(), &self.settled), 0, "{call:?} with work signed by another key");
            self.calls.lock().unwrap().push(call);
        }

        fn take_calls(&self) -> Vec<Call> {
            std::mem::take(&mut *self.calls.lock().unwrap())
        }
    }

    impl AppAccess for FakeNet {
        fn app(&self) -> &App {
            self.handle.app()
        }

        fn emit(&self, event: &str, payload: serde_json::Value) {
            self.handle.emit(event, payload);
        }

        fn save_later(&self) {
            self.handle.save_later();
        }
    }

    impl KeyNet for FakeNet {
        async fn announce(&self, profile: &UserProfile) {
            self.record(Call::Announce(self.app().announced(profile.clone()).identity));
        }

        async fn neighbors(&self) -> Vec<(ServerId, Vec<[u8; 32]>)> {
            self.record(Call::Neighbors);
            self.neighbors.clone()
        }

        async fn flush(&self, server: ServerId) {
            self.record(Call::Flush(server));
        }

        fn start_session(&self, server: ServerId, peer: [u8; 32]) {
            self.record(Call::Session(server, peer));
        }
    }

    #[tokio::test]
    async fn settling_signs_again_then_catches_up_with_current_neighbors() {
        let device = Device::new("settle");
        work_while_held(&device);
        device.account_key_arrives();
        let key = device.account.public();
        assert!(stale(&device.app, &key) > 0);
        let (one, two) = ([1u8; 32], [2u8; 32]);
        let net = FakeNet {
            handle: TestHandle::new(device.app.clone()),
            neighbors: vec![(device.sid, vec![one, two]), (ServerId::new(), vec![])],
            settled: key,
            calls: Mutex::new(Vec::new()),
        };

        settle_key(&net).await;
        assert_eq!(
            net.take_calls(),
            vec![
                Call::Announce(Some(key)),
                Call::Neighbors,
                Call::Flush(device.sid),
                Call::Session(device.sid, one),
                Call::Session(device.sid, two),
            ]
        );
        assert!(device.app.key_settled() && !device.app.key_marked_unsettled());
        assert_eq!(device.app.identity().unwrap().public(), key);
        assert_eq!(device.me().identity, Some(key));
        let names: Vec<String> = net.handle.take_events().into_iter().map(|(name, _)| name).collect();
        assert!(names.contains(&"profile-changed".to_string()));

        // Settled once: later syncs leave it alone.
        settle_key(&net).await;
        assert!(net.take_calls().is_empty());
    }

    #[tokio::test]
    async fn a_key_never_held_is_not_settled_again() {
        let device = Device::new("never-held");
        device.app.settle_key();
        let net = FakeNet {
            handle: TestHandle::new(device.app.clone()),
            neighbors: vec![(device.sid, vec![[1u8; 32]])],
            settled: device.held.public(),
            calls: Mutex::new(Vec::new()),
        };
        settle_key(&net).await;
        assert!(net.take_calls().is_empty());
        assert!(net.handle.take_events().is_empty());
    }

    #[test]
    fn everything_is_signed_again_before_the_key_settles() {
        let device = Device::new("order");
        work_while_held(&device);
        device.account_key_arrives();
        let key = device.account.public();
        let edit = ServerEvent::MessageEdit {
            channel: device.channel,
            message: commy_core::MessageId::new(),
            content: "x".into(),
            edited_at: chrono::Utc::now(),
            sig: commy_core::Signature([0; 64]),
        };
        let settled = settle_after_resigning(&device.app, |app| {
            // The moment it settles: all of ours already carries the new
            // key, and until now nothing signed could go out.
            assert_eq!(app.identity().unwrap().public(), key);
            assert_eq!(stale(app, &key), 0);
            assert!(!app.key_settled() && !outbox::may_send(app, 1, &edit));
            app.settle_key()
        });
        assert!(settled.unwrap());
        assert!(outbox::may_send(&device.app, 1, &edit));
    }

    fn fake_net(device: &Device) -> FakeNet {
        FakeNet {
            handle: TestHandle::new(device.app.clone()),
            neighbors: vec![(device.sid, vec![[1u8; 32]])],
            settled: device.account.public(),
            calls: Mutex::new(Vec::new()),
        }
    }

    #[tokio::test]
    async fn settling_waits_for_a_send_signing_with_the_held_key() {
        let device = Device::new("in-flight");
        device.account_key_arrives();
        let key = device.account.public();
        let net = Arc::new(fake_net(&device));

        // A send signed with the held key and not applied yet.
        let sending = device.app.signing.read().await;
        let message = device.post("in flight");
        let settling = tokio::spawn({
            let net = net.clone();
            async move { settle_key(&*net).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!device.app.key_settled(), "the key is not swapped under a send");
        assert_eq!(device.app.identity().unwrap().public(), device.held.public());
        assert!(!device.send(ServerEvent::MessageCreate { message: message.clone() }), "still held back");
        drop(sending);

        settling.await.unwrap();
        assert!(device.app.key_settled());
        assert_eq!(stale(&device.app, &key), 0);
        let state = device.app.servers.lock().unwrap()[&device.sid].clone();
        assert!(state.message(message.id).unwrap().verify(device.sid, &key));
        assert!(net.take_calls().contains(&Call::Announce(Some(key))));
    }

    #[tokio::test]
    async fn a_flush_leaves_what_waits_for_the_key_where_settling_signs_it_again() {
        let device = Device::new("flush");
        let sid = device.sid;
        work_while_held(&device);
        let rename = ServerEvent::ServerRename { server: sid, name: "renamed".into() };
        device.app.outbox.lock().unwrap().note_local(sid, &rename, false);
        device.account_key_arrives();
        let key = device.account.public();
        let queued = |app: &App| app.outbox.lock().unwrap().clone().take_queued(sid);

        // A flush waiting for the network took only what may go now.
        let network = device.app.net.lock().await;
        let flushing = tokio::spawn({
            let app = device.app.clone();
            async move { outbox::flush_queued(&app, sid).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        let waiting = queued(&device.app);
        assert_eq!(waiting.len(), 2, "the edit and the delete stay queued");
        assert!(!waiting.contains(&rename));

        // The key settles once the flush is done.
        let settling = tokio::spawn({
            let app = device.app.clone();
            async move {
                let swap = app.signing.write().await;
                settle_loaded_key(&app, &swap)
            }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!device.app.key_settled());
        drop(network);
        flushing.await.unwrap();
        assert!(settling.await.unwrap().unwrap());

        // Offline, so nothing went out; all of it carries the settled key.
        let waiting = queued(&device.app);
        assert_eq!(waiting.len(), 3);
        assert!(waiting.contains(&rename));
        assert_eq!(stale(&device.app, &key), 0);
    }

    #[test]
    fn a_key_that_cannot_be_loaded_stays_held_back() {
        let device = Device::new("unreadable");
        work_while_held(&device);
        let path = device.dir.join(commy_core::identity::IDENTITY_FILE);
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        let app = &device.app;
        assert!(settle_loaded_key(app, &app.signing.try_write().unwrap()).is_err());
        assert!(app.settle_pending() && !app.key_settled() && app.key_marked_unsettled());
        assert_eq!(app.identity().unwrap().public(), device.held.public());
        let queued = app.outbox.lock().unwrap().clone().take_queued(device.sid);
        assert!(queued.iter().all(|e| !outbox::may_send(app, 1, e)), "what we signed still waits");

        // Readable again: it settles, with our work signed again.
        std::fs::remove_dir(&path).unwrap();
        device.account_key_arrives();
        assert!(settle_loaded_key(app, &app.signing.try_write().unwrap()).unwrap());
        assert!(app.key_settled() && !app.key_marked_unsettled());
        assert_eq!(stale(app, &device.account.public()), 0);
    }

    #[test]
    fn an_edit_made_while_held_to_a_delivered_message_is_relayed_with_the_settled_key() {
        let device = Device::new("edit");
        let sid = device.sid;
        let sent = device.post("delivered");
        device.app.hold_key(Instant::now());
        assert!(device.send(ServerEvent::MessageCreate { message: sent.clone() }));
        // A member holds it, and knows us by the key that settles.
        let mut peer = device.app.servers.lock().unwrap()[&sid].clone();
        let me = device.me().id;
        peer.profiles.get_mut(&me).unwrap().identity = Some(device.account.public());

        device.app.hold_key(Instant::now() + Duration::from_secs(120));
        let edit = sent.edit_event(sid, "edited while held", chrono::Utc::now(), &device.held);
        assert!(!device.send(edit));
        assert!(!device.app.outbox.lock().unwrap().has_pending(sid), "not pending: it was delivered");
        device.account_key_arrives();
        assert!(settle_loaded_key(&device.app, &device.app.signing.try_write().unwrap()).unwrap());

        // Catch-up relays it as kept here...
        let state = device.app.servers.lock().unwrap()[&sid].clone();
        let (relayed, _) = commy_net::catchup::collect(&state, &[sent.id]);
        assert_eq!(relayed.len(), 1);
        assert!(relayed[0].verify(sid, &device.account.public()));
        let mut caught_up = peer.clone();
        assert_eq!(caught_up.apply(ServerEvent::MessageCreate { message: relayed[0].clone() }), Ok(true));
        assert_eq!(caught_up.message(sent.id).unwrap().content, "edited while held");
        // ...and the queued edit goes out signed with the settled key too.
        let queued = device.app.outbox.lock().unwrap().take_queued(sid);
        assert_eq!(queued.len(), 1);
        assert_eq!(peer.apply(queued[0].clone()), Ok(true));
        assert_eq!(peer.message(sent.id).unwrap().content, "edited while held");
    }
}
