use commy_core::{ChannelId, UserId};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MEDIA_ALPN: &[u8] = b"commy/media/1";

pub const MAX_MEDIA_FRAME: u32 = 4 * 1024 * 1024;

pub const MAX_INK_POINTS: usize = 64;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("media frame of {0} bytes exceeds limit")]
    FrameTooLarge(u32),
    #[error("malformed media frame")]
    Malformed,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MediaHello {
    Voice { user: UserId, channel: ChannelId },
    ScreenSubscribe { user: UserId, channel: ChannelId },
    CameraSubscribe { user: UserId, channel: ChannelId },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MediaPacket {
    Voice { seq: u32, pcm: Vec<i16> },
    Video {
        seq: u64,
        width: u32,
        height: u32,
        jpeg: Vec<u8>,
    },
    Opus { seq: u32, data: Vec<u8> },
    StreamAudio { seq: u32, data: Vec<u8> },
    StreamAudioWanted,
    InkReady,
    InkAllowed { on: bool },
    Ink { mark: InkMark },
    InkFrom { user: UserId, mark: InkMark },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InkMark {
    Point { x: u16, y: u16 },
    Line { stroke: u32, points: Vec<[u16; 2]> },
    Clear,
}

impl InkMark {
    pub fn is_valid(&self) -> bool {
        match self {
            InkMark::Line { points, .. } => !points.is_empty() && points.len() <= MAX_INK_POINTS,
            InkMark::Point { .. } | InkMark::Clear => true,
        }
    }
}

impl MediaPacket {
    pub fn is_video(&self) -> bool {
        matches!(self, MediaPacket::Video { .. })
    }
}

pub async fn write_frame<W, T>(writer: &mut W, value: &T) -> Result<(), MediaError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let mut bytes = postcard::to_extend(value, vec![0u8; 4]).map_err(|_| MediaError::Malformed)?;
    let len = (bytes.len() - 4) as u32;
    if len > MAX_MEDIA_FRAME {
        return Err(MediaError::FrameTooLarge(len));
    }
    bytes[..4].copy_from_slice(&len.to_le_bytes());
    writer.write_all(&bytes).await?;
    Ok(())
}

pub async fn read_frame<R, T>(reader: &mut R) -> Result<Option<T>, MediaError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    read_frame_capped(reader, MAX_MEDIA_FRAME).await
}

pub(crate) async fn read_frame_capped<R, T>(reader: &mut R, max: u32) -> Result<Option<T>, MediaError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let Some(buf) = read_frame_bytes(reader, max).await? else {
        return Ok(None);
    };
    postcard::from_bytes(&buf)
        .map(Some)
        .map_err(|_| MediaError::Malformed)
}

