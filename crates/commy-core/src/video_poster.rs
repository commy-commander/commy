use crate::message::Attachment;

pub const VIDEO_POSTER_MIME: &str = "image/jpeg";
pub const MAX_VIDEO_POSTER_BYTES: usize = 256 * 1024;
const POSTER_NAME_PREFIX: &str = "video-poster-";
const POSTER_NAME_SUFFIX: &str = ".jpg";
const JPEG_START: [u8; 3] = [0xFF, 0xD8, 0xFF];

pub fn is_poster_picture(bytes: &[u8]) -> bool {
    bytes.len() <= MAX_VIDEO_POSTER_BYTES && bytes.starts_with(&JPEG_START)
}

impl Attachment {
    pub fn is_video(&self) -> bool {
        self.mime_type.starts_with("video/")
    }

    pub fn video_poster(video: &Attachment, blob: String, size_bytes: u64, provider: Option<String>) -> Self {
        Self {
            blob,
            filename: format!("{POSTER_NAME_PREFIX}{}{POSTER_NAME_SUFFIX}", video.blob),
            size_bytes,
            mime_type: VIDEO_POSTER_MIME.to_string(),
            provider,
        }
    }

    fn poster_of(&self) -> Option<&str> {
        if self.mime_type != VIDEO_POSTER_MIME || self.blob.is_empty() || self.size_bytes > MAX_VIDEO_POSTER_BYTES as u64 {
            return None;
        }
        let video = self.filename.strip_prefix(POSTER_NAME_PREFIX)?.strip_suffix(POSTER_NAME_SUFFIX)?;
        (!video.is_empty()).then_some(video)
    }
}

pub fn poster_for<'a>(video: &Attachment, attachments: &'a [Attachment]) -> Option<&'a Attachment> {
    if !video.is_video() || video.blob.is_empty() {
        return None;
    }
    attachments.iter().find(|a| a.poster_of() == Some(video.blob.as_str()))
}

pub fn is_paired_poster(attachment: &Attachment, attachments: &[Attachment]) -> bool {
    attachment
        .poster_of()
        .is_some_and(|video| attachments.iter().any(|a| a.is_video() && a.blob == video))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video() -> Attachment {
        Attachment { blob: "v1".into(), filename: "clip.mp4".into(), size_bytes: 9_000_000, mime_type: "video/mp4".into(), provider: Some("node".into()) }
    }

    #[test]
    fn a_poster_pairs_with_the_video_it_names() {
        let video = video();
        let poster = Attachment::video_poster(&video, "p1".into(), 30_000, Some("node".into()));
        let all = vec![video.clone(), poster.clone()];
        assert_eq!(poster.filename, "video-poster-v1.jpg");
        assert!(poster.is_image());
        assert_eq!(poster_for(&video, &all), Some(&poster));
        assert!(is_paired_poster(&poster, &all));
        assert!(!is_paired_poster(&video, &all));
    }

    #[test]
    fn a_poster_without_its_video_stays_a_plain_picture() {
        let video = video();
        let poster = Attachment::video_poster(&video, "p1".into(), 30_000, None);
        let other = Attachment { blob: "v2".into(), ..video.clone() };
        let all = vec![other.clone(), poster.clone()];
        assert!(!is_paired_poster(&poster, &all));
        assert_eq!(poster_for(&other, &all), None);
    }

    #[test]
    fn oversized_or_mislabelled_posters_are_ignored() {
        let video = video();
        let big = Attachment::video_poster(&video, "p1".into(), MAX_VIDEO_POSTER_BYTES as u64 + 1, None);
        let png = Attachment { mime_type: "image/png".into(), ..Attachment::video_poster(&video, "p2".into(), 10, None) };
        let empty = Attachment::video_poster(&video, String::new(), 10, None);
        for poster in [big, png, empty] {
            assert_eq!(poster_for(&video, &[video.clone(), poster.clone()]), None);
        }
    }

    #[test]
    fn only_small_jpegs_are_accepted_as_poster_pictures() {
        assert!(is_poster_picture(&[0xFF, 0xD8, 0xFF, 0xE0, 1, 2]));
        assert!(!is_poster_picture(b"\x89PNG\r\n\x1a\n"));
        assert!(!is_poster_picture(&[]));
        let mut huge = vec![0xFF, 0xD8, 0xFF];
        huge.resize(MAX_VIDEO_POSTER_BYTES + 1, 0);
        assert!(!is_poster_picture(&huge));
    }
}
