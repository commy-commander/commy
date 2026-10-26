use std::collections::HashSet;

use commy_core::server::{clean_forum_tags, clean_post_title, MAX_FORUM_TAGS, MAX_FORUM_TAG_CHARS, MAX_POST_TITLE_CHARS};
use commy_core::{Channel, ChannelId, ForumTag, MessageId, Permissions, ServerEvent, ServerState, TagId, UserId};
use serde::Deserialize;

use crate::act::Act;
use crate::manage::need;

#[derive(Debug, Deserialize)]
pub struct TagInput {
    pub id: Option<String>,
    pub name: String,
    pub emoji: Option<String>,
}

pub fn forum(state: &ServerState, channel: ChannelId) -> Act<&Channel> {
    let found = state.server.channel(channel).ok_or("unknown channel")?;
    if !found.is_forum() {
        return Err("that is not a forum channel".into());
    }
    Ok(found)
}

pub fn title(raw: &str) -> Act<String> {
    clean_post_title(raw).ok_or_else(|| format!("a post title is 1 to {MAX_POST_TITLE_CHARS} characters"))
}

pub fn tag_picks(raw: &[String]) -> Act<Vec<TagId>> {
    raw.iter().map(|t| t.parse::<TagId>().map_err(|_| "that tag no longer exists".to_string())).collect()
}

fn known_tags(forum: &Channel, picks: &[TagId]) -> Act<()> {
    if picks.iter().all(|pick| forum.tags.iter().any(|t| t.id == *pick)) {
        Ok(())
    } else {
        Err("that tag is not on this forum".into())
    }
}

pub fn set_tags(state: &ServerState, perms: Permissions, me: UserId, channel: ChannelId, tags: Vec<TagInput>) -> Act<ServerEvent> {
    need(perms, Permissions::MANAGE_CHANNELS, "edit channels")?;
    forum(state, channel)?;
    let tags: Vec<ForumTag> = tags
        .into_iter()
        .map(|t| ForumTag { id: t.id.and_then(|id| id.parse().ok()).unwrap_or_else(TagId::new), name: t.name, emoji: t.emoji })
        .collect();
    let tags = clean_forum_tags(&tags)
        .ok_or_else(|| format!("a forum can have up to {MAX_FORUM_TAGS} tags of {MAX_FORUM_TAG_CHARS} characters each"))?;
    Ok(ServerEvent::ForumTagsSet { server: state.server.id, channel, tags, by: me })
}

pub fn can_post(state: &ServerState, channel: ChannelId, tags: &[TagId]) -> Act<()> {
    known_tags(forum(state, channel)?, tags)
}

pub fn post(state: &ServerState, channel: ChannelId, message: MessageId, title: String, tags: Vec<TagId>, by: UserId) -> ServerEvent {
    ServerEvent::ForumPost { server: state.server.id, channel, message, title, tags, by }
}

pub fn edit_post(
    state: &ServerState,
    perms: Permissions,
    me: UserId,
    channel: ChannelId,
    message: MessageId,
    title: String,
    tags: Vec<TagId>,
) -> Act<ServerEvent> {
    let forum = forum(state, channel)?;
    known_tags(forum, &tags)?;
    let post = forum.posts.get(&message).ok_or("that post no longer exists")?;
    if post.author != me && !perms.allows(Permissions::MANAGE_MESSAGES) {
        return Err("you can only change your own posts".into());
    }
    Ok(self::post(state, channel, message, title, tags, me))
}

fn post_root(state: &ServerState, channel: ChannelId, from: MessageId) -> Option<MessageId> {
    let messages = state.messages(channel);
    let mut current = messages.iter().find(|m| m.id == from)?;
    let mut seen = HashSet::new();
    while let Some(parent) = current.reply_to.and_then(|p| messages.iter().find(|m| m.id == p)) {
        if !seen.insert(current.id) {
            break;
        }
        current = parent;
    }
    Some(current.id)
}

pub fn inside_post(state: &ServerState, channel: ChannelId, reply_to: Option<MessageId>) -> Act<()> {
    let Some(forum) = state.server.channel(channel).filter(|c| c.is_forum()) else { return Ok(()) };
    let Some(target) = reply_to else {
        return Err("start a post to write in a forum channel".into());
    };
    match post_root(state, channel, target) {
        Some(root) if forum.posts.contains_key(&root) => Ok(()),
        _ => Err("replies in a forum go inside a post".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{ChannelKind, Server};

    fn hangout() -> (ServerState, UserId, ChannelId) {
        let owner = UserId::new();
        let mut state = ServerState::new(Server::new("Hangout", owner));
        let channel = Channel::new("ideas", ChannelKind::Forum, 9);
        let id = channel.id;
        state.server.channels.push(channel);
        (state, owner, id)
    }

    #[test]
    fn only_channel_managers_set_forum_tags_and_only_on_forums() {
        let (state, owner, forum) = hangout();
        let text = state.server.channels.iter().find(|c| c.kind == ChannelKind::Text).map(|c| c.id).unwrap();
        let tag = || vec![TagInput { id: None, name: "Help".into(), emoji: None }];
        let mine = state.server.permissions_for(owner);
        assert!(matches!(set_tags(&state, mine, owner, forum, tag()), Ok(ServerEvent::ForumTagsSet { tags, .. }) if tags.len() == 1));
        assert!(set_tags(&state, Permissions::default_member(), owner, forum, tag()).is_err());
        assert_eq!(set_tags(&state, mine, owner, text, tag()).unwrap_err(), "that is not a forum channel");
    }

    #[test]
    fn posts_take_known_tags_and_only_their_authors_or_moderators_edit_them() {
        let (mut state, owner, forum_id) = hangout();
        let member = UserId::new();
        let help = ForumTag { id: TagId::new(), name: "Help".into(), emoji: None };
        let channel = state.server.channels.iter_mut().find(|c| c.id == forum_id).unwrap();
        channel.tags.push(help.clone());
        let root = MessageId::new();
        channel.posts.insert(root, commy_core::server::ForumPost { title: "Hi".into(), tags: vec![], author: member });
        assert!(can_post(&state, forum_id, &[help.id]).is_ok());
        assert_eq!(can_post(&state, forum_id, &[TagId::new()]).unwrap_err(), "that tag is not on this forum");
        let theirs = Permissions::default_member();
        assert!(edit_post(&state, theirs, member, forum_id, root, "Hello".into(), vec![help.id]).is_ok());
        assert_eq!(edit_post(&state, theirs, UserId::new(), forum_id, root, "x".into(), vec![]).unwrap_err(), "you can only change your own posts");
        assert!(edit_post(&state, state.server.permissions_for(owner), owner, forum_id, root, "x".into(), vec![]).is_ok());
        assert!(title("   ").is_err());
        assert_eq!(title("  a   post ").unwrap(), "a post");
        assert!(tag_picks(&["nope".into()]).is_err());
    }

    #[test]
    fn writing_in_a_forum_means_replying_inside_a_post() {
        let (state, _, forum) = hangout();
        assert_eq!(inside_post(&state, forum, None).unwrap_err(), "start a post to write in a forum channel");
        assert_eq!(inside_post(&state, forum, Some(MessageId::new())).unwrap_err(), "replies in a forum go inside a post");
        let text = state.server.channels.iter().find(|c| c.kind == ChannelKind::Text).map(|c| c.id).unwrap();
        assert!(inside_post(&state, text, None).is_ok());
    }
}
