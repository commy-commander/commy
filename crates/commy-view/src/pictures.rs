pub const MAX_EMOJI_BYTES: usize = 256 * 1024;
pub const MAX_STICKER_BYTES: usize = 512 * 1024;
pub const MAX_SOUND_BYTES: usize = 1024 * 1024;
const LONGEST_SOUND_SECS: usize = 15;
const HIGHEST_SOUND_KBPS: usize = 320;
const _: () = assert!(LONGEST_SOUND_SECS * HIGHEST_SOUND_KBPS * 1000 / 8 < MAX_SOUND_BYTES);

pub const MAX_PROFILE_IMAGE_BYTES: usize = 512 * 1024;
pub const MAX_SERVER_IMAGE_BYTES: usize = 1024 * 1024;

pub fn is_jpeg_or_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xff, 0xd8, 0xff]) || bytes.starts_with(&[0x89, b'P', b'N', b'G'])
}

pub fn picture_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, b'P']) {
        "image/png"
    } else {
        "image/jpeg"
    }
}

pub fn pictures_url(sync_url: &str) -> Option<String> {
    let (scheme, rest) = match sync_url.strip_prefix("wss://") {
        Some(rest) => ("https", rest),
        None => ("http", sync_url.strip_prefix("ws://")?),
    };
    Some(format!("{scheme}://{}/pictures", rest.trim_end_matches('/')))
}

pub fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

pub fn audio_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"OggS") {
        Some("audio/ogg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        Some("audio/wav")
    } else if bytes.starts_with(b"ID3") || (bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0) {
        Some("audio/mpeg")
    } else {
        None
    }
}

pub fn expression_mime(bytes: &[u8]) -> Option<&'static str> {
    match (image_mime(bytes), audio_mime(bytes)) {
        (Some(mime), _) if bytes.len() <= MAX_STICKER_BYTES => Some(mime),
        (None, Some(mime)) if bytes.len() <= MAX_SOUND_BYTES => Some(mime),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_images() {
        assert_eq!(image_mime(b"\x89PNG\r\n"), Some("image/png"));
        assert_eq!(image_mime(b"GIF89a..."), Some("image/gif"));
        assert_eq!(image_mime(&[0xff, 0xd8, 0xff, 0xe0]), Some("image/jpeg"));
        assert_eq!(image_mime(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(image_mime(b"<svg>"), None);
    }

    #[test]
    fn sniffs_audio() {
        assert_eq!(audio_mime(b"OggS\0"), Some("audio/ogg"));
        assert_eq!(audio_mime(b"RIFF\0\0\0\0WAVEfmt "), Some("audio/wav"));
        assert_eq!(audio_mime(b"ID3\x04"), Some("audio/mpeg"));
        assert_eq!(audio_mime(&[0xff, 0xfb, 0x90]), Some("audio/mpeg"));
        assert_eq!(audio_mime(b"MZ\x90\0"), None);
    }

    #[test]
    fn enforces_size_per_kind() {
        let mut png = b"\x89PNG".to_vec();
        png.resize(MAX_STICKER_BYTES, 0);
        assert!(expression_mime(&png).is_some());
        png.push(0);
        assert!(expression_mime(&png).is_none());
        let mut ogg = b"OggS".to_vec();
        ogg.resize(MAX_SOUND_BYTES, 0);
        assert!(expression_mime(&ogg).is_some());
        ogg.push(0);
        assert!(expression_mime(&ogg).is_none());
    }

    #[test]
    fn profile_and_server_pictures_are_jpeg_or_png() {
        assert!(is_jpeg_or_png(&[0xff, 0xd8, 0xff, 0xe0]));
        assert!(is_jpeg_or_png(b"\x89PNG\r\n"));
        assert!(!is_jpeg_or_png(b"GIF89a"));
        assert_eq!(picture_mime(b"\x89PNG"), "image/png");
        assert_eq!(picture_mime(&[0xff, 0xd8, 0xff]), "image/jpeg");
    }

    #[test]
    fn pictures_live_beside_the_sync_service() {
        assert_eq!(pictures_url("wss://commy.chat/sync").as_deref(), Some("https://commy.chat/sync/pictures"));
        assert_eq!(pictures_url("ws://localhost:3012/").as_deref(), Some("http://localhost:3012/pictures"));
        assert_eq!(pictures_url("https://commy.chat/sync"), None);
    }
}
