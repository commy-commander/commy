use chrono::{DateTime, Utc};
use commy_core::server::{clean_thread_name, valid_reaction, valid_thread_name, MAX_PINS_PER_CHANNEL, MAX_THREAD_NAME_CHARS};
use commy_core::{
    ChannelId, ChannelKind, Identity, Message, MessageId, Permissions, Poll, ServerEvent, ServerId, ServerState, Signature,
    TombstoneProof, UserId, MAX_MESSAGE_BYTES,
};

use crate::keys::KeyHolder;

pub type Act<T> = Result<T, String>;

pub fn can_view(state: &ServerState, user: UserId, channel: ChannelId) -> Act<()> {
    if state.server.can_view(channel, user) {
        Ok(())
    } else {
        Err("you do not have a role that can see that channel".into())
    }
}

pub fn not_timed_out(state: &ServerState, user: UserId, now: DateTime<Utc>) -> Act<()> {
    match state.server.timeout_of(user, now) {
        Some(timeout) => {
            let left = commy_core::audit::duration_words(timeout.remaining_secs(now).max(1));
            Err(format!("you are timed out in this server for another {left}"))
        }
        None => Ok(()),
    }
}

pub fn can_post(state: &ServerState, perms: Permissions, user: UserId, channel: ChannelId, now: DateTime<Utc>) -> Act<()> {
    can_view(state, user, channel)?;
    not_timed_out(state, user, now)?;
    let restricted = state.server.channel(channel).is_some_and(|c| c.is_restricted());
    if restricted && !state.server.can_post_in(channel, user) {
        return Err("only some roles can send messages in this channel".into());
    }
    let announcement = state.server.channel(channel).is_some_and(|c| c.kind == ChannelKind::Announcement);
    if announcement && !perms.can_announce() {
        return Err("only members allowed to post announcements can write in this channel".into());
    }
    Ok(())
}

pub fn author_of(state: &ServerState, channel: ChannelId, message: MessageId) -> Act<UserId> {
    state
        .messages(channel)
        .iter()
        .find(|m| m.id == message)
        .map(|m| m.author)
        .ok_or_else(|| "that message no longer exists".to_string())
}

pub fn edit_time_after_last_edit(original: &Message, now: DateTime<Utc>) -> DateTime<Utc> {
    match original.edited_at {
        Some(last) if now.timestamp_millis() <= last.timestamp_millis() => last + chrono::Duration::milliseconds(1),
        _ => now,
    }
}

pub fn sign_edit(server: ServerId, original: &Message, content: String, now: DateTime<Utc>, identity: &Identity) -> Act<ServerEvent> {
    let edited_at = edit_time_after_last_edit(original, now);
    let placeholder_signature_of_real_size = Some(Signature([0; 64]));
    let edited = Message { content, edited_at: Some(edited_at), sig: placeholder_signature_of_real_size, ..original.clone() };
    if edited.authored_size() > MAX_MESSAGE_BYTES {
        return Err("that edit is too long; try shortening it".into());
    }
    Ok(original.edit_event(server, edited.content, edited_at, identity))
}

#[derive(Debug, Clone, Copy)]
pub struct Me {
    pub user: UserId,
    pub perms: Permissions,
    pub now: DateTime<Utc>,
}

pub fn edit(
    state: &ServerState,
    me: &Me,
    identity: &Identity,
    keys: &KeyHolder,
    channel: ChannelId,
    message: MessageId,
    content: String,
) -> Act<ServerEvent> {
    let original = state.messages(channel).iter().find(|m| m.id == message).ok_or("that message no longer exists")?;
    if original.author != me.user {
        return Err("you can only edit your own messages".into());
    }
    can_post(state, me.perms, me.user, channel, me.now)?;
    let rules = state.automod();
    if !rules.exempt(&state.server, me.user) {
        if let Some(hold) = rules.content_hold(&content) {
            return Err(hold.refusal().into());
        }
    }
    let content = keys.seal_edit(state, original, content)?;
    sign_edit(state.server.id, original, content, me.now, identity)
}

pub fn may_delete(state: &ServerState, perms: Permissions, user: UserId, channel: ChannelId, message: MessageId) -> Act<()> {
    if author_of(state, channel, message)? != user && !perms.allows(Permissions::MANAGE_MESSAGES) {
        return Err("you do not have permission to delete other people's messages".into());
    }
    Ok(())
}

pub fn delete(state: &ServerState, me: &Me, identity: &Identity, channel: ChannelId, message: MessageId) -> Act<ServerEvent> {
    may_delete(state, me.perms, me.user, channel, message)?;
    Ok(TombstoneProof::sign(state.server.id, channel, message, me.user, me.now, identity).into_event())
}

