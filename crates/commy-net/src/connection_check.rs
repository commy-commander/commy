use iroh::unstable_net_report::NetReport;
use iroh::{Endpoint, RelayUrl, Watcher};
use serde::Serialize;
use std::time::Duration;

pub const REPORT_WAIT: Duration = Duration::from_secs(10);
pub const GOOD_RELAY_RTT: Duration = Duration::from_millis(120);
pub const FAIR_RELAY_RTT: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectPath {
    Open,
    Limited,
    Blocked,
    Unchecked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Good,
    Fair,
    Poor,
    Offline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Observed {
    pub relay_connected: bool,
    pub can_discover_address: bool,
    pub udp: bool,
    pub ipv6: bool,
    pub address_varies: Option<bool>,
    pub relay_rtt: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConnectionCheck {
    pub quality: Quality,
    pub direct: DirectPath,
    pub relay_reachable: bool,
    pub relay_rtt_ms: Option<u32>,
    pub ipv6: bool,
}

impl Observed {
    pub fn from_report(
        report: Option<&NetReport>,
        home_relay: Option<&RelayUrl>,
        relay_connected: bool,
        can_discover_address: bool,
    ) -> Self {
        let Some(report) = report else {
            return Self { relay_connected, can_discover_address, ..Self::default() };
        };
        let measured_relay = home_relay.or(report.preferred_relay.as_ref());
        let relay_rtt = report
            .relay_latency
            .iter()
            .filter(|(_, url, _)| measured_relay.is_none_or(|wanted| *url == wanted))
            .map(|(_, _, rtt)| rtt)
            .min();
        Self {
            relay_connected,
            can_discover_address,
            udp: report.has_udp(),
            ipv6: report.udp_v6,
            address_varies: report.mapping_varies_by_dest(),
            relay_rtt,
        }
    }

    pub fn direct(&self) -> DirectPath {
        match (self.udp, self.address_varies) {
            (true, Some(true)) => DirectPath::Limited,
            (true, _) => DirectPath::Open,
            (false, _) if self.can_discover_address && self.relay_connected => DirectPath::Blocked,
            (false, _) => DirectPath::Unchecked,
        }
    }

    pub fn quality(&self) -> Quality {
        if !self.relay_connected && !self.udp {
            return Quality::Offline;
        }
        let by_rtt = match self.relay_rtt {
            Some(rtt) if rtt <= GOOD_RELAY_RTT => Quality::Good,
            Some(rtt) if rtt <= FAIR_RELAY_RTT => Quality::Fair,
            Some(_) => Quality::Poor,
            None => Quality::Fair,
        };
        match self.direct() {
            DirectPath::Limited | DirectPath::Blocked => by_rtt.max(Quality::Fair),
            DirectPath::Open | DirectPath::Unchecked => by_rtt,
        }
    }

    pub fn check(&self) -> ConnectionCheck {
        ConnectionCheck {
            quality: self.quality(),
            direct: self.direct(),
            relay_reachable: self.relay_connected,
            relay_rtt_ms: self.relay_rtt.map(|rtt| u32::try_from(rtt.as_millis()).unwrap_or(u32::MAX)),
            ipv6: self.ipv6,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConnectionChecker {
    pub(crate) endpoint: Endpoint,
    pub(crate) can_discover_address: bool,
}

impl ConnectionChecker {
    pub async fn run(self) -> ConnectionCheck {
        let mut reports = self.endpoint.net_report();
        let report = match reports.get() {
            Some(report) => Some(report),
            None => n0_future::time::timeout(REPORT_WAIT, reports.initialized()).await.ok(),
        };
        let relays = self.endpoint.home_relay_status().get();
        let connected = relays.iter().find(|status| status.is_connected());
        let home = connected.or(relays.first()).map(|status| status.url().clone());
        Observed::from_report(report.as_ref(), home.as_ref(), connected.is_some(), self.can_discover_address).check()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(udp: bool, varies: Option<bool>, rtt_ms: Option<u64>) -> Observed {
        Observed {
            relay_connected: true,
            can_discover_address: true,
            udp,
            ipv6: false,
            address_varies: varies,
            relay_rtt: rtt_ms.map(Duration::from_millis),
        }
    }

    #[test]
    fn open_network_with_a_quick_relay_is_good() {
        let observed = seen(true, Some(false), Some(40));
        assert_eq!(observed.direct(), DirectPath::Open);
        assert_eq!(observed.quality(), Quality::Good);
    }

    #[test]
    fn a_single_address_check_still_counts_as_open() {
        assert_eq!(seen(true, None, Some(40)).direct(), DirectPath::Open);
    }

    #[test]
    fn relay_only_networks_are_at_best_fair() {
        let blocked = seen(false, None, Some(30));
        assert_eq!(blocked.direct(), DirectPath::Blocked);
        assert_eq!(blocked.quality(), Quality::Fair);
        let limited = seen(true, Some(true), Some(30));
        assert_eq!(limited.direct(), DirectPath::Limited);
        assert_eq!(limited.quality(), Quality::Fair);
        assert_eq!(seen(false, None, Some(400)).quality(), Quality::Poor);
    }

    #[test]
    fn relay_round_trip_sets_the_grade() {
        assert_eq!(seen(true, None, Some(120)).quality(), Quality::Good);
        assert_eq!(seen(true, None, Some(121)).quality(), Quality::Fair);
        assert_eq!(seen(true, None, Some(250)).quality(), Quality::Fair);
        assert_eq!(seen(true, None, Some(251)).quality(), Quality::Poor);
        assert_eq!(seen(true, None, None).quality(), Quality::Fair);
    }

    #[test]
    fn without_address_discovery_the_direct_path_is_unchecked() {
        let observed = Observed { can_discover_address: false, ..seen(false, None, Some(50)) };
        assert_eq!(observed.direct(), DirectPath::Unchecked);
        assert_eq!(observed.quality(), Quality::Good);
    }

    #[test]
    fn nothing_reachable_is_offline() {
        let observed = Observed { relay_connected: false, ..seen(false, None, None) };
        assert_eq!(observed.quality(), Quality::Offline);
        assert_eq!(observed.direct(), DirectPath::Unchecked);
        assert!(!observed.check().relay_reachable);
    }

    #[test]
    fn no_report_yet_keeps_what_the_relay_says() {
        let observed = Observed::from_report(None, None, true, true);
        assert_eq!(observed.check().relay_rtt_ms, None);
        assert_eq!(observed.direct(), DirectPath::Blocked);
    }

    #[test]
    fn reports_read_the_home_relay_round_trip() {
        let mut report = NetReport::default();
        report.udp_v4 = true;
        let observed = Observed::from_report(Some(&report), None, true, true);
        assert!(observed.udp);
        assert_eq!(observed.relay_rtt, None);
        assert_eq!(observed.direct(), DirectPath::Open);
    }

    #[test]
    fn checks_serialize_in_plain_words() {
        let json = serde_json::to_value(seen(false, None, Some(80)).check()).unwrap();
        assert_eq!(json["quality"], "fair");
        assert_eq!(json["direct"], "blocked");
        assert_eq!(json["relay_rtt_ms"], 80);
        assert_eq!(json["relay_reachable"], true);
    }

    #[tokio::test]
    async fn a_node_without_relays_reports_offline() {
        let (node, _, _, _) = crate::CommyNode::spawn_with(None, Some(iroh::RelayMap::empty())).await.unwrap();
        let checker = node.connection_checker();
        assert!(!checker.can_discover_address);
        let check = tokio::time::timeout(REPORT_WAIT * 2, checker.run()).await.unwrap();
        assert_eq!(check.quality, Quality::Offline);
        assert!(!check.relay_reachable);
        node.shutdown().await.ok();
    }
}
