use std::rc::Rc;

use commy_core::{ChannelId, DmMessage, Permissions, ServerId};
use commy_net::invite::Expiry;
use commy_view::voice_invite::{invite_from, invite_target, presence_of};
use serde_json::Value;

use super::{parse, to_json, Args, Session};

impl Session {
    pub(super) async fn voice_invite_command(self: &Rc<Self>, cmd: &str, args: Args) -> Result<Value, String> {
        let server: ServerId = parse(args.server_id.as_ref(), "server")?;
        let channel: ChannelId = parse(args.channel_id.as_ref(), "channel")?;
        match cmd {
            "voice_channel_presence" => {
                let me = self.client.borrow().profile().map(|p| p.id);
                let client = self.client.borrow();
                let servers = client.servers.lock().unwrap();
                match servers.get(&server) {
                    Some(state) => to_json(&presence_of(state, channel, me)?),
                    None => Ok(Value::Null),
                }
            }
            "send_voice_invite" => {
                let to_server: ServerId = parse(args.to_server_id.as_ref(), "server")?;
                let to_channel: ChannelId = parse(args.to_channel_id.as_ref(), "channel")?;
                let invite = self.read_state(server, |state, me| invite_from(state, server, channel, me))?;
                self.read_state(to_server, |state, me| invite_target(state, me, to_channel))?;
                self.post_attachment(to_server, to_channel, invite.into_attachment(), Permissions::SEND_MESSAGES, "send messages here").await
            }
            "send_dm_voice_invite" => {
                let (me, partner) = self.recipient(args.user_id.as_ref())?;
                let mut invite = self.read_state(server, |state, me| invite_from(state, server, channel, me))?;
                let already_in = self.read_state(server, |state, _| Ok(state.server.member(partner).is_some()))?;
                if !already_in {
                    invite.invite_link = self.link_for_outsiders(server).await;
                }
                let mut message = DmMessage::new(me.id, partner, String::new());
                message.attachments.push(invite.into_attachment());
                to_json(&self.post_dm(&me, partner, message).await?.ok_or("invite not sent")?)
            }
            _ => Err(format!("{cmd} is not available in the web app yet")),
        }
    }

    async fn link_for_outsiders(&self, server: ServerId) -> Option<String> {
        let (group, public) = self.read_state(server, |state, _| Ok((state.server.group, state.server.public))).ok()?;
        if group {
            return None;
        }
        let expiry = if public { Expiry::Never } else { Expiry::Week };
        self.create_invite(server, expiry, None).await.ok().map(|created| created.url)
    }
}

