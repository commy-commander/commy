//! Relay servers the node uses: Commy's own relay first, with n0's public
//! relays kept as a fallback so connections survive if ours is down or
//! paused at its monthly bandwidth limit.

use anyhow::{Context, Result};
use iroh::net::relay::{RelayMap, RelayNode, RelayUrl};

pub const DEFAULT_COMMY_RELAY: &str = "https://relay.commy.chat:8443";

/// Our relay URLs: `COMMY_RELAY_URLS` (comma separated, `none` to use only
/// n0's), else the default Commy relay.
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

/// Own relays followed by n0's production relays, without duplicates.
pub fn relay_map(own: &[String]) -> Result<RelayMap> {
    let mut nodes: Vec<RelayNode> = Vec::new();
    for url in own {
        let url: RelayUrl = url.parse().with_context(|| format!("invalid relay url {url}"))?;
        nodes.push(RelayNode { url, stun_only: false, stun_port: 0 });
    }
    for node in iroh::net::defaults::prod::default_relay_map().nodes() {
        if !nodes.iter().any(|n| n.url == node.url) {
            nodes.push((**node).clone());
        }
    }
    RelayMap::from_nodes(nodes)
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
    fn env_overrides_and_filters() {
        assert!(own_relays_from(Some("none")).is_empty());
        assert_eq!(
            own_relays_from(Some("https://a.example/, http://evil.example ,http://localhost:3340")),
            vec!["https://a.example".to_string(), "http://localhost:3340".to_string()]
        );
    }

    #[test]
    fn own_relays_come_first_with_n0_fallback() {
        let n0 = iroh::net::defaults::prod::default_relay_map();
        let map = relay_map(&[DEFAULT_COMMY_RELAY.to_string()]).unwrap();
        assert_eq!(map.len(), n0.len() + 1);
        assert!(map.contains_node(&DEFAULT_COMMY_RELAY.parse().unwrap()));
        for node in n0.nodes() {
            assert!(map.contains_node(&node.url));
        }
    }

    #[test]
    fn duplicates_and_bad_urls() {
        let n0_url = iroh::net::defaults::prod::default_relay_map().nodes().next().unwrap().url.to_string();
        let n0_len = iroh::net::defaults::prod::default_relay_map().len();
        assert_eq!(relay_map(&[n0_url]).unwrap().len(), n0_len);
        assert!(relay_map(&["not a url".to_string()]).is_err());
        assert_eq!(relay_map(&[]).unwrap().len(), n0_len);
    }
}
