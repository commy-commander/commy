use commy_core::{DmMessage, Message, VoiceInvite};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct VoiceInviteView {
    pub server: String,
    pub channel: String,
    pub invite_link: Option<String>,
    pub server_name: String,
    pub channel_name: String,
}

impl VoiceInviteView {
    pub fn of(invite: VoiceInvite) -> Self {
        Self {
            server: invite.server.to_string(),
            channel: invite.channel.to_string(),
            invite_link: invite.invite_link,
            server_name: invite.server_name,
            channel_name: invite.channel_name,
        }
    }
}

pub fn excerpt(invite: &VoiceInvite) -> String {
    if invite.channel_name.is_empty() {
        "Invite to a voice call".to_string()
    } else {
        format!("Invite to a call in {}", invite.channel_name)
    }
}

pub fn dm_view_of(message: &DmMessage) -> Option<VoiceInviteView> {
    message.voice_invite().map(VoiceInviteView::of)
}

pub fn view_of(message: &Message) -> Option<VoiceInviteView> {
    message.voice_invite().map(VoiceInviteView::of)
}
