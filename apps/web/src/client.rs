use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use commy_core::{
    Action, ChannelId, ChannelKind, Identity, Message, MessageId, Permissions, ServerEvent, ServerId, ServerState,
    Snapshot, UserId, UserProfile,
};
use commy_net::protocol::{gossip_size, verify_sync_response, SyncMember, MAX_GOSSIP_BYTES};
use commy_net::{ServerTicket, WirePayload};
use commy_view::keys::KeyHolder;
use commy_view::{MessageView, ServerSummary, ServerView};

pub type Servers = Arc<Mutex<HashMap<ServerId, ServerState>>>;
pub type Me = Arc<Mutex<Option<(UserProfile, Identity)>>>;

pub const MAX_MESSAGE_LEN: usize = 4000;
pub const SNAPSHOT_MAX_AGE_MS: i64 = 900_000;
const PAGE: usize = 100;

#[derive(Debug, Clone, PartialEq)]
pub enum Applied {
    Nothing,
    Changed,
    Created(Message),
}

#[derive(Default)]
pub struct Client {
    pub servers: Servers,
    pub me: Me,
    pub tickets: HashMap<ServerId, ServerTicket>,
    pub pending: HashSet<MessageId>,
}

impl Client {
    pub fn profile(&self) -> Option<UserProfile> {
        self.me.lock().unwrap().as_ref().map(|(profile, _)| profile.clone())
    }

    fn me_id(&self) -> Option<UserId> {
        self.me.lock().unwrap().as_ref().map(|(profile, _)| profile.id)
    }

    fn keys(&self) -> KeyHolder {
        KeyHolder::new(self.me.lock().unwrap().as_ref().map(|(profile, identity)| (profile.id, identity.clone())))
    }

    pub fn has(&self, server: ServerId) -> bool {
        self.servers.lock().unwrap().contains_key(&server)
    }

