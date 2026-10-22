use std::collections::HashMap;
use std::sync::Mutex;

use commy_sync::wire::MAX_STREAMS_PER_CONNECTION;
use commy_sync::{AppendError, StreamId, StreamKeys, SyncClient, SyncEvent};
use tokio::sync::mpsc;

static URL: Mutex<Option<String>> = Mutex::new(None);

fn host_part(url: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next()?,
        None => authority.split(':').next()?,
    };
    Some((scheme, host))
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

pub fn accepted_url(server: &str, offered: Option<&str>) -> Option<String> {
    let offered = offered?.trim().trim_end_matches('/');
    let (scheme, host) = host_part(offered)?;
    let (_, server_host) = host_part(server)?;
    let scheme_ok = scheme == "wss" || (scheme == "ws" && is_loopback(host));
    (scheme_ok && host.eq_ignore_ascii_case(server_host)).then(|| offered.to_string())
}

pub fn set_url(server: &str, offered: Option<&str>) -> bool {
    let next = accepted_url(server, offered);
    let mut url = URL.lock().unwrap();
    let changed = *url != next;
    *url = next;
    changed
}

pub fn clear_url() {
    *URL.lock().unwrap() = None;
}

pub fn url() -> Option<String> {
    URL.lock().unwrap().clone()
}

struct Slot {
    client: SyncClient,
    streams: usize,
}

pub struct Pool {
    url: String,
    slots: Vec<Slot>,
    home: HashMap<StreamId, usize>,
    events: mpsc::UnboundedSender<SyncEvent>,
}

impl Pool {
    pub fn new(url: &str) -> (Pool, mpsc::UnboundedReceiver<SyncEvent>) {
        let (events, received) = mpsc::unbounded_channel();
        (Pool { url: url.to_string(), slots: Vec::new(), home: HashMap::new(), events }, received)
    }

    pub fn follows(&self, stream: StreamId) -> bool {
        self.home.contains_key(&stream)
    }

    fn slot_with_room(&mut self, needed: usize) -> usize {
        if let Some(at) = self.slots.iter().position(|s| s.streams + needed <= MAX_STREAMS_PER_CONNECTION) {
            return at;
        }
        let (client, mut received) = SyncClient::spawn(self.url.clone());
        let events = self.events.clone();
        tokio::spawn(async move {
            while let Some(event) = received.recv().await {
                if events.send(event).is_err() {
                    break;
                }
            }
        });
        self.slots.push(Slot { client, streams: 0 });
        self.slots.len() - 1
    }

    pub fn follow_together(&mut self, group: Vec<(StreamKeys, u64)>) {
        let fresh: Vec<(StreamKeys, u64)> = group.into_iter().filter(|(keys, _)| !self.home.contains_key(&keys.id)).collect();
        if fresh.is_empty() {
            return;
        }
        let at = self.slot_with_room(fresh.len().min(MAX_STREAMS_PER_CONNECTION));
        for (keys, after) in fresh {
            let at = if self.slots[at].streams < MAX_STREAMS_PER_CONNECTION { at } else { self.slot_with_room(1) };
            self.home.insert(keys.id, at);
            self.slots[at].streams += 1;
            self.slots[at].client.follow(keys, after);
        }
    }

    pub fn follow(&mut self, keys: StreamKeys, after: u64) {
        self.follow_together(vec![(keys, after)]);
    }

    pub fn unfollow(&mut self, stream: StreamId) {
        if let Some(at) = self.home.remove(&stream) {
            self.slots[at].streams -= 1;
            self.slots[at].client.unfollow(stream);
        }
    }

    pub fn nudge(&self) {
        for slot in &self.slots {
            slot.client.nudge();
        }
    }

    pub fn client_for(&self, stream: StreamId) -> Option<SyncClient> {
        self.home.get(&stream).map(|at| self.slots[*at].client.clone())
    }
}

pub async fn append(client: Option<SyncClient>, stream: StreamId, plain: Vec<u8>) -> Result<u64, AppendError> {
    client.ok_or(AppendError::NotFollowed)?.append(stream, plain).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_secure_addresses_on_the_account_servers_host_are_used() {
        let ok = |server: &str, offered: &str| accepted_url(server, Some(offered));
        assert_eq!(ok("https://commy.chat", "wss://commy.chat/sync").as_deref(), Some("wss://commy.chat/sync"));
        assert_eq!(ok("https://commy.chat", "wss://commy.chat/sync/").as_deref(), Some("wss://commy.chat/sync"));
        assert_eq!(ok("https://Commy.Chat", "wss://commy.chat:8443/sync").as_deref(), Some("wss://commy.chat:8443/sync"));
        assert_eq!(ok("https://commy.chat", "ws://commy.chat/sync"), None, "no plain sockets off this machine");
        assert_eq!(ok("https://commy.chat", "wss://elsewhere.example/sync"), None);
        assert_eq!(ok("https://commy.chat", "wss://commy.chat.evil.example/sync"), None);
        assert_eq!(ok("https://commy.chat", "wss://user@commy.chat/sync"), None);
        assert_eq!(ok("https://commy.chat", "https://commy.chat/sync"), None);
        assert_eq!(ok("http://localhost:3001", "ws://localhost:3002/sync").as_deref(), Some("ws://localhost:3002/sync"));
        assert_eq!(ok("http://[::1]:3001", "ws://[::1]:3002/sync").as_deref(), Some("ws://[::1]:3002/sync"));
        assert_eq!(accepted_url("https://commy.chat", None), None);
        assert_eq!(ok("https://commy.chat", " "), None);
    }
}
