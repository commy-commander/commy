use std::rc::Rc;

use chrono::Utc;
use commy_core::{ChannelId, ServerEvent, ServerId};
use commy_view::{manage, templates};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{parse, to_json, Args, Session};
use crate::sync_book::left_record;

#[derive(Deserialize)]
pub(super) struct MoveArg {
    channel: String,
    parent: Option<String>,
    position: u32,
}

impl Session {
    pub(super) async fn server_command(self: &Rc<Self>, cmd: &str, args: Args) -> Result<Value, String> {
        match cmd {
            "list_server_templates" => to_json(&templates::list()),
            "preview_server_template" => to_json(&templates::preview(args.code.as_deref().unwrap_or(""))?),
            "create_server" => {
                let name = manage::server_name(args.name.as_deref().unwrap_or(""))?;
                let template = templates::resolve(args.template.as_deref())?;
                let server = self.start_server(&name, |owner| template.build(&name, owner))?;
                to_json(&self.client.borrow().get_server(server)?)
            }
            "leave_server" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                self.leave_server(server).await
            }
            _ => self.manage(cmd, args),
        }
    }

    fn manage(&self, cmd: &str, args: Args) -> Result<Value, String> {
        let server: ServerId = parse(args.server_id.as_ref(), "server")?;
        let channel = || parse::<ChannelId>(args.channel_id.as_ref(), "channel");
        let name = args.name.as_deref().unwrap_or("");
        let payloads = match cmd {
            "rename_server" => vec![self.client.borrow().act(server, |state, me, _, _| manage::rename(state, me.perms, name))?],
            "create_channel" => {
                let kind = args.kind.as_deref().unwrap_or("text");
                let parent = args.parent_id.as_deref().and_then(|p| p.parse::<ChannelId>().ok());
                vec![self.client.borrow().act(server, |state, me, _, _| manage::create_channel(state, me.perms, name, kind, parent))?]
            }
            "create_category" => vec![self.client.borrow().act(server, |state, me, _, _| manage::create_category(state, me.perms, name))?],
            "update_channel" => {
                let channel = channel()?;
                let retention = args.retention_days;
                vec![self.client.borrow().act(server, |state, me, _, _| manage::update_channel(state, me.perms, channel, name, retention))?]
            }
            "delete_channel" => {
                let channel = channel()?;
                vec![self.client.borrow().act(server, |state, me, _, _| manage::delete_channel(state, me.perms, channel))?]
            }
            "set_channel_kind" => {
                let channel = channel()?;
                let kind = args.kind.as_deref().unwrap_or("");
                vec![self.client.borrow().act(server, |state, me, _, _| manage::channel_kind_set(state, me.perms, channel, kind))?]
            }
            "move_channels" => {
                let mut moves = Vec::new();
                for m in args.moves.unwrap_or_default().into_iter().take(manage::MAX_CHANNEL_MOVES) {
                    let parent = m.parent.as_deref().map(str::parse::<ChannelId>).transpose().map_err(|_| "invalid channel".to_string())?;
                    moves.push(manage::ChannelMove { channel: m.channel.parse().map_err(|_| "invalid channel".to_string())?, parent, position: m.position });
                }
                self.client.borrow().act_all(server, |state, me| manage::move_channels(state, me.perms, moves))?
            }
            _ => return Err(format!("{cmd} is not available in the web app yet")),
        };
        for payload in &payloads {
            self.publish(server, payload);
        }
        self.emit("server-event", json!(server.to_string()));
        if cmd == "rename_server" {
            self.emit("servers-changed", Value::Null);
        }
        Ok(Value::Null)
    }

    async fn leave_server(self: &Rc<Self>, server: ServerId) -> Result<Value, String> {
        let payload = self.client.borrow().act(server, |state, me, _, _| {
            if state.server.owner == me.user && !state.server.group {
                return Err("the owner cannot leave; delete the server instead".into());
            }
            Ok(ServerEvent::MemberLeave { server, user: me.user })
        })?;
        if self.voice_here(server).is_some() {
            self.leave_room().await;
        }
        self.publish_stored(server, &payload).await;
        let now = Utc::now();
        if let Some(live) = self.sync.borrow_mut().as_mut() {
            live.book.leave(server, now);
        }
        self.append_account(left_record(server, now));
        self.forget_left(server).await;
        Ok(Value::Null)
    }

    async fn forget_left(self: &Rc<Self>, server: ServerId) {
        let group = self.client.borrow().is_group(server);
        self.client.borrow_mut().forget(server);
        if let Some(node) = self.node.lock().await.as_mut() {
            node.leave_server(server);
        }
        self.refresh_sync();
        self.emit("servers-changed", Value::Null);
        if group {
            self.emit("dm-changed", json!(server.to_string()));
        }
    }
}
