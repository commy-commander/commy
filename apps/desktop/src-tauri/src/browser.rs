use tauri::AppHandle;

#[cfg(desktop)]
pub fn open(_handle: &AppHandle, url: &str) -> Result<(), String> {
    open::that_detached(url).map_err(|e| e.to_string())
}

#[cfg(mobile)]
pub fn open(handle: &AppHandle, url: &str) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    handle.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}
