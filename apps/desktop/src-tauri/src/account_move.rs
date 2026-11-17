use crate::lock_or_recover::LockOrRecover;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use commy_core::moved::MovePointer;
use commy_core::UserId;
use commy_net::entry_list::EntryList;
use commy_net::sealed::SealKey;
use commy_net::server_address::public_server_origin;
use commy_net::sync_streams;
use commy_sync::{StreamKeys, SyncClient, SyncEvent};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::entry_list::{self, Fetched, Put, Session, VAULT_ITEM};
use crate::state::App;

const TICKET_FILE: &str = "account-move.json";
const MOVED_IN_FILE: &str = "moved-in.json";
const TICKET_KEPT_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const LOG_WAIT: Duration = Duration::from_secs(60);
const ASK_WAIT: Duration = Duration::from_secs(15);

static MOVED_TO: Mutex<Option<String>> = Mutex::new(None);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Ticket {
    from: String,
    to: String,
    key: String,
    list: String,
    at: i64,
}

#[derive(Debug, PartialEq, Eq)]
enum TicketUse {
    NotHere,
    Stale,
    Here,
}

fn ticket_use(ticket: &Ticket, server: &str, now: i64) -> TicketUse {
    if now - ticket.at > TICKET_KEPT_MS {
        TicketUse::Stale
    } else if ticket.to == server {
        TicketUse::Here
    } else {
        TicketUse::NotHere
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Landing {
    Empty,
    Ours,
    Taken,
}

fn landing(vault: Option<&str>, opens: impl Fn(&str) -> bool) -> Landing {
    match vault {
        None => Landing::Empty,
        Some(data) if opens(data) => Landing::Ours,
        Some(_) => Landing::Taken,
    }
}

fn to_copy(already: &[Vec<u8>], mut entries: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    if entries.starts_with(already) {
        entries.split_off(already.len())
    } else {
        tracing::warn!("the new home already keeps a different account log, so this one was not copied");
        Vec::new()
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn ticket_path(app: &App) -> Option<PathBuf> {
    app.base_dir().map(|base| base.join(TICKET_FILE))
}

fn remove_quietly(path: &Path) {
    if let Err(e) = std::fs::remove_file(path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("could not remove {}: {e}", path.display());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MovedIn {
    from: String,
    to: String,
}

#[tauri::command]
pub fn account_move_pending(app: tauri::State<'_, App>) -> Option<String> {
    let ticket = entry_list::read::<Ticket>(&ticket_path(&app)?)?;
    (ticket_use(&ticket, &ticket.to, now_ms()) != TicketUse::Stale).then_some(ticket.to)
}

#[tauri::command]
pub fn take_account_moved_in(app: tauri::State<'_, App>) -> Option<MovedIn> {
    let path = app.data_dir()?.join(MOVED_IN_FILE);
    let moved = entry_list::read::<MovedIn>(&path);
    remove_quietly(&path);
    moved
}

fn remember_moved_in(dir: &Path, ticket: &Ticket) {
    let moved = MovedIn { from: ticket.from.clone(), to: ticket.to.clone() };
    if let Err(e) = crate::persist::write_secret_json(&dir.join(MOVED_IN_FILE), &moved) {
        tracing::warn!("could not keep the moved in notice: {e}");
    }
}

#[derive(Clone, Serialize)]
pub struct MoveView {
    moved_to: Option<String>,
}

#[tauri::command]
pub fn account_moved_to() -> MoveView {
    MoveView { moved_to: MOVED_TO.lock_or_recover().clone() }
}

#[tauri::command]
pub async fn start_account_move(handle: AppHandle, to: String) -> Result<(), String> {
    let to = public_server_origin(to.trim()).map_err(|e| e.message_key().to_string())?;
    leave_for(&handle, &to, true).await
}

#[tauri::command]
pub async fn follow_account_move(handle: AppHandle) -> Result<(), String> {
    let to = MOVED_TO.lock_or_recover().clone().ok_or("Your account has not moved")?;
    leave_for(&handle, &to, false).await
}

async fn leave_for(handle: &AppHandle, to: &str, announce: bool) -> Result<(), String> {
    let app = handle.state::<App>();
    let s = entry_list::session(&app).ok_or("Sign in to your Commy account first")?;
    if s.server == to {
        return Err("Your account already lives there".into());
    }
    let target = crate::account::discover(&app.http, to).await?;
    let Some(to_sync) = target.sync_url(to) else {
        return Err("That account server does not offer a sync service".into());
    };
    if !target.keeps_server_lists() {
        return Err("That account server cannot keep a server list".into());
    }
    let dir = app.data_dir().cloned().ok_or("No data folder")?;
    let key = entry_list::load_key(&dir, &s.server, &s.account).ok_or("This device does not hold your server list yet. Let it sync, then try again.")?;
    let list = match entry_list::fetch(&app, &s, VAULT_ITEM).await? {
        Fetched::Item(item) => item.data.filter(|data| EntryList::open(&key, data).is_ok()),
        Fetched::Unsupported => None,
    }
    .ok_or("Your server list is not saved with your account yet. Let it sync, then try again.")?;
    let path = ticket_path(&app).ok_or("No data folder")?;

    if announce {
        let identity = app.identity().ok_or("This device has no identity yet")?;
        let user = app.profile.lock_or_recover().as_ref().map(|p| p.id).ok_or("This device has no profile yet")?;
        if let Some(from_sync) = crate::sync_link::url() {
            let copied = copy_log(&from_sync, &to_sync, sync_streams::account_stream(&key)).await?;
            tracing::info!("copied {copied} account log entries to the new home");
        }
        crate::pin_owned_in_place(&app, handle).await?;
        announce_move(&app, &s, &identity, user, to).await?;
    }

    let ticket = Ticket { from: s.server.clone(), to: to.to_string(), key: key.to_text(), list, at: now_ms() };
    crate::persist::write_secret_json(&path, &ticket).map_err(|e| format!("could not keep the move ticket: {e}"))?;
    *MOVED_TO.lock_or_recover() = None;
    crate::account::sign_out_to_move(&app, handle, to).await
}

async fn announce_move(app: &App, s: &Session, identity: &commy_core::Identity, user: UserId, to: &str) -> Result<(), String> {
    let pointer = MovePointer::sign(identity, &s.server, to, user, now_ms());
    let request = entry_list::account_api(app, s, reqwest::Method::POST, "moved/set").json(&pointer).timeout(ASK_WAIT);
    let res = request.send().await.map_err(|_| "Could not reach your account server".to_string())?;
    match res.status() {
        status if status.is_success() => Ok(()),
        reqwest::StatusCode::NOT_FOUND => {
            tracing::info!("the old home cannot point to the new one");
            Ok(())
        }
        status => Err(format!("Your account server could not record the move ({status}). Nothing was lost, and moving again picks up where this left off.")),
    }
}

async fn read_log(url: &str, keys: &StreamKeys) -> Result<(SyncClient, Vec<Vec<u8>>), String> {
    let (client, mut events) = SyncClient::spawn(url.to_string());
    client.follow(keys.clone(), 0);
    let stream = keys.id;
    let mut entries = Vec::new();
    tokio::time::timeout(LOG_WAIT, async {
        while let Some(event) = events.recv().await {
            match event {
                SyncEvent::Entry { stream: from, plain, .. } if from == stream => entries.push(plain),
                SyncEvent::CaughtUp { stream: from, .. } if from == stream => return Ok(()),
                SyncEvent::Refused { stream: from, code } if from == stream => return Err(format!("the sync service refused: {code:?}")),
                _ => {}
            }
        }
        Err("the sync connection stopped".to_string())
    })
    .await
    .map_err(|_| "the sync service took too long".to_string())??;
    Ok((client, entries))
}

async fn copy_log(from: &str, to: &str, keys: StreamKeys) -> Result<usize, String> {
    let (old, entries) = read_log(from, &keys).await?;
    old.unfollow(keys.id);
    let (new, already) = read_log(to, &keys).await?;
    let entries = to_copy(&already, entries);
    for plain in &entries {
        new.append(keys.id, plain.clone()).await.map_err(|e| format!("could not copy your account log: {e}"))?;
    }
    new.unfollow(keys.id);
    Ok(entries.len())
}

pub(crate) async fn move_in_if_due(app: &App, dir: &Path, s: &Session) {
    let Some(path) = ticket_path(app) else { return };
    let Some(ticket) = entry_list::read::<Ticket>(&path) else { return };
    match ticket_use(&ticket, &s.server, now_ms()) {
        TicketUse::NotHere => return,
        TicketUse::Stale => return remove_quietly(&path),
        TicketUse::Here => {}
    }
    let Ok(key) = SealKey::from_text(&ticket.key) else { return remove_quietly(&path) };
    let item = match entry_list::fetch(app, s, VAULT_ITEM).await {
        Ok(Fetched::Item(item)) => item,
        Ok(Fetched::Unsupported) => return remove_quietly(&path),
        Err(e) => return tracing::info!("moving in waits for the account server: {e}"),
    };
    match landing(item.data.as_deref(), |data| EntryList::open(&key, data).is_ok()) {
        Landing::Empty => match entry_list::put(app, s, VAULT_ITEM, ticket.list.clone(), item.version).await {
            Ok(Put::Done) => {}
            Ok(Put::Conflict) => return,
            Err(e) => return tracing::info!("moving in waits for the account server: {e}"),
        },
        Landing::Ours => {}
        Landing::Taken => {
            tracing::warn!("this account already keeps another server list, so the moved one was not brought in");
            return remove_quietly(&path);
        }
    }
    entry_list::save_key(dir, &s.server, &s.account, &key);
    remember_moved_in(dir, &ticket);
    tracing::info!("moved in from {}", ticket.from);
    remove_quietly(&path);
}

#[derive(Deserialize)]
struct MovedReply {
    moved: MovePointer,
}

pub async fn check_moved(handle: &AppHandle) {
    let app = handle.state::<App>();
    let Some(s) = entry_list::session(&app) else { return };
    let Some(identity) = app.identity() else { return };
    let Some(user) = app.profile.lock_or_recover().as_ref().map(|p| p.id) else { return };
    let request = app
        .http
        .get(format!("{}/api/auth/commy/moved", s.server))
        .query(&[("identity", identity.public().to_hex())])
        .timeout(ASK_WAIT);
    let Ok(res) = request.send().await else { return };
    let moved = if res.status().is_success() {
        res.json::<MovedReply>().await.ok().map(|reply| reply.moved)
    } else if res.status() == reqwest::StatusCode::NOT_FOUND {
        None
    } else {
        return;
    };
    let to = moved
        .filter(|pointer| pointer.moves(user, &identity.public(), &s.server))
        .map(|pointer| pointer.to)
        .filter(|to| !crate::server_guard::blocks_origin(to));
    let changed = {
        let mut known = MOVED_TO.lock_or_recover();
        let changed = *known != to;
        known.clone_from(&to);
        changed
    };
    if changed {
        let _ = handle.emit("account-moved", MoveView { moved_to: to });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(to: &str, at: i64) -> Ticket {
        Ticket { from: "https://commy.chat".into(), to: to.into(), key: String::new(), list: String::new(), at }
    }

    #[test]
    fn a_move_ticket_is_used_only_at_the_new_home_and_not_forever() {
        let now = 1_800_000_000_000;
        assert_eq!(ticket_use(&ticket("https://friends.org", now), "https://friends.org", now), TicketUse::Here);
        assert_eq!(ticket_use(&ticket("https://friends.org", now), "https://commy.chat", now), TicketUse::NotHere);
        assert_eq!(ticket_use(&ticket("https://friends.org", now - TICKET_KEPT_MS - 1), "https://friends.org", now), TicketUse::Stale);
    }

    #[test]
    fn moving_in_never_overwrites_a_list_it_cannot_open() {
        assert_eq!(landing(None, |_| false), Landing::Empty);
        assert_eq!(landing(Some("mine"), |data| data == "mine"), Landing::Ours);
        assert_eq!(landing(Some("theirs"), |data| data == "mine"), Landing::Taken);
    }

    #[test]
    fn copying_the_account_log_again_adds_only_what_is_missing() {
        let entries = vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()];
        assert_eq!(to_copy(&[], entries.clone()), entries);
        assert_eq!(to_copy(&entries[..1], entries.clone()), entries[1..].to_vec(), "a move that stopped part way finishes the copy");
        assert!(to_copy(&entries, entries.clone()).is_empty(), "a retry after a full copy writes nothing twice");
    }

    #[test]
    fn a_different_log_at_the_new_home_is_never_mixed_with_this_one() {
        let entries = vec![b"a".to_vec(), b"b".to_vec()];
        assert!(to_copy(&[b"x".to_vec()], entries.clone()).is_empty());
        assert!(to_copy(&[b"a".to_vec(), b"b".to_vec(), b"c".to_vec()], entries).is_empty());
    }

    #[test]
    fn the_pointer_the_app_sends_is_what_the_account_server_reads() {
        let me = commy_core::Identity::from_bytes(&[9; 32]);
        let pointer = MovePointer::sign(&me, "https://commy.chat", "https://friends.org", UserId::new(), 5);
        let json = serde_json::to_value(&pointer).unwrap();
        for field in ["to", "user", "identity", "at", "proof"] {
            assert!(json.get(field).is_some(), "{field} is sent");
        }
    }
}
