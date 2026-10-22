use n0_future::{SinkExt, StreamExt};
use ws_stream_wasm::{WsMessage, WsMeta, WsStream};

use super::CONNECT_TIMEOUT;

pub enum Incoming {
    Text(String),
    Other,
    Closed,
}

pub struct Socket {
    meta: WsMeta,
    stream: WsStream,
}

impl Socket {
    pub async fn connect(url: &str) -> Result<Socket, String> {
        let (meta, stream) = n0_future::time::timeout(CONNECT_TIMEOUT, WsMeta::connect(url, None))
            .await
            .map_err(|_| "timed out".to_string())?
            .map_err(|e| e.to_string())?;
        Ok(Socket { meta, stream })
    }

    pub async fn send(&mut self, text: String) -> Result<(), ()> {
        self.stream.send(WsMessage::Text(text)).await.map_err(|_| ())
    }

    pub async fn next(&mut self) -> Incoming {
        match self.stream.next().await {
            Some(WsMessage::Text(text)) => Incoming::Text(text),
            Some(WsMessage::Binary(_)) => Incoming::Other,
            None => Incoming::Closed,
        }
    }

    pub async fn close(&mut self) {
        let _ = self.meta.close().await;
    }
}
