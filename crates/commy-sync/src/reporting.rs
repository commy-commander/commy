use n0_future::time::{self, Duration};

use crate::client::socket::{Incoming, Socket};
use crate::report::{HostReport, Receipt};
use crate::report_seal::ReportKey;
use crate::wire::{ClientFrame, ErrorCode, Greeting, ServerFrame, REPORTS_SINCE};

const HELLO_WAIT: Duration = Duration::from_secs(10);
const ANSWER_WAIT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReportError {
    #[error("could not reach the sync server: {0}")]
    Unreachable(String),
    #[error("this sync server does not take reports yet")]
    TooOld,
    #[error("the sync server refused the report: {0:?}")]
    Refused(ErrorCode),
    #[error("the sync server did not answer")]
    Unanswered,
}

pub async fn send(url: &str, report: HostReport) -> Result<Receipt, ReportError> {
    let mut socket = Socket::connect(url).await.map_err(ReportError::Unreachable)?;
    let outcome = deliver(&mut socket, report).await;
    socket.close().await;
    outcome
}

async fn deliver(socket: &mut Socket, report: HostReport) -> Result<Receipt, ReportError> {
    let greeting = match time::timeout(HELLO_WAIT, next_frame(socket)).await {
        Ok(Some(frame)) => frame.greeting().ok_or_else(|| ReportError::Unreachable("no greeting".into()))?,
        _ => return Err(ReportError::Unreachable("no greeting".into())),
    };
    if greeting.protocol < REPORTS_SINCE {
        return Err(ReportError::TooOld);
    }
    let text = serde_json::to_string(&report_frame(&greeting, report)).map_err(|_| ReportError::Refused(ErrorCode::BadFrame))?;
    if socket.send(text).await.is_err() {
        return Err(ReportError::Unanswered);
    }
    match time::timeout(ANSWER_WAIT, answer(socket)).await {
        Ok(Some(found)) => found,
        _ => Err(ReportError::Unanswered),
    }
}

pub fn report_frame(greeting: &Greeting, report: HostReport) -> ClientFrame {
    let sealed = greeting.report_key.as_deref().and_then(ReportKey::from_text).and_then(|key| key.seal(&report));
    match sealed {
        Some(sealed) => ClientFrame::SealedReport { report: Box::new(sealed) },
        None => ClientFrame::Report { report: Box::new(report) },
    }
}

async fn answer(socket: &mut Socket) -> Option<Result<Receipt, ReportError>> {
    loop {
        match next_frame(socket).await? {
            ServerFrame::Reported { receipt } => return Some(Ok(receipt)),
            ServerFrame::Error { code, stream: None, nonce: None, .. } => return Some(Err(ReportError::Refused(code))),
            _ => {}
        }
    }
}

async fn next_frame(socket: &mut Socket) -> Option<ServerFrame> {
    loop {
        match socket.next().await {
            Incoming::Text(text) => {
                if let Ok(frame) = serde_json::from_str::<ServerFrame>(&text) {
                    return Some(frame);
                }
            }
            Incoming::Other => {}
            Incoming::Closed => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::tests::sample;
    use crate::report_seal::ReportSecret;
    use crate::wire::PROTOCOL;

    #[test]
    fn a_service_with_a_report_key_gets_the_report_sealed_to_it() {
        let secret = ReportSecret::generate();
        let greeting = Greeting { report_key: Some(secret.public().to_text()), ..Greeting::of(PROTOCOL) };
        let sent = sample();
        let ClientFrame::SealedReport { report } = report_frame(&greeting, sent.clone()) else { panic!("a sealed report") };
        assert_eq!(secret.open(&report), Ok(sent));
    }

    #[test]
    fn a_service_without_a_usable_report_key_gets_the_report_it_always_took() {
        for report_key in [None, Some("commy-report-key1.nonsense".to_string()), Some("commy-report-key1.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string())] {
            let greeting = Greeting { report_key, ..Greeting::of(PROTOCOL) };
            let sent = sample();
            assert_eq!(report_frame(&greeting, sent.clone()), ClientFrame::Report { report: Box::new(sent) });
        }
    }
}
