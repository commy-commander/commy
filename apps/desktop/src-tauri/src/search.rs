use crate::catchup::names_mentioned;
use crate::channel_lock::KeyHolder;
use crate::view::{dm_message_view, held_view, sees_message, server_name, DmMessageView, MessageView};
use chrono::{DateTime, Utc};
use commy_core::{ChannelId, ChannelKind, DmMessage, DmStore, Message, ServerId, ServerState, UserId};
use serde::Serialize;
use std::collections::HashMap;

pub const MAX_HITS: usize = 50;
const MAX_TERMS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub offset: usize,
    pub limit: usize,
}

impl Page {
    pub fn newest() -> Self {
        Self::at(0)
    }

    pub fn at(offset: usize) -> Self {
        Self { offset, limit: MAX_HITS }
    }

    pub fn start(self, total: usize) -> usize {
        let limit = self.limit.clamp(1, MAX_HITS);
        if total == 0 {
            return 0;
        }
        self.offset.min((total - 1) / limit * limit)
    }

    fn cut<T>(self, found: Vec<T>) -> (usize, usize, Vec<T>) {
        let total = found.len();
        let start = self.start(total);
        let hits = found.into_iter().skip(start).take(self.limit.clamp(1, MAX_HITS)).collect();
        (total, start, hits)
    }
}

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
    pub offset: usize,
    pub hits: Vec<SearchHit>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DmSearchHit {
    Dm { user: String, name: String, message: DmMessageView },
    Group { server: String, name: String, channel: String, message: MessageView },
}

#[derive(Debug, Serialize)]
pub struct DmSearchResults {
    pub total: usize,
    pub offset: usize,
    pub hits: Vec<DmSearchHit>,
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
        Self::with_profile(query, query.mentions.and_then(|user| state.profiles.get(&user)))
    }

    fn for_dms(dms: &DmStore, query: &'a SearchQuery) -> Self {
        Self::with_profile(query, query.mentions.and_then(|user| dms.contacts.get(&user)))
    }

    fn with_profile(query: &'a SearchQuery, profile: Option<&commy_core::UserProfile>) -> Self {
        let mention_names = profile
            .map(|p| {
                let mut names = vec![p.username.clone()];
                names.extend(p.display_name.clone());
                names
            })
            .unwrap_or_default();
        Self { query, mention_names }
    }

    fn accepts(&self, author: UserId, content: &str, haystack: impl FnOnce() -> String) -> bool {
        if self.query.from.is_some_and(|from| from != author) {
            return false;
        }
        if self.query.mentions.is_some() {
            let names: Vec<&str> = self.mention_names.iter().map(String::as_str).collect();
            if !names_mentioned(content, &names) {
                return false;
            }
        }
        if self.query.terms.is_empty() {
            return true;
        }
        let haystack = haystack();
        self.query.terms.iter().all(|term| haystack.contains(term.as_str()))
    }

    fn matches(&self, message: &Message) -> bool {
        self.accepts(message.author, &message.content, || searchable_text(message))
    }

    fn matches_dm(&self, message: &DmMessage) -> bool {
        self.accepts(message.from, &message.content, || dm_text(message))
    }
}

fn dm_text(message: &DmMessage) -> String {
    let mut text = message.content.to_lowercase();
    for attachment in &message.attachments {
        text.push('\n');
        text.push_str(&attachment.filename.to_lowercase());
    }
    text
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
    if let Some(poll) = message.poll() {
        for part in std::iter::once(&poll.question).chain(poll.options.iter()) {
            text.push('\n');
            text.push_str(&part.to_lowercase());
        }
    }
    text
}

fn holds_messages(kind: ChannelKind) -> bool {
    kind.holds_messages()
}

fn channel_matches<'a>(
    state: &'a ServerState,
    query: &'a SearchQuery,
    viewer: Option<UserId>,
    keys: &'a KeyHolder,
    opens: crate::view::ChannelOpens,
    now: Option<chrono::DateTime<chrono::Utc>>,
) -> impl Iterator<Item = (&'a Message, &'a str)> + 'a {
    let matcher = Matcher::new(state, query);
    let sees = sees_message(state, viewer, keys);
    let readable = move |channel: ChannelId| viewer.is_none_or(|v| state.server.can_view(channel, v));
    let floor = move |channel: ChannelId| now.and_then(|now| crate::view::history_floor(state, viewer, &opens, channel, now));
    state
        .server
        .channels
        .iter()
        .filter(move |c| holds_messages(c.kind) && query.channel.is_none_or(|id| id == c.id) && readable(c.id))
        .flat_map(move |c| {
            let floor = floor(c.id);
            state.messages(c.id).iter().filter(move |m| crate::view::within_history(floor, m)).map(move |m| (m, c.name.as_str()))
        })
        .filter(move |(m, _)| keys.open(state, m).is_some_and(|opened| matcher.matches(&opened)) && sees(m))
}

