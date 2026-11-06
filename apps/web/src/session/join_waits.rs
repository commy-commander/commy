use std::rc::Rc;
use std::time::Duration;

use commy_core::ServerId;
use commy_net::{KnockReply, ServerTicket};
use serde_json::Value;

use super::{parse, Args, Session};
use crate::knocks::{knocks_key, Knocking, Knocks};

const FIRST_CHECK_AFTER: Duration = Duration::from_secs(20);
const RECHECK_EVERY: Duration = Duration::from_secs(90);

impl Session {
    pub(super) async fn join_wait_command(self: &Rc<Self>, cmd: &str, args: Args) -> Result<Value, String> {
        match cmd {
            "list_join_waits" => Ok(self.knocks().views()),
            "cancel_join_request" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                self.edit_knocks(|knocks| knocks.remove(server));
                self.emit("join-waits-changed", Value::Null);
                Ok(Value::Null)
            }
            "send_join_note" => {
                let server: ServerId = parse(args.server_id.as_ref(), "server")?;
                let knock = self.edit_knocks(|knocks| knocks.set_note(server, args.note.as_deref().unwrap_or("")))?;
                self.recheck_knock(server, knock).await;
                self.emit("join-waits-changed", Value::Null);
                Ok(Value::Null)
            }
            _ => Err(format!("{cmd} is not available in the web app yet")),
        }
    }

    fn knocks_storage_key(&self) -> Option<String> {
        self.account().ok().map(|account| knocks_key(&account.id))
    }

    pub(super) fn knocks(&self) -> Knocks {
        Knocks::parse(self.knocks_storage_key().and_then(|key| self.host.load(&key)).as_deref())
    }

    pub(super) fn edit_knocks<T>(&self, edit: impl FnOnce(&mut Knocks) -> T) -> T {
        let mut knocks = self.knocks();
        let result = edit(&mut knocks);
        if let Some(key) = self.knocks_storage_key() {
            self.host.save(&key, &knocks.to_json());
        }
        result
    }

    pub(super) async fn watch_knocks(self: Rc<Self>) {
        n0_future::time::sleep(FIRST_CHECK_AFTER).await;
        loop {
            for (server, knock) in self.knocks().waiting() {
                self.recheck_knock(server, knock).await;
            }
            n0_future::time::sleep(RECHECK_EVERY).await;
        }
    }

    async fn recheck_knock(self: &Rc<Self>, server: ServerId, knock: Knocking) {
        let Ok(ticket) = ServerTicket::decode(&knock.ticket) else { return };
        match self.knock(&ticket, &knock.note, knock.asked_at).await {
            Some(KnockReply::Waiting(lobby)) => self.edit_knocks(|knocks| knocks.learn(server, lobby)),
            Some(KnockReply::TurnedAway) => {
                if self.edit_knocks(|knocks| knocks.turn_away(server)) {
                    self.emit("join-waits-changed", Value::Null);
                }
            }
            Some(KnockReply::Admitted | KnockReply::Open) => {
                let Some(waiting) = self.edit_knocks(|knocks| knocks.remove(server)) else { return };
                self.join_group(&waiting.ticket, waiting.key(), true);
                self.emit("join-waits-changed", Value::Null);
                self.emit("servers-changed", Value::Null);
            }
            Some(KnockReply::Unknown) | None => {}
        }
    }
}
