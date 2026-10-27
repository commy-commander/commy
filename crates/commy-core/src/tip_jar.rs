pub const TIP_JAR_HOST: &str = "ko-fi.com";
pub const MAX_PAGE_CHARS: usize = 64;

pub fn clean_tip_jar(input: &str) -> Option<String> {
    let text = input.trim();
    let rest = text.strip_prefix("https://").or_else(|| text.strip_prefix("http://")).unwrap_or(text);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let page = match rest.split_once('/') {
        Some((host, page)) if host.eq_ignore_ascii_case(TIP_JAR_HOST) => page.trim_end_matches('/'),
        Some(_) => return None,
        None if rest.contains('.') || rest.contains(':') => return None,
        None => rest,
    };
    let valid = !page.is_empty() && page.chars().count() <= MAX_PAGE_CHARS && page.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    valid.then(|| format!("https://{TIP_JAR_HOST}/{page}"))
}

pub fn is_tip_jar(url: &str) -> bool {
    clean_tip_jar(url).as_deref() == Some(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_way_of_writing_a_page_becomes_one_link() {
        let want = Some("https://ko-fi.com/rockets".to_string());
        for input in ["rockets", " ko-fi.com/rockets ", "https://ko-fi.com/rockets/", "http://www.ko-fi.com/rockets", "https://KO-FI.com/rockets"] {
            assert_eq!(clean_tip_jar(input), want, "{input}");
        }
        assert!(is_tip_jar("https://ko-fi.com/rockets"));
        assert!(!is_tip_jar("http://ko-fi.com/rockets"));
    }

    #[test]
    fn only_a_page_on_ko_fi_is_accepted() {
        for input in [
            "",
            "https://evil.example/rockets",
            "https://ko-fi.com.evil.example/rockets",
            "https://ko-fi.com/rockets?x=1",
            "https://ko-fi.com/a/b",
            "https://ko-fi.com/",
            "javascript:alert(1)",
            "evil.example",
            "https://user@ko-fi.com/rockets",
            "ko-fi.com:8080/rockets",
        ] {
            assert_eq!(clean_tip_jar(input), None, "{input}");
        }
        assert_eq!(clean_tip_jar(&"a".repeat(MAX_PAGE_CHARS + 1)), None);
    }
}
