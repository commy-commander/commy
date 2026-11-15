use std::net::SocketAddr;
use std::path::PathBuf;

use crate::limits::Limits;

pub const DEFAULT_PORT: u16 = 3002;
pub const DEFAULT_RETENTION_DAYS: u32 = 90;

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    pub database_url: Option<String>,
    pub retention_days: u32,
    pub trust_proxy: bool,
    pub limits: Limits,
    pub fcm_service_account: Option<String>,
    pub apns: Option<ApnsSettings>,
    pub version: Option<String>,
    pub relay: Option<String>,
    pub admin: Option<AdminSettings>,
    pub operator: Option<String>,
    pub abuse_contact: Option<Contact>,
    pub smtp: Option<SmtpSettings>,
    pub server_key: Option<KeySeed>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AdminSettings {
    pub password: String,
    pub listen: SocketAddr,
    pub domain: Option<String>,
    pub caddy_data: PathBuf,
    pub custom_tls: PathBuf,
    pub caddyfile: PathBuf,
    pub caddy_admin: String,
    pub caddy_tls: String,
    pub relay_probe: String,
    pub releases: String,
}

impl std::fmt::Debug for AdminSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdminSettings").field("listen", &self.listen).field("domain", &self.domain).finish_non_exhaustive()
    }
}

pub const ADMIN_PORT: u16 = 3003;
pub const MIN_ADMIN_PASSWORD_CHARS: usize = 16;

