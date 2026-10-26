use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use commy_core::{Permissions, ServerId};
use commy_net::invite::{self, BackgroundResponse, CreateRequest, CreateResponse, InviteLink, ResolveResponse};
use commy_net::ServerTicket;
use commy_view::ServerSummary;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{parse, to_json, Args, Session};
use crate::account::Reply;
use crate::files;

const JOIN_WAIT: Duration = Duration::from_secs(20);
const JOIN_POLL: Duration = Duration::from_millis(250);
const DEFAULT_ACCOUNT_SERVER: &str = "https://commy.chat";

#[derive(Deserialize)]
struct ErrorReply {
    message: Option<String>,
}

pub(super) fn invite_reply<T: DeserializeOwned>(reply: Reply) -> Result<T, String> {
    if !reply.ok() {
        let message = serde_json::from_str::<ErrorReply>(&reply.body).ok().and_then(|e| e.message);
        return Err(message.map_or_else(|| format!("the invite server answered {}", reply.status), |m| invite::clip(&m, 200)));
    }
    serde_json::from_str(&reply.body).map_err(|_| "The invite server sent an invalid reply".to_string())
}

impl Session {
    pub(super) async fn invite_command(self: &Rc<Self>, cmd: &str, args: Args) -> Result<Value, String> {
        match cmd {
            "account_server_info" => Ok(json!({ "origin": self.origin, "host": self.host_name(), "is_default": self.origin == DEFAULT_ACCOUNT_SERVER })),
            "get_invite" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                self.need_permission(server, Permissions::CREATE_INVITES, "invite people")?;
                let (ticket, _) = self.client.borrow().invite_ticket(server, None)?;
                Ok(Value::String(ticket.encode()))
            }
            "create_invite_link" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                let expiry = invite::expiry_choice(args.expires.as_deref())?;
                let name = invite::name_choice(args.name.as_deref())?;
                to_json(&self.create_invite(server, expiry, name).await?)
            }
            "resolve_invite_link" => to_json(&self.resolve_invite(args.link.as_deref().unwrap_or("")).await?),
            "join_server" => to_json(&self.join_invited(args.ticket.as_deref().unwrap_or("")).await?),
            "list_invites" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                let me = self.client.borrow().profile().ok_or("Your servers are not unlocked yet")?.id;
                let client = self.client.borrow();
                let servers = client.servers.lock().unwrap();
                let state = servers.get(&server).ok_or("This server is still loading")?;
                to_json(&invite::invite_views(state, me, Utc::now().timestamp_millis()))
            }
            "invite_full_link" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                to_json(&self.full_link(server, args.origin.as_deref().unwrap_or(""), args.code.as_deref().unwrap_or("")).await?)
            }
            "revoke_invite" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                self.revoke(server, args.origin.as_deref().unwrap_or(""), args.code.as_deref().unwrap_or("")).await.map(|_| Value::Null)
            }
            _ => Err(format!("{cmd} is not available in the web app yet")),
        }
    }

    pub(super) fn need_permission(&self, server: ServerId, needed: Permissions, action: &str) -> Result<(), String> {
        let me = self.client.borrow().profile().ok_or("Your servers are not unlocked yet")?.id;
        let client = self.client.borrow();
        let servers = client.servers.lock().unwrap();
        let state = servers.get(&server).ok_or("This server is still loading")?;
        commy_view::manage::need(state.server.permissions_for(me), needed, action)
    }

    pub(super) async fn create_invite(&self, server: ServerId, expiry: invite::Expiry, name: Option<String>) -> Result<invite::CreatedInvite, String> {
        let me = self.client.borrow().profile().ok_or("Your servers are not unlocked yet")?.id;
        {
            let client = self.client.borrow();
            let servers = client.servers.lock().unwrap();
            let state = servers.get(&server).ok_or("This server is still loading")?;
            invite::check_create(state, me, expiry, name.is_some())?;
        }
        let given = self.sync.borrow().as_ref().and_then(|live| live.book.given().remove(&server));
        let (ticket, community) = self.client.borrow().invite_ticket(server, given)?;
        let raw = ticket.encode();
        let revoke_key = invite::new_revoke_key();
        let (key, key_text) = invite::link_key(&revoke_key, name.is_some());
        let request = CreateRequest {
            raw_ticket: &raw,
            server_name: &ticket.server_name,
            expiry,
            custom: name.as_deref(),
            community: community.as_ref().map(|(id, secret)| (*id, secret)),
            background: None,
            revoke_key: &revoke_key,
        };
        let (body, sealed_at) = invite::create_body(&key, &request, Utc::now());
        let reply: CreateResponse = invite_reply(self.request("POST", invite::CREATE_PATH, Some(body)).await?)?;
        let made = invite::created(&self.origin, name.as_deref(), key_text, reply, sealed_at)?;
        let event = invite::track_event(server, me, self.origin.clone(), &made, revoke_key, Utc::now().timestamp_millis());
        let tracked = self.client.borrow().act(server, |_, _, _, _| Ok(event));
        if let Ok(payload) = tracked {
            self.publish(server, &payload);
            self.emit("server-event", json!(server.to_string()));
        }
        Ok(made.invite)
    }

    fn same_site(&self, link: &InviteLink) -> Result<(), String> {
        if link.origin == self.origin {
            Ok(())
        } else {
            Err(format!("This invite comes from {}. Open it in the desktop app to join from another account server.", link.host()))
        }
    }

    async fn full_link(&self, server: ServerId, origin: &str, code: &str) -> Result<String, String> {
        let me = self.client.borrow().profile().ok_or("Your servers are not unlocked yet")?.id;
        let tracked = {
            let client = self.client.borrow();
            let servers = client.servers.lock().unwrap();
            let state = servers.get(&server).ok_or("This server is still loading")?;
            invite::revocable(state, me, origin, code)?
        };
        let link = invite::tracked_link(&tracked);
        self.same_site(&link)?;
        let body: ResolveResponse = invite_reply(self.request("GET", &link.endpoint(invite::RESOLVE_PATH), None).await?)?;
        invite::open_reply(&link, &body, Utc::now()).map_err(|_| invite::OLD_LINK.to_string())?;
        Ok(link.url())
    }

    async fn resolve_invite(&self, raw: &str) -> Result<invite::ResolvedInvite, String> {
        let link = invite::parse(raw).ok_or("That is not a Commy invite link")?;
        self.same_site(&link)?;
        let body: ResolveResponse = invite_reply(self.request("GET", &link.endpoint(invite::RESOLVE_PATH), None).await?)?;
        let opened = invite::open_reply(&link, &body, Utc::now())?;
        if let Some(key) = opened.community.clone() {
            self.invited.borrow_mut().insert(opened.ticket.server_id, key);
        }
        let background = match (body.has_background, link.key.as_deref()) {
            (true, Some(key)) => self.invite_background(&link, key).await,
            _ => None,
        };
        Ok(invite::resolved(&link, &opened, &self.origin, background))
    }

    async fn invite_background(&self, link: &InviteLink, key: &str) -> Option<String> {
        let reply = self.request("GET", &link.endpoint(invite::BACKGROUND_PATH), None).await.ok()?;
        let body: BackgroundResponse = invite_reply(reply).ok()?;
        let bytes = invite::open_background(&body.background, key).filter(|b| commy_view::pictures::is_jpeg_or_png(b))?;
        Some(files::data_url(commy_view::pictures::picture_mime(&bytes), &bytes))
    }

    async fn join_invited(self: &Rc<Self>, raw: &str) -> Result<ServerSummary, String> {
        let ticket = ServerTicket::decode(raw.trim()).map_err(|_| "That invite is damaged".to_string())?;
        let server = ticket.server_id;
        if let Some(summary) = self.client.borrow().summary(server) {
            return Ok(summary);
        }
        let key = self.invited.borrow_mut().remove(&server).ok_or("Open an invite link to join a server from the browser")?;
        if ticket.owner.is_none() {
            return Err("That invite is damaged".into());
        }
        self.join_group(raw.trim(), Some(key));
        let deadline = web_time::Instant::now() + JOIN_WAIT;
        while web_time::Instant::now() < deadline {
            if let Some(summary) = self.client.borrow().summary(server) {
                return Ok(summary);
            }
            n0_future::time::sleep(JOIN_POLL).await;
        }
        self.emit("servers-changed", Value::Null);
        Ok(ServerSummary { id: server.to_string(), name: ticket.server_name, member_count: 0, icon: None, can_invite: false, group: false })
    }

    async fn revoke(&self, server: ServerId, origin: &str, code: &str) -> Result<(), String> {
        let me = self.client.borrow().profile().ok_or("Your servers are not unlocked yet")?.id;
        let invite = {
            let client = self.client.borrow();
            let servers = client.servers.lock().unwrap();
            let state = servers.get(&server).ok_or("This server is still loading")?;
            invite::revocable(state, me, origin, code)?
        };
        if invite.origin != self.origin {
            return Err(format!("This invite was made on {}. Revoke it from the desktop app.", invite::host_of(&invite.origin)));
        }
        let reply = self.request("POST", invite::REVOKE_PATH, Some(invite::revoke_body(&invite))).await?;
        if !reply.ok() && reply.status != 404 {
            invite_reply::<Value>(reply)?;
        }
        let event = invite::revoke_event(server, invite);
        let payload = self.client.borrow().act(server, |_, _, _, _| Ok(event))?;
        self.publish(server, &payload);
        self.emit("server-event", json!(server.to_string()));
        Ok(())
    }
}
