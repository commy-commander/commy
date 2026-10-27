use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::ids::UserId;

pub const MAX_PET_NAME: usize = 24;
pub const CARE_COOLDOWN_MS: i64 = 10 * 60 * 1000;
pub const MAX_CARE_COUNTED: u32 = 50;
pub const SUITS: &[&str] = &["white", "orange", "blue", "pink", "green", "purple"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pet {
    pub name: String,
    pub suit: String,
    pub adopted: i64,
}

impl Pet {
    pub fn new(name: &str, suit: &str, adopted: i64) -> Option<Self> {
        let pet = Pet { name: name.trim().to_string(), suit: suit.to_string(), adopted };
        pet.is_valid().then_some(pet)
    }

    pub fn is_valid(&self) -> bool {
        let count = self.name.chars().count();
        (1..=MAX_PET_NAME).contains(&count)
            && self.name.trim() == self.name
            && !self.name.chars().any(char::is_control)
            && SUITS.contains(&self.suit.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Care {
    Feed,
    Pat,
    Play,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CareLog {
    #[serde(default)]
    pub fed: i64,
    #[serde(default)]
    pub patted: i64,
    #[serde(default)]
    pub played: i64,
    #[serde(default)]
    pub count: u32,
}

impl CareLog {
    pub fn last(&self, care: Care) -> i64 {
        match care {
            Care::Feed => self.fed,
            Care::Pat => self.patted,
            Care::Play => self.played,
        }
    }

    fn last_mut(&mut self, care: Care) -> &mut i64 {
        match care {
            Care::Feed => &mut self.fed,
            Care::Pat => &mut self.patted,
            Care::Play => &mut self.played,
        }
    }
}

pub fn care_name(care: Care) -> &'static str {
    match care {
        Care::Feed => "feed",
        Care::Pat => "pat",
        Care::Play => "play",
    }
}

pub fn set(pet: &mut Option<Pet>, care: &mut BTreeMap<UserId, CareLog>, next: Option<Pet>) -> bool {
    if next.as_ref().is_some_and(|p| !p.is_valid()) || *pet == next {
        return false;
    }
    let same_pet = matches!((&*pet, &next), (Some(a), Some(b)) if a.adopted == b.adopted);
    if !same_pet {
        care.clear();
    }
    *pet = next;
    true
}

pub fn tend(care: &mut BTreeMap<UserId, CareLog>, user: UserId, kind: Care, at: i64) -> bool {
    let log = care.entry(user).or_default();
    let last = log.last(kind);
    if last != 0 && at < last.saturating_add(CARE_COOLDOWN_MS) {
        return false;
    }
    *log.last_mut(kind) = at;
    log.count = log.count.saturating_add(1);
    true
}

pub fn xp(care: &BTreeMap<UserId, CareLog>) -> u32 {
    care.values().map(|log| log.count.min(MAX_CARE_COUNTED)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pet_has_a_short_clean_name_and_a_known_suit() {
        assert_eq!(Pet::new("  Cosmo ", "orange", 1).map(|p| p.name), Some("Cosmo".into()));
        assert!(Pet::new("", "orange", 1).is_none());
        assert!(Pet::new(&"a".repeat(MAX_PET_NAME + 1), "orange", 1).is_none());
        assert!(Pet::new("Co\u{7}smo", "orange", 1).is_none());
        assert!(Pet::new("Cosmo", "gold", 1).is_none());
        assert!(!Pet { name: " Cosmo".into(), suit: "blue".into(), adopted: 1 }.is_valid());
    }

    #[test]
    fn each_member_can_care_once_per_cooldown_for_each_kind() {
        let mut care = BTreeMap::new();
        let user = UserId::new();
        assert!(tend(&mut care, user, Care::Feed, 1_000));
        assert!(!tend(&mut care, user, Care::Feed, 1_000 + CARE_COOLDOWN_MS - 1));
        assert!(tend(&mut care, user, Care::Pat, 1_000));
        assert!(tend(&mut care, user, Care::Feed, 1_000 + CARE_COOLDOWN_MS));
        assert_eq!(care[&user].count, 3);
        assert_eq!(xp(&care), 3);
    }

    #[test]
    fn nobody_can_level_the_pet_up_alone() {
        let mut care = BTreeMap::new();
        let user = UserId::new();
        for i in 0..200 {
            tend(&mut care, user, Care::Play, i * CARE_COOLDOWN_MS + 1);
        }
        assert_eq!(xp(&care), MAX_CARE_COUNTED);
    }

    #[test]
    fn a_new_pet_starts_fresh_and_a_rename_keeps_its_memories() {
        let mut pet = None;
        let mut care = BTreeMap::new();
        assert!(set(&mut pet, &mut care, Pet::new("Cosmo", "white", 5)));
        tend(&mut care, UserId::new(), Care::Feed, 10);
        assert!(set(&mut pet, &mut care, Pet::new("Nova", "blue", 5)));
        assert_eq!(care.len(), 1);
        assert!(!set(&mut pet, &mut care, Pet::new("Nova", "blue", 5)));
        assert!(set(&mut pet, &mut care, Pet::new("Nova", "blue", 9)));
        assert!(care.is_empty());
        let forged = Some(Pet { name: "x".into(), suit: "gold".into(), adopted: 1 });
        assert!(!set(&mut pet, &mut care, forged));
        assert!(set(&mut pet, &mut care, None));
    }
}