fn admin_settings(env: &impl Fn(&str) -> Option<String>) -> Result<Option<AdminSettings>, String> {
    let set = |name: &str| env(name).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let Some(password) = set("SYNC_ADMIN_PASSWORD") else {
        return Ok(None);
    };
    if password.chars().count() < MIN_ADMIN_PASSWORD_CHARS {
        return Err(format!("SYNC_ADMIN_PASSWORD must be at least {MIN_ADMIN_PASSWORD_CHARS} characters"));
    }
    let port = match set("SYNC_ADMIN_PORT") {
        Some(text) => text.parse::<u16>().map_err(|_| "SYNC_ADMIN_PORT must be a TCP port".to_string())?,
        None => ADMIN_PORT,
    };
    Ok(Some(AdminSettings {
        password,
        listen: SocketAddr::from(([0, 0, 0, 0], port)),
        domain: set("SYNC_DOMAIN").map(|d| d.to_ascii_lowercase()),
        caddy_data: PathBuf::from(set("SYNC_CADDY_DATA").unwrap_or_else(|| "/caddy-data".into())),
        custom_tls: PathBuf::from(set("SYNC_CUSTOM_TLS").unwrap_or_else(|| "/custom-tls".into())),
        caddyfile: PathBuf::from(set("SYNC_CADDYFILE").unwrap_or_else(|| "/etc/commy/Caddyfile".into())),
        caddy_admin: set("SYNC_CADDY_ADMIN").unwrap_or_else(|| "http://caddy:2019".into()),
        caddy_tls: set("SYNC_CADDY_TLS").unwrap_or_else(|| "caddy:443".into()),
        relay_probe: set("SYNC_RELAY_PROBE").unwrap_or_else(|| "http://relay:3340/generate_204".into()),
        releases: set("SYNC_RELEASES_URL").unwrap_or_else(|| "https://releases.commy.chat/stable/latest.json".into()),
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Contact {
    Email(String),
    Page(String),
}

impl Contact {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let text = raw.trim();
        let plain = !text.is_empty() && text.len() <= 254 && !text.chars().any(|c| c.is_whitespace() || c.is_control() || "<>\"".contains(c));
        if plain && text.starts_with("https://") && text.len() > "https://".len() {
            return Ok(Contact::Page(text.to_string()));
        }
        let email = text.strip_prefix("mailto:").unwrap_or(text);
        match email.split_once('@') {
            Some((name, domain)) if plain && !name.is_empty() && domain.contains('.') && !domain.contains('@') => Ok(Contact::Email(email.to_string())),
            _ => Err("SYNC_ABUSE_CONTACT must be an email address or an https:// page".into()),
        }
    }

    pub fn shown(&self) -> &str {
        match self {
            Contact::Email(address) | Contact::Page(address) => address,
        }
    }

    pub fn email(&self) -> Option<&str> {
        match self {
            Contact::Email(address) => Some(address),
            Contact::Page(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmtpSettings {
    Url { url: String, from: String },
    Host { host: String, port: u16, user: Option<String>, password: Option<String>, from: String },
}

impl SmtpSettings {
    pub fn from(&self) -> &str {
        match self {
            SmtpSettings::Url { from, .. } | SmtpSettings::Host { from, .. } => from,
        }
    }
}

const DEFAULT_SMTP_PORT: u16 = 587;

fn smtp_settings(env: &impl Fn(&str) -> Option<String>) -> Result<Option<SmtpSettings>, String> {
    let set = |name: &str| env(name).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let user = set("SMTP_USER");
    let from = || set("EMAIL_FROM").or_else(|| user.clone()).ok_or("EMAIL_FROM or SMTP_USER must be set to send reports by email".to_string());
    if let Some(url) = set("SMTP_URL") {
        return Ok(Some(SmtpSettings::Url { url, from: from()? }));
    }
    let Some(host) = set("SMTP_HOST") else {
        return Ok(None);
    };
    let port = match set("SMTP_PORT") {
        Some(text) => text.parse::<u16>().map_err(|_| "SMTP_PORT must be a TCP port".to_string())?,
        None => DEFAULT_SMTP_PORT,
    };
    Ok(Some(SmtpSettings::Host { host, port, user: user.clone(), password: set("SMTP_PASSWORD"), from: from()? }))
}

#[derive(Clone, PartialEq, Eq)]
pub struct KeySeed(pub [u8; 32]);

impl std::fmt::Debug for KeySeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeySeed(hidden)")
    }
}

fn server_key_seed(env: &impl Fn(&str) -> Option<String>) -> Result<Option<KeySeed>, String> {
    let Some(text) = env("SYNC_SERVER_KEY").map(|v| v.trim().to_ascii_lowercase()).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let bytes = data_encoding::HEXLOWER.decode(text.as_bytes()).map_err(|_| "SYNC_SERVER_KEY must be 64 hex characters".to_string())?;
    let seed: [u8; 32] = bytes.try_into().map_err(|_| "SYNC_SERVER_KEY must be 64 hex characters".to_string())?;
    Ok(Some(KeySeed(seed)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApnsSettings {
    pub key: String,
    pub key_id: String,
    pub team_id: String,
    pub topic: String,
}

fn apns_settings(env: &impl Fn(&str) -> Option<String>) -> Result<Option<ApnsSettings>, String> {
    let set = |name: &str| env(name).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let Some(key) = set("APNS_KEY") else {
        return Ok(None);
    };
    let key_id = set("APNS_KEY_ID").ok_or("APNS_KEY_ID must be set with APNS_KEY")?;
    let team_id = set("APNS_TEAM_ID").ok_or("APNS_TEAM_ID must be set with APNS_KEY")?;
    let topic = set("APNS_TOPIC").unwrap_or_else(|| crate::apns::DEFAULT_TOPIC.to_string());
    Ok(Some(ApnsSettings { key, key_id, team_id, topic }))
}

fn relay_url(env: &impl Fn(&str) -> Option<String>) -> Result<Option<String>, String> {
    let Some(url) = env("SYNC_RELAY_URL").map(|v| v.trim().trim_end_matches('/').to_string()).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let host = url.strip_prefix("https://").ok_or("SYNC_RELAY_URL must start with https://")?;
    if host.is_empty() || host.contains(['/', '?', '#', '@', ' ']) {
        return Err("SYNC_RELAY_URL must be https:// followed by a domain, like https://sync.example.org".into());
    }
    Ok(Some(url))
}

fn release_version(env: &impl Fn(&str) -> Option<String>) -> Option<String> {
    env("COMMY_VERSION").map(|v| v.trim().trim_start_matches('v').to_string()).filter(|v| !v.is_empty() && v.len() <= 40)
}

impl Config {
    pub fn from_env(env: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let port = match env("PORT") {
            Some(text) => text.trim().parse::<u16>().map_err(|_| "PORT must be a TCP port".to_string())?,
            None => DEFAULT_PORT,
        };
        let database_url = env("DATABASE_URL").map(|url| url.trim().to_string()).filter(|url| !url.is_empty());
        if let Some(url) = &database_url {
            if !url.starts_with("postgres") {
                return Err("DATABASE_URL must be a postgres:// connection string".into());
            }
        }
        let retention_days = match env("SYNC_RETENTION_DAYS") {
            Some(text) => text.trim().parse::<u32>().map_err(|_| "SYNC_RETENTION_DAYS must be a whole number of days, 0 keeps everything".to_string())?,
            None => DEFAULT_RETENTION_DAYS,
        };
        let mut limits = Limits::default();
        if let Some(text) = env("SYNC_DAILY_BYTES_PER_ADDRESS") {
            limits.daily_bytes_per_address =
                text.trim().parse().map_err(|_| "SYNC_DAILY_BYTES_PER_ADDRESS must be a number of bytes".to_string())?;
        }
        if let Some(text) = env("SYNC_DAILY_BYTES_TOTAL") {
            limits.daily_bytes_total = text.trim().parse().map_err(|_| "SYNC_DAILY_BYTES_TOTAL must be a number of bytes".to_string())?;
        }
        if let Some(text) = env("SYNC_CONNECTIONS_PER_ADDRESS") {
            limits.connections_per_address =
                text.trim().parse().map_err(|_| "SYNC_CONNECTIONS_PER_ADDRESS must be a whole number".to_string())?;
        }
        Ok(Config {
            listen: SocketAddr::from(([0, 0, 0, 0], port)),
            database_url,
            retention_days,
            trust_proxy: env("SYNC_TRUST_PROXY").is_none_or(|v| v.trim() != "false"),
            limits,
            fcm_service_account: env("FCM_SERVICE_ACCOUNT").filter(|v| !v.trim().is_empty()),
            apns: apns_settings(&env)?,
            version: release_version(&env),
            relay: relay_url(&env)?,
            admin: admin_settings(&env)?,
            operator: env("SYNC_OPERATOR_NAME").map(|v| v.trim().chars().take(100).collect::<String>()).filter(|v| !v.is_empty()),
            abuse_contact: env("SYNC_ABUSE_CONTACT").filter(|v| !v.trim().is_empty()).map(|v| Contact::parse(&v)).transpose()?,
            smtp: smtp_settings(&env)?,
            server_key: server_key_seed(&env)?,
        })
    }

    pub fn summary(&self) -> String {
        format!(
            "port={} store={} retention={} trust_proxy={} wake={} reports={}",
            self.listen.port(),
            if self.database_url.is_some() { "postgres" } else { "memory" },
            if self.retention_days == 0 { "forever".to_string() } else { format!("{}d", self.retention_days) },
            self.trust_proxy,
            self.wake_summary(),
            self.reports_summary(),
        )
    }

    fn reports_summary(&self) -> &'static str {
        match (&self.abuse_contact, &self.smtp) {
            (Some(Contact::Email(_)), Some(_)) => "emailed",
            (Some(_), _) => "kept",
            (None, _) => "kept-no-contact",
        }
    }

    fn wake_summary(&self) -> &'static str {
        match (self.fcm_service_account.is_some(), self.apns.is_some()) {
            (true, true) => "fcm+apns",
            (true, false) => "fcm",
            (false, true) => "apns",
            (false, false) => "off",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config(pairs: &[(&str, &str)]) -> Result<Config, String> {
        let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Config::from_env(|key| map.get(key).cloned())
    }

    #[test]
    fn defaults_keep_ninety_days_in_memory_behind_a_proxy() {
        let c = config(&[]).unwrap();
        assert_eq!(c.listen.port(), DEFAULT_PORT);
        assert_eq!(c.database_url, None);
        assert_eq!(c.retention_days, 90);
        assert!(c.trust_proxy);
        assert_eq!(c.summary(), "port=3002 store=memory retention=90d trust_proxy=true wake=off reports=kept-no-contact");
    }

    #[test]
    fn a_server_key_seed_is_hex_and_kept_out_of_logs() {
        let seed = "ab".repeat(32);
        let c = config(&[("SYNC_SERVER_KEY", &seed)]).unwrap();
        assert_eq!(c.server_key, Some(KeySeed([0xab; 32])));
        assert!(!format!("{c:?}").contains("abab"));
        assert_eq!(config(&[]).unwrap().server_key, None);
        assert!(config(&[("SYNC_SERVER_KEY", "abc")]).is_err());
    }

    #[test]
    fn settings_are_read_and_checked() {
        let c = config(&[
            ("PORT", "4000"),
            ("DATABASE_URL", "postgres://u:p@db/commy_sync"),
            ("SYNC_RETENTION_DAYS", "0"),
            ("SYNC_TRUST_PROXY", "false"),
            ("SYNC_DAILY_BYTES_PER_ADDRESS", "1000"),
        ])
        .unwrap();
        assert_eq!(c.listen.port(), 4000);
        assert_eq!(c.database_url.as_deref(), Some("postgres://u:p@db/commy_sync"));
        assert_eq!(c.retention_days, 0);
        assert!(!c.trust_proxy);
        assert_eq!(c.limits.daily_bytes_per_address, 1000);
        assert!(c.summary().contains("retention=forever"));
        assert!(!c.summary().contains("u:p"));
        assert!(config(&[("PORT", "x")]).is_err());
        assert!(config(&[("DATABASE_URL", "mysql://x")]).is_err());
        assert!(config(&[("SYNC_RETENTION_DAYS", "-1")]).is_err());
        assert_eq!(config(&[("DATABASE_URL", " ")]).unwrap().database_url, None);
    }

    #[test]
    fn apple_push_needs_its_key_id_and_team() {
        assert!(config(&[("APNS_KEY", "k")]).err().unwrap().contains("APNS_KEY_ID"));
        assert!(config(&[("APNS_KEY", "k"), ("APNS_KEY_ID", "ABC")]).err().unwrap().contains("APNS_TEAM_ID"));
        let c = config(&[("APNS_KEY", "k"), ("APNS_KEY_ID", " ABC "), ("APNS_TEAM_ID", "TEAM")]).unwrap();
        assert_eq!(
            c.apns,
            Some(ApnsSettings { key: "k".into(), key_id: "ABC".into(), team_id: "TEAM".into(), topic: "chat.commy.mobile".into() })
        );
        assert!(c.summary().contains("wake=apns "));
        let both = config(&[("FCM_SERVICE_ACCOUNT", "{}"), ("APNS_KEY", "k"), ("APNS_KEY_ID", "A"), ("APNS_TEAM_ID", "T"), ("APNS_TOPIC", "x.y")]).unwrap();
        assert_eq!(both.apns.unwrap().topic, "x.y");
        assert!(config(&[("FCM_SERVICE_ACCOUNT", "{}"), ("APNS_KEY", "k"), ("APNS_KEY_ID", "A"), ("APNS_TEAM_ID", "T")]).unwrap().summary().contains("wake=fcm+apns "));
    }

    #[test]
    fn the_abuse_contact_is_an_email_or_an_https_page() {
        assert_eq!(Contact::parse(" abuse@example.org "), Ok(Contact::Email("abuse@example.org".into())));
        assert_eq!(Contact::parse("mailto:abuse@example.org"), Ok(Contact::Email("abuse@example.org".into())));
        assert_eq!(Contact::parse("https://example.org/abuse"), Ok(Contact::Page("https://example.org/abuse".into())));
        for bad in ["abuse", "@example.org", "abuse@localhost", "http://example.org", "a b@example.org", "<x>@example.org", "https://"] {
            assert!(Contact::parse(bad).is_err(), "{bad}");
        }
        let c = config(&[("SYNC_ABUSE_CONTACT", "abuse@example.org"), ("SYNC_OPERATOR_NAME", " Example Chat ")]).unwrap();
        assert_eq!(c.abuse_contact.as_ref().and_then(Contact::email), Some("abuse@example.org"));
        assert_eq!(c.operator.as_deref(), Some("Example Chat"));
        assert!(c.summary().ends_with("reports=kept"));
        assert!(config(&[("SYNC_ABUSE_CONTACT", "nope")]).is_err());
    }

    #[test]
    fn report_email_uses_the_shared_smtp_settings() {
        let c = config(&[("SYNC_ABUSE_CONTACT", "abuse@example.org"), ("SMTP_HOST", "smtp.example.org"), ("SMTP_USER", "bot@example.org"), ("SMTP_PASSWORD", "pw")]).unwrap();
        assert_eq!(
            c.smtp,
            Some(SmtpSettings::Host { host: "smtp.example.org".into(), port: 587, user: Some("bot@example.org".into()), password: Some("pw".into()), from: "bot@example.org".into() })
        );
        assert!(c.summary().ends_with("reports=emailed"));
        assert!(!c.summary().contains("pw"));
        let url = config(&[("SMTP_URL", "smtps://u:p@smtp.example.org"), ("EMAIL_FROM", "Commy <no-reply@example.org>")]).unwrap();
        assert_eq!(url.smtp.unwrap().from(), "Commy <no-reply@example.org>");
        assert!(config(&[("SMTP_HOST", "smtp.example.org")]).err().unwrap().contains("EMAIL_FROM"));
        assert!(config(&[("SMTP_HOST", "smtp.example.org"), ("SMTP_USER", "u"), ("SMTP_PORT", "x")]).is_err());
    }

    #[test]
    fn the_greeting_names_the_release_and_the_relay() {
        let c = config(&[("COMMY_VERSION", "v0.2.5"), ("SYNC_RELAY_URL", "https://sync.example.org/")]).unwrap();
        assert_eq!(c.version.as_deref(), Some("0.2.5"));
        assert_eq!(c.relay.as_deref(), Some("https://sync.example.org"));
        let plain = config(&[("SYNC_RELAY_URL", " ")]).unwrap();
        assert_eq!((plain.version, plain.relay), (None, None));
        assert!(config(&[("SYNC_RELAY_URL", "http://sync.example.org")]).is_err());
        assert!(config(&[("SYNC_RELAY_URL", "https://sync.example.org/relay")]).is_err());
    }

    #[test]
    fn the_admin_page_is_off_until_it_has_a_long_password() {
        assert_eq!(config(&[]).unwrap().admin, None);
        assert!(config(&[("SYNC_ADMIN_PASSWORD", "short")]).err().unwrap().contains("at least 16"));
        let c = config(&[("SYNC_ADMIN_PASSWORD", "0123456789abcdef"), ("SYNC_DOMAIN", "Sync.Example.org"), ("SYNC_ADMIN_PORT", "4000")]).unwrap(); // gitleaks:allow
        let admin = c.admin.clone().unwrap();
        assert_eq!(admin.listen.port(), 4000);
        assert_eq!(admin.domain.as_deref(), Some("sync.example.org"));
        assert_eq!(admin.caddy_admin, "http://caddy:2019");
        assert!(!c.summary().contains("0123456789abcdef"));
        assert!(!format!("{c:?}").contains("0123456789abcdef"), "the password never shows in logs");
    }
}
