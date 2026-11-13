use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use commy_core::mention::names_mentioned;
use commy_core::{ChannelId, DmAction, DmMessage, DmStore, Message, MessageId, Presence, ServerId, ServerState, UserId};

pub const FRESH_SECS: i64 = 10 * 60;
pub const MAX_TEXT_CHARS: usize = 300;
const CLOCK_SLACK_SECS: i64 = 60;
const HOUR_SECS: i64 = 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub gap_secs: i64,
    pub per_hour: usize,
}

pub const SENDING: Limits = Limits { gap_secs: 10 * 60, per_hour: 5 };
pub const HEARING: Limits = Limits { gap_secs: 5 * 60, per_hour: 10 };

#[derive(Debug, Default)]
pub struct NudgeLog {
    sent: VecDeque<(UserId, i64)>,
}

impl NudgeLog {
    pub const fn new() -> Self {
        Self { sent: VecDeque::new() }
    }

    pub fn take(&mut self, who: UserId, now_secs: i64, limits: Limits) -> Result<(), i64> {
        self.sent.retain(|(_, at)| now_secs - at < HOUR_SECS);
        let since_theirs = self.sent.iter().rev().find(|(sent_to, _)| *sent_to == who).map(|(_, at)| now_secs - at);
        if let Some(since) = since_theirs.filter(|since| *since < limits.gap_secs) {
            return Err(limits.gap_secs - since);
        }
        if self.sent.len() >= limits.per_hour {
            let oldest = self.sent.front().map_or(now_secs, |(_, at)| *at);
            return Err(HOUR_SECS - (now_secs - oldest));
        }
        self.sent.push_back((who, now_secs));
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heard {
    pub from: UserId,
    pub target: MessageId,
    pub server: Option<ServerId>,
    pub channel: Option<ChannelId>,
    pub text: String,
}

pub fn paused(presence: Presence) -> bool {
    presence == Presence::DoNotDisturb
}

pub fn is_nudge(message: &DmMessage) -> bool {
    matches!(message.action, DmAction::Nudge { .. })
}

pub fn fresh(created_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    (-CLOCK_SLACK_SECS..FRESH_SECS).contains(&(now - created_at).num_seconds())
}

pub fn names_of(state: &ServerState, user: UserId) -> Vec<String> {
    let profile = state.profiles.get(&user);
    let nickname = state.server.member(user).and_then(|m| m.nickname.clone());
    [profile.map(|p| p.username.clone()), profile.and_then(|p| p.display_name.clone()), nickname].into_iter().flatten().collect()
}

pub fn latest_mention(state: &ServerState, me: UserId, target: UserId, channel: ChannelId, now: DateTime<Utc>) -> Option<&Message> {
    let names = names_of(state, target);
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    state
        .messages(channel)
        .iter()
        .rev()
        .filter(|m| m.author == me && fresh(m.created_at, now))
        .find(|m| names_mentioned(&m.content, &names))
}

pub fn latest_dm(dms: &DmStore, me: UserId, target: UserId, now: DateTime<Utc>) -> Option<&DmMessage> {
    dms.conversation(target).iter().rev().find(|m| m.from == me && m.is_post() && fresh(m.created_at, now))
}

pub fn wait_line(wait_secs: i64) -> String {
    let minutes = (wait_secs.max(1) + 59) / 60;
    let unit = if minutes == 1 { "minute" } else { "minutes" };
    format!("You can notify them anyway again in {minutes} {unit}")
}

pub fn short_text(text: &str) -> String {
    text.chars().take(MAX_TEXT_CHARS).collect()
}

pub fn nudge(from: UserId, to: UserId, target: MessageId, server: Option<ServerId>, channel: Option<ChannelId>, text: &str) -> DmMessage {
    DmMessage::acting(from, to, DmAction::Nudge { target, server, channel }, short_text(text))
}

pub fn heard(message: &DmMessage, now: DateTime<Utc>) -> Option<Heard> {
    let DmAction::Nudge { target, server, channel } = message.action else { return None };
    fresh(message.created_at, now).then(|| Heard { from: message.from, target, server, channel, text: short_text(&message.content) })
}

pub fn server_nudge_welcome(state: &ServerState, from: UserId, me: UserId, channel: Option<ChannelId>) -> bool {
    let members = state.server.is_current_member(from) && state.server.is_current_member(me);
    members && channel.is_some_and(|channel| state.server.can_view(channel, from) && state.server.can_view(channel, me))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use commy_core::{Identity, Server, ServerEvent, UserProfile};

    const MINUTE: i64 = 60;

    #[test]
    fn one_nudge_per_person_every_few_minutes() {
        let (sam, mia) = (UserId::new(), UserId::new());
        let mut log = NudgeLog::default();
        assert_eq!(log.take(sam, 0, SENDING), Ok(()));
        assert_eq!(log.take(sam, 4 * MINUTE, SENDING), Err(6 * MINUTE), "says how long until the next one");
        assert_eq!(log.take(mia, 4 * MINUTE, SENDING), Ok(()), "someone else can still be reached");
        assert_eq!(log.take(sam, 10 * MINUTE, SENDING), Ok(()));
    }

    #[test]
    fn the_wait_reads_in_whole_minutes() {
        assert_eq!(wait_line(30), "You can notify them anyway again in 1 minute");
        assert_eq!(wait_line(6 * MINUTE), "You can notify them anyway again in 6 minutes");
        assert_eq!(wait_line(6 * MINUTE + 1), "You can notify them anyway again in 7 minutes");
    }

    #[test]
    fn only_a_handful_of_nudges_an_hour() {
        let mut log = NudgeLog::default();
        for n in 0..SENDING.per_hour as i64 {
            assert_eq!(log.take(UserId::new(), n * MINUTE, SENDING), Ok(()));
        }
        assert_eq!(log.take(UserId::new(), 10 * MINUTE, SENDING), Err(50 * MINUTE));
        assert_eq!(log.take(UserId::new(), HOUR_SECS, SENDING), Ok(()), "the oldest one ages out");
    }

    #[test]
    fn only_do_not_disturb_counts_as_paused() {
        assert!(paused(Presence::DoNotDisturb));
        for presence in [Presence::Online, Presence::Idle, Presence::Invisible, Presence::Offline] {
            assert!(!paused(presence));
        }
    }

    #[test]
    fn a_nudge_carries_its_message_and_goes_stale() {
        let (me, mia) = (UserId::new(), UserId::new());
        let target = MessageId::new();
        let channel = ChannelId::new();
        let server = ServerId::new();
        let long = "a".repeat(MAX_TEXT_CHARS + 50);
        let message = nudge(me, mia, target, Some(server), Some(channel), &long);
        assert!(is_nudge(&message));
        assert!(!message.action.is_stored(), "a nudge never shows up as a DM");
        let now = message.created_at;
        let heard = heard(&message, now + Duration::minutes(2)).unwrap();
        assert_eq!((heard.from, heard.target, heard.server, heard.channel), (me, target, Some(server), Some(channel)));
        assert_eq!(heard.text.chars().count(), MAX_TEXT_CHARS);
        assert_eq!(super::heard(&message, now + Duration::seconds(FRESH_SECS)), None, "an old nudge is ignored");
        assert_eq!(super::heard(&DmMessage::new(me, mia, "hi"), now), None);
    }

    struct Fixture {
        state: ServerState,
        ids: Vec<UserId>,
        keys: Vec<Identity>,
        general: ChannelId,
    }

    fn server_with(members: &[&str]) -> Fixture {
        let identity = Identity::generate();
        let mut owner = UserProfile::new("owner");
        owner.identity = Some(identity.public());
        let mut state = ServerState::new(Server::new("s", owner.id));
        state.profiles.insert(owner.id, owner.clone());
        let (mut ids, mut keys) = (vec![owner.id], vec![identity]);
        for name in members {
            let identity = Identity::generate();
            let mut profile = UserProfile::new(*name);
            profile.identity = Some(identity.public());
            ids.push(profile.id);
            keys.push(identity);
            state.apply(ServerEvent::MemberJoin { server: state.server.id, profile }).unwrap();
        }
        let general = state.server.channels[0].id;
        Fixture { state, ids, keys, general }
    }

    impl Fixture {
        fn post(&mut self, author: UserId, text: &str, minutes_ago: i64) {
            let mut message = Message::new(self.general, author, text);
            message.created_at = Utc::now() - Duration::minutes(minutes_ago);
            let at = self.ids.iter().position(|id| *id == author).unwrap();
            message.sign(self.state.server.id, &self.keys[at]);
            self.state.apply(ServerEvent::MessageCreate { message }).unwrap();
        }
    }

    #[test]
    fn the_nudge_is_for_your_own_recent_mention_of_them() {
        let mut f = server_with(&["mia", "sam"]);
        let (me, mia, sam, general) = (f.ids[0], f.ids[1], f.ids[2], f.general);
        let now = Utc::now();
        assert!(latest_mention(&f.state, me, mia, general, now).is_none());
        f.post(me, "@mia old news", 30);
        f.post(sam, "@mia from sam", 1);
        f.post(me, "no mention here", 1);
        assert!(latest_mention(&f.state, me, mia, general, now).is_none(), "stale, someone else's, or not a mention");
        f.post(me, "hey @Mia, are you around?", 1);
        assert_eq!(latest_mention(&f.state, me, mia, general, now).map(|m| m.content.as_str()), Some("hey @Mia, are you around?"));
        assert!(latest_mention(&f.state, me, sam, general, now).is_none());
    }

    #[test]
    fn the_nudge_is_for_your_own_recent_dm() {
        let (me, mia) = (UserId::new(), UserId::new());
        let mut dms = DmStore::new();
        let now = Utc::now();
        let mut theirs = DmMessage::new(mia, me, "from mia");
        theirs.created_at = now;
        dms.apply(mia, theirs);
        assert!(latest_dm(&dms, me, mia, now).is_none());
        let mut mine = DmMessage::new(me, mia, "are you up?");
        mine.created_at = now - Duration::minutes(1);
        dms.apply(mia, mine);
        assert_eq!(latest_dm(&dms, me, mia, now).map(|m| m.content.as_str()), Some("are you up?"));
        assert!(latest_dm(&dms, me, mia, now + Duration::seconds(FRESH_SECS)).is_none());
    }

    #[test]
    fn a_server_nudge_needs_both_people_in_that_server() {
        let f = server_with(&["mia"]);
        let (state, owner, mia, general) = (&f.state, f.ids[0], f.ids[1], f.general);
        assert!(server_nudge_welcome(state, owner, mia, Some(general)));
        assert!(!server_nudge_welcome(state, UserId::new(), mia, Some(general)), "a stranger cannot nudge through a server");
        assert!(!server_nudge_welcome(state, owner, mia, Some(ChannelId::new())));
        assert!(!server_nudge_welcome(state, owner, mia, None));
    }
}
