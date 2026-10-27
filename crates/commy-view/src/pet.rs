use commy_core::pet::{self, Care, Pet, CARE_COOLDOWN_MS};
use commy_core::{Permissions, ServerEvent, ServerState, UserId};
use serde::Serialize;

use crate::act::Act;
use crate::manage::need;

pub const PET_INVALID: &str = "Give your astronaut a name of up to 24 characters";
pub const NO_PET: &str = "This server has no astronaut yet";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct CareTimes {
    pub fed: i64,
    pub patted: i64,
    pub played: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PetView {
    pub name: String,
    pub suit: String,
    pub adopted: i64,
    pub last: CareTimes,
    pub mine: CareTimes,
    pub xp: u32,
    pub level: u32,
    pub next_level_xp: u32,
    pub cooldown_ms: i64,
}

pub fn level_for(xp: u32) -> u32 {
    let mut level = 1;
    while xp_for(level + 1) <= xp {
        level += 1;
    }
    level
}

pub fn xp_for(level: u32) -> u32 {
    let step = level.saturating_sub(1);
    step.saturating_mul(step).saturating_mul(3)
}

pub fn view(state: &ServerState, viewer: Option<UserId>) -> Option<PetView> {
    let pet = state.server.pet.as_ref()?;
    let care = &state.server.pet_care;
    let latest = |kind| care.values().map(|log| log.last(kind)).max().unwrap_or(0);
    let mine = viewer.and_then(|v| care.get(&v)).map(|log| CareTimes { fed: log.fed, patted: log.patted, played: log.played }).unwrap_or_default();
    let xp = pet::xp(care);
    let level = level_for(xp);
    Some(PetView {
        name: pet.name.clone(),
        suit: pet.suit.clone(),
        adopted: pet.adopted,
        last: CareTimes { fed: latest(Care::Feed), patted: latest(Care::Pat), played: latest(Care::Play) },
        mine,
        xp,
        level,
        next_level_xp: xp_for(level + 1),
        cooldown_ms: CARE_COOLDOWN_MS,
    })
}

pub fn adopt(state: &ServerState, perms: Permissions, name: &str, suit: &str, now: i64) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_SERVER, "look after the server pet")?;
    let adopted = state.server.pet.as_ref().map_or(now, |p| p.adopted);
    let pet = Pet::new(name, suit, adopted).ok_or(PET_INVALID)?;
    Ok(ServerEvent::PetSet { server: state.server.id, pet: Some(pet) })
}

pub fn release(state: &ServerState, perms: Permissions) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_SERVER, "look after the server pet")?;
    state.server.pet.as_ref().ok_or(NO_PET)?;
    Ok(ServerEvent::PetSet { server: state.server.id, pet: None })
}

pub fn care(state: &ServerState, me: UserId, kind: Care, at: i64) -> Act<ServerEvent> {
    let pet = state.server.pet.as_ref().ok_or(NO_PET)?;
    if !state.server.is_current_member(me) {
        return Err("You are not in this server".into());
    }
    let last = state.server.pet_care.get(&me).map_or(0, |log| log.last(kind));
    let wait = if last == 0 { 0 } else { last.saturating_add(CARE_COOLDOWN_MS) - at };
    if wait > 0 {
        let minutes = (wait + 59_999) / 60_000;
        return Err(format!("{} needs a little break. Try again in {minutes} minute{}", pet.name, if minutes == 1 { "" } else { "s" }));
    }
    Ok(ServerEvent::PetCare { server: state.server.id, user: me, care: kind, at })
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Server, UserProfile};

    fn fixture() -> (ServerState, UserId) {
        let owner = UserProfile::new("owner");
        let mut state = ServerState::new(Server::new("test", owner.id));
        state.profiles.insert(owner.id, owner.clone());
        (state, owner.id)
    }

    #[test]
    fn levels_get_further_apart() {
        assert_eq!((level_for(0), level_for(2), level_for(3), level_for(11), level_for(12), level_for(27)), (1, 1, 2, 2, 3, 4));
        assert_eq!(xp_for(2), 3);
        assert_eq!(xp_for(5), 48);
    }

    #[test]
    fn owners_adopt_and_rename_and_members_take_turns() {
        let (mut state, owner) = fixture();
        assert!(view(&state, Some(owner)).is_none());
        assert_eq!(care(&state, owner, Care::Feed, 1).unwrap_err(), NO_PET);
        state.apply(adopt(&state, Permissions::all(), "Cosmo", "orange", 50).unwrap()).unwrap();
        let ServerEvent::PetSet { pet: Some(renamed), .. } = adopt(&state, Permissions::all(), "Nova", "blue", 900).unwrap() else { panic!() };
        assert_eq!(renamed.adopted, 50);
        assert!(adopt(&state, Permissions::default_member(), "Nova", "blue", 900).is_err());
        assert_eq!(adopt(&state, Permissions::all(), "", "blue", 900).unwrap_err(), PET_INVALID);
        state.apply(care(&state, owner, Care::Feed, 1_000).unwrap()).unwrap();
        let wait = care(&state, owner, Care::Feed, 1_000 + 60_000).unwrap_err();
        assert!(wait.starts_with("Cosmo needs a little break. Try again in 9 minutes"), "{wait}");
        assert!(care(&state, owner, Care::Play, 1_000 + 60_000).is_ok());
        let shown = view(&state, Some(owner)).unwrap();
        assert_eq!((shown.last.fed, shown.mine.fed, shown.xp, shown.level), (1_000, 1_000, 1, 1));
        assert_eq!(view(&state, None).unwrap().mine, CareTimes::default());
        assert!(release(&state, Permissions::all()).is_ok());
    }
}
