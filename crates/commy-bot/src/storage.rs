use anyhow::{bail, Context, Result};
use commy_core::identity::{self, Identity};
use commy_core::{ServerId, ServerState, Snapshot, UserProfile};
use crate::webhook::{Webhook, MAX_WEBHOOKS};
use commy_net::ServerTicket;
use data_encoding::HEXLOWER;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

pub const NODE_FILE: &str = "node.key";
pub const PROFILE_FILE: &str = "profile.json";
pub const SERVERS_DIR: &str = "servers";
pub const TOKEN_FILE: &str = "bridge.token";
pub const WEBHOOKS_FILE: &str = "webhooks.json";
pub const MAX_NAME_CHARS: usize = 32;

#[derive(Serialize, Deserialize)]
struct SavedServer {
    ticket: String,
    #[serde(default)]
    state: Option<Snapshot>,
}

pub struct DataDir {
    root: PathBuf,
}

impl DataDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn identity(&self) -> Result<Identity> {
        identity::load_or_create(&self.root).context("could not read or create the bot's identity key")
    }

    pub fn node_secret(&self) -> Result<[u8; 32]> {
        let path = self.root.join(NODE_FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let bytes = HEXLOWER.decode(text.trim().as_bytes()).ok();
                return bytes.and_then(|b| <[u8; 32]>::try_from(b).ok()).context("node.key is damaged");
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("could not read node.key"),
        }
        let secret: [u8; 32] = rand::random();
        write_atomically(&self.root, &path, HEXLOWER.encode(&secret).as_bytes())?;
        Ok(secret)
    }

    pub fn profile(&self) -> Result<Option<UserProfile>> {
        match std::fs::read(self.root.join(PROFILE_FILE)) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes).context("profile.json is damaged")?)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).context("could not read profile.json"),
        }
    }

    pub fn create_profile(&self, name: &str) -> Result<UserProfile> {
        if self.profile()?.is_some() {
            bail!("this folder already has a bot; use another --data folder for a second one");
        }
        let name = valid_name(name)?;
        let identity = self.identity()?;
        self.node_secret()?;
        let mut profile = UserProfile::new(name);
        profile.identity = Some(identity.public());
        self.save_profile(&profile)?;
        Ok(profile)
    }

    pub fn save_profile(&self, profile: &UserProfile) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(profile)?;
        write_atomically(&self.root, &self.root.join(PROFILE_FILE), &bytes)
    }

    pub fn servers(&self) -> Result<Vec<(ServerTicket, Option<ServerState>)>> {
        let dir = self.root.join(SERVERS_DIR);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).context("could not list saved servers"),
        };
        let mut out = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let saved: SavedServer = match std::fs::read(&path).map_err(anyhow::Error::from).and_then(|b| Ok(serde_json::from_slice(&b)?)) {
                Ok(saved) => saved,
                Err(e) => {
                    tracing::warn!("skipping {}: {e}", path.display());
                    continue;
                }
            };
            let Ok(ticket) = ServerTicket::decode(&saved.ticket) else {
                tracing::warn!("skipping {}: the invite is damaged", path.display());
                continue;
            };
            out.push((ticket, saved.state.map(ServerState::from_snapshot)));
        }
        out.sort_by(|a, b| a.0.server_name.cmp(&b.0.server_name));
        Ok(out)
    }

    pub fn save_server(&self, ticket: &ServerTicket, state: Option<&ServerState>) -> Result<()> {
        let saved = SavedServer { ticket: ticket.encode(), state: state.map(ServerState::snapshot) };
        let dir = self.root.join(SERVERS_DIR);
        let bytes = serde_json::to_vec(&saved)?;
        write_atomically(&dir, &dir.join(server_file(ticket.server_id)), &bytes)
    }

    pub fn bridge_token(&self, reset: bool) -> Result<String> {
        let path = self.root.join(TOKEN_FILE);
        if !reset {
            match std::fs::read_to_string(&path) {
                Ok(text) if !text.trim().is_empty() => return Ok(text.trim().to_string()),
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).context("could not read bridge.token"),
            }
        }
        let token = HEXLOWER.encode(&rand::random::<[u8; 32]>());
        write_atomically(&self.root, &path, token.as_bytes())?;
        Ok(token)
    }

    pub fn webhooks(&self) -> Result<Vec<Webhook>> {
        match std::fs::read(self.root.join(WEBHOOKS_FILE)) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("webhooks.json is damaged"),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e).context("could not read webhooks.json"),
        }
    }

    pub fn add_webhook(&self, hook: Webhook) -> Result<Webhook> {
        let mut hooks = self.webhooks()?;
        if hooks.len() >= MAX_WEBHOOKS {
            bail!("this bot already has {MAX_WEBHOOKS} webhooks; remove one first");
        }
        hooks.push(hook.clone());
        self.save_webhooks(&hooks)?;
        Ok(hook)
    }

    pub fn remove_webhook(&self, id: &str) -> Result<bool> {
        let mut hooks = self.webhooks()?;
        let before = hooks.len();
        hooks.retain(|h| h.id != id.trim());
        if hooks.len() == before {
            return Ok(false);
        }
        self.save_webhooks(&hooks)?;
        Ok(true)
    }

    fn save_webhooks(&self, hooks: &[Webhook]) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(hooks)?;
        write_atomically(&self.root, &self.root.join(WEBHOOKS_FILE), &bytes)
    }

    pub fn has_server(&self, server: ServerId) -> bool {
        self.root.join(SERVERS_DIR).join(server_file(server)).exists()
    }
}

fn server_file(server: ServerId) -> String {
    format!("{server}.json")
}

pub fn valid_name(name: &str) -> Result<String> {
    let name = name.trim();
    let ok = !name.is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && name.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'));
    if !ok {
        bail!("a bot name is 1 to {MAX_NAME_CHARS} letters, digits, spaces, dots, dashes or underscores");
    }
    Ok(name.to_string())
}

pub fn ticket_in(text: &str) -> Result<ServerTicket> {
    let text = text.trim();
    if text.starts_with("http://") || text.starts_with("https://") {
        bail!("use Copy Invite Code in Invite People (the code starts with commy), not a link");
    }
    let ticket = text
        .split_whitespace()
        .map(|word| word.trim_matches(|c: char| !c.is_ascii_alphanumeric()))
        .filter(|word| word.starts_with("commy"))
        .find_map(|word| ServerTicket::decode(word).ok())
        .context("no Commy invite found in that text")?;
    if ticket.owner.is_none() {
        bail!("this invite code is from an older version; copy a new one from Invite People in the app");
    }
    Ok(ticket)
}

fn write_atomically(dir: &Path, path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    let tmp = dir.join(format!(".{}.tmp", HEXLOWER.encode(&rand::random::<[u8; 6]>())));
    let result = identity::write_private(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.with_context(|| format!("could not write {}", path.display()))
}
