use std::rc::Rc;

use commy_core::{ChannelId, MessageId, ServerId, UserId};
use commy_view::events::{self, EventForm};
use commy_view::{forum, moderate};
use serde_json::{json, Value};

use super::{parse, Args, Session};

fn tags_of<T: serde::de::DeserializeOwned>(raw: Option<Value>) -> Result<Vec<T>, String> {
    raw.map_or(Ok(Vec::new()), |tags| serde_json::from_value(tags).map_err(|_| "invalid tags".to_string()))
}

impl Session {
    pub(super) async fn community_command(self: &Rc<Self>, cmd: &str, args: Args) -> Result<Value, String> {
        let server: ServerId = parse(args.server_id.as_ref(), "server")?;
        let channel = || parse::<ChannelId>(args.channel_id.as_ref(), "channel");
        let user = || parse::<UserId>(args.user_id.as_ref(), "user");
        let client = || self.client.borrow();
        let payload = match cmd {
            "post_forum" => return self.post_forum(server, args).await,
            "save_event" => {
                let channel = args.channel_id.as_deref().filter(|c| !c.is_empty()).map(str::parse::<ChannelId>).transpose().map_err(|_| "invalid channel".to_string())?;
                let form = EventForm {
                    id: args.event_id.clone(),
                    title: args.title.clone().unwrap_or_default(),
                    description: args.description.clone().unwrap_or_default(),
                    start: args.start.clone().unwrap_or_default(),
                    end: args.end.clone(),
                    channel,
                };
                client().act(server, |state, me, _, _| events::save(state, me.perms, me.user, form, || MessageId::new().to_string()))?
            }
            "delete_event" => {
                let id = args.event_id.clone().ok_or("missing event")?;
                client().act(server, |state, me, _, _| events::delete(state, me.perms, me.user, id))?
            }
            "set_rsvp" => {
                let id = args.event_id.clone().ok_or("missing event")?;
                let rsvp = args.rsvp.as_deref();
                client().act(server, |state, me, _, _| events::rsvp(state, me.user, id, rsvp))?
            }
            "set_forum_tags" => {
                let (channel, tags) = (channel()?, tags_of::<forum::TagInput>(args.tags.clone())?);
                client().act(server, |state, me, _, _| forum::set_tags(state, me.perms, me.user, channel, tags))?
            }
            "edit_forum_post" => {
                let title = forum::title(args.title.as_deref().unwrap_or(""))?;
                let (channel, message) = (channel()?, parse::<MessageId>(args.message_id.as_ref(), "message")?);
                let tags = forum::tag_picks(&tags_of::<String>(args.tags.clone())?)?;
                client().act(server, |state, me, _, _| forum::edit_post(state, me.perms, me.user, channel, message, title, tags))?
            }
            "set_voice_status" => {
                let (channel, text) = (channel()?, args.text.clone().unwrap_or_default());
                client().act(server, |state, me, _, _| moderate::voice_status(state, me.perms, me.user, channel, text))?
            }
            "moderate_voice" => {
                let (user, muted, deafened) = (user()?, args.muted.unwrap_or(false), args.deafened.unwrap_or(false));
                client().act(server, |state, me, _, _| moderate::voice_moderate(state, me.perms, user, muted, deafened))?
            }
            "set_stage_speaker" => {
                let (channel, user, speaker) = (channel()?, user()?, args.speaker.unwrap_or(false));
                client().act(server, |state, me, _, _| moderate::stage_speaker(state, me.perms, me.user, channel, user, speaker))?
            }
            "raise_hand" => {
                let (channel, _) = self.voice_here(server).ok_or("you are not in a stage channel here")?;
                let raised = args.raised.unwrap_or(false);
                client().act(server, |state, me, _, _| moderate::raise_hand(state, me.user, channel, raised))?
            }
            _ => return Err(format!("{cmd} is not available in the web app yet")),
        };
        self.publish(server, &payload);
        self.refresh_gates();
        self.emit("server-event", json!(server.to_string()));
        Ok(Value::Null)
    }

    async fn post_forum(self: &Rc<Self>, server: ServerId, args: Args) -> Result<Value, String> {
        let title = forum::title(args.title.as_deref().unwrap_or(""))?;
        let channel: ChannelId = parse(args.channel_id.as_ref(), "channel")?;
        let tags = forum::tag_picks(&tags_of::<String>(args.tags)?)?;
        let (message, view) = self.client.borrow().compose_post(server, channel, args.content.as_deref().unwrap_or(""), &tags)?;
        let root = message.id;
        let posted = self.deliver(server, message, view).await?;
        let payload = self.client.borrow().act(server, |state, me, _, _| Ok(forum::post(state, channel, root, title, tags, me.user)))?;
        self.publish(server, &payload);
        self.emit("server-event", json!(server.to_string()));
        Ok(posted)
    }
}
