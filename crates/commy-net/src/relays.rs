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

pub const ROOM_RELAY_WAIT: std::time::Duration = std::time::Duration::from_secs(10);
pub const ROOM_RELAY_RETRY: std::time::Duration = std::time::Duration::from_secs(5 * 60);

pub fn room_relay_url(text: &str) -> Result<RelayUrl> {
    let url = url::Url::parse(text.trim()).with_context(|| format!("{text} is not a relay address"))?;
    let plain = url.scheme() == "https"
        && url.host_str().is_some_and(|h| !h.is_empty())
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none();
    anyhow::ensure!(plain, "a community relay must be an https:// address with nothing after the domain");
    Ok(RelayUrl::from(url))
}

pub fn room_relay_config(url: RelayUrl) -> Arc<RelayConfig> {
    Arc::new(RelayConfig::new(url, Some(RelayQuicConfig::new(iroh_relay::defaults::DEFAULT_RELAY_QUIC_PORT))))
}

pub fn public_relays() -> Vec<Arc<RelayConfig>> {
    iroh::defaults::prod::default_relay_map().relays::<Vec<Arc<RelayConfig>>>()
}

#[derive(Clone)]
pub struct Relays {
    own: Arc<Vec<RelayUrl>>,
    rooms: Arc<std::sync::RwLock<Vec<RelayUrl>>>,
    set_aside: Arc<tokio::sync::Mutex<Vec<Arc<RelayConfig>>>>,
    failed: Arc<std::sync::Mutex<Option<(RelayUrl, n0_future::time::Instant)>>>,
}

impl Relays {
    pub fn new(own: Vec<RelayUrl>) -> Self {
        Self { own: Arc::new(own), rooms: Arc::default(), set_aside: Arc::default(), failed: Arc::default() }
    }

    pub fn room(&self) -> Option<RelayUrl> {
        self.rooms().into_iter().next()
    }

    fn rooms(&self) -> Vec<RelayUrl> {
        self.rooms.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set_rooms(&self, rooms: Vec<RelayUrl>) {
        *self.rooms.write().unwrap_or_else(|e| e.into_inner()) = rooms;
    }

    pub fn hints(&self) -> Vec<RelayUrl> {
        let rooms = self.rooms();
        let own = self.own.iter().filter(|url| !rooms.contains(url)).cloned().collect::<Vec<_>>();
        rooms.into_iter().chain(own).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.own.is_empty() && self.rooms().is_empty()
    }

    fn others(&self) -> Vec<RelayUrl> {
        let mut urls: Vec<RelayUrl> = self.own.iter().cloned().collect();
        urls.extend(iroh::defaults::prod::default_relay_map().urls::<Vec<RelayUrl>>());
        urls
    }

    pub async fn use_room(&self, endpoint: &iroh::Endpoint, room: Option<Arc<RelayConfig>>) -> Result<bool> {
        self.use_rooms(endpoint, room.into_iter().collect()).await
    }

    pub async fn use_rooms(&self, endpoint: &iroh::Endpoint, rooms: Vec<Arc<RelayConfig>>) -> Result<bool> {
        let mut set_aside = self.set_aside.lock().await;
        let current = self.rooms();
        let wanted: Vec<RelayUrl> = rooms.iter().map(|config| config.url.clone()).collect();
        if current == wanted {
            return Ok(false);
        }
        let Some(first) = wanted.first().cloned() else {
            put_back(endpoint, &mut set_aside, &current).await;
            self.set_rooms(Vec::new());
            return Ok(true);
        };
        if let Some((failed, at)) = self.failed.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            if *failed == first && at.elapsed() < ROOM_RELAY_RETRY {
                anyhow::bail!("the community's relay at {first} did not answer a moment ago");
            }
        }
        if current.is_empty() {
            set_aside.clear();
            for other in self.others().into_iter().filter(|other| !wanted.contains(other)) {
                if let Some(config) = endpoint.remove_relay(&other).await {
                    set_aside.push(config);
                }
            }
        }
        for room in rooms {
            let url = room.url.clone();
            if let Some(before) = endpoint.insert_relay(url.clone(), room).await {
                if current.is_empty() {
                    set_aside.push(before);
                }
            }
        }
        let mut left = Vec::new();
        for old in current.iter().filter(|old| !wanted.contains(old)) {
            if let Some(config) = endpoint.remove_relay(old).await {
                left.push(config);
            }
        }
        self.set_rooms(wanted.clone());
        if home_becomes(endpoint, &wanted, ROOM_RELAY_WAIT).await {
            return Ok(true);
        }
        *self.failed.lock().unwrap_or_else(|e| e.into_inner()) = Some((first.clone(), n0_future::time::Instant::now()));
        if current.is_empty() {
            put_back(endpoint, &mut set_aside, &wanted).await;
        } else {
            for url in wanted.iter().filter(|url| !current.contains(url)) {
                endpoint.remove_relay(url).await;
            }
            for config in left {
                endpoint.insert_relay(config.url.clone(), config).await;
            }
        }
        self.set_rooms(current);
        anyhow::bail!("the community's relay at {first} did not answer")
    }
}

