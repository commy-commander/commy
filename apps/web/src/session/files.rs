use std::rc::Rc;

use chrono::Utc;
use commy_core::dm::DmMessage;
use commy_core::{Attachment, ChannelId, Permissions, ServerId};
use commy_net::node::BlobFetcher;
use serde_json::Value;

use super::{parse, to_json, Args, Session};
use crate::files;
use crate::recording::Recorder;

impl Session {
    pub(super) async fn file_command(self: &Rc<Self>, cmd: &str, args: Args) -> Result<Value, String> {
        match cmd {
            "attach_dm_bytes" => {
                let filename = files::upload_name(args.filename.as_deref().unwrap_or(""))?;
                let bytes = files::upload_bytes(args.data.as_deref().unwrap_or(""))?;
                self.send_dm_file(args.user_id.as_ref(), filename, bytes).await
            }
            "get_attachment_preview" => {
                let mime = args.mime.unwrap_or_default();
                if !files::is_image_mime(&mime) {
                    return Err("not an image".into());
                }
                let max = files::preview_limit(args.clicked.unwrap_or(false));
                files::fits(args.size_bytes, max, "image too large to preview")?;
                let (blob, provider, server) = (args.blob.as_deref(), args.provider.as_deref(), args.server_id.as_deref());
                let bytes = match args.shared.unwrap_or(false) {
                    true => self.shared_preview(blob, provider, server, max).await?,
                    false => self.fetch_blob(blob, provider, server, max).await?,
                };
                Ok(Value::String(files::data_url(&mime, &bytes)))
            }
            "attach_bytes" => {
                let filename = files::upload_name(args.filename.as_deref().unwrap_or(""))?;
                let bytes = files::upload_bytes(args.data.as_deref().unwrap_or(""))?;
                let (server, channel) = server_channel(&args)?;
                self.send_server_file(server, channel, filename, bytes).await
            }
            "start_voice_recording" => self.start_recording().await.map(|_| Value::Null),
            "cancel_voice_recording" => {
                self.stop_recording();
                Ok(Value::Null)
            }
            "send_voice_message" => {
                let (server, channel) = server_channel(&args)?;
                let clip = self.take_recording()?;
                let size_bytes = clip.len() as u64;
                let (blob, provider) = self.store_blob(clip).await?;
                let attachment = Attachment::voice_message(blob, size_bytes, Some(provider));
                self.post_attachment(server, channel, attachment, Permissions::SEND_VOICE_MESSAGES, "send voice messages here").await
            }
            "send_dm_voice_message" => {
                let (me, partner) = self.recipient(args.user_id.as_ref())?;
                let clip = self.take_recording()?;
                let size_bytes = clip.len() as u64;
                let (blob, provider) = self.store_blob(clip).await?;
                let mut message = DmMessage::new(me.id, partner, String::new());
                message.attachments.push(Attachment::voice_message(blob, size_bytes, Some(provider)));
                to_json(&self.post_dm(&me, partner, message).await?.ok_or("voice message not sent")?)
            }
            _ => Err(format!("{cmd} is not available in the web app yet")),
        }
    }

    pub async fn attach_server(self: &Rc<Self>, server_id: &str, channel_id: &str, filename: &str, bytes: Vec<u8>) -> Result<Value, String> {
        let server: ServerId = parse(Some(&server_id.to_string()), "server")?;
        let channel: ChannelId = parse(Some(&channel_id.to_string()), "channel")?;
        let filename = files::upload_name(filename)?;
        files::check_upload(&bytes)?;
        self.send_server_file(server, channel, filename, bytes).await
    }

    async fn send_server_file(self: &Rc<Self>, server: ServerId, channel: ChannelId, filename: String, bytes: Vec<u8>) -> Result<Value, String> {
        let size_bytes = bytes.len() as u64;
        self.client.borrow().upload_fits(server, size_bytes)?;
        let (blob, provider) = self.store_blob(bytes).await?;
        let attachment = files::attachment(filename, size_bytes, blob, provider);
        self.post_attachment(server, channel, attachment, Permissions::ATTACH_FILES, "send files or images here").await
    }

