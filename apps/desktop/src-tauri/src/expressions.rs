pub const MAX_EMOJI_BYTES: usize = 256 * 1024;
pub const MAX_STICKER_BYTES: usize = 512 * 1024;
pub const MAX_SOUND_BYTES: usize = 1024 * 1024;
const LONGEST_SOUND_SECS: usize = 15;
const HIGHEST_SOUND_KBPS: usize = 320;
const _: () = assert!(LONGEST_SOUND_SECS * HIGHEST_SOUND_KBPS * 1000 / 8 < MAX_SOUND_BYTES);

pub const DEFAULT_SOUND_PREFIX: &str = "default:";

pub struct DefaultSound {
    pub id: &'static str,
    pub label: &'static str,
}

pub const DEFAULT_SOUNDS: &[DefaultSound] = &[
    DefaultSound { id: "airhorn", label: "Airhorn" },
    DefaultSound { id: "ba_dum_tss", label: "Ba Dum Tss" },
    DefaultSound { id: "cricket", label: "Crickets" },
    DefaultSound { id: "losing_horn", label: "Losing Horn" },
];

pub fn is_default_sound(name: &str) -> bool {
    name.strip_prefix(DEFAULT_SOUND_PREFIX).is_some_and(|id| DEFAULT_SOUNDS.iter().any(|s| s.id == id))
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
    fn default_sounds_need_the_prefix_and_a_known_id() {
        assert!(is_default_sound("default:airhorn"));
        assert!(is_default_sound("default:losing_horn"));
        assert!(!is_default_sound("airhorn"));
        assert!(!is_default_sound("default:"));
        assert!(!is_default_sound("default:../../app"));
    }

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
}
