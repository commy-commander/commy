use crate::catchup::names_mentioned;
use crate::channel_lock::KeyHolder;
use crate::view::{held_view, sees_message, MessageView};
use commy_core::{ChannelId, ChannelKind, Message, ServerState, UserId};
use serde::Serialize;

pub const MAX_HITS: usize = 50;
const MAX_TERMS: usize = 16;

#[derive(Debug, Default)]
pub struct SearchQuery {
    pub terms: Vec<String>,
    pub from: Option<UserId>,
    pub mentions: Option<UserId>,
    pub channel: Option<ChannelId>,
}

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub channel: String,
    pub channel_name: String,
    pub message: MessageView,
}

#[derive(Debug, Serialize)]
pub struct SearchResults {
    pub total: usize,
    pub hits: Vec<SearchHit>,
}

impl SearchQuery {
    pub fn new(terms: &[String], from: Option<UserId>, mentions: Option<UserId>, channel: Option<ChannelId>) -> Self {
        let terms = terms
            .iter()
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .take(MAX_TERMS)
            .collect();
        Self { terms, from, mentions, channel }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.from.is_none() && self.mentions.is_none() && self.channel.is_none()
    }
}

struct Matcher<'a> {
    query: &'a SearchQuery,
    mention_names: Vec<String>,
}

impl<'a> Matcher<'a> {
    fn new(state: &ServerState, query: &'a SearchQuery) -> Self {
        let mention_names = query
            .mentions
            .and_then(|user| state.profiles.get(&user))
            .map(|p| {
                let mut names = vec![p.username.clone()];
                names.extend(p.display_name.clone());
                names
            })
            .unwrap_or_default();
        Self { query, mention_names }
    }

    fn matches(&self, message: &Message) -> bool {
        if self.query.from.is_some_and(|from| from != message.author) {
            return false;
        }
        if self.query.mentions.is_some() {
            let names: Vec<&str> = self.mention_names.iter().map(String::as_str).collect();
            if !names_mentioned(&message.content, &names) {
                return false;
            }
        }
        if self.query.terms.is_empty() {
            return true;
        }
        let haystack = searchable_text(message);
        self.query.terms.iter().all(|term| haystack.contains(term.as_str()))
    }
}

fn searchable_text(message: &Message) -> String {
    let mut text = message.content.to_lowercase();
    for attachment in message.files() {
        text.push('\n');
        text.push_str(&attachment.filename.to_lowercase());
    }
    for (preview, _) in message.link_previews() {
        for part in [&preview.site, &preview.title, &preview.description] {
            text.push('\n');
            text.push_str(&part.to_lowercase());
        }
    }
    text
}

fn holds_messages(kind: ChannelKind) -> bool {
    matches!(kind, ChannelKind::Text | ChannelKind::Announcement)
}

