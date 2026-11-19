use std::sync::Arc;

use iroh::{RelayConfig, RelayMap, RelayUrl};

pub fn relay_map_of(relays: &[String]) -> Result<RelayMap, String> {
    let map = RelayMap::empty();
    for url in relays {
        let url: RelayUrl = url.parse().map_err(|_| format!("invalid relay {url}"))?;
        map.insert(url.clone(), Arc::new(RelayConfig::new(url, None)));
    }
    Ok(map)
}

pub fn public_fallback_for(own: &RelayMap) -> Vec<Arc<RelayConfig>> {
    if own.is_empty() { Vec::new() } else { commy_net::relays::fallback_relays(own) }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn fallback_urls(own: &[String]) -> HashSet<RelayUrl> {
        public_fallback_for(&relay_map_of(own).unwrap()).into_iter().map(|config| config.url.clone()).collect()
    }

    #[test]
    fn a_browser_turned_away_by_its_relay_can_move_to_the_public_relays() {
        let public: HashSet<RelayUrl> = commy_net::relays::public_relay_urls().into_iter().collect();
        assert_eq!(fallback_urls(&["https://relay.commy.chat:8443".into()]), public);
    }

    #[test]
    fn a_page_set_to_use_no_relays_is_left_without_any() {
        assert!(fallback_urls(&[]).is_empty());
    }

    #[test]
    fn a_public_relay_the_page_already_uses_is_not_added_twice() {
        let first = commy_net::relays::public_relay_urls()[0].to_string();
        assert!(!fallback_urls(&[first.trim_end_matches('/').into()]).iter().any(|url| url.to_string() == first));
    }

    #[test]
    fn a_relay_address_that_does_not_parse_is_refused() {
        assert!(relay_map_of(&["not a relay".into()]).is_err());
    }
}
