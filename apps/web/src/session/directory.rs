use std::collections::HashMap;
use std::rc::Rc;

use chrono::Utc;
use commy_core::{Permissions, ServerId};
use commy_net::directory::{self as shared, clean_category, clean_description, current_listing, listing_name, member_bucket, Listing, ListResponse, PublishResponse};
use commy_net::invite::{self, InviteLink};
use serde_json::{json, Value};

use super::invites::invite_reply;
use super::{parse, to_json, Args, Session};
use crate::account;

impl Session {
    pub(super) async fn directory_command(self: &Rc<Self>, cmd: &str, args: Args) -> Result<Value, String> {
        match cmd {
            "browse_directory" => {
                let category = args.category.as_ref().and_then(Value::as_str);
                let path = shared::list_path(args.q.as_deref(), category, args.cursor.as_deref())?;
                let reply = self.fetch_list(&path).await?;
                to_json(&shared::page(&self.origin, self.host_name(), reply))
            }
            "server_listing" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                let listing = self.listings().remove(&server.to_string()).filter(|l| l.origin == self.origin);
                to_json(&listing)
            }
            "publish_listing" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                let category = args.category.as_ref().and_then(Value::as_str).unwrap_or("");
                to_json(&self.publish_listing(server, args.description.as_deref().unwrap_or(""), category).await?)
            }
            "unlist_server" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                self.unlist(server).await.map(|_| Value::Null)
            }
            _ => Err(format!("{cmd} is not available in the web app yet")),
        }
    }

    fn listings(&self) -> HashMap<String, Listing> {
        let Ok(account) = self.account() else { return HashMap::new() };
        self.host.load(&account::listings_key(&account.id)).and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or_default()
    }

    fn change_listings(&self, edit: impl FnOnce(&mut HashMap<String, Listing>)) -> Result<(), String> {
        let account = self.account()?;
        let mut listings = self.listings();
        edit(&mut listings);
        self.host.save(&account::listings_key(&account.id), &serde_json::to_string(&listings).map_err(|e| e.to_string())?);
        Ok(())
    }

    async fn fetch_list(&self, path: &str) -> Result<Option<ListResponse>, String> {
        let reply = self.request("GET", path, None).await?;
        if shared::directory_missing(reply.status) {
            return Ok(None);
        }
        invite_reply::<ListResponse>(reply).map(Some)
    }

    async fn publish_listing(&self, server: ServerId, description: &str, category: &str) -> Result<Listing, String> {
        self.need_permission(server, Permissions::MANAGE_SERVER, "list this server in Discover")?;
        let category = clean_category(category)?;
        let description = clean_description(description)?;
        let (name, members, public) =
            self.read_state(server, |state, _| Ok((listing_name(&state.server.name), member_bucket(state.server.members.len()), state.server.public)))?;
        if !public {
            return Err("Make the server public before listing it in Discover".into());
        }
        if self.fetch_list(shared::LIST_PATH).await?.is_none() {
            return Err(format!("{} does not list public servers yet", self.host_name()));
        }
        let link = self.permanent_link(server).await?;
        let mut listing = Listing {
            origin: self.origin.clone(),
            code: link.code.clone(),
            link: link.url(),
            name,
            category,
            description,
            members,
            listed_at: Utc::now().timestamp_millis(),
        };
        listing.code = self.send_listing(&listing).await?;
        let stored = listing.clone();
        self.change_listings(|listings| {
            listings.insert(server.to_string(), stored);
        })?;
        Ok(listing)
    }

    fn reusable_link(&self, server: ServerId) -> Option<String> {
        let listing = self.listings().remove(&server.to_string()).filter(|l| l.origin == self.origin)?;
        let link = invite::parse(&listing.link)?;
        self.read_state(server, |state, _| state.tracked_invite(&link.origin, &link.code).map(|_| ()).ok_or_else(String::new)).ok()?;
        Some(listing.link)
    }

    async fn permanent_link(&self, server: ServerId) -> Result<InviteLink, String> {
        let url = match self.reusable_link(server) {
            Some(url) => url,
            None => self.create_invite(server, invite::Expiry::Never, None).await?.url,
        };
        invite::parse(&url)
            .filter(|link| link.origin == self.origin && link.key.is_some())
            .ok_or_else(|| "The invite server sent an unexpected link".to_string())
    }

    async fn send_listing(&self, listing: &Listing) -> Result<String, String> {
        let body = shared::publish_body(listing).ok_or("This listing lost its invite link")?;
        let reply: PublishResponse = invite_reply(self.request("POST", shared::PUBLISH_PATH, Some(body)).await?)?;
        Ok(reply.code)
    }

    pub(super) async fn refresh_listing(self: Rc<Self>, server: ServerId) {
        let Some(listing) = self.listings().remove(&server.to_string()).filter(|l| l.origin == self.origin) else { return };
        let current = self.read_state(server, |state, _| Ok(state.server.public.then(|| current_listing(&listing, &state.server.name, state.server.members.len()))));
        let Ok(Some(current)) = current else { return };
        if current == listing {
            return;
        }
        if let Ok(code) = self.send_listing(&current).await {
            let refreshed = Listing { code, ..current };
            let _ = self.change_listings(|listings| {
                listings.insert(server.to_string(), refreshed);
            });
        }
    }

    async fn unlist(&self, server: ServerId) -> Result<(), String> {
        self.need_permission(server, Permissions::MANAGE_SERVER, "change this server's Discover listing")?;
        let listing = self.listings().remove(&server.to_string()).ok_or("This server was not listed from this device")?;
        if listing.origin != self.origin {
            return Err(format!("This server was listed on {}. Unlist it from the desktop app.", invite::parse(&listing.link).map_or(listing.origin.clone(), |l| l.host())));
        }
        let reply = self.request("POST", shared::UNLIST_PATH, Some(json!({ "code": listing.code }))).await?;
        if !reply.ok() && reply.status != 404 {
            invite_reply::<Value>(reply)?;
        }
        self.change_listings(|listings| {
            listings.remove(&server.to_string());
        })
    }
}
