use anyhow::{Context, Result};
use iroh::{RelayConfig, RelayMap, RelayUrl};
use iroh_relay::RelayQuicConfig;
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

pub fn address_discovery_port_from(var: Option<&str>, built: Option<&str>) -> u16 {
    [var, built]
        .into_iter()
        .flatten()
        .find_map(|port| port.trim().parse::<u16>().ok().filter(|p| *p != 0))
        .unwrap_or(iroh_relay::defaults::DEFAULT_RELAY_QUIC_PORT)
}

pub fn address_discovery_port() -> u16 {
    address_discovery_port_from(
        std::env::var("COMMY_RELAY_QAD_PORT").ok().as_deref(),
        option_env!("COMMY_RELAY_QAD_PORT"),
    )
}

pub const FALLBACK_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

pub fn relay_map(own: &[String]) -> Result<RelayMap> {
    relay_map_with(own, address_discovery_port())
}

pub fn relay_map_with(own: &[String], address_discovery_port: u16) -> Result<RelayMap> {
    let map = RelayMap::empty();
    for url in own {
        let url: RelayUrl = url.parse().with_context(|| format!("invalid relay url {url}"))?;
        let quic = RelayQuicConfig::new(address_discovery_port);
        map.insert(url.clone(), Arc::new(RelayConfig::new(url, Some(quic))));
    }
    if map.is_empty() {
        return Ok(iroh::defaults::prod::default_relay_map());
    }
    Ok(map)
}

pub fn fallback_relays(own: &RelayMap) -> Vec<Arc<RelayConfig>> {
    iroh::defaults::prod::default_relay_map()
        .relays::<Vec<Arc<RelayConfig>>>()
        .into_iter()
        .filter(|node| !own.contains(&node.url))
        .collect()
}

pub async fn fall_back_when_unreachable(endpoint: iroh::Endpoint, fallback: Vec<Arc<RelayConfig>>) {
    loop {
        n0_future::time::sleep(FALLBACK_AFTER).await;
        if endpoint.is_closed() {
            return;
        }
        if endpoint.addr().relay_urls().next().is_some() {
            continue;
        }
        tracing::warn!("Commy's relay is out of reach, adding the public fallback relays");
        for node in fallback {
            endpoint.insert_relay(node.url.clone(), node).await;
        }
        return;
    }
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
    fn our_relays_tell_apps_their_public_address() {
        let map = relay_map_with(&[DEFAULT_COMMY_RELAY.to_string(), "https://other.example".to_string()], 7843).unwrap();
        for url in [DEFAULT_COMMY_RELAY, "https://other.example"] {
            let relay = map.get(&url.parse().unwrap()).unwrap();
            assert_eq!(relay.quic.as_ref().map(|q| q.port), Some(7843));
        }
    }

    #[test]
    fn address_discovery_port_prefers_the_environment_then_the_build() {
        assert_eq!(address_discovery_port_from(None, None), 7842);
        assert_eq!(address_discovery_port_from(None, Some("7843")), 7843);
        assert_eq!(address_discovery_port_from(Some(" 7844 "), Some("7843")), 7844);
        assert_eq!(address_discovery_port_from(Some("nope"), Some("0")), 7842);
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
    fn only_commy_relays_until_they_fail() {
        let n0 = iroh::defaults::prod::default_relay_map();
        let map = relay_map(&[DEFAULT_COMMY_RELAY.to_string()]).unwrap();
        assert_eq!(map.urls::<Vec<RelayUrl>>(), vec![DEFAULT_COMMY_RELAY.parse::<RelayUrl>().unwrap()]);
        let fallback = fallback_relays(&map);
        assert_eq!(fallback.len(), n0.len());
        assert!(fallback.iter().all(|node| n0.contains(&node.url)));
        assert_eq!(relay_map(&[]).unwrap().len(), n0.len());
    }

    #[test]
    fn duplicates_and_bad_urls() {
        let n0_url = iroh::defaults::prod::default_relay_map().urls::<Vec<RelayUrl>>()[0].to_string();
        let map = relay_map(&[n0_url.clone(), n0_url]).unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(fallback_relays(&map).len(), iroh::defaults::prod::default_relay_map().len() - 1);
        assert!(relay_map(&["not a url".to_string()]).is_err());
    }
}