pub(crate) async fn read_frame_bytes<R>(reader: &mut R, max: u32) -> Result<Option<Vec<u8>>, MediaError>
where
    R: AsyncRead + Unpin,
{
    let mut len_bytes = [0u8; 4];
    match reader.read_exact(&mut len_bytes).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_le_bytes(len_bytes);
    if len > max {
        return Err(MediaError::FrameTooLarge(len));
    }
    let mut buf = vec![0u8; len as usize];
    reader.read_exact(&mut buf).await?;
    Ok(Some(buf))
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
        let end: Option<MediaPacket> = read_frame(&mut server).await.unwrap();
        assert!(end.is_none());
    }

    #[test]
    fn appended_variants_leave_earlier_wire_tags_unchanged() {
        let wire_tag = |bytes: Vec<u8>| bytes[0];
        let (user, channel) = (UserId::new(), ChannelId::new());
        assert_eq!(wire_tag(postcard::to_stdvec(&MediaHello::Voice { user, channel }).unwrap()), 0);
        assert_eq!(wire_tag(postcard::to_stdvec(&MediaHello::CameraSubscribe { user, channel }).unwrap()), 2);
        assert_eq!(wire_tag(postcard::to_stdvec(&MediaPacket::Voice { seq: 0, pcm: vec![] }).unwrap()), 0);
        assert_eq!(wire_tag(postcard::to_stdvec(&MediaPacket::Opus { seq: 0, data: vec![] }).unwrap()), 2);
        assert_eq!(wire_tag(postcard::to_stdvec(&MediaPacket::StreamAudio { seq: 0, data: vec![] }).unwrap()), 3);
        assert_eq!(wire_tag(postcard::to_stdvec(&MediaPacket::StreamAudioWanted).unwrap()), 4);
        assert_eq!(wire_tag(postcard::to_stdvec(&MediaPacket::InkReady).unwrap()), 5);
        assert_eq!(wire_tag(postcard::to_stdvec(&MediaPacket::InkAllowed { on: true }).unwrap()), 6);
        assert_eq!(wire_tag(postcard::to_stdvec(&MediaPacket::Ink { mark: InkMark::Clear }).unwrap()), 7);
        assert_eq!(wire_tag(postcard::to_stdvec(&MediaPacket::InkFrom { user, mark: InkMark::Clear }).unwrap()), 8);
    }

    #[test]
    fn ink_lines_carry_a_bounded_number_of_points() {
        let line = |n: usize| InkMark::Line { stroke: 1, points: vec![[0, 0]; n] };
        assert!(!line(0).is_valid());
        assert!(line(1).is_valid());
        assert!(line(MAX_INK_POINTS).is_valid());
        assert!(!line(MAX_INK_POINTS + 1).is_valid());
        assert!(InkMark::Point { x: u16::MAX, y: 0 }.is_valid());
    }

    #[tokio::test]
    async fn ink_kinds_roundtrip() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        let user = UserId::new();
        let sent = [
            MediaPacket::InkReady,
            MediaPacket::InkAllowed { on: true },
            MediaPacket::Ink { mark: InkMark::Line { stroke: 7, points: vec![[1, 2], [3, 4]] } },
            MediaPacket::InkFrom { user, mark: InkMark::Point { x: 9, y: 10 } },
        ];
        for packet in &sent {
            write_frame(&mut a, packet).await.unwrap();
        }
        for packet in sent {
            let got: MediaPacket = read_frame(&mut b).await.unwrap().unwrap();
            assert_eq!(got, packet);
        }
    }

    #[derive(Debug, PartialEq, Deserialize)]
    enum PacketBeforeStreamAudio {
        Voice { seq: u32, pcm: Vec<i16> },
        Video { seq: u64, width: u32, height: u32, jpeg: Vec<u8> },
        Opus { seq: u32, data: Vec<u8> },
    }

    #[tokio::test]
    async fn a_reader_without_the_stream_audio_kinds_rejects_them_so_senders_wait_to_be_asked() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        write_frame(&mut a, &MediaPacket::Opus { seq: 1, data: vec![9] }).await.unwrap();
        write_frame(&mut a, &MediaPacket::StreamAudio { seq: 2, data: vec![8] }).await.unwrap();
        let old: PacketBeforeStreamAudio = read_frame(&mut b).await.unwrap().unwrap();
        assert_eq!(old, PacketBeforeStreamAudio::Opus { seq: 1, data: vec![9] });
        let unknown: Result<Option<PacketBeforeStreamAudio>, _> = read_frame(&mut b).await;
        assert!(matches!(unknown, Err(MediaError::Malformed)));
    }

    #[tokio::test]
    async fn stream_audio_kinds_roundtrip() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        write_frame(&mut a, &MediaPacket::StreamAudioWanted).await.unwrap();
        write_frame(&mut a, &MediaPacket::StreamAudio { seq: 3, data: vec![1, 2] }).await.unwrap();
        let first: MediaPacket = read_frame(&mut b).await.unwrap().unwrap();
        assert_eq!(first, MediaPacket::StreamAudioWanted);
        assert!(!first.is_video());
        let second: MediaPacket = read_frame(&mut b).await.unwrap().unwrap();
        assert_eq!(second, MediaPacket::StreamAudio { seq: 3, data: vec![1, 2] });
    }

    #[tokio::test]
    async fn opus_voice_roundtrip() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        let packet = MediaPacket::Opus { seq: 42, data: vec![0x78, 1, 2, 3] };
        write_frame(&mut a, &packet).await.unwrap();
        let got: MediaPacket = read_frame(&mut b).await.unwrap().unwrap();
        assert_eq!(got, packet);
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
