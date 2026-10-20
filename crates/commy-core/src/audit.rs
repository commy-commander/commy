use crate::event::ServerEvent;
use crate::identity::Signature;
use crate::ids::UserId;
use crate::store::ServerState;
use serde::{Deserialize, Serialize};

pub const MAX_AUDIT_ENTRIES: usize = 1000;
const MAX_DETAIL_CHARS: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditKind {
    ServerRename,
    OwnerTransfer,
    ServerImages,
    ServerPublic,
    GameTags,
    Rules,
    AutoMod,
    AfkChannel,
    AfkTimeout,
    Expressions,
    SoundSharing,
    ChannelCreate,
    ChannelDelete,
    ChannelRename,
    ChannelRetention,
    ChannelMove,
    ChannelLimit,
    ChannelSlowMode,
    ChannelBanner,
    ChannelColor,
    ChannelKind,
    RoleCreate,
    RoleUpdate,
    RoleDelete,
    RoleAssign,
    RoleUnassign,
    MemberKick,
    MemberBan,
    MemberUnban,
    VoiceModerate,
    VoiceMove,
    VoiceDisconnect,
    EmojiAdd,
    EmojiRemove,
    SoundAdd,
    SoundRemove,
    EventCreate,
    EventUpdate,
    EventDelete,
    MessagePin,
    MessageUnpin,
    MessageDelete,
    InviteCreate,
    InviteRevoke,
    InviteBackground,
    StickerAdd,
    StickerRemove,
    EmoticonAdd,
    EmoticonRemove,
    ExpressionSharing,
    ChannelAccess,
    MemberTimeout,
    MemberTimeoutClear,
    MemberNickname,
    PollClose,
    ChannelSensitive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub at: i64,
    pub by: UserId,
    pub kind: AuditKind,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
    pub sig: Signature,
}

impl AuditEntry {
    fn order(&self) -> (i64, [u8; 64]) {
        (self.at, self.sig.0)
    }
}

fn clip(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(MAX_DETAIL_CHARS).collect()
}

fn on_off(on: bool) -> Option<String> {
    Some(if on { "on" } else { "off" }.into())
}

fn set_or_removed(set: bool) -> Option<String> {
    Some(if set { "set" } else { "removed" }.into())
}

pub fn member_name(state: &ServerState, user: UserId) -> Option<String> {
    let nickname = state.server.member(user).and_then(|m| m.nickname.clone());
    let profile = state.profiles.get(&user);
    let name = nickname
        .or_else(|| profile.and_then(|p| p.display_name.clone()))
        .or_else(|| profile.map(|p| p.username.clone()))?;
    Some(clip(&name))
}

fn account_name(state: &ServerState, user: UserId) -> Option<String> {
    state.profiles.get(&user).map(|p| clip(p.visible_name()))
}

fn channel_name(state: &ServerState, channel: crate::ids::ChannelId) -> Option<String> {
    state.server.channel(channel).map(|c| clip(&c.name))
}

fn role_name(state: &ServerState, role: crate::ids::RoleId) -> Option<String> {
    state.server.roles.iter().find(|r| r.id == role).map(|r| clip(&r.name))
}

pub fn duration_words(secs: i64) -> String {
    const UNITS: [(i64, &str, &str); 5] = [
        (7 * 86_400, "week", "weeks"),
        (86_400, "day", "days"),
        (3_600, "hour", "hours"),
        (60, "minute", "minutes"),
        (1, "second", "seconds"),
    ];
    let secs = secs.max(1);
    let (size, one, many) = UNITS.iter().copied().find(|(size, _, _)| secs % size == 0 && secs >= *size).unwrap_or(UNITS[4]);
    let n = secs / size;
    format!("{n} {}", if n == 1 { one } else { many })
}

fn voice_detail(muted: bool, deafened: bool) -> &'static str {
    match (muted, deafened) {
        (true, true) => "muted and deafened",
        (true, false) => "muted",
        (false, true) => "deafened",
        (false, false) => "cleared",
    }
}

