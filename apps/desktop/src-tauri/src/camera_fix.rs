use std::path::Path;

pub const PLUGIN_DIR: &str = "/usr/lib/commy/gstreamer-1.0";
pub const PLUGIN_FILE: &str = "libgstcommycamerafix.so";
pub const TRACER: &str = "commycamerafix";

pub fn with_entry(existing: Option<&str>, entry: &str, separator: char) -> Option<String> {
    match existing.map(str::trim).filter(|s| !s.is_empty()) {
        None => Some(entry.to_string()),
        Some(current) if current.split(separator).any(|part| part.trim() == entry) => None,
        Some(current) => Some(format!("{current}{separator}{entry}")),
    }
}

pub fn plugin_path_var(versioned_is_set: bool) -> &'static str {
    if versioned_is_set {
        "GST_PLUGIN_PATH_1_0"
    } else {
        "GST_PLUGIN_PATH"
    }
}

fn add_to_var(name: &str, entry: &str, separator: char) {
    if let Some(value) = with_entry(std::env::var(name).ok().as_deref(), entry, separator) {
        std::env::set_var(name, value);
    }
}

pub fn apply_before_any_window() {
    let in_appimage = std::env::var_os("APPIMAGE").is_some();
    if in_appimage || !Path::new(PLUGIN_DIR).join(PLUGIN_FILE).is_file() {
        return;
    }
    let path_var = plugin_path_var(std::env::var_os("GST_PLUGIN_PATH_1_0").is_some());
    add_to_var(path_var, PLUGIN_DIR, ':');
    add_to_var("GST_TRACERS", TRACER, ';');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_an_empty_list() {
        assert_eq!(with_entry(None, TRACER, ';').as_deref(), Some(TRACER));
        assert_eq!(with_entry(Some("  "), TRACER, ';').as_deref(), Some(TRACER));
    }

    #[test]
    fn appends_to_what_the_person_set() {
        assert_eq!(
            with_entry(Some("latency(flags=pipeline)"), TRACER, ';').as_deref(),
            Some("latency(flags=pipeline);commycamerafix")
        );
        assert_eq!(
            with_entry(Some("/opt/gst"), PLUGIN_DIR, ':').as_deref(),
            Some("/opt/gst:/usr/lib/commy/gstreamer-1.0")
        );
    }

    #[test]
    fn leaves_a_list_that_already_has_it() {
        assert_eq!(with_entry(Some("leaks;commycamerafix"), TRACER, ';'), None);
        assert_eq!(with_entry(Some("/usr/lib/commy/gstreamer-1.0:/opt/gst"), PLUGIN_DIR, ':'), None);
    }

    #[test]
    fn follows_the_versioned_plugin_path_when_set() {
        assert_eq!(plugin_path_var(true), "GST_PLUGIN_PATH_1_0");
        assert_eq!(plugin_path_var(false), "GST_PLUGIN_PATH");
    }
}
