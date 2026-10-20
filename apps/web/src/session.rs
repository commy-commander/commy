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

use crate::account::{self, AccountUser, Reply};
use crate::client::{Applied, Client};
use crate::host::Host;
use crate::store::WebStore;

const DEVICE_NAME: &str = "Web browser";

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
}

pub struct Session {
    host: Host,
    origin: String,
    client: RefCell<Client>,
    status: RefCell<Status>,
    node: Mutex<Option<CommyNode>>,
    store: RefCell<Option<SharedCatchupStore>>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Args {
    server_id: Option<String>,
    channel_id: Option<String>,
    content: Option<String>,
    reply_to: Option<String>,
    around: Option<String>,
    code: Option<String>,
}

fn parse<T: std::str::FromStr>(raw: Option<&String>, what: &str) -> Result<T, String> {
    raw.ok_or_else(|| format!("missing {what}"))?.parse().map_err(|_| format!("invalid {what}"))
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

impl Session {
    pub fn new(host: Host, origin: String) -> Rc<Self> {
        Rc::new(Self {
            host,
            origin,
            client: RefCell::new(Client::default()),
            status: RefCell::new(Status {
                unlock: Unlock::SignedOut,
                detail: String::new(),
                account: None,
                device_linked: false,
                sealed: None,
            }),
            node: Mutex::new(None),
            store: RefCell::new(None),
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
            _ => Err(format!("{cmd} is not available in the web app yet")),
        }
    }

    fn emit(&self, event: &str, payload: Value) {
        self.host.emit(event, &payload.to_string());
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
            "server_gifs": false,
            "server_games": false,
            "server_media": false,
        }))
    }

