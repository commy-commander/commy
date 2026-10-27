use std::collections::HashMap;

use commy_core::server::ImageRef;
use commy_net::node::is_blob_of;
use commy_sync::picture::MAX_PICTURE_PLAIN;
use commy_sync::PictureKey;
use commy_view::pictures::{self, expression_mime, is_jpeg_or_png, is_profile_picture, picture_mime};
use serde_json::Value;

use super::{Args, Session};
use crate::files;

const KEPT_PICTURES: usize = 512;

#[derive(Default)]
pub(super) struct Kept {
    pictures: HashMap<String, Vec<u8>>,
}

impl Session {
    pub(super) async fn picture_command(&self, cmd: &str, args: Args) -> Result<Value, String> {
        match cmd {
            "get_profile_image" => {
                let blob = args.blob.as_deref().filter(|b| !b.is_empty()).ok_or("missing picture")?;
                let bytes = self.picture(blob, args.provider.as_deref(), pictures::MAX_PROFILE_IMAGE_BYTES, is_profile_picture).await?;
                Ok(Value::String(files::data_url(picture_mime(&bytes), &bytes)))
            }
            "fetch_server_image" => {
                let image = ImageRef::parse(args.image.as_deref().unwrap_or("")).ok_or("bad image reference")?;
                let bytes = self.picture(&image.blob, Some(&image.provider), pictures::MAX_SERVER_IMAGE_BYTES, is_jpeg_or_png).await?;
                Ok(Value::String(files::data_url(picture_mime(&bytes), &bytes)))
            }
            "fetch_expression" => {
                let image = ImageRef::parse(args.blob.as_deref().unwrap_or("")).ok_or("bad reference")?;
                let bytes = self.picture(&image.blob, Some(&image.provider), pictures::MAX_SOUND_BYTES, |b| expression_mime(b).is_some()).await?;
                let mime = expression_mime(&bytes).ok_or("not a valid emoji or sound")?;
                Ok(Value::String(files::data_url(mime, &bytes)))
            }
            _ => Err(format!("{cmd} is not available in the web app yet")),
        }
    }

    async fn picture(&self, blob: &str, provider: Option<&str>, max: usize, valid: fn(&[u8]) -> bool) -> Result<Vec<u8>, String> {
        let kept = self.kept.borrow().pictures.get(blob).filter(|bytes| bytes.len() <= max && valid(bytes)).cloned();
        if let Some(bytes) = kept {
            return Ok(bytes);
        }
        let bytes = match self.shared_picture(blob, max).await.filter(|bytes| valid(bytes)) {
            Some(bytes) => bytes,
            None => {
                let bytes = self.fetch_blob(Some(blob), provider.filter(|p| !p.is_empty()), None, max as u64).await?;
                if !valid(&bytes) {
                    return Err("that is not a valid picture".into());
                }
                bytes
            }
        };
        let mut kept = self.kept.borrow_mut();
        if kept.pictures.len() >= KEPT_PICTURES {
            kept.pictures.clear();
        }
        kept.pictures.insert(blob.to_string(), bytes.clone());
        Ok(bytes)
    }

    pub(super) async fn offer_picture(&self, blob: &str, bytes: &[u8]) -> bool {
        let Some(base) = self.sync.borrow().as_ref().and_then(|live| live.pictures.clone()) else { return false };
        if bytes.len() > MAX_PICTURE_PLAIN || !is_blob_of(blob, bytes) {
            return false;
        }
        let key = PictureKey::for_blob(blob);
        let Ok(sealed) = key.seal(bytes) else { return false };
        let sent = self.host.put_bytes(&format!("{base}/{}", key.id_hex()), &sealed).await;
        sent.ok().and_then(|ok| ok.as_bool()).unwrap_or(false)
    }

    async fn shared_picture(&self, blob: &str, max: usize) -> Option<Vec<u8>> {
        let base = self.sync.borrow().as_ref()?.pictures.clone()?;
        let key = PictureKey::for_blob(blob);
        let body = self.host.fetch_bytes(&format!("{base}/{}", key.id_hex())).await.ok()?;
        if body.is_null() || body.is_undefined() {
            return None;
        }
        let plain = key.open(&js_sys::Uint8Array::new(&body).to_vec()).ok()?;
        (plain.len() <= max && is_blob_of(blob, &plain)).then_some(plain)
    }
}
