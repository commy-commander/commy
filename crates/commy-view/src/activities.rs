use chrono::{DateTime, Utc};
use commy_core::activity::{admits_input, admits_queue, admits_set};
use commy_core::{Activity, ActivityInput, ChannelId, QueueChange, ServerEvent, ServerState, UserId};
use serde::Serialize;

use crate::act::{not_timed_out, Act};

pub const UNSUPPORTED_SOURCE: &str = "pick a YouTube link or a direct https link to an .mp4, .webm or .ogg file";

pub const QUEUE_NOT_ALLOWED: &str = "that cannot go in the queue right now";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum HeardInput {
    Input(ActivityInput),
    Queue { queue: QueueChange },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InputView {
    pub server: String,
    pub channel: String,
    pub seq: u64,
    pub input: HeardInput,
    pub by: String,
}

pub fn set(state: &ServerState, me: UserId, channel: ChannelId, activity: Option<Activity>, now: DateTime<Utc>) -> Act<ServerEvent> {
    not_timed_out(state, me, now)?;
    if activity.is_some() && !state.server.activities_enabled {
        return Err("Activities are turned off in this server's settings".into());
    }
    if activity.as_ref().is_some_and(|a| !a.valid()) {
        return Err(UNSUPPORTED_SOURCE.into());
    }
    if !admits_set(state, me, channel, activity.as_ref()) {
        return Err(match (&activity, state.activities.get(&channel)) {
            (Some(_), Some(_)) => "another activity is already running in this channel",
            (None, Some(_)) => "only the person who started it or a moderator can end it",
            (None, None) => "nothing is running in this channel",
            (Some(_), None) => "you are not in this voice channel",
        }
        .into());
    }
    Ok(ServerEvent::ActivitySet { server: state.server.id, channel, activity, by: me })
}

pub fn input(state: &ServerState, me: UserId, channel: ChannelId, seq: u64, input: ActivityInput, now: DateTime<Utc>) -> Act<ServerEvent> {
    not_timed_out(state, me, now)?;
    if !admits_input(state, me, channel, &input) {
        return Err("that is not possible in the running activity".into());
    }
    Ok(ServerEvent::ActivityInput { server: state.server.id, channel, seq, input, by: me })
}

pub fn queue(state: &ServerState, me: UserId, channel: ChannelId, seq: u64, change: QueueChange, now: DateTime<Utc>) -> Act<ServerEvent> {
    not_timed_out(state, me, now)?;
    if !admits_queue(state, me, channel, &change) {
        return Err(QUEUE_NOT_ALLOWED.into());
    }
    Ok(ServerEvent::ActivityQueue { server: state.server.id, channel, seq, change, by: me })
}

pub fn heard_input(state: &ServerState, my_channel: ChannelId, event: &ServerEvent) -> Option<InputView> {
    let (server, channel, seq, by, heard) = match event {
        ServerEvent::ActivityInput { server, channel, seq, input, by } => {
            (*server, *channel, *seq, *by, admits_input(state, *by, *channel, input).then(|| HeardInput::Input(input.clone())))
        }
        ServerEvent::ActivityQueue { server, channel, seq, change, by } => {
            (*server, *channel, *seq, *by, admits_queue(state, *by, *channel, change).then(|| HeardInput::Queue { queue: change.clone() }))
        }
        _ => return None,
    };
    if channel != my_channel || server != state.server.id {
        return None;
    }
    Some(InputView { server: server.to_string(), channel: channel.to_string(), seq, input: heard?, by: by.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::server::{ChannelKind, Member};
    use commy_core::{ActivityKind, Server, WatchSource};

    fn room() -> (ServerState, ChannelId, UserId, UserId) {
        let owner = UserId::new();
        let member = UserId::new();
        let mut state = ServerState::new(Server::new("Hangout", owner));
        state.server.members.push(Member { user: member, nickname: None, roles: vec![], joined_at: Utc::now(), timeout: None });
        state.server.activities_enabled = true;
        let voice = state.server.channels.iter().find(|c| c.kind == ChannelKind::Voice).map(|c| c.id).unwrap();
        for user in [owner, member] {
            let event = ServerEvent::VoiceState { user, channel: Some(voice), muted: false, deafened: false, since: None };
            state.apply(event).unwrap();
        }
        (state, voice, owner, member)
    }

    fn film(starter: UserId) -> Activity {
        let source = WatchSource::YouTube { video: "dQw4w9WgXcQ".into() };
        Activity { id: "run-1".into(), kind: ActivityKind::Watch { source }, starter, started_at: 1 }
    }

    #[test]
    fn starting_follows_the_server_switch_and_the_source_rules() {
        let (mut state, voice, _, member) = room();
        let now = Utc::now();
        let event = set(&state, member, voice, Some(film(member)), now).unwrap();
        assert!(matches!(event, ServerEvent::ActivitySet { by, activity: Some(_), .. } if by == member));
        let bad = Activity { kind: ActivityKind::Watch { source: WatchSource::YouTube { video: "nope".into() } }, ..film(member) };
        assert_eq!(set(&state, member, voice, Some(bad), now), Err(UNSUPPORTED_SOURCE.into()));
        assert!(set(&state, member, voice, None, now).is_err(), "nothing to end yet");
        state.server.activities_enabled = false;
        assert!(set(&state, member, voice, Some(film(member)), now).unwrap_err().contains("turned off"));
    }

    #[test]
    fn inputs_need_a_run_and_reach_only_the_same_channel() {
        let (mut state, voice, owner, member) = room();
        let now = Utc::now();
        let play = ActivityInput::Playback { playing: true, position: 3.0, at: 1 };
        assert!(input(&state, member, voice, 1, play.clone(), now).is_err(), "no run yet");
        state.apply(set(&state, owner, voice, Some(film(owner)), now).unwrap()).unwrap();
        let event = input(&state, member, voice, 2, play.clone(), now).unwrap();
        let view = heard_input(&state, voice, &event).unwrap();
        assert_eq!(view.seq, 2);
        assert_eq!(view.by, member.to_string());
        assert_eq!(view.input, HeardInput::Input(play));
        assert_eq!(heard_input(&state, ChannelId::new(), &event), None);
        let stranger = ServerEvent::ActivityInput { server: state.server.id, channel: voice, seq: 3, input: ActivityInput::Join, by: UserId::new() };
        assert_eq!(heard_input(&state, voice, &stranger), None);
    }

    fn queued(video: &str) -> QueueChange {
        QueueChange::Add { source: WatchSource::YouTube { video: video.into() }, title: "Song".into() }
    }

    #[test]
    fn queue_changes_need_a_watch_run_and_reach_only_the_same_channel() {
        let (mut state, voice, owner, member) = room();
        let now = Utc::now();
        assert_eq!(queue(&state, member, voice, 1, queued("dQw4w9WgXcQ"), now), Err(QUEUE_NOT_ALLOWED.into()), "no run yet");
        state.apply(set(&state, owner, voice, Some(film(owner)), now).unwrap()).unwrap();
        let event = queue(&state, member, voice, 2, queued("dQw4w9WgXcQ"), now).unwrap();
        let view = heard_input(&state, voice, &event).unwrap();
        assert_eq!(view.seq, 2);
        assert_eq!(view.input, HeardInput::Queue { queue: queued("dQw4w9WgXcQ") });
        assert_eq!(heard_input(&state, ChannelId::new(), &event), None);
        assert!(queue(&state, member, voice, 3, queued("nope"), now).is_err(), "only real videos go in");
        let lineup = QueueChange::Lineup { items: vec![] };
        assert!(queue(&state, member, voice, 4, lineup.clone(), now).is_err(), "only the starter sends the lineup");
        assert!(queue(&state, owner, voice, 5, lineup, now).is_ok());
        let stranger = ServerEvent::ActivityQueue { server: state.server.id, channel: voice, seq: 6, change: queued("dQw4w9WgXcQ"), by: UserId::new() };
        assert_eq!(heard_input(&state, voice, &stranger), None);
    }

    #[test]
    fn queue_changes_read_as_one_queue_key_for_the_ui() {
        let view = HeardInput::Queue { queue: QueueChange::Skip { run: "run-1".into() } };
        assert_eq!(serde_json::to_string(&view).unwrap(), r#"{"queue":{"skip":{"run":"run-1"}}}"#);
        assert_eq!(serde_json::to_string(&HeardInput::Input(ActivityInput::Join)).unwrap(), r#""join""#);
    }
}
