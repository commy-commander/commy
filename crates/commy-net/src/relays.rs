use anyhow::{Context, Result};
use iroh::net::relay::{RelayMap, RelayNode, RelayUrl};

pub const DEFAULT_COMMY_RELAY: &str = match option_env!("COMMY_DEFAULT_RELAY") {
    Some(url) => url,
    None => "https://relay.commy.chat:8443",
};

pub const DEFAULT_COMMY_RELAY_STUN_PORT: u16 = match option_env!("COMMY_DEFAULT_RELAY_STUN_PORT") {
    Some(port) => parse_port(port),
    None => USE_IROH_DEFAULT_STUN_PORT,
};

const USE_IROH_DEFAULT_STUN_PORT: u16 = 0;

const fn parse_port(s: &str) -> u16 {
    let bytes = s.as_bytes();
    assert!(!bytes.is_empty() && bytes.len() <= 5, "COMMY_DEFAULT_RELAY_STUN_PORT must be a port number");
    let mut value: u32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        assert!(bytes[i].is_ascii_digit(), "COMMY_DEFAULT_RELAY_STUN_PORT must be a port number");
        value = value * 10 + (bytes[i] - b'0') as u32;
        i += 1;
    }
    assert!(value <= 65535, "COMMY_DEFAULT_RELAY_STUN_PORT must be a port number");
    value as u16
}

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
    let mut nodes: Vec<RelayNode> = Vec::new();
    for url in own {
        let url: RelayUrl = url.parse().with_context(|| format!("invalid relay url {url}"))?;
        let is_default = DEFAULT_COMMY_RELAY.parse::<RelayUrl>().is_ok_and(|d| d == url);
        let stun_port = if is_default { DEFAULT_COMMY_RELAY_STUN_PORT } else { USE_IROH_DEFAULT_STUN_PORT };
        nodes.push(RelayNode { url, stun_only: false, stun_port });
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
    fn parses_stun_ports() {
        assert_eq!(parse_port("3479"), 3479);
        assert_eq!(parse_port("65535"), 65535);
    }

    #[test]
    fn default_relay_gets_its_stun_port() {
        let map = relay_map(&[DEFAULT_COMMY_RELAY.to_string(), "https://other.example".to_string()]).unwrap();
        let port = |url: &str| {
            let url: RelayUrl = url.parse().unwrap();
            map.nodes().find(|n| n.url == url).map(|n| n.stun_port)
        };
        assert_eq!(port(DEFAULT_COMMY_RELAY), Some(DEFAULT_COMMY_RELAY_STUN_PORT));
        assert_eq!(port("https://other.example"), Some(0));
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
