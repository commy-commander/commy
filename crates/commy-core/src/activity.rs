use crate::ids::{ChannelId, UserId};
use crate::message::Attachment;
use crate::permissions::Permissions;
use crate::server::Server;
use crate::store::ServerState;
use serde::{Deserialize, Serialize};

pub const MAX_ACTIVITY_ID_CHARS: usize = 64;
pub const MAX_WATCH_URL_CHARS: usize = 2048;
pub const BOARD_CELLS: usize = 9;
pub const MAX_PLAYERS: usize = 2;
pub const MAX_PARTICIPANTS: usize = 99;
pub const VIDEO_LINK_EXTENSIONS: [&str; 3] = ["mp4", "webm", "ogg"];
pub const YOUTUBE_VIDEO_ID_CHARS: usize = 11;
pub const MAX_QUEUE_LENGTH: usize = 50;
pub const MAX_QUEUE_TITLE_CHARS: usize = 120;
const CELL_EMPTY: u8 = 0;
const CELL_O: u8 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchSource {
    Link { url: String },
    Attachment { attachment: Attachment },
    #[serde(rename = "youtube")]
    YouTube { video: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    Watch { source: WatchSource },
    TicTacToe,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Activity {
    pub id: String,
    pub kind: ActivityKind,
    pub starter: UserId,
    pub started_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityInput {
    Join,
    Leave,
    Playback { playing: bool, position: f64, at: i64 },
    Move { cell: u8 },
    Rematch,
    Sync {
        participants: Vec<UserId>,
        players: Vec<UserId>,
        board: Vec<u8>,
        playing: bool,
        position: f64,
        at: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueuedWatch {
    pub slot: u32,
    pub source: WatchSource,
    pub title: String,
    pub by: UserId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueChange {
    Add { source: WatchSource, title: String },
    Remove { slot: u32 },
    Skip { run: String },
    Finished { run: String },
    Lineup { items: Vec<QueuedWatch> },
}

pub fn is_video_link(url: &str) -> bool {
    if url.len() > MAX_WATCH_URL_CHARS || url.chars().any(char::is_whitespace) {
        return false;
    }
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let path = rest.split(['?', '#']).next().unwrap_or("");
    let Some((host, _)) = path.split_once('/') else { return false };
    if host.is_empty() {
        return false;
    }
    let file = path.rsplit('/').next().unwrap_or("");
    let Some((stem, ext)) = file.rsplit_once('.') else { return false };
    !stem.is_empty() && VIDEO_LINK_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e))
}

pub fn is_youtube_video_id(id: &str) -> bool {
    id.len() == YOUTUBE_VIDEO_ID_CHARS && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl WatchSource {
    pub fn valid(&self) -> bool {
        match self {
            WatchSource::Link { url } => is_video_link(url),
            WatchSource::Attachment { attachment } => {
                attachment.mime_type.starts_with("video/") && !attachment.blob.is_empty() && !attachment.filename.is_empty()
            }
            WatchSource::YouTube { video } => is_youtube_video_id(video),
        }
    }
}

impl ActivityKind {
    pub fn name(&self) -> &'static str {
        match self {
            ActivityKind::Watch { .. } => "Watch together",
            ActivityKind::TicTacToe => "Tic-tac-toe",
        }
    }

    pub fn valid(&self) -> bool {
        match self {
            ActivityKind::Watch { source } => source.valid(),
            ActivityKind::TicTacToe => true,
        }
    }
}

impl Activity {
    pub fn valid(&self) -> bool {
        !self.id.is_empty()
            && self.id.len() <= MAX_ACTIVITY_ID_CHARS
            && self.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && self.kind.valid()
    }
}

fn finite_position(position: f64) -> bool {
    position.is_finite() && position >= 0.0
}

impl ActivityInput {
    pub fn valid_for(&self, kind: &ActivityKind) -> bool {
        let watch = matches!(kind, ActivityKind::Watch { .. });
        match self {
            ActivityInput::Join | ActivityInput::Leave => true,
            ActivityInput::Playback { position, .. } => watch && finite_position(*position),
            ActivityInput::Move { cell } => !watch && (*cell as usize) < BOARD_CELLS,
            ActivityInput::Rematch => !watch,
            ActivityInput::Sync { participants, players, board, position, .. } => {
                participants.len() <= MAX_PARTICIPANTS
                    && players.len() <= MAX_PLAYERS
                    && board.len() == BOARD_CELLS
                    && board.iter().all(|c| (CELL_EMPTY..=CELL_O).contains(c))
                    && finite_position(*position)
            }
        }
    }

    pub fn starter_only(&self) -> bool {
        matches!(self, ActivityInput::Sync { .. })
    }
}

fn plain_title(title: &str) -> bool {
    title.chars().count() <= MAX_QUEUE_TITLE_CHARS && !title.chars().any(char::is_control)
}

fn plain_run_id(run: &str) -> bool {
    !run.is_empty() && run.len() <= MAX_ACTIVITY_ID_CHARS && run.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

impl QueuedWatch {
    pub fn valid(&self) -> bool {
        self.source.valid() && plain_title(&self.title)
    }
}

impl QueueChange {
    pub fn valid(&self) -> bool {
        match self {
            QueueChange::Add { source, title } => source.valid() && plain_title(title),
            QueueChange::Remove { .. } => true,
            QueueChange::Skip { run } | QueueChange::Finished { run } => plain_run_id(run),
            QueueChange::Lineup { items } => {
                let mut slots: Vec<u32> = items.iter().map(|item| item.slot).collect();
                slots.sort_unstable();
                slots.dedup();
                items.len() <= MAX_QUEUE_LENGTH && slots.len() == items.len() && items.iter().all(QueuedWatch::valid)
            }
        }
    }

    pub fn starter_only(&self) -> bool {
        matches!(self, QueueChange::Lineup { .. })
    }
}

pub fn moderates_voice(perms: Permissions) -> bool {
    perms.allows(Permissions::MOVE_MEMBERS) || perms.allows(Permissions::MUTE_MEMBERS) || perms.allows(Permissions::DEAFEN_MEMBERS)
}

pub fn may_end(server: &Server, by: UserId, running: &Activity) -> bool {
    running.starter == by || moderates_voice(server.permissions_for(by))
}

pub fn present(state: &ServerState, user: UserId, channel: ChannelId) -> bool {
    state.voice_states.get(&user) == Some(&channel)
}

pub fn admits_set(state: &ServerState, by: UserId, channel: ChannelId, activity: Option<&Activity>) -> bool {
    if !present(state, by, channel) || (activity.is_some() && !state.server.activities_enabled) {
        return false;
    }
    let running = state.activities.get(&channel);
    match (activity, running) {
        (Some(next), Some(current)) => next.starter == by && next.valid() && may_end(&state.server, by, current),
        (Some(next), None) => next.starter == by && next.valid(),
        (None, Some(current)) => may_end(&state.server, by, current),
        (None, None) => false,
    }
}

pub fn admits_input(state: &ServerState, by: UserId, channel: ChannelId, input: &ActivityInput) -> bool {
    let Some(running) = state.activities.get(&channel) else { return false };
    present(state, by, channel) && input.valid_for(&running.kind) && (!input.starter_only() || running.starter == by)
}

pub fn admits_queue(state: &ServerState, by: UserId, channel: ChannelId, change: &QueueChange) -> bool {
    let Some(running) = state.activities.get(&channel) else { return false };
    matches!(running.kind, ActivityKind::Watch { .. })
        && present(state, by, channel)
        && change.valid()
        && (!change.starter_only() || running.starter == by)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ServerEvent;
    use crate::test_support::{fixture, join};

    fn game(starter: UserId) -> Activity {
        Activity { id: "run-1".into(), kind: ActivityKind::TicTacToe, starter, started_at: 1_000 }
    }

    fn watch(starter: UserId, url: &str) -> Activity {
        let source = WatchSource::Link { url: url.into() };
        Activity { id: "run-2".into(), kind: ActivityKind::Watch { source }, starter, started_at: 1_000 }
    }

    fn enter_voice(state: &mut ServerState, user: UserId, channel: ChannelId) {
        let event = ServerEvent::VoiceState { user, channel: Some(channel), muted: false, deafened: false, since: None };
        assert_eq!(state.apply(event), Ok(true));
    }

    #[test]
    fn only_direct_https_video_links_count() {
        assert!(is_video_link("https://example.org/clips/final.mp4"));
        assert!(is_video_link("https://example.org/a/b.WEBM?token=1#t=10"));
        assert!(is_video_link("https://example.org/talk.ogg"));
        assert!(!is_video_link("http://example.org/final.mp4"), "plain http leaks the viewer");
        assert!(!is_video_link("https://example.org/watch?v=abc"));
        assert!(!is_video_link("https://example.org/final.mkv"));
        assert!(!is_video_link("https://example.org/.mp4"));
        assert!(!is_video_link("https://example.org"));
        assert!(!is_video_link("https:///final.mp4"));
        assert!(!is_video_link("https://example.org/a b.mp4"));
        assert!(!is_video_link(&format!("https://example.org/{}.mp4", "x".repeat(MAX_WATCH_URL_CHARS))));
    }

    #[test]
    fn attachments_must_be_videos() {
        let video = Attachment { blob: "b".into(), filename: "clip.mp4".into(), size_bytes: 1, mime_type: "video/mp4".into(), provider: None };
        assert!(WatchSource::Attachment { attachment: video.clone() }.valid());
        let image = Attachment { mime_type: "image/png".into(), ..video.clone() };
        assert!(!WatchSource::Attachment { attachment: image }.valid());
        let blank = Attachment { blob: String::new(), ..video };
        assert!(!WatchSource::Attachment { attachment: blank }.valid());
    }

    #[test]
    fn youtube_sources_carry_only_a_video_id() {
        assert!(WatchSource::YouTube { video: "dQw4w9WgXcQ".into() }.valid());
        assert!(WatchSource::YouTube { video: "a-b_c-d_e-f".into() }.valid());
        assert!(!WatchSource::YouTube { video: "short".into() }.valid());
        assert!(!WatchSource::YouTube { video: "dQw4w9WgXcQx".into() }.valid());
        assert!(!WatchSource::YouTube { video: "dQw4w9WgX/Q".into() }.valid());
        assert!(!WatchSource::YouTube { video: "https://you".into() }.valid());
    }

    #[test]
    fn youtube_keeps_earlier_wire_tags_and_has_a_plain_json_name() {
        let tag = |source: WatchSource| postcard::to_stdvec(&source).unwrap()[0];
        assert_eq!(tag(WatchSource::Link { url: "https://example.org/a.mp4".into() }), 0);
        assert_eq!(tag(WatchSource::YouTube { video: "dQw4w9WgXcQ".into() }), 2);
        let json = serde_json::to_string(&WatchSource::YouTube { video: "dQw4w9WgXcQ".into() }).unwrap();
        assert_eq!(json, r#"{"youtube":{"video":"dQw4w9WgXcQ"}}"#);
    }

    #[test]
    fn activity_ids_are_short_and_plain() {
        let user = UserId::new();
        assert!(game(user).valid());
        assert!(!Activity { id: String::new(), ..game(user) }.valid());
        assert!(!Activity { id: "x".repeat(MAX_ACTIVITY_ID_CHARS + 1), ..game(user) }.valid());
        assert!(!Activity { id: "run 1".into(), ..game(user) }.valid());
        assert!(watch(user, "https://example.org/a.mp4").valid());
        assert!(!watch(user, "https://example.org/a").valid());
    }

    #[test]
    fn inputs_match_their_activity() {
        let board = ActivityKind::TicTacToe;
        let film = ActivityKind::Watch { source: WatchSource::Link { url: "https://example.org/a.mp4".into() } };
        assert!(ActivityInput::Join.valid_for(&board) && ActivityInput::Leave.valid_for(&film));
        assert!(ActivityInput::Move { cell: 8 }.valid_for(&board));
        assert!(!ActivityInput::Move { cell: 9 }.valid_for(&board));
        assert!(!ActivityInput::Move { cell: 0 }.valid_for(&film));
        assert!(ActivityInput::Rematch.valid_for(&board) && !ActivityInput::Rematch.valid_for(&film));
        assert!(ActivityInput::Playback { playing: true, position: 12.5, at: 1 }.valid_for(&film));
        assert!(!ActivityInput::Playback { playing: true, position: -1.0, at: 1 }.valid_for(&film));
        assert!(!ActivityInput::Playback { playing: true, position: f64::NAN, at: 1 }.valid_for(&film));
        assert!(!ActivityInput::Playback { playing: true, position: 1.0, at: 1 }.valid_for(&board));
        let sync = |board: Vec<u8>, players: usize| ActivityInput::Sync {
            participants: vec![],
            players: vec![UserId::new(); players],
            board,
            playing: false,
            position: 0.0,
            at: 0,
        };
        assert!(sync(vec![0; 9], 2).valid_for(&board));
        assert!(!sync(vec![0; 8], 2).valid_for(&board));
        assert!(!sync(vec![3; 9], 2).valid_for(&board));
        assert!(!sync(vec![0; 9], 3).valid_for(&board));
        assert!(sync(vec![0; 9], 0).starter_only() && !ActivityInput::Join.starter_only());
    }

    #[test]
    fn starting_needs_voice_presence_and_one_activity_per_channel() {
        let (mut state, _, owner) = fixture();
        let voice = state.server.channels.iter().find(|c| c.kind == crate::server::ChannelKind::Voice).map(|c| c.id).unwrap();
        let user = join(&mut state, "user");
        let other = join(&mut state, "other");
        state.server.activities_enabled = true;
        assert!(!admits_set(&state, user, voice, Some(&game(user))), "not in the channel yet");
        enter_voice(&mut state, user, voice);
        enter_voice(&mut state, other, voice);
        enter_voice(&mut state, owner, voice);
        state.server.activities_enabled = false;
        assert!(!admits_set(&state, user, voice, Some(&game(user))), "Activities are off for this server");
        state.server.activities_enabled = true;
        assert!(admits_set(&state, user, voice, Some(&game(user))));
        assert!(!admits_set(&state, user, voice, Some(&game(other))), "a start names its own starter");
        assert!(!admits_set(&state, user, voice, None), "nothing to end yet");
        let start = ServerEvent::ActivitySet { server: state.server.id, channel: voice, activity: Some(game(user)), by: user };
        assert_eq!(state.apply(start), Ok(true));
        assert_eq!(state.activities.get(&voice), Some(&game(user)));
        assert!(!admits_set(&state, other, voice, Some(&game(other))), "one activity per channel");
        assert!(admits_set(&state, user, voice, Some(&game(user))), "the starter may re-announce its own run");
        assert!(!admits_set(&state, other, voice, None), "a participant cannot end it");
        assert!(admits_set(&state, user, voice, None));
        assert!(admits_set(&state, owner, voice, None), "a voice moderator can end it");
        assert!(admits_set(&state, owner, voice, Some(&game(owner))), "a moderator may replace it");
        assert!(admits_input(&state, other, voice, &ActivityInput::Join));
        assert!(!admits_input(&state, other, voice, &ActivityInput::Playback { playing: true, position: 0.0, at: 0 }));
        let sync = ActivityInput::Sync { participants: vec![], players: vec![], board: vec![0; 9], playing: false, position: 0.0, at: 0 };
        assert!(!admits_input(&state, other, voice, &sync), "only the starter syncs");
        assert!(admits_input(&state, user, voice, &sync));
        let stranger = UserId::new();
        assert!(!admits_input(&state, stranger, voice, &ActivityInput::Join));
    }

    #[test]
    fn the_run_ends_with_its_starter_or_when_the_starter_leaves_voice() {
        let (mut state, _, _) = fixture();
        state.server.activities_enabled = true;
        let sid = state.server.id;
        let voice = state.server.channels.iter().find(|c| c.kind == crate::server::ChannelKind::Voice).map(|c| c.id).unwrap();
        let user = join(&mut state, "user");
        enter_voice(&mut state, user, voice);
        let start = ServerEvent::ActivitySet { server: sid, channel: voice, activity: Some(game(user)), by: user };
        assert_eq!(state.apply(start.clone()), Ok(true));
        assert_eq!(state.apply(start), Ok(false), "announcing the same run again changes nothing");
        let end = ServerEvent::ActivitySet { server: sid, channel: voice, activity: None, by: user };
        assert_eq!(state.apply(end.clone()), Ok(true));
        assert!(state.activities.is_empty());
        assert_eq!(state.apply(end), Ok(false));
        let again = ServerEvent::ActivitySet { server: sid, channel: voice, activity: Some(game(user)), by: user };
        assert_eq!(state.apply(again), Ok(true));
        let leave = ServerEvent::VoiceState { user, channel: None, muted: false, deafened: false, since: None };
        assert_eq!(state.apply(leave), Ok(true));
        assert!(state.activities.is_empty(), "the starter leaving voice ends the run");
        let input = ServerEvent::ActivityInput { server: sid, channel: voice, seq: 1, input: ActivityInput::Join, by: user };
        assert_eq!(state.apply(input), Ok(false), "inputs never touch the store");
    }

    #[test]
    fn a_stranger_cannot_start_on_the_wire() {
        let (mut state, _, _) = fixture();
        let sid = state.server.id;
        let voice = state.server.channels.iter().find(|c| c.kind == crate::server::ChannelKind::Voice).map(|c| c.id).unwrap();
        let user = join(&mut state, "user");
        let start = ServerEvent::ActivitySet { server: sid, channel: voice, activity: Some(game(user)), by: user };
        assert_eq!(state.apply(start), Ok(false), "not in voice, so nothing starts");
        enter_voice(&mut state, user, voice);
        let forged = ServerEvent::ActivitySet { server: sid, channel: voice, activity: Some(game(UserId::new())), by: user };
        assert_eq!(state.apply(forged), Ok(false));
    }

    fn tune(video: &str) -> WatchSource {
        WatchSource::YouTube { video: video.into() }
    }

    fn slot(slot: u32, video: &str, by: UserId) -> QueuedWatch {
        QueuedWatch { slot, source: tune(video), title: String::new(), by }
    }

    #[test]
    fn queue_changes_carry_only_real_videos_and_plain_titles() {
        let user = UserId::new();
        assert!(QueueChange::Add { source: tune("dQw4w9WgXcQ"), title: "Song".into() }.valid());
        assert!(!QueueChange::Add { source: tune("nope"), title: String::new() }.valid());
        assert!(!QueueChange::Add { source: tune("dQw4w9WgXcQ"), title: "x".repeat(MAX_QUEUE_TITLE_CHARS + 1) }.valid());
        assert!(!QueueChange::Add { source: tune("dQw4w9WgXcQ"), title: "two\nlines".into() }.valid());
        assert!(QueueChange::Add { source: WatchSource::Link { url: "https://example.org/a.mp4".into() }, title: String::new() }.valid());
        assert!(QueueChange::Skip { run: "run-1".into() }.valid());
        assert!(!QueueChange::Finished { run: "run 1".into() }.valid());
        assert!(!QueueChange::Skip { run: String::new() }.valid());
        assert!(QueueChange::Lineup { items: vec![slot(1, "dQw4w9WgXcQ", user), slot(2, "a-b_c-d_e-f", user)] }.valid());
        assert!(!QueueChange::Lineup { items: vec![slot(1, "dQw4w9WgXcQ", user), slot(1, "a-b_c-d_e-f", user)] }.valid(), "slots are unique");
        let long = (0..=MAX_QUEUE_LENGTH as u32).map(|n| slot(n, "dQw4w9WgXcQ", user)).collect();
        assert!(!QueueChange::Lineup { items: long }.valid());
        assert!(QueueChange::Lineup { items: vec![] }.starter_only() && !QueueChange::Remove { slot: 1 }.starter_only());
    }

    #[test]
    fn the_queue_belongs_to_a_running_watch_in_your_voice_channel() {
        let (mut state, _, _) = fixture();
        state.server.activities_enabled = true;
        let sid = state.server.id;
        let voice = state.server.channels.iter().find(|c| c.kind == crate::server::ChannelKind::Voice).map(|c| c.id).unwrap();
        let user = join(&mut state, "user");
        let other = join(&mut state, "other");
        let add = QueueChange::Add { source: tune("dQw4w9WgXcQ"), title: String::new() };
        let lineup = QueueChange::Lineup { items: vec![slot(1, "dQw4w9WgXcQ", other)] };
        enter_voice(&mut state, user, voice);
        assert!(!admits_queue(&state, user, voice, &add), "nothing is running");
        let start = ServerEvent::ActivitySet { server: sid, channel: voice, activity: Some(game(user)), by: user };
        assert_eq!(state.apply(start), Ok(true));
        assert!(!admits_queue(&state, user, voice, &add), "a game has no queue");
        let film = Activity { kind: ActivityKind::Watch { source: tune("dQw4w9WgXcQ") }, ..game(user) };
        let start = ServerEvent::ActivitySet { server: sid, channel: voice, activity: Some(film), by: user };
        assert_eq!(state.apply(start), Ok(true));
        assert!(!admits_queue(&state, other, voice, &add), "not in the voice channel");
        enter_voice(&mut state, other, voice);
        assert!(admits_queue(&state, other, voice, &add));
        assert!(!admits_queue(&state, other, voice, &lineup), "only the starter sends the lineup");
        assert!(admits_queue(&state, user, voice, &lineup));
        let event = ServerEvent::ActivityQueue { server: sid, channel: voice, seq: 1, change: add, by: other };
        assert_eq!(state.apply(event), Ok(false), "queue changes never touch the store");
    }

    #[test]
    fn queue_changes_have_plain_json_names() {
        let json = serde_json::to_string(&QueueChange::Skip { run: "run-1".into() }).unwrap();
        assert_eq!(json, r#"{"skip":{"run":"run-1"}}"#);
        let add = QueueChange::Add { source: tune("dQw4w9WgXcQ"), title: "Song".into() };
        assert_eq!(serde_json::to_string(&add).unwrap(), r#"{"add":{"source":{"youtube":{"video":"dQw4w9WgXcQ"}},"title":"Song"}}"#);
    }
}
