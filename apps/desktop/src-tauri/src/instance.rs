use std::path::PathBuf;

pub const WEBVIEW_DIR: &str = "webview";

pub fn webview_data_dir(data_dir: Option<&str>, existing: Option<&str>) -> Option<PathBuf> {
    if existing.is_some_and(|s| !s.trim().is_empty()) {
        return None;
    }
    let dir = data_dir.map(str::trim).filter(|s| !s.is_empty())?;
    Some(PathBuf::from(dir).join(WEBVIEW_DIR))
}

pub fn apply_webview_data_dir_before_any_window() {
    let target = webview_data_dir(
        std::env::var("COMMY_DATA_DIR").ok().as_deref(),
        std::env::var("WEBVIEW2_USER_DATA_FOLDER").ok().as_deref(),
    );
    if let Some(dir) = target {
        std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", dir);
    }
}

pub const TEST_BUILD_DIR: &str = "test-build";

pub const TEST_BUILD: bool = option_env!("COMMY_TEST_BUILD").is_some();

pub fn test_build_data_dir(test_build: bool, data_dir: Option<&str>, default: Option<PathBuf>) -> Option<PathBuf> {
    if !test_build || data_dir.is_some_and(|s| !s.trim().is_empty()) {
        return None;
    }
    default.map(|dir| dir.join(TEST_BUILD_DIR))
}

pub fn apply_test_build_data_dir() {
    let target = test_build_data_dir(
        TEST_BUILD,
        std::env::var("COMMY_DATA_DIR").ok().as_deref(),
        crate::invite_link::default_data_dir(),
    );
    if let Some(dir) = target {
        std::env::set_var("COMMY_DATA_DIR", dir);
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

pub fn autostart_allowed(test_build: bool, label: Option<&str>, data_dir: Option<&str>) -> bool {
    let set = |v: Option<&str>| v.is_some_and(|s| !s.trim().is_empty());
    !test_build && !set(label) && !set(data_dir)
}

pub fn autostart_allowed_here() -> bool {
    autostart_allowed(
        TEST_BUILD,
        std::env::var("COMMY_INSTANCE_LABEL").ok().as_deref(),
        std::env::var("COMMY_DATA_DIR").ok().as_deref(),
    )
}

pub fn server_window_title(base: &str, server: Option<&str>) -> String {
    let name: String = server.unwrap_or("").chars().filter(|c| !c.is_control()).take(80).collect();
    let name = name.trim();
    if name.is_empty() {
        base.to_string()
    } else {
        format!("{name} - {base}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_title_leads_with_the_active_server() {
        assert_eq!(server_window_title("Commy", None), "Commy");
        assert_eq!(server_window_title("Commy", Some("  ")), "Commy");
        assert_eq!(server_window_title("Commy", Some("Pixel Club")), "Pixel Club - Commy");
        assert_eq!(server_window_title("Commy (test build)", Some("Pixel\nClub")), "PixelClub - Commy (test build)");
        let long = "x".repeat(200);
        assert_eq!(server_window_title("Commy", Some(&long)).chars().count(), 80 + " - Commy".len());
    }

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

    #[test]
    fn autostart_is_refused_for_test_builds_and_labelled_or_custom_data_instances() {
        assert!(autostart_allowed(false, None, None));
        assert!(autostart_allowed(false, Some(" "), Some("")));
        assert!(!autostart_allowed(true, None, None));
        assert!(!autostart_allowed(false, Some("bench"), None));
        assert!(!autostart_allowed(false, None, Some("C:\\bench")));
    }
}
