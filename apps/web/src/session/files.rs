use std::rc::Rc;

use commy_core::dm::DmMessage;
use commy_net::node::BlobFetcher;
use serde_json::Value;

use super::{to_json, Args, Session};
use crate::files;

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
                let bytes = self.fetch_blob(args.blob.as_deref(), args.provider.as_deref(), max).await?;
                Ok(Value::String(files::data_url(&mime, &bytes)))
            }
            _ => Err(format!("{cmd} is not available in the web app yet")),
        }
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
        self.fetch_blob(args.blob.as_deref(), args.provider.as_deref(), max).await
    }

    async fn send_dm_file(self: &Rc<Self>, user_id: Option<&String>, filename: String, bytes: Vec<u8>) -> Result<Value, String> {
        let (me, partner) = self.recipient(user_id)?;
        let size_bytes = bytes.len() as u64;
        let (blob, provider) = {
            let node = self.node.lock().await;
            let node = node.as_ref().ok_or("Your servers are not unlocked yet")?;
            (node.add_blob(bytes).await.map_err(|e| e.to_string())?, node.node_id_hex())
        };
        let mut message = DmMessage::new(me.id, partner, String::new());
        message.attachments.push(files::attachment(filename, size_bytes, blob, provider));
        to_json(&self.post_dm(&me, partner, message).await?.ok_or("file not sent")?)
    }

    async fn fetch_blob(&self, blob: Option<&str>, provider: Option<&str>, max: u64) -> Result<Vec<u8>, String> {
        let blob = blob.filter(|b| !b.is_empty()).ok_or("missing file")?;
        let fetcher: BlobFetcher = {
            let node = self.node.lock().await;
            node.as_ref().ok_or("Your servers are not unlocked yet")?.blob_fetcher()
        };
        fetcher.fetch(blob, provider, max).await.map_err(|e| format!("{e:#}"))
    }
}