pub fn reaction(state: &ServerState, me: &Me, channel: ChannelId, message: MessageId, emoji: &str, active: bool) -> Act<ServerEvent> {
    let emoji = emoji.trim().to_string();
    if !valid_reaction(&emoji) {
        return Err("reactions must be a single emoji".into());
    }
    can_view(state, me.user, channel)?;
    not_timed_out(state, me.user, me.now)?;
    Ok(ServerEvent::ReactionSet { channel, message, emoji, user: me.user, active })
}

pub fn pin(state: &ServerState, me: &Me, channel: ChannelId, message: MessageId, pinned: bool) -> Act<ServerEvent> {
    if !me.perms.can_pin() {
        return Err("you do not have permission to pin messages".into());
    }
    can_view(state, me.user, channel)?;
    author_of(state, channel, message)?;
    let pins = state.server.pins.iter().filter(|p| p.channel == channel).count();
    if pinned && pins >= MAX_PINS_PER_CHANNEL {
        return Err(format!("a channel can hold {MAX_PINS_PER_CHANNEL} pinned messages; unpin one first"));
    }
    Ok(ServerEvent::MessagePin { server: state.server.id, channel, message, by: me.user, at: me.now, pinned })
}

pub fn poll(perms: Permissions, question: String, options: Vec<String>, multi: bool, hours: Option<u32>, now: DateTime<Utc>) -> Act<Poll> {
    use commy_core::poll::{MAX_DURATION_HOURS, MAX_OPTIONS, MAX_OPTION_CHARS, MAX_QUESTION_CHARS, MIN_OPTIONS};
    if !perms.can_send_polls() {
        return Err("you do not have permission to create polls here".into());
    }
    let hours = hours.filter(|h| *h > 0);
    if hours.is_some_and(|h| h > MAX_DURATION_HOURS) {
        return Err(format!("a poll runs for up to {} days", MAX_DURATION_HOURS / 24));
    }
    let closes_at = hours.map(|h| now + chrono::Duration::hours(i64::from(h)));
    Poll { question, options, multi, closes_at }.tidy().ok_or_else(|| {
        format!(
            "a poll needs a question of up to {MAX_QUESTION_CHARS} characters and {MIN_OPTIONS} to {MAX_OPTIONS} answers of up to {MAX_OPTION_CHARS} characters each"
        )
    })
}

pub fn poll_target(state: &ServerState, message: MessageId, user: UserId) -> Act<(ChannelId, Message, Poll)> {
    let found = state.message(message).ok_or("that message no longer exists")?;
    can_view(state, user, found.channel)?;
    let poll = found.poll().ok_or("that message is not a poll")?;
    Ok((found.channel, found.clone(), poll))
}

pub fn poll_vote(state: &ServerState, me: &Me, message: MessageId, choices: &[u8]) -> Act<ServerEvent> {
    let (channel, _, poll) = poll_target(state, message, me.user)?;
    if state.poll_closed(message, me.now) {
        return Err("that poll has closed".into());
    }
    let choices = poll.normalize_choices(choices).ok_or(if poll.multi { "pick answers from the poll" } else { "this poll takes one answer" })?;
    Ok(ServerEvent::PollVote { server: state.server.id, channel, message, user: me.user, choices })
}

pub fn poll_close(state: &ServerState, me: &Me, message: MessageId) -> Act<ServerEvent> {
    let (channel, found, _) = poll_target(state, message, me.user)?;
    if found.author != me.user && !me.perms.allows(Permissions::MANAGE_MESSAGES) {
        return Err("only the poll's author or a moderator can close it".into());
    }
    if state.poll_closed(message, me.now) {
        return Err("that poll has already closed".into());
    }
    Ok(ServerEvent::PollClose { server: state.server.id, channel, message, by: me.user })
}

pub fn thread(
    state: &ServerState,
    me: &Me,
    channel: ChannelId,
    root: MessageId,
    name: Option<&str>,
    archived: bool,
    locked: bool,
) -> Act<ServerEvent> {
    let name = name.and_then(clean_thread_name);
    if name.as_deref().is_some_and(|n| !valid_thread_name(n)) {
        return Err(format!("thread names can be at most {MAX_THREAD_NAME_CHARS} characters"));
    }
    can_view(state, me.user, channel)?;
    if author_of(state, channel, root)? != me.user && !me.perms.allows(Permissions::MANAGE_MESSAGES) {
        return Err("only the person who started a thread or a moderator can change it".into());
    }
    if state.thread_of(channel, root).is_none() && !me.perms.can_create_threads() {
        return Err("you do not have permission to start threads here".into());
    }
    Ok(ServerEvent::ThreadSet { server: state.server.id, channel, root, name, archived, locked, by: me.user })
}

