use chrono::{DateTime, SecondsFormat, Utc};
use commy_core::channel_key::{ChannelSecret, KeyId};
use commy_core::{Permissions, ServerState, TrackedInvite, UserId};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::sealed::{self, Purpose, SealKey};
use crate::sync_streams::SharedKey;

pub const SCHEME: &str = "commy";
const CODE_ALPHABET: &str = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const CODE_LENGTH: usize = 8;
const MIN_NAME_LEN: usize = 3;
const MAX_NAME_LEN: usize = 32;
const RESERVED_NAMES: &[&str] = &[
    "admin", "commy", "support", "help", "staff", "official", "moderator", "mod", "team", "security", "login",
    "account", "download", "docs", "privacy", "invite", "invites",
];
pub const MAX_INVITE_BACKGROUND_BYTES: usize = 400 * 1024;
pub const MAX_LINK_LEN: usize = 2048;
pub const CREATE_PATH: &str = "/api/auth/commy/invites/create";
pub const RESOLVE_PATH: &str = "/api/auth/commy/invites/resolve";
pub const BACKGROUND_PATH: &str = "/api/auth/commy/invites/background";
pub const REVOKE_PATH: &str = "/api/auth/commy/invites/revoke";

pub fn host_of(origin: &str) -> String {
    origin.split("://").nth(1).unwrap_or(origin).to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expiry {
    Hour,
    Day,
    Week,
    Month,
    Never,
}

impl Expiry {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "1h" => Expiry::Hour,
            "1d" => Expiry::Day,
            "7d" => Expiry::Week,
            "30d" => Expiry::Month,
            "never" => Expiry::Never,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Expiry::Hour => "1h",
            Expiry::Day => "1d",
            Expiry::Week => "7d",
            Expiry::Month => "30d",
            Expiry::Never => "never",
        }
    }

    pub fn length(self) -> Option<chrono::Duration> {
        match self {
            Expiry::Hour => Some(chrono::Duration::hours(1)),
            Expiry::Day => Some(chrono::Duration::days(1)),
            Expiry::Week => Some(chrono::Duration::days(7)),
            Expiry::Month => Some(chrono::Duration::days(30)),
            Expiry::Never => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteLink {
    pub origin: String,
    pub code: String,
    pub key: Option<String>,
}

impl InviteLink {
    pub fn url(&self) -> String {
        match &self.key {
            Some(key) => format!("{}/i/{}#{key}", self.origin, self.code),
            None => format!("{}/i/{}", self.origin, self.code),
        }
    }

    pub fn host(&self) -> String {
        host_of(&self.origin)
    }

    pub fn endpoint(&self, path: &str) -> String {
        let mut url = Url::parse(&format!("{}{path}", self.origin)).expect("an invite origin makes a valid url");
        url.query_pairs_mut().append_pair("code", &self.code);
        url.into()
    }
}

fn is_loopback(host: &str) -> bool {
    host == "localhost" || host == "127.0.0.1" || host == "[::1]" || host.ends_with(".localhost")
}

fn origin_of(url: &Url) -> Option<String> {
    let host = url.host_str()?.to_ascii_lowercase();
    match url.scheme() {
        "https" => {}
        "http" if is_loopback(&host) => {}
        _ => return None,
    }
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    Some(match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    })
}

pub fn normalize_origin(input: &str) -> Option<String> {
    origin_of(&Url::parse(input.trim()).ok()?)
}

pub fn normalize_code(input: &str) -> Option<String> {
    let code = input.trim().to_ascii_uppercase();
    (code.len() == CODE_LENGTH && code.chars().all(|c| CODE_ALPHABET.contains(c))).then_some(code)
}

fn normalize_name(input: &str) -> Option<String> {
    let name = input.trim().to_ascii_lowercase();
    let shape = (MIN_NAME_LEN..=MAX_NAME_LEN).contains(&name.len())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--");
    shape.then_some(name)
}

fn normalize_link_code(input: &str) -> Option<String> {
    normalize_code(input).or_else(|| normalize_name(input))
}

pub fn custom_name(input: &str) -> Result<String, String> {
    let name = normalize_name(input)
        .ok_or("Custom names are 3 to 32 letters, numbers or single dashes, like pixel-pioneers")?;
    if RESERVED_NAMES.contains(&name.as_str()) {
        return Err("That name is reserved. Pick another one.".into());
    }
    Ok(name)
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = input.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn valid_key(fragment: &str) -> Option<String> {
    SealKey::from_text(fragment).ok().map(|_| fragment.to_string())
}

pub fn parse(input: &str) -> Option<InviteLink> {
    let input = input.trim();
    if input.len() > MAX_LINK_LEN {
        return None;
    }
    let url = Url::parse(input).ok()?;
    let mut segments: Vec<&str> = url.path_segments()?.collect();
    if segments.last() == Some(&"") {
        segments.pop();
    }

    let (origin, code) = if url.scheme() == SCHEME {
        if url.host_str() != Some("i") || segments.len() != 2 {
            return None;
        }
        let host = percent_decode(segments[0])?.to_ascii_lowercase();
        let host_ok = !host.is_empty()
            && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
        if !host_ok {
            return None;
        }
        (normalize_origin(&format!("https://{host}"))?, segments[1])
    } else {
        if segments.len() != 2 || segments[0] != "i" || url.query().is_some() {
            return None;
        }
        (origin_of(&url)?, segments[1])
    };
    let key = match url.fragment() {
        None => None,
        Some(fragment) => Some(valid_key(fragment)?),
    };
    Some(InviteLink { origin, code: normalize_link_code(code)?, key })
}

pub fn looks_like_link(input: &str) -> bool {
    parse(input).is_some()
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedInvite {
    pub link: String,
    pub host: String,
    pub server_name: String,
    pub ticket: String,
    pub expires_at: Option<String>,
    pub trusted: bool,
    pub account_server_host: String,
    pub background: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveResponse {
    pub ticket: String,
    pub expires_at: Option<String>,
    #[serde(default)]
    pub has_background: bool,
}

#[derive(Deserialize)]
pub struct BackgroundResponse {
    pub background: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateResponse {
    pub url: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreatedInvite {
    pub url: String,
    pub expires_at: Option<String>,
}

pub fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.trim()).ok().map(|t| t.with_timezone(&Utc))
}

pub fn effective_expiry(
    server: Option<DateTime<Utc>>,
    sealed: Option<SealedExpiry>,
    now: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, String> {
    let sealed = match sealed {
        Some(SealedExpiry::At(t)) if t <= now => return Err("This invite has expired. Ask for a new link.".into()),
        Some(SealedExpiry::At(t)) => Some(t),
        Some(SealedExpiry::Never) | None => None,
    };
    Ok(earliest(server, sealed))
}

pub fn earliest(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

pub fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn clip(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

pub fn revoke_hash(revoke_key: &str) -> String {
    use sha2::Digest;
    data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(revoke_key.as_bytes()))
}

#[derive(Serialize, Deserialize)]
struct SealedInvite {
    #[serde(rename = "t")]
    ticket: String,
    #[serde(rename = "n")]
    name: String,
    #[serde(rename = "e", default, skip_serializing_if = "Option::is_none")]
    expires: Option<i64>,
    #[serde(rename = "p", default, skip_serializing_if = "std::ops::Not::not")]
    permanent: bool,
    #[serde(rename = "k", default, skip_serializing_if = "Option::is_none")]
    community: Option<SharedKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealedExpiry {
    At(DateTime<Utc>),
    Never,
}

#[cfg(test)]
fn seal_ticket(key: &SealKey, raw: &str, name: &str, expiry: SealedExpiry) -> String {
    seal_ticket_with(key, raw, name, expiry, None)
}

pub fn seal_ticket_with(key: &SealKey, raw: &str, name: &str, expiry: SealedExpiry, community: Option<(KeyId, &ChannelSecret)>) -> String {
    let (expires, permanent) = match expiry {
        SealedExpiry::At(t) => (Some(t.timestamp()), false),
        SealedExpiry::Never => (None, true),
    };
    let community = community.map(|(id, secret)| SharedKey::of(id, secret));
    let invite = SealedInvite { ticket: raw.to_string(), name: name.to_string(), expires, permanent, community };
    let payload = serde_json::to_vec(&invite).expect("invite serialization cannot fail");
    key.seal(Purpose::Invite, &payload)
}

pub fn community_key_in(stored: &str, key: &str) -> Option<(KeyId, ChannelSecret)> {
    let bytes = SealKey::from_text(key).ok()?.open(Purpose::Invite, stored.trim()).ok()?;
    serde_json::from_slice::<SealedInvite>(&bytes).ok()?.community?.open()
}

pub fn open_ticket(stored: &str, key: Option<&str>) -> Result<(String, Option<SealedExpiry>), String> {
    let stored = stored.trim();
    match key {
        Some(key) => {
            let key = SealKey::from_text(key).map_err(|_| "This invite link is damaged".to_string())?;
            let bytes = key
                .open(Purpose::Invite, stored)
                .map_err(|_| "This invite link is damaged or was changed".to_string())?;
            let invite: SealedInvite =
                serde_json::from_slice(&bytes).map_err(|_| "The invite server sent an invalid invite".to_string())?;
            let expiry = match (invite.permanent, invite.expires) {
                (true, _) => Some(SealedExpiry::Never),
                (false, Some(secs)) => Some(SealedExpiry::At(
                    DateTime::from_timestamp(secs, 0).ok_or("The invite server sent an invalid invite")?,
                )),
                (false, None) => None,
            };
            Ok((invite.ticket, expiry))
        }
        None if sealed::is_sealed(stored) => Err("This invite link is missing its key. Ask for a new link.".into()),
        None => Ok((stored.to_string(), None)),
    }
}

pub fn expiry_choice(expires: Option<&str>) -> Result<Expiry, String> {
    match expires {
        None => Ok(Expiry::Week),
        Some(value) => Expiry::parse(value).ok_or_else(|| "unknown invite expiry".to_string()),
    }
}

pub fn name_choice(name: Option<&str>) -> Result<Option<String>, String> {
    name.filter(|n| !n.trim().is_empty()).map(custom_name).transpose()
}

pub fn check_create(state: &ServerState, me: UserId, expiry: Expiry, custom: bool) -> Result<(), String> {
    let perms = state.server.permissions_for(me);
    if !perms.allows(Permissions::CREATE_INVITES) {
        return Err("you do not have permission to invite people".into());
    }
    if custom && !perms.allows(Permissions::MANAGE_SERVER) {
        return Err("you do not have permission to pick a custom invite name".into());
    }
    if expiry == Expiry::Never && !state.server.public {
        return Err("Only links to a public server can last forever".into());
    }
    Ok(())
}

pub const OLD_LINK: &str = "This link was made by an older version of Commy, so its full link cannot be shown again. Make a new one to share.";

pub fn link_key(revoke_key: &str, custom: bool) -> (SealKey, String) {
    SealKey::for_invite_link(revoke_key, custom)
}

pub fn is_custom_code(code: &str) -> bool {
    normalize_code(code).as_deref() != Some(code)
}

pub fn tracked_link(invite: &TrackedInvite) -> InviteLink {
    let (_, key) = link_key(&invite.revoke_key, is_custom_code(&invite.code));
    InviteLink { origin: invite.origin.clone(), code: invite.code.clone(), key: Some(key) }
}

pub fn same_code(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

pub fn new_revoke_key() -> String {
    SealKey::generate().to_text()
}

pub struct CreateRequest<'a> {
    pub raw_ticket: &'a str,
    pub server_name: &'a str,
    pub expiry: Expiry,
    pub custom: Option<&'a str>,
    pub community: Option<(KeyId, &'a ChannelSecret)>,
    pub background: Option<&'a str>,
    pub revoke_key: &'a str,
}

pub fn create_body(key: &SealKey, request: &CreateRequest, now: DateTime<Utc>) -> (serde_json::Value, Option<DateTime<Utc>>) {
    let sealed_expiry = match request.expiry.length() {
        Some(length) => SealedExpiry::At(now + length),
        None => SealedExpiry::Never,
    };
    let mut body = serde_json::json!({
        "ticket": seal_ticket_with(key, request.raw_ticket, request.server_name, sealed_expiry, request.community),
        "expiresIn": request.expiry.as_str(),
        "revokeHash": revoke_hash(request.revoke_key),
    });
    if let Some(name) = request.custom {
        body["code"] = serde_json::json!(name);
    }
    if let Some(background) = request.background {
        body["background"] = serde_json::json!(background);
    }
    let sealed_at = match sealed_expiry {
        SealedExpiry::At(t) => Some(t),
        SealedExpiry::Never => None,
    };
    (body, sealed_at)
}

pub struct Created {
    pub invite: CreatedInvite,
    pub code: String,
    pub expires_at: Option<DateTime<Utc>>,
}

pub fn created(origin: &str, custom: Option<&str>, key_text: String, reply: CreateResponse, sealed_at: Option<DateTime<Utc>>) -> Result<Created, String> {
    let Some(link) = parse(&reply.url).filter(|l| l.origin == origin && l.key.is_none()) else {
        return Err("The invite server sent an unexpected link".into());
    };
    if custom.is_some_and(|n| !same_code(n, &link.code)) {
        return Err("Your account server does not support custom invite names yet".into());
    }
    let code = custom.map_or_else(|| link.code.clone(), str::to_string);
    let url = InviteLink { code: code.clone(), key: Some(key_text), ..link }.url();
    let expires_at = earliest(reply.expires_at.as_deref().and_then(parse_time), sealed_at);
    Ok(Created { invite: CreatedInvite { url, expires_at: expires_at.map(stamp) }, code, expires_at })
}

pub fn track_event(server: commy_core::ServerId, creator: UserId, origin: String, created: &Created, revoke_key: String, now_ms: i64) -> commy_core::ServerEvent {
    let invite = TrackedInvite {
        origin,
        code: created.code.clone(),
        creator,
        created_at: now_ms,
        expires_at: created.expires_at.map(|t| t.timestamp_millis()),
        revoke_key,
    };
    commy_core::ServerEvent::InviteTrack { server, invite }
}

pub struct Opened {
    pub ticket: crate::ServerTicket,
    pub expires_at: Option<DateTime<Utc>>,
    pub community: Option<(KeyId, ChannelSecret)>,
}

pub fn open_reply(link: &InviteLink, reply: &ResolveResponse, now: DateTime<Utc>) -> Result<Opened, String> {
    let (raw, sealed_expiry) = open_ticket(&reply.ticket, link.key.as_deref())?;
    let ticket = crate::ServerTicket::decode(&raw).map_err(|_| "The invite server sent an invalid invite".to_string())?;
    let community = link.key.as_deref().and_then(|key| community_key_in(&reply.ticket, key));
    let expires_at = effective_expiry(reply.expires_at.as_deref().and_then(parse_time), sealed_expiry, now)?;
    Ok(Opened { ticket, expires_at, community })
}

pub fn resolved(link: &InviteLink, opened: &Opened, account_origin: &str, background: Option<String>) -> ResolvedInvite {
    ResolvedInvite {
        link: link.url(),
        host: link.host(),
        server_name: clip(&opened.ticket.server_name, 100),
        ticket: opened.ticket.encode(),
        expires_at: opened.expires_at.map(stamp),
        trusted: link.origin == account_origin,
        account_server_host: host_of(account_origin),
        background,
    }
}

pub fn open_background(sealed: &str, key: &str) -> Option<Vec<u8>> {
    let bytes = SealKey::from_text(key).ok()?.open(Purpose::InviteBackground, sealed).ok()?;
    (bytes.len() <= MAX_INVITE_BACKGROUND_BYTES).then_some(bytes)
}

#[derive(Debug, Clone, Serialize)]
pub struct InviteView {
    pub origin: String,
    pub host: String,
    pub code: String,
    pub custom: bool,
    pub creator: String,
    pub creator_name: String,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub can_revoke: bool,
}

pub fn invite_views(state: &ServerState, me: UserId, now_ms: i64) -> Vec<InviteView> {
    let manager = state.server.permissions_for(me).allows(Permissions::MANAGE_SERVER);
    state
        .tracked_invites(now_ms)
        .into_iter()
        .filter(|i| manager || i.creator == me)
        .map(|i| InviteView {
            origin: i.origin.clone(),
            host: host_of(&i.origin),
            code: i.code.clone(),
            custom: is_custom_code(&i.code),
            creator: i.creator.to_string(),
            creator_name: commy_core::audit::member_name(state, i.creator).unwrap_or_else(|| "Someone".into()),
            created_at: i.created_at,
            expires_at: i.expires_at,
            can_revoke: true,
        })
        .collect()
}

pub fn revocable(state: &ServerState, me: UserId, origin: &str, code: &str) -> Result<TrackedInvite, String> {
    let invite = state.tracked_invite(origin, code).cloned().ok_or("That invite is already gone")?;
    let manager = state.server.permissions_for(me).allows(Permissions::MANAGE_SERVER);
    if !manager && invite.creator != me {
        return Err("Only its maker or someone who can manage the server can revoke this invite".into());
    }
    Ok(invite)
}

pub fn revoke_body(invite: &TrackedInvite) -> serde_json::Value {
    serde_json::json!({ "code": invite.code, "secret": invite.revoke_key })
}

pub fn revoke_event(server: commy_core::ServerId, invite: TrackedInvite) -> commy_core::ServerEvent {
    commy_core::ServerEvent::InviteRevoke { server, origin: invite.origin, code: invite.code, creator: invite.creator }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_created_invite_round_trips_through_resolve_with_its_community_key() {
        let server = commy_core::ServerId::new();
        let raw = crate::ServerTicket::new(server, "Pixel Pioneers", [3; 32], Vec::new()).encode();
        let (id, secret) = (KeyId::random(), ChannelSecret::random());
        let revoke_key = new_revoke_key();
        let (key, key_text) = link_key(&revoke_key, false);
        let request = CreateRequest { raw_ticket: &raw, server_name: "Pixel Pioneers", expiry: Expiry::Day, custom: None, community: Some((id, &secret)), background: None, revoke_key: &revoke_key };
        let now = Utc::now();
        let (body, sealed_at) = create_body(&key, &request, now);
        assert_eq!(body["expiresIn"], "1d");
        assert_eq!(body["revokeHash"], revoke_hash(&revoke_key));
        assert!(!body["ticket"].as_str().unwrap().contains(&raw));
        let reply = CreateResponse { url: "https://commy.chat/i/AB23CD45".into(), expires_at: None };
        let made = created("https://commy.chat", None, key_text, reply, sealed_at).unwrap();
        assert_eq!(made.code, "AB23CD45");
        let link = parse(&made.invite.url).unwrap();
        let resolve = ResolveResponse { ticket: body["ticket"].as_str().unwrap().into(), expires_at: None, has_background: false };
        let opened = open_reply(&link, &resolve, now).unwrap();
        assert_eq!(opened.ticket.server_id, server);
        assert_eq!(opened.community, Some((id, secret)));
        let shown = resolved(&link, &opened, "https://commy.chat", None);
        assert!(shown.trusted);
        assert_eq!(shown.server_name, "Pixel Pioneers");
        assert!(open_reply(&link, &resolve, now + chrono::Duration::days(2)).is_err());
        let elsewhere = CreateResponse { url: "https://evil.example/i/AB23CD45".into(), expires_at: None };
        assert!(created("https://commy.chat", None, "k".into(), elsewhere, None).is_err());
        let event = track_event(server, UserId::new(), "https://commy.chat".into(), &made, revoke_key, 1);
        let commy_core::ServerEvent::InviteTrack { invite, .. } = event else { panic!("not a tracked invite") };
        assert_eq!(tracked_link(&invite).url(), made.invite.url);
    }

    #[test]
    fn an_eight_letter_custom_name_keeps_its_name_and_its_full_link() {
        let server = commy_core::ServerId::new();
        let raw = crate::ServerTicket::new(server, "Gabagool", [3; 32], Vec::new()).encode();
        let revoke_key = new_revoke_key();
        let (key, key_text) = link_key(&revoke_key, true);
        assert_eq!(key_text.len(), sealed::SHORT_KEY_TEXT_LEN);
        let request = CreateRequest { raw_ticket: &raw, server_name: "Gabagool", expiry: Expiry::Week, custom: Some("partyzzz"), community: None, background: None, revoke_key: &revoke_key };
        let now = Utc::now();
        let (body, sealed_at) = create_body(&key, &request, now);
        let reply = CreateResponse { url: "https://commy.chat/i/PARTYZZZ".into(), expires_at: None };
        let made = created("https://commy.chat", Some("partyzzz"), key_text, reply, sealed_at).unwrap();
        assert_eq!(made.code, "partyzzz");
        assert!(made.invite.url.starts_with("https://commy.chat/i/partyzzz#"));
        let link = parse(&made.invite.url).unwrap();
        let resolve = ResolveResponse { ticket: body["ticket"].as_str().unwrap().into(), expires_at: None, has_background: false };
        assert_eq!(open_reply(&link, &resolve, now).unwrap().ticket.server_id, server);
        let event = track_event(server, UserId::new(), "https://commy.chat".into(), &made, revoke_key, 1);
        let commy_core::ServerEvent::InviteTrack { invite, .. } = event else { panic!("not a tracked invite") };
        assert!(is_custom_code(&invite.code));
        assert_eq!(tracked_link(&invite).url(), made.invite.url);
        assert!(!is_custom_code("AB23CD45"));
    }

    #[test]
    fn only_inviters_make_links_and_only_managers_name_them_or_make_them_forever() {
        let owner = UserId::new();
        let mut state = ServerState::new(commy_core::Server::new("Hangout", owner));
        let profile = commy_core::UserProfile::new("member");
        let member = profile.id;
        state.apply(commy_core::ServerEvent::MemberJoin { server: state.server.id, profile }).unwrap();
        assert!(check_create(&state, owner, Expiry::Week, true).is_ok());
        assert!(check_create(&state, owner, Expiry::Never, false).is_err());
        assert!(check_create(&state, member, Expiry::Week, true).is_err());
        assert_eq!(expiry_choice(None), Ok(Expiry::Week));
        assert!(expiry_choice(Some("90d")).is_err());
        assert_eq!(name_choice(Some("  ")), Ok(None));
        assert!(name_choice(Some("admin")).is_err());
    }

    fn test_key() -> String {
        data_encoding::BASE64URL_NOPAD.encode(&(0u8..32).collect::<Vec<_>>())
    }

    fn link(origin: &str, code: &str) -> Option<InviteLink> {
        Some(InviteLink { origin: origin.into(), code: code.into(), key: None })
    }

    fn keyed(origin: &str, code: &str) -> Option<InviteLink> {
        Some(InviteLink { origin: origin.into(), code: code.into(), key: Some(test_key()) })
    }

    #[test]
    fn parses_links_with_keys() {
        let key = test_key();
        assert_eq!(parse(&format!("https://commy.chat/i/AB23CD45#{key}")), keyed("https://commy.chat", "AB23CD45"));
        assert_eq!(parse(&format!("commy://i/commy.chat/AB23CD45#{key}")), keyed("https://commy.chat", "AB23CD45"));
        let l = parse(&format!("commy://i/localhost%3A8443/AB23CD45#{key}")).unwrap();
        assert_eq!(l.url(), format!("https://localhost:8443/i/AB23CD45#{key}"));
        assert_eq!(parse(&l.url()), Some(l));
        let too_long = format!("https://commy.chat/i/AB23CD45#{key}A");
        let bad_char = format!("https://commy.chat/i/AB23CD45#{}+", &key[..key.len() - 1]);
        for bad in ["https://commy.chat/i/AB23CD45#", "https://commy.chat/i/AB23CD45#short", &too_long, &bad_char] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn sealed_tickets_need_their_key() {
        let text = test_key();
        let key = SealKey::from_text(&text).unwrap();
        let sealed = seal_ticket(&key, "commyticket", "Pixel Pioneers", SealedExpiry::Never);
        assert!(!sealed.contains("commyticket") && !sealed.contains("Pixel"));
        assert_eq!(open_ticket(&sealed, Some(&text)).unwrap(), ("commyticket".into(), Some(SealedExpiry::Never)));
        assert!(open_ticket(&sealed, None).is_err());
        assert!(open_ticket(&sealed, Some(&SealKey::generate().to_text())).is_err());
        assert!(open_ticket("commyticket", Some(&text)).is_err());
        assert_eq!(open_ticket(" commyticket ", None).unwrap(), ("commyticket".into(), None));
    }

    #[test]
    fn the_sealed_expiry_wins() {
        let text = test_key();
        let key = SealKey::from_text(&text).unwrap();
        let at = DateTime::from_timestamp(1_800_000_000, 0).unwrap();
        let sealed = seal_ticket(&key, "commyticket", "Pixel Pioneers", SealedExpiry::At(at));
        let (_, expiry) = open_ticket(&sealed, Some(&text)).unwrap();
        assert_eq!(expiry, Some(SealedExpiry::At(at)));

        let before = at - chrono::Duration::days(1);
        let later = at + chrono::Duration::days(300);
        assert_eq!(effective_expiry(Some(later), expiry, before), Ok(Some(at)));
        assert_eq!(effective_expiry(None, expiry, before), Ok(Some(at)));
        assert!(effective_expiry(Some(later), expiry, at).is_err());
        assert!(effective_expiry(None, expiry, later).is_err());
        assert_eq!(effective_expiry(Some(before), expiry, before - chrono::Duration::hours(1)), Ok(Some(before)));
        assert_eq!(effective_expiry(None, Some(SealedExpiry::Never), later), Ok(None));
        assert_eq!(effective_expiry(Some(at), None, before), Ok(Some(at)));
    }

    #[test]
    fn a_sealed_invite_can_carry_the_community_key() {
        let text = test_key();
        let key = SealKey::from_text(&text).unwrap();
        let (id, secret) = (KeyId::random(), ChannelSecret::random());
        let sealed = seal_ticket_with(&key, "commyticket", "Pixel Pioneers", SealedExpiry::Never, Some((id, &secret)));
        assert!(!sealed.contains(&data_encoding::BASE64URL_NOPAD.encode(secret.bytes())));
        assert_eq!(open_ticket(&sealed, Some(&text)).unwrap(), ("commyticket".into(), Some(SealedExpiry::Never)));
        assert_eq!(community_key_in(&sealed, &text), Some((id, secret)));
        assert_eq!(community_key_in(&sealed, &SealKey::generate().to_text()), None);
        let plain = seal_ticket(&key, "commyticket", "Pixel Pioneers", SealedExpiry::Never);
        assert_eq!(community_key_in(&plain, &text), None);
    }

    #[test]
    fn invites_sealed_before_expiries_still_open() {
        let text = test_key();
        let key = SealKey::from_text(&text).unwrap();
        let old = key.seal(Purpose::Invite, br#"{"t":"commyticket","n":"Pixel Pioneers"}"#);
        assert_eq!(open_ticket(&old, Some(&text)).unwrap(), ("commyticket".into(), None));
    }

    #[test]
    fn expiry_choices_round_trip() {
        for value in ["1h", "1d", "7d", "30d", "never"] {
            assert_eq!(Expiry::parse(value).unwrap().as_str(), value);
        }
        assert_eq!(Expiry::parse("90d"), None);
        assert_eq!(Expiry::Never.length(), None);
        assert_eq!(Expiry::Month.length(), Some(chrono::Duration::days(30)));
    }

    #[test]
    fn parses_https_links() {
        assert_eq!(parse("https://commy.chat/i/AB23CD45"), link("https://commy.chat", "AB23CD45"));
        assert_eq!(parse("  https://Chat.Example.org/i/ab23cd45/ "), link("https://chat.example.org", "AB23CD45"));
        assert_eq!(parse("https://example.org:8443/i/AB23CD45"), link("https://example.org:8443", "AB23CD45"));
        assert_eq!(parse("http://localhost:3000/i/AB23CD45"), link("http://localhost:3000", "AB23CD45"));
    }

    #[test]
    fn parses_deep_links() {
        assert_eq!(parse("commy://i/commy.chat/AB23CD45"), link("https://commy.chat", "AB23CD45"));
        assert_eq!(parse("commy://i/localhost%3A8443/AB23CD45"), link("https://localhost:8443", "AB23CD45"));
        assert_eq!(parse("commy://i/Example.ORG/ab23cd45/"), link("https://example.org", "AB23CD45"));
    }

    #[test]
    fn rejects_everything_else() {
        for bad in [
            "",
            "commyabcdefghijklmnopqrstuvwxyz234567abcdefgh",
            "http://example.org/i/AB23CD45",
            "ftp://example.org/i/AB23CD45",
            "https://example.org/invite/AB23CD45",
            "https://example.org/i/AB23CD45/extra",
            "https://example.org/i/AB23CD45?next=evil",
            "https://user:pw@example.org/i/AB23CD45",
            "https://example.org/i/AB",
            "https://example.org/i/AB_23",
            "commy://x/example.org/AB23CD45",
            "commy://i/evil.com%2Fpath/AB23CD45",
            "commy://i/a%40evil.com/AB23CD45",
            "commy://i/example.org/AB23CD45/more",
            "commy://i//AB23CD45",
            "commy://i/%ZZ/AB23CD45",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        assert_eq!(parse(&format!("https://example.org/i/{}", "A".repeat(MAX_LINK_LEN))), None);
        assert_eq!(parse("https://example.org/i/AB23CD4O").unwrap().code, "ab23cd4o");
    }

    #[test]
    fn custom_names_parse_with_a_short_key() {
        let (_, short) = SealKey::generate_short();
        let l = parse(&format!("https://commy.chat/i/Pixel-Pioneers#{short}")).unwrap();
        assert_eq!(l, InviteLink { origin: "https://commy.chat".into(), code: "pixel-pioneers".into(), key: Some(short.clone()) });
        assert_eq!(parse(&l.url()), Some(l));
        assert_eq!(parse(&format!("commy://i/commy.chat/team42#{short}")).unwrap().code, "team42");
        for bad in ["https://commy.chat/i/-pixel", "https://commy.chat/i/pixel--pioneers", "https://commy.chat/i/pi", "https://commy.chat/i/pixel_pioneers"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn custom_names_are_checked_before_they_are_claimed() {
        assert_eq!(custom_name("  Pixel-Pioneers "), Ok("pixel-pioneers".into()));
        assert_eq!(custom_name("abc"), Ok("abc".into()));
        assert_eq!(custom_name(&"a".repeat(MAX_NAME_LEN)), Ok("a".repeat(MAX_NAME_LEN)));
        for bad in ["ab", "", "has space", "-lead", "trail-", "two--dashes", "emoji\u{1f600}", "admin", "Commy"] {
            assert!(custom_name(bad).is_err(), "{bad}");
        }
        assert!(custom_name(&"a".repeat(MAX_NAME_LEN + 1)).is_err());
        assert_eq!(custom_name("pixel-pi"), Ok("pixel-pi".into()));
        assert_eq!(custom_name("Gabagool"), Ok("gabagool".into()));
        assert_eq!(custom_name("partyzzz"), Ok("partyzzz".into()));
    }

    #[test]
    fn link_round_trips_through_its_url() {
        let l = parse("commy://i/localhost%3A8443/AB23CD45").unwrap();
        assert_eq!(l.url(), "https://localhost:8443/i/AB23CD45");
        assert_eq!(l.host(), "localhost:8443");
        assert_eq!(parse(&l.url()), Some(l));
    }

    #[test]
    fn origins_are_normalized() {
        assert_eq!(normalize_origin("https://Commy.Chat/"), Some("https://commy.chat".into()));
        assert_eq!(normalize_origin("https://commy.chat/some/path"), Some("https://commy.chat".into()));
        assert_eq!(normalize_origin("http://commy.chat"), None);
        assert_eq!(normalize_origin("http://127.0.0.1:3001"), Some("http://127.0.0.1:3001".into()));
        assert_eq!(normalize_origin("not a url"), None);
    }
}
