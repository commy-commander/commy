use std::rc::Rc;
use std::time::Duration;

use commy_core::identity::IdentityKey;
use commy_core::UserId;
use serde_json::{json, Value};
use wasm_bindgen_futures::spawn_local;
use web_time::Instant;

use super::Session;
use crate::account::moved_to_of;

const CHECK_EVERY: Duration = Duration::from_secs(10 * 60);
const RETRY_AFTER: Duration = Duration::from_secs(2 * 60);

#[derive(Default)]
pub(super) struct MoveWatch {
    moved_to: Option<String>,
    next_check: Option<Instant>,
    checking: bool,
}

impl Session {
    pub(super) fn keep_move_checked(self: &Rc<Self>) {
        let Some((user, identity)) = self.client.borrow().me_pair() else { return };
        let start = {
            let mut watch = self.moved.borrow_mut();
            let start = !watch.checking && watch.next_check.is_none_or(|at| Instant::now() >= at);
            watch.checking |= start;
            start
        };
        if start {
            spawn_local(self.clone().check_moved(user, identity));
        }
    }

    async fn check_moved(self: Rc<Self>, user: UserId, identity: IdentityKey) {
        let path = format!("{}?identity={}", crate::account::MOVED_PATH, identity.to_hex());
        let found = self.request("GET", &path, None).await.ok().and_then(|reply| moved_to_of(&reply, user, &identity, &self.origin));
        let found = found.map(|to| to.filter(|to| !self.blocks_sync_origin(to)));
        let changed = {
            let mut watch = self.moved.borrow_mut();
            watch.checking = false;
            watch.next_check = Some(Instant::now() + if found.is_some() { CHECK_EVERY } else { RETRY_AFTER });
            match found {
                Some(to) if to != watch.moved_to => {
                    watch.moved_to = to;
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.emit("web-account-moved", self.moved_view());
        }
    }

    pub(super) fn moved_view(&self) -> Value {
        json!({ "moved_to": self.moved.borrow().moved_to })
    }
}
