use chrono::{DateTime, Utc};
use commy_core::server::{self, CustomEmoji, Emoticon, ImageRef, SoundboardSound, Sticker};
use commy_core::{ChannelId, Permissions, Server, ServerEvent, ServerId, ServerState, UserId};
use serde::Serialize;

use crate::act::{not_timed_out, Act};
use crate::manage::need;
use crate::pictures::{audio_mime, image_mime, MAX_EMOJI_BYTES, MAX_SOUND_BYTES, MAX_STICKER_BYTES};

pub const SOUND_INVALID: &str = "sounds must be an MP3, OGG or WAV under 1 MB";
pub const DEFAULT_SOUND_PREFIX: &str = "default:";

pub struct DefaultSound {
    pub id: &'static str,
    pub label: &'static str,
}

pub const DEFAULT_SOUNDS: &[DefaultSound] = &[
    DefaultSound { id: "airhorn", label: "Airhorn" },
    DefaultSound { id: "ba_dum_tss", label: "Ba Dum Tss" },
    DefaultSound { id: "cricket", label: "Crickets" },
    DefaultSound { id: "losing_horn", label: "Losing Horn" },
];

pub fn is_default_sound(name: &str) -> bool {
    name.strip_prefix(DEFAULT_SOUND_PREFIX).is_some_and(|id| DEFAULT_SOUNDS.iter().any(|s| s.id == id))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Picture {
    Emoji,
    Sticker,
}

impl Picture {
    fn noun(self) -> &'static str {
        match self {
            Picture::Emoji => "emoji",
            Picture::Sticker => "sticker",
        }
    }

    pub fn invalid(self) -> &'static str {
        match self {
            Picture::Emoji => "emojis must be a PNG, GIF, JPEG or WebP under 256 KB",
            Picture::Sticker => "stickers must be a PNG, GIF, JPEG or WebP under 512 KB",
        }
    }

    pub fn check(self, bytes: &[u8]) -> Act<()> {
        let max = match self {
            Picture::Emoji => MAX_EMOJI_BYTES,
            Picture::Sticker => MAX_STICKER_BYTES,
        };
        if bytes.len() > max || image_mime(bytes).is_none() {
            return Err(self.invalid().into());
        }
        Ok(())
    }

    pub fn token(self, name: &str, image: &str) -> String {
        match self {
            Picture::Emoji => server::emoji_token(name, image),
            Picture::Sticker => server::sticker_token(name, image),
        }
    }
}

pub fn name(raw: &str) -> Act<String> {
    let name = raw.trim().to_ascii_lowercase();
    if server::valid_expression_name(&name) {
        Ok(name)
    } else {
        Err("names must be 2 to 32 letters, numbers or underscores".into())
    }
}

fn can_add(state: &ServerState, perms: Permissions, me: UserId, kind: Permissions, taken: bool, noun: &str) -> Act<()> {
    if !state.server.is_member(me) || !perms.can_add(kind) {
        return Err(format!("you can't add {noun}s in this server"));
    }
    if taken && !perms.allows(Permissions::MANAGE_EXPRESSIONS) {
        return Err(format!("a {noun} with that name already exists"));
    }
    Ok(())
}

fn room(replacing: bool, count: usize, max: usize, plural: &str) -> Act<()> {
    if !replacing && count >= max {
        return Err(format!("servers can have up to {max} {plural}"));
    }
    Ok(())
}

pub fn add_picture(state: &ServerState, perms: Permissions, me: UserId, kind: Picture, raw_name: &str, blob: String) -> Act<ServerEvent> {
    let name = name(raw_name)?;
    let sid = state.server.id;
    match kind {
        Picture::Emoji => {
            let replacing = state.server.emojis.iter().any(|e| e.name == name);
            can_add(state, perms, me, Permissions::ADD_EMOJIS, replacing, kind.noun())?;
            room(replacing, state.server.emojis.len(), server::MAX_EMOJIS, "emojis")?;
            Ok(ServerEvent::EmojiAdd { server: sid, emoji: CustomEmoji { name, blob } })
        }
        Picture::Sticker => {
            let replacing = state.server.stickers.iter().any(|s| s.name == name);
            can_add(state, perms, me, Permissions::ADD_STICKERS, replacing, kind.noun())?;
            room(replacing, state.server.stickers.len(), server::MAX_STICKERS, "stickers")?;
            Ok(ServerEvent::StickerAdd { server: sid, sticker: Sticker { name, blob } })
        }
    }
}

