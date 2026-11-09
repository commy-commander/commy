use std::net::SocketAddr;

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
        })
    }

    pub fn summary(&self) -> String {
        format!(
            "port={} store={} retention={} trust_proxy={} wake={}",
            self.listen.port(),
            if self.database_url.is_some() { "postgres" } else { "memory" },
            if self.retention_days == 0 { "forever".to_string() } else { format!("{}d", self.retention_days) },
            self.trust_proxy,
            self.wake_summary(),
        )
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
        assert_eq!(c.summary(), "port=3002 store=memory retention=90d trust_proxy=true wake=off");
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
        assert!(c.summary().ends_with("wake=apns"));
        let both = config(&[("FCM_SERVICE_ACCOUNT", "{}"), ("APNS_KEY", "k"), ("APNS_KEY_ID", "A"), ("APNS_TEAM_ID", "T"), ("APNS_TOPIC", "x.y")]).unwrap();
        assert_eq!(both.apns.unwrap().topic, "x.y");
        assert!(config(&[("FCM_SERVICE_ACCOUNT", "{}"), ("APNS_KEY", "k"), ("APNS_KEY_ID", "A"), ("APNS_TEAM_ID", "T")]).unwrap().summary().ends_with("wake=fcm+apns"));
    }
}
