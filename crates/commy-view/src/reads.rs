use std::collections::{BTreeMap, HashSet};

use commy_core::{ChannelId, Message, ServerState, UserId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadMark {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub at: i64,
    #[serde(default)]
    pub set: i64,
    #[serde(default)]
    pub back: bool,
}

impl ReadMark {
    pub fn newer_than(&self, other: &ReadMark) -> bool {
        if self.back || other.back {
            (self.set, self.at, &self.id) > (other.set, other.at, &other.id)
        } else {
            (self.at, &self.id, self.set) > (other.at, &other.id, other.set)
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadState {
    #[serde(default)]
    pub channels: BTreeMap<String, ReadMark>,
}

impl ReadState {
    pub fn merge(a: &ReadState, b: &ReadState) -> ReadState {
        let mut channels = a.channels.clone();
        for (channel, mark) in &b.channels {
            let take = channels.get(channel).map_or(true, |mine| mark.newer_than(mine));
            if take {
                channels.insert(channel.clone(), mark.clone());
            }
        }
        ReadState { channels }
    }

    pub fn changed_from(&self, before: &ReadState) -> Vec<String> {
        self.channels
            .iter()
            .filter(|(channel, mark)| before.channels.get(*channel) != Some(*mark))
            .map(|(channel, _)| channel.clone())
            .collect()
    }
}

pub fn count_after(messages: &[Message], mark: &ReadMark, me: Option<UserId>, blocked: &HashSet<UserId>) -> usize {
    let anchored = mark.id.as_deref().and_then(|id| messages.iter().position(|m| m.id.to_string() == id));
    let unread: Box<dyn Iterator<Item = &Message>> = match anchored {
        Some(index) => Box::new(messages[index + 1..].iter()),
        None => Box::new(messages.iter().filter(|m| m.created_at.timestamp_millis() > mark.at)),
    };
    unread.filter(|m| Some(m.author) != me && !blocked.contains(&m.author)).count()
}

pub fn latest_mark(messages: &[Message], now: i64) -> Option<ReadMark> {
    let last = messages.last()?;
    Some(ReadMark { id: Some(last.id.to_string()), at: last.created_at.timestamp_millis(), set: now, back: false })
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChannelRead {
    pub mark: ReadMark,
    pub server: Option<String>,
    pub unread: usize,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ReadView {
    pub channels: BTreeMap<String, ChannelRead>,
}

pub fn view<'a>(
    servers: impl Iterator<Item = &'a ServerState> + Clone,
    state: &ReadState,
    only: Option<&[String]>,
    me: Option<UserId>,
    blocked: &HashSet<UserId>,
) -> ReadView {
    let channels = state
        .channels
        .iter()
        .filter(|(channel, _)| only.map_or(true, |list| list.contains(channel)))
        .map(|(channel, mark)| {
            let cid: Option<ChannelId> = channel.parse().ok();
            let found = cid.and_then(|cid| servers.clone().find(|s| s.server.channel(cid).is_some()).map(|s| (s, cid)));
            let (server, unread) = match found {
                Some((state, cid)) if me.is_none_or(|me| state.server.can_view(cid, me)) => {
                    (Some(state.server.id.to_string()), count_after(state.messages(cid), mark, me, blocked))
                }
                Some((state, _)) => (Some(state.server.id.to_string()), 0),
                None => (None, 0),
            };
            (channel.clone(), ChannelRead { mark: mark.clone(), server, unread })
        })
        .collect();
    ReadView { channels }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrefPiece {
    #[serde(default)]
    pub value: serde_json::Value,
    pub at: i64,
}

impl PrefPiece {
    pub fn beats(&self, other: Option<&PrefPiece>) -> bool {
        other.is_none_or(|old| (self.at, self.value.to_string()) > (old.at, old.value.to_string()))
    }
}
