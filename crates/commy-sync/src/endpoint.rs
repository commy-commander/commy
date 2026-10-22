fn host_part(url: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next()?,
        None => authority.split(':').next()?,
    };
    Some((scheme, host))
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

pub fn accepted_url(server: &str, offered: Option<&str>) -> Option<String> {
    let offered = offered?.trim().trim_end_matches('/');
    let (scheme, host) = host_part(offered)?;
    let (_, server_host) = host_part(server)?;
    let scheme_ok = scheme == "wss" || (scheme == "ws" && is_loopback(host));
    (scheme_ok && host.eq_ignore_ascii_case(server_host)).then(|| offered.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_secure_addresses_on_the_account_servers_host_are_used() {
        let ok = |server: &str, offered: &str| accepted_url(server, Some(offered));
        assert_eq!(ok("https://commy.chat", "wss://commy.chat/sync").as_deref(), Some("wss://commy.chat/sync"));
        assert_eq!(ok("https://commy.chat", "wss://commy.chat/sync/").as_deref(), Some("wss://commy.chat/sync"));
        assert_eq!(ok("https://Commy.Chat", "wss://commy.chat:8443/sync").as_deref(), Some("wss://commy.chat:8443/sync"));
        assert_eq!(ok("https://commy.chat", "ws://commy.chat/sync"), None, "no plain sockets off this machine");
        assert_eq!(ok("https://commy.chat", "wss://elsewhere.example/sync"), None);
        assert_eq!(ok("https://commy.chat", "wss://commy.chat.evil.example/sync"), None);
        assert_eq!(ok("https://commy.chat", "wss://user@commy.chat/sync"), None);
        assert_eq!(ok("https://commy.chat", "https://commy.chat/sync"), None);
        assert_eq!(ok("http://localhost:3001", "ws://localhost:3002/sync").as_deref(), Some("ws://localhost:3002/sync"));
        assert_eq!(ok("http://[::1]:3001", "ws://[::1]:3002/sync").as_deref(), Some("ws://[::1]:3002/sync"));
        assert_eq!(accepted_url("https://commy.chat", None), None);
        assert_eq!(ok("https://commy.chat", " "), None);
    }
}
