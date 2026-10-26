use url::{Host, Url};

const PUBLIC_TLDS: &str = include_str!("public_tlds.txt");
const MAX_DOMAIN_LEN: usize = 253;
const MAX_LABEL_LEN: usize = 63;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerAddressError {
    NotHttps,
    HasLogin,
    IpAddress,
    NotADomain,
    PrivateEnding,
}

impl ServerAddressError {
    pub fn message_key(self) -> &'static str {
        match self {
            Self::NotHttps => "signin.server.bad.https",
            Self::HasLogin => "signin.server.bad.login",
            Self::IpAddress => "signin.server.bad.ip",
            Self::NotADomain => "signin.server.bad.domain",
            Self::PrivateEnding => "signin.server.bad.ending",
        }
    }
}

pub fn public_server_origin(input: &str) -> Result<String, ServerAddressError> {
    let typed = input.trim();
    let with_scheme = match typed.split_once("://") {
        Some((scheme, _)) if scheme.eq_ignore_ascii_case("https") => typed.to_string(),
        Some(_) => return Err(ServerAddressError::NotHttps),
        None if has_other_scheme(typed) => return Err(ServerAddressError::NotHttps),
        None => format!("https://{typed}"),
    };
    let url = Url::parse(&with_scheme).map_err(|_| ServerAddressError::NotADomain)?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ServerAddressError::HasLogin);
    }
    let domain = match url.host() {
        Some(Host::Domain(domain)) => domain.strip_suffix('.').unwrap_or(domain).to_ascii_lowercase(),
        Some(Host::Ipv4(_) | Host::Ipv6(_)) => return Err(ServerAddressError::IpAddress),
        None => return Err(ServerAddressError::NotADomain),
    };
    let labels: Vec<&str> = domain.split('.').collect();
    if labels.len() < 2 || domain.len() > MAX_DOMAIN_LEN || !labels.iter().all(|label| is_dns_label(label)) {
        return Err(ServerAddressError::NotADomain);
    }
    if !is_public_tld(labels[labels.len() - 1]) {
        return Err(ServerAddressError::PrivateEnding);
    }
    Ok(match url.port() {
        Some(port) => format!("https://{domain}:{port}"),
        None => format!("https://{domain}"),
    })
}

fn has_other_scheme(typed: &str) -> bool {
    let Some((scheme, rest)) = typed.split_once(':') else { return false };
    let looks_like_scheme = scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    let port_follows = rest.split(['/', '?', '#']).next().is_some_and(|port| !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()));
    looks_like_scheme && !scheme.contains('.') && !port_follows
}

fn is_dns_label(label: &str) -> bool {
    (1..=MAX_LABEL_LEN).contains(&label.len())
        && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !label.starts_with('-')
        && !label.ends_with('-')
}

fn is_public_tld(tld: &str) -> bool {
    PUBLIC_TLDS.lines().any(|known| known == tld)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ServerAddressError::*;

    #[test]
    fn keeps_the_https_origin_of_a_public_domain() {
        assert_eq!(public_server_origin("  chat.example.com  "), Ok("https://chat.example.com".into()));
        assert_eq!(public_server_origin("HTTPS://Chat.Example.COM/sign-in?x=1#y"), Ok("https://chat.example.com".into()));
        assert_eq!(public_server_origin("https://chat.example.org:8443"), Ok("https://chat.example.org:8443".into()));
        assert_eq!(public_server_origin("chat.example.org:8443"), Ok("https://chat.example.org:8443".into()));
        assert_eq!(public_server_origin("https://commy.chat/"), Ok("https://commy.chat".into()));
        assert_eq!(public_server_origin("https://chat.example.com."), Ok("https://chat.example.com".into()));
        assert_eq!(public_server_origin("https://пример.рф"), Ok("https://xn--e1afmkfd.xn--p1ai".into()));
    }

    #[test]
    fn refuses_anything_but_https() {
        for input in ["http://chat.example.com", "ftp://chat.example.com", "ws://chat.example.com", "javascript:alert(1)", "file:///etc/passwd", "mailto:a@example.com"] {
            assert_eq!(public_server_origin(input), Err(NotHttps), "{input}");
        }
    }

    #[test]
    fn refuses_ip_addresses() {
        for input in ["https://1.2.3.4", "10.0.0.5", "https://[::1]", "[2001:db8::1]:443", "https://0x7f.1"] {
            assert_eq!(public_server_origin(input), Err(IpAddress), "{input}");
        }
    }

    #[test]
    fn refuses_names_that_are_not_public_domains() {
        for input in ["", "localhost", "https://localhost:3000", "commy", "https://chat_example.com", "https://-chat.example.com", "https://chat..example.com", "https://"] {
            assert_eq!(public_server_origin(input), Err(NotADomain), "{input}");
        }
        for input in ["https://commy.localhost", "server.local", "box.lan", "chat.example.test", "auth.internal", "x.onion", "chat.example.invalid", "chat.example.comm", "in-addr.arpa"] {
            assert_eq!(public_server_origin(input), Err(PrivateEnding), "{input}");
        }
    }

    #[test]
    fn refuses_a_login_in_the_address() {
        assert_eq!(public_server_origin("https://user:pw@chat.example.com"), Err(HasLogin));
        assert_eq!(public_server_origin("user@chat.example.com"), Err(HasLogin));
    }

    #[test]
    fn every_error_has_a_sign_in_message() {
        for error in [NotHttps, HasLogin, IpAddress, NotADomain, PrivateEnding] {
            assert!(error.message_key().starts_with("signin.server.bad."));
        }
    }
}
