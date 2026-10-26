use std::collections::HashSet;
use std::sync::Mutex;

use commy_core::{Identity, IdentityKey};
use commy_net::key_handoff::{self as handoff, Note};
use commy_net::sealed::SealKey;
use commy_sync::{StreamId, SyncClient, SyncEvent};
use tauri::{AppHandle, Manager};

use crate::entry_list;
use crate::state::App;

struct Live {
    url: String,
    account: String,
    task: tauri::async_runtime::JoinHandle<()>,
}

static LIVE: Mutex<Option<Live>> = Mutex::new(None);

pub fn own_node(app: &App) -> Option<Identity> {
    let dir = app.data_dir()?;
    crate::persist::load_or_create_node_key(dir).map(|secret| Identity::from_bytes(&secret))
}

pub fn ensure(handle: &AppHandle) {
    let app = handle.state::<App>();
    let (Some(url), Some(s)) = (crate::sync_link::url(), entry_list::session(&app)) else {
        stop();
        return;
    };
    let Some(me) = own_node(&app) else { return };
    let Some(dir) = app.data_dir() else { return };
    if entry_list::load_key(dir, &s.server, &s.account).is_none() {
        stop();
        return;
    }
    let mut live = LIVE.lock().unwrap();
    if live.as_ref().is_some_and(|l| l.url == url && l.account == s.account) {
        return;
    }
    if let Some(old) = live.take() {
        old.task.abort();
    }
    let task = tauri::async_runtime::spawn(watch(handle.clone(), url.clone(), s.account.clone(), me.public()));
    *live = Some(Live { url, account: s.account, task });
}

pub fn stop() {
    if let Some(old) = LIVE.lock().unwrap().take() {
        old.task.abort();
    }
}

async fn watch(handle: AppHandle, url: String, account: String, me: IdentityKey) {
    let keys = handoff::mailbox(&account);
    let stream = keys.id;
    let (client, mut events) = SyncClient::spawn(url);
    client.follow_tail(keys, 0, Some(handoff::TAIL));
    let mut caught_up = false;
    let mut waiting: Vec<IdentityKey> = Vec::new();
    let mut seen: HashSet<u64> = HashSet::new();
    while let Some(event) = events.recv().await {
        match event {
            SyncEvent::Entry { stream: from, seq, at, plain } if from == stream => {
                if !seen.insert(seq) {
                    continue;
                }
                match Note::parse(&plain) {
                    Note::Ask { node, .. } if node != me && handoff::is_fresh(at, chrono::Utc::now().timestamp_millis()) => {
                        waiting.retain(|asking| *asking != node);
                        waiting.push(node);
                    }
                    Note::Give { to, .. } => waiting.retain(|asking| *asking != to),
                    _ => {}
                }
            }
            SyncEvent::CaughtUp { stream: from, .. } if from == stream => caught_up = true,
            _ => {}
        }
        if !caught_up {
            continue;
        }
        for node in waiting.drain(..) {
            if let Some(key) = entry_list::key_for_own_device(&handle, &node).await {
                answer_with_key(&client, stream, &node, &key).await;
            }
        }
    }
}

async fn answer_with_key(client: &SyncClient, stream: StreamId, to: &IdentityKey, key: &SealKey) {
    let Some(note) = handoff::give(to, key) else { return };
    if let Err(e) = client.append(stream, note.encode()).await {
        tracing::warn!("could not hand the server list key over: {e}");
    }
}

pub async fn ask(url: &str, account: &str, me: &Identity, name: &str) -> Option<SealKey> {
    let keys = handoff::mailbox(account);
    let stream = keys.id;
    let (client, mut events) = SyncClient::spawn(url.to_string());
    client.follow_tail(keys, 0, Some(handoff::TAIL));
    let asking = handoff::ask(me, name).encode();
    let waited = tokio::time::timeout(commy_net::entry_key::ANSWER_TIMEOUT, async {
        if let Err(e) = client.append(stream, asking).await {
            tracing::warn!("could not post the device approval request: {e}");
            return None;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_handoff_identity_is_the_linked_device() {
        let secret = commy_net::CommyNode::generate_secret();
        let (node_id, _) = crate::account::sign_with_node_key(&secret, b"link");
        assert_eq!(Identity::from_bytes(&secret).public().to_hex(), node_id);
    }
}
