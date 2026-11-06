use std::rc::Rc;

use chrono::Utc;
use commy_core::drive::DriveFile;
use commy_core::ids::DriveId;
use commy_core::{ChannelId, ServerId};
use serde_json::{json, Value};

use super::{parse, to_json, Args, Session};
use crate::files;

fn item_id(raw: Option<&String>) -> Result<DriveId, String> {
    raw.and_then(|r| r.parse().ok()).ok_or_else(|| "unknown drive item".to_string())
}

fn parent_id(raw: Option<&String>) -> Result<Option<DriveId>, String> {
    raw.filter(|r| !r.is_empty()).map(|r| item_id(Some(r))).transpose()
}

impl Session {
    pub(super) async fn drive_command(self: &Rc<Self>, cmd: &str, args: Args) -> Result<Value, String> {
        let server: ServerId = parse(args.server_id.as_ref(), "server")?;
        let client = || self.client.borrow();
        let payload = match cmd {
            "drive_view" => {
                let channel: ChannelId = parse(args.channel_id.as_ref(), "channel")?;
                let node = self.blob_node().await;
                return to_json(&self.read_state(server, |state, me| commy_view::drive::view(state, channel, me, node.as_deref()))?);
            }
            "drive_new_folder" => {
                let channel: ChannelId = parse(args.channel_id.as_ref(), "channel")?;
                let parent = parent_id(args.parent_id.as_ref())?;
                let name = args.name.as_deref().unwrap_or("");
                let now = Utc::now().timestamp_millis();
                client().act(server, |state, me, _, _| commy_view::drive::new_folder(state, me.user, channel, parent, name, now))?
            }
            "drive_rename" => {
                let item = item_id(args.item_id.as_ref())?;
                let parent = parent_id(args.parent_id.as_ref())?;
                let name = args.name.as_deref().unwrap_or("");
                client().act(server, |state, me, _, _| commy_view::drive::edit(state, me.user, item, name, parent))?
            }
            "drive_remove" => {
                let item = item_id(args.item_id.as_ref())?;
                client().act(server, |state, me, _, _| commy_view::drive::remove(state, me.user, item))?
            }
            "drive_stop_keeping" => {
                let item = item_id(args.item_id.as_ref())?;
                client().act(server, |state, me, _, _| commy_view::drive::keep(state, me.user, item, None))?
            }
            _ => return Err(format!("{cmd} is not available in the web app yet")),
        };
        self.publish(server, &payload);
        self.emit("server-event", json!(server.to_string()));
        Ok(Value::Null)
    }

    async fn blob_node(&self) -> Option<String> {
        self.node.lock().await.as_ref().map(|node| node.node_id_hex())
    }

    pub async fn drive_upload(self: &Rc<Self>, server_id: &str, channel_id: &str, parent: Option<String>, filename: &str, bytes: Vec<u8>) -> Result<Value, String> {
        let server: ServerId = parse(Some(&server_id.to_string()), "server")?;
        let channel: ChannelId = parse(Some(&channel_id.to_string()), "channel")?;
        let parent = parent_id(parent.as_ref())?;
        files::check_upload(&bytes)?;
        let name = commy_core::drive::clean_name(filename).ok_or(commy_view::drive::BAD_NAME)?;
        self.read_state(server, |state, me| {
            commy_view::drive::view(state, channel, me, None).and_then(|v| if v.can_upload { Ok(()) } else { Err("you do not have permission to upload files here".into()) })
        })?;
        let mime = mime_guess::from_path(&name).first_raw().unwrap_or("application/octet-stream").to_string();
        let size = bytes.len() as u64;
        let (blob, node) = self.store_blob(bytes).await?;
        let file = DriveFile { blob, size, mime };
        let now = Utc::now().timestamp_millis();
        let payload = self.client.borrow().act(server, |state, me, _, _| commy_view::drive::add_file(state, me.user, channel, parent, &name, file, &node, now))?;
        self.publish(server, &payload);
        self.emit("server-event", json!(server.to_string()));
        Ok(Value::Bool(true))
    }

    pub(super) async fn drive_bytes(&self, args: &Args) -> Result<Vec<u8>, String> {
        let server: ServerId = parse(args.server_id.as_ref(), "server")?;
        let item = item_id(args.item_id.as_ref())?;
        let id = item.to_string();
        let (blob, size, sources) = self.read_state(server, |state, me| {
            let found = commy_core::drive::find(&state.server.drive, item).ok_or(commy_view::drive::NO_SUCH_ITEM)?;
            let file = found.file.as_ref().ok_or("that is a folder")?;
            if !state.server.can_view(found.channel, me) {
                return Err(commy_view::drive::NO_SUCH_ITEM.into());
            }
            let view = commy_view::drive::view(state, found.channel, me, None)?;
            let sources = view.items.into_iter().find(|i| i.id == id).map(|i| i.sources).unwrap_or_default();
            Ok((file.blob.clone(), file.size, sources))
        })?;
        let (max, refusal) = files::byte_limit("download_attachment").ok_or("downloads are not available")?;
        files::fits(Some(size), max, refusal)?;
        let mut last = None;
        for source in sources.iter().map(Some).chain([None]) {
            match self.fetch_blob(Some(&blob), source.map(String::as_str), Some(&server.to_string()), max).await {
                Ok(bytes) => return Ok(bytes),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| "nobody who has this file is online right now".into()))
    }
}
