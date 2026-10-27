use std::rc::Rc;

use chrono::Utc;
use commy_core::{Action, ServerEvent, ServerId};
use commy_view::activities;
use serde_json::{json, Value};

use super::{parse, to_json, Args, Session};

impl Session {
    pub(super) async fn activity_command(self: &Rc<Self>, cmd: &str, args: Args) -> Result<Value, String> {
        let server: ServerId = parse(args.server_id.as_ref(), "server")?;
        let (channel, _) = self.voice_here(server).ok_or("join a voice channel in this server first")?;
        match cmd {
            "set_activity" => {
                let activity = args.activity;
                let payload = self.client.borrow().act(server, |state, me, _, _| activities::set(state, me.user, channel, activity, Utc::now()))?;
                self.publish(server, &payload);
                self.tell_neighbors(server, &payload).await;
                self.emit("server-event", json!(server.to_string()));
                Ok(Value::Null)
            }
            "send_activity_input" => {
                let seq = args.seq.ok_or("missing input number")?;
                let input = args.input.ok_or("missing input")?;
                let payload = self.client.borrow().act(server, |state, me, _, _| activities::input(state, me.user, channel, seq, input, Utc::now()))?;
                self.tell_neighbors(server, &payload).await;
                Ok(Value::Null)
            }
            _ => Err(format!("{cmd} is not available in the web app yet")),
        }
    }

    pub(super) fn hear_activity(&self, server: ServerId, event: &ServerEvent, action: Option<Action>) -> bool {
        if !matches!(event, ServerEvent::ActivityInput { .. }) {
            return false;
        }
        let Some((channel, _)) = self.voice_here(server) else { return true };
        if !self.client.borrow().admits(server, event, action) {
            return true;
        }
        let heard = self.read_state(server, |state, _| Ok(activities::heard_input(state, channel, event))).ok().flatten();
        if let Some(view) = heard.and_then(|view| to_json(&view).ok()) {
            self.emit("activity-input", view);
        }
        true
    }
}
