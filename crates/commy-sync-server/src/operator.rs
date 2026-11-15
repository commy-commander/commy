use crate::ramp::{ServerRecord, Tier};
use crate::reports::{describe, ReportState};
use crate::server_proof::clean_origin;
use crate::store::{Store, StoreError};

pub const USAGE: &str = "usage: commy-sync [check <wss://domain/sync> | reports [--all] | show <report> | drop <report> | dismiss <report> | gateway | gateway cut <https://server> <reason> | gateway restore <https://server> | gateway tier <https://server> <new|settled|trusted|auto>]";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    List { all: bool },
    Show(String),
    Drop(String),
    Dismiss(String),
    Gateway,
    GatewayCut { origin: String, reason: String },
    GatewayRestore(String),
    GatewayTier { origin: String, tier: Option<Tier> },
}

impl Command {
    pub fn parse(words: &[String]) -> Result<Self, &'static str> {
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        match words.as_slice() {
            ["reports"] => Ok(Command::List { all: false }),
            ["reports", "--all"] => Ok(Command::List { all: true }),
            ["show", id] => Ok(Command::Show(id.to_string())),
            ["drop", id] => Ok(Command::Drop(id.to_string())),
            ["dismiss", id] => Ok(Command::Dismiss(id.to_string())),
            ["gateway"] => Ok(Command::Gateway),
            ["gateway", "cut", origin, reason @ ..] if !reason.is_empty() => Ok(Command::GatewayCut { origin: origin_of(origin)?, reason: reason.join(" ") }),
            ["gateway", "restore", origin] => Ok(Command::GatewayRestore(origin_of(origin)?)),
            ["gateway", "tier", origin, "auto"] => Ok(Command::GatewayTier { origin: origin_of(origin)?, tier: None }),
            ["gateway", "tier", origin, tier] => Ok(Command::GatewayTier { origin: origin_of(origin)?, tier: Some(Tier::parse(tier).ok_or(USAGE)?) }),
            _ => Err(USAGE),
        }
    }
}

pub(crate) fn origin_of(text: &str) -> Result<String, &'static str> {
    clean_origin(text).ok_or(USAGE)
}

async fn gateway_list(store: &Store) -> Result<String, StoreError> {
    let now = crate::now_ms();
    let day = commy_sync::wake::wake_day(now);
    let servers = store.gateway_servers().await?;
    if servers.is_empty() {
        return Ok("No servers have used the push gateway.\n".into());
    }
    let mut out = String::new();
    for record in servers {
        let today = store.gateway_day(&record.origin, day).await?;
        let tier = record.tier(now);
        let caps = tier.caps();
        let state = match &record.cut_off {
            Some(cut) => format!("CUT OFF ({}{})", cut.reason, if cut.automatic { ", automatic" } else { "" }),
            None => "ok".to_string(),
        };
        let pinned = if record.pinned.is_some() { " (pinned)" } else { "" };
        out.push_str(&format!(
            "{}  {}{pinned}  today {}/{} pings, {}/{} new phones, {} refused  {state}\n",
            record.origin,
            tier.as_str(),
            today.pushes,
            caps.pushes,
            today.new_phones,
            caps.new_phones,
            today.refused
        ));
    }
    Ok(out)
}

async fn gateway_change(store: &Store, origin: &str, change: impl FnOnce(&mut ServerRecord, i64)) -> Result<ServerRecord, StoreError> {
    let now = crate::now_ms();
    let mut record = store.gateway_server(origin).await?.unwrap_or_else(|| ServerRecord::first(origin, now));
    change(&mut record, now);
    store.put_gateway_server(&record).await?;
    Ok(record)
}

