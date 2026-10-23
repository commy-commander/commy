use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use chrono::Utc;
use commy_core::{ChannelId, MessageId, ServerEvent, ServerId};
use commy_net::entry_list::EntryList;
use commy_net::sealed::SealKey;
use commy_net::{
    CommyNode, EntryKeyReply, EntryKeyRequest, MirrorRequest, NetEvent, ServerTicket, SharedCatchupStore, WirePayload,
};
use iroh::{RelayConfig, RelayMap, RelayUrl};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{mpsc, Mutex};
use wasm_bindgen_futures::spawn_local;

use commy_view::act;
use commy_view::catalog;
use commy_view::pins::Pins;
use commy_view::search::{Page, SearchQuery};

use crate::account::{self, AccountUser, Reply};
use crate::client::{Applied, Client, PollForm};
use crate::host::Host;
use crate::store::WebStore;

mod handoff;
mod sync;

const DEVICE_NAME: &str = "Web browser";
const MAX_PREF_KEY: usize = 200;
const MAX_PREFS_PER_CALL: usize = 500;
const FAREWELL_LINGER: std::time::Duration = std::time::Duration::from_millis(300);

#[derive(Debug, Clone, Copy, PartialEq)]
enum Unlock {
    SignedOut,
    NoList,
    Locked,
    Asking,
    Unlocked,
}

impl Unlock {
    fn name(self) -> &'static str {
        match self {
            Unlock::SignedOut => "signed_out",
            Unlock::NoList => "no_list",
            Unlock::Locked => "locked",
            Unlock::Asking => "asking",
            Unlock::Unlocked => "unlocked",
        }
    }
}

struct Status {
    unlock: Unlock,
    detail: String,
    account: Option<AccountUser>,
    device_linked: bool,
    sealed: Option<String>,
    features: Option<account::Features>,
}

