use crate::expressions::image_mime;

pub const MAX_SAVED_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const IMAGE_NAME_HEADER: &str = "x-commy-name";
const FALLBACK_NAME: &str = "image";

fn extensions_for(mime: &str) -> &'static [&'static str] {
    match mime {
        "image/png" => &["png"],
        "image/jpeg" => &["jpg", "jpeg"],
        "image/gif" => &["gif"],
        "image/webp" => &["webp"],
        _ => &[],
    }
}

pub fn checked_image(bytes: &[u8]) -> Result<&'static str, String> {
    if bytes.len() > MAX_SAVED_IMAGE_BYTES {
        return Err("that image is too large to save".into());
    }
    image_mime(bytes).ok_or_else(|| "that is not a picture Commy can save".to_string())
}

fn decoded_name(raw: &str) -> String {
    reqwest::Url::parse(&format!("http://name.invalid/?n={raw}"))
        .ok()
        .and_then(|url| url.query_pairs().find(|(key, _)| key == "n").map(|(_, value)| value.into_owned()))
        .unwrap_or_default()
}

pub fn image_name(raw: Option<&str>, mime: &str) -> String {
    let decoded = raw.map(decoded_name).unwrap_or_default();
    let name = match decoded.trim() {
        "" => FALLBACK_NAME,
        trimmed => trimmed,
    };
    let extensions = extensions_for(mime);
    let has_extension = name
        .rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.is_empty() && extensions.iter().any(|known| known.eq_ignore_ascii_case(ext)));
    match extensions.first() {
        Some(ext) if !has_extension => format!("{name}.{ext}"),
        _ => name.to_string(),
    }
}

#[tauri::command]
pub async fn save_image(handle: tauri::AppHandle, request: tauri::ipc::Request<'_>) -> Result<Option<String>, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected the picture".into());
    };
    let mime = checked_image(bytes)?;
    let raw_name = request.headers().get(IMAGE_NAME_HEADER).and_then(|value| value.to_str().ok());
    let name = crate::safe_file_name(&image_name(raw_name, mime));
    crate::picker::save_file(&handle, &name, bytes.clone()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    const GIF: &[u8] = b"GIF89a....";

    #[test]
    fn only_pictures_can_be_saved() {
        assert_eq!(checked_image(PNG), Ok("image/png"));
        assert_eq!(checked_image(GIF), Ok("image/gif"));
        assert!(checked_image(b"<html>").is_err());
        assert!(checked_image(&[]).is_err());
    }

    #[test]
    fn a_picture_that_is_too_big_is_refused() {
        let mut big = PNG.to_vec();
        big.resize(MAX_SAVED_IMAGE_BYTES + 1, 0);
        assert_eq!(checked_image(&big), Err("that image is too large to save".to_string()));
    }

    #[test]
    fn the_name_from_the_chat_is_kept_and_given_the_right_ending() {
        assert_eq!(image_name(Some("cat%20photo.png"), "image/png"), "cat photo.png");
        assert_eq!(image_name(Some("giphy.GIF"), "image/gif"), "giphy.GIF");
        assert_eq!(image_name(Some("holiday.jpeg"), "image/jpeg"), "holiday.jpeg");
        assert_eq!(image_name(Some("photo"), "image/jpeg"), "photo.jpg");
        assert_eq!(image_name(Some("notes.txt"), "image/webp"), "notes.txt.webp");
        assert_eq!(image_name(Some("caf%C3%A9.png"), "image/png"), "café.png");
    }

    #[test]
    fn a_missing_name_becomes_image() {
        assert_eq!(image_name(None, "image/png"), "image.png");
        assert_eq!(image_name(Some("%20%20"), "image/gif"), "image.gif");
        assert_eq!(image_name(Some(".png"), "image/png"), ".png.png");
    }
}
