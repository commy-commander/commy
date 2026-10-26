use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use commy_view::links::{self, Blocklist};
use serde_json::Value;
use wasm_bindgen_futures::spawn_local;
use web_time::Instant;

use super::{to_json, Args, Session};

const LIST_PATH: &str = "/api/auth/commy/link-blocklist";
const REFRESH_EVERY: Duration = Duration::from_secs(12 * 60 * 60);
const RETRY_AFTER: Duration = Duration::from_secs(30 * 60);
const MAX_TRUSTED_DOMAINS: usize = 500;

#[derive(Default)]
pub(super) struct LinkSafety {
    list: Arc<Blocklist>,
    next_refresh: Option<Instant>,
    refreshing: bool,
}

impl Session {
    pub(super) fn keep_link_list_fresh(self: &Rc<Self>) {
        {
            let mut safety = self.links.borrow_mut();
            let due = safety.next_refresh.is_none_or(|at| Instant::now() >= at);
            if !due || safety.refreshing {
                return;
            }
            safety.refreshing = true;
        }
        spawn_local(self.clone().refresh_link_list());
    }

    async fn refresh_link_list(self: Rc<Self>) {
        let fresh = match self.request("GET", LIST_PATH, None).await {
            Ok(reply) if reply.ok() => Some(Blocklist::parse(&reply.body)).filter(|list| !list.is_empty()),
            _ => None,
        };
        let mut safety = self.links.borrow_mut();
        safety.refreshing = false;
        safety.next_refresh = Some(Instant::now() + if fresh.is_some() { REFRESH_EVERY } else { RETRY_AFTER });
        if let Some(list) = fresh {
            safety.list = Arc::new(list);
        }
    }

    pub(super) fn check_link(&self, args: Args) -> Result<Value, String> {
        let mut trusted = args.trusted.unwrap_or_default();
        trusted.truncate(MAX_TRUSTED_DOMAINS);
        let list = self.links.borrow().list.clone();
        let url = args.url.unwrap_or_default();
        let checked = links::check_link(&url, args.label.as_deref(), &trusted, &list).ok_or("only http and https links can be opened")?;
        to_json(&checked)
    }
}