pub fn search(
    state: &ServerState,
    query: &SearchQuery,
    viewer: Option<UserId>,
    page: Page,
    keys: &KeyHolder,
    opens: &crate::view::ChannelOpens,
    now: Option<chrono::DateTime<chrono::Utc>>,
) -> SearchResults {
    if query.is_empty() {
        return SearchResults { total: 0, offset: 0, hits: Vec::new() };
    }
    let mut found: Vec<(&Message, &str)> = channel_matches(state, query, viewer, keys, opens.clone(), now).collect();
    found.sort_by_key(|(m, _)| std::cmp::Reverse(m.created_at));
    let (total, offset, page) = page.cut(found);
    let hits = page
        .into_iter()
        .map(|(message, channel_name)| SearchHit {
            channel: message.channel.to_string(),
            channel_name: channel_name.to_string(),
            message: held_view(state, message, keys),
        })
        .collect();
    SearchResults { total, offset, hits }
}

enum Found<'a> {
    Dm(UserId, &'a DmMessage),
    Group(&'a ServerState, &'a Message),
}

impl Found<'_> {
    fn at(&self) -> DateTime<Utc> {
        match self {
            Found::Dm(_, m) => m.created_at,
            Found::Group(_, m) => m.created_at,
        }
    }
}

pub fn search_dms(
    dms: &DmStore,
    servers: &HashMap<ServerId, ServerState>,
    query: &SearchQuery,
    me: UserId,
    partner: Option<UserId>,
    blocked: &[UserId],
    page: Page,
    keys: &KeyHolder,
) -> DmSearchResults {
    if query.is_empty() || query.channel.is_some() {
        return DmSearchResults { total: 0, offset: 0, hits: Vec::new() };
    }
    let matcher = Matcher::for_dms(dms, query);
    let partners = match partner {
        Some(one) => vec![one],
        None => dms.partners().into_iter().filter(|p| !blocked.contains(p)).collect(),
    };
    let mut found: Vec<Found> = partners
        .iter()
        .flat_map(|&p| dms.posts(p).filter(|m| matcher.matches_dm(m)).map(move |m| Found::Dm(p, m)))
        .collect();
    if partner.is_none() {
        found.extend(
            servers
                .values()
                .filter(|s| s.server.group)
                .flat_map(|state| channel_matches(state, query, Some(me), keys, crate::view::ChannelOpens::new(), None).map(move |(m, _)| Found::Group(state, m))),
        );
    }
    found.sort_by_key(|f| std::cmp::Reverse(f.at()));
    let (total, offset, page) = page.cut(found);
    let hits = page
        .into_iter()
        .map(|found| match found {
            Found::Dm(user, message) => DmSearchHit::Dm {
                user: user.to_string(),
                name: dms.contacts.get(&user).map(|p| p.visible_name().to_string()).unwrap_or_else(|| "Unknown User".to_string()),
                message: dm_message_view(message, me, dms, false),
            },
            Found::Group(state, message) => DmSearchHit::Group {
                server: state.server.id.to_string(),
                name: server_name(state, Some(me)),
                channel: message.channel.to_string(),
                message: held_view(state, message, keys),
            },
        })
        .collect();
    DmSearchResults { total, offset, hits }
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
            search(&self.state, &query, Some(self.owner()), Page::newest(), &KeyHolder::default(), &crate::view::ChannelOpens::new(), None).hits.into_iter().map(|h| h.message.content).collect()
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
    fn members_without_read_message_history_search_only_what_they_could_see() {
        let mut f = Fixture::new();
        let owner = f.owner();
        let reader = f.join("reader", None);
        let (general, news) = (f.general, f.news);
        f.post(general, owner, "lake trip last month", 40);
        f.post(general, owner, "lake trip this week", 5);
        f.post(news, owner, "lake photos posted", 2);
        let now = Utc::now();
        let opens = crate::view::ChannelOpens::from([(general, now - Duration::minutes(10))]);
        let lake = SearchQuery::new(&words("lake"), None, None, None);
        let find = |f: &Fixture, viewer| {
            search(&f.state, &lake, Some(viewer), Page::newest(), &KeyHolder::default(), &opens, Some(now))
                .hits
                .into_iter()
                .map(|h| h.message.content)
                .collect::<Vec<_>>()
        };
        assert_eq!(find(&f, reader).len(), 3, "history readers search everything");
        let sid = f.state.server.id;
        let mut everyone = f.state.server.roles[0].clone();
        everyone.permissions.insert(commy_core::Permissions::NO_READ_MESSAGE_HISTORY);
        f.state.apply(ServerEvent::RoleUpdate { server: sid, role: everyone }).unwrap();
        assert_eq!(find(&f, reader), ["lake trip this week"], "only what arrived since opening, and nothing from channels never opened");
        assert_eq!(find(&f, owner).len(), 3, "the owner keeps history");
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
        let results = search(&f.state, &SearchQuery::new(&words("note"), None, None, None), None, Page::newest(), &KeyHolder::default(), &crate::view::ChannelOpens::new(), None);
        assert_eq!(results.total, 60);
        assert_eq!(results.offset, 0);
        assert_eq!(results.hits.len(), MAX_HITS);
        assert_eq!(results.hits[0].message.content, "note 59");
        assert_eq!(results.hits[0].channel_name, "general");
    }

    #[test]
    fn later_pages_continue_where_the_newest_left_off_and_clamp_to_the_last_page() {
        let mut f = Fixture::new();
        let owner = f.owner();
        for i in 0..60 {
            f.post(f.general, owner, &format!("note {i}"), 100 - i);
        }
        let query = SearchQuery::new(&words("note"), None, None, None);
        let second = search(&f.state, &query, None, Page::at(50), &KeyHolder::default(), &crate::view::ChannelOpens::new(), None);
        assert_eq!((second.total, second.offset, second.hits.len()), (60, 50, 10));
        assert_eq!(second.hits[0].message.content, "note 9");
        assert_eq!(second.hits[9].message.content, "note 0");
        let beyond = search(&f.state, &query, None, Page::at(500), &KeyHolder::default(), &crate::view::ChannelOpens::new(), None);
        assert_eq!((beyond.offset, beyond.hits.len()), (50, 10));
        let odd = search(&f.state, &query, None, Page { offset: 7, limit: 1000 }, &KeyHolder::default(), &crate::view::ChannelOpens::new(), None);
        assert_eq!((odd.offset, odd.hits.len()), (7, MAX_HITS));
        assert_eq!(odd.hits[0].message.content, "note 52");
        assert_eq!(Page::at(3).start(0), 0);
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
            search(&f.state, &SearchQuery::new(&words("gold"), None, None, None), Some(viewer), Page::newest(), &KeyHolder::default(), &crate::view::ChannelOpens::new(), None)
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
        assert_eq!(search(&f.state, &SearchQuery::new(&blank, None, None, None), None, Page::newest(), &KeyHolder::default(), &crate::view::ChannelOpens::new(), None).total, 0);
    }

    struct Inbox {
        dms: DmStore,
        servers: HashMap<ServerId, ServerState>,
        me: UserId,
        sam: UserId,
        robin: UserId,
    }

    impl Inbox {
        fn new() -> Self {
            let mut dms = DmStore::new();
            let me = UserId::new();
            let mut sam = UserProfile::new("sam");
            sam.display_name = Some("Sam Lee".into());
            let robin = UserProfile::new("robin");
            let (sam_id, robin_id) = (sam.id, robin.id);
            dms.contacts.insert(sam.id, sam);
            dms.contacts.insert(robin.id, robin);
            Self { dms, servers: HashMap::new(), me, sam: sam_id, robin: robin_id }
        }

        fn say(&mut self, from: UserId, to: UserId, text: &str, minutes_ago: i64) {
            let partner = if from == self.me { to } else { from };
            let mut message = DmMessage::new(from, to, text);
            message.created_at = Utc::now() - Duration::minutes(minutes_ago);
            assert!(self.dms.insert(partner, message));
        }

        fn group(&mut self, members: &[UserId]) -> ServerId {
            let mut f = Fixture::new();
            f.state.server.group = true;
            f.state.server.name = "the crew".into();
            let owner = f.owner();
            for (i, member) in members.iter().enumerate() {
                let mut profile = UserProfile::new(format!("member{i}"));
                profile.id = *member;
                let server = f.state.server.id;
                f.state.apply(ServerEvent::MemberJoin { server, profile }).unwrap();
            }
            f.post(f.general, owner, "crew lunch at noon", 4);
            f.post(f.general, owner, "bring the lake photos", 30);
            let id = f.state.server.id;
            self.servers.insert(id, f.state);
            id
        }

        fn find(&self, query: SearchQuery, partner: Option<UserId>, page: Page) -> DmSearchResults {
            search_dms(&self.dms, &self.servers, &query, self.me, partner, &[], page, &KeyHolder::default())
        }
    }

    fn hit_texts(results: &DmSearchResults) -> Vec<String> {
        results
            .hits
            .iter()
            .map(|h| match h {
                DmSearchHit::Dm { message, .. } => message.content.clone(),
                DmSearchHit::Group { message, .. } => message.content.clone(),
            })
            .collect()
    }

    #[test]
    fn direct_messages_and_groups_are_searched_together_newest_first() {
        let mut inbox = Inbox::new();
        let (me, sam, robin) = (inbox.me, inbox.sam, inbox.robin);
        inbox.say(sam, me, "lunch tomorrow?", 50);
        inbox.say(me, sam, "lunch works", 40);
        inbox.say(robin, me, "no lunch for me", 10);
        inbox.say(robin, me, "dinner instead", 5);
        inbox.group(&[sam, robin]);
        let all = inbox.find(SearchQuery::new(&words("lunch"), None, None, None), None, Page::newest());
        assert_eq!(all.total, 4);
        assert_eq!(hit_texts(&all), ["crew lunch at noon", "no lunch for me", "lunch works", "lunch tomorrow?"]);
        match &all.hits[0] {
            DmSearchHit::Group { name, channel, .. } => {
                assert_eq!(name, "the crew");
                assert!(!channel.is_empty());
            }
            other => panic!("expected a group hit, got {other:?}"),
        }
        match &all.hits[1] {
            DmSearchHit::Dm { user, name, message } => {
                assert_eq!(user, &robin.to_string());
                assert_eq!(name, "robin");
                assert!(!message.outgoing);
            }
            other => panic!("expected a dm hit, got {other:?}"),
        }
        let with_sam = inbox.find(SearchQuery::new(&words("lunch"), None, None, None), Some(sam), Page::newest());
        assert_eq!(hit_texts(&with_sam), ["lunch works", "lunch tomorrow?"]);
        let mine = inbox.find(SearchQuery::new(&[], Some(me), None, None), None, Page::newest());
        assert_eq!(hit_texts(&mine), ["lunch works"]);
    }

    #[test]
    fn dm_mentions_use_contact_names_and_blocked_partners_are_skipped() {
        let mut inbox = Inbox::new();
        let (me, sam, robin) = (inbox.me, inbox.sam, inbox.robin);
        inbox.say(me, robin, "ask @Sam Lee about the keys", 3);
        inbox.say(me, robin, "@samantha is someone else", 2);
        inbox.say(sam, me, "@robin says hi", 1);
        let mentions_sam = inbox.find(SearchQuery::new(&[], None, Some(sam), None), None, Page::newest());
        assert_eq!(hit_texts(&mentions_sam), ["ask @Sam Lee about the keys"]);
        let query = SearchQuery::new(&words("@"), None, None, None);
        let without_sam = search_dms(&inbox.dms, &inbox.servers, &query, me, None, &[sam], Page::newest(), &KeyHolder::default());
        assert_eq!(without_sam.total, 2);
        let channel_scoped = SearchQuery::new(&words("keys"), None, None, Some(ChannelId::new()));
        assert_eq!(inbox.find(channel_scoped, None, Page::newest()).total, 0);
    }

    #[test]
    fn dm_pages_report_the_whole_count_and_the_offset_used() {
        let mut inbox = Inbox::new();
        let (me, sam) = (inbox.me, inbox.sam);
        for i in 0..70 {
            inbox.say(sam, me, &format!("log {i}"), 200 - i);
        }
        let query = SearchQuery::new(&words("log"), None, None, None);
        let first = inbox.find(SearchQuery::new(&words("log"), None, None, None), Some(sam), Page::newest());
        assert_eq!((first.total, first.offset, first.hits.len()), (70, 0, 50));
        assert_eq!(hit_texts(&first)[0], "log 69");
        let second = inbox.find(query, Some(sam), Page::at(50));
        assert_eq!((second.total, second.offset, second.hits.len()), (70, 50, 20));
        assert_eq!(hit_texts(&second)[0], "log 19");
        assert_eq!(hit_texts(&second)[19], "log 0");
    }
}
