use std::collections::{BTreeMap, HashMap, HashSet};

use commy_core::{IdentityKey, UserId};
use commy_sync::{StreamId, StreamKeys};
use commy_view::away::{cursor_key, HomeBook};

pub type Wanted = HashMap<StreamId, (UserId, IdentityKey, String, StreamKeys)>;

#[derive(Debug, Default)]
pub struct AwayPlan {
    pub unfollow: Vec<StreamId>,
    pub follow: BTreeMap<String, Vec<(StreamKeys, u64)>>,
}

#[derive(Default)]
pub struct AwayBook {
    pub homes: HomeBook,
    streams: HashMap<StreamId, (UserId, IdentityKey, String)>,
    after: HashMap<String, u64>,
    live: HashSet<StreamId>,
    waiting: Vec<String>,
}

impl AwayBook {
    pub fn plan(&mut self, wanted: Wanted) -> AwayPlan {
        let mut plan = AwayPlan::default();
        let stale: Vec<StreamId> = self
            .streams
            .iter()
            .filter(|(stream, (user, _, url))| wanted.get(stream).is_none_or(|(want, _, want_url, _)| want != user || want_url != url))
            .map(|(stream, _)| *stream)
            .collect();
        for stream in stale {
            self.streams.remove(&stream);
            self.live.remove(&stream);
            plan.unfollow.push(stream);
        }
        for (stream, (user, key, url, keys)) in wanted {
            if self.streams.contains_key(&stream) {
                continue;
            }
            let after = self.after.get(&cursor_key(&stream.to_hex(), &url)).copied().unwrap_or(0);
            self.streams.insert(stream, (user, key, url.clone()));
            plan.follow.entry(url).or_default().push((keys, after));
        }
        plan
    }

    pub fn advance(&mut self, stream: StreamId, seq: u64) -> Option<(UserId, IdentityKey, bool)> {
        let (user, key, url) = self.streams.get(&stream)?.clone();
        self.after.insert(cursor_key(&stream.to_hex(), &url), seq);
        Some((user, key, self.live.contains(&stream)))
    }

    pub fn caught_up(&mut self, stream: StreamId, seq: u64) {
        if self.advance(stream, seq).is_some() {
            self.live.insert(stream);
        }
    }

    pub fn forget_streams(&mut self) {
        self.streams.clear();
        self.live.clear();
    }

    pub fn wait_for(&mut self, homes: Vec<String>) {
        self.waiting.extend(homes);
        self.waiting.sort();
        self.waiting.dedup();
    }

    pub fn take_waiting(&mut self) -> Vec<String> {
        std::mem::take(&mut self.waiting)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::Identity;
    use commy_sync::StreamSecret;

    const FRIENDS: &str = "wss://friends.org/sync";

    fn wanted(stream: &StreamKeys, user: UserId, key: IdentityKey, url: &str) -> Wanted {
        HashMap::from([(stream.id, (user, key, url.to_string(), stream.clone()))])
    }

    #[test]
    fn a_partners_streams_are_read_on_their_home_and_dropped_when_they_move() {
        let mut book = AwayBook::default();
        let keys = StreamSecret::random().keys();
        let (partner, key) = (UserId::new(), Identity::generate().public());
        let plan = book.plan(wanted(&keys, partner, key, FRIENDS));
        assert!(plan.unfollow.is_empty());
        assert_eq!(plan.follow[FRIENDS].len(), 1);
        assert!(book.plan(wanted(&keys, partner, key, FRIENDS)).follow.is_empty(), "already followed");
        book.caught_up(keys.id, 9);
        let moved = book.plan(wanted(&keys, partner, key, "wss://new.org/sync"));
        assert_eq!(moved.unfollow, vec![keys.id]);
        assert_eq!(moved.follow["wss://new.org/sync"][0].1, 0, "the new home has its own numbering");
        book.plan(wanted(&keys, partner, key, FRIENDS));
        let back = book.plan(wanted(&keys, partner, key, FRIENDS));
        assert!(back.follow.is_empty());
        let gone = book.plan(HashMap::new());
        assert_eq!(gone.unfollow, vec![keys.id]);
        let again = book.plan(wanted(&keys, partner, key, FRIENDS));
        assert_eq!(again.follow[FRIENDS][0].1, 9, "returning to a home resumes where we left off");
    }

    #[test]
    fn only_followed_streams_move_their_cursor_and_catching_up_makes_them_live() {
        let mut book = AwayBook::default();
        let keys = StreamSecret::random().keys();
        let (partner, key) = (UserId::new(), Identity::generate().public());
        assert_eq!(book.advance(keys.id, 4), None);
        book.plan(wanted(&keys, partner, key, FRIENDS));
        assert_eq!(book.advance(keys.id, 5), Some((partner, key, false)));
        book.caught_up(keys.id, 6);
        assert_eq!(book.advance(keys.id, 7), Some((partner, key, true)));
    }

    #[test]
    fn homes_still_unknown_wait_once_each() {
        let mut book = AwayBook::default();
        book.wait_for(vec!["https://b.org".into(), "https://a.org".into()]);
        book.wait_for(vec!["https://a.org".into()]);
        assert_eq!(book.take_waiting(), vec!["https://a.org".to_string(), "https://b.org".to_string()]);
        assert!(book.take_waiting().is_empty());
    }
}
