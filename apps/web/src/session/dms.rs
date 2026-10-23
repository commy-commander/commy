use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use commy_core::dm::{DmAction, DmMessage};
use commy_core::{Identity, MessageId, ServerId, UserId, UserProfile};
use commy_net::inbox;
use commy_net::node::IncomingDm;
use commy_net::sync_streams::{self, DmEntry};
use commy_net::DmEnvelope;
use commy_sync::StreamId;
use commy_view::dms;
use commy_view::safety::{self, DmPolicy, FriendSignal};
use commy_view::{dm_message_view, DmMessageView, DmThreadView};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use wasm_bindgen_futures::spawn_local;

use super::{parse, to_json, Args, Session};
use crate::account;
use crate::client::clean_message;
use crate::dm_book::{self, DmBook, Received, Source};
use crate::sync_book::partner_record;

const HOLD_WAIT: Duration = Duration::from_secs(8);

fn node_of(hex: &str) -> Option<[u8; 32]> {
    data_encoding::HEXLOWER_PERMISSIVE.decode(hex.trim().as_bytes()).ok().and_then(|node| node.try_into().ok())
}

fn named(id: UserId, name: String) -> Value {
    json!({ "id": id.to_string(), "name": name })
}

impl Session {
    pub(super) fn load_dms(&self) {
        let Ok(account) = self.account() else { return };
        let safety = self.host.load(&account::safety_key(&account.id)).and_then(|raw| serde_json::from_str(&raw).ok());
        *self.dms.borrow_mut() = DmBook::new(safety.unwrap_or_default());
    }

    fn save_safety(&self) {
        let Ok(account) = self.account() else { return };
        let Ok(raw) = serde_json::to_string(&self.dms.borrow().safety) else { return };
        self.host.save(&account::safety_key(&account.id), &raw);
    }

    fn save_safety_if_changed(&self) {
        if self.dms.borrow_mut().take_dirty() {
            self.save_safety();
        }
    }

    fn me_and_key(&self) -> Option<(UserProfile, Identity)> {
        self.client.borrow().me.lock().unwrap().clone()
    }

    fn node_hex(&self) -> Option<String> {
        self.sync.borrow().as_ref().map(|live| data_encoding::HEXLOWER.encode(&live.node))
    }

    pub(super) fn refresh_dms(&self) {
        let Some((_, identity)) = self.me_and_key() else { return };
        let Some(given) = self.sync.borrow().as_ref().map(|live| live.book.given()) else { return };
        let now = Utc::now();
        let client = self.client.borrow();
        let inboxes = client.own_inboxes(&given, now.timestamp_millis());
        let plan = {
            let servers = client.servers.lock().unwrap();
            self.dms.borrow_mut().plan(&identity, &servers, inboxes, now)
        };
        let mut guard = self.sync.borrow_mut();
        let Some(live) = guard.as_mut() else { return };
        for stream in plan.unfollow {
            live.pool.unfollow(stream);
        }
        if !plan.follow.is_empty() {
            live.pool.follow_together(plan.follow);
        }
    }

    pub(super) fn adopt_partner(&self, profile: UserProfile, at: i64, accepted: bool) {
        let Some((me, _)) = self.me_and_key() else { return };
        if self.dms.borrow_mut().note_partner(me.id, profile, at, accepted) {
            self.save_safety_if_changed();
            self.refresh_dms();
        }
    }

    fn note_partner(&self, partner: UserId, accepted: bool) {
        let Some((me, _)) = self.me_and_key() else { return };
        let now = Utc::now().timestamp_millis();
        let profile = {
            let client = self.client.borrow();
            let servers = client.servers.lock().unwrap();
            let book = self.dms.borrow();
            book.partner_profile(&servers, partner).filter(|_| book.should_note(partner, accepted, now))
        };
        let Some(mut profile) = profile else { return };
        profile.activity = None;
        profile.activity_cover = None;
        profile.activity_since = None;
        self.dms.borrow_mut().note_partner(me.id, profile.clone(), now, accepted);
        self.append_account(partner_record(&profile, now, accepted));
    }

