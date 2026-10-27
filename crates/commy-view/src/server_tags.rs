use commy_core::{Permissions, ServerEvent, ServerState, ServerTag, UserId, WornTag};
use serde::Serialize;

use crate::act::Act;
use crate::manage::need;

pub const TAG_INVALID: &str = "A tag is 1 to 4 letters or numbers, with an optional emoji";
pub const NO_TAG: &str = "This server has no tag yet";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerTagView {
    pub letters: String,
    pub emoji: Option<String>,
    pub color: String,
    pub effect: String,
    pub server: String,
}

pub fn tag_view(tag: &ServerTag, server: &str) -> ServerTagView {
    ServerTagView {
        letters: tag.letters.clone(),
        emoji: tag.emoji.clone(),
        color: tag.color.clone(),
        effect: tag.effect.clone(),
        server: server.to_string(),
    }
}

pub fn set_tag(state: &ServerState, perms: Permissions, letters: Option<&str>, emoji: Option<&str>, color: &str, effect: &str) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_SERVER, "change the server tag")?;
    let tag = match letters.map(str::trim).filter(|l| !l.is_empty()) {
        None => None,
        Some(letters) => Some(ServerTag::new(letters, emoji, color, effect).ok_or(TAG_INVALID)?),
    };
    Ok(ServerEvent::ServerTagSet { server: state.server.id, tag })
}

pub fn wear(state: &ServerState, me: UserId) -> Act<WornTag> {
    if !state.server.is_member(me) {
        return Err("You are not in this server".into());
    }
    WornTag::of(&state.server).ok_or_else(|| NO_TAG.into())
}

pub fn shown(worn: Option<&WornTag>, state: &ServerState, wearer: UserId) -> Option<ServerTagView> {
    let worn = worn?;
    let tag = worn.shown_in(&state.server, wearer)?;
    let server = if worn.server == state.server.id { WornTag::of(&state.server).map(|w| w.name)? } else { worn.name.clone() };
    Some(tag_view(&tag, &server))
}

pub fn shown_anywhere(worn: Option<&WornTag>) -> Option<ServerTagView> {
    worn.filter(|w| w.is_valid()).map(|w| tag_view(&w.tag, &w.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Server, UserProfile};

    fn state_with(owner: &UserProfile, name: &str) -> ServerState {
        let mut state = ServerState::new(Server::new(name, owner.id));
        state.profiles.insert(owner.id, owner.clone());
        state
    }

    #[test]
    fn only_managers_set_the_tag_and_empty_letters_remove_it() {
        let owner = UserProfile::new("owner");
        let state = state_with(&owner, "Rockets");
        let event = set_tag(&state, Permissions::all(), Some("rkt"), Some("\u{1f680}"), "blue", "glow").unwrap();
        let ServerEvent::ServerTagSet { tag: Some(tag), .. } = event else { panic!("expected a tag") };
        assert_eq!(tag.letters, "RKT");
        assert!(matches!(set_tag(&state, Permissions::all(), Some("  "), None, "blue", "none"), Ok(ServerEvent::ServerTagSet { tag: None, .. })));
        assert_eq!(set_tag(&state, Permissions::all(), Some("toolong"), None, "blue", "none"), Err(TAG_INVALID.into()));
        assert!(set_tag(&state, Permissions::NONE, Some("rkt"), None, "blue", "none").is_err());
    }

    #[test]
    fn members_wear_the_current_tag_and_everyone_sees_it_checked_against_its_server() {
        let owner = UserProfile::new("owner");
        let mut home = state_with(&owner, "Rockets");
        assert_eq!(wear(&home, owner.id), Err(NO_TAG.into()));
        assert!(wear(&home, UserId::new()).is_err());
        home.server.tag = ServerTag::new("rkt", None, "teal", "shine");
        let worn = wear(&home, owner.id).unwrap();
        let view = shown(Some(&worn), &home, owner.id).unwrap();
        assert_eq!((view.letters.as_str(), view.server.as_str(), view.effect.as_str()), ("RKT", "Rockets", "shine"));
        home.server.name = "Rocket Club".into();
        assert_eq!(shown(Some(&worn), &home, owner.id).unwrap().server, "Rocket Club");
        let elsewhere = state_with(&owner, "Elsewhere");
        assert_eq!(shown(Some(&worn), &elsewhere, owner.id).unwrap().server, "Rockets");
        assert_eq!(shown_anywhere(Some(&worn)).unwrap().letters, "RKT");
        home.server.tag = None;
        assert_eq!(shown(Some(&worn), &home, owner.id), None);
        assert_eq!(shown(None, &elsewhere, owner.id), None);
    }
}