pub fn typing(state: &ServerState, user: UserId, channel: ChannelId, now: DateTime<Utc>) -> Act<Option<ServerEvent>> {
    if state.server.channel(channel).is_none() {
        return Err("unknown channel".into());
    }
    if state.server.is_timed_out(user, now) {
        return Ok(None);
    }
    Ok(Some(ServerEvent::TypingStart { channel, user }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Server, UserProfile};

    struct World {
        state: ServerState,
        owner: UserId,
        member: UserId,
        member_identity: Identity,
        channel: ChannelId,
        theirs: MessageId,
        mine: MessageId,
    }

    fn world() -> World {
        let owner_identity = Identity::generate();
        let mut owner = UserProfile::new("owner");
        owner.identity = Some(owner_identity.public());
        let member_identity = Identity::generate();
        let mut member = UserProfile::new("member");
        member.identity = Some(member_identity.public());
        let server = Server::new("Hangout", owner.id);
        let (sid, channel) = (server.id, server.channels[0].id);
        let mut state = ServerState::new(server);
        state.profiles.insert(owner.id, owner.clone());
        state.apply(ServerEvent::MemberJoin { server: sid, profile: member.clone() }).unwrap();
        let mut theirs = Message::new(channel, owner.id, "hello");
        theirs.sign(sid, &owner_identity);
        let mut mine = Message::new(channel, member.id, "hi");
        mine.sign(sid, &member_identity);
        let ids = (theirs.id, mine.id);
        state.apply(ServerEvent::MessageCreate { message: theirs }).unwrap();
        state.apply(ServerEvent::MessageCreate { message: mine }).unwrap();
        World { owner: owner.id, member: member.id, member_identity, channel, theirs: ids.0, mine: ids.1, state }
    }

    fn me(w: &World) -> Me {
        Me { user: w.member, perms: w.state.server.permissions_for(w.member), now: Utc::now() }
    }

    #[test]
    fn members_edit_only_their_own_messages() {
        let w = world();
        let keys = KeyHolder::as_user(w.member, w.member_identity.clone());
        let edit_mine = edit(&w.state, &me(&w), &w.member_identity, &keys, w.channel, w.mine, "hi again".into()).unwrap();
        let mut state = w.state.clone();
        assert_eq!(state.apply(edit_mine), Ok(true));
        assert_eq!(state.message(w.mine).unwrap().content, "hi again");
        assert!(edit(&w.state, &me(&w), &w.member_identity, &keys, w.channel, w.theirs, "mine now".into()).is_err());
    }

    #[test]
    fn deleting_someone_elses_message_needs_manage_messages() {
        let w = world();
        assert!(delete(&w.state, &me(&w), &w.member_identity, w.channel, w.theirs).is_err());
        let moderator = Me { perms: Permissions::MANAGE_MESSAGES | me(&w).perms, ..me(&w) };
        assert!(delete(&w.state, &moderator, &w.member_identity, w.channel, w.theirs).is_ok());
        let mut state = w.state.clone();
        assert_eq!(state.apply(delete(&w.state, &me(&w), &w.member_identity, w.channel, w.mine).unwrap()), Ok(true));
        assert!(state.message(w.mine).is_none());
    }

    #[test]
    fn reactions_take_one_emoji_on_a_visible_channel() {
        let w = world();
        assert!(reaction(&w.state, &me(&w), w.channel, w.theirs, "not an emoji", true).is_err());
        let event = reaction(&w.state, &me(&w), w.channel, w.theirs, " 👍 ", true).unwrap();
        assert!(matches!(&event, ServerEvent::ReactionSet { emoji, active: true, .. } if emoji == "👍"));
        let mut state = w.state.clone();
        assert_eq!(state.apply(event), Ok(true));
    }

    #[test]
    fn pinning_needs_the_permission_and_an_existing_message() {
        let w = world();
        let without = Me { perms: Permissions::NONE, ..me(&w) };
        assert!(pin(&w.state, &without, w.channel, w.theirs, true).is_err());
        let with = Me { perms: Permissions::all(), ..me(&w) };
        assert!(pin(&w.state, &with, w.channel, MessageId::new(), true).is_err());
        assert!(matches!(pin(&w.state, &with, w.channel, w.theirs, true), Ok(ServerEvent::MessagePin { pinned: true, .. })));
    }

    #[test]
    fn typing_in_an_unknown_channel_is_refused() {
        let w = world();
        assert!(typing(&w.state, w.member, ChannelId::new(), Utc::now()).is_err());
        assert!(matches!(typing(&w.state, w.member, w.channel, Utc::now()), Ok(Some(ServerEvent::TypingStart { .. }))));
        assert_ne!(w.owner, w.member);
    }
}
