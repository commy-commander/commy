use commy_bot::runner::run;
use commy_bot::storage::DataDir;
use commy_bot::Policy;
use commy_core::catchup::{digests, items, BucketItems};
use commy_core::test_support::{fixture, identity_for, join, post};
use commy_core::{
    AutoMod, BucketDigest, BucketKey, Identity, IdentityKey, Message, MessageId, Permissions, Role, RoleId, ServerEvent,
    ServerId, ServerState, SignedEvent, TombstoneProof, UserId, UserProfile,
};
use commy_net::catchup::{apply_items, check_peer, collect, profiles_of, ApplyReport, CatchupStore, PeerCheck};
use commy_net::protocol::{build_sync_response, sign_sync_response};
use commy_net::{CommyNode, NetEvent, WirePayload};
use iroh::RelayMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;

const WITHIN: Duration = Duration::from_secs(60);

struct OwnerStore {
    state: Mutex<ServerState>,
    owner: UserId,
}

impl CatchupStore for OwnerStore {
    fn local_identity(&self, _: ServerId) -> Option<(UserId, Identity)> {
        Some((self.owner, identity_for(self.owner)))
    }

    fn verify_peer(&self, _: ServerId, user: UserId, key: &IdentityKey) -> PeerCheck {
        check_peer(&self.state.lock().unwrap(), user, key)
    }

    fn digests(&self, _: ServerId) -> Vec<BucketDigest> {
        digests(&self.state.lock().unwrap(), chrono::Utc::now())
    }

    fn items(&self, _: ServerId, keys: &[BucketKey]) -> Vec<BucketItems> {
        items(&self.state.lock().unwrap(), keys, chrono::Utc::now())
    }

    fn get(&self, _: ServerId, ids: &[MessageId]) -> (Vec<Message>, Vec<TombstoneProof>) {
        collect(&self.state.lock().unwrap(), ids)
    }

    fn profiles(&self, _: ServerId, users: &[UserId]) -> Vec<UserProfile> {
        profiles_of(&self.state.lock().unwrap(), users)
    }

    fn apply_profiles(&self, _: ServerId, _: Vec<UserProfile>) {}

    fn apply(&self, _: ServerId, messages: Vec<Message>, tombstones: Vec<TombstoneProof>) -> ApplyReport {
        apply_items(&mut self.state.lock().unwrap(), messages, tombstones).0
    }
}

async fn until(
    state: &mut ServerState,
    node: &CommyNode,
    events: &mut UnboundedReceiver<NetEvent>,
    owner: UserId,
    mut done: impl FnMut(&ServerState, &ServerEvent) -> bool,
) {
    let sid = state.server.id;
    tokio::time::timeout(WITHIN, async {
        loop {
            match events.recv().await.expect("owner node stopped") {
                NetEvent::SyncRequested { .. } => {
                    let mut payload = build_sync_response(state, 50, owner);
                    sign_sync_response(&mut payload, chrono::Utc::now().timestamp_millis(), &identity_for(owner));
                    node.broadcast(sid, payload).await.unwrap();
                }
                NetEvent::Event { event, action, .. } => {
                    let _ = match action {
                        Some(action) => state.apply_signed(&SignedEvent { action, event: event.clone() }),
                        None => state.apply_unsigned(event.clone()),
                    };
                    if done(state, &event) {
                        return;
                    }
                }
                _ => {}
            }
        }
    })
    .await
    .expect("the bot did not answer in time");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_running_bot_joins_wears_its_badge_and_removes_held_messages() {
    let (mut state, channel, owner) = fixture();
    let member = join(&mut state, "member");
    let sid = state.server.id;
    let (mut node, mut events, _, _) = CommyNode::spawn_with(None, Some(RelayMap::empty())).await.unwrap();
    let ticket = node.host_server(sid, &state.server.name, Some((owner, identity_for(owner).public()))).await.unwrap();

    let dir = std::env::temp_dir().join(format!("commy-bot-live-{}", UserId::new()));
    let data = DataDir::new(&dir);
    let bot = data.create_profile("Tidy").unwrap().id;
    data.save_server(&ticket, None).unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let policy = Policy { log_channel: Some("general".into()), ban_after: None };
    let runner = tokio::spawn(async move {
        let result = run(&data, policy.into(), Some(RelayMap::empty()), async {
            let _ = stopped.await;
        })
        .await;
        (result, data)
    });

    let mut badges = 0;
    until(&mut state, &node, &mut events, owner, |state, event| {
        badges += matches!(event, ServerEvent::BotSet { user, bot: true, .. } if *user == bot) as usize;
        badges >= 2 && state.is_bot(bot)
    })
    .await;
    assert!(state.server.is_member(bot));

    let role = Role {
        id: RoleId::new(),
        name: "bots".into(),
        color: None,
        permissions: Permissions::MANAGE_MESSAGES | Permissions::SEND_MESSAGES,
        position: 1,
        hoist: false,
        self_assignable: false,
        mentionable: false,
        tag: None,
        banner: None,
    };
    let role_id = role.id;
    let rules = AutoMod { blocked_terms: vec!["gold".into()], ..AutoMod::default() };
    let at = chrono::Utc::now().timestamp_millis();
    let setup = [
        ServerEvent::RoleCreate { server: sid, role },
        ServerEvent::RoleAssign { server: sid, user: bot, role: role_id },
        ServerEvent::AutoModSet { server: sid, rules },
    ];
    for (i, event) in setup.into_iter().enumerate() {
        let signed = SignedEvent::sign(sid, owner, at + i as i64, event, &identity_for(owner));
        state.apply_signed(&signed).unwrap();
        node.broadcast(sid, WirePayload::Signed(signed)).await.unwrap();
    }
    let spam = post(&state, channel, member, "cheap gold");
    state.apply_unsigned(ServerEvent::MessageCreate { message: spam.clone() }).unwrap();
    node.broadcast(sid, WirePayload::Event(ServerEvent::MessageCreate { message: spam.clone() })).await.unwrap();

    until(&mut state, &node, &mut events, owner, |state, _| {
        !state.has_message(spam.id) && state.messages(channel).iter().any(|m| m.author == bot)
    })
    .await;
    let note = state.messages(channel).iter().find(|m| m.author == bot).unwrap();
    assert_eq!(note.content, "Removed a message from member: blocked word");

    let missed = post(&state, channel, member, "gold it never saw live");
    state.apply_unsigned(ServerEvent::MessageCreate { message: missed.clone() }).unwrap();
    let store = Arc::new(OwnerStore { state: Mutex::new(state.clone()), owner });
    let peer = *node.neighbors(sid).first().expect("the bot is a neighbor");
    let report = tokio::time::timeout(WITHIN, node.catch_up(sid, peer, store)).await.unwrap().unwrap();
    assert!(report.delivered.contains(&missed.id), "the bot learns it only through catch-up");
    until(&mut state, &node, &mut events, owner, |state, _| !state.has_message(missed.id)).await;

    stop.send(()).unwrap();
    let (result, data) = tokio::time::timeout(WITHIN, runner).await.unwrap().unwrap();
    result.unwrap();
    let saved = data.servers().unwrap();
    let kept = saved[0].1.as_ref().expect("the server is saved for the next start");
    assert!(kept.is_bot(bot) && !kept.has_message(spam.id) && !kept.has_message(missed.id));
    node.shutdown().await.ok();
    std::fs::remove_dir_all(dir).ok();
}
