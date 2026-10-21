use crate::ids::{ChannelId, UserId};
use crate::message::{Attachment, Message};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const COMMAND_MIME: &str = "application/vnd.commy.command+json";
pub const MAX_COMMANDS: usize = 25;
pub const MAX_NAME_CHARS: usize = 32;
pub const MAX_DESCRIPTION_CHARS: usize = 100;
pub const MAX_OPTIONS: usize = 10;
pub const MAX_VALUE_CHARS: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionKind {
    String,
    Integer,
    User,
    Channel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOption {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub required: bool,
    pub kind: OptionKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotCommand {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub options: Vec<CommandOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandUse {
    pub bot: UserId,
    pub command: String,
    #[serde(default)]
    pub options: BTreeMap<String, String>,
}

pub fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=MAX_NAME_CHARS).contains(&bytes.len())
        && bytes.iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
}

fn valid_description(text: &str) -> bool {
    let count = text.chars().count();
    (1..=MAX_DESCRIPTION_CHARS).contains(&count) && !text.chars().any(char::is_control)
}

impl CommandOption {
    pub fn valid(&self) -> bool {
        valid_name(&self.name) && valid_description(&self.description)
    }
}

impl BotCommand {
    pub fn valid(&self) -> bool {
        valid_name(&self.name)
            && valid_description(&self.description)
            && self.options.len() <= MAX_OPTIONS
            && self.options.iter().all(CommandOption::valid)
            && unique(self.options.iter().map(|o| o.name.as_str()))
            && self.options.windows(2).all(|pair| pair[0].required || !pair[1].required)
    }

    pub fn option(&self, name: &str) -> Option<&CommandOption> {
        self.options.iter().find(|o| o.name == name)
    }
}

fn unique<'a>(names: impl Iterator<Item = &'a str>) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    names.into_iter().all(|n| seen.insert(n))
}

pub fn valid_commands(commands: &[BotCommand]) -> bool {
    commands.len() <= MAX_COMMANDS && commands.iter().all(BotCommand::valid) && unique(commands.iter().map(|c| c.name.as_str()))
}

impl CommandUse {
    pub fn new(bot: UserId, command: &str) -> Self {
        Self { bot, command: command.to_string(), options: BTreeMap::new() }
    }

    pub fn with(mut self, name: &str, value: impl Into<String>) -> Self {
        self.options.insert(name.to_string(), value.into());
        self
    }

    pub fn tidy(mut self) -> Option<Self> {
        if !valid_name(&self.command) {
            return None;
        }
        self.options.retain(|_, value| !value.trim().is_empty());
        let sized = self.options.iter().all(|(name, value)| valid_name(name) && value.chars().count() <= MAX_VALUE_CHARS);
        (sized && self.options.len() <= MAX_OPTIONS).then_some(self)
    }

    pub fn fits(&self, command: &BotCommand) -> bool {
        self.command == command.name
            && command.options.iter().filter(|o| o.required).all(|o| self.options.contains_key(&o.name))
            && self.options.iter().all(|(name, value)| command.option(name).is_some_and(|o| o.accepts(value)))
    }

    pub fn into_attachment(self) -> Attachment {
        Attachment {
            blob: String::new(),
            filename: serde_json::to_string(&self).expect("plain strings encode"),
            size_bytes: 0,
            mime_type: COMMAND_MIME.to_string(),
            provider: None,
        }
    }
}

impl CommandOption {
    pub fn accepts(&self, value: &str) -> bool {
        match self.kind {
            OptionKind::String => true,
            OptionKind::Integer => value.parse::<i64>().is_ok(),
            OptionKind::User => value.parse::<UserId>().is_ok(),
            OptionKind::Channel => value.parse::<ChannelId>().is_ok(),
        }
    }
}

impl Attachment {
    pub fn is_command(&self) -> bool {
        self.mime_type == COMMAND_MIME
    }

