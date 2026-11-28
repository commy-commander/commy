use std::path::{Path, PathBuf};

const SANDBOX_INFO: &str = "/.flatpak-info";

pub fn inside() -> bool {
    Path::new(SANDBOX_INFO).is_file()
}

pub fn shared_with_host(runtime_dir: Option<&str>, app_id: Option<&str>) -> Option<PathBuf> {
    let runtime_dir = runtime_dir.filter(|dir| dir.starts_with('/'))?;
    let app_id = app_id.map(str::trim).filter(|id| !id.is_empty() && !id.contains('/'))?;
    Some(Path::new(runtime_dir).join("app").join(app_id))
}

pub fn tray_icon_dir() -> Option<PathBuf> {
    if !inside() {
        return None;
    }
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").ok();
    let app_id = std::env::var("FLATPAK_ID").ok();
    shared_with_host(runtime_dir.as_deref(), app_id.as_deref()).map(|dir| dir.join("tray-icon"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_sees_the_app_folder_in_the_runtime_dir() {
        assert_eq!(
            shared_with_host(Some("/run/user/1000"), Some("chat.commy.desktop")),
            Some(PathBuf::from("/run/user/1000/app/chat.commy.desktop"))
        );
    }

    #[test]
    fn nothing_is_shared_without_a_usable_runtime_dir_or_app_id() {
        assert_eq!(shared_with_host(None, Some("chat.commy.desktop")), None);
        assert_eq!(shared_with_host(Some("relative/dir"), Some("chat.commy.desktop")), None);
        assert_eq!(shared_with_host(Some("/run/user/1000"), None), None);
        assert_eq!(shared_with_host(Some("/run/user/1000"), Some("  ")), None);
        assert_eq!(shared_with_host(Some("/run/user/1000"), Some("../escape")), None);
    }
}
