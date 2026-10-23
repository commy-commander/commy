use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use commy_core::channel_key::{ChannelSecret, KeyId, Keyring};
use commy_core::{
    action, Action, ChannelId, ChannelKind, Identity, Message, MessageId, Permissions, Presence, ServerEvent, ServerId,
    ServerState, SignedEvent, Snapshot, UserId, UserProfile,
};
use commy_net::community_streams::{self as streams, Lane, Want};
use commy_net::protocol::{gossip_size, verify_sync_response, SyncMember, MAX_GOSSIP_BYTES};
use commy_net::{ServerTicket, WirePayload};
use commy_view::act::{self, Act};
use commy_view::keys::KeyHolder;
use commy_view::liveness::{self, Liveness};
use commy_view::reads::{ReadMark, ReadState, ReadView};
use commy_view::search::{Page, SearchQuery, SearchResults};
use commy_view::{MessageView, PinView, ServerSummary, ServerView};

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
    pub liveness: Mutex<Liveness>,
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

    pub fn get_pins(&self, server: ServerId, channel: Option<ChannelId>) -> Result<Vec<PinView>, String> {
        let me = self.me_id().ok_or("Your servers are not unlocked yet")?;
        let keys = self.keys();
        let servers = self.servers.lock().unwrap();
        let state = servers.get(&server).ok_or("This server is still loading")?;
        Ok(commy_view::pin_views(state, channel, me, &keys))
    }

    pub fn act(
        &self,
        server: ServerId,
        build: impl FnOnce(&ServerState, &act::Me, &Identity, &KeyHolder) -> Act<ServerEvent>,
    ) -> Result<WirePayload, String> {
        let (profile, identity) = self.me.lock().unwrap().clone().ok_or("Your servers are not unlocked yet")?;
        let keys = KeyHolder::new(Some((profile.id, identity.clone())));
        let mut servers = self.servers.lock().unwrap();
        let state = servers.get_mut(&server).ok_or("This server is still loading")?;
        let me = act::Me { user: profile.id, perms: state.server.permissions_for(profile.id), now: Utc::now() };
        let event = build(state, &me, &identity, &keys)?;
        let signed = SignedEvent::sign(server, profile.id, me.now.timestamp_millis(), event, &identity);
        state.apply_own(&signed).map_err(|e| e.to_string())?;
        Ok(if action::self_signed(&signed.event) { WirePayload::Event(signed.event) } else { WirePayload::Signed(signed) })
    }

    pub fn typing(&self, server: ServerId, channel: ChannelId) -> Result<Option<WirePayload>, String> {
        let (profile, identity) = self.me.lock().unwrap().clone().ok_or("Your servers are not unlocked yet")?;
        let servers = self.servers.lock().unwrap();
        let state = servers.get(&server).ok_or("This server is still loading")?;
        let now = Utc::now();
        let Some(event) = act::typing(state, profile.id, channel, now)? else { return Ok(None) };
        Ok(Some(WirePayload::Signed(SignedEvent::sign(server, profile.id, now.timestamp_millis(), event, &identity))))
    }

    pub fn admits(&self, server: ServerId, event: &ServerEvent, action: Option<Action>) -> bool {
        let servers = self.servers.lock().unwrap();
        let Some(state) = servers.get(&server) else { return false };
        match action {
            Some(action) => state.admits_signed(&SignedEvent { action, event: event.clone() }).is_ok(),
            None => state.admits_unsigned(event).is_ok(),
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
        seal_sign_apply(state, &keys, &identity, message)
    }

    pub fn compose_poll(&self, server: ServerId, channel: ChannelId, poll: PollForm) -> Result<(Message, MessageView), String> {
        let (profile, identity) = self.me.lock().unwrap().clone().ok_or("Your servers are not unlocked yet")?;
        let keys = KeyHolder::new(Some((profile.id, identity.clone())));
        let mut servers = self.servers.lock().unwrap();
        let state = servers.get_mut(&server).ok_or("This server is still loading")?;
        check_can_post(state, profile.id, channel)?;
        let perms = state.server.permissions_for(profile.id);
        let poll = act::poll(perms, poll.question, poll.options, poll.multi, poll.hours, Utc::now())?;
        let mut message = Message::new(channel, profile.id, "");
        message.attachments.push(poll.into_attachment());
        seal_sign_apply(state, &keys, &identity, message)
    }

    pub fn read_view(&self, reads: &ReadState, only: Option<&[String]>) -> ReadView {
        let me = self.me_id();
        let servers = self.servers.lock().unwrap();
        commy_view::reads::view(servers.values(), reads, only, me, &HashSet::new())
    }

    pub fn latest_marks(&self, server: ServerId, channels: Option<&[String]>, now_ms: i64) -> Result<BTreeMap<String, ReadMark>, String> {
        let servers = self.servers.lock().unwrap();
        let state = servers.get(&server).ok_or("This server is still loading")?;
        Ok(state
            .server
            .channels
            .iter()
            .filter(|c| channels.is_none_or(|ids| ids.contains(&c.id.to_string())))
            .filter_map(|c| commy_view::reads::latest_mark(state.messages(c.id), now_ms).map(|mark| (c.id.to_string(), mark)))
            .collect())
    }

    pub fn search(&self, server: ServerId, query: &SearchQuery, page: Page) -> Result<SearchResults, String> {
        let me = self.me_id();
        let keys = self.keys();
        let servers = self.servers.lock().unwrap();
        let state = servers.get(&server).ok_or("This server is still loading")?;
        Ok(commy_view::search::search(state, query, me, page, &keys, &commy_view::ChannelOpens::new(), Some(Utc::now())))
    }

    pub fn apply_live(&self, server: ServerId, event: ServerEvent, action: Option<Action>) -> Applied {
        self.apply_from(server, event, action, false)
    }

    pub fn apply_stored(&self, server: ServerId, event: ServerEvent, action: Option<Action>) -> Applied {
        self.apply_from(server, event, action, true)
    }

    fn apply_from(&self, server: ServerId, event: ServerEvent, action: Option<Action>, stored: bool) -> Applied {
        let keys = self.keys();
        let me = self.me_id();
        let mut servers = self.servers.lock().unwrap();
        let Some(state) = servers.get_mut(&server) else { return Applied::Nothing };
        let profile = match &event {
            ServerEvent::ProfileUpdate { profile } | ServerEvent::MemberJoin { profile, .. } if stored => Some(profile.id),
            _ => None,
        };
        let shown = profile.map(|user| state.profiles.get(&user).map_or(Presence::Offline, |p| p.presence));
        let applied = commy_net::catchup::apply_live(state, event, action);
        if let (Some(user), Some(shown)) = (profile, shown) {
            if let Some(profile) = state.profiles.get_mut(&user) {
                profile.presence = shown;
            }
        }
        match applied {
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

    pub fn apply_signal(&self, server: ServerId, node: &[u8; 32], event: ServerEvent, action: Option<Action>) -> bool {
        let speaker = liveness::speaker(&event, action.as_ref());
        let said = match &event {
            ServerEvent::PresenceUpdate { user, presence } => Some((*user, *presence)),
            _ => None,
        };
        let now = web_time::Instant::now();
        let mut servers = self.servers.lock().unwrap();
        let Some(state) = servers.get_mut(&server) else { return false };
        let mut changed = match commy_net::catchup::apply_live(state, event, action).0 {
            Ok(changed) => changed,
            Err(_) => return false,
        };
        let mut live = self.liveness.lock().unwrap();
        if let Some((user, presence)) = said {
            changed |= live.settle_device_presence(state, user, node, presence, now);
        }
        if let Some(user) = speaker {
            live.heard_through_service(server, user, now);
        }
        changed
    }

    pub fn hear_peer(&self, server: ServerId, event: &ServerEvent, action: Option<&Action>) {
        if let Some(user) = liveness::speaker(event, action) {
            self.liveness.lock().unwrap().heard(server, user, web_time::Instant::now());
        }
    }

    pub fn is_shown_online(&self, server: ServerId, user: UserId) -> bool {
        let servers = self.servers.lock().unwrap();
        servers.get(&server).and_then(|s| s.profiles.get(&user)).is_some_and(|p| p.presence.is_visible_online())
    }

    pub fn here(&self, server: ServerId, node: &[u8; 32]) -> Vec<WirePayload> {
        let Some((profile, identity)) = self.me.lock().unwrap().clone() else { return Vec::new() };
        if profile.presence == Presence::Invisible {
            return Vec::new();
        }
        let announce = ServerEvent::DeviceAnnounce { server, user: profile.id, node: liveness::node_hex(node), voice: None, since: None };
        if let Some(state) = self.servers.lock().unwrap().get_mut(&server) {
            let _ = state.apply(announce.clone());
        }
        let at = Utc::now().timestamp_millis();
        [announce, ServerEvent::PresenceUpdate { user: profile.id, presence: profile.presence }]
            .into_iter()
            .map(|event| WirePayload::Signed(SignedEvent::sign(server, profile.id, at, event, &identity)))
            .collect()
    }

    pub fn farewell(&self) -> Vec<(ServerId, WirePayload)> {
        let Some((profile, identity)) = self.me.lock().unwrap().clone() else { return Vec::new() };
        if profile.presence == Presence::Invisible {
            return Vec::new();
        }
        let at = Utc::now().timestamp_millis();
        let servers: Vec<ServerId> = self.servers.lock().unwrap().keys().copied().collect();
        servers
            .into_iter()
            .map(|server| {
                let event = ServerEvent::PresenceUpdate { user: profile.id, presence: Presence::Offline };
                (server, WirePayload::Signed(SignedEvent::sign(server, profile.id, at, event, &identity)))
            })
            .collect()
    }

    pub fn sweep_presence(&self, wall_ms: i64) -> Vec<ServerId> {
        let Some(me) = self.me_id() else { return Vec::new() };
        let mut servers = self.servers.lock().unwrap();
        let mut live = self.liveness.lock().unwrap();
        live.sweep(servers.iter_mut(), me, web_time::Instant::now(), wall_ms, liveness::EXPIRE)
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
        self.install_snapshot(server, payload, SNAPSHOT_MAX_AGE_MS, None)
    }

    pub fn install_service_snapshot(&self, server: ServerId, payload: WirePayload, joined_ms: Option<i64>) -> Result<bool, String> {
        self.install_snapshot(server, payload, streams::RETENTION_MS, joined_ms)
    }

    pub fn vouched(&self, server: ServerId, payload: &WirePayload, max_age_ms: i64) -> Result<(), String> {
        let ticket = self.tickets.get(&server).ok_or("No invite for that server")?;
        let (owner, key) = ticket.owner.ok_or("That invite does not name its owner")?;
        let WirePayload::SyncResponse { server: remote, by, at, .. } = payload else {
            return Err("That is not a server copy".into());
        };
        let fresh = (Utc::now().timestamp_millis() - *at).abs() <= max_age_ms;
        if remote.id != server || *by != owner || remote.owner != owner || !fresh || !verify_sync_response(payload, &key) {
            return Err("Ignored a server copy its owner did not sign".into());
        }
        Ok(())
    }

    fn install_snapshot(&self, server: ServerId, payload: WirePayload, max_age_ms: i64, joined_ms: Option<i64>) -> Result<bool, String> {
        if !matches!(payload, WirePayload::SyncResponse { .. }) {
            return Ok(false);
        }
        self.vouched(server, &payload, max_age_ms)?;
        let WirePayload::SyncResponse { server: mut remote, members, profiles, recent_messages, at, .. } = payload else {
            return Ok(false);
        };
        let me = self.profile().ok_or("Your servers are not unlocked yet")?;
        remote.members = members.into_iter().map(member_of).collect();
        let member = remote.is_member(me.id);
        let before_i_joined = joined_ms.is_some_and(|joined| at < joined);
        if remote.banned.contains(&me.id) || (!member && !before_i_joined) {
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
        if !member && state.apply(ServerEvent::MemberJoin { server, profile: me }) != Ok(true) {
            return Err("Could not join that server from its copy".into());
        }
        for message in recent_messages {
            let _ = state.apply(ServerEvent::MessageCreate { message });
        }
        servers.insert(server, state);
        Ok(true)
    }

    pub fn learn_profiles(&self, server: ServerId, payload: &WirePayload) -> bool {
        if self.vouched(server, payload, streams::RETENTION_MS).is_err() {
            return false;
        }
        let WirePayload::SyncResponse { profiles, .. } = payload else { return false };
        let mut servers = self.servers.lock().unwrap();
        let Some(state) = servers.get_mut(&server) else { return false };
        !commy_net::catchup::apply_profiles(state, profiles.clone()).is_empty()
    }

    pub fn forget(&mut self, server: ServerId) {
        self.tickets.remove(&server);
        self.servers.lock().unwrap().remove(&server);
    }

    pub fn sync_wants(&self, given: &HashMap<ServerId, (KeyId, ChannelSecret)>, now_ms: i64) -> HashMap<ServerId, Vec<Want>> {
        let Some((me, identity)) = self.me.lock().unwrap().as_ref().map(|(profile, identity)| (profile.id, identity.clone())) else {
            return HashMap::new();
        };
        let holder = KeyHolder::new(Some((me, identity.clone())));
        let servers = self.servers.lock().unwrap();
        self.tickets
            .keys()
            .filter_map(|sid| {
                let given = given.get(sid).cloned();
                let Some(state) = servers.get(sid) else {
                    let (key, secret) = given?;
                    return Some((*sid, vec![streams::bootstrap(*sid, key, &secret)]));
                };
                let community = holder.community_ring(state, given.clone())?;
                let locked = |channel| Keyring::unlock(state, channel, me, &identity);
                let mut wants = streams::wanted(state, me, &community, locked, now_ms);
                let stranded = !wants.iter().any(|w| w.lane == Lane::State) && state.server.is_current_member(me) && !state.is_deleted();
                if let Some((key, secret)) = given.filter(|_| stranded) {
                    wants.push(streams::bootstrap(*sid, key, &secret));
                }
                (!wants.is_empty()).then_some((*sid, wants))
            })
            .collect()
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
    state.server.channel(channel).ok_or("That channel no longer exists")?;
    act::can_post(state, perms, user, channel, Utc::now())
}

pub struct PollForm {
    pub question: String,
    pub options: Vec<String>,
    pub multi: bool,
    pub hours: Option<u32>,
}

fn seal_sign_apply(state: &mut ServerState, keys: &KeyHolder, identity: &Identity, mut message: Message) -> Result<(Message, MessageView), String> {
    let server = state.server.id;
    keys.seal_outgoing(state, &mut message)?;
    message.sign(server, identity);
    if !fits_one_gossip_frame(&message) {
        return Err("That message is too long to send; try splitting it up".into());
    }
    if let Some(hold) = keys.hold(state, &message) {
        return Err(hold.refusal().into());
    }
    state.apply(ServerEvent::MessageCreate { message: message.clone() }).map_err(|e| e.to_string())?;
    let view = MessageView::build(&message, state, keys);
    Ok((message, view))
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
    fn the_browser_says_it_is_here_and_a_friend_stays_online_while_one_of_their_devices_is() {
        let w = world();
        assert!(w.client.install_mirror(w.server, &mirror(&w)).unwrap());
        let node = [7u8; 32];
        let said = w.client.here(w.server, &node);
        assert_eq!(said.len(), 2);
        let me = w.client.profile().unwrap().id;
        let mut observer = w.state.clone();
        for payload in &said {
            let WirePayload::Signed(signed) = payload else { panic!("presence is signed") };
            assert!(observer.admits_signed(signed).is_ok());
            observer.apply(signed.event.clone()).unwrap();
        }
        assert!(observer.devices_of(me).iter().any(|d| d.node == liveness::node_hex(&node)));

        let (owner, identity) = w.owner.clone();
        let (desk, phone) = ([1u8; 32], [2u8; 32]);
        let at = Utc::now().timestamp_millis();
        let signed = |event: ServerEvent| {
            let signed = SignedEvent::sign(w.server, owner.id, at, event, &identity);
            (signed.event, Some(signed.action))
        };
        for node in [desk, phone] {
            let announce = ServerEvent::DeviceAnnounce { server: w.server, user: owner.id, node: liveness::node_hex(&node), voice: None, since: None };
            let (event, action) = signed(announce);
            w.client.apply_signal(w.server, &node, event, action);
        }
        let online = |presence| ServerEvent::PresenceUpdate { user: owner.id, presence };
        let (event, action) = signed(online(Presence::Online));
        w.client.apply_signal(w.server, &desk, event, action);
        let (event, action) = signed(online(Presence::Online));
        w.client.apply_signal(w.server, &phone, event, action);
        let (event, action) = signed(online(Presence::Offline));
        w.client.apply_signal(w.server, &phone, event, action);
        assert!(w.client.is_shown_online(w.server, owner.id), "the desk is still on");
        let (event, action) = signed(online(Presence::Offline));
        w.client.apply_signal(w.server, &desk, event, action);
        assert!(!w.client.is_shown_online(w.server, owner.id));
        assert!(w.client.farewell().iter().all(|(_, p)| matches!(p, WirePayload::Signed(s) if s.event == ServerEvent::PresenceUpdate { user: me, presence: Presence::Offline })));
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
    fn a_poll_posts_and_takes_a_vote_and_search_finds_messages() {
        let w = world();
        w.client.install_mirror(w.server, &mirror(&w)).unwrap();
        let form = |options: Vec<&str>| PollForm { question: "Lunch?".into(), options: options.into_iter().map(String::from).collect(), multi: false, hours: Some(1) };
        assert!(w.client.compose_poll(w.server, w.channel, form(vec!["only one"])).is_err());
        let (poll, _) = w.client.compose_poll(w.server, w.channel, form(vec!["Soup", "Salad"])).unwrap();
        let vote = w.client.act(w.server, |state, me, _, _| act::poll_vote(state, me, poll.id, &[1]));
        assert!(matches!(vote, Ok(WirePayload::Signed(_))));
        let query = SearchQuery::new(&["hello".to_string()], None, None, None);
        let found = w.client.search(w.server, &query, Page::newest()).unwrap();
        assert_eq!(found.hits.iter().map(|h| h.message.content.as_str()).collect::<Vec<_>>(), ["hello"]);
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

    fn signed_snapshot(w: &World, at: i64) -> WirePayload {
        let mut payload = build_sync_response(&w.state, 50, w.owner.0.id);
        sign_sync_response(&mut payload, at, &w.owner.1);
        payload
    }

    #[test]
    fn a_service_snapshot_installs_a_listed_server_even_before_i_appear_in_it() {
        let mut w = world();
        let me = w.client.profile().unwrap();
        w.state.server.members.retain(|m| m.user != me.id);
        let owner_key = w.owner.1.public();
        with_ticket(&mut w, owner_key);
        let week_old = Utc::now().timestamp_millis() - 7 * 24 * 3600 * 1000;
        assert!(w.client.install_owner_snapshot(w.server, signed_snapshot(&w, week_old)).is_err(), "too old for a live copy");
        assert!(w.client.install_service_snapshot(w.server, signed_snapshot(&w, week_old), None).is_err(), "not in it and no later join");
        let joined_after = Some(week_old + 1);
        assert!(w.client.install_service_snapshot(w.server, signed_snapshot(&w, week_old), joined_after).unwrap());
        assert!(w.client.servers.lock().unwrap()[&w.server].server.is_member(me.id));
        assert_eq!(w.client.get_messages(w.server, w.channel, None).unwrap()[0].content, "hello");
    }

    #[test]
    fn a_service_snapshot_past_retention_or_from_a_stranger_is_refused() {
        let mut w = world();
        let owner_key = w.owner.1.public();
        with_ticket(&mut w, owner_key);
        let ancient = Utc::now().timestamp_millis() - streams::RETENTION_MS - 60_000;
        assert!(w.client.install_service_snapshot(w.server, signed_snapshot(&w, ancient), None).is_err());
        let mut forged = build_sync_response(&w.state, 50, w.owner.0.id);
        sign_sync_response(&mut forged, Utc::now().timestamp_millis(), &Identity::generate());
        assert!(w.client.install_service_snapshot(w.server, forged, None).is_err());
        assert!(!w.client.has(w.server));
    }

    #[test]
    fn a_service_snapshot_that_bans_me_is_refused() {
        let mut w = world();
        let me = w.client.profile().unwrap();
        w.state.server.members.retain(|m| m.user != me.id);
        w.state.server.banned.push(me.id);
        let owner_key = w.owner.1.public();
        with_ticket(&mut w, owner_key);
        let now = Utc::now().timestamp_millis();
        assert!(w.client.install_service_snapshot(w.server, signed_snapshot(&w, now), Some(now + 1)).is_err());
        assert!(!w.client.has(w.server));
    }

    #[test]
    fn a_vouched_snapshot_teaches_profiles_of_an_installed_server() {
        let mut w = world();
        let owner_key = w.owner.1.public();
        with_ticket(&mut w, owner_key);
        w.client.install_mirror(w.server, &mirror(&w)).unwrap();
        let mut newcomer = UserProfile::new("newcomer");
        newcomer.identity = Some(Identity::generate().public());
        w.state.apply(ServerEvent::MemberJoin { server: w.server, profile: newcomer.clone() }).unwrap();
        assert!(w.client.learn_profiles(w.server, &signed_snapshot(&w, Utc::now().timestamp_millis())));
        assert!(w.client.servers.lock().unwrap()[&w.server].identity_of(newcomer.id).is_some());
        let mut forged = build_sync_response(&w.state, 50, w.owner.0.id);
        sign_sync_response(&mut forged, Utc::now().timestamp_millis(), &Identity::generate());
        assert!(!w.client.learn_profiles(w.server, &forged));
    }

    fn community_key(w: &mut World) -> (KeyId, ChannelSecret) {
        let (owner, identity) = (w.owner.0.id, w.owner.1.clone());
        let ring = Keyring::unlock_community(&w.state, owner, &identity);
        let task = commy_core::channel_key::community_key_task(&w.state, owner, &ring).unwrap();
        let mut ring = ring;
        for event in ring.task_events(&w.state, commy_core::channel_key::KeyScope::Community, task, Utc::now().timestamp_millis()) {
            let signed = commy_core::action::SignedEvent::sign(w.server, owner, Utc::now().timestamp_millis(), event, &identity);
            w.state.apply_signed(&signed).unwrap();
        }
        let ring = Keyring::unlock_community(&w.state, owner, &identity);
        ring.newest_in(&w.state, commy_core::channel_key::KeyScope::Community).map(|(k, s)| (k, s.clone())).unwrap()
    }

    #[test]
    fn a_listed_server_without_a_copy_bootstraps_from_its_state_stream() {
        let mut w = world();
        let owner_key = w.owner.1.public();
        with_ticket(&mut w, owner_key);
        let (key, secret) = community_key(&mut w);
        assert!(w.client.sync_wants(&HashMap::new(), 0).is_empty(), "nothing to follow without a key or a copy");
        let wants = w.client.sync_wants(&[(w.server, (key, secret))].into(), 0);
        assert_eq!(wants[&w.server].len(), 1);
        assert_eq!(wants[&w.server][0].lane, Lane::State);
        assert!(!wants[&w.server][0].writes);
    }

    #[test]
    fn an_installed_server_follows_its_state_and_every_text_channel() {
        let mut w = world();
        let owner_key = w.owner.1.public();
        with_ticket(&mut w, owner_key);
        let given = community_key(&mut w);
        w.client.install_mirror(w.server, &mirror(&w)).unwrap();
        let now = Utc::now().timestamp_millis();
        let wants = w.client.sync_wants(&[(w.server, given)].into(), now);
        let lanes: Vec<Lane> = wants[&w.server].iter().map(|w| w.lane).collect();
        assert!(lanes.contains(&Lane::State));
        assert!(lanes.contains(&Lane::Channel(w.channel)));
        assert!(wants[&w.server].iter().all(|w| w.writes));
        w.client.forget(w.server);
        assert!(w.client.sync_wants(&HashMap::new(), now).is_empty());
    }
}
