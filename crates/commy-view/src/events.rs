use chrono::{DateTime, Utc};
use commy_core::server::{valid_event, Rsvp, ScheduledEvent};
use commy_core::{ChannelId, Permissions, ServerEvent, ServerState, UserId};

use crate::act::Act;
use crate::manage::need;

pub struct EventForm {
    pub id: Option<String>,
    pub title: String,
    pub description: String,
    pub start: String,
    pub end: Option<String>,
    pub channel: Option<ChannelId>,
}

pub fn parse_time(raw: &str) -> Act<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw).map(|t| t.with_timezone(&Utc)).map_err(|_| "invalid date or time".to_string())
}

pub fn save(state: &ServerState, perms: Permissions, me: UserId, form: EventForm, new_id: impl FnOnce() -> String) -> Act<ServerEvent> {
    let existing = form.id.as_ref().and_then(|id| state.server.events.iter().find(|e| &e.id == id));
    let creator = match existing {
        Some(e) if e.creator == me || perms.allows(Permissions::MANAGE_EVENTS) => e.creator,
        Some(_) => return Err("you can only edit events you created".into()),
        None if perms.can_create_events() => me,
        None => return Err("you do not have permission to create events".into()),
    };
    let event = ScheduledEvent {
        id: form.id.unwrap_or_else(new_id),
        title: form.title.trim().to_string(),
        description: form.description.trim().to_string(),
        start: parse_time(&form.start)?,
        end: form.end.filter(|e| !e.is_empty()).map(|e| parse_time(&e)).transpose()?,
        channel: form.channel,
        creator,
        rsvps: Vec::new(),
    };
    if !valid_event(&event) {
        return Err("events need a title (up to 100 characters), and the end must be after the start".into());
    }
    Ok(ServerEvent::EventUpsert { server: state.server.id, event })
}

pub fn delete(state: &ServerState, perms: Permissions, me: UserId, id: String) -> Act<ServerEvent> {
    let mine = state.server.events.iter().any(|e| e.id == id && e.creator == me);
    if !mine {
        need(perms, Permissions::MANAGE_EVENTS, "delete events")?;
    }
    Ok(ServerEvent::EventDelete { server: state.server.id, event: id })
}

pub fn rsvp(state: &ServerState, me: UserId, id: String, rsvp: Option<&str>) -> Act<ServerEvent> {
    let rsvp = match rsvp {
        None | Some("") => None,
        Some("going") => Some(Rsvp::Going),
        Some("interested") => Some(Rsvp::Interested),
        Some(_) => return Err("unknown RSVP".into()),
    };
    Ok(ServerEvent::EventRsvp { server: state.server.id, event: id, user: me, rsvp })
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::Server;

    fn form(id: Option<&str>) -> EventForm {
        EventForm {
            id: id.map(str::to_string),
            title: "  Game night ".into(),
            description: "bring snacks".into(),
            start: "2026-11-01T19:00:00Z".into(),
            end: Some("2026-11-01T21:00:00Z".into()),
            channel: None,
        }
    }

    #[test]
    fn members_make_events_and_only_the_creator_or_managers_change_them() {
        let owner = UserId::new();
        let member = UserId::new();
        let mut state = ServerState::new(Server::new("Hangout", owner));
        let (mine, theirs) = (state.server.permissions_for(owner), Permissions::default_member());
        let made = save(&state, theirs, member, form(None), || "e1".into()).unwrap();
        let ServerEvent::EventUpsert { event, .. } = made else { panic!() };
        assert_eq!((event.id.as_str(), event.title.as_str(), event.creator), ("e1", "Game night", member));
        state.server.events.push(ScheduledEvent { creator: owner, ..event });
        assert_eq!(save(&state, theirs, member, form(Some("e1")), String::new).unwrap_err(), "you can only edit events you created");
        assert!(save(&state, mine, owner, form(Some("e1")), String::new).is_ok());
        assert!(delete(&state, theirs, member, "e1".into()).is_err());
        assert!(delete(&state, mine, owner, "e1".into()).is_ok());
        let backwards = EventForm { end: Some("2026-11-01T18:00:00Z".into()), ..form(None) };
        assert!(save(&state, mine, owner, backwards, || "e2".into()).is_err());
        assert_eq!(save(&state, mine, owner, EventForm { start: "soon".into(), ..form(None) }, || "e3".into()).unwrap_err(), "invalid date or time");
    }

    #[test]
    fn rsvps_are_going_interested_or_cleared() {
        let owner = UserId::new();
        let state = ServerState::new(Server::new("Hangout", owner));
        assert!(matches!(rsvp(&state, owner, "e1".into(), Some("going")), Ok(ServerEvent::EventRsvp { rsvp: Some(Rsvp::Going), .. })));
        assert!(matches!(rsvp(&state, owner, "e1".into(), None), Ok(ServerEvent::EventRsvp { rsvp: None, .. })));
        assert!(rsvp(&state, owner, "e1".into(), Some("maybe")).is_err());
    }
}
