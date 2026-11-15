use crate::reports::{describe, ReportState};
use crate::store::{Store, StoreError};

pub const USAGE: &str = "usage: commy-sync [check <wss://domain/sync> | reports [--all] | show <report> | drop <report> | dismiss <report>]";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    List { all: bool },
    Show(String),
    Drop(String),
    Dismiss(String),
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
            _ => Err(USAGE),
        }
    }
}

pub async fn run(store: &Store, command: Command) -> Result<String, StoreError> {
    Ok(match command {
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
        for bad in ["drop", "drop a b", "erase abc", "reports --everything"] {
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
}