pub fn add_emoticon(state: &ServerState, perms: Permissions, me: UserId, raw_name: &str, text: &str) -> Act<ServerEvent> {
    let name = name(raw_name)?;
    let text = server::clean_emoticon(text)
        .ok_or_else(|| format!("emoticons are one line of up to {} characters", server::MAX_EMOTICON_CHARS))?;
    let replacing = state.server.emoticons.iter().any(|e| e.name == name);
    can_add(state, perms, me, Permissions::ADD_EMOTICONS, replacing, "emoticon")?;
    room(replacing, state.server.emoticons.len(), server::MAX_EMOTICONS, "emoticons")?;
    Ok(ServerEvent::EmoticonAdd { server: state.server.id, emoticon: Emoticon { name, text } })
}

pub fn remove(state: &ServerState, perms: Permissions, kind: &str, name: String) -> Act<ServerEvent> {
    let server = state.server.id;
    let (action, event) = match kind {
        "emoji" => ("manage emojis", ServerEvent::EmojiRemove { server, name }),
        "sticker" => ("remove stickers", ServerEvent::StickerRemove { server, name }),
        "emoticon" => ("remove emoticons", ServerEvent::EmoticonRemove { server, name }),
        "sound" => ("remove sounds", ServerEvent::SoundRemove { server, name }),
        _ => return Err("unknown expression".into()),
    };
    need(perms, Permissions::MANAGE_EXPRESSIONS, action)?;
    Ok(event)
}

pub fn sharing(state: &ServerState, perms: Permissions, stickers: bool, emoticons: bool) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_SERVER, "change sharing")?;
    Ok(ServerEvent::ExpressionSharingSet { server: state.server.id, stickers, emoticons })
}

pub fn settings(state: &ServerState, perms: Permissions, emojis_shareable: bool, soundboard_open: bool) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_SERVER, "change expression settings")?;
    Ok(ServerEvent::ExpressionSettings { server: state.server.id, emojis_shareable, soundboard_open })
}

pub fn sound_sharing(state: &ServerState, perms: Permissions, shareable: bool) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_SERVER, "change sound sharing")?;
    Ok(ServerEvent::SoundSharingSet { server: state.server.id, shareable })
}

pub fn check_sound(bytes: &[u8]) -> Act<()> {
    if bytes.len() > MAX_SOUND_BYTES || audio_mime(bytes).is_none() {
        return Err(SOUND_INVALID.into());
    }
    Ok(())
}

pub fn sound_emoji(emoji: Option<&str>) -> Act<Option<String>> {
    match emoji.map(str::trim).filter(|e| !e.is_empty()) {
        None => Ok(None),
        Some(e) if server::valid_sound_emoji(e) => Ok(Some(e.to_string())),
        Some(_) => Err("pick a single emoji for the sound".into()),
    }
}

pub struct NewSound<'a> {
    pub name: &'a str,
    pub blob: String,
    pub volume: Option<f32>,
    pub emoji: Option<&'a str>,
}

pub fn add_sound(state: &ServerState, perms: Permissions, me: UserId, sound: NewSound) -> Act<ServerEvent> {
    let name = name(sound.name)?;
    let emoji = sound_emoji(sound.emoji)?;
    let manages = perms.allows(Permissions::MANAGE_EXPRESSIONS);
    if !manages && !(state.server.soundboard_open && state.server.is_member(me)) {
        return Err("you do not have permission to add sounds".into());
    }
    let existing = state.server.soundboard.iter().any(|s| s.name == name);
    if existing && !manages {
        return Err("a sound with that name already exists".into());
    }
    room(existing, state.server.soundboard.len(), server::MAX_SOUNDS, "sounds")?;
    let volume = sound.volume.unwrap_or(1.0).clamp(0.0, 2.0);
    Ok(ServerEvent::SoundAdd { server: state.server.id, sound: SoundboardSound { name, blob: sound.blob, volume, emoji } })
}

