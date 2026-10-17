use std::path::PathBuf;

pub const WEBVIEW_DIR: &str = "webview";

pub fn webview_data_dir(data_dir: Option<&str>, existing: Option<&str>) -> Option<PathBuf> {
    if existing.is_some_and(|s| !s.trim().is_empty()) {
        return None;
    }
    let dir = data_dir.map(str::trim).filter(|s| !s.is_empty())?;
    Some(PathBuf::from(dir).join(WEBVIEW_DIR))
}

pub fn apply_webview_data_dir() {
    let target = webview_data_dir(
        std::env::var("COMMY_DATA_DIR").ok().as_deref(),
        std::env::var("WEBVIEW2_USER_DATA_FOLDER").ok().as_deref(),
    );
    if let Some(dir) = target {
        std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", dir);
    }
}

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
