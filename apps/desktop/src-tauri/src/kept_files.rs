use commy_core::message::{Attachment, Message};
use commy_core::{ServerId, UserId};
use std::time::Duration;
use tauri::Manager;

use crate::App;

pub(crate) const STORED_FOR_OTHERS_BUDGET: u64 = 2 * 1024 * 1024 * 1024;

pub(crate) const ROLLING: bool = cfg!(mobile);

pub(crate) const ROLLING_CACHE_BYTES: u64 = 500 * 1024 * 1024;

const ROLL_EVERY: Duration = Duration::from_secs(60 * 60);

const SPREAD_SECS: std::ops::RangeInclusive<u64> = 2..=45;

static ONE_AT_A_TIME: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Wanted {
    pub blob: String,
    pub size: u64,
    pub provider: String,
}

pub(crate) fn wanted(message: &Message, me: UserId, up_to_bytes: u64) -> Vec<Wanted> {
    if message.author == me || up_to_bytes == 0 {
        return Vec::new();
    }
    let mut wanted: Vec<Wanted> = Vec::new();
    for file in message.files().filter(|file| fits(file, up_to_bytes)) {
        if wanted.iter().all(|w| w.blob != file.blob) {
            wanted.push(Wanted { blob: file.blob.clone(), size: file.size_bytes, provider: file.provider.clone().unwrap_or_default() });
        }
    }
    wanted
}

fn fits(file: &Attachment, up_to_bytes: u64) -> bool {
    file.size_bytes > 0 && file.size_bytes <= up_to_bytes && !file.blob.is_empty()
}

pub(crate) fn replica_sources(provider: &str, neighbors: Vec<[u8; 32]>, shuffle: impl FnOnce(&mut Vec<String>)) -> Vec<String> {
    let mut sources: Vec<String> = commy_net::node::blob_sources("", neighbors).into_iter().filter(|n| n != provider).collect();
    shuffle(&mut sources);
    if !provider.is_empty() {
        sources.push(provider.to_string());
    }
    sources
}

pub(crate) fn has_room(stored: u64, size: u64) -> bool {
    stored.saturating_add(size) <= STORED_FOR_OTHERS_BUDGET
}

pub(crate) fn store_new(handle: &tauri::AppHandle, server: ServerId, message: &Message) {
    let app = handle.state::<App>();
    let up_to = app.storage.lock().unwrap().store_for_others_bytes();
    if up_to == 0 || !cfg!(desktop) {
        return;
    }
    let Some(me) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { return };
    if app.safety.lock().unwrap().blocked.iter().any(|user| *user == message.author) {
        return;
    }
    let keys = crate::channel_lock::holder_of(&app);
    let files = {
        let servers = app.servers.lock().unwrap();
        let Some(state) = servers.get(&server) else { return };
        if !state.server.can_view(message.channel, me) || keys.hold(state, message).is_some() {
            return;
        }
        let Some(opened) = keys.open(state, message) else { return };
        wanted(&opened, me, up_to)
    };
    for file in files {
        let handle = handle.clone();
        tauri::async_runtime::spawn(async move { store(&handle, server, file, up_to).await });
    }
}

async fn store(handle: &tauri::AppHandle, server: ServerId, file: Wanted, up_to: u64) {
    tokio::time::sleep(spread()).await;
    let Ok(_turn) = ONE_AT_A_TIME.acquire().await else { return };
    let app = handle.state::<App>();
    let (fetcher, shelf, sources) = {
        let net = app.net.lock().await;
        let Some(node) = net.as_ref() else { return };
        let Some(shelf) = node.shelf() else { return };
        let sources = replica_sources(&file.provider, node.neighbors(server), |sources| {
            use rand::seq::SliceRandom;
            sources.shuffle(&mut rand::thread_rng());
        });
        (node.blob_fetcher(), shelf, sources)
    };
    if shelf.shared(&file.blob).await.is_some() {
        return;
    }
    match shelf.stored_for_others_bytes().await {
        Ok(stored) if has_room(stored, file.size) => {}
        Ok(_) => {
            tracing::info!("not keeping a new file for others: files kept for others already fill 2 GB");
            return;
        }
        Err(e) => {
            tracing::debug!("not keeping a new file for others: {e:#}");
            return;
        }
    }
    let members = crate::member_nodes(&app, server);
    match fetcher.store_among(&file.blob, sources.iter().map(String::as_str), &members, up_to, now_secs()).await {
        Ok(size) => tracing::info!("keeping a {size} byte file for members of a server for 30 days"),
        Err(e) => tracing::info!("could not keep a new file for others: {e:#}"),
    }
}

