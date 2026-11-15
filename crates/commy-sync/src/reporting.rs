use n0_future::time::{self, Duration};

use crate::client::socket::{Incoming, Socket};
use crate::report::{HostReport, Receipt};
use crate::wire::{ClientFrame, ErrorCode, ServerFrame, REPORTS_SINCE};

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
    match time::timeout(HELLO_WAIT, next_frame(socket)).await {
        Ok(Some(ServerFrame::Hello { protocol, .. })) if protocol >= REPORTS_SINCE => {}
        Ok(Some(ServerFrame::Hello { .. })) => return Err(ReportError::TooOld),
        _ => return Err(ReportError::Unreachable("no greeting".into())),
    }
    let text = serde_json::to_string(&ClientFrame::Report { report: Box::new(report) }).map_err(|_| ReportError::Refused(ErrorCode::BadFrame))?;
    if socket.send(text).await.is_err() {
        return Err(ReportError::Unanswered);
    }
    match time::timeout(ANSWER_WAIT, answer(socket)).await {
        Ok(Some(found)) => found,
        _ => Err(ReportError::Unanswered),
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
