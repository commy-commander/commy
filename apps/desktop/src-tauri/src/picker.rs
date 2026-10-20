use tauri::AppHandle;

pub struct Picked {
    pub name: String,
    pub bytes: Vec<u8>,
}

pub struct Filter<'a> {
    pub label: &'a str,
    pub extensions: &'a [&'a str],
}

fn err_to_string(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[cfg(desktop)]
pub async fn pick_file(
    _handle: &AppHandle,
    title: &str,
    filter: Option<Filter<'_>>,
    max_bytes: u64,
    too_large: &str,
) -> Result<Option<Picked>, String> {
    let mut dialog = rfd::AsyncFileDialog::new().set_title(title);
    if let Some(filter) = filter {
        dialog = dialog.add_filter(filter.label, filter.extensions);
    }
    let Some(picked) = dialog.pick_file().await else {
        return Ok(None);
    };
    let path = picked.path();
    if tokio::fs::metadata(path).await.map_err(err_to_string)?.len() > max_bytes {
        return Err(too_large.into());
    }
    let bytes = tokio::fs::read(path).await.map_err(err_to_string)?;
    Ok(Some(Picked { name: picked.file_name(), bytes }))
}

#[cfg(desktop)]
pub async fn save_file(_handle: &AppHandle, name: &str, bytes: Vec<u8>) -> Result<Option<String>, String> {
    let Some(target) = rfd::AsyncFileDialog::new().set_file_name(name).save_file().await else {
        return Ok(None);
    };
    let path = target.path().to_path_buf();
    tokio::fs::write(&path, bytes).await.map_err(err_to_string)?;
    Ok(Some(path.display().to_string()))
}

#[cfg(mobile)]
pub async fn pick_file(
    handle: &AppHandle,
    title: &str,
    filter: Option<Filter<'_>>,
    max_bytes: u64,
    too_large: &str,
) -> Result<Option<Picked>, String> {
    use tauri_plugin_dialog::DialogExt;
    let mut dialog = handle.dialog().file().set_title(title);
    if let Some(filter) = filter {
        dialog = dialog.add_filter(filter.label, filter.extensions);
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    dialog.pick_file(move |picked| {
        let _ = tx.send(picked);
    });
    let Some(path) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let name = picked_name(&path);
    let bytes = read_picked(handle, path, max_bytes)?.ok_or_else(|| too_large.to_string())?;
    Ok(Some(Picked { name: named_for_contents(name, &bytes), bytes }))
}

#[cfg(mobile)]
pub async fn save_file(handle: &AppHandle, name: &str, bytes: Vec<u8>) -> Result<Option<String>, String> {
    use std::io::Write;
    use tauri_plugin_dialog::DialogExt;
    use tauri_plugin_fs::{FsExt, OpenOptions};
    let (tx, rx) = tokio::sync::oneshot::channel();
    handle.dialog().file().set_file_name(name).save_file(move |picked| {
        let _ = tx.send(picked);
    });
    let Some(path) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    let mut file = handle.fs().open(path.clone(), options).map_err(err_to_string)?;
    file.write_all(&bytes).map_err(err_to_string)?;
    Ok(Some(picked_name(&path)))
}

#[cfg(mobile)]
fn read_picked(handle: &AppHandle, path: tauri_plugin_fs::FilePath, max_bytes: u64) -> Result<Option<Vec<u8>>, String> {
    use std::io::Read;
    use tauri_plugin_fs::{FsExt, OpenOptions};
    let mut options = OpenOptions::new();
    options.read(true);
    let file = handle.fs().open(path.clone(), options).map_err(err_to_string)?;
    let mut bytes = Vec::new();
    let read = file.take(max_bytes + 1).read_to_end(&mut bytes).map_err(err_to_string);
    #[cfg(target_os = "ios")]
    let _ = handle.fs().stop_accessing_security_scoped_resource(path);
    read?;
    Ok((bytes.len() as u64 <= max_bytes).then_some(bytes))
}

#[cfg(mobile)]
fn picked_name(path: &tauri_plugin_fs::FilePath) -> String {
    let raw = match path {
        tauri_plugin_fs::FilePath::Path(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()),
        tauri_plugin_fs::FilePath::Url(url) => url.path_segments().and_then(|mut parts| parts.next_back()).map(str::to_string),
    };
    last_name_part(raw.as_deref().unwrap_or(""))
}

#[cfg(any(mobile, test))]
pub fn last_name_part(raw: &str) -> String {
    let decoded = raw.replace("%20", " ").replace("%3A", ":").replace("%3a", ":").replace("%2F", "/").replace("%2f", "/");
    let name = decoded.rsplit([':', '/']).next().unwrap_or("").trim();
    if name.is_empty() {
        "file".to_string()
    } else {
        name.to_string()
    }
}

#[cfg(any(mobile, test))]
pub fn named_for_contents(name: String, bytes: &[u8]) -> String {
    let has_extension = name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && !ext.is_empty());
    match sniffed_extension(bytes) {
        Some(ext) if !has_extension => format!("{name}.{ext}"),
        _ => name,
    }
}

#[cfg(any(mobile, test))]
fn sniffed_extension(bytes: &[u8]) -> Option<&'static str> {
    if let Ok(format) = image::guess_format(bytes) {
        return format.extensions_str().first().copied();
    }
    match bytes.get(4..12) {
        Some(b"ftypheic") | Some(b"ftypheix") | Some(b"ftypmif1") => return Some("heic"),
        Some(brand) if brand.starts_with(b"ftypqt") => return Some("mov"),
        Some(brand) if brand.starts_with(b"ftyp") => return Some("mp4"),
        _ => {}
    }
    if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some("webm");
    }
    if bytes.starts_with(b"OggS") {
        return Some("ogg");
    }
    if bytes.starts_with(b"ID3") || bytes.starts_with(&[0xFF, 0xFB]) {
        return Some("mp3");
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") {
        return Some("wav");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_keep_their_own_extension() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];
        assert_eq!(named_for_contents("cat.jpg".into(), &png), "cat.jpg");
        assert_eq!(named_for_contents("notes".into(), b"plain words"), "notes");
    }

    #[test]
    fn extensionless_names_get_one_from_the_contents() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];
        assert_eq!(named_for_contents("1000000033".into(), &png), "1000000033.png");
        assert_eq!(named_for_contents("clip".into(), b"\0\0\0\x18ftypmp42rest"), "clip.mp4");
        assert_eq!(named_for_contents("clip".into(), b"\0\0\0\x14ftypqt  rest"), "clip.mov");
        assert_eq!(named_for_contents("photo".into(), b"\0\0\0\x18ftypheicrest"), "photo.heic");
        assert_eq!(named_for_contents("beep".into(), b"RIFF\0\0\0\0WAVEfmt "), "beep.wav");
    }

    #[test]
    fn content_uri_segments_become_plain_names() {
        assert_eq!(last_name_part("image%3A1234"), "1234");
        assert_eq!(last_name_part("primary%3ADownload%2Fholiday%20photo.jpg"), "holiday photo.jpg");
        assert_eq!(last_name_part("cat.png"), "cat.png");
        assert_eq!(last_name_part(""), "file");
    }
}
