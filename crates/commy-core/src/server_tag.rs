use serde::{Deserialize, Serialize};

use crate::ids::{ServerId, UserId};
use crate::server::Server;

pub const MAX_TAG_LETTERS: usize = 4;
pub const MAX_TAG_LETTER_BYTES: usize = 16;
pub const MAX_WORN_NAME_CHARS: usize = 32;
pub const TAG_COLORS: [&str; 9] = ["red", "orange", "yellow", "green", "teal", "blue", "purple", "pink", "gray"];
pub const TAG_EFFECTS: [&str; 4] = ["none", "shine", "glow", "pulse"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerTag {
    pub letters: String,
    pub emoji: Option<String>,
    pub color: String,
    pub effect: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WornTag {
    pub server: ServerId,
    pub name: String,
    pub tag: ServerTag,
}

pub fn clean_letters(raw: &str) -> Option<String> {
    let letters: String = raw.trim().to_uppercase();
    let count = letters.chars().count();
    let fits = (1..=MAX_TAG_LETTERS).contains(&count) && letters.len() <= MAX_TAG_LETTER_BYTES;
    (fits && letters.chars().all(char::is_alphanumeric)).then_some(letters)
}

impl ServerTag {
    pub fn new(letters: &str, emoji: Option<&str>, color: &str, effect: &str) -> Option<Self> {
        let tag = Self {
            letters: clean_letters(letters)?,
            emoji: emoji.map(str::trim).filter(|e| !e.is_empty()).map(str::to_string),
            color: color.to_string(),
            effect: effect.to_string(),
        };
        tag.is_valid().then_some(tag)
    }

    pub fn is_valid(&self) -> bool {
        clean_letters(&self.letters).as_deref() == Some(self.letters.as_str())
            && self.emoji.as_deref().is_none_or(crate::user::is_emoji_like)
            && TAG_COLORS.contains(&self.color.as_str())
            && TAG_EFFECTS.contains(&self.effect.as_str())
    }
}

impl WornTag {
    pub fn of(server: &Server) -> Option<Self> {
        let tag = server.tag.clone().filter(ServerTag::is_valid)?;
        Some(Self { server: server.id, name: worn_name(&server.name), tag })
    }

    pub fn is_valid(&self) -> bool {
        self.tag.is_valid() && self.name.chars().count() <= MAX_WORN_NAME_CHARS
    }

    pub fn refreshed(&self, server: Option<&Server>, wearer: UserId) -> Option<WornTag> {
        let server = server.filter(|s| s.id == self.server && s.is_member(wearer))?;
        WornTag::of(server)
    }

    pub fn shown_in(&self, server: &Server, wearer: UserId) -> Option<ServerTag> {
        if self.server != server.id {
            return self.is_valid().then(|| self.tag.clone());
        }
        server.tag.clone().filter(|t| t.is_valid() && server.is_member(wearer))
    }
}

fn worn_name(name: &str) -> String {
    name.trim().chars().take(MAX_WORN_NAME_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(letters: &str) -> ServerTag {
        ServerTag::new(letters, Some("\u{1f680}"), "blue", "shine").unwrap()
    }

    #[test]
    fn letters_are_one_to_four_letters_or_digits_in_capitals() {
        assert_eq!(clean_letters(" gg "), Some("GG".into()));
        assert_eq!(clean_letters("ab12"), Some("AB12".into()));
        assert_eq!(clean_letters("\u{e9}t\u{e9}"), Some("\u{c9}T\u{c9}".into()));
        assert_eq!(clean_letters(""), None);
        assert_eq!(clean_letters("abcde"), None);
        assert_eq!(clean_letters("a b"), None);
        assert_eq!(clean_letters("a-b"), None);
        assert_eq!(clean_letters("\u{1f680}"), None);
    }

    #[test]
    fn a_tag_needs_a_known_colour_and_effect_and_a_real_emoji() {
        assert!(tag("rkt").is_valid());
        assert_eq!(tag("rkt").letters, "RKT");
        assert!(ServerTag::new("ok", None, "teal", "none").is_some());
        assert!(ServerTag::new("ok", Some("  "), "teal", "none").unwrap().emoji.is_none());
        assert!(ServerTag::new("ok", None, "chartreuse", "none").is_none());
        assert!(ServerTag::new("ok", None, "teal", "sparkle").is_none());
        assert!(ServerTag::new("ok", Some("abc"), "teal", "none").is_none());
        let forged = ServerTag { letters: "lower".into(), ..tag("ok") };
        assert!(!forged.is_valid());
    }

    #[test]
    fn a_worn_tag_follows_the_server_and_drops_when_the_wearer_leaves() {
        let owner = UserId::new();
        let mut server = Server::new("Rocket Club for people who like rockets a lot", owner);
        assert_eq!(WornTag::of(&server), None);
        server.tag = Some(tag("rkt"));
        let worn = WornTag::of(&server).unwrap();
        assert_eq!(worn.name.chars().count(), MAX_WORN_NAME_CHARS);
        assert!(worn.is_valid());
        assert_eq!(worn.refreshed(Some(&server), owner), Some(worn.clone()));
        server.tag = Some(tag("rc"));
        assert_eq!(worn.refreshed(Some(&server), owner).unwrap().tag.letters, "RC");
        assert_eq!(worn.refreshed(Some(&server), UserId::new()), None);
        assert_eq!(worn.refreshed(None, owner), None);
        server.tag = None;
        assert_eq!(worn.refreshed(Some(&server), owner), None);
    }

    #[test]
    fn the_tags_own_server_shows_its_current_tag_and_others_show_the_copy() {
        let owner = UserId::new();
        let mut home = Server::new("Home", owner);
        home.tag = Some(tag("old"));
        let worn = WornTag::of(&home).unwrap();
        home.tag = Some(tag("new"));
        assert_eq!(worn.shown_in(&home, owner).unwrap().letters, "NEW");
        assert_eq!(worn.shown_in(&home, UserId::new()), None);
        let elsewhere = Server::new("Elsewhere", owner);
        assert_eq!(worn.shown_in(&elsewhere, owner).unwrap().letters, "OLD");
        let broken = WornTag { tag: ServerTag { color: "neon".into(), ..tag("x") }, ..worn };
        assert_eq!(broken.shown_in(&elsewhere, owner), None);
        home.tag = None;
        let copy = WornTag { server: home.id, ..broken };
        assert_eq!(copy.shown_in(&home, owner), None);
    }
}
