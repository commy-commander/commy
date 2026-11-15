pub const MAX_SYNC_HOST_CHARS: usize = 200;
pub const SOCKET_PATH: &str = "/sync";

fn strip_scheme(text: &str) -> Option<&str> {
    match text.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("wss") => Some(rest),
        Some(_) => None,
        None => Some(text),
    }
}

pub fn clean_sync_host(input: &str) -> Option<String> {
    let text = input.trim();
    if text.is_empty() || text.chars().count() > MAX_SYNC_HOST_CHARS {
        return None;
    }
    let rest = strip_scheme(text)?.trim_end_matches('/');
    let host = match rest.split_once('/') {
        Some((host, path)) if path.trim_end_matches('/') == SOCKET_PATH.trim_start_matches('/') => host,
        Some(_) => return None,
        None => rest,
    };
    crate::file_store::host_ok(host).then(|| format!("https://{}", host.to_ascii_lowercase()))
}

pub fn is_sync_host(origin: &str) -> bool {
    clean_sync_host(origin).as_deref() == Some(origin)
}

fn host_of(origin: &str) -> Option<&str> {
    is_sync_host(origin).then(|| origin.trim_start_matches("https://"))
}

pub fn socket_url(origin: &str) -> Option<String> {
    Some(format!("wss://{}{SOCKET_PATH}", host_of(origin)?))
}

pub fn display_host(origin: &str) -> Option<&str> {
    host_of(origin)
}

pub fn chosen_socket(chosen: Option<&str>, account_socket: Option<&str>) -> Option<String> {
    match chosen {
        Some(origin) => socket_url(origin),
        None => account_socket.map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_way_of_typing_a_host_becomes_one_form() {
        let want = Some("https://sync.example.org".to_string());
        for input in [
            "sync.example.org",
            " Sync.Example.org ",
            "https://sync.example.org",
            "https://sync.example.org/",
            "wss://sync.example.org/sync",
            "WSS://sync.example.org/sync/",
            "https://sync.example.org/sync",
        ] {
            assert_eq!(clean_sync_host(input), want, "{input}");
        }
        assert!(is_sync_host("https://sync.example.org"));
        assert!(!is_sync_host("sync.example.org"));
        assert!(!is_sync_host("https://Sync.example.org"));
    }

    #[test]
    fn only_a_public_secure_host_is_accepted() {
        for input in [
            "",
            "   ",
            "http://sync.example.org",
            "ws://sync.example.org/sync",
            "ftp://sync.example.org",
            "https://user@sync.example.org",
            "https://sync.example.org:8443",
            "https://sync.example.org/other",
            "https://sync.example.org/sync/more",
            "https://sync.example.org/?x=1",
            "https://localhost",
            "https://box.local",
            "https://10.0.0.4",
            "https://[::1]",
            "https://intranet",
            "https://-bad.example.org",
            &format!("https://{}.org", "a".repeat(MAX_SYNC_HOST_CHARS)),
        ] {
            assert_eq!(clean_sync_host(input), None, "{input}");
        }
    }

    #[test]
    fn a_host_gives_its_socket_address() {
        let origin = "https://sync.example.org";
        assert_eq!(socket_url(origin).as_deref(), Some("wss://sync.example.org/sync"));
        assert_eq!(display_host(origin), Some("sync.example.org"));
        assert_eq!(socket_url("https://sync.example.org/"), None, "only the clean form is trusted");
        assert_eq!(socket_url("http://sync.example.org"), None);
    }

    #[test]
    fn no_chosen_host_means_the_account_servers_sync() {
        let account = Some("wss://commy.chat/sync");
        assert_eq!(chosen_socket(None, account).as_deref(), Some("wss://commy.chat/sync"));
        assert_eq!(chosen_socket(Some("https://sync.example.org"), account).as_deref(), Some("wss://sync.example.org/sync"));
        assert_eq!(chosen_socket(Some("https://sync.example.org"), None).as_deref(), Some("wss://sync.example.org/sync"));
        assert_eq!(chosen_socket(None, None), None);
        assert_eq!(chosen_socket(Some("not a host"), account), None, "a bad host never falls back to someone else's service");
    }
}
