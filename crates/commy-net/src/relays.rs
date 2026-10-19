use anyhow::{Context, Result};
use iroh::{RelayConfig, RelayMap, RelayUrl};
use std::sync::Arc;

pub const DEFAULT_COMMY_RELAY: &str = match option_env!("COMMY_DEFAULT_RELAY") {
    Some(url) => url,
    None => "https://relay.commy.chat:8443",
};

pub fn own_relays_from(var: Option<&str>) -> Vec<String> {
    match var.map(str::trim) {
        Some("none") => Vec::new(),
        Some(list) if !list.is_empty() => list
            .split(',')
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| s.starts_with("https://") || s.starts_with("http://localhost") || s.starts_with("http://127.0.0.1"))
            .collect(),
        _ => vec![DEFAULT_COMMY_RELAY.to_string()],
    }
}

pub fn own_relays() -> Vec<String> {
    own_relays_from(std::env::var("COMMY_RELAY_URLS").ok().as_deref())
}

pub fn relay_map(own: &[String]) -> Result<RelayMap> {
    let map = RelayMap::empty();
    for url in own {
        let url: RelayUrl = url.parse().with_context(|| format!("invalid relay url {url}"))?;
        map.insert(url.clone(), Arc::new(RelayConfig::new(url, None)));
    }
    for node in iroh::defaults::prod::default_relay_map().relays::<Vec<Arc<RelayConfig>>>() {
        if !map.contains(&node.url) {
            map.insert(node.url.clone(), node);
        }
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_the_commy_relay() {
        assert_eq!(own_relays_from(None), vec![DEFAULT_COMMY_RELAY.to_string()]);
        assert_eq!(own_relays_from(Some("  ")), vec![DEFAULT_COMMY_RELAY.to_string()]);
    }

    #[test]
    fn our_relay_has_no_address_discovery_port() {
        let map = relay_map(&[DEFAULT_COMMY_RELAY.to_string(), "https://other.example".to_string()]).unwrap();
        for url in [DEFAULT_COMMY_RELAY, "https://other.example"] {
            assert!(map.get(&url.parse().unwrap()).is_some_and(|r| r.quic.is_none()));
        }
    }

    #[test]
    fn env_overrides_and_filters() {
        assert!(own_relays_from(Some("none")).is_empty());
        assert_eq!(
            own_relays_from(Some("https://a.example/, http://evil.example ,http://localhost:3340")),
            vec!["https://a.example".to_string(), "http://localhost:3340".to_string()]
        );
    }

    #[test]
    fn own_relays_come_first_with_n0_fallback() {
        let n0 = iroh::defaults::prod::default_relay_map();
        let map = relay_map(&[DEFAULT_COMMY_RELAY.to_string()]).unwrap();
        assert_eq!(map.len(), n0.len() + 1);
        assert!(map.contains(&DEFAULT_COMMY_RELAY.parse().unwrap()));
        for url in n0.urls::<Vec<RelayUrl>>() {
            assert!(map.contains(&url));
        }
    }

    #[test]
    fn duplicates_and_bad_urls() {
        let n0_url = iroh::defaults::prod::default_relay_map().urls::<Vec<RelayUrl>>()[0].to_string();
        let n0_len = iroh::defaults::prod::default_relay_map().len();
        assert_eq!(relay_map(&[n0_url]).unwrap().len(), n0_len);
        assert!(relay_map(&["not a url".to_string()]).is_err());
        assert_eq!(relay_map(&[]).unwrap().len(), n0_len);
    }
}
