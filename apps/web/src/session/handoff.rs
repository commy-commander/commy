use std::time::Duration;

use commy_core::Identity;
use commy_net::key_handoff::{self as handoff, Note};
use commy_net::sealed::SealKey;
use commy_sync::{SyncClient, SyncEvent};

const WAIT: Duration = Duration::from_millis(handoff::FRESH_MS as u64);

pub(super) async fn ask(url: &str, account: &str, me: &Identity, name: &str) -> Option<SealKey> {
    let keys = handoff::mailbox(account);
    let stream = keys.id;
    let (client, mut events) = SyncClient::spawn(url.to_string());
    client.follow_tail(keys, 0, Some(handoff::TAIL));
    let asking = handoff::ask(me, name).encode();
    let waited = n0_future::time::timeout(WAIT, async {
        client.append(stream, asking).await.ok()?;
        while let Some(event) = events.recv().await {
            if let SyncEvent::Entry { stream: from, plain, .. } = event {
                if from == stream {
                    if let Some(key) = handoff::open_give(&Note::parse(&plain), me) {
                        return Some(key);
                    }
                }
            }
        }
        None
    })
    .await;
    waited.ok().flatten()
}

pub(super) async fn tell_devices_we_have_the_key(url: &str, account: &str, me: &Identity, key: &SealKey) {
    let Some(note) = handoff::give(&me.public(), key) else { return };
    let keys = handoff::mailbox(account);
    let stream = keys.id;
    let (client, _events) = SyncClient::spawn(url.to_string());
    client.follow_tail(keys, 0, Some(handoff::TAIL));
    let _ = n0_future::time::timeout(WAIT, client.append(stream, note.encode())).await;
}