    async fn sign_out(&self) -> Result<Value, String> {
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

    fn account(&self) -> Result<AccountUser, String> {
        self.status.borrow().account.clone().ok_or_else(|| "Sign in first".to_string())
    }

    fn device_secret(&self) -> [u8; 32] {
        if let Some(secret) = account::parse_device_secret(self.host.load(account::DEVICE_SECRET_KEY).as_deref()) {
            return secret;
        }
        let secret = CommyNode::generate_secret();
        self.host.save(account::DEVICE_SECRET_KEY, &data_encoding::HEXLOWER.encode(&secret));
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
        let secret = self.device_secret();
        self.start_node(secret).await?;
        self.link_device(secret).await?;
        let reply = self.request("GET", account::VAULT_PATH, None).await?;
        let Some(sealed) = account::vault_data(&reply)? else {
            self.set_unlock(Unlock::NoList, "Open Commy on your computer and sign in there first. The web app opens the servers your computer has.");
            return Ok(self.unlock_view());
        };
        self.status.borrow_mut().sealed = Some(sealed.clone());
        let stored = account::stored_entry_key(self.host.load(account::ENTRY_KEY_KEY).as_deref(), &account.id);
        if let Some(list) = stored.and_then(|key| EntryList::open(&key, &sealed).ok()) {
            self.finish(list)?;
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
        let (mut node, events, _media, _dms) = CommyNode::spawn_with(Some(secret), Some(own_relay_map()?))
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

    async fn node_hex(&self) -> Option<String> {
        self.node.lock().await.as_ref().map(|n| n.node_id_hex())
    }

    async fn link_device(&self, secret: [u8; 32]) -> Result<(), String> {
        let key = iroh::SecretKey::from_bytes(&secret);
        let node_id = data_encoding::HEXLOWER.encode(key.public().as_bytes());
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
        let own = self.node_hex().await.unwrap_or_default();
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
        self.set_unlock(Unlock::Asking, "Check your other devices and choose Allow.");
        let endpoint = {
            let node = self.node.lock().await;
            let node = node.as_ref().ok_or("Not connected yet")?;
            node.reach_via_relays(&devices);
            node.endpoint()
        };
        let request = EntryKeyRequest { server: self.origin.clone(), account: account.id.clone(), device_name: DEVICE_NAME.into() };
        let (tx, mut rx) = mpsc::unbounded_channel();
        for device in &devices {
            let (tx, endpoint, request, device) = (tx.clone(), endpoint.clone(), request.clone(), device.clone());
            spawn_local(async move {
                let _ = tx.send(commy_net::entry_key::request(endpoint, &device, &request).await.ok());
            });
        }
        drop(tx);
        while let Some(reply) = rx.recv().await {
            if let Some(EntryKeyReply::Key(text)) = reply {
                if let Ok(key) = SealKey::from_text(&text) {
                    if self.accept_key(&account, key).is_ok() {
                        return Ok(self.unlock_view());
                    }
                }
            }
        }
        self.set_unlock(Unlock::Locked, "None of your devices approved this browser. Try again, or enter your recovery code.");
        Ok(self.unlock_view())
    }

    async fn use_recovery_code(self: &Rc<Self>, code: &str) -> Result<Value, String> {
        let account = self.account()?;
        let key = SealKey::from_text(code.trim()).map_err(|_| "That is not a recovery code".to_string())?;
        self.accept_key(&account, key)?;
        Ok(self.unlock_view())
    }

    fn accept_key(self: &Rc<Self>, account: &AccountUser, key: SealKey) -> Result<(), String> {
        let sealed = self.status.borrow().sealed.clone().ok_or("Your server list has not loaded yet")?;
        let list = EntryList::open(&key, &sealed)?;
        self.host.save(account::ENTRY_KEY_KEY, &account::entry_key_record(&account.id, &key));
        self.finish(list)
    }

    fn finish(self: &Rc<Self>, list: EntryList) -> Result<(), String> {
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
        Ok(())
    }

    async fn load_servers(self: Rc<Self>, tickets: Vec<ServerTicket>) {
        let devices = self.other_devices().await.unwrap_or_default();
        {
            let mut node = self.node.lock().await;
            let Some(node) = node.as_mut() else { return };
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
                NetEvent::Event { server, event, action, .. } => self.on_live(server, event, action),
                NetEvent::SyncReceived { server, payload } => {
                    let installed = self.client.borrow().install_owner_snapshot(server, payload).unwrap_or(false);
                    if installed {
                        self.emit("servers-changed", Value::Null);
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

    fn on_live(&self, server: ServerId, event: ServerEvent, action: Option<commy_core::Action>) {
        let applied = self.client.borrow().apply_live(server, event, action);
        if let Applied::Created(message) = &applied {
            if let Some(payload) = self.client.borrow().message_created_payload(server, message) {
                self.emit("message-created", payload);
            }
        }
        if applied != Applied::Nothing {
            self.emit("server-event", json!(server.to_string()));
        }
    }

    async fn run_changes(self: Rc<Self>, mut changed: mpsc::UnboundedReceiver<ServerId>) {
        while let Some(server) = changed.recv().await {
            self.emit("server-event", json!(server.to_string()));
        }
    }

    async fn send_message(&self, args: Args) -> Result<Value, String> {
        let server: ServerId = parse(args.server_id.as_ref(), "server")?;
        let channel: ChannelId = parse(args.channel_id.as_ref(), "channel")?;
        let reply_to: Option<MessageId> = args.reply_to.as_ref().map(|r| parse(Some(r), "message")).transpose()?;
        let (message, view) = self.client.borrow().compose(server, channel, args.content.as_deref().unwrap_or(""), reply_to)?;
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
        self.emit("server-event", json!(server.to_string()));
        to_json(&view.with_pending(!delivered))
    }
}

fn own_relay_map() -> Result<RelayMap, String> {
    let map = RelayMap::empty();
    for url in commy_net::relays::own_relays() {
        let url: RelayUrl = url.parse().map_err(|_| format!("invalid relay {url}"))?;
        map.insert(url.clone(), Arc::new(RelayConfig::new(url, None)));
    }
    Ok(map)
}