type Described = (AuditKind, Option<String>, Option<String>, Option<String>);

fn describe(state: &ServerState, by: UserId, event: &ServerEvent) -> Option<Described> {
    use AuditKind as K;
    use ServerEvent as E;
    let server = &state.server;
    let member = |user: &UserId| (Some(user.to_string()), member_name(state, *user));
    let channel = |id: &crate::ids::ChannelId| (Some(id.to_string()), channel_name(state, *id));
    Some(match event {
        E::ServerRename { name, .. } => (K::ServerRename, None, Some(clip(&server.name)), Some(clip(name))),
        E::OwnerTransfer { user, .. } => {
            let (target, label) = member(user);
            (K::OwnerTransfer, target, label, None)
        }
        E::ServerImages { icon, banner, .. } => {
            let changed = [("icon", *icon != server.icon), ("banner", *banner != server.banner)];
            let names: Vec<&str> = changed.iter().filter(|(_, c)| *c).map(|(n, _)| *n).collect();
            (K::ServerImages, None, None, Some(names.join(" and ")))
        }
        E::PublicSet { public, .. } => (K::ServerPublic, None, None, on_off(*public)),
        E::GameTagsSet { tags, .. } => (K::GameTags, None, None, Some(clip(&tags.join(", ")))),
        E::RulesSet { rules, .. } => (K::Rules, None, None, set_or_removed(rules.is_some())),
        E::AutoModSet { .. } => (K::AutoMod, None, None, None),
        E::AfkChannelSet { channel: afk, .. } => {
            let (target, label) = afk.as_ref().map(channel).unwrap_or((None, None));
            (K::AfkChannel, target, label, None)
        }
        E::AfkTimeoutSet { minutes, .. } => (K::AfkTimeout, None, None, minutes.map(|m| m.to_string())),
        E::ExpressionSettings { .. } => (K::Expressions, None, None, None),
        E::SoundSharingSet { shareable, .. } => (K::SoundSharing, None, None, on_off(*shareable)),
        E::ChannelCreate { channel: c, .. } => {
            let kind = serde_json::to_value(c.kind).ok().and_then(|v| v.as_str().map(str::to_string));
            (K::ChannelCreate, Some(c.id.to_string()), Some(clip(&c.name)), kind)
        }
        E::ChannelDelete { channel: id, .. } => {
            let (target, label) = channel(id);
            (K::ChannelDelete, target, label, None)
        }
        E::ChannelUpdate { channel: id, name, retention_days, .. } => {
            let current = server.channel(*id)?;
            let (target, label) = channel(id);
            if current.name != *name {
                (K::ChannelRename, target, label, Some(clip(name)))
            } else {
                (K::ChannelRetention, target, label, retention_days.map(|d| d.to_string()))
            }
        }
        E::ChannelLayout { channel: id, .. } => {
            let (target, label) = channel(id);
            (K::ChannelMove, target, label, None)
        }
        E::ChannelLimit { channel: id, user_limit, .. } => {
            let (target, label) = channel(id);
            (K::ChannelLimit, target, label, Some(user_limit.to_string()))
        }
        E::ChannelSlowMode { channel: id, seconds, .. } => {
            let (target, label) = channel(id);
            (K::ChannelSlowMode, target, label, Some(seconds.to_string()))
        }
        E::ChannelBanner { channel: id, banner, .. } => {
            let (target, label) = channel(id);
            (K::ChannelBanner, target, label, set_or_removed(banner.is_some()))
        }
        E::ChannelColor { channel: id, color, .. } => {
            let (target, label) = channel(id);
            (K::ChannelColor, target, label, color.as_deref().map(clip))
        }
        E::ChannelKindSet { channel: id, kind, .. } => {
            let (target, label) = channel(id);
            let kind = serde_json::to_value(kind).ok().and_then(|v| v.as_str().map(str::to_string));
            (K::ChannelKind, target, label, kind)
        }
        E::ChannelAccessSet { channel: id, view, send, .. } => {
            let (target, label) = channel(id);
            let detail = if !view.is_empty() { "private" } else if !send.is_empty() { "send" } else { "open" };
            (K::ChannelAccess, target, label, Some(detail.into()))
        }
        E::ChannelSensitiveSet { channel: id, sensitive, .. } => {
            let (target, label) = channel(id);
            (K::ChannelSensitive, target, label, Some(if *sensitive { "on" } else { "off" }.into()))
        }
        E::RoleCreate { role, .. } => (K::RoleCreate, Some(role.id.to_string()), Some(clip(&role.name)), None),
        E::RoleUpdate { role, .. } => (K::RoleUpdate, Some(role.id.to_string()), Some(clip(&role.name)), None),
        E::RoleDelete { role, .. } => (K::RoleDelete, Some(role.to_string()), role_name(state, *role), None),
        E::RoleAssign { user, role, .. } | E::RoleUnassign { user, role, .. } if *user != by => {
            let kind = if matches!(event, E::RoleAssign { .. }) { K::RoleAssign } else { K::RoleUnassign };
            let (target, label) = member(user);
            (kind, target, label, role_name(state, *role))
        }
        E::MemberLeave { user, .. } if *user != by => {
            let (target, label) = member(user);
            (K::MemberKick, target, label, None)
        }
        E::MemberBan { user, .. } => {
            let (target, label) = member(user);
            (K::MemberBan, target, label, None)
        }
        E::MemberUnban { user, .. } => {
            let (target, label) = member(user);
            (K::MemberUnban, target, label, None)
        }
        E::VoiceModerate { user, muted, deafened, .. } => {
            let (target, label) = member(user);
            (K::VoiceModerate, target, label, Some(voice_detail(*muted, *deafened).into()))
        }
        E::MemberTimeout { user, at, until, .. } => {
            let (target, label) = member(user);
            match until {
                Some(until) => {
                    let length = until.signed_duration_since(*at).num_seconds();
                    (K::MemberTimeout, target, label, Some(duration_words(length)))
                }
                None => (K::MemberTimeoutClear, target, label, None),
            }
        }
        E::MemberNickname { user, nickname, .. } => {
            (K::MemberNickname, Some(user.to_string()), account_name(state, *user), nickname.as_deref().map(clip))
        }
        E::VoiceMove { user, channel: to, .. } => {
            let (target, label) = member(user);
            match to {
                Some(to) => (K::VoiceMove, target, label, channel_name(state, *to)),
                None => (K::VoiceDisconnect, target, label, None),
            }
        }
        E::EmojiAdd { emoji, .. } => (K::EmojiAdd, None, Some(clip(&emoji.name)), None),
        E::EmojiRemove { name, .. } => (K::EmojiRemove, None, Some(clip(name)), None),
        E::SoundAdd { sound, .. } => (K::SoundAdd, None, Some(clip(&sound.name)), None),
        E::SoundRemove { name, .. } => (K::SoundRemove, None, Some(clip(name)), None),
        E::StickerAdd { sticker, .. } => (K::StickerAdd, None, Some(clip(&sticker.name)), None),
        E::StickerRemove { name, .. } => (K::StickerRemove, None, Some(clip(name)), None),
        E::EmoticonAdd { emoticon, .. } => (K::EmoticonAdd, None, Some(clip(&emoticon.name)), None),
        E::EmoticonRemove { name, .. } => (K::EmoticonRemove, None, Some(clip(name)), None),
        E::ExpressionSharingSet { stickers, emoticons, .. } => {
            let shared: Vec<&str> = [("stickers", *stickers), ("emoticons", *emoticons)].iter().filter(|(_, on)| *on).map(|(n, _)| *n).collect();
            (K::ExpressionSharing, None, None, Some(shared.join(" and ")))
        }
        E::EventUpsert { event: scheduled, .. } => {
            let known = server.events.iter().any(|e| e.id == scheduled.id);
            let kind = if known { K::EventUpdate } else { K::EventCreate };
            (kind, Some(scheduled.id.clone()), Some(clip(&scheduled.title)), None)
        }
        E::EventDelete { event: id, .. } => {
            let title = server.events.iter().find(|e| e.id == *id).map(|e| clip(&e.title));
            (K::EventDelete, Some(id.clone()), title, None)
        }
        E::MessagePin { channel: id, message, pinned, .. } => {
            let kind = if *pinned { K::MessagePin } else { K::MessageUnpin };
            (kind, Some(message.to_string()), channel_name(state, *id), None)
        }
        E::PollClose { channel: id, message, by: closer, .. } => {
            let author = state.message(*message).map(|m| m.author);
            if author == Some(*closer) {
                return None;
            }
            (K::PollClose, Some(message.to_string()), channel_name(state, *id), None)
        }
        E::InviteTrack { invite, .. } => (K::InviteCreate, Some(invite.creator.to_string()), Some(clip(&invite.code)), None),
        E::InviteRevoke { code, creator, .. } => {
            let (target, made_by) = member(creator);
            (K::InviteRevoke, target, Some(clip(code)), made_by)
        }
        E::InviteBackgroundSet { background, .. } => (K::InviteBackground, None, None, set_or_removed(background.is_some())),
        _ => return None,
    })
}

