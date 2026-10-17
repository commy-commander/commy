//! Running more than one copy on a machine for testing. `COMMY_DATA_DIR`
//! gives a copy its own identity; with it set, the webview storage moves
//! into that folder too, and `COMMY_INSTANCE_LABEL` names the window.

use std::path::PathBuf;

/// Folder inside the data dir that holds the webview's storage.
pub const WEBVIEW_DIR: &str = "webview";

/// Where the webview should keep its storage, when it should move: only
/// for a custom data dir, and never over an explicit WebView2 setting.
pub fn webview_data_dir(data_dir: Option<&str>, existing: Option<&str>) -> Option<PathBuf> {
    if existing.is_some_and(|s| !s.trim().is_empty()) {
        return None;
    }
    let dir = data_dir.map(str::trim).filter(|s| !s.is_empty())?;
    Some(PathBuf::from(dir).join(WEBVIEW_DIR))
}

/// Points WebView2 at the data dir, so two copies do not share one
/// browser profile. Must run before any window is created.
pub fn apply_webview_data_dir() {
    let target = webview_data_dir(
        std::env::var("COMMY_DATA_DIR").ok().as_deref(),
        std::env::var("WEBVIEW2_USER_DATA_FOLDER").ok().as_deref(),
    );
    if let Some(dir) = target {
        std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", dir);
    }
}

pub const TEST_BUILD_DIR: &str = "test-build";

pub fn test_build_data_dir(test_build: bool, data_dir: Option<&str>, default: Option<PathBuf>) -> Option<PathBuf> {
    if !test_build || data_dir.is_some_and(|s| !s.trim().is_empty()) {
        return None;
    }
    default.map(|dir| dir.join(TEST_BUILD_DIR))
}

pub fn apply_test_build_data_dir() {
    let target = test_build_data_dir(
        option_env!("COMMY_TEST_BUILD").is_some(),
        std::env::var("COMMY_DATA_DIR").ok().as_deref(),
        crate::invite_link::default_data_dir(),
    );
    if let Some(dir) = target {
        std::env::set_var("COMMY_DATA_DIR", dir);
    }
}

/// The main window title, or None to keep the default.
pub fn window_title(test_build: bool, label: Option<&str>) -> Option<String> {
    let label = label.map(str::trim).filter(|s| !s.is_empty());
    match (test_build, label) {
        (false, None) => None,
        (true, None) => Some("Commy (test build)".to_string()),
        (false, Some(label)) => Some(format!("Commy ({label})")),
        (true, Some(label)) => Some(format!("Commy (test build, {label})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builds_get_their_own_data_dir() {
        let home = Some(PathBuf::from("appdata/chat.commy.desktop"));
        assert_eq!(test_build_data_dir(false, None, home.clone()), None);
        assert_eq!(
            test_build_data_dir(true, None, home.clone()),
            Some(PathBuf::from("appdata/chat.commy.desktop").join("test-build"))
        );
        assert_eq!(test_build_data_dir(true, Some("bench/A"), home.clone()), None);
        assert_eq!(
            test_build_data_dir(true, Some(" "), home),
            Some(PathBuf::from("appdata/chat.commy.desktop").join("test-build"))
        );
        assert_eq!(test_build_data_dir(true, None, None), None);
    }

    #[test]
    fn webview_moves_only_with_a_custom_data_dir() {
        assert_eq!(webview_data_dir(None, None), None);
        assert_eq!(webview_data_dir(Some("  "), None), None);
        assert_eq!(webview_data_dir(Some("bench/A"), None), Some(PathBuf::from("bench/A").join("webview")));
    }

    #[test]
    fn explicit_webview_setting_wins() {
        assert_eq!(webview_data_dir(Some("bench/A"), Some("elsewhere")), None);
        assert_eq!(webview_data_dir(Some("bench/A"), Some("")), Some(PathBuf::from("bench/A").join("webview")));
    }

    #[test]
    fn titles() {
        assert_eq!(window_title(false, None), None);
        assert_eq!(window_title(false, Some(" ")), None);
        assert_eq!(window_title(true, None).as_deref(), Some("Commy (test build)"));
        assert_eq!(window_title(false, Some("A")).as_deref(), Some("Commy (A)"));
        assert_eq!(window_title(true, Some("B")).as_deref(), Some("Commy (test build, B)"));
    }
}
