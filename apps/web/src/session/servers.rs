use std::rc::Rc;

use chrono::Utc;
use commy_core::channel_key::KeyScope;
use commy_core::server::ImageRef;
use commy_core::{ChannelId, Permissions, RoleId, ServerEvent, ServerId, ServerState, UserId};
use commy_net::{invite, WirePayload};
use commy_view::act::Act;
use commy_view::pictures::MAX_SERVER_IMAGE_BYTES;
use commy_view::{manage, moderate, profile, roles, templates};
use serde::Deserialize;
use serde_json::{json, Value};
use wasm_bindgen_futures::spawn_local;

use super::{parse, to_json, Args, Session};
use crate::sync_book::left_record;

#[derive(Deserialize)]
pub(super) struct MoveArg {
    channel: String,
    parent: Option<String>,
    position: u32,
}

fn role_list(raw: Option<Vec<String>>) -> Result<Vec<RoleId>, String> {
    raw.unwrap_or_default().iter().map(|r| r.parse().map_err(|_| "invalid role".to_string())).collect()
}

fn role_form(args: &Args) -> roles::RoleForm {
    roles::RoleForm {
        name: args.name.clone().unwrap_or_default(),
        color: args.color.clone(),
        permissions: args.permissions.clone(),
        tag: args.tag.clone(),
        self_assignable: args.self_assignable,
        hoist: args.hoist,
        mentionable: args.mentionable,
    }
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
            "delete_server" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                self.delete_server(server, args.confirm_name.as_deref().unwrap_or("")).await
            }
            "set_server_image" | "set_channel_banner" | "set_role_banner" => self.set_picture(cmd, args).await,
            "audit_log" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                to_json(&self.read_state(server, |state, me| roles::audit_log(state, state.server.permissions_for(me)))?)
            }
            "export_server_template" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                Ok(Value::String(self.read_state(server, |state, me| templates::export(state, state.server.permissions_for(me)))?))
            }
            _ => self.manage(cmd, args).await,
        }
    }

    async fn set_picture(self: &Rc<Self>, cmd: &str, args: Args) -> Result<Value, String> {
        let server: ServerId = parse(args.server_id.as_ref(), "server")?;
        let which = args.which.clone().unwrap_or_default();
        let (channel, role) = match cmd {
            "set_channel_banner" => (Some(parse::<ChannelId>(args.channel_id.as_ref(), "channel")?), None),
            "set_role_banner" => (None, Some(parse::<RoleId>(args.role_id.as_ref(), "role")?)),
            _ => (None, None),
        };
        let build = |state: &ServerState, perms: Permissions, image: Option<String>| -> Act<ServerEvent> {
            match (channel, role) {
                (Some(channel), _) => manage::channel_banner(state, perms, channel, image),
                (_, Some(role)) => manage::role_banner(state, perms, role, image),
                _ => manage::server_images(state, perms, &which, image),
            }
        };
        self.read_state(server, |state, me| build(state, state.server.permissions_for(me), None).map(|_| ()))?;
        let invalid = match (channel, role) {
            (Some(_), _) => manage::CHANNEL_BANNER_INVALID,
            (_, Some(_)) => manage::ROLE_BANNER_INVALID,
            _ => manage::SERVER_IMAGE_INVALID,
        };
        let image = match args.data.as_deref() {
            None => None,
            Some(data) => {
                let bytes = profile::image_bytes(data, MAX_SERVER_IMAGE_BYTES, invalid, invalid)?;
                if which == "invite" && bytes.len() > invite::MAX_INVITE_BACKGROUND_BYTES {
                    return Err("invite backgrounds must be under 400 KB".into());
                }
                Some(self.store_server_picture(bytes).await?)
            }
        };
        let payload = self.client.borrow().act(server, |state, me, _, _| build(state, me.perms, image))?;
        self.publish(server, &payload);
        self.emit("server-event", json!(server.to_string()));
        if cmd == "set_server_image" {
            self.emit("servers-changed", Value::Null);
        }
        Ok(Value::Null)
    }

    pub(super) async fn store_server_picture(&self, bytes: Vec<u8>) -> Result<String, String> {
        let (blob, provider) = self.store_blob(bytes.clone()).await?;
        if !self.offer_picture(&blob, &bytes).await {
            return Err("Could not save the picture right now, try again in a moment".into());
        }
        Ok(ImageRef { blob, provider }.encode())
    }

    pub(super) fn read_state<T>(&self, server: ServerId, read: impl FnOnce(&ServerState, UserId) -> Result<T, String>) -> Result<T, String> {
        let me = self.client.borrow().profile().ok_or("Your servers are not unlocked yet")?.id;
        let client = self.client.borrow();
        let servers = client.servers.lock().unwrap();
        read(servers.get(&server).ok_or("This server is still loading")?, me)
    }

    async fn manage(self: &Rc<Self>, cmd: &str, args: Args) -> Result<Value, String> {
        let server: ServerId = parse(args.server_id.as_ref(), "server")?;
        let channel = || parse::<ChannelId>(args.channel_id.as_ref(), "channel");
        let user = || parse::<UserId>(args.user_id.as_ref(), "user");
        let role = || parse::<RoleId>(args.role_id.as_ref(), "role");
        let name = args.name.as_deref().unwrap_or("");
        let client = || self.client.borrow();
        let mut reply = Value::Null;
        let payloads = match cmd {
            "rename_server" => vec![client().act(server, |state, me, _, _| manage::rename(state, me.perms, name))?],
            "create_channel" => {
                let kind = args.kind.as_deref().unwrap_or("text");
                let parent = args.parent_id.as_deref().and_then(|p| p.parse::<ChannelId>().ok());
                vec![client().act(server, |state, me, _, _| manage::create_channel(state, me.perms, name, kind, parent))?]
            }
            "create_category" => vec![client().act(server, |state, me, _, _| manage::create_category(state, me.perms, name))?],
            "update_channel" => {
                let channel = channel()?;
                let retention = args.retention_days;
                vec![client().act(server, |state, me, _, _| manage::update_channel(state, me.perms, channel, name, retention))?]
            }
            "delete_channel" => {
                let channel = channel()?;
                vec![client().act(server, |state, me, _, _| manage::delete_channel(state, me.perms, channel))?]
            }
            "set_channel_kind" => {
                let channel = channel()?;
                let kind = args.kind.as_deref().unwrap_or("");
                vec![client().act(server, |state, me, _, _| manage::channel_kind_set(state, me.perms, channel, kind))?]
            }
            "move_channels" => {
                let mut moves = Vec::new();
                for m in args.moves.unwrap_or_default().into_iter().take(manage::MAX_CHANNEL_MOVES) {
                    let parent = m.parent.as_deref().map(str::parse::<ChannelId>).transpose().map_err(|_| "invalid channel".to_string())?;
                    moves.push(manage::ChannelMove { channel: m.channel.parse().map_err(|_| "invalid channel".to_string())?, parent, position: m.position });
                }
                client().act_all(server, |state, me| manage::move_channels(state, me.perms, moves))?
            }
            "create_role" => {
                let form = role_form(&args);
                let mut made = None;
                let payload = client().act(server, |state, me, _, _| {
                    let (id, event) = roles::create_role(state, me.perms, form)?;
                    made = Some(id);
                    Ok(event)
                })?;
                reply = made.map_or(Value::Null, |id| json!(id.to_string()));
                vec![payload]
            }
            "update_role" => {
                let (role, form) = (role()?, role_form(&args));
                vec![client().act(server, |state, me, _, _| roles::update_role(state, me.perms, role, form))?]
            }
            "delete_role" => {
                let role = role()?;
                vec![client().act(server, |state, me, _, _| roles::delete_role(state, me.perms, role))?]
            }
            "set_member_role" => {
                let (user, role, grant) = (user()?, role()?, args.grant.as_ref().and_then(Value::as_bool).unwrap_or(true));
                vec![client().act(server, |state, me, _, _| roles::member_role(state, me.perms, user, role, grant))?]
            }
            "set_own_role" => {
                let (role, held) = (role()?, args.held.unwrap_or(true));
                vec![client().act(server, |state, me, _, _| roles::own_role(state, me.user, role, held))?]
            }
            "set_role_access" => {
                let allowed = role_list(args.role_ids.clone())?;
                let (access, restricted) = (args.access.as_deref().unwrap_or(""), args.restricted.unwrap_or(false));
                client().act_all(server, |state, me| roles::role_access(state, me.perms, access, restricted, &allowed))?
            }
            "kick_member" => {
                let user = user()?;
                vec![client().act(server, |state, me, _, _| moderate::kick(state, me.perms, user))?]
            }
            "move_voice_member" => {
                let user = user()?;
                let to = args.channel_id.as_deref().map(str::parse::<ChannelId>).transpose().map_err(|_| "invalid channel".to_string())?;
                vec![client().act(server, |state, me, _, _| moderate::move_voice(state, me.perms, me.user, user, to))?]
            }
            "ban_member" => {
                let user = user()?;
                vec![client().act(server, |state, me, _, _| moderate::ban(state, me.perms, me.user, user))?]
            }
            "unban_member" => {
                let user = user()?;
                vec![client().act(server, |state, me, _, _| moderate::unban(state, me.perms, user))?]
            }
            "time_out_member" | "clear_timeout" => {
                let user = user()?;
                let seconds = if cmd == "clear_timeout" { None } else { Some(args.seconds.unwrap_or(0)) };
                vec![client().act(server, |state, me, _, _| moderate::timeout(state, me.perms, me.user, user, seconds, me.now))?]
            }
            "set_nickname" => {
                let user = user()?;
                let nickname = args.nickname.as_deref();
                vec![client().act(server, |state, me, _, _| moderate::nickname(state, me.perms, me.user, user, nickname))?]
            }
            "transfer_ownership" => {
                let user = user()?;
                let confirm = args.confirm_name.as_deref().unwrap_or("");
                vec![client().act(server, |state, me, _, _| moderate::transfer(state, me.user, user, confirm))?]
            }
            "set_rules" => {
                let rules = args.rules.as_ref().and_then(Value::as_str);
                vec![client().act(server, |state, me, _, _| moderate::rules(state, me.perms, rules))?]
            }
            "accept_rules" => vec![client().act(server, |state, me, _, _| moderate::accept_rules(state, me.user))?],
            "set_onboarding" => {
                let onboarding = match args.onboarding.clone().filter(|o| !o.is_null()) {
                    Some(raw) => Some(serde_json::from_value(raw).map_err(|_| "invalid onboarding questions".to_string())?),
                    None => None,
                };
                vec![client().act(server, |state, me, _, _| moderate::onboarding(state, me.perms, onboarding))?]
            }
            "answer_onboarding" => {
                let grant: Vec<String> = args.grant.clone().map_or(Ok(Vec::new()), serde_json::from_value).map_err(|_| "invalid roles".to_string())?;
                let (grant, revoke) = (role_list(Some(grant))?, role_list(args.revoke.clone())?);
                client().act_all(server, |state, me| moderate::answer_onboarding(state, me.user, grant, revoke))?
            }
            "set_automod" => {
                let rules = serde_json::from_value(args.rules.clone().unwrap_or_default()).map_err(|_| "invalid auto moderation rules".to_string())?;
                vec![client().act(server, |state, me, _, _| moderate::automod(state, me.perms, rules))?]
            }
            "set_activities_enabled" => {
                let enabled = args.enabled.unwrap_or(false);
                vec![client().act(server, |state, me, _, _| moderate::activities(state, me.perms, enabled))?]
            }
            "set_afk_channel" => {
                let afk = args.channel_id.as_deref().filter(|c| !c.is_empty()).map(str::parse::<ChannelId>).transpose().map_err(|_| "invalid channel".to_string())?;
                vec![client().act(server, |state, me, _, _| moderate::afk_channel(state, me.perms, afk))?]
            }
            "set_server_afk_timeout" => vec![client().act(server, |state, me, _, _| moderate::afk_timeout(state, me.perms, args.minutes))?],
            "set_server_public" => {
                let public = args.public.unwrap_or(false);
                vec![client().act(server, |state, me, _, _| moderate::public(state, me.perms, public))?]
            }
            "set_channel_limit" => {
                let (channel, limit) = (channel()?, args.limit.unwrap_or(0));
                vec![client().act(server, |state, me, _, _| moderate::channel_limit(state, me.perms, channel, limit))?]
            }
            "set_channel_slow_mode" => {
                let (channel, seconds) = (channel()?, args.seconds.unwrap_or(0));
                vec![client().act(server, |state, me, _, _| moderate::slow_mode(state, me.perms, channel, seconds))?]
            }
            "set_channel_color" => {
                let (channel, color) = (channel()?, args.color.as_deref());
                vec![client().act(server, |state, me, _, _| moderate::channel_color(state, me.perms, channel, color))?]
            }
            "set_channel_sensitive" => {
                let (channel, sensitive) = (channel()?, args.sensitive.unwrap_or(false));
                vec![client().act(server, |state, me, _, _| moderate::sensitive(state, me.perms, channel, sensitive))?]
            }
            "set_channel_access" => {
                let channel = channel()?;
                let (seen, send) = (role_list(args.view_roles.clone())?, role_list(args.send_roles.clone())?);
                vec![client().act(server, |state, me, _, _| moderate::channel_access(state, me.perms, channel, seen, send))?]
            }
            _ => return Err(format!("{cmd} is not available in the web app yet")),
        };
        for payload in &payloads {
            self.publish(server, payload);
        }
        self.refresh_gates();
        match cmd {
            "ban_member" | "set_member_role" => self.tend_keys(server, None),
            "set_channel_access" => {
                self.leave_unseen_voice(server).await;
                self.tend_keys(server, Some(KeyScope::Channel(channel()?)));
            }
            _ => {}
        }
        self.emit("server-event", json!(server.to_string()));
        if cmd == "rename_server" {
            wasm_bindgen_futures::spawn_local(self.clone().refresh_listing(server));
        }
        if cmd == "transfer_ownership" {
            self.follow_owner(server);
        }
        if matches!(cmd, "rename_server" | "set_role_access" | "set_server_public" | "transfer_ownership" | "set_activities_enabled") {
            self.emit("servers-changed", Value::Null);
        }
        Ok(reply)
    }

    fn tend_keys(&self, server: ServerId, scope: Option<KeyScope>) {
        let scopes = scope.map_or_else(|| self.client.borrow().key_work(server), |scope| vec![scope]);
        for scope in scopes {
            let payloads = self.client.borrow().tend_scope(server, scope);
            for payload in &payloads {
                self.publish(server, payload);
            }
        }
    }

    async fn leave_unseen_voice(self: &Rc<Self>, server: ServerId) {
        let Some((channel, _)) = self.voice_here(server) else { return };
        let sees = self.read_state(server, |state, me| Ok(state.server.can_view(channel, me))).unwrap_or(true);
        if !sees {
            self.leave_room().await;
            self.emit("voice-left", json!(server.to_string()));
        }
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
        self.drop_from_list(server);
        let group = self.client.borrow().is_group(server);
        self.forget_left(server, group).await;
        Ok(Value::Null)
    }

    async fn delete_server(self: &Rc<Self>, server: ServerId, confirm: &str) -> Result<Value, String> {
        let payload = self.client.borrow().act(server, |state, me, _, _| moderate::delete(state, me.user, confirm))?;
        if self.voice_here(server).is_some() {
            self.hang_up_media();
        }
        let group = self.client.borrow().is_group(server);
        self.client.borrow_mut().forget(server);
        self.publish_stored(server, &payload).await;
        self.tell_neighbors(server, &payload).await;
        self.drop_from_list(server);
        self.forget_left(server, group).await;
        Ok(Value::Null)
    }

    pub(super) async fn tell_neighbors(&self, server: ServerId, payload: &WirePayload) {
        let node = self.node.lock().await;
        if let Some(node) = node.as_ref().filter(|node| node.neighbor_count(server) > 0) {
            let _ = node.broadcast(server, payload.clone()).await;
        }
    }

    fn drop_from_list(&self, server: ServerId) {
        let now = Utc::now();
        if let Some(live) = self.sync.borrow_mut().as_mut() {
            live.book.leave(server, now);
        }
        self.append_account(left_record(server, now));
    }

    pub(super) fn retire_if_gone(self: &Rc<Self>, server: ServerId) {
        if self.client.borrow().gone(server).is_none() {
            return;
        }
        let this = self.clone();
        spawn_local(async move {
            let Some((name, notice)) = this.client.borrow().gone(server) else { return };
            if this.voice_here(server).is_some() {
                this.hang_up_media();
            }
            let group = this.client.borrow().is_group(server);
            this.client.borrow_mut().forget(server);
            this.drop_from_list(server);
            this.forget_left(server, group).await;
            if notice != "left" {
                this.emit(notice, json!({ "server": server.to_string(), "name": name }));
            }
        });
    }

    async fn forget_left(self: &Rc<Self>, server: ServerId, group: bool) {
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