    pub fn list_servers(&self) -> Vec<ServerSummary> {
        let me = self.me_id();
        let servers = self.servers.lock().unwrap();
        let mut list: Vec<ServerSummary> = servers.values().map(|s| ServerSummary::from_state(s, me)).collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    pub fn get_server(&self, server: ServerId) -> Result<ServerView, String> {
        let me = self.me_id();
        let keys = self.keys();
        let servers = self.servers.lock().unwrap();
        let state = servers.get(&server).ok_or("This server is still loading")?;
        Ok(ServerView::for_viewer(state, me, &keys))
    }

    pub fn get_messages(&self, server: ServerId, channel: ChannelId, around: Option<MessageId>) -> Result<Vec<MessageView>, String> {
        let me = self.me_id();
        let keys = self.keys();
        let servers = self.servers.lock().unwrap();
        let state = servers.get(&server).ok_or("This server is still loading")?;
        match around {
            Some(id) => commy_view::message_views_around(state, channel, id, &self.pending, me, &keys, None)
                .ok_or_else(|| "That message is not loaded in this browser".to_string()),
            None => Ok(commy_view::message_views(state, channel, PAGE, &self.pending, me, &keys, None)),
        }
    }

    pub fn compose(
        &self,
        server: ServerId,
        channel: ChannelId,
        content: &str,
        reply_to: Option<MessageId>,
    ) -> Result<(Message, MessageView), String> {
        let content = clean_message(content)?;
        let (profile, identity) = self.me.lock().unwrap().clone().ok_or("Your servers are not unlocked yet")?;
        let keys = KeyHolder::new(Some((profile.id, identity.clone())));
        let mut servers = self.servers.lock().unwrap();
        let state = servers.get_mut(&server).ok_or("This server is still loading")?;
        check_can_post(state, profile.id, channel)?;
        let mut message = Message::new(channel, profile.id, content);
        if let Some(target) = reply_to {
            if !state.messages(channel).iter().any(|m| m.id == target) {
                return Err("The message you replied to no longer exists".into());
            }
            message.reply_to = Some(target);
        }
        if let Some(wait) = state.slow_mode_wait(channel, profile.id, message.created_at) {
            return Err(format!("Slow mode is on here: you can send again in {wait} seconds"));
        }
        if let Some(hold) = state.hold(&message) {
            return Err(hold.refusal().into());
        }
        keys.seal_outgoing(state, &mut message)?;
        message.sign(server, &identity);
        if !fits_one_gossip_frame(&message) {
            return Err("That message is too long to send; try splitting it up".into());
        }
        if let Some(hold) = keys.hold(state, &message) {
            return Err(hold.refusal().into());
        }
        state
            .apply(ServerEvent::MessageCreate { message: message.clone() })
            .map_err(|e| e.to_string())?;
        let view = MessageView::build(&message, state, &keys);
        Ok((message, view))
    }

    pub fn apply_live(&self, server: ServerId, event: ServerEvent, action: Option<Action>) -> Applied {
        let keys = self.keys();
        let me = self.me_id();
        let mut servers = self.servers.lock().unwrap();
        let Some(state) = servers.get_mut(&server) else { return Applied::Nothing };
        match commy_net::catchup::apply_live(state, event, action) {
            (Ok(true), Some(message), _) => {
                let visible = me.is_some_and(|me| state.server.can_view(message.channel, me));
                match keys.open(state, &message) {
                    Some(opened) if visible && keys.hold(state, &message).is_none() => Applied::Created(opened),
                    _ => Applied::Changed,
                }
            }
            (Ok(true), _, _) => Applied::Changed,
            _ => Applied::Nothing,
        }
    }

    pub fn install_mirror(&self, server: ServerId, bytes: &[u8]) -> Result<bool, String> {
        let snapshot: Snapshot = serde_json::from_slice(bytes).map_err(|_| "That server copy is damaged".to_string())?;
        if snapshot.server.id != server {
            return Err("That server copy is for another server".into());
        }
        let me = self.me_id().ok_or("Your servers are not unlocked yet")?;
        if !snapshot.server.is_member(me) {
            return Err("You are not a member of that server".into());
        }
        let mut servers = self.servers.lock().unwrap();
        if servers.contains_key(&server) {
            return Ok(false);
        }
        servers.insert(server, ServerState::from_snapshot(snapshot));
        Ok(true)
    }

    pub fn install_owner_snapshot(&self, server: ServerId, payload: WirePayload) -> Result<bool, String> {
        let ticket = self.tickets.get(&server).ok_or("No invite for that server")?;
        let (owner, key) = ticket.owner.ok_or("That invite does not name its owner")?;
        let WirePayload::SyncResponse { server: remote, by, at, .. } = &payload else {
            return Ok(false);
        };
        let fresh = (Utc::now().timestamp_millis() - *at).abs() <= SNAPSHOT_MAX_AGE_MS;
        if remote.id != server || *by != owner || remote.owner != owner || !fresh || !verify_sync_response(&payload, &key) {
            return Err("Ignored a server copy its owner did not sign".into());
        }
        let WirePayload::SyncResponse { server: mut remote, members, profiles, recent_messages, at, .. } = payload else {
            return Ok(false);
        };
        let me = self.me_id().ok_or("Your servers are not unlocked yet")?;
        remote.members = members.into_iter().map(member_of).collect();
        if !remote.is_member(me) {
            return Err("You are not a member of that server yet".into());
        }
        let mut servers = self.servers.lock().unwrap();
        if servers.contains_key(&server) {
            return Ok(false);
        }
        let mut state = ServerState::new(remote);
        state.set_action_floor(at);
        for profile in profiles {
            state.profiles.insert(profile.id, profile.as_seen_by_others());
        }
        for message in recent_messages {
            let _ = state.apply(ServerEvent::MessageCreate { message });
        }
        servers.insert(server, state);
        Ok(true)
    }

    pub fn message_created_payload(&self, server: ServerId, message: &Message) -> Option<serde_json::Value> {
        let servers = self.servers.lock().unwrap();
        let state = servers.get(&server)?;
        let channel = state.server.channel(message.channel);
        Some(serde_json::json!({
            "server": server.to_string(),
            "server_name": state.server.name,
            "channel": message.channel.to_string(),
            "channel_name": channel.map(|c| c.name.clone()).unwrap_or_default(),
            "announcement": channel.is_some_and(|c| c.kind == ChannelKind::Announcement),
            "author": message.author.to_string(),
            "author_name": state.profiles.get(&message.author).map(|p| p.visible_name().to_string()).unwrap_or_else(|| "Someone".into()),
            "content": message.content,
            "has_attachments": message.files().next().is_some(),
        }))
    }
}

pub fn clean_message(content: &str) -> Result<String, String> {
    let content = content.trim();
    if content.is_empty() || content.len() > MAX_MESSAGE_LEN {
        return Err(format!("Messages must be 1 to {MAX_MESSAGE_LEN} characters"));
    }
    Ok(content.to_string())
}

fn check_can_post(state: &ServerState, user: UserId, channel: ChannelId) -> Result<(), String> {
    let perms = state.server.permissions_for(user);
    let exempt = user == state.server.owner || perms.allows(Permissions::MANAGE_SERVER);
    if state.server.rules.is_some() && !exempt && !state.server.rules_accepted.contains(&user) {
        return Err("Accept the server rules in the desktop app before posting".into());
    }
    let channel = state.server.channel(channel).ok_or("That channel no longer exists")?;
    if channel.kind == ChannelKind::Announcement && !perms.can_announce() {
        return Err("Only members allowed to post announcements can write in this channel".into());
    }
    if !state.server.can_view(channel.id, user) {
        return Err("You do not have a role that can see that channel".into());
    }
    if channel.is_restricted() && !state.server.can_post_in(channel.id, user) {
        return Err("Only some roles can send messages in this channel".into());
    }
    if state.server.is_timed_out(user, Utc::now()) {
        return Err("You are timed out in this server for now".into());
    }
    Ok(())
}

fn fits_one_gossip_frame(message: &Message) -> bool {
    let wire = WirePayload::Event(ServerEvent::MessageCreate { message: message.clone() });
    gossip_size(&wire) <= MAX_GOSSIP_BYTES
}

fn member_of(m: SyncMember) -> commy_core::Member {
    let joined_at = chrono::DateTime::from_timestamp_millis(m.joined_ms).unwrap_or_default();
    commy_core::Member { user: m.user, nickname: m.nickname, roles: m.roles, joined_at, timeout: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::Server;
    use commy_net::protocol::{build_sync_response, sign_sync_response};

    struct World {
        client: Client,
        owner: (UserProfile, Identity),
        server: ServerId,
        channel: ChannelId,
        state: ServerState,
    }

    fn world() -> World {
        let owner_identity = Identity::generate();
        let mut owner = UserProfile::new("owner");
        owner.identity = Some(owner_identity.public());
        let my_identity = Identity::generate();
        let mut me = UserProfile::new("me");
        me.identity = Some(my_identity.public());
        let server = Server::new("Hangout", owner.id);
        let (sid, channel) = (server.id, server.channels[0].id);
        let mut state = ServerState::new(server);
        state.profiles.insert(owner.id, owner.clone());
        state.apply(ServerEvent::MemberJoin { server: sid, profile: me.clone() }).unwrap();
        let mut hello = Message::new(channel, owner.id, "hello");
        hello.sign(sid, &owner_identity);
        state.apply(ServerEvent::MessageCreate { message: hello }).unwrap();
        let client = Client::default();
        *client.me.lock().unwrap() = Some((me, my_identity));
        World { client, owner: (owner, owner_identity), server: sid, channel, state }
    }

    fn mirror(world: &World) -> Vec<u8> {
        serde_json::to_vec(&world.state.snapshot()).unwrap()
    }

    #[test]
    fn a_mirrored_server_lists_and_shows_its_history() {
        let w = world();
        assert!(w.client.install_mirror(w.server, &mirror(&w)).unwrap());
        assert_eq!(w.client.list_servers()[0].name, "Hangout");
        assert_eq!(w.client.get_server(w.server).unwrap().id, w.server.to_string());
        let messages = w.client.get_messages(w.server, w.channel, None).unwrap();
        assert_eq!(messages.iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), ["hello"]);
        assert!(!w.client.install_mirror(w.server, &mirror(&w)).unwrap(), "only the first copy is taken");
    }

    #[test]
    fn a_mirror_for_another_server_or_a_stranger_is_refused() {
        let w = world();
        assert!(w.client.install_mirror(ServerId::new(), &mirror(&w)).is_err());
        let stranger = Client::default();
        let mut profile = UserProfile::new("stranger");
        let identity = Identity::generate();
        profile.identity = Some(identity.public());
        *stranger.me.lock().unwrap() = Some((profile, identity));
        assert!(stranger.install_mirror(w.server, &mirror(&w)).is_err());
    }

    #[test]
    fn sending_signs_stores_and_shows_the_message() {
        let w = world();
        w.client.install_mirror(w.server, &mirror(&w)).unwrap();
        let (message, view) = w.client.compose(w.server, w.channel, "  hi there ", None).unwrap();
        assert_eq!(view.content, "hi there");
        let me = w.client.profile().unwrap();
        assert!(message.verify(w.server, &me.identity.unwrap()));
        let messages = w.client.get_messages(w.server, w.channel, None).unwrap();
        assert_eq!(messages.last().unwrap().content, "hi there");
    }

    #[test]
    fn sending_refuses_empty_and_oversized_text() {
        let w = world();
        w.client.install_mirror(w.server, &mirror(&w)).unwrap();
        assert!(w.client.compose(w.server, w.channel, "   ", None).is_err());
        let long = "a".repeat(MAX_MESSAGE_LEN + 1);
        assert!(w.client.compose(w.server, w.channel, &long, None).is_err());
    }

    #[test]
    fn live_messages_from_members_are_applied_once() {
        let w = world();
        w.client.install_mirror(w.server, &mirror(&w)).unwrap();
        let (owner, identity) = &w.owner;
        let mut message = Message::new(w.channel, owner.id, "live");
        message.sign(w.server, identity);
        let event = ServerEvent::MessageCreate { message: message.clone() };
        assert_eq!(w.client.apply_live(w.server, event.clone(), None), Applied::Created(message.clone()));
        assert_eq!(w.client.apply_live(w.server, event, None), Applied::Nothing);
        let payload = w.client.message_created_payload(w.server, &message).unwrap();
        assert_eq!(payload["author_name"], "owner");
    }

    #[test]
    fn a_forged_live_message_is_dropped() {
        let w = world();
        w.client.install_mirror(w.server, &mirror(&w)).unwrap();
        let (owner, _) = &w.owner;
        let mut forged = Message::new(w.channel, owner.id, "forged");
        forged.sign(w.server, &Identity::generate());
        assert_eq!(w.client.apply_live(w.server, ServerEvent::MessageCreate { message: forged }, None), Applied::Nothing);
    }

    fn with_ticket(w: &mut World, owner_key: commy_core::IdentityKey) {
        let mut ticket = ServerTicket::new(w.server, "Hangout", [7; 32], vec![]);
        ticket.owner = Some((w.owner.0.id, owner_key));
        w.client.tickets.insert(w.server, ticket);
    }

    #[test]
    fn an_owner_signed_snapshot_installs_the_server() {
        let mut w = world();
        let owner_key = w.owner.1.public();
        with_ticket(&mut w, owner_key);
        let mut payload = build_sync_response(&w.state, 50, w.owner.0.id);
        sign_sync_response(&mut payload, Utc::now().timestamp_millis(), &w.owner.1);
        assert!(w.client.install_owner_snapshot(w.server, payload).unwrap());
        let messages = w.client.get_messages(w.server, w.channel, None).unwrap();
        assert_eq!(messages[0].content, "hello");
    }

    #[test]
    fn a_snapshot_not_signed_by_the_owner_is_refused() {
        let mut w = world();
        let owner_key = w.owner.1.public();
        with_ticket(&mut w, owner_key);
        let mut payload = build_sync_response(&w.state, 50, w.owner.0.id);
        sign_sync_response(&mut payload, Utc::now().timestamp_millis(), &Identity::generate());
        assert!(w.client.install_owner_snapshot(w.server, payload).is_err());
        assert!(!w.client.has(w.server));
    }
}
