use commy_core::Attachment;

pub const MAX_SEND_BYTES: u64 = 100 * 1024 * 1024;
pub const SEND_TOO_LARGE: &str = "files larger than 100 MB can be sent from the desktop app";
pub const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_VIDEO_BYTES: u64 = 100 * 1024 * 1024;
pub const MAX_VOICE_MESSAGE_BYTES: u64 = 8 * 1024 * 1024;
pub const AUTO_PREVIEW_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_PREVIEW_BYTES: u64 = 20 * 1024 * 1024;
const MAX_FILENAME_BYTES: usize = 255;

pub fn upload_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() || name.len() > MAX_FILENAME_BYTES {
        return Err("bad filename".into());
    }
    Ok(name.to_string())
}

pub fn upload_bytes(data: &str) -> Result<Vec<u8>, String> {
    let bytes = data_encoding::BASE64.decode(data.as_bytes()).map_err(|_| "bad file data".to_string())?;
    check_upload(&bytes)?;
    Ok(bytes)
}

pub fn check_upload(bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("empty file".into());
    }
    if bytes.len() as u64 > MAX_SEND_BYTES {
        return Err(SEND_TOO_LARGE.into());
    }
    Ok(())
}

pub fn attachment(filename: String, size_bytes: u64, blob: String, provider: String) -> Attachment {
    let mime_type = mime_guess::from_path(&filename).first_raw().unwrap_or("application/octet-stream").to_string();
    Attachment { blob, filename, size_bytes, mime_type, provider: Some(provider) }
}

pub fn is_image_mime(peer_supplied: &str) -> bool {
    let subtype = peer_supplied.strip_prefix("image/").unwrap_or("");
    !subtype.is_empty() && subtype.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'))
}

pub fn preview_limit(clicked: bool) -> u64 {
    if clicked {
        MAX_PREVIEW_BYTES
    } else {
        AUTO_PREVIEW_BYTES
    }
}

pub fn data_url(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", data_encoding::BASE64.encode(bytes))
}

pub fn fits(size_bytes: Option<u64>, max: u64, refusal: &str) -> Result<(), String> {
    match size_bytes {
        Some(size) if size > max => Err(refusal.to_string()),
        _ => Ok(()),
    }
}

pub fn byte_limit(cmd: &str) -> Option<(u64, &'static str)> {
    match cmd {
        "get_attachment_video" => Some((MAX_VIDEO_BYTES, "video too large to play here")),
        "get_voice_message" => Some((MAX_VOICE_MESSAGE_BYTES, "voice message too large to play here")),
        "download_attachment" => Some((MAX_DOWNLOAD_BYTES, "files larger than 512 MB are not supported yet")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_names_are_trimmed_and_bounded() {
        assert_eq!(upload_name("  notes.txt ").unwrap(), "notes.txt");
        assert!(upload_name("   ").is_err());
        assert!(upload_name(&"a".repeat(256)).is_err());
        assert!(upload_name(&"a".repeat(255)).is_ok());
    }

    #[test]
    fn uploads_must_be_real_non_empty_and_small_enough() {
        assert_eq!(upload_bytes("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(upload_bytes("").unwrap_err(), "empty file");
        assert_eq!(upload_bytes("not base64!").unwrap_err(), "bad file data");
        assert_eq!(check_upload(&vec![0; MAX_SEND_BYTES as usize + 1]).unwrap_err(), SEND_TOO_LARGE);
    }

    #[test]
    fn an_attachment_names_this_browser_as_provider_and_guesses_its_type() {
        let picture = attachment("cat.PNG".into(), 3, "hash".into(), "node".into());
        assert_eq!((picture.mime_type.as_str(), picture.provider.as_deref()), ("image/png", Some("node")));
        assert_eq!(attachment("data.weird".into(), 3, "h".into(), "n".into()).mime_type, "application/octet-stream");
    }

    #[test]
    fn only_plain_image_types_become_data_urls() {
        assert!(is_image_mime("image/png"));
        assert!(is_image_mime("image/svg+xml"));
        assert!(!is_image_mime("image/"));
        assert!(!is_image_mime("text/html"));
        assert!(!is_image_mime("image/png;base64,PHN2Zz4="));
        assert!(!is_image_mime("image/png\"><script>"));
        assert_eq!(data_url("image/png", b"hi"), "data:image/png;base64,aGk=");
    }

    #[test]
    fn previews_load_small_images_on_their_own_and_bigger_ones_on_click() {
        assert!(preview_limit(false) < preview_limit(true));
        assert!(fits(Some(AUTO_PREVIEW_BYTES), preview_limit(false), "big").is_ok());
        assert_eq!(fits(Some(AUTO_PREVIEW_BYTES + 1), preview_limit(false), "big").unwrap_err(), "big");
        assert!(fits(None, 1, "big").is_ok());
    }

    #[test]
    fn only_file_fetches_return_bytes() {
        assert_eq!(byte_limit("get_attachment_video").map(|l| l.0), Some(MAX_VIDEO_BYTES));
        assert_eq!(byte_limit("download_attachment").map(|l| l.0), Some(MAX_DOWNLOAD_BYTES));
        assert!(byte_limit("get_voice_message").is_some());
        assert!(byte_limit("send_dm").is_none());
    }
}
