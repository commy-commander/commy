use data_encoding::BASE64URL_NOPAD;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::stream::{Eraser, StreamId, StreamKeys, APPEND_NONCE_BYTES, MAX_SEALED};

pub const PROTOCOL: u32 = 4;
pub const SIGNALS_SINCE: u32 = 2;
pub const WILLS_SINCE: u32 = 3;
pub const ERASE_SINCE: u32 = 4;
pub const ERASE_WINDOW_MS: i64 = 10 * 60 * 1000;
pub const ERASE_EVERYTHING: u64 = i64::MAX as u64;
pub const PAGE_SIZE: usize = 200;
pub const MAX_STREAMS_PER_CONNECTION: usize = 16;
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub const MAX_CLIENT_FRAME_BYTES: usize = 256 * 1024;
pub const FRAME_BODY_BUDGET: usize = 256 * 1024;
pub const KEEPALIVE_SECS: u64 = 45;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bytes<const N: usize>(pub [u8; N]);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Blob(pub Vec<u8>);

impl<const N: usize> Serialize for Bytes<N> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&BASE64URL_NOPAD.encode(&self.0))
    }
}

impl<'de, const N: usize> Deserialize<'de> for Bytes<N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let bytes = BASE64URL_NOPAD.decode(text.as_bytes()).map_err(serde::de::Error::custom)?;
        let fixed: [u8; N] = bytes
            .try_into()
            .map_err(|bytes: Vec<u8>| serde::de::Error::invalid_length(bytes.len(), &"a fixed number of bytes"))?;
        Ok(Self(fixed))
    }
}

impl Serialize for Blob {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&BASE64URL_NOPAD.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for Blob {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.len() > MAX_SEALED.div_ceil(3) * 4 {
            return Err(serde::de::Error::custom("body too large"));
        }
        BASE64URL_NOPAD.decode(text.as_bytes()).map(Blob).map_err(serde::de::Error::custom)
    }
}

impl Serialize for StreamId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for StreamId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        StreamId::from_hex(&text).ok_or_else(|| serde::de::Error::custom("not a stream id"))
    }
}

pub type AppendNonce = Bytes<APPEND_NONCE_BYTES>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientFrame {
    Open {
        stream: StreamId,
        write: Bytes<32>,
        read: Bytes<32>,
        after: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tail: Option<u32>,
    },
    Append { stream: StreamId, nonce: AppendNonce, body: Blob, sig: Bytes<64> },
    Signal { stream: StreamId, nonce: AppendNonce, body: Blob, sig: Bytes<64> },
    Will { stream: StreamId, nonce: AppendNonce, body: Blob, sig: Bytes<64> },
    Close { stream: StreamId },
    Claim { stream: StreamId, write: Bytes<32>, read: Bytes<32>, eraser: Bytes<32>, sig: Bytes<64> },
    Erase { stream: StreamId, through: u64, at: i64, sig: Bytes<64> },
    Release { stream: StreamId, at: i64, sig: Bytes<64> },
    Ping,
}

impl ClientFrame {
    pub fn open(keys: &StreamKeys, after: u64) -> Self {
        Self::open_tail(keys, after, None)
    }

    pub fn open_tail(keys: &StreamKeys, after: u64, tail: Option<u32>) -> Self {
        let tail = tail.filter(|_| after == 0);
        ClientFrame::Open { stream: keys.id, write: Bytes(keys.write_public()), read: Bytes(keys.read_token()), after, tail }
    }

    pub fn append(keys: &StreamKeys, nonce: [u8; APPEND_NONCE_BYTES], body: Vec<u8>) -> Self {
        let sig = keys.sign_append(&nonce, &body);
        ClientFrame::Append { stream: keys.id, nonce: Bytes(nonce), body: Blob(body), sig: Bytes(sig) }
    }

    pub fn signal(keys: &StreamKeys, nonce: [u8; APPEND_NONCE_BYTES], body: Vec<u8>) -> Self {
        let sig = keys.sign_signal(&nonce, &body);
        ClientFrame::Signal { stream: keys.id, nonce: Bytes(nonce), body: Blob(body), sig: Bytes(sig) }
    }

    pub fn will(keys: &StreamKeys, nonce: [u8; APPEND_NONCE_BYTES], body: Vec<u8>) -> Self {
        let sig = keys.sign_signal(&nonce, &body);
        ClientFrame::Will { stream: keys.id, nonce: Bytes(nonce), body: Blob(body), sig: Bytes(sig) }
    }