pub fn set_sound_emoji(state: &ServerState, perms: Permissions, name: &str, emoji: Option<&str>) -> Act<ServerEvent> {
    let emoji = sound_emoji(emoji)?;
    need(perms, Permissions::MANAGE_EXPRESSIONS, "change sounds")?;
    let mut sound = state.server.soundboard.iter().find(|s| s.name == name).cloned().ok_or("unknown sound")?;
    sound.emoji = emoji;
    Ok(ServerEvent::SoundAdd { server: state.server.id, sound })
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SoundPlay {
    pub server: String,
    pub channel: String,
    pub user: String,
    pub name: String,
    pub blob: String,
    pub volume: f32,
}

pub fn play_view(state: &ServerState, channel: ChannelId, user: UserId, name: &str) -> Option<SoundPlay> {
    let (name, blob, volume) = if is_default_sound(name) {
        (name.to_string(), name.to_string(), 1.0)
    } else {
        let sound = state.server.soundboard.iter().find(|s| s.name == name)?;
        (sound.name.clone(), sound.blob.clone(), sound.volume)
    };
    Some(SoundPlay { server: state.server.id.to_string(), channel: channel.to_string(), user: user.to_string(), name, blob, volume })
}

pub fn shared_play_view(server: ServerId, channel: ChannelId, user: UserId, name: &str, blob: &str, volume: f32) -> Option<SoundPlay> {
    ImageRef::parse(blob)?;
    Some(SoundPlay {
        server: server.to_string(),
        channel: channel.to_string(),
        user: user.to_string(),
        name: name.to_string(),
        blob: blob.to_string(),
        volume: volume.clamp(0.0, 2.0),
    })
}

pub fn play(state: &ServerState, perms: Permissions, me: UserId, channel: ChannelId, name: String, now: DateTime<Utc>) -> Act<(ServerEvent, SoundPlay)> {
    need(perms, Permissions::USE_SOUNDBOARD, "use the soundboard")?;
    not_timed_out(state, me, now)?;
    let view = play_view(state, channel, me, &name).ok_or("that sound no longer exists")?;
    Ok((ServerEvent::SoundboardPlay { server: state.server.id, channel, user: me, name }, view))
}

pub fn play_shared(
    state: &ServerState,
    perms: Permissions,
    me: UserId,
    channel: ChannelId,
    from: &Server,
    name: String,
) -> Act<(ServerEvent, SoundPlay)> {
    need(perms, Permissions::USE_SOUNDBOARD, "use the soundboard")?;
    if !from.sounds_shareable {
        return Err("that server does not share its sounds".into());
    }
    let sound = from.soundboard.iter().find(|s| s.name == name).ok_or("that sound no longer exists")?;
    let (blob, volume) = (sound.blob.clone(), sound.volume);
    let server = state.server.id;
    let view = shared_play_view(server, channel, me, &name, &blob, volume).ok_or("that sound cannot be played")?;
    Ok((ServerEvent::SharedSoundPlay { server, channel, user: me, name, blob, volume }, view))
}

pub fn heard(state: &ServerState, listening: ChannelId, event: &ServerEvent) -> Option<SoundPlay> {
    let (channel, user) = match event {
        ServerEvent::SoundboardPlay { channel, user, .. } | ServerEvent::SharedSoundPlay { channel, user, .. } => (*channel, *user),
        _ => return None,
    };
    let allowed = channel == listening
        && state.voice_states.get(&user) == Some(&channel)
        && state.server.permissions_for(user).allows(Permissions::USE_SOUNDBOARD);
    if !allowed {
        return None;
    }
    match event {
        ServerEvent::SoundboardPlay { name, .. } => play_view(state, channel, user, name),
        ServerEvent::SharedSoundPlay { server, name, blob, volume, .. } => shared_play_view(*server, channel, user, name, blob, *volume),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SharedImage {
    pub server: String,
    pub server_name: String,
    pub name: String,
    pub image: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SharedEmoticon {
    pub server: String,
    pub server_name: String,
    pub name: String,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct Shared {
    pub emojis: Vec<SharedImage>,
    pub stickers: Vec<SharedImage>,
    pub emoticons: Vec<SharedEmoticon>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SharedSound {
    pub server: String,
    pub server_name: String,
    pub name: String,
    pub blob: String,
    pub volume: f32,
    pub emoji: Option<String>,
}

fn sorted<T>(mut items: Vec<T>, key: impl Fn(&T) -> (&str, &str)) -> Vec<T> {
    items.sort_by(|a, b| key(a).cmp(&key(b)));
    items
}

pub fn shareable<'a>(servers: impl Iterator<Item = &'a ServerState>) -> Shared {
    let (mut emojis, mut stickers, mut emoticons) = (Vec::new(), Vec::new(), Vec::new());
    for s in servers {
        let (id, server_name) = (s.server.id.to_string(), s.server.name.clone());
        let image = |name: &str, image: &str| SharedImage {
            server: id.clone(),
            server_name: server_name.clone(),
            name: name.to_string(),
            image: image.to_string(),
        };
        if s.server.emojis_shareable {
            emojis.extend(s.server.emojis.iter().map(|e| image(&e.name, &e.blob)));
        }
        if s.server.stickers_shareable {
            stickers.extend(s.server.stickers.iter().map(|e| image(&e.name, &e.blob)));
        }
        if s.server.emoticons_shareable {
            emoticons.extend(s.server.emoticons.iter().map(|e| SharedEmoticon {
                server: id.clone(),
                server_name: server_name.clone(),
                name: e.name.clone(),
                text: e.text.clone(),
            }));
        }
    }
    Shared {
        emojis: sorted(emojis, |e| (&e.server_name, &e.name)),
        stickers: sorted(stickers, |e| (&e.server_name, &e.name)),
        emoticons: sorted(emoticons, |e| (&e.server_name, &e.name)),
    }
}

pub fn shareable_sounds<'a>(servers: impl Iterator<Item = &'a ServerState>) -> Vec<SharedSound> {
    let sounds = servers
        .filter(|s| s.server.sounds_shareable)
        .flat_map(|s| {
            s.server.soundboard.iter().map(|sound| SharedSound {
                server: s.server.id.to_string(),
                server_name: s.server.name.clone(),
                name: sound.name.clone(),
                blob: sound.blob.clone(),
                volume: sound.volume,
                emoji: sound.emoji.clone(),
            })
        })
        .collect();
    sorted(sounds, |s| (&s.server_name, &s.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hangout() -> (ServerState, UserId, UserId) {
        let owner = UserId::new();
        let member = UserId::new();
        let mut state = ServerState::new(Server::new("Hangout", owner));
        state.server.members.push(commy_core::server::Member { user: member, nickname: None, roles: vec![], joined_at: Utc::now(), timeout: None });
        (state, owner, member)
    }

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n0000";

    #[test]
    fn default_sounds_need_the_prefix_and_a_known_id() {
        assert!(is_default_sound("default:airhorn"));
        assert!(is_default_sound("default:losing_horn"));
        assert!(!is_default_sound("airhorn"));
        assert!(!is_default_sound("default:"));
        assert!(!is_default_sound("default:../../app"));
    }

    #[test]
    fn names_are_cleaned_and_checked() {
        assert_eq!(name("  Party_Cat ").unwrap(), "party_cat");
        assert!(name("x").is_err());
        assert!(name("no spaces").is_err());
    }

    #[test]
    fn sound_emoji_input_is_trimmed_and_checked() {
        assert_eq!(sound_emoji(None), Ok(None));
        assert_eq!(sound_emoji(Some("  ")), Ok(None));
        assert_eq!(sound_emoji(Some(" 🥁 ")), Ok(Some("🥁".into())));
        assert!(sound_emoji(Some("drum")).is_err());
    }

    #[test]
    fn pictures_must_be_small_images() {
        assert!(Picture::Emoji.check(PNG).is_ok());
        assert!(Picture::Emoji.check(b"hello").is_err());
        let mut big = PNG.to_vec();
        big.resize(MAX_EMOJI_BYTES + 1, 0);
        assert!(Picture::Emoji.check(&big).is_err());
        assert!(Picture::Sticker.check(&big).is_ok());
        assert_eq!(Picture::Emoji.token("cat", "img"), "<:cat:img>");
        assert_eq!(Picture::Sticker.token("cat", "img"), "<sticker:cat:img>");
    }

    #[test]
    fn members_add_but_only_managers_replace_or_remove() {
        let (mut state, owner, member) = hangout();
        let (mine, theirs) = (state.server.permissions_for(owner), state.server.permissions_for(member));
        let event = add_picture(&state, theirs, member, Picture::Emoji, "Cat", "b".into()).unwrap();
        assert!(matches!(&event, ServerEvent::EmojiAdd { emoji, .. } if emoji.name == "cat"));
        state.server.emojis.push(CustomEmoji { name: "cat".into(), blob: "b".into() });
        assert_eq!(add_picture(&state, theirs, member, Picture::Emoji, "cat", "c".into()).unwrap_err(), "a emoji with that name already exists");
        assert!(add_picture(&state, mine, owner, Picture::Emoji, "cat", "c".into()).is_ok());
        assert!(add_picture(&state, theirs, UserId::new(), Picture::Sticker, "dog", "c".into()).is_err());
        assert!(remove(&state, theirs, "emoji", "cat".into()).is_err());
        assert!(matches!(remove(&state, mine, "sticker", "cat".into()), Ok(ServerEvent::StickerRemove { .. })));
        assert!(remove(&state, mine, "gif", "cat".into()).is_err());
        assert!(add_emoticon(&state, theirs, member, "shrug", "¯\\_(ツ)_/¯").is_ok());
        assert!(add_emoticon(&state, theirs, member, "shrug", "two\nlines").is_err());
    }

    #[test]
    fn sounds_follow_the_open_soundboard_and_play_only_with_permission() {
        let (mut state, owner, member) = hangout();
        let (mine, theirs) = (state.server.permissions_for(owner), state.server.permissions_for(member));
        let boop = || NewSound { name: "boop", blob: "b".into(), volume: Some(5.0), emoji: None };
        assert!(add_sound(&state, theirs, member, boop()).is_err());
        state.server.soundboard_open = true;
        assert!(add_sound(&state, theirs, member, boop()).is_ok());
        let ServerEvent::SoundAdd { sound, .. } = add_sound(&state, mine, owner, boop()).unwrap() else { panic!() };
        assert_eq!(sound.volume, 2.0);
        state.server.soundboard.push(sound);
        assert!(set_sound_emoji(&state, theirs, "boop", None).is_err());
        assert!(set_sound_emoji(&state, mine, "nope", None).is_err());
        let channel = ChannelId::new();
        let (event, view) = play(&state, mine, owner, channel, "boop".into(), Utc::now()).unwrap();
        assert_eq!(view.blob, "b");
        assert!(heard(&state, channel, &event).is_none());
        state.voice_states.insert(owner, channel);
        assert_eq!(heard(&state, channel, &event), Some(view));
        assert!(heard(&state, ChannelId::new(), &event).is_none());
        assert!(play(&state, mine, owner, channel, "default:airhorn".into(), Utc::now()).is_ok());
        assert!(play(&state, mine, owner, channel, "gone".into(), Utc::now()).is_err());
        let from = state.server.clone();
        assert!(play_shared(&state, mine, owner, channel, &from, "boop".into()).is_err());
        let from = Server { sounds_shareable: true, ..from };
        assert!(play_shared(&state, mine, owner, channel, &from, "boop".into()).is_err(), "the blob must be a picture reference");
        let shared = Server { soundboard: vec![SoundboardSound { name: "boop".into(), blob: ImageRef { blob: "ab".into(), provider: "cd".into() }.encode(), volume: 1.0, emoji: None }], ..from };
        let (event, _) = play_shared(&state, mine, owner, channel, &shared, "boop".into()).unwrap();
        assert!(heard(&state, channel, &event).is_some());
    }

    #[test]
    fn only_shared_servers_show_up_in_the_lists() {
        let (mut a, _, _) = hangout();
        let (mut b, _, _) = hangout();
        a.server.emojis.push(CustomEmoji { name: "cat".into(), blob: "b".into() });
        b.server.emojis.push(CustomEmoji { name: "dog".into(), blob: "b".into() });
        a.server.emojis_shareable = true;
        b.server.emojis_shareable = false;
        a.server.soundboard.push(SoundboardSound { name: "boop".into(), blob: "b".into(), volume: 1.0, emoji: None });
        a.server.sounds_shareable = true;
        let shared = shareable([&a, &b].into_iter());
        assert_eq!(shared.emojis.len(), 1);
        assert_eq!(shared.emojis[0].name, "cat");
        assert_eq!(shareable_sounds([&a, &b].into_iter()).len(), 1);
    }
}