pub fn search(state: &ServerState, query: &SearchQuery, viewer: Option<UserId>, limit: usize, keys: &KeyHolder) -> SearchResults {
    if query.is_empty() {
        return SearchResults { total: 0, hits: Vec::new() };
    }
    let matcher = Matcher::new(state, query);
    let sees = sees_message(state, viewer, keys);
    let readable = |channel: ChannelId| viewer.is_none_or(|v| state.server.can_view(channel, v));
    let mut found: Vec<(&Message, &str)> = state
        .server
        .channels
        .iter()
        .filter(|c| holds_messages(c.kind) && query.channel.is_none_or(|id| id == c.id) && readable(c.id))
        .flat_map(|c| state.messages(c.id).iter().map(move |m| (m, c.name.as_str())))
        .filter(|(m, _)| keys.open(state, m).is_some_and(|opened| matcher.matches(&opened)) && sees(m))
        .collect();
    found.sort_by_key(|(m, _)| std::cmp::Reverse(m.created_at));
    let total = found.len();
    let hits = found
        .into_iter()
        .take(limit)
        .map(|(message, channel_name)| SearchHit {
            channel: message.channel.to_string(),
            channel_name: channel_name.to_string(),
            message: held_view(state, message, keys),
        })
        .collect();
    SearchResults { total, hits }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Identity, Server, ServerEvent, UserProfile};
    use chrono::{Duration, Utc};

    struct Fixture {
        state: ServerState,
        general: ChannelId,
        news: ChannelId,
        keys: Vec<(UserId, Identity)>,
    }

    impl Fixture {
        fn new() -> Self {
            let identity = Identity::generate();
            let mut owner = UserProfile::new("owner");
            owner.identity = Some(identity.public());
            let mut server = Server::new("s", owner.id);
            let general = server.channels[0].id;
            let news_channel = commy_core::Channel::new("news", ChannelKind::Text, 3);
            let news = news_channel.id;
            server.channels.push(news_channel);
            let mut state = ServerState::new(server);
            state.profiles.insert(owner.id, owner.clone());
            Self { state, general, news, keys: vec![(owner.id, identity)] }
        }

        fn owner(&self) -> UserId {
            self.keys[0].0
        }

        fn join(&mut self, username: &str, display_name: Option<&str>) -> UserId {
            let identity = Identity::generate();
            let mut profile = UserProfile::new(username);
            profile.display_name = display_name.map(str::to_string);
            profile.identity = Some(identity.public());
            let id = profile.id;
            let server = self.state.server.id;
            self.state.apply(ServerEvent::MemberJoin { server, profile }).unwrap();
            self.keys.push((id, identity));
            id
        }

        fn post(&mut self, channel: ChannelId, author: UserId, text: &str, minutes_ago: i64) {
            let mut message = Message::new(channel, author, text);
            message.created_at = Utc::now() - Duration::minutes(minutes_ago);
            let key = &self.keys.iter().find(|(id, _)| *id == author).unwrap().1;
            message.sign(self.state.server.id, key);
            self.state.apply(ServerEvent::MessageCreate { message }).unwrap();
        }

        fn find(&self, query: SearchQuery) -> Vec<String> {
            search(&self.state, &query, Some(self.owner()), MAX_HITS, &KeyHolder::default()).hits.into_iter().map(|h| h.message.content).collect()
        }
    }

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn every_word_must_appear_and_case_is_ignored() {
        let mut f = Fixture::new();
        let owner = f.owner();
        f.post(f.general, owner, "Raid night is Friday", 3);
        f.post(f.general, owner, "friday works for me", 2);
        f.post(f.general, owner, "Saturday then", 1);
        assert_eq!(f.find(SearchQuery::new(&words("FRIDAY"), None, None, None)), ["friday works for me", "Raid night is Friday"]);
        assert_eq!(f.find(SearchQuery::new(&words("raid friday"), None, None, None)), ["Raid night is Friday"]);
        assert!(f.find(SearchQuery::new(&words("sunday"), None, None, None)).is_empty());
    }

    #[test]
    fn phrases_match_as_written() {
        let mut f = Fixture::new();
        let owner = f.owner();
        f.post(f.general, owner, "see you at the lake", 2);
        f.post(f.general, owner, "the big lake at noon", 1);
        let phrase = vec!["at the lake".to_string()];
        assert_eq!(f.find(SearchQuery::new(&phrase, None, None, None)), ["see you at the lake"]);
    }

    #[test]
    fn filters_narrow_by_author_mention_and_channel() {
        let mut f = Fixture::new();
        let owner = f.owner();
        let sam = f.join("sam", Some("Sam Lee"));
        let (general, news) = (f.general, f.news);
        f.post(general, owner, "hey @Sam Lee, lunch?", 5);
        f.post(general, sam, "lunch sounds good", 4);
        f.post(news, sam, "lunch menu is up", 3);
        f.post(news, owner, "ping @sam about the menu", 2);
        f.post(news, owner, "@samantha is someone else", 1);
        assert_eq!(f.find(SearchQuery::new(&[], Some(sam), None, None)), ["lunch menu is up", "lunch sounds good"]);
        assert_eq!(f.find(SearchQuery::new(&words("lunch"), Some(sam), None, Some(news))), ["lunch menu is up"]);
        assert_eq!(f.find(SearchQuery::new(&[], None, Some(sam), None)), ["ping @sam about the menu", "hey @Sam Lee, lunch?"]);
        assert_eq!(f.find(SearchQuery::new(&[], None, None, Some(general))).len(), 2);
    }

    #[test]
    fn attachment_names_are_searched_too() {
        let mut f = Fixture::new();
        let owner = f.owner();
        let mut message = Message::new(f.general, owner, "");
        message.attachments.push(commy_core::Attachment {
            blob: "b".into(),
            filename: "Trip-Photos.zip".into(),
            size_bytes: 10,
            mime_type: "application/zip".into(),
            provider: None,
        });
        message.sign(f.state.server.id, &f.keys[0].1);
        f.state.apply(ServerEvent::MessageCreate { message }).unwrap();
        assert_eq!(f.find(SearchQuery::new(&words("photos"), None, None, None)).len(), 1);
    }

    #[test]
    fn results_are_newest_first_capped_with_the_full_count() {
        let mut f = Fixture::new();
        let owner = f.owner();
        for i in 0..60 {
            f.post(f.general, owner, &format!("note {i}"), 100 - i);
        }
        let results = search(&f.state, &SearchQuery::new(&words("note"), None, None, None), None, MAX_HITS, &KeyHolder::default());
        assert_eq!(results.total, 60);
        assert_eq!(results.hits.len(), MAX_HITS);
        assert_eq!(results.hits[0].message.content, "note 59");
        assert_eq!(results.hits[0].channel_name, "general");
    }

    #[test]
    fn held_messages_are_found_only_by_moderators_and_their_author() {
        let mut f = Fixture::new();
        let (owner, sam, robin) = (f.owner(), f.join("sam", None), f.join("robin", None));
        f.post(f.general, sam, "cheap gold here", 2);
        f.post(f.general, robin, "gold rush tonight is fine", 1);
        let server = f.state.server.id;
        let rules = commy_core::AutoMod { blocked_terms: vec!["cheap".into()], ..commy_core::AutoMod::default() };
        f.state.apply(ServerEvent::AutoModSet { server, rules }).unwrap();
        let found = |viewer| {
            search(&f.state, &SearchQuery::new(&words("gold"), None, None, None), Some(viewer), MAX_HITS, &KeyHolder::default())
                .hits
                .into_iter()
                .map(|h| (h.message.content, h.message.held.is_some()))
                .collect::<Vec<_>>()
        };
        let all = vec![("gold rush tonight is fine".to_string(), false), ("cheap gold here".to_string(), true)];
        assert_eq!(found(owner), all);
        assert_eq!(found(sam), all);
        assert_eq!(found(robin), vec![("gold rush tonight is fine".to_string(), false)]);
    }

    #[test]
    fn an_empty_query_finds_nothing() {
        let mut f = Fixture::new();
        let owner = f.owner();
        f.post(f.general, owner, "hello", 1);
        let blank = vec!["  ".to_string()];
        assert!(SearchQuery::new(&blank, None, None, None).is_empty());
        assert_eq!(search(&f.state, &SearchQuery::new(&blank, None, None, None), None, MAX_HITS, &KeyHolder::default()).total, 0);
    }
}
