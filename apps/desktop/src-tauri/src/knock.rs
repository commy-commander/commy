use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use commy_core::action::SignedEvent;
use commy_core::{ServerEvent, ServerId, ServerState};
use commy_net::{KnockAsk, KnockReply, KnockRequest, Lobby, ServerTicket, WirePayload};
use commy_view::ServerSummary;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::App;
use crate::CmdResult;

pub const KNOCKS_FILE: &str = "knocks.json";
const RECHECK_EVERY: Duration = Duration::from_secs(90);
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(20);
pub const TURNED_AWAY: &str = "This server did not let you in";
const KEY_NOT_READY: &str = "your account key is still being confirmed; try again in a moment";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Knocking {
    pub ticket: String,
    pub name: String,
    pub rules: Option<String>,
    pub members: u32,
    pub note: String,
    pub asked_at: i64,
    #[serde(default)]
    pub turned_away: bool,
}

impl Knocking {
    fn learn(&mut self, lobby: Lobby) {
        self.name = lobby.name;
        self.rules = lobby.rules;
        self.members = lobby.members;
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Knocks {
    servers: HashMap<ServerId, Knocking>,
}

impl Knocks {
    pub fn load(dir: &Path) -> Self {
        crate::persist::read_json(&dir.join(KNOCKS_FILE)).unwrap_or_default()
    }

    fn save(&self, dir: &Path) {
        if let Err(e) = crate::persist::write_secret_json(&dir.join(KNOCKS_FILE), self) {
            tracing::warn!("failed to save join requests: {e}");
        }
    }

    fn waiting(&self) -> Vec<(ServerId, Knocking)> {
        self.servers.iter().filter(|(_, k)| !k.turned_away).map(|(id, k)| (*id, k.clone())).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WaitingView {
    pub id: String,
    pub name: String,
    pub rules: Option<String>,
    pub members: u32,
    pub note: String,
    pub asked_at: i64,
    pub turned_away: bool,
    pub waiting: bool,
}

fn view_of(id: ServerId, knock: &Knocking) -> WaitingView {
    WaitingView {
        id: id.to_string(),
        name: knock.name.clone(),
        rules: knock.rules.clone(),
        members: knock.members,
        note: knock.note.clone(),
        asked_at: knock.asked_at,
        turned_away: knock.turned_away,
        waiting: true,
    }
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Joined {
    Server(ServerSummary),
    Waiting(WaitingView),
}

fn edit(app: &App, change: impl FnOnce(&mut Knocks)) {
    let mut knocks = app.knocks.lock().unwrap();
    change(&mut knocks);
    if let Some(dir) = app.data_dir() {
        knocks.save(dir);
    }
}

pub fn list(app: &App) -> Vec<WaitingView> {
    let knocks = app.knocks.lock().unwrap();
    let mut list: Vec<WaitingView> = knocks.servers.iter().map(|(id, k)| view_of(*id, k)).collect();
    list.sort_by_key(|v| v.asked_at);
    list
}

pub fn cancel(app: &App, handle: &AppHandle, sid: ServerId) {
    edit(app, |knocks| {
        knocks.servers.remove(&sid);
    });
    let _ = handle.emit("join-waits-changed", ());
}

pub async fn send_note(app: &App, handle: &AppHandle, sid: ServerId, note: &str) -> CmdResult<()> {
    let note = commy_core::join_request::clean_note(note).ok_or(commy_view::join_requests::NOTE_TOO_LONG)?;
    let mut knock = app.knocks.lock().unwrap().servers.get(&sid).cloned().ok_or("You are not waiting to join that server")?;
    knock.note = note;
    edit(app, |knocks| {
        knocks.servers.insert(sid, knock.clone());
    });
    recheck(handle, sid, knock).await;
    let _ = handle.emit("join-waits-changed", ());
    Ok(())
}

fn knock_for(app: &App, sid: ServerId, note: &str, asked_at: i64) -> CmdResult<KnockRequest> {
    let me = app.announced(crate::current_profile(app)?);
    let event = commy_view::join_requests::ask(sid, &me, note, asked_at)?;
    let ask = app.sign_event(sid, event).ok_or(KEY_NOT_READY)?;
    Ok(KnockRequest { server: sid, ask })
}

async fn send(app: &App, ticket: &ServerTicket, request: KnockRequest) -> Option<KnockReply> {
    let knocking = {
        let net = app.net.lock().await;
        net.as_ref()?.knock(ticket, request)
    };
    knocking.await
}

pub async fn knock_then_join(app: &App, handle: &AppHandle, ticket: ServerTicket, note: &str) -> CmdResult<Joined> {
    let sid = ticket.server_id;
    if app.servers.lock().unwrap().contains_key(&sid) {
        return crate::join_ticket(app, handle, ticket).await.map(Joined::Server);
    }
    let asked_at = chrono::Utc::now().timestamp_millis();
    let request = knock_for(app, sid, note, asked_at)?;
    match send(app, &ticket, request).await {
        Some(KnockReply::Waiting(lobby)) => {
            let mut knock = Knocking { ticket: ticket.encode(), name: String::new(), rules: None, members: 0, note: note.trim().to_string(), asked_at, turned_away: false };
            knock.learn(lobby);
            let view = view_of(sid, &knock);
            edit(app, |knocks| {
                knocks.servers.insert(sid, knock);
            });
            let _ = handle.emit("join-waits-changed", ());
            Ok(Joined::Waiting(view))
        }
        Some(KnockReply::TurnedAway) => Err(TURNED_AWAY.into()),
        _ => {
            edit(app, |knocks| {
                knocks.servers.remove(&sid);
            });
            crate::join_ticket(app, handle, ticket).await.map(Joined::Server)
        }
    }
}

async fn recheck(handle: &AppHandle, sid: ServerId, knock: Knocking) {
    let app = handle.state::<App>();
    let Ok(ticket) = ServerTicket::decode(&knock.ticket) else { return };
    let Ok(request) = knock_for(&app, sid, &knock.note, knock.asked_at) else { return };
    match send(&app, &ticket, request).await {
        Some(KnockReply::Waiting(lobby)) => {
            edit(&app, |knocks| {
                if let Some(entry) = knocks.servers.get_mut(&sid) {
                    entry.learn(lobby);
                }
            });
        }
        Some(KnockReply::TurnedAway) => {
            edit(&app, |knocks| {
                if let Some(entry) = knocks.servers.get_mut(&sid) {
                    entry.turned_away = true;
                }
            });
            let _ = handle.emit("join-waits-changed", ());
        }
        Some(KnockReply::Admitted | KnockReply::Open) => {
            if !app.knocks.lock().unwrap().servers.contains_key(&sid) {
                return;
            }
            match crate::join_ticket(&app, handle, ticket).await {
                Ok(summary) => {
                    edit(&app, |knocks| {
                        knocks.servers.remove(&sid);
                    });
                    crate::entry_list::record_join(&app, sid);
                    crate::entry_list::schedule(handle);
                    crate::community_sync::refresh_soon(handle);
                    let _ = handle.emit("join-waits-changed", ());
                    let _ = handle.emit("join-request-accepted", serde_json::json!({ "server": summary.id, "name": summary.name }));
                }
                Err(e) => tracing::warn!("could not join {} after being let in: {e}", knock.name),
            }
        }
        Some(KnockReply::Unknown) | None => {}
    }
}

pub async fn run(handle: AppHandle) {
    tokio::time::sleep(FIRST_CHECK_AFTER).await;
    loop {
        let waiting = handle.state::<App>().knocks.lock().unwrap().waiting();
        for (sid, knock) in waiting {
            recheck(&handle, sid, knock).await;
        }
        tokio::time::sleep(RECHECK_EVERY).await;
    }
}

fn worth_recording(state: &ServerState, ask: &SignedEvent) -> bool {
    let ServerEvent::JoinAsk { profile, note, .. } = &ask.event else { return false };
    commy_core::join_request::request_of(&state.server, profile.id).is_none_or(|r| r.note != note.trim())
}

pub async fn run_requests(handle: AppHandle, mut asks: tokio::sync::mpsc::UnboundedReceiver<KnockAsk>) {
    while let Some(ask) = asks.recv().await {
        let app = handle.state::<App>();
        let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
        let (reply, record) = {
            let servers = app.servers.lock().unwrap();
            match (me, servers.get(&ask.request.server)) {
                (Some(me), Some(state)) => {
                    let reply = commy_net::knock::answer(state, me, &ask.request);
                    let record = matches!(reply, KnockReply::Waiting(_)) && worth_recording(state, &ask.request.ask);
                    (reply, record)
                }
                _ => (KnockReply::Unknown, false),
            }
        };
        let _ = ask.reply.send(reply);
        if record {
            record_ask(&app, &handle, ask.request.server, ask.request.ask).await;
        }
    }
}

async fn record_ask(app: &App, handle: &AppHandle, sid: ServerId, ask: SignedEvent) {
    let applied = app.servers.lock().unwrap().get_mut(&sid).map(|state| state.apply_signed(&ask));
    match applied {
        Some(Ok(true)) => {}
        Some(Err(e)) => {
            tracing::debug!("ignored a join request: {e}");
            return;
        }
        _ => return,
    }
    app.persist_server(sid);
    let _ = handle.emit("server-event", sid.to_string());
    let payload = WirePayload::Signed(ask);
    crate::community_sync::publish(sid, &payload);
    crate::broadcast(app, sid, payload).await;
}

pub fn must_wait(state: &ServerState, me: commy_core::UserId) -> bool {
    state.server.join_approval && !state.server.is_member(me) && state.server.owner != me
}

pub async fn wait_outside(app: &App, handle: &AppHandle, sid: ServerId) {
    let Some(me) = app.profile.lock().unwrap().as_ref().map(|p| p.id) else { return };
    let lobby = {
        let servers = app.servers.lock().unwrap();
        match servers.get(&sid) {
            Some(state) if must_wait(state, me) => commy_net::knock::lobby(state),
            _ => return,
        }
    };
    let Some(ticket) = app.invites.lock().unwrap().get(&sid).cloned() else { return };
    let asked_at = chrono::Utc::now().timestamp_millis();
    let mut knock = Knocking { ticket, name: String::new(), rules: None, members: 0, note: String::new(), asked_at, turned_away: false };
    knock.learn(lobby);
    crate::leave_without_trace(app, handle, sid).await;
    edit(app, |knocks| {
        knocks.servers.insert(sid, knock.clone());
    });
    let _ = handle.emit("join-waits-changed", ());
    recheck(handle, sid, knock).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::{Identity, Server, UserId, UserProfile};

    fn keyed(name: &str) -> (UserProfile, Identity) {
        let identity = Identity::generate();
        let mut profile = UserProfile::new(name);
        profile.identity = Some(identity.public());
        (profile, identity)
    }

    fn keyed_profile(name: &str) -> UserProfile {
        keyed(name).0
    }

    fn fixture() -> (ServerState, UserId) {
        let owner = keyed_profile("owner");
        let mut state = ServerState::new(Server::new("test", owner.id));
        state.profiles.insert(owner.id, owner.clone());
        (state, owner.id)
    }

    fn join(state: &mut ServerState, name: &str) -> UserId {
        let profile = keyed_profile(name);
        let id = profile.id;
        state.apply(ServerEvent::MemberJoin { server: state.server.id, profile }).unwrap();
        id
    }

    #[test]
    fn only_people_who_were_never_let_in_wait_outside() {
        let (mut state, owner) = fixture();
        let sid = state.server.id;
        let member = join(&mut state, "member");
        let stranger = keyed_profile("stranger");
        assert!(!must_wait(&state, stranger.id));
        state.apply(ServerEvent::JoinApprovalSet { server: sid, on: true, at: 1 }).unwrap();
        assert!(must_wait(&state, stranger.id));
        assert!(!must_wait(&state, member) && !must_wait(&state, owner));
    }

    #[test]
    fn the_same_note_is_recorded_once() {
        let (mut state, _) = fixture();
        let sid = state.server.id;
        state.apply(ServerEvent::JoinApprovalSet { server: sid, on: true, at: 1 }).unwrap();
        let (asker, identity) = keyed("asker");
        let event = |note: &str| ServerEvent::JoinAsk { server: sid, profile: asker.clone(), note: note.into(), at: 1 };
        let signed = |note: &str| SignedEvent::sign(sid, asker.id, 1, event(note), &identity);
        assert!(worth_recording(&state, &signed("hi")));
        state.apply(event("hi")).unwrap();
        assert!(!worth_recording(&state, &signed("hi")));
        assert!(worth_recording(&state, &signed("hello")));
    }

    #[test]
    fn saved_requests_round_trip_and_turned_away_ones_stop_asking() {
        let dir = std::env::temp_dir().join(format!("commy-knocks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut knocks = Knocks::default();
        let knock = Knocking { ticket: "t".into(), name: "Rockets".into(), rules: Some("Be kind".into()), members: 3, note: String::new(), asked_at: 5, turned_away: false };
        let sid = ServerId::new();
        knocks.servers.insert(sid, knock.clone());
        knocks.servers.insert(ServerId::new(), Knocking { turned_away: true, ..knock.clone() });
        knocks.save(&dir);
        let loaded = Knocks::load(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(loaded.servers.len(), 2);
        assert_eq!(loaded.waiting(), vec![(sid, knock)]);
    }
}