pub(crate) fn keep_opened(shelf: Option<commy_net::shelf::Shelf>, blob: &str, bytes: &[u8]) {
    let Some(shelf) = shelf else { return };
    let (blob, bytes) = (blob.to_string(), bytes.to_vec());
    tauri::async_runtime::spawn(async move {
        if shelf.shared(&blob).await.is_some() {
            return;
        }
        if let Err(e) = shelf.keep_opened_bytes(bytes, now_secs()).await {
            tracing::info!("an opened file is passed on only until Commy closes: {e:#}");
            return;
        }
        if ROLLING {
            trim(&shelf).await;
        }
    });
}

pub(crate) async fn roll(shelf: commy_net::shelf::Shelf) {
    loop {
        trim(&shelf).await;
        tokio::time::sleep(ROLL_EVERY).await;
    }
}

async fn trim(shelf: &commy_net::shelf::Shelf) {
    match shelf.trim_to(ROLLING_CACHE_BYTES).await {
        Ok(0) => {}
        Ok(dropped) => tracing::info!("let go of {dropped} older kept files to stay under 500 MB"),
        Err(e) => tracing::debug!("kept files were not trimmed: {e:#}"),
    }
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn spread() -> Duration {
    use rand::Rng;
    Duration::from_secs(rand::thread_rng().gen_range(SPREAD_SECS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::ChannelId;

    fn attachment(blob: &str, size: u64) -> Attachment {
        Attachment {
            blob: blob.into(),
            filename: format!("{blob}.bin"),
            size_bytes: size,
            mime_type: "application/octet-stream".into(),
            provider: Some("sender".into()),
        }
    }

    fn message_from(author: UserId, files: Vec<Attachment>) -> Message {
        let mut message = Message::new(ChannelId::new(), author, String::new());
        message.attachments = files;
        message
    }

    #[test]
    fn files_up_to_the_size_are_wanted_once_each() {
        let (me, friend) = (UserId::new(), UserId::new());
        let mb = 1024 * 1024;
        let message = message_from(friend, vec![attachment("a", 3 * mb), attachment("big", 26 * mb), attachment("a", 3 * mb), attachment("edge", 25 * mb), attachment("empty", 0)]);
        let wanted = wanted(&message, me, 25 * mb);
        let blobs: Vec<&str> = wanted.iter().map(|w| w.blob.as_str()).collect();
        assert_eq!(blobs, ["a", "edge"]);
        assert_eq!(wanted[0], Wanted { blob: "a".into(), size: 3 * mb, provider: "sender".into() });
    }

    #[test]
    fn nothing_is_wanted_from_my_own_messages_or_when_turned_off() {
        let me = UserId::new();
        let mine = message_from(me, vec![attachment("a", 10)]);
        assert!(wanted(&mine, me, u64::MAX).is_empty());
        let theirs = message_from(UserId::new(), vec![attachment("a", 10)]);
        assert!(wanted(&theirs, me, 0).is_empty());
    }

    #[test]
    fn link_cards_are_not_files_to_keep() {
        let mut message = message_from(UserId::new(), vec![attachment("a", 10)]);
        let mut card = Attachment::tts_marker();
        card.blob = "card".into();
        card.size_bytes = 10;
        message.attachments.push(card);
        assert_eq!(wanted(&message, UserId::new(), u64::MAX).len(), 1);
    }

    #[test]
    fn the_sender_is_asked_last_so_early_copies_take_the_load() {
        let sender = [7u8; 32];
        let neighbors = vec![[1u8; 32], sender, [2u8; 32], [1u8; 32]];
        let sender_hex = data_encoding::HEXLOWER.encode(&sender);
        let sources = replica_sources(&sender_hex, neighbors, |sources| sources.reverse());
        let expected = [data_encoding::HEXLOWER.encode(&[2u8; 32]), data_encoding::HEXLOWER.encode(&[1u8; 32]), sender_hex];
        assert_eq!(sources, expected);
        assert_eq!(replica_sources("", vec![[1u8; 32]], |_| {}), [data_encoding::HEXLOWER.encode(&[1u8; 32])]);
    }

    #[test]
    fn files_for_others_stop_at_two_gigabytes() {
        let gb = 1024 * 1024 * 1024;
        assert!(has_room(0, 25 * 1024 * 1024));
        assert!(has_room(gb, gb));
        assert!(!has_room(2 * gb, 1));
        assert!(!has_room(u64::MAX, 1));
    }
}
