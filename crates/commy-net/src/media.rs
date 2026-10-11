//! Framing for direct media connections (voice and screen share).
//!
//! Media never rides the gossip topic. Peers open a dedicated QUIC
//! connection with the `commy/media/1` ALPN, send one `MediaHello` on
//! a bidirectional stream, then exchange length-prefixed postcard
//! frames. Voice is raw 48 kHz mono PCM for now (10 ms, 480 samples);
//! screen share is MJPEG. Both leave room for real codecs later.

use commy_core::{ChannelId, UserId};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MEDIA_ALPN: &[u8] = b"commy/media/1";

/// Big enough for a scaled JPEG frame, small enough to bound memory.
pub const MAX_MEDIA_FRAME: u32 = 4 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("media frame of {0} bytes exceeds limit")]
    FrameTooLarge(u32),
    #[error("malformed media frame")]
    Malformed,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// First frame on every media stream: who is calling and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MediaHello {
    /// Two-way voice for a channel both peers are connected to.
    Voice { user: UserId, channel: ChannelId },
    /// Viewer asking the sharer to stream their screen.
    ScreenSubscribe { user: UserId, channel: ChannelId },
    /// Viewer asking a member to stream their camera. Appended last so
    /// the earlier variants keep their wire tags.
    CameraSubscribe { user: UserId, channel: ChannelId },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MediaPacket {
    /// 10 ms of 48 kHz mono PCM, already noise-processed by the sender.
    Voice { seq: u32, pcm: Vec<i16> },
    /// One MJPEG screen frame.
    Video {
        seq: u64,
        width: u32,
        height: u32,
        jpeg: Vec<u8>,
    },
}

/// Writes one length-prefixed postcard frame.
pub async fn write_frame<W, T>(writer: &mut W, value: &T) -> Result<(), MediaError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let bytes = postcard::to_stdvec(value).map_err(|_| MediaError::Malformed)?;
    let len = bytes.len() as u32;
    if len > MAX_MEDIA_FRAME {
        return Err(MediaError::FrameTooLarge(len));
    }
    writer.write_all(&len.to_le_bytes()).await?;
    writer.write_all(&bytes).await?;
    Ok(())
}

/// Reads one frame; Ok(None) means the peer closed the stream cleanly.
pub async fn read_frame<R, T>(reader: &mut R) -> Result<Option<T>, MediaError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut len_bytes = [0u8; 4];
    match reader.read_exact(&mut len_bytes).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_le_bytes(len_bytes);
    if len > MAX_MEDIA_FRAME {
        return Err(MediaError::FrameTooLarge(len));
    }
    let mut buf = vec![0u8; len as usize];
    reader.read_exact(&mut buf).await?;
    postcard::from_bytes(&buf)
        .map(Some)
        .map_err(|_| MediaError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hello_then_packets_roundtrip() {
        let (mut client, mut server) = tokio::io::duplex(64 * 1024);
        let hello = MediaHello::Voice {
            user: UserId::new(),
            channel: ChannelId::new(),
        };
        let packet = MediaPacket::Voice {
            seq: 7,
            pcm: vec![0i16; 480],
        };

        write_frame(&mut client, &hello).await.unwrap();
        write_frame(&mut client, &packet).await.unwrap();
        drop(client);

        let got_hello: MediaHello = read_frame(&mut server).await.unwrap().unwrap();
        assert_eq!(got_hello, hello);
        let got_packet: MediaPacket = read_frame(&mut server).await.unwrap().unwrap();
        assert_eq!(got_packet, packet);
        // Clean EOF after the sender hangs up.
        let end: Option<MediaPacket> = read_frame(&mut server).await.unwrap();
        assert!(end.is_none());
    }

    #[tokio::test]
    async fn video_frame_roundtrip() {
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);
        let packet = MediaPacket::Video {
            seq: 1,
            width: 1280,
            height: 720,
            jpeg: vec![0xff, 0xd8, 0xff, 0xe0, 1, 2, 3],
        };
        write_frame(&mut a, &packet).await.unwrap();
        let got: MediaPacket = read_frame(&mut b).await.unwrap().unwrap();
        assert_eq!(got, packet);
    }

    #[tokio::test]
    async fn oversized_length_prefix_is_rejected() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        let bogus = (MAX_MEDIA_FRAME + 1).to_le_bytes();
        tokio::io::AsyncWriteExt::write_all(&mut a, &bogus)
            .await
            .unwrap();
        let result: Result<Option<MediaPacket>, _> = read_frame(&mut b).await;
        assert!(matches!(result, Err(MediaError::FrameTooLarge(_))));
    }

    #[tokio::test]
    async fn garbage_payload_is_malformed() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        tokio::io::AsyncWriteExt::write_all(&mut a, &4u32.to_le_bytes())
            .await
            .unwrap();
        tokio::io::AsyncWriteExt::write_all(&mut a, &[0xde, 0xad, 0xbe, 0xef])
            .await
            .unwrap();
        let result: Result<Option<MediaHello>, _> = read_frame(&mut b).await;
        assert!(matches!(result, Err(MediaError::Malformed)));
    }
}