async fn put_back(endpoint: &iroh::Endpoint, set_aside: &mut Vec<Arc<RelayConfig>>, rooms: &[RelayUrl]) {
    for room in rooms {
        endpoint.remove_relay(room).await;
    }
    for config in set_aside.drain(..) {
        endpoint.insert_relay(config.url.clone(), config).await;
    }
}

async fn home_becomes(endpoint: &iroh::Endpoint, urls: &[RelayUrl], wait: std::time::Duration) -> bool {
    use iroh::Watcher;
    use n0_future::StreamExt;
    let mut addrs = endpoint.watch_addr().stream();
    let home_is_one = |addr: &iroh::EndpointAddr| addr.relay_urls().next().is_some_and(|home| urls.contains(home));
    let reached = async {
        if home_is_one(&endpoint.addr()) {
            return true;
        }
        while let Some(addr) = addrs.next().await {
            if home_is_one(&addr) {
                return true;
            }
        }
        false
    };
    n0_future::time::timeout(wait, reached).await.unwrap_or(false)
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
    fn a_community_relay_is_a_bare_https_address() {
        assert_eq!(room_relay_url("https://sync.example.org").unwrap().to_string(), "https://sync.example.org/");
        assert!(room_relay_url(" https://sync.example.org/ ").is_ok());
        for bad in ["http://sync.example.org", "https://sync.example.org/relay", "https://u:p@sync.example.org", "https://sync.example.org/?x=1", "sync.example.org", ""] {
            assert!(room_relay_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_room_relay_is_hinted_first() {
        let own: RelayUrl = DEFAULT_COMMY_RELAY.parse().unwrap();
        let relays = Relays::new(vec![own.clone()]);
        assert_eq!(relays.hints(), vec![own.clone()]);
        let room = room_relay_url("https://sync.example.org").unwrap();
        relays.set_rooms(vec![room.clone()]);
        assert_eq!(relays.hints(), vec![room.clone(), own.clone()]);
        relays.set_rooms(vec![own.clone()]);
        assert_eq!(relays.hints(), vec![own], "a relay is hinted once");
        assert!(Relays::new(Vec::new()).is_empty());
    }

    async fn endpoint_on(relay: &Arc<RelayConfig>) -> iroh::Endpoint {
        let map = RelayMap::empty();
        map.insert(relay.url.clone(), relay.clone());
        iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .relay_mode(iroh::RelayMode::Custom(map))
            .ca_tls_config(iroh::tls::CaTlsConfig::insecure_skip_verify())
            .clear_ip_transports()
            .bind()
            .await
            .unwrap()
    }

    fn only(map: &RelayMap, url: &RelayUrl) -> Arc<RelayConfig> {
        map.get(url).unwrap()
    }

    #[tokio::test]
    async fn a_room_moves_the_home_relay_there_and_back() {
        let (ours_map, ours, _ours) = iroh::test_utils::run_relay_server().await.unwrap();
        let (room_map, room, _room) = iroh::test_utils::run_relay_server().await.unwrap();
        let endpoint = endpoint_on(&only(&ours_map, &ours)).await;
        assert!(home_becomes(&endpoint, std::slice::from_ref(&ours), ROOM_RELAY_WAIT).await);
        let relays = Relays::new(vec![ours.clone()]);

        assert!(relays.use_room(&endpoint, Some(only(&room_map, &room))).await.unwrap());
        assert_eq!(endpoint.addr().relay_urls().next(), Some(&room));
        assert_eq!(relays.hints(), vec![room.clone(), ours.clone()]);
        assert!(!relays.use_room(&endpoint, Some(only(&room_map, &room))).await.unwrap(), "already there");

        assert!(relays.use_room(&endpoint, None).await.unwrap());
        assert!(home_becomes(&endpoint, std::slice::from_ref(&ours), ROOM_RELAY_WAIT).await);
        assert_eq!(relays.hints(), vec![ours.clone()]);

        let unreachable = room_relay_config(room_relay_url("https://127.0.0.1:9").unwrap());
        let refused = relays.use_room(&endpoint, Some(unreachable)).await.unwrap_err();
        assert!(refused.to_string().contains("did not answer"));
        assert_eq!(relays.room(), None);
        assert!(home_becomes(&endpoint, std::slice::from_ref(&ours), ROOM_RELAY_WAIT).await, "our relay comes back after a failed switch");
        let unreachable = room_relay_config(room_relay_url("https://127.0.0.1:9").unwrap());
        let started = std::time::Instant::now();
        assert!(relays.use_room(&endpoint, Some(unreachable)).await.unwrap_err().to_string().contains("a moment ago"));
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "a relay that just failed is not waited on again");
        assert_eq!(endpoint.addr().relay_urls().next(), Some(&ours));
        endpoint.close().await;
    }

    #[tokio::test]
    async fn a_set_of_public_relays_stands_in_and_a_failed_room_keeps_it() {
        let (ours_map, ours, _ours) = iroh::test_utils::run_relay_server().await.unwrap();
        let (one_map, one, _one) = iroh::test_utils::run_relay_server().await.unwrap();
        let (two_map, two, _two) = iroh::test_utils::run_relay_server().await.unwrap();
        let endpoint = endpoint_on(&only(&ours_map, &ours)).await;
        assert!(home_becomes(&endpoint, std::slice::from_ref(&ours), ROOM_RELAY_WAIT).await);
        let relays = Relays::new(vec![ours.clone()]);
        let public = vec![only(&one_map, &one), only(&two_map, &two)];

        assert!(relays.use_rooms(&endpoint, public.clone()).await.unwrap());
        assert!(home_becomes(&endpoint, &[one.clone(), two.clone()], ROOM_RELAY_WAIT).await);
        assert_eq!(relays.hints(), vec![one.clone(), two.clone(), ours.clone()]);
        assert!(!relays.use_rooms(&endpoint, public.clone()).await.unwrap(), "already there");

        let unreachable = room_relay_config(room_relay_url("https://127.0.0.1:9").unwrap());
        assert!(relays.use_room(&endpoint, Some(unreachable)).await.is_err());
        assert_eq!(relays.hints(), vec![one.clone(), two.clone(), ours.clone()], "a room that does not answer leaves the public relays in place");
        assert!(home_becomes(&endpoint, &[one.clone(), two.clone()], ROOM_RELAY_WAIT).await);

        assert!(relays.use_room(&endpoint, None).await.unwrap());
        assert!(home_becomes(&endpoint, std::slice::from_ref(&ours), ROOM_RELAY_WAIT).await);
        assert_eq!(relays.hints(), vec![ours]);
        endpoint.close().await;
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