pub struct Session {
    host: Host,
    origin: String,
    relays: Vec<String>,
    client: RefCell<Client>,
    status: RefCell<Status>,
    node: Mutex<Option<CommyNode>>,
    store: RefCell<Option<SharedCatchupStore>>,
    sync: RefCell<Option<sync::Live>>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Args {
    server_id: Option<String>,
    entry: Option<commy_view::saved::Entry>,
    user_id: Option<String>,
    saved: Option<bool>,
    channel_id: Option<String>,
    content: Option<String>,
    reply_to: Option<String>,
    around: Option<String>,
    code: Option<String>,
    message_id: Option<String>,
    root_id: Option<String>,
    emoji: Option<String>,
    active: Option<bool>,
    pinned: Option<bool>,
    choices: Option<Vec<u8>>,
    name: Option<String>,
    archived: Option<bool>,
    locked: Option<bool>,
    question: Option<String>,
    options: Option<Vec<String>>,
    multi: Option<bool>,
    hours: Option<u32>,
    terms: Option<Vec<String>>,
    from: Option<String>,
    query: Option<String>,
    page: Option<u32>,
    category: Option<u32>,
    mentions: Option<String>,
    offset: Option<usize>,
    channels: Option<std::collections::BTreeMap<String, commy_view::reads::ReadMark>>,
    channel_ids: Option<Vec<String>>,
    changes: Option<std::collections::BTreeMap<String, Value>>,
}

fn parse<T: std::str::FromStr>(raw: Option<&String>, what: &str) -> Result<T, String> {
    raw.ok_or_else(|| format!("missing {what}"))?.parse().map_err(|_| format!("invalid {what}"))
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

impl Session {
    pub fn new(host: Host, origin: String, relays: Option<String>) -> Rc<Self> {
        Rc::new(Self {
            host,
            origin,
            relays: commy_net::relays::own_relays_from(relays.as_deref()),
            client: RefCell::new(Client::default()),
            status: RefCell::new(Status {
                unlock: Unlock::SignedOut,
                detail: String::new(),
                account: None,
                device_linked: false,
                sealed: None,
                features: None,
            }),
            node: Mutex::new(None),
            store: RefCell::new(None),
            sync: RefCell::new(None),
        })
    }

    pub async fn invoke(self: &Rc<Self>, cmd: &str, args: &str) -> Result<Value, String> {
        let args: Args = serde_json::from_str(args).unwrap_or_default();
        match cmd {
            "account_status" => self.account_status().await,
            "account_sign_out" => self.sign_out().await,
            "web_unlock" => self.unlock().await,
            "web_ask_devices" => self.ask_devices().await,
            "entry_list_use_recovery_code" => self.use_recovery_code(args.code.as_deref().unwrap_or("")).await,
            "get_user" => to_json(&self.client.borrow().profile()),
            "list_servers" => to_json(&self.client.borrow().list_servers()),
            "get_server" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                to_json(&self.client.borrow().get_server(server)?)
            }
            "get_messages" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                let channel: ChannelId = parse(args.channel_id.as_ref(), "channel")?;
                let around: Option<MessageId> = args.around.as_ref().map(|a| parse(Some(a), "message")).transpose()?;
                to_json(&self.client.borrow().get_messages(server, channel, around)?)
            }
            "send_message" => self.send_message(args).await,
            "send_poll" => self.send_poll(args).await,
            "read_state_load" | "read_state_sync" => to_json(&self.read_view(None)),
            "read_state_set" => {
                self.note_reads(args.channels.unwrap_or_default());
                Ok(Value::Null)
            }
            "read_state_mark_read" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                let marks = self.client.borrow().latest_marks(server, args.channel_ids.as_deref(), Utc::now().timestamp_millis())?;
                let touched: Vec<String> = marks.keys().cloned().collect();
                self.note_reads(marks);
                to_json(&self.read_view(Some(&touched)))
            }
            "sync_prefs_load" => to_json(&self.prefs()),
            "sync_prefs_set" => {
                let at = Utc::now().timestamp_millis();
                let set = args
                    .changes
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|(key, _)| !key.is_empty() && key.len() <= MAX_PREF_KEY)
                    .take(MAX_PREFS_PER_CALL)
                    .map(|(key, value)| (key, commy_view::reads::PrefPiece { value, at }))
                    .collect();
                self.note_prefs(set);
                Ok(Value::Null)
            }
            "search_messages" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                let user = |raw: Option<&String>| raw.map(|u| parse::<commy_core::UserId>(Some(u), "user")).transpose();
                let channel: Option<ChannelId> = args.channel_id.as_ref().map(|c| parse(Some(c), "channel")).transpose()?;
                let query = SearchQuery::new(&args.terms.unwrap_or_default(), user(args.from.as_ref())?, user(args.mentions.as_ref())?, channel);
                let page = args.offset.map_or(Page::newest(), Page::at);
                to_json(&self.client.borrow().search(server, &query, page)?)
            }
            "gif_search" => {
                let params = [("q", catalog::gif_query(args.query.as_deref().unwrap_or("")))];
                let found: Option<catalog::Found<catalog::GifHit>> = self.search("gifs", &params).await?;
                to_json(&found.map(|found| catalog::safe_gifs(found.results)))
            }
            "sticker_search" => {
                let params = catalog::sticker_params(args.query.as_deref().unwrap_or(""), args.from.as_deref(), args.page);
                let found: Option<catalog::StickerPage> = self.search("stickers", &params).await?;
                to_json(&found.map(catalog::safe_stickers))
            }
            "search_emoji_catalog" => {
                let params = catalog::emoji_params(args.query.as_deref().unwrap_or(""), args.category, args.page);
                let found: Option<catalog::EmojiPage> = self.search("emojis", &params).await?;
                to_json(&catalog::safe_emojis(found.ok_or("This site does not offer emoji search")?))
            }
            "pins_load" => to_json(&self.pins()),
            "pin_channel" => {
                let (server, channel) = (args.server_id.unwrap_or_default(), args.channel_id.unwrap_or_default());
                to_json(&self.change_pins(|pins| pins.set_channel(&server, &channel, args.pinned.unwrap_or(true)))?)
            }
            "pin_dm" => {
                let user = args.user_id.unwrap_or_default();
                to_json(&self.change_pins(|pins| pins.set_dm(&user, args.pinned.unwrap_or(true)))?)
            }
            "saved_list" => to_json(&self.saved()),
            "saved_set" => {
                let entry: commy_view::saved::Entry = args.entry.ok_or("Pick a message to save")?;
                to_json(&self.set_saved(entry, args.saved.unwrap_or(true))?)
            }
            "get_pins" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                let channel: Option<ChannelId> = args.channel_id.as_ref().map(|c| parse(Some(c), "channel")).transpose()?;
                to_json(&self.client.borrow().get_pins(server, channel)?)
            }
            "send_typing" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                let channel: ChannelId = parse(args.channel_id.as_ref(), "channel")?;
                if let Some(payload) = self.client.borrow().typing(server, channel)? {
                    self.publish(server, &payload);
                }
                Ok(Value::Null)
            }
            "edit_message" | "delete_message" | "set_reaction" | "pin_message" | "vote_poll" | "close_poll" | "set_thread" => {
                self.act(cmd, args)
            }
            "web_nudge" => {
                self.nudge_sync();
                if self.said_goodbye() {
                    self.say_here_everywhere();
                }
                Ok(Value::Null)
            }
            "web_farewell" => {
                self.say_goodbye();
                Ok(Value::Null)
            }
            _ => Err(format!("{cmd} is not available in the web app yet")),
        }
    }

    fn emit(&self, event: &str, payload: Value) {
        self.host.emit(event, &payload.to_string());
    }

    async fn search<T: serde::de::DeserializeOwned>(&self, kind: &str, params: &[(&str, String)]) -> Result<Option<T>, String> {
        let reply = self.request("GET", &account::search_path(kind, params), None).await?;
        account::search_reply(&reply)
    }

    async fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<Reply, String> {
        let raw = self
            .host
            .request(method, path, body.map(|b| b.to_string()))
            .await
            .map_err(|_| "Could not reach your account server".to_string())?;
        raw.as_string().and_then(|raw| Reply::parse(&raw)).ok_or_else(|| "Unexpected reply from your account server".into())
    }

    fn host_name(&self) -> String {
        self.origin.split("://").nth(1).unwrap_or(&self.origin).to_string()
    }

    async fn account_status(&self) -> Result<Value, String> {
        let reply = self.request("GET", account::SESSION_PATH, None).await?;
        let user = account::parse_session(&reply);
        let known = self.status.borrow().features;
        let offers = match known {
            Some(features) => features,
            None => {
                let features = account::features(&self.request("GET", account::DISCOVERY_PATH, None).await?);
                self.status.borrow_mut().features = Some(features);
                features
            }
        };
        let offers = if user.is_some() { offers } else { account::Features::default() };
        let mut status = self.status.borrow_mut();
        status.account = user.clone();
        if user.is_none() {
            status.unlock = Unlock::SignedOut;
        }
        Ok(json!({
            "signed_in": user.is_some(),
            "user": user,
            "server": self.origin,
            "server_host": self.host_name(),
            "is_default_server": true,
            "default_server_host": self.host_name(),
            "server_from_env": true,
            "device_linked": status.device_linked,
            "pending_code": null,
            "server_gifs": offers.gifs,
            "server_games": offers.games,
            "server_emojis": offers.emojis,
            "server_stickers": offers.stickers,
            "server_media": offers.media,
        }))
    }

    async fn sign_out(&self) -> Result<Value, String> {
        self.say_goodbye();
        n0_future::time::sleep(FAREWELL_LINGER).await;
        self.stop_sync();
        let _ = self.request("POST", account::SIGN_OUT_PATH, Some(json!({}))).await;
        self.host.remove(account::ENTRY_KEY_KEY);
        self.account_status().await
    }

    fn unlock_view(&self) -> Value {
        let status = self.status.borrow();
        json!({
            "state": status.unlock.name(),
            "detail": status.detail,
            "servers": self.client.borrow().list_servers().len(),
        })
    }

    fn set_unlock(&self, unlock: Unlock, detail: impl Into<String>) {
        {
            let mut status = self.status.borrow_mut();
            status.unlock = unlock;
            status.detail = detail.into();
        }
        self.emit("web-unlock-changed", self.unlock_view());
    }

    fn pins(&self) -> Pins {
        let Ok(account) = self.account() else { return Pins::default() };
        self.host.load(&account::pins_key(&account.id)).and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or_default()
    }

    fn change_pins(&self, edit: impl FnOnce(&mut Pins) -> bool) -> Result<Pins, String> {
        let account = self.account()?;
        let mut pins = self.pins();
        if edit(&mut pins) {
            self.host.save(&account::pins_key(&account.id), &serde_json::to_string(&pins).map_err(|e| e.to_string())?);
        }
        Ok(pins)
    }

    fn account(&self) -> Result<AccountUser, String> {
        self.status.borrow().account.clone().ok_or_else(|| "Sign in first".to_string())
    }

    fn device_secret(&self, account: &AccountUser) -> [u8; 32] {
        let key = account::device_secret_key(&account.id);
        if let Some(secret) = account::parse_device_secret(self.host.load(&key).as_deref()) {
            return secret;
        }
        let shared = self.host.load(account::DEVICE_SECRET_KEY);
        let ours = account::shared_secret_belongs_to(self.host.load(account::ENTRY_KEY_KEY).as_deref(), &account.id);
        let secret = match account::parse_device_secret(shared.as_deref()) {
            Some(secret) if ours => secret,
            _ => CommyNode::generate_secret(),
        };
        if ours {
            self.host.remove(account::DEVICE_SECRET_KEY);
        }
        self.host.save(&key, &data_encoding::HEXLOWER.encode(&secret));
        secret
    }

    async fn unlock(self: &Rc<Self>) -> Result<Value, String> {
        if self.status.borrow().unlock == Unlock::Unlocked {
            return Ok(self.unlock_view());
        }
        if self.status.borrow().account.is_none() {
            self.account_status().await?;
        }
        let account = match self.account() {
            Ok(account) => account,
            Err(_) => {
                self.set_unlock(Unlock::SignedOut, "Sign in first");
                return Ok(self.unlock_view());
            }
        };
        let secret = self.device_secret(&account);
        self.start_node(secret).await.ok();
        self.link_device(secret).await?;
        let reply = self.request("GET", account::VAULT_PATH, None).await?;
        let Some(sealed) = account::vault_data(&reply)? else {
            self.set_unlock(Unlock::NoList, "Open Commy on your computer and sign in there first. The web app opens the servers your computer has.");
            return Ok(self.unlock_view());
        };
        self.status.borrow_mut().sealed = Some(sealed.clone());
        let stored = account::stored_entry_key(self.host.load(account::ENTRY_KEY_KEY).as_deref(), &account.id);
        if let Some((key, list)) = stored.and_then(|key| Some((key.clone(), EntryList::open(&key, &sealed).ok()?))) {
            self.finish(list, key)?;
            return Ok(self.unlock_view());
        }
        if let Some(key) = self.escrowed_key().await.filter(|key| EntryList::open(key, &sealed).is_ok()) {
            self.accept_key(&account, key)?;
            return Ok(self.unlock_view());
        }
        self.set_unlock(Unlock::Locked, "Approve this browser on one of your Commy devices, or enter your recovery code.");
        Ok(self.unlock_view())
    }

    async fn start_node(self: &Rc<Self>, secret: [u8; 32]) -> Result<(), String> {
        let mut slot = self.node.lock().await;
        if slot.is_some() {
            return Ok(());
        }
        let (mut node, events, _media, _dms) = CommyNode::spawn_with(Some(secret), Some(relay_map_of(&self.relays)?))
            .await
            .map_err(|e| format!("Could not connect to the Commy network: {e}"))?;
        let (changed_tx, changed_rx) = mpsc::unbounded_channel();
        let store: SharedCatchupStore = Arc::new(WebStore {
            servers: self.client.borrow().servers.clone(),
            me: self.client.borrow().me.clone(),
            changed: changed_tx,
        });
        node.set_catchup_store(store.clone());
        let _ = node.take_entry_key_requests();
        let _ = node.take_mirror_requests();
        *self.store.borrow_mut() = Some(store);
        *slot = Some(node);
        spawn_local(self.clone().run_events(events));
        spawn_local(self.clone().run_changes(changed_rx));
        Ok(())
    }

    async fn link_device(&self, secret: [u8; 32]) -> Result<(), String> {
        let key = iroh::SecretKey::from_bytes(&secret);
        let node_id = node_id_of(&secret);
        let reply = self.request("POST", account::LINK_CHALLENGE_PATH, Some(json!({ "nodeId": node_id }))).await?;
        let challenge = account::parse_challenge(&reply, &self.origin, &node_id)?;
        let signature = data_encoding::HEXLOWER.encode(&key.sign(challenge.message.as_bytes()).to_bytes());
        let body = json!({ "nodeId": node_id, "challenge": challenge.challenge, "signature": signature });
        let reply = self.request("POST", account::LINK_REGISTER_PATH, Some(body)).await?;
        if !reply.ok() {
            return Err("Your account server could not verify this browser".into());
        }
        self.status.borrow_mut().device_linked = true;
        Ok(())
    }

    async fn other_devices(&self) -> Result<Vec<String>, String> {
        let own = self.account().map(|account| node_id_of(&self.device_secret(&account))).unwrap_or_default();
        let reply = self.request("GET", account::DEVICES_PATH, None).await?;
        Ok(account::other_devices(&reply, &own))
    }

    async fn ask_devices(self: &Rc<Self>) -> Result<Value, String> {
        let account = self.account()?;
        let devices = self.other_devices().await?;
        if devices.is_empty() {
            self.set_unlock(Unlock::Locked, "No other device is linked to your account. Enter your recovery code instead.");
            return Ok(self.unlock_view());
        }
        let node = commy_core::Identity::from_bytes(&self.device_secret(&account));
        let code = commy_net::key_handoff::code(&node.public());
        self.set_unlock(Unlock::Asking, format!("Open Commy on one of your other devices and choose Allow. It will show the code {code}."));
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sync = self.sync_service_url().await;
        if let Some(url) = sync.clone() {
            let (tx, account) = (tx.clone(), account.id.clone());
            spawn_local(async move {
                let got = handoff::ask(&url, &account, &node, DEVICE_NAME).await;
                let _ = tx.send(got.map(|key| EntryKeyReply::Key(key.to_text())));
            });
        }
        let endpoint = match sync {
            Some(_) => None,
            None => self.node.lock().await.as_ref().map(|node| {
                node.reach_via_relays(&devices);
                node.endpoint()
            }),
        };
        if let Some(endpoint) = endpoint {
            let request = EntryKeyRequest { server: self.origin.clone(), account: account.id.clone(), device_name: DEVICE_NAME.into() };
            for device in &devices {
                let (tx, endpoint, request, device) = (tx.clone(), endpoint.clone(), request.clone(), device.clone());
                spawn_local(async move {
                    let _ = tx.send(commy_net::entry_key::request(endpoint, &device, &request).await.ok());
                });
            }
        }
        drop(tx);
        while let Some(reply) = rx.recv().await {
            if self.status.borrow().unlock == Unlock::Unlocked {
                return Ok(self.unlock_view());
            }
            if let Some(EntryKeyReply::Key(text)) = reply {
                if let Ok(key) = SealKey::from_text(&text) {
                    if self.accept_key(&account, key).is_ok() {
                        return Ok(self.unlock_view());
                    }
                }
            }
        }
        if self.status.borrow().unlock != Unlock::Unlocked {
            self.set_unlock(Unlock::Locked, "None of your devices approved this browser yet. Ask again, or enter your recovery code.");
        }
        Ok(self.unlock_view())
    }

    async fn sync_service_url(&self) -> Option<String> {
        let reply = self.request("GET", account::DISCOVERY_PATH, None).await.ok()?;
        account::sync_url(&reply, &self.origin)
    }

    async fn use_recovery_code(self: &Rc<Self>, code: &str) -> Result<Value, String> {
        let account = self.account()?;
        let key = SealKey::from_text(code.trim()).map_err(|_| "That is not a recovery code".to_string())?;
        self.accept_key(&account, key.clone())?;
        if let Some(url) = self.sync_service_url().await {
            let me = commy_core::Identity::from_bytes(&self.device_secret(&account));
            spawn_local(async move { handoff::tell_devices_we_have_the_key(&url, &account.id, &me, &key).await });
        }
        Ok(self.unlock_view())
    }

    fn accept_key(self: &Rc<Self>, account: &AccountUser, key: SealKey) -> Result<(), String> {
        let sealed = self.status.borrow().sealed.clone().ok_or("Your server list has not loaded yet")?;
        let list = EntryList::open(&key, &sealed)?;
        self.host.save(account::ENTRY_KEY_KEY, &account::entry_key_record(&account.id, &key));
        let this = self.clone();
        let kept = key.clone();
        spawn_local(async move { this.escrow(&kept).await });
        self.finish(list, key)
    }

    async fn wrapping_key(&self) -> Option<SealKey> {
        let body = json!({ "lockId": commy_net::entry_escrow::LOCK_ID });
        let reply = self.request("POST", account::FOLDER_KEY_PATH, Some(body)).await.ok()?;
        commy_net::entry_escrow::wrapping_key(&account::folder_key(&reply)?)
    }

    async fn escrowed_key(&self) -> Option<SealKey> {
        let reply = self.request("GET", &account::escrow_path(), None).await.ok()?;
        let sealed = account::vault_data(&reply).ok()??;
        commy_net::entry_escrow::unwrap(&self.wrapping_key().await?, &sealed)
    }

    async fn escrow(&self, key: &SealKey) {
        let Ok(reply) = self.request("GET", &account::escrow_path(), None).await else { return };
        let Some(version) = account::vault_version(&reply) else { return };
        let Some(wrapping) = self.wrapping_key().await else { return };
        let held = account::vault_data(&reply).ok().flatten().and_then(|sealed| commy_net::entry_escrow::unwrap(&wrapping, &sealed));
        if held.as_ref() == Some(key) {
            return;
        }
        let body = json!({ "name": commy_net::entry_escrow::VAULT_ITEM, "data": commy_net::entry_escrow::wrap(&wrapping, key), "version": version });
        let _ = self.request("POST", account::VAULT_PUT_PATH, Some(body)).await;
    }

    fn finish(self: &Rc<Self>, list: EntryList, key: SealKey) -> Result<(), String> {
        let identity = list.shared_identity().ok_or(
            "Your server list does not carry your identity yet. Update Commy on your computer and let it sync.",
        )?;
        let mut profile = list.profile.clone().ok_or("Your server list has no profile yet")?;
        profile.identity = Some(identity.public());
        let tickets: Vec<ServerTicket> = list.servers.values().filter_map(|e| ServerTicket::decode(&e.ticket).ok()).collect();
        {
            let mut client = self.client.borrow_mut();
            *client.me.lock().unwrap() = Some((profile, identity));
            client.tickets = tickets.iter().map(|t| (t.server_id, t.clone())).collect();
        }
        self.set_unlock(Unlock::Unlocked, "");
        spawn_local(self.clone().load_servers(tickets));
        spawn_local(self.clone().start_sync(key, list));
        Ok(())
    }

    fn announce_if_loaded(&self) {
        let loaded = {
            let client = self.client.borrow();
            client.tickets.keys().all(|server| client.has(*server))
        };
        if loaded {
            self.emit("web-servers-loaded", json!(self.client.borrow().list_servers().len()));
        }
    }

    async fn load_servers(self: Rc<Self>, tickets: Vec<ServerTicket>) {
        self.announce_if_loaded();
        let devices = self.other_devices().await.unwrap_or_default();
        if let Some(node) = self.node.lock().await.as_mut() {
            for ticket in &tickets {
                if node.rejoin_server(ticket).await.is_ok() {
                    let _ = node.join_peers(ticket.server_id, &devices).await;
                }
            }
        }
        let wanted: Vec<ServerId> = tickets.iter().map(|t| t.server_id).collect();
        for device in &devices {
            self.mirror_from(device, &wanted).await;
        }
        let missing: Vec<ServerId> = wanted.into_iter().filter(|id| !self.client.borrow().has(*id)).collect();
        self.emit("web-servers-loaded", json!(self.client.borrow().list_servers().len()));
        let node = self.node.lock().await;
        if let Some(node) = node.as_ref() {
            for server in missing {
                let _ = node.broadcast(server, WirePayload::SyncRequest).await;
            }
        }
    }

    async fn mirror_from(self: &Rc<Self>, device: &str, wanted: &[ServerId]) {
        let missing: Vec<ServerId> = wanted.iter().copied().filter(|id| !self.client.borrow().has(*id)).collect();
        if missing.is_empty() {
            return;
        }
        let Some(identity) = self.client.borrow().me.lock().unwrap().as_ref().map(|(_, identity)| identity.clone()) else {
            return;
        };
        let (endpoint, own) = {
            let node = self.node.lock().await;
            let Some(node) = node.as_ref() else { return };
            (node.endpoint(), node.node_id_bytes())
        };
        let Some(responder) = data_encoding::HEXLOWER_PERMISSIVE.decode(device.as_bytes()).ok().and_then(|b| b.try_into().ok()) else {
            return;
        };
        let request = MirrorRequest::new(&identity, own, responder, Utc::now().timestamp_millis(), missing);
        let Ok(copies) = commy_net::mirror::fetch(&endpoint, device, &request).await else { return };
        let mut installed = false;
        for copy in copies {
            installed |= self.client.borrow().install_mirror(copy.server, &copy.snapshot).unwrap_or(false);
        }
        if installed {
            self.emit("servers-changed", Value::Null);
            self.announce_if_loaded();
            self.refresh_sync();
            let node = self.node.lock().await;
            if let Some(node) = node.as_ref() {
                for server in wanted.iter().filter(|s| node.is_joined(**s)) {
                    self.catch_up(node, *server, responder);
                }
            }
        }
    }

    fn catch_up(self: &Rc<Self>, node: &CommyNode, server: ServerId, peer: [u8; 32]) {
        let Some(store) = self.store.borrow().clone() else { return };
        if !self.client.borrow().has(server) {
            return;
        }
        let session = node.catch_up(server, peer, store);
        let this = self.clone();
        spawn_local(async move {
            if let Ok(report) = session.await {
                this.settle_delivered(server, &report.delivered);
                this.emit("server-event", json!(server.to_string()));
            }
        });
    }

    fn settle_delivered(&self, server: ServerId, delivered: &[MessageId]) {
        let delivered: HashSet<&MessageId> = delivered.iter().collect();
        let mut client = self.client.borrow_mut();
        let before = client.pending.len();
        client.pending.retain(|id| !delivered.contains(id));
        if client.pending.len() != before {
            drop(client);
            self.emit("server-event", json!(server.to_string()));
        }
    }

    async fn run_events(self: Rc<Self>, mut events: mpsc::UnboundedReceiver<NetEvent>) {
        while let Some(event) = events.recv().await {
            match event {
                NetEvent::Event { server, event, action, .. } => {
                    if self.client.borrow().admits(server, &event, action) {
                        self.client.borrow().hear_peer(server, &event, action.as_ref());
                    }
                    self.apply_event(server, event, action, true, true);
                }
                NetEvent::SyncReceived { server, payload } => {
                    let installed = self.client.borrow().install_owner_snapshot(server, payload).unwrap_or(false);
                    if installed {
                        self.emit("servers-changed", Value::Null);
                        self.announce_if_loaded();
                        self.refresh_sync();
                        let node = self.node.lock().await;
                        if let Some(node) = node.as_ref() {
                            for peer in node.neighbors(server) {
                                self.catch_up(node, server, peer);
                            }
                        }
                    }
                }
                NetEvent::NeighborUp { server, peer } => {
                    let node = self.node.lock().await;
                    let Some(node) = node.as_ref() else { continue };
                    if self.client.borrow().has(server) {
                        self.catch_up(node, server, peer);
                    } else {
                        let _ = node.broadcast(server, WirePayload::SyncRequest).await;
                    }
                }
                NetEvent::CaughtUp { server, report, .. } => {
                    self.settle_delivered(server, &report.delivered);
                    self.emit("server-event", json!(server.to_string()));
                }
                _ => {}
            }
        }
    }

    fn apply_event(&self, server: ServerId, event: ServerEvent, action: Option<commy_core::Action>, rings: bool, shown: bool) -> Applied {
        let applied = self.client.borrow().apply_live(server, event, action);
        self.after_apply(server, applied, rings, shown)
    }

    fn apply_stored(&self, server: ServerId, event: ServerEvent, action: Option<commy_core::Action>, rings: bool, shown: bool) -> Applied {
        let applied = self.client.borrow().apply_stored(server, event, action);
        self.after_apply(server, applied, rings, shown)
    }

    fn after_apply(&self, server: ServerId, applied: Applied, rings: bool, shown: bool) -> Applied {
        if let Applied::Created(message) = &applied {
            if let Some(payload) = self.client.borrow().message_created_payload(server, message).filter(|_| rings) {
                self.emit("message-created", payload);
            }
        }
        if applied != Applied::Nothing && shown {
            self.emit("server-event", json!(server.to_string()));
        }
        applied
    }

    async fn run_changes(self: Rc<Self>, mut changed: mpsc::UnboundedReceiver<ServerId>) {
        while let Some(server) = changed.recv().await {
            self.emit("server-event", json!(server.to_string()));
        }
    }

    fn act(&self, cmd: &str, args: Args) -> Result<Value, String> {
        let server: ServerId = parse(args.server_id.as_ref(), "server")?;
        let channel = || parse::<ChannelId>(args.channel_id.as_ref(), "channel");
        let message = || parse::<MessageId>(args.message_id.as_ref(), "message");
        let payload = match cmd {
            "edit_message" => {
                let (channel, message) = (channel()?, message()?);
                let content = crate::client::clean_message(args.content.as_deref().unwrap_or(""))?;
                self.client.borrow().act(server, |state, me, identity, keys| {
                    act::edit(state, me, identity, keys, channel, message, content)
                })?
            }
            "delete_message" => {
                let (channel, message) = (channel()?, message()?);
                self.client.borrow().act(server, |state, me, identity, _| act::delete(state, me, identity, channel, message))?
            }
            "set_reaction" => {
                let (channel, message) = (channel()?, message()?);
                let emoji = args.emoji.unwrap_or_default();
                let active = args.active.unwrap_or(true);
                self.client.borrow().act(server, |state, me, _, _| act::reaction(state, me, channel, message, &emoji, active))?
            }
            "pin_message" => {
                let (channel, message) = (channel()?, message()?);
                let pinned = args.pinned.unwrap_or(true);
                self.client.borrow().act(server, |state, me, _, _| act::pin(state, me, channel, message, pinned))?
            }
            "vote_poll" => {
                let message = message()?;
                let choices = args.choices.unwrap_or_default();
                self.client.borrow().act(server, |state, me, _, _| act::poll_vote(state, me, message, &choices))?
            }
            "close_poll" => {
                let message = message()?;
                self.client.borrow().act(server, |state, me, _, _| act::poll_close(state, me, message))?
            }
            "set_thread" => {
                let channel = channel()?;
                let root: MessageId = parse(args.root_id.as_ref(), "thread")?;
                let (archived, locked) = (args.archived.unwrap_or(false), args.locked.unwrap_or(false));
                let name = args.name;
                self.client.borrow().act(server, |state, me, _, _| act::thread(state, me, channel, root, name.as_deref(), archived, locked))?
            }
            _ => return Err(format!("{cmd} is not available in the web app yet")),
        };
        self.publish(server, &payload);
        self.emit("server-event", json!(server.to_string()));
        Ok(Value::Null)
    }

    async fn send_message(self: &Rc<Self>, args: Args) -> Result<Value, String> {
        let server: ServerId = parse(args.server_id.as_ref(), "server")?;
        let channel: ChannelId = parse(args.channel_id.as_ref(), "channel")?;
        let reply_to: Option<MessageId> = args.reply_to.as_ref().map(|r| parse(Some(r), "message")).transpose()?;
        let (message, view) = self.client.borrow().compose(server, channel, args.content.as_deref().unwrap_or(""), reply_to)?;
        self.deliver(server, message, view).await
    }

    async fn send_poll(self: &Rc<Self>, args: Args) -> Result<Value, String> {
        let server: ServerId = parse(args.server_id.as_ref(), "server")?;
        let channel: ChannelId = parse(args.channel_id.as_ref(), "channel")?;
        let form = PollForm {
            question: args.question.unwrap_or_default(),
            options: args.options.unwrap_or_default(),
            multi: args.multi.unwrap_or(false),
            hours: args.hours,
        };
        let (message, view) = self.client.borrow().compose_poll(server, channel, form)?;
        self.deliver(server, message, view).await
    }

    async fn deliver(self: &Rc<Self>, server: ServerId, message: commy_core::Message, view: commy_view::MessageView) -> Result<Value, String> {
        let delivered = {
            let node = self.node.lock().await;
            match node.as_ref() {
                Some(node) if node.neighbor_count(server) > 0 => node
                    .broadcast(server, WirePayload::Event(ServerEvent::MessageCreate { message: message.clone() }))
                    .await
                    .is_ok(),
                _ => false,
            }
        };
        if !delivered {
            self.client.borrow_mut().pending.insert(message.id);
        }
        self.send_to_service(server, &message);
        self.emit("server-event", json!(server.to_string()));
        to_json(&view.with_pending(!delivered))
    }
}

fn node_id_of(secret: &[u8; 32]) -> String {
    data_encoding::HEXLOWER.encode(iroh::SecretKey::from_bytes(secret).public().as_bytes())
}

fn relay_map_of(relays: &[String]) -> Result<RelayMap, String> {
    let map = RelayMap::empty();
    for url in relays {
        let url: RelayUrl = url.parse().map_err(|_| format!("invalid relay {url}"))?;
        map.insert(url.clone(), Arc::new(RelayConfig::new(url, None)));
    }
    Ok(map)
}