    pub fn claim(keys: &StreamKeys, eraser: &Eraser) -> Self {
        ClientFrame::Claim {
            stream: keys.id,
            write: Bytes(keys.write_public()),
            read: Bytes(keys.read_token()),
            eraser: Bytes(eraser.public()),
            sig: Bytes(eraser.sign_claim(&keys.id)),
        }
    }

    pub fn erase(stream: StreamId, eraser: &Eraser, through: u64, at: i64) -> Self {
        ClientFrame::Erase { stream, through, at, sig: Bytes(eraser.sign_erase(&stream, through, at)) }
    }

    pub fn release(stream: StreamId, eraser: &Eraser, at: i64) -> Self {
        ClientFrame::Release { stream, at, sig: Bytes(eraser.sign_release(&stream, at)) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireEntry {
    pub seq: u64,
    pub at: i64,
    pub body: Blob,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadFrame,
    BadStream,
    BadSignature,
    TooLarge,
    RateLimited,
    TooManyStreams,
    NotOpen,
    Unavailable,
    NotYours,
    Expired,
}

impl ErrorCode {
    pub fn is_retryable(self) -> bool {
        matches!(self, ErrorCode::RateLimited | ErrorCode::Unavailable | ErrorCode::NotOpen)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerFrame {
    Hello { protocol: u32 },
    Opened { stream: StreamId, head: u64 },
    Entries { stream: StreamId, entries: Vec<WireEntry>, caught_up: bool },
    Appended { stream: StreamId, nonce: AppendNonce, seq: u64, at: i64 },
    Signal { stream: StreamId, body: Blob },
    Claimed { stream: StreamId, mine: bool },
    Erased { stream: StreamId, through: u64 },
    Released { stream: StreamId },
    Error {
        code: ErrorCode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stream: Option<StreamId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        nonce: Option<AppendNonce>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_after_ms: Option<u64>,
    },
    Pong,
}

impl ServerFrame {
    pub fn error(code: ErrorCode) -> Self {
        ServerFrame::Error { code, stream: None, nonce: None, retry_after_ms: None }
    }

    pub fn stream_error(code: ErrorCode, stream: StreamId) -> Self {
        ServerFrame::Error { code, stream: Some(stream), nonce: None, retry_after_ms: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::{verify_append, verify_signal, StreamSecret};

    #[test]
    fn frames_round_trip_as_tagged_json() {
        let keys = StreamSecret::random().keys();
        let frames = [
            ClientFrame::open(&keys, 41),
            ClientFrame::open_tail(&keys, 0, Some(50)),
            ClientFrame::append(&keys, rand::random(), vec![1, 2, 3]),
            ClientFrame::signal(&keys, rand::random(), vec![4, 5]),
            ClientFrame::will(&keys, rand::random(), vec![6]),
            ClientFrame::Close { stream: keys.id },
            ClientFrame::claim(&keys, &Eraser::derive(&[1; 32], &keys.id)),
            ClientFrame::erase(keys.id, &Eraser::derive(&[1; 32], &keys.id), ERASE_EVERYTHING, 1_760_000_000_000),
            ClientFrame::release(keys.id, &Eraser::derive(&[1; 32], &keys.id), 1_760_000_000_000),
            ClientFrame::Ping,
        ];
        for frame in frames {
            let json = serde_json::to_string(&frame).unwrap();
            assert_eq!(serde_json::from_str::<ClientFrame>(&json).unwrap(), frame);
        }
        let json = serde_json::to_value(ClientFrame::open(&keys, 41)).unwrap();
        assert_eq!(json["t"], "open");
        assert_eq!(json["stream"], keys.id.to_hex());
        assert_eq!(json["after"], 41);
        assert!(json.get("tail").is_none(), "older services see the frame they know");
        assert_eq!(serde_json::to_value(ClientFrame::open_tail(&keys, 0, Some(200))).unwrap()["tail"], 200);
        assert!(serde_json::to_value(ClientFrame::open_tail(&keys, 9, Some(200))).unwrap().get("tail").is_none(), "a cursor wins over a tail");
    }

    #[test]
    fn an_append_frame_carries_a_signature_the_service_can_check() {
        let keys = StreamSecret::random().keys();
        let ClientFrame::Append { stream, nonce, body, sig } = ClientFrame::append(&keys, rand::random(), b"x".to_vec()) else {
            unreachable!()
        };
        assert!(verify_append(&keys.write_public(), &stream, &nonce.0, &body.0, &sig.0));
        let ClientFrame::Signal { stream, nonce, body, sig } = ClientFrame::signal(&keys, rand::random(), b"x".to_vec()) else {
            unreachable!()
        };
        assert!(verify_signal(&keys.write_public(), &stream, &nonce.0, &body.0, &sig.0));
        assert!(!verify_append(&keys.write_public(), &stream, &nonce.0, &body.0, &sig.0), "a signal cannot be stored as an entry");
        let ClientFrame::Will { stream, nonce, body, sig } = ClientFrame::will(&keys, rand::random(), b"x".to_vec()) else {
            unreachable!()
        };
        assert!(verify_signal(&keys.write_public(), &stream, &nonce.0, &body.0, &sig.0), "a last word is a signal held for later");
    }

    #[test]
    fn server_frames_round_trip() {
        let id = StreamSecret::random().keys().id;
        let frames = [
            ServerFrame::Hello { protocol: PROTOCOL },
            ServerFrame::Opened { stream: id, head: 7 },
            ServerFrame::Entries { stream: id, entries: vec![WireEntry { seq: 1, at: 1_760_000_000_000, body: Blob(vec![4; 40]) }], caught_up: true },
            ServerFrame::Appended { stream: id, nonce: Bytes([1; APPEND_NONCE_BYTES]), seq: 8, at: 5 },
            ServerFrame::Signal { stream: id, body: Blob(vec![2; 30]) },
            ServerFrame::Claimed { stream: id, mine: false },
            ServerFrame::Erased { stream: id, through: ERASE_EVERYTHING },
            ServerFrame::Released { stream: id },
            ServerFrame::stream_error(ErrorCode::NotYours, id),
            ServerFrame::stream_error(ErrorCode::BadSignature, id),
            ServerFrame::Error { code: ErrorCode::RateLimited, stream: None, nonce: None, retry_after_ms: Some(900) },
            ServerFrame::Pong,
        ];
        for frame in frames {
            let json = serde_json::to_string(&frame).unwrap();
            assert_eq!(serde_json::from_str::<ServerFrame>(&json).unwrap(), frame);
        }
        assert_eq!(serde_json::to_string(&ServerFrame::error(ErrorCode::BadFrame)).unwrap(), r#"{"t":"error","code":"bad_frame"}"#);
    }

    #[test]
    fn a_claim_proves_the_stream_and_the_eraser() {
        use crate::stream::{verify_claim, verify_erase, verify_release};
        let keys = StreamSecret::random().keys();
        let eraser = Eraser::derive(&[9; 32], &keys.id);
        let ClientFrame::Claim { stream, write, read, eraser: public, sig } = ClientFrame::claim(&keys, &eraser) else { unreachable!() };
        assert_eq!(crate::stream::StreamId::of(&write.0, &read.0), stream);
        assert!(verify_claim(&public.0, &stream, &sig.0));
        let ClientFrame::Erase { stream, through, at, sig } = ClientFrame::erase(keys.id, &eraser, 7, 99) else { unreachable!() };
        assert!(verify_erase(&eraser.public(), &stream, through, at, &sig.0));
        let ClientFrame::Release { stream, at, sig } = ClientFrame::release(keys.id, &eraser, 99) else { unreachable!() };
        assert!(verify_release(&eraser.public(), &stream, at, &sig.0));
        assert!(i64::try_from(ERASE_EVERYTHING).is_ok(), "the whole stream fits a database sequence number");
    }

    #[test]
    fn malformed_frames_are_refused() {
        let id = StreamSecret::random().keys().id;
        for json in [
            r#"{"t":"open"}"#.to_string(),
            r#"{"t":"nope"}"#.to_string(),
            format!(r#"{{"t":"open","stream":"{id}","write":"AAAA","read":"AAAA","after":0}}"#),
            r#"{"t":"close","stream":"abcd"}"#.to_string(),
        ] {
            assert!(serde_json::from_str::<ClientFrame>(&json).is_err(), "{json}");
        }
        let huge = format!(
            r#"{{"t":"append","stream":"{id}","nonce":"{}","body":"{}","sig":"{}"}}"#,
            BASE64URL_NOPAD.encode(&[0; APPEND_NONCE_BYTES]),
            "A".repeat(MAX_SEALED * 2),
            BASE64URL_NOPAD.encode(&[0; 64])
        );
        assert!(serde_json::from_str::<ClientFrame>(&huge).is_err());
    }

    #[test]
    fn only_passing_troubles_are_retried() {
        assert!(ErrorCode::RateLimited.is_retryable());
        assert!(ErrorCode::Unavailable.is_retryable());
        assert!(!ErrorCode::BadSignature.is_retryable());
        assert!(!ErrorCode::TooLarge.is_retryable());
    }
}
