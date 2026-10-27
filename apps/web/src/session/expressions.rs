use std::collections::HashMap;
use std::rc::Rc;

use commy_core::{Action, ServerEvent, ServerId, ServerState};
use commy_view::catalog;
use commy_view::expressions::{self, NewSound, Picture};
use serde_json::{json, Value};

use super::{parse, to_json, Args, Session};
use crate::files;

const MAX_CATALOG_IMAGE_BYTES: usize = 4 * 1024 * 1024;

fn upload(data: Option<&str>) -> Result<Vec<u8>, String> {
    files::upload_bytes(data.unwrap_or(""))
}

impl Session {
    pub(super) async fn expression_command(self: &Rc<Self>, cmd: &str, args: Args) -> Result<Value, String> {
        match cmd {
            "list_shareable_expressions" => to_json(&self.with_servers(|servers| expressions::shareable(servers.values()))),
            "list_shareable_sounds" => to_json(&self.with_servers(|servers| expressions::shareable_sounds(servers.values()))),
            "catalog_media_path" => {
                let url = args.url.as_deref().unwrap_or("");
                let source = catalog::source(url).ok_or("only emoji.gg and KLIPY images can be added")?;
                Ok(json!({ "source": source, "max": MAX_CATALOG_IMAGE_BYTES }))
            }
            "store_catalog_emoji" | "store_catalog_sticker" => {
                let kind = if cmd == "store_catalog_emoji" { Picture::Emoji } else { Picture::Sticker };
                let name = expressions::name(args.name.as_deref().unwrap_or(""))?;
                let bytes = upload(args.data.as_deref())?;
                kind.check(&bytes)?;
                let image = self.store_server_picture(bytes).await?;
                Ok(Value::String(kind.token(&name, &image)))
            }
            "play_soundboard" | "play_shared_sound" => self.play_sound(cmd, args).await,
            _ => self.change_expressions(cmd, args).await,
        }
    }

    fn with_servers<T>(&self, read: impl FnOnce(&HashMap<ServerId, ServerState>) -> T) -> T {
        let client = self.client.borrow();
        let servers = client.servers.lock().unwrap();
        read(&servers)
    }

    async fn change_expressions(self: &Rc<Self>, cmd: &str, args: Args) -> Result<Value, String> {
        let server: ServerId = parse(args.server_id.as_ref(), "server")?;
        let name = args.name.clone().unwrap_or_default();
        let emoji = args.emoji.as_deref();
        let client = || self.client.borrow();
        let payload = match cmd {
            "add_emoji" | "add_sticker" => {
                let kind = if cmd == "add_emoji" { Picture::Emoji } else { Picture::Sticker };
                self.read_state(server, |state, me| expressions::add_picture(state, state.server.permissions_for(me), me, kind, &name, String::new()).map(|_| ()))?;
                let bytes = upload(args.data.as_deref())?;
                kind.check(&bytes)?;
                let blob = self.store_server_picture(bytes).await?;
                client().act(server, |state, me, _, _| expressions::add_picture(state, me.perms, me.user, kind, &name, blob))?
            }
            "add_sound" => {
                let sound = |blob: String| NewSound { name: &name, blob, volume: args.volume, emoji };
                self.read_state(server, |state, me| expressions::add_sound(state, state.server.permissions_for(me), me, sound(String::new())).map(|_| ()))?;
                let bytes = upload(args.data.as_deref())?;
                expressions::check_sound(&bytes)?;
                let blob = self.store_server_picture(bytes).await?;
                client().act(server, |state, me, _, _| expressions::add_sound(state, me.perms, me.user, sound(blob)))?
            }
            "add_emoticon" => {
                let text = args.text.as_deref().unwrap_or("");
                client().act(server, |state, me, _, _| expressions::add_emoticon(state, me.perms, me.user, &name, text))?
            }
            "remove_emoji" | "remove_sticker" | "remove_emoticon" | "remove_sound" => {
                let kind = cmd.trim_start_matches("remove_");
                client().act(server, |state, me, _, _| expressions::remove(state, me.perms, kind, name))?
            }
            "set_sound_emoji" => client().act(server, |state, me, _, _| expressions::set_sound_emoji(state, me.perms, &name, emoji))?,
            "set_expression_sharing" => {
                let (stickers, emoticons) = (args.stickers.unwrap_or(false), args.emoticons.unwrap_or(false));
                client().act(server, |state, me, _, _| expressions::sharing(state, me.perms, stickers, emoticons))?
            }
            "set_expression_settings" => {
                let (emojis, open) = (args.emojis_shareable.unwrap_or(false), args.soundboard_open.unwrap_or(false));
                client().act(server, |state, me, _, _| expressions::settings(state, me.perms, emojis, open))?
            }
            "set_sound_sharing" => {
                let shareable = args.shareable.unwrap_or(false);
                client().act(server, |state, me, _, _| expressions::sound_sharing(state, me.perms, shareable))?
            }
            "set_join_sounds_allowed" => {
                let allowed = args.enabled.unwrap_or(true);
                client().act(server, |state, me, _, _| expressions::join_sounds_allowed(state, me.perms, allowed))?
            }
            "set_join_sound" => {
                let sound = args.name.as_deref();
                client().act(server, |state, me, _, _| expressions::join_sound(state, me.perms, me.user, sound))?
            }
            _ => return Err(format!("{cmd} is not available in the web app yet")),
        };
        self.publish(server, &payload);
        self.emit("server-event", json!(server.to_string()));
        if matches!(cmd, "set_expression_sharing" | "set_expression_settings" | "set_sound_sharing" | "set_join_sounds_allowed") {
            self.emit("servers-changed", Value::Null);
        }
        Ok(Value::Null)
    }

    async fn play_sound(self: &Rc<Self>, cmd: &str, args: Args) -> Result<Value, String> {
        let server: ServerId = parse(args.server_id.as_ref(), "server")?;
        let name = args.name.clone().unwrap_or_default();
        let (channel, _) = self.voice_here(server).ok_or("join a voice channel in this server first")?;
        let from = match cmd {
            "play_shared_sound" => {
                let source: ServerId = parse(args.source_server_id.as_ref(), "server")?;
                Some(self.read_state(source, |state, _| Ok(state.server.clone()))?)
            }
            _ => None,
        };
        let mut played = None;
        let payload = self.client.borrow().act(server, |state, me, _, _| {
            let (event, view) = match &from {
                Some(from) => expressions::play_shared(state, me.perms, me.user, channel, from, name)?,
                None => expressions::play(state, me.perms, me.user, channel, name, me.now)?,
            };
            played = Some(view);
            Ok(event)
        })?;
        self.tell_neighbors(server, &payload).await;
        if let Some(view) = played {
            self.emit("soundboard-play", to_json(&view)?);
        }
        Ok(Value::Null)
    }

    pub(super) fn hear_sound(&self, server: ServerId, event: &ServerEvent, action: Option<Action>) -> bool {
        if !matches!(event, ServerEvent::SoundboardPlay { .. } | ServerEvent::SharedSoundPlay { .. }) {
            return false;
        }
        let Some((channel, _)) = self.voice_here(server) else { return true };
        if !self.client.borrow().admits(server, event, action) {
            return true;
        }
        let heard = self.read_state(server, |state, _| Ok(expressions::heard(state, channel, event))).ok().flatten();
        if let Some(view) = heard.and_then(|view| to_json(&view).ok()) {
            self.emit("soundboard-play", view);
        }
        true
    }
}
