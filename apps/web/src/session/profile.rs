use std::rc::Rc;

use chrono::Utc;
use commy_core::{Avatar, UserProfile};
use commy_view::pictures::MAX_PROFILE_IMAGE_BYTES;
use commy_view::profile::{self, ProfileEdit};
use commy_view::safety::ProfilePolicy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{to_json, Args, Session};
use crate::account;
use crate::sync_book::profile_record;

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Personal {
    #[serde(default)]
    pub(super) pronouns: Option<String>,
    #[serde(default)]
    pub(super) presence_until: Option<i64>,
    #[serde(default)]
    pub(super) status_until: Option<i64>,
}

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
            "set_pronouns" => {
                let pronouns = profile::pronouns(args.pronouns.as_deref())?;
                let mut personal = self.personal();
                personal.pronouns = pronouns.clone();
                self.save_personal(personal);
                if self.dms.borrow().safety.profile_policy.shows_details_to_servers() {
                    self.send_pronouns(pronouns);
                }
                return Ok(self.voice_settings());
            }
            "set_status_expiry" => {
                let now = Utc::now().timestamp();
                let mut personal = self.personal();
                personal.presence_until = profile::future(args.presence_until, now);
                personal.status_until = profile::future(args.status_until, now);
                let until = personal.presence_until;
                self.save_personal(personal);
                if self.client.borrow().profile().is_some_and(|p| p.presence_until != until) {
                    self.edit_profile(false, |p| p.presence_until = until)?;
                }
                return Ok(self.voice_settings());
            }
            "set_profile_privacy" => {
                let policy = ProfilePolicy::parse(args.policy.as_deref().unwrap_or_default()).ok_or("unknown profile setting")?;
                let changed = {
                    let mut book = self.dms.borrow_mut();
                    let changed = book.safety.profile_policy != policy;
                    book.safety.profile_policy = policy;
                    changed
                };
                if changed {
                    self.save_safety();
                    self.edit_profile(false, |_| {})?;
                    self.send_pronouns(self.personal().pronouns.filter(|_| policy.shows_details_to_servers()));
                }
                return Ok(Value::Null);
            }
            _ => return Err(format!("{cmd} is not available in the web app yet")),
        };
        to_json(&edited)
    }

    pub(super) fn personal(&self) -> Personal {
        let kept = self.voice.borrow().personal.clone();
        kept.unwrap_or_else(|| Personal { pronouns: self.client.borrow().own_pronouns(), ..Personal::default() })
    }

    fn save_personal(&self, personal: Personal) {
        if let (Ok(account), Ok(raw)) = (self.account(), serde_json::to_string(&personal)) {
            self.host.save(&account::personal_key(&account.id), &raw);
        }
        self.voice.borrow_mut().personal = Some(personal);
    }

    fn send_pronouns(&self, pronouns: Option<String>) {
        let sent = self.client.borrow().pronoun_updates(pronouns);
        for (server, payload) in sent {
            self.publish(server, &payload);
        }
        self.emit("servers-changed", Value::Null);
    }

    pub(super) fn expire_status(&self) {
        let mut personal = self.personal();
        let (presence, status) = profile::expired(personal.presence_until, personal.status_until, Utc::now().timestamp());
        if !presence && !status {
            return;
        }
        if presence {
            personal.presence_until = None;
        }
        if status {
            personal.status_until = None;
        }
        self.save_personal(personal);
        let _ = self.edit_profile(false, |p| profile::expire(p, presence, status));
    }

    async fn store_picture(&self, bytes: Vec<u8>) -> Result<String, String> {
        let (blob, _) = self.store_blob(bytes.clone()).await?;
        if !self.offer_picture(&blob, &bytes).await {
            return Err("Could not save the picture right now, try again in a moment".into());
        }
        Ok(blob)
    }

    fn edit_profile(&self, shared: bool, edit: impl FnOnce(&mut UserProfile)) -> Result<UserProfile, String> {
        let shown = self.client.borrow().profile().map(|p| p.presence.as_seen_by_others());
        let edited = self.client.borrow().edit_me(edit)?;
        let details = self.dms.borrow().safety.profile_policy.shows_details_to_servers();
        let mut sent = self.client.borrow().announce_profile(details);
        if shown != Some(edited.presence.as_seen_by_others()) {
            sent.extend(self.client.borrow().presence_now());
        }
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
