use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use commy_net::node::is_blob_of;
use commy_sync::picture::MAX_PICTURE_PLAIN;
use commy_sync::PictureKey;

use crate::sync_link;

const FETCH_WAIT: Duration = Duration::from_secs(15);
const STORE_WAIT: Duration = Duration::from_secs(60);
const MAX_OFFERED: usize = 4096;

static OFFERED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

pub fn pictures_url(sync_url: &str) -> Option<String> {
    let (scheme, rest) = match sync_url.strip_prefix("wss://") {
        Some(rest) => ("https", rest),
        None => ("http", sync_url.strip_prefix("ws://")?),
    };
    Some(format!("{scheme}://{}/pictures", rest.trim_end_matches('/')))
}

fn picture_url(blob: &str) -> Option<(PictureKey, String)> {
    let base = sync_link::url().as_deref().and_then(pictures_url)?;
    let key = PictureKey::for_blob(blob);
    let url = format!("{base}/{}", key.id_hex());
    Some((key, url))
}

pub fn opened(key: &PictureKey, blob: &str, body: &[u8], max: u64) -> Option<Vec<u8>> {
    let plain = key.open(body).ok()?;
    (plain.len() as u64 <= max && is_blob_of(blob, &plain)).then_some(plain)
}

pub async fn fetch(http: &reqwest::Client, blob: &str, max: u64) -> Option<Vec<u8>> {
    let (key, url) = picture_url(blob)?;
    let fetched = tokio::time::timeout(FETCH_WAIT, async {
        let response = http.get(url).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.bytes().await.ok()
    })
    .await
    .ok()
    .flatten()?;
    let plain = opened(&key, blob, &fetched, max);
    if plain.is_none() {
        tracing::debug!("the sync service's copy of a picture did not match its reference");
    }
    plain
}

fn first_offer(blob: &str) -> bool {
    let mut guard = OFFERED.lock().unwrap();
    let offered = guard.get_or_insert_with(HashSet::new);
    if offered.len() >= MAX_OFFERED {
        offered.clear();
    }
    offered.insert(blob.to_string())
}

fn withdraw_offer(blob: &str) {
    if let Some(offered) = OFFERED.lock().unwrap().as_mut() {
        offered.remove(blob);
    }
}

pub fn offer(http: &reqwest::Client, blob: &str, bytes: &[u8]) {
    if bytes.len() > MAX_PICTURE_PLAIN || !is_blob_of(blob, bytes) {
        return;
    }
    let Some((key, url)) = picture_url(blob) else { return };
    if !first_offer(blob) {
        return;
    }
    let Ok(sealed) = key.seal(bytes) else { return };
    let (http, blob) = (http.clone(), blob.to_string());
    tauri::async_runtime::spawn(async move {
        let sent = tokio::time::timeout(STORE_WAIT, http.put(url).body(sealed).send()).await;
        match sent {
            Ok(Ok(response)) if response.status().is_success() => {}
            Ok(Ok(response)) => {
                tracing::debug!("the sync service did not keep a picture: {}", response.status());
                withdraw_offer(&blob);
            }
            _ => {
                tracing::debug!("could not hand a picture to the sync service");
                withdraw_offer(&blob);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    use commy_net::node::blob_hash as blob_of;

    #[test]
    fn pictures_live_next_to_the_sync_service() {
        assert_eq!(pictures_url("wss://dev.commy.chat/sync").as_deref(), Some("https://dev.commy.chat/sync/pictures"));
        assert_eq!(pictures_url("ws://localhost:3002/sync/").as_deref(), Some("http://localhost:3002/sync/pictures"));
        assert_eq!(pictures_url("https://x/sync"), None);
    }

    #[test]
    fn a_fetched_picture_counts_only_when_it_is_the_one_referenced() {
        let real = b"the real picture".to_vec();
        let blob = blob_of(&real);
        let key = PictureKey::for_blob(&blob);
        assert_eq!(opened(&key, &blob, &key.seal(&real).unwrap(), 1024), Some(real.clone()));
        assert_eq!(opened(&key, &blob, &key.seal(b"something else").unwrap(), 1024), None, "a member cannot swap in another picture");
        assert_eq!(opened(&key, &blob, &key.seal(&real).unwrap(), 4), None, "size limits still hold");
        assert_eq!(opened(&key, &blob, b"garbage", 1024), None);
    }

    #[test]
    fn each_picture_is_offered_once_until_an_offer_fails() {
        let blob = blob_of(b"offered");
        assert!(first_offer(&blob));
        assert!(!first_offer(&blob));
        withdraw_offer(&blob);
        assert!(first_offer(&blob));
    }
}