pub fn entry_for(state: &ServerState, by: UserId, at: i64, sig: Signature, event: &ServerEvent) -> Option<AuditEntry> {
    let (kind, target, label, detail) = describe(state, by, event)?;
    Some(AuditEntry { at, by, kind, target, label, detail, sig })
}

pub fn record(log: &mut Vec<AuditEntry>, entry: AuditEntry) -> bool {
    if log.iter().any(|e| e.sig == entry.sig) {
        return false;
    }
    let full = log.len() >= MAX_AUDIT_ENTRIES;
    if full && log.first().is_some_and(|oldest| entry.order() < oldest.order()) {
        return false;
    }
    let at = log.partition_point(|e| e.order() < entry.order());
    log.insert(at, entry);
    if log.len() > MAX_AUDIT_ENTRIES {
        log.remove(0);
    }
    true
}

pub fn tidy(mut log: Vec<AuditEntry>) -> Vec<AuditEntry> {
    log.sort_by_key(AuditEntry::order);
    log.dedup_by(|a, b| a.sig == b.sig);
    let excess = log.len().saturating_sub(MAX_AUDIT_ENTRIES);
    log.drain(..excess);
    log
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::SignedEvent;
    use crate::permissions::Permissions;
    use crate::server::{Channel, ChannelKind, Role};
    use crate::test_support::{fixture, identity_for, join};

    fn sig(n: u8) -> Signature {
        Signature([n; 64])
    }

    fn entry(at: i64, n: u8) -> AuditEntry {
        AuditEntry { at, by: UserId::new(), kind: AuditKind::AutoMod, target: None, label: None, detail: None, sig: sig(n) }
    }

    #[test]
    fn durations_read_in_the_largest_whole_unit() {
        assert_eq!(duration_words(60), "1 minute");
        assert_eq!(duration_words(600), "10 minutes");
        assert_eq!(duration_words(3_600), "1 hour");
        assert_eq!(duration_words(86_400), "1 day");
        assert_eq!(duration_words(7 * 86_400), "1 week");
        assert_eq!(duration_words(90), "90 seconds");
        assert_eq!(duration_words(0), "1 second");
    }

    #[test]
    fn the_log_keeps_order_drops_copies_and_forgets_the_oldest() {
        let mut log = Vec::new();
        assert!(record(&mut log, entry(20, 2)));
        assert!(record(&mut log, entry(10, 1)));
        assert!(!record(&mut log, entry(20, 2)), "a copy is ignored");
        assert_eq!(log.iter().map(|e| e.at).collect::<Vec<_>>(), vec![10, 20]);
        let mut full: Vec<AuditEntry> = (0..MAX_AUDIT_ENTRIES as i64).map(|i| entry(100 + i, 0)).collect();
        for (i, e) in full.iter_mut().enumerate() {
            e.sig = Signature([(i % 251) as u8; 64]);
            e.sig.0[1] = (i / 251) as u8;
        }
        let newest = entry(10_000, 255);
        assert!(!record(&mut full, entry(1, 254)), "older than everything kept");
        assert!(record(&mut full, newest.clone()));
        assert_eq!(full.len(), MAX_AUDIT_ENTRIES);
        assert_eq!(full.first().unwrap().at, 101);
        assert_eq!(full.last(), Some(&newest));
    }

    #[test]
    fn tidy_sorts_and_caps_a_log_read_from_disk() {
        let log = vec![entry(30, 3), entry(10, 1), entry(30, 3), entry(20, 2)];
        assert_eq!(tidy(log).iter().map(|e| e.at).collect::<Vec<_>>(), vec![10, 20, 30]);
    }

    #[test]
    fn signed_admin_changes_land_in_the_log_with_names_from_before() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let user = join(&mut state, "theo");
        let sign = |at, event| SignedEvent::sign(sid, owner, at, event, &identity_for(owner));
        let old_name = state.server.channel(channel).unwrap().name.clone();
        let rename = ServerEvent::ChannelUpdate { server: sid, channel, name: "lobby".into(), retention_days: None };
        assert_eq!(state.apply_signed(&sign(1_000, rename)), Ok(true));
        assert_eq!(state.apply_signed(&sign(2_000, ServerEvent::MemberBan { server: sid, user })), Ok(true));
        assert_eq!(state.apply_signed(&sign(3_000, ServerEvent::ChannelDelete { server: sid, channel })), Ok(true));
        let log = state.audit_log();
        let kinds: Vec<AuditKind> = log.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec![AuditKind::ChannelRename, AuditKind::MemberBan, AuditKind::ChannelDelete]);
        assert_eq!(log[0].label.as_deref(), Some(old_name.as_str()));
        assert_eq!(log[0].detail.as_deref(), Some("lobby"));
        assert_eq!(log[1].label.as_deref(), Some("theo"));
        assert_eq!(log[1].target, Some(user.to_string()));
        assert_eq!(log[2].label.as_deref(), Some("lobby"), "named as it was when deleted");
        assert!(log.iter().all(|e| e.by == owner));
        let restored = ServerState::from_snapshot(state.snapshot());
        assert_eq!(restored.audit_log(), log, "kept across restarts");
    }

    #[test]
    fn locking_a_channel_to_roles_is_logged() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let sign = |at, event| SignedEvent::sign(sid, owner, at, event, &identity_for(owner));
        let crew = Role {
            id: crate::ids::RoleId::new(),
            name: "crew".into(),
            color: None,
            permissions: Permissions::NONE,
            position: 1,
            hoist: false,
            self_assignable: false,
            mentionable: false,
            tag: None,
            banner: None,
        };
        let role = crew.id;
        assert_eq!(state.apply_signed(&sign(1_000, ServerEvent::RoleCreate { server: sid, role: crew })), Ok(true));
        let lock = ServerEvent::ChannelAccessSet { server: sid, channel, view: vec![role], send: vec![] };
        assert_eq!(state.apply_signed(&sign(2_000, lock)), Ok(true));
        let open = ServerEvent::ChannelAccessSet { server: sid, channel, view: vec![], send: vec![role] };
        assert_eq!(state.apply_signed(&sign(3_000, open)), Ok(true));
        let access: Vec<Option<&str>> =
            state.audit_log().iter().filter(|e| e.kind == AuditKind::ChannelAccess).map(|e| e.detail.as_deref()).collect();
        assert_eq!(access, vec![Some("private"), Some("send")]);
    }

    #[test]
    fn live_state_self_service_and_no_ops_stay_out_of_the_log() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let user = join(&mut state, "mia");
        let picks = Role {
            id: crate::ids::RoleId::new(),
            name: "gamer".into(),
            color: None,
            permissions: Permissions::NONE,
            position: 1,
            hoist: false,
            self_assignable: true,
            mentionable: false,
            tag: None,
            banner: None,
        };
        let rid = picks.id;
        let by_owner = |at, event| SignedEvent::sign(sid, owner, at, event, &identity_for(owner));
        let by_user = |at, event| SignedEvent::sign(sid, user, at, event, &identity_for(user));
        state.apply_signed(&by_owner(1_000, ServerEvent::RoleCreate { server: sid, role: picks })).unwrap();
        state.apply_signed(&by_user(2_000, ServerEvent::RoleAssign { server: sid, user, role: rid })).unwrap();
        state.apply_signed(&by_user(2_500, ServerEvent::VoiceState { user, channel: None, muted: true, deafened: false, since: None })).unwrap();
        let same_name = state.server.channel(channel).unwrap().name.clone();
        let no_op = ServerEvent::ChannelUpdate { server: sid, channel, name: same_name, retention_days: None };
        assert_eq!(state.apply_signed(&by_owner(3_000, no_op)), Ok(false));
        state.apply_signed(&by_user(4_000, ServerEvent::MemberLeave { server: sid, user })).unwrap();
        let kinds: Vec<AuditKind> = state.audit_log().iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec![AuditKind::RoleCreate]);
    }

    #[test]
    fn kicks_and_role_grants_by_others_are_logged() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let user = join(&mut state, "sam");
        let mods = Role {
            id: crate::ids::RoleId::new(),
            name: "mods".into(),
            color: None,
            permissions: Permissions::KICK_MEMBERS,
            position: 1,
            hoist: false,
            self_assignable: false,
            mentionable: false,
            tag: None,
            banner: None,
        };
        let rid = mods.id;
        let sign = |at, event| SignedEvent::sign(sid, owner, at, event, &identity_for(owner));
        state.apply_signed(&sign(1_000, ServerEvent::RoleCreate { server: sid, role: mods })).unwrap();
        state.apply_signed(&sign(2_000, ServerEvent::RoleAssign { server: sid, user, role: rid })).unwrap();
        state.apply_signed(&sign(3_000, ServerEvent::MemberLeave { server: sid, user })).unwrap();
        let log = state.audit_log();
        assert_eq!(log[1].kind, AuditKind::RoleAssign);
        assert_eq!(log[1].detail.as_deref(), Some("mods"));
        assert_eq!(log[1].label.as_deref(), Some("sam"));
        assert_eq!(log[2].kind, AuditKind::MemberKick);
    }

    #[test]
    fn nickname_changes_name_the_account_and_the_new_nickname() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let user = join(&mut state, "sam");
        let sign = |at, event| SignedEvent::sign(sid, owner, at, event, &identity_for(owner));
        let rename = |nickname: Option<&str>| ServerEvent::MemberNickname { server: sid, user, nickname: nickname.map(Into::into) };
        state.apply_signed(&sign(1_000, rename(Some("Sammy")))).unwrap();
        state.apply_signed(&sign(2_000, rename(Some("Sammy")))).unwrap();
        state.apply_signed(&sign(3_000, rename(None))).unwrap();
        let log = state.audit_log();
        assert_eq!(log.len(), 2, "a repeat changes nothing");
        assert_eq!(log[0].kind, AuditKind::MemberNickname);
        assert_eq!(log[0].target.as_deref(), Some(user.to_string().as_str()));
        assert_eq!(log[0].label.as_deref(), Some("sam"));
        assert_eq!(log[0].detail.as_deref(), Some("Sammy"));
        assert_eq!(log[1].label.as_deref(), Some("sam"), "the account name, not the nickname being cleared");
        assert_eq!(log[1].detail, None);
    }

    #[test]
    fn age_restricting_a_channel_is_logged_with_its_direction() {
        let (mut state, channel, owner) = fixture();
        let sid = state.server.id;
        let sign = |at, sensitive| {
            SignedEvent::sign(sid, owner, at, ServerEvent::ChannelSensitiveSet { server: sid, channel, sensitive }, &identity_for(owner))
        };
        state.apply_signed(&sign(1_000, true)).unwrap();
        state.apply_signed(&sign(2_000, true)).unwrap();
        state.apply_signed(&sign(3_000, false)).unwrap();
        let log = state.audit_log();
        assert_eq!(log.len(), 2, "a repeat changes nothing");
        assert_eq!(log[0].kind, AuditKind::ChannelSensitive);
        assert_eq!(log[0].target.as_deref(), Some(channel.to_string().as_str()));
        assert_eq!(log[0].detail.as_deref(), Some("on"));
        assert_eq!(log[1].detail.as_deref(), Some("off"));
    }

    #[test]
    fn a_moderator_deleting_someone_elses_message_is_logged_but_own_deletes_are_not() {
        let (mut state, channel, owner) = fixture();
        let user = join(&mut state, "nova");
        let theirs = crate::test_support::post(&state, channel, user, "spam");
        let mine = crate::test_support::post(&state, channel, owner, "oops");
        state.apply(ServerEvent::MessageCreate { message: theirs.clone() }).unwrap();
        state.apply(ServerEvent::MessageCreate { message: mine.clone() }).unwrap();
        assert_eq!(state.apply(crate::test_support::delete(&state, &mine, owner)), Ok(true));
        assert!(state.audit_log().is_empty());
        assert_eq!(state.apply(crate::test_support::delete(&state, &theirs, owner)), Ok(true));
        let log = state.audit_log();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].kind, AuditKind::MessageDelete);
        assert_eq!(log[0].label.as_deref(), Some("nova"));
        assert_eq!(log[0].target, Some(user.to_string()));
        assert!(log[0].detail.is_some(), "names the channel");
    }

    #[test]
    fn a_channel_created_names_its_kind() {
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let voice = Channel::new("hangout", ChannelKind::Voice, 9);
        let signed = SignedEvent::sign(sid, owner, 1_000, ServerEvent::ChannelCreate { server: sid, channel: voice }, &identity_for(owner));
        state.apply_signed(&signed).unwrap();
        assert_eq!(state.audit_log()[0].detail.as_deref(), Some("voice"));
    }

    #[test]
    fn stickers_emoticons_and_their_sharing_are_logged() {
        use crate::server::{Emoticon, Sticker};
        let (mut state, _, owner) = fixture();
        let sid = state.server.id;
        let sign = |at, event| SignedEvent::sign(sid, owner, at, event, &identity_for(owner));
        let image = format!("{}@{}", "c".repeat(52), "0".repeat(64));
        let sticker = ServerEvent::StickerAdd { server: sid, sticker: Sticker { name: "party".into(), blob: image } };
        let emoticon = ServerEvent::EmoticonAdd { server: sid, emoticon: Emoticon { name: "shrug".into(), text: "o_O".into() } };
        let share = ServerEvent::ExpressionSharingSet { server: sid, stickers: true, emoticons: true };
        assert_eq!(state.apply_signed(&sign(1_000, sticker)), Ok(true));
        assert_eq!(state.apply_signed(&sign(2_000, emoticon)), Ok(true));
        assert_eq!(state.apply_signed(&sign(3_000, share)), Ok(true));
        let log = state.audit_log();
        let kinds: Vec<AuditKind> = log.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec![AuditKind::StickerAdd, AuditKind::EmoticonAdd, AuditKind::ExpressionSharing]);
        assert_eq!(log[0].label.as_deref(), Some("party"));
        assert_eq!(log[2].detail.as_deref(), Some("stickers and emoticons"));
    }
}
