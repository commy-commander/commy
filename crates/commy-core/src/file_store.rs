pub const MAX_FILE_STORE_CHARS: usize = 200;

fn private_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".lan")
        || host.parse::<std::net::Ipv4Addr>().is_ok()
        || host.starts_with('[')
        || !host.contains('.')
}

fn host_ok(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| !label.is_empty() && label.len() <= 63 && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !label.starts_with('-') && !label.ends_with('-'))
        && !private_host(host)
}

fn path_ok(path: &str) -> bool {
    path.split('/').all(|part| part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) && part != "." && part != "..")
}

pub fn clean_file_store(input: &str) -> Option<String> {
    let text = input.trim();
    if text.chars().count() > MAX_FILE_STORE_CHARS {
        return None;
    }
    let rest = text.strip_prefix("https://")?;
    let rest = rest.trim_end_matches('/');
    let (host, path) = match rest.split_once('/') {
        Some((host, path)) => (host, Some(path)),
        None => (rest, None),
    };
    if !host_ok(host) || path.is_some_and(|p| p.is_empty() || !path_ok(p)) {
        return None;
    }
    let host = host.to_ascii_lowercase();
    Some(match path {
        Some(path) => format!("https://{host}/{path}"),
        None => format!("https://{host}"),
    })
}

pub fn is_file_store(url: &str) -> bool {
    clean_file_store(url).as_deref() == Some(url)
}

pub fn reaches_in_browser(url: &str) -> bool {
    url.strip_prefix("https://").and_then(|rest| rest.split('/').next()).is_some_and(|host| host.ends_with(".workers.dev"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_https_address_becomes_one_form() {
        let want = Some("https://commy-files.rockets.workers.dev".to_string());
        for input in [" https://commy-files.rockets.workers.dev ", "https://commy-files.rockets.workers.dev/", "https://Commy-Files.Rockets.workers.dev"] {
            assert_eq!(clean_file_store(input), want, "{input}");
        }
        assert_eq!(clean_file_store("https://files.example.org/commy/").as_deref(), Some("https://files.example.org/commy"));
        assert!(is_file_store("https://commy-files.rockets.workers.dev"));
        assert!(!is_file_store("https://commy-files.rockets.workers.dev/"));
    }

    #[test]
    fn only_a_public_https_address_is_accepted() {
        for input in [
            "",
            "http://files.example.org",
            "files.example.org",
            "javascript:alert(1)",
            "https://user@files.example.org",
            "https://files.example.org:8443",
            "https://files.example.org/x?y=1",
            "https://files.example.org/x#y",
            "https://files.example.org/../x",
            "https://localhost",
            "https://nas.local",
            "https://192.168.1.10",
            "https://[::1]",
            "https://intranet",
            "https://-bad.example.org",
        ] {
            assert_eq!(clean_file_store(input), None, "{input}");
        }
        assert_eq!(clean_file_store(&format!("https://{}.example.org", "a".repeat(MAX_FILE_STORE_CHARS))), None);
    }

    #[test]
    fn browsers_reach_workers_dev_addresses() {
        assert!(reaches_in_browser("https://commy-files.rockets.workers.dev"));
        assert!(!reaches_in_browser("https://files.example.org"));
        assert!(!reaches_in_browser("https://workers.dev.example.org"));
    }
}