pub async fn run(store: &Store, command: Command) -> Result<String, StoreError> {
    Ok(match command {
        Command::Gateway => gateway_list(store).await?,
        Command::GatewayCut { origin, reason } => {
            gateway_change(store, &origin, |record, now| record.cut(now, &reason, false)).await?;
            format!("{origin} is cut off from the push gateway. Its pings are refused from now on.\n")
        }
        Command::GatewayRestore(origin) => {
            gateway_change(store, &origin, |record, now| record.restore(now)).await?;
            format!("{origin} may use the push gateway again, starting over at the new server caps.\n")
        }
        Command::GatewayTier { origin, tier } => {
            let record = gateway_change(store, &origin, |record, _| record.pinned = tier).await?;
            match tier {
                Some(tier) => format!("{origin} is pinned at {} ({} pings a day).\n", tier.as_str(), tier.caps().pushes),
                None => format!("{origin} follows the trust ramp again ({} now).\n", record.tier(crate::now_ms()).as_str()),
            }
        }
        Command::List { all } => {
            let reports = store.reports(!all).await?;
            if reports.is_empty() {
                return Ok(if all { "No reports.\n".into() } else { "No open reports.\n".into() });
            }
            reports
                .iter()
                .map(|r| format!("{}  {:<9}  {:<13}  {}  ({} messages)\n", r.id, r.state.as_str(), r.report.reason, r.report.community_name, r.report.messages.len()))
                .collect()
        }
        Command::Show(id) => match store.report(&id).await? {
            Some(stored) => format!("{}\n", describe(&stored)),
            None => format!("No report {id}.\n"),
        },
        Command::Dismiss(id) => match store.settle_report(&id, ReportState::Dismissed).await? {
            true => format!("Report {id} dismissed.\n"),
            false => format!("No report {id}.\n"),
        },
        Command::Drop(id) => {
            let Some(stored) = store.report(&id).await? else { return Ok(format!("No report {id}.\n")) };
            let streams = &stored.report.streams;
            let removed = store.drop_streams(streams, &id, crate::now_ms()).await?;
            store.settle_report(&id, ReportState::Dropped).await?;
            format!(
                "Removed community {} ({}): {} streams refused from now on, {removed} stored entries deleted. Running services stop serving it within a minute.\n",
                stored.report.community_name,
                stored.report.community,
                streams.len()
            )
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reports::{new_report_id, StoredReport};
    use commy_sync::report::{HostReport, Person};
    use commy_sync::StreamSecret;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn commands_are_parsed_strictly() {
        assert_eq!(Command::parse(&words("reports")), Ok(Command::List { all: false }));
        assert_eq!(Command::parse(&words("reports --all")), Ok(Command::List { all: true }));
        assert_eq!(Command::parse(&words("drop abc")), Ok(Command::Drop("abc".into())));
        assert_eq!(Command::parse(&words("show abc")), Ok(Command::Show("abc".into())));
        assert_eq!(Command::parse(&words("dismiss abc")), Ok(Command::Dismiss("abc".into())));
        assert_eq!(Command::parse(&words("gateway")), Ok(Command::Gateway));
        assert_eq!(
            Command::parse(&words("gateway cut https://Bad.Example.org sent spam")),
            Ok(Command::GatewayCut { origin: "https://bad.example.org".into(), reason: "sent spam".into() })
        );
        assert_eq!(Command::parse(&words("gateway restore https://bad.example.org")), Ok(Command::GatewayRestore("https://bad.example.org".into())));
        assert_eq!(
            Command::parse(&words("gateway tier https://bad.example.org trusted")),
            Ok(Command::GatewayTier { origin: "https://bad.example.org".into(), tier: Some(Tier::Trusted) })
        );
        assert_eq!(Command::parse(&words("gateway tier https://bad.example.org auto")), Ok(Command::GatewayTier { origin: "https://bad.example.org".into(), tier: None }));
        for bad in ["drop", "drop a b", "erase abc", "reports --everything", "gateway cut https://bad.example.org", "gateway cut http://bad.example.org spam", "gateway tier https://bad.example.org gold"] {
            assert_eq!(Command::parse(&words(bad)), Err(USAGE), "{bad}");
        }
    }

    #[tokio::test]
    async fn an_operator_lists_drops_and_dismisses_reports() {
        let store = Store::memory();
        assert_eq!(run(&store, Command::List { all: false }).await.unwrap(), "No open reports.\n");
        let stream = StreamSecret::random().keys().id;
        store.append(&stream, &[2; 16], b"x", 1).await.unwrap();
        let person = |id: &str| Person { id: id.into(), name: id.into() };
        let report = HostReport {
            community: "c9".into(),
            community_name: "Loud Place".into(),
            owner: person("o"),
            reporter: person("r"),
            reported: person("o"),
            reason: "inappropriate".into(),
            note: String::new(),
            messages: Vec::new(),
            streams: vec![stream],
            sent_at: "now".into(),
        };
        let (first, second) = (new_report_id(), new_report_id());
        for id in [&first, &second] {
            store.add_report(&StoredReport { id: id.clone(), received: 1, state: ReportState::Open, report: report.clone() }).await.unwrap();
        }
        let listed = run(&store, Command::List { all: false }).await.unwrap();
        assert!(listed.contains(&first) && listed.contains("Loud Place"));
        assert!(run(&store, Command::Show(first.clone())).await.unwrap().contains("Reported: o (o)"));
        let dropped = run(&store, Command::Drop(first.clone())).await.unwrap();
        assert!(dropped.contains("Removed community Loud Place (c9): 1 streams refused from now on, 1 stored entries deleted"));
        assert_eq!(store.dropped_streams().await.unwrap(), vec![stream]);
        assert_eq!(run(&store, Command::Dismiss(second.clone())).await.unwrap(), format!("Report {second} dismissed.\n"));
        assert_eq!(run(&store, Command::List { all: false }).await.unwrap(), "No open reports.\n");
        assert!(run(&store, Command::List { all: true }).await.unwrap().contains("dropped"));
        assert_eq!(run(&store, Command::Drop("missing".into())).await.unwrap(), "No report missing.\n");
    }

    #[tokio::test]
    async fn an_operator_cuts_off_restores_and_pins_a_server() {
        let store = Store::memory();
        assert_eq!(run(&store, Command::Gateway).await.unwrap(), "No servers have used the push gateway.\n");
        let origin = "https://bad.example.org".to_string();
        let cut = run(&store, Command::GatewayCut { origin: origin.clone(), reason: "spam".into() }).await.unwrap();
        assert!(cut.contains("cut off"));
        assert!(run(&store, Command::Gateway).await.unwrap().contains("CUT OFF (spam)"));
        run(&store, Command::GatewayRestore(origin.clone())).await.unwrap();
        let listed = run(&store, Command::Gateway).await.unwrap();
        assert!(listed.contains("https://bad.example.org  new  today 0/500 pings") && listed.contains("  ok"), "{listed}");
        assert!(run(&store, Command::GatewayTier { origin: origin.clone(), tier: Some(Tier::Trusted) }).await.unwrap().contains("pinned at trusted"));
        assert!(run(&store, Command::Gateway).await.unwrap().contains("trusted (pinned)"));
        assert!(run(&store, Command::GatewayTier { origin, tier: None }).await.unwrap().contains("follows the trust ramp again (new now)"));
    }
}