    pub fn command_use(&self) -> Option<CommandUse> {
        if !self.is_command() {
            return None;
        }
        serde_json::from_str::<CommandUse>(&self.filename).ok()?.tidy()
    }
}

impl Message {
    pub fn command_use(&self) -> Option<CommandUse> {
        self.attachments.iter().find_map(Attachment::command_use)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ChannelId;

    fn option(name: &str, kind: OptionKind, required: bool) -> CommandOption {
        CommandOption { name: name.into(), description: format!("the {name}"), required, kind }
    }

    fn remind() -> BotCommand {
        BotCommand {
            name: "remind".into(),
            description: "Remind someone later".into(),
            options: vec![option("who", OptionKind::User, true), option("minutes", OptionKind::Integer, true), option("note", OptionKind::String, false)],
        }
    }

    #[test]
    fn names_are_short_lowercase_ascii_with_dashes() {
        assert!(valid_name("roll"));
        assert!(valid_name("roll-d20"));
        assert!(!valid_name(""));
        assert!(!valid_name("Roll"));
        assert!(!valid_name("-roll"));
        assert!(!valid_name("roll-"));
        assert!(!valid_name("roll dice"));
        assert!(!valid_name(&"a".repeat(MAX_NAME_CHARS + 1)));
        assert!(valid_name(&"a".repeat(MAX_NAME_CHARS)));
    }

    #[test]
    fn command_lists_are_bounded_and_unique() {
        assert!(valid_commands(&[remind()]));
        assert!(!valid_commands(&[remind(), remind()]), "names repeat");
        let many: Vec<BotCommand> = (0..=MAX_COMMANDS).map(|i| BotCommand { name: format!("c{i}"), description: "d".into(), options: vec![] }).collect();
        assert!(!valid_commands(&many));
        let mut long = remind();
        long.description = "x".repeat(MAX_DESCRIPTION_CHARS + 1);
        assert!(!long.valid());
        let mut crowded = remind();
        crowded.options = (0..=MAX_OPTIONS).map(|i| option(&format!("o{i}"), OptionKind::String, false)).collect();
        assert!(!crowded.valid());
        let mut gap = remind();
        gap.options = vec![option("a", OptionKind::String, false), option("b", OptionKind::String, true)];
        assert!(!gap.valid(), "required options come first");
        let mut twice = remind();
        twice.options = vec![option("a", OptionKind::String, true), option("a", OptionKind::String, false)];
        assert!(!twice.valid());
    }

    #[test]
    fn a_use_marker_rides_as_an_attachment_and_is_checked_against_the_command() {
        let bot = UserId::new();
        let who = UserId::new();
        let used = CommandUse::new(bot, "remind").with("who", who.to_string()).with("minutes", "10").with("note", "  ");
        let mut message = Message::new(ChannelId::new(), UserId::new(), "/remind @nova 10");
        message.attachments.push(used.clone().into_attachment());
        let back = message.command_use().unwrap();
        assert_eq!(back.options.len(), 2, "blank values drop");
        assert!(back.fits(&remind()));
        assert!(message.attachments[0].is_card());
        assert!(!message.attachments[0].carries_a_file());
        assert!(!CommandUse::new(bot, "remind").with("who", who.to_string()).fits(&remind()), "minutes is required");
        assert!(!CommandUse::new(bot, "remind").with("who", "nova").with("minutes", "10").fits(&remind()), "a user is an id");
        assert!(!CommandUse::new(bot, "remind").with("who", who.to_string()).with("minutes", "ten").fits(&remind()));
        assert!(!CommandUse::new(bot, "remind").with("who", who.to_string()).with("minutes", "10").with("extra", "x").fits(&remind()));
        assert!(CommandUse::new(bot, "Remind").tidy().is_none());
        let plain = Message::new(ChannelId::new(), UserId::new(), "/remind");
        assert!(plain.command_use().is_none());
    }
}
