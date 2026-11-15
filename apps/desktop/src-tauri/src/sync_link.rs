use crate::lock_or_recover::LockOrRecover;
use std::sync::Mutex;

use commy_sync::endpoint::accepted_url;
pub use commy_sync::Pool;
use commy_sync::{AppendError, StreamId, SyncClient};

static URL: Mutex<Option<String>> = Mutex::new(None);

pub fn set_url(server: &str, offered: Option<&str>) -> bool {
    let next = accepted_url(server, offered);
    let mut url = URL.lock_or_recover();
    let changed = *url != next;
    *url = next;
    changed
}

pub fn clear_url() {
    *URL.lock_or_recover() = None;
}

pub fn url() -> Option<String> {
    URL.lock_or_recover().clone()
}

pub async fn append(client: Option<SyncClient>, stream: StreamId, plain: Vec<u8>) -> Result<u64, AppendError> {
    client.ok_or(AppendError::NotFollowed)?.append(stream, plain).await
}
