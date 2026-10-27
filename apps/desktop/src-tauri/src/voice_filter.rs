use crate::state::App;
use commy_audio::filters::VoiceFilter;
use commy_core::{ServerId, ServerState, UserId};
use std::sync::atomic::Ordering;

pub fn allowed_by(state: Option<&ServerState>, me: UserId) -> bool {
    state.is_none_or(|s| s.server.permissions_for(me).can_use_voice_filters())
}

pub fn sent_filter(chosen: VoiceFilter, allowed: bool) -> VoiceFilter {
    if allowed {
        chosen
    } else {
        VoiceFilter::Off
    }
}

fn allowed_in(app: &App, server: Option<ServerId>) -> bool {
    let (Some(sid), Some(me)) = (server, app.profile.lock().unwrap().as_ref().map(|p| p.id)) else {
        return true;
    };
    allowed_by(app.servers.lock().unwrap().get(&sid), me)
}

pub fn apply_for(app: &App, server: Option<ServerId>) -> Option<bool> {
    let chosen = app.voice.lock().unwrap().voice_filter();
    let allowed = allowed_in(app, server);
    app.processor.lock().unwrap().set_voice_filter(sent_filter(chosen, allowed));
    let held_back = chosen.is_on() && !allowed;
    let was = app.voice_filter_held_back.swap(held_back, Ordering::Relaxed);
    (was != held_back).then_some(held_back)
}

pub fn apply(app: &App) -> Option<bool> {
    let server = app.voice_session.lock().unwrap().as_ref().map(|s| s.server);
    apply_for(app, server)
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Permissions, Server, ServerEvent, UserProfile};

    #[test]
    fn a_server_that_switches_filters_off_sends_the_plain_voice() {
        let owner = UserProfile::new("owner");
        let me = UserProfile::new("me");
        let mut state = ServerState::new(Server::new("Hangout", owner.id));
        let sid = state.server.id;
        state.apply(ServerEvent::MemberJoin { server: sid, profile: me.clone() }).unwrap();
        assert!(allowed_by(Some(&state), me.id));
        assert!(allowed_by(None, me.id), "calls outside a server always allow filters");
        state.server.roles[0].permissions.insert(Permissions::NO_VOICE_FILTERS);
        assert!(!allowed_by(Some(&state), me.id));
        assert!(allowed_by(Some(&state), owner.id));

        let app = App::default();
        *app.profile.lock().unwrap() = Some(me.clone());
        app.servers.lock().unwrap().insert(sid, state);
        app.voice.lock().unwrap().voice_filter = VoiceFilter::Robot.name().to_string();
        assert_eq!(apply_for(&app, Some(sid)), Some(true));
        assert_eq!(app.processor.lock().unwrap().voice_filter(), VoiceFilter::Off);
        assert_eq!(apply_for(&app, Some(sid)), None, "nothing new to say the second time");
        assert_eq!(apply_for(&app, None), Some(false));
        assert_eq!(app.processor.lock().unwrap().voice_filter(), VoiceFilter::Robot);
    }

    #[test]
    fn off_is_never_held_back() {
        assert_eq!(sent_filter(VoiceFilter::Off, false), VoiceFilter::Off);
        assert_eq!(sent_filter(VoiceFilter::Cave, true), VoiceFilter::Cave);
        assert_eq!(sent_filter(VoiceFilter::Cave, false), VoiceFilter::Off);
    }
}