    fn noticed(&self, partner: UserId) {
        let Some((me, _)) = self.me_and_key() else { return };
        let talks = self.dms.borrow().talks_with(me.id, partner);
        self.note_partner(partner, talks);
        self.refresh_dms();
    }

    pub(super) fn on_dm_entry(self: &Rc<Self>, stream: StreamId, seq: u64, source: Source, plain: &[u8]) {
        let live_now = self.dms.borrow().is_live(stream);
        self.dms.borrow_mut().advance(stream, seq);
        let Some((me, identity)) = self.me_and_key() else { return };
        match source {
            Source::Inbox(server) => {
                let Some(envelope) = inbox::open(&identity, plain) else { return };
                let from = envelope.message.from;
                let member = self.client.borrow().servers.lock().unwrap().get(&server).is_some_and(|state| {
                    state.server.is_current_member(from)
                        && state.profiles.get(&from).and_then(|p| p.identity) == envelope.sender.identity
                });
                if !member || dms::is_call(&envelope.message) {
                    return;
                }
                let Some(node) = node_of(&envelope.message.from_node) else { return };
                self.take_dm(envelope, node, live_now);
            }
            Source::Pair(partner, key) => {
                match sync_streams::decode_dm(plain, me.id, &identity.public(), partner, &key) {
                    Some(DmEntry::FromPartner(envelope)) if dms::syncable(&envelope.message) => {
                        let Some(node) = node_of(&envelope.message.from_node) else { return };
                        self.take_dm(envelope, node, live_now);
                    }
                    Some(DmEntry::FromMe(envelope)) if dms::syncable(&envelope.message) => {
                        if self.dms.borrow_mut().take_mine(partner, envelope.message) {
                            self.emit("dm-changed", json!(partner.to_string()));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    pub(super) async fn run_dms(self: Rc<Self>, mut incoming: mpsc::UnboundedReceiver<IncomingDm>) {
        while let Some(IncomingDm { envelope, node }) = incoming.recv().await {
            if !dms::is_call(&envelope.message) {
                self.take_dm(envelope, node, true);
            }
        }
    }

    fn take_dm(self: &Rc<Self>, envelope: DmEnvelope, node: [u8; 32], live_now: bool) {
        let Some((me, _)) = self.me_and_key() else { return };
        let now = Utc::now();
        let rings = dms::should_ring(live_now, envelope.message.created_at, now);
        let at = if live_now { now.timestamp() } else { envelope.message.created_at.timestamp() };
        let received = {
            let client = self.client.borrow();
            let servers = client.servers.lock().unwrap();
            self.dms.borrow_mut().receive(me.id, &servers, envelope, node, at)
        };
        self.save_safety_if_changed();
        match received {
            Received::Posted { partner, notice } => {
                self.emit("dm-changed", json!(partner.to_string()));
                if rings {
                    self.emit("dm-received", notice);
                }
                self.noticed(partner);
            }
            Received::Changed(partner) => self.emit("dm-changed", json!(partner.to_string())),
            Received::Friends { notice, reply } => {
                self.emit("friends-changed", notice);
                if let Some((to, signal)) = reply {
                    let this = self.clone();
                    spawn_local(async move {
                        let _ = this.send_friend_signal(to, signal, None).await;
                    });
                }
            }
            Received::Invited { ticket, key, .. } => self.join_group(&ticket, key),
            Received::Dropped => {}
        }
    }

    fn profile_for(&self, me: &UserProfile, partner: UserId) -> UserProfile {
        let shares_server = self.client.borrow().servers.lock().unwrap().values().any(|s| s.server.is_member(partner));
        let profile = me.as_seen_by_others();
        if self.dms.borrow().safety.shows_details_to(partner, shares_server) {
            profile
        } else {
            profile.without_details()
        }
    }

    fn partner(&self, raw: Option<&String>) -> Result<(UserProfile, UserId), String> {
        let partner: UserId = parse(raw, "user")?;
        let (me, _) = self.me_and_key().ok_or("Your servers are not unlocked yet")?;
        if partner == me.id {
            return Err("you cannot message yourself".into());
        }
        if self.dms.borrow().safety.is_blocked(partner) {
            return Err("you blocked this person; unblock them to send a message".into());
        }
        Ok((me, partner))
    }

    fn recipient(&self, raw: Option<&String>) -> Result<(UserProfile, UserId), String> {
        let (me, partner) = self.partner(raw)?;
        let known = {
            let client = self.client.borrow();
            let servers = client.servers.lock().unwrap();
            self.dms.borrow().known_user(&servers, partner)
        };
        if !known {
            return Err("you can only message people you share a server with or who have messaged you".into());
        }
        Ok((me, partner))
    }

    fn own_post(&self, partner: UserId, raw: Option<&String>, me: UserId) -> Result<MessageId, String> {
        let target: MessageId = parse(raw, "message")?;
        let book = self.dms.borrow();
        let original = book.store.post(partner, target).ok_or("that message no longer exists")?;
        if original.from != me {
            return Err("you can only change your own messages".into());
        }
        Ok(target)
    }

    async fn append_pair(&self, identity: &Identity, partner: UserId, envelope: &DmEnvelope) -> bool {
        if !dms::syncable(&envelope.message) {
            return false;
        }
        let key = {
            let client = self.client.borrow();
            let servers = client.servers.lock().unwrap();
            self.dms.borrow().partner_key(&servers, partner)
        };
        let Some(keys) = key.and_then(|key| sync_streams::dm_stream(identity, &key, &sync_streams::dm_epoch(Utc::now()))) else {
            return false;
        };
        let Some(client) = self.sync.borrow().as_ref().and_then(|live| live.pool.client_for(keys.id)) else { return false };
        let plain = sync_streams::encode_dm(envelope);
        matches!(n0_future::time::timeout(HOLD_WAIT, client.append(keys.id, plain)).await, Ok(Ok(_)))
    }

    async fn post_inbox(&self, to: UserId, envelope: &DmEnvelope) -> bool {
        if dms::syncable(&envelope.message) && self.dms.borrow().follows(to, Utc::now().timestamp_millis()) {
            return false;
        }
        let Some(given) = self.sync.borrow().as_ref().map(|live| live.book.given()) else { return false };
        let key = {
            let client = self.client.borrow();
            let servers = client.servers.lock().unwrap();
            self.dms.borrow().partner_key(&servers, to)
        };
        let (Some(key), Some(keys)) = (key, self.client.borrow().inbox_to(&given, to)) else { return false };
        let Some(sealed) = inbox::seal(&key, envelope) else { return false };
        let stream = keys.id;
        let (client, borrowed) = {
            let mut guard = self.sync.borrow_mut();
            let Some(live) = guard.as_mut() else { return false };
            let borrowed = !live.pool.follows(stream);
            if borrowed {
                live.pool.follow_together_tail(vec![(keys, 0)], Some(0));
            }
            (live.pool.client_for(stream), borrowed)
        };
        let sent = match client {
            Some(client) => matches!(n0_future::time::timeout(HOLD_WAIT, client.append(stream, sealed)).await, Ok(Ok(_))),
            None => false,
        };
        if borrowed {
            if let Some(live) = self.sync.borrow_mut().as_mut() {
                live.pool.unfollow(stream);
            }
        }
        sent
    }

    async fn send_direct(&self, to: UserId, envelope: &DmEnvelope) -> bool {
        let hex = {
            let client = self.client.borrow();
            let servers = client.servers.lock().unwrap();
            dm_book::known_node(&self.dms.borrow().store, &servers, to)
        };
        let Some(hex) = hex else { return false };
        let send = {
            let node = self.node.lock().await;
            let Some(node) = node.as_ref() else { return false };
            node.send_dm(&hex, envelope)
        };
        matches!(n0_future::time::timeout(HOLD_WAIT, send).await, Ok(Ok(())))
    }

    async fn deliver_dm(&self, to: UserId, envelope: &DmEnvelope) -> bool {
        let inboxed = self.post_inbox(to, envelope).await;
        self.send_direct(to, envelope).await || inboxed
    }

    async fn post_dm(&self, me: &UserProfile, partner: UserId, mut message: DmMessage) -> Result<Option<DmMessageView>, String> {
        let (_, identity) = self.me_and_key().ok_or("Your servers are not unlocked yet")?;
        let node = self.node_hex().ok_or("Your servers are not unlocked yet")?;
        message.sign(&node, &identity);
        if !self.dms.borrow_mut().sent(partner, message.clone()) {
            return Err(if message.is_post() { "message not sent" } else { "nothing to change" }.into());
        }
        self.save_safety_if_changed();
        self.emit("dm-sending", json!(partner.to_string()));
        let envelope = DmEnvelope::new(message.clone(), self.profile_for(me, partner));
        self.note_partner(partner, true);
        self.refresh_dms();
        let stored = self.append_pair(&identity, partner, &envelope).await;
        if stored {
            self.dms.borrow_mut().stored(message.id);
        }
        let held = stored && self.dms.borrow().follows(partner, Utc::now().timestamp_millis());
        let delivered = !held && self.deliver_dm(partner, &envelope).await;
        if delivered {
            self.dms.borrow_mut().delivered(message.id);
        }
        self.emit("dm-changed", json!(partner.to_string()));
        let book = self.dms.borrow();
        Ok(message.is_post().then(|| dm_message_view(&message, me.id, &book.store, !delivered && !held)))
    }

    async fn send_friend_signal(&self, to: UserId, signal: FriendSignal, note: Option<String>) -> Result<(), String> {
        let (me, identity) = self.me_and_key().ok_or("Your servers are not unlocked yet")?;
        let node = self.node_hex().ok_or("Your servers are not unlocked yet")?;
        let mut message = DmMessage::new(me.id, to, signal.encode());
        message.sign(&node, &identity);
        let sender = match signal {
            FriendSignal::Remove | FriendSignal::Decline => me.without_details(),
            FriendSignal::Request | FriendSignal::Accept => self.profile_for(&me, to),
        };
        let envelope = DmEnvelope::new(message, sender).with_note(note);
        if self.deliver_dm(to, &envelope).await {
            Ok(())
        } else {
            Err("they are offline right now; try again later".into())
        }
    }

    fn threads(&self) -> Vec<DmThreadView> {
        let Some((me, _)) = self.me_and_key() else { return Vec::new() };
        let client = self.client.borrow();
        let servers = client.servers.lock().unwrap();
        self.dms.borrow().threads(me.id, &servers)
    }

    async fn invite_to_group(&self, server: ServerId, user_ids: Vec<String>) -> Result<(), String> {
        let given = self.sync.borrow().as_ref().and_then(|live| live.book.given().remove(&server));
        let (ticket, key) = self.client.borrow().group_invite_from(server, given)?;
        let raw = ticket.encode();
        for user in user_ids {
            let (me, partner) = self.recipient(Some(&user))?;
            let already = self.client.borrow().servers.lock().unwrap().get(&server).is_some_and(|s| s.server.is_current_member(partner));
            if already {
                continue;
            }
            let invite = dms::group_invite(me.id, partner, raw.clone(), key.as_ref().map(|(id, secret)| (*id, secret)));
            self.post_dm(&me, partner, invite).await?;
        }
        Ok(())
    }

    fn friends_view(&self) -> Value {
        let client = self.client.borrow();
        let servers = client.servers.lock().unwrap();
        let book = self.dms.borrow();
        let people = |set: &std::collections::HashSet<UserId>| {
            let mut list: Vec<(UserId, String)> = set.iter().map(|id| (*id, dm_book::known_name(&book.store, &servers, *id))).collect();
            list.sort_by_key(|(_, name)| name.to_lowercase());
            list
        };
        let incoming: Vec<Value> = people(&book.safety.friend_in)
            .into_iter()
            .map(|(id, name)| json!({ "id": id.to_string(), "name": name, "note": book.safety.friend_notes.get(&id) }))
            .collect();
        json!({
            "friends": people(&book.safety.friends).into_iter().map(|(id, name)| named(id, name)).collect::<Vec<_>>(),
            "incoming": incoming,
            "outgoing": people(&book.safety.friend_out).into_iter().map(|(id, name)| named(id, name)).collect::<Vec<_>>(),
        })
    }

    fn safety_view(&self) -> Value {
        let client = self.client.borrow();
        let servers = client.servers.lock().unwrap();
        let book = self.dms.borrow();
        let blocked: Vec<Value> =
            book.safety.blocked.iter().map(|id| named(*id, dm_book::known_name(&book.store, &servers, *id))).collect();
        json!({
            "blocked": blocked,
            "dm_policy": book.safety.dm_policy,
            "spam_filter": book.safety.spam_filter,
            "profile_policy": book.safety.profile_policy,
        })
    }

    pub(super) async fn dm_command(self: &Rc<Self>, cmd: &str, args: Args) -> Result<Value, String> {
        match cmd {
            "list_dm_threads" => to_json(&self.threads()),
            "create_group_dm" => {
                let users = args.user_ids.unwrap_or_default();
                if users.is_empty() {
                    return Err("pick at least one person".into());
                }
                for user in &users {
                    self.recipient(Some(user))?;
                }
                let server = self.start_group().await?;
                self.invite_to_group(server, users).await?;
                to_json(&self.client.borrow().summary(server).ok_or("the group is gone")?)
            }
            "add_group_members" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                self.invite_to_group(server, args.user_ids.unwrap_or_default()).await.map(|_| Value::Null)
            }
            "get_dm_messages" => {
                let partner: UserId = parse(args.user_id.as_ref(), "user")?;
                let Some((me, _)) = self.me_and_key() else { return Ok(json!([])) };
                to_json(&self.dms.borrow().messages(me.id, partner, Utc::now().timestamp_millis()))
            }
            "open_dm" => {
                let partner: UserId = parse(args.user_id.as_ref(), "user")?;
                let (me, _) = self.me_and_key().ok_or("Your servers are not unlocked yet")?;
                if partner == me.id {
                    return Err("you cannot message yourself".into());
                }
                let profile = dms::member_profile(&self.client.borrow().servers.lock().unwrap(), partner);
                let mut book = self.dms.borrow_mut();
                if let Some(profile) = profile {
                    book.store.remember_contact(profile);
                }
                let name = book.store.contacts.get(&partner).map(|p| p.visible_name().to_string()).ok_or("unknown user")?;
                to_json(&DmThreadView {
                    user: partner.to_string(),
                    name,
                    last_message: String::new(),
                    last_at: String::new(),
                    kind: "inbox".to_string(),
                })
            }
            "send_dm" => {
                let content = clean_message(args.content.as_deref().unwrap_or(""))?;
                let (me, partner) = self.recipient(args.user_id.as_ref())?;
                let mut message = DmMessage::new(me.id, partner, content);
                if let Some(raw) = args.reply_to.as_ref() {
                    let target: MessageId = parse(Some(raw), "message")?;
                    if self.dms.borrow().store.post(partner, target).is_none() {
                        return Err("the message you replied to no longer exists".into());
                    }
                    message.reply_to = Some(target);
                }
                to_json(&self.post_dm(&me, partner, message).await?.ok_or("message not sent")?)
            }
            "edit_dm" => {
                let content = clean_message(args.content.as_deref().unwrap_or(""))?;
                let (me, partner) = self.partner(args.user_id.as_ref())?;
                let target = self.own_post(partner, args.message_id.as_ref(), me.id)?;
                let message = DmMessage::acting(me.id, partner, DmAction::Edit { target }, content);
                self.post_dm(&me, partner, message).await.map(|_| Value::Null)
            }
            "delete_dm" => {
                let (me, partner) = self.partner(args.user_id.as_ref())?;
                let target = self.own_post(partner, args.message_id.as_ref(), me.id)?;
                let message = DmMessage::acting(me.id, partner, DmAction::Delete { target }, "");
                self.post_dm(&me, partner, message).await.map(|_| Value::Null)
            }
            "set_dm_reaction" => {
                let emoji = args.emoji.as_deref().unwrap_or("").trim().to_string();
                if !commy_core::server::valid_reaction(&emoji) {
                    return Err("reactions must be a single emoji".into());
                }
                let (me, partner) = self.partner(args.user_id.as_ref())?;
                let target: MessageId = parse(args.message_id.as_ref(), "message")?;
                let active = args.active.unwrap_or(true);
                let message = DmMessage::acting(me.id, partner, DmAction::React { target, emoji, active }, "");
                self.post_dm(&me, partner, message).await.map(|_| Value::Null)
            }
            "accept_dm_request" => {
                let user: UserId = parse(args.user_id.as_ref(), "user")?;
                {
                    let mut book = self.dms.borrow_mut();
                    book.safety.accepted.insert(user);
                    book.safety.spam.remove(&user);
                }
                self.save_safety();
                self.note_partner(user, true);
                self.emit("dm-changed", json!(user.to_string()));
                Ok(Value::Null)
            }
            "decline_dm_request" => {
                let user: UserId = parse(args.user_id.as_ref(), "user")?;
                {
                    let mut book = self.dms.borrow_mut();
                    book.store.remove_conversation(user);
                    book.safety.spam.remove(&user);
                }
                self.save_safety();
                self.emit("dm-changed", json!(user.to_string()));
                Ok(Value::Null)
            }
            "get_safety" => Ok(self.safety_view()),
            "set_blocked" => {
                let user: UserId = parse(args.user_id.as_ref(), "user")?;
                if self.me_and_key().is_some_and(|(me, _)| me.id == user) {
                    return Err("you cannot block yourself".into());
                }
                if self.dms.borrow_mut().safety.set_blocked(user, args.blocked.unwrap_or(true)) {
                    self.save_safety();
                    self.emit("safety-changed", Value::Null);
                }
                Ok(Value::Null)
            }
            "set_dm_settings" => {
                let policy = DmPolicy::parse(args.policy.as_deref().unwrap_or("")).ok_or("unknown DM setting")?;
                {
                    let mut book = self.dms.borrow_mut();
                    book.safety.dm_policy = policy;
                    book.safety.spam_filter = args.spam_filter.unwrap_or(true);
                }
                self.save_safety();
                Ok(Value::Null)
            }
            "list_friends" => Ok(self.friends_view()),
            "friend_action" => {
                let user: UserId = parse(args.user_id.as_ref(), "user")?;
                if self.me_and_key().is_some_and(|(me, _)| me.id == user) {
                    return Err("you cannot friend yourself".into());
                }
                let signal = match args.action.as_deref() {
                    Some("request") => FriendSignal::Request,
                    Some("accept") => FriendSignal::Accept,
                    Some("decline") => FriendSignal::Decline,
                    Some("remove") => FriendSignal::Remove,
                    _ => return Err("unknown friend action".into()),
                };
                if signal == FriendSignal::Request && self.dms.borrow().safety.is_blocked(user) {
                    return Err("unblock them first".into());
                }
                let note = match (signal, args.note.as_deref().map(str::trim)) {
                    (FriendSignal::Request, Some(raw)) if !raw.is_empty() => Some(
                        safety::clean_friend_note(raw)
                            .ok_or_else(|| format!("a note can be up to {} characters", safety::MAX_FRIEND_NOTE_CHARS))?,
                    ),
                    _ => None,
                };
                self.send_friend_signal(user, signal, note).await?;
                {
                    let mut book = self.dms.borrow_mut();
                    let s = &mut book.safety;
                    s.friend_notes.remove(&user);
                    match signal {
                        FriendSignal::Request => {
                            if s.friend_in.remove(&user) {
                                s.friends.insert(user);
                            } else {
                                s.friend_out.insert(user);
                            }
                        }
                        FriendSignal::Accept => {
                            if s.friend_in.remove(&user) {
                                s.friends.insert(user);
                            }
                        }
                        FriendSignal::Decline => {
                            s.friend_in.remove(&user);
                        }
                        FriendSignal::Remove => {
                            s.friends.remove(&user);
                            s.friend_out.remove(&user);
                        }
                    }
                }
                self.save_safety();
                self.emit("friends-changed", json!({ "user": user.to_string() }));
                Ok(Value::Null)
            }
            _ => Err(format!("{cmd} is not available in the web app yet")),
        }
    }
}
