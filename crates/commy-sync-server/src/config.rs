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
        })
    }

    pub fn summary(&self) -> String {
        format!(
            "port={} store={} retention={} trust_proxy={}",
            self.listen.port(),
            if self.database_url.is_some() { "postgres" } else { "memory" },
            if self.retention_days == 0 { "forever".to_string() } else { format!("{}d", self.retention_days) },
            self.trust_proxy,
        )
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
        assert_eq!(c.summary(), "port=3002 store=memory retention=90d trust_proxy=true");
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
}