    async fn post_attachment(self: &Rc<Self>, server: ServerId, channel: ChannelId, attachment: Attachment, needed: Permissions, action: &str) -> Result<Value, String> {
        let (message, view) = self.client.borrow().compose_attachment(server, channel, attachment, needed, action)?;
        self.deliver(server, message, view).await
    }

    pub(super) async fn store_blob(&self, bytes: Vec<u8>) -> Result<(String, String), String> {
        let node = self.node.lock().await;
        let node = node.as_ref().ok_or("Your servers are not unlocked yet")?;
        Ok((node.add_blob(bytes).await.map_err(|e| e.to_string())?, node.node_id_hex()))
    }

    async fn start_recording(&self) -> Result<(), String> {
        let serial = Utc::now().timestamp_subsec_nanos();
        let mic_running = self.voice_link_open();
        self.set_recorder(Some(Recorder::new(serial)));
        if mic_running {
            return Ok(());
        }
        let started = self.host.start_voice().await.ok().and_then(|v| v.as_bool()).unwrap_or(false);
        if !started {
            self.set_recorder(None);
            return Err("Commy needs the microphone to record a voice message".into());
        }
        Ok(())
    }

    fn stop_recording(&self) -> Option<Recorder> {
        let recorder = self.set_recorder(None);
        if !self.voice_link_open() {
            self.host.stop_voice();
        }
        recorder
    }

    fn take_recording(&self) -> Result<Vec<u8>, String> {
        self.stop_recording().ok_or("no voice message is being recorded")?.finish()
    }

    pub async fn attach_dm(self: &Rc<Self>, user_id: &str, filename: &str, bytes: Vec<u8>) -> Result<Value, String> {
        let filename = files::upload_name(filename)?;
        files::check_upload(&bytes)?;
        self.send_dm_file(Some(&user_id.to_string()), filename, bytes).await
    }

    pub async fn file_bytes(self: &Rc<Self>, cmd: &str, args: &str) -> Result<Vec<u8>, String> {
        let args: Args = serde_json::from_str(args).unwrap_or_default();
        let (max, refusal) = files::byte_limit(cmd).ok_or_else(|| format!("{cmd} is not available in the web app yet"))?;
        files::fits(args.size_bytes, max, refusal)?;
        self.fetch_blob(args.blob.as_deref(), args.provider.as_deref(), args.server_id.as_deref(), max).await
    }

    async fn send_dm_file(self: &Rc<Self>, user_id: Option<&String>, filename: String, bytes: Vec<u8>) -> Result<Value, String> {
        let (me, partner) = self.recipient(user_id)?;
        let size_bytes = bytes.len() as u64;
        let (blob, provider) = self.store_blob(bytes).await?;
        let mut message = DmMessage::new(me.id, partner, String::new());
        message.attachments.push(files::attachment(filename, size_bytes, blob, provider));
        to_json(&self.post_dm(&me, partner, message).await?.ok_or("file not sent")?)
    }

    pub(super) async fn fetch_blob(&self, blob: Option<&str>, provider: Option<&str>, server: Option<&str>, max: u64) -> Result<Vec<u8>, String> {
        let blob = blob.filter(|b| !b.is_empty()).ok_or("missing file")?;
        let server = server.and_then(|s| s.parse::<ServerId>().ok());
        let members = server.and_then(|s| self.client.borrow().servers.lock().unwrap().get(&s).map(|state| state.member_nodes())).unwrap_or_default();
        let (fetcher, sources): (BlobFetcher, Vec<String>) = {
            let node = self.node.lock().await;
            let node = node.as_ref().ok_or("Your servers are not unlocked yet")?;
            let neighbors = server.map(|s| node.neighbors(s)).unwrap_or_default();
            (node.blob_fetcher(), commy_net::node::blob_sources(provider.unwrap_or_default(), neighbors))
        };
        fetcher.fetch_among(blob, sources.iter().map(String::as_str), &members, max).await.map_err(|e| format!("{e:#}"))
    }
}

fn server_channel(args: &Args) -> Result<(ServerId, ChannelId), String> {
    Ok((parse(args.server_id.as_ref(), "server")?, parse(args.channel_id.as_ref(), "channel")?))
}
