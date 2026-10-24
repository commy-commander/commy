pub use commy_view::pictures::{audio_mime, expression_mime, image_mime, MAX_EMOJI_BYTES, MAX_SOUND_BYTES, MAX_STICKER_BYTES};

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
}
