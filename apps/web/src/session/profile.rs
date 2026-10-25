use std::rc::Rc;

use chrono::Utc;
use commy_core::{Avatar, UserProfile};
use commy_view::pictures::MAX_PROFILE_IMAGE_BYTES;
use commy_view::profile::{self, ProfileEdit};
use serde_json::{json, Value};

use super::{to_json, Args, Session};
use crate::sync_book::profile_record;

impl Session {
    pub(super) async fn profile_command(self: &Rc<Self>, cmd: &str, args: Args) -> Result<Value, String> {
        let edited = match cmd {
            "update_profile" => {
                let edit = ProfileEdit {
                    display_name: profile::display_name(args.display_name),
                    custom_status: profile::status_text(args.custom_status),
                    presence: profile::presence(args.presence.as_deref().unwrap_or_default())?,
                    status_emoji: profile::status_emoji(args.status_emoji)?,
                };
                self.edit_profile(true, |p| profile::apply_edit(p, edit))?
            }
            "set_about" => {
                let about = profile::about(args.about.as_deref().unwrap_or_default())?;
                self.edit_profile(false, |p| p.about = (!about.is_empty()).then_some(about))?
            }
            "set_avatar" => {
                let avatar = match args.kind.as_deref() {
                    Some("none") => None,
                    Some("emoji") => Some(profile::emoji_avatar(args.data)?),
                    Some("image") => {
                        let bytes = profile::image_bytes(args.data.as_deref().unwrap_or_default(), MAX_PROFILE_IMAGE_BYTES, "avatar image is too large", "avatar must be a JPEG or PNG image")?;
                        Some(Avatar::Image { blob: self.store_picture(bytes).await? })
                    }
                    _ => return Err("unknown avatar kind".into()),
                };
                self.edit_profile(true, |p| p.avatar = avatar.map(|a| a.encode()))?
            }
            "set_profile_banner" => {
                let banner = match args.data.as_deref() {
                    None => None,
                    Some(data) => {
                        let bytes = profile::image_bytes(data, MAX_PROFILE_IMAGE_BYTES, "banner image is too large", "banner must be a JPEG or PNG image")?;
                        Some(Avatar::Image { blob: self.store_picture(bytes).await? }.encode())
                    }
                };
                self.edit_profile(false, |p| p.banner = banner)?
            }
            _ => return Err(format!("{cmd} is not available in the web app yet")),
        };
        to_json(&edited)
    }

    async fn store_picture(&self, bytes: Vec<u8>) -> Result<String, String> {
        let (blob, _) = self.store_blob(bytes.clone()).await?;
        if !self.offer_picture(&blob, &bytes).await {
            return Err("Could not save the picture right now, try again in a moment".into());
        }
        Ok(blob)
    }

    fn edit_profile(&self, shared: bool, edit: impl FnOnce(&mut UserProfile)) -> Result<UserProfile, String> {
        let edited = self.client.borrow().edit_me(edit)?;
        let details = self.dms.borrow().safety.profile_policy.shows_details_to_servers();
        let sent = self.client.borrow().announce_profile(details);
        for (server, payload) in sent {
            self.publish(server, &payload);
            self.emit("server-event", json!(server.to_string()));
        }
        if shared {
            self.share_profile(&edited);
        }
        self.emit("profile-changed", json!(edited));
        Ok(edited)
    }

    fn share_profile(&self, edited: &UserProfile) {
        let kept = profile::for_list(edited);
        let at = Utc::now();
        match self.sync.borrow_mut().as_mut() {
            Some(live) => live.book.note_profile(kept.clone(), at),
            None => return,
        }
        self.append_account(profile_record(&kept, at));
    }

    pub(super) fn adopt_shared_profile(&self) {
        let Some(shared) = self.sync.borrow().as_ref().and_then(|live| live.book.profile().cloned()) else { return };
        let taken = self.client.borrow().edit_me_if(|p| profile::take_shared_fields(p, &shared));
        if let Some(taken) = taken {
            self.emit("profile-changed", json!(taken));
        }
    }
}
