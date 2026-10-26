use std::collections::HashSet;

use serde::Serialize;
use url::{Host, Url};

const WELL_KNOWN_SITES: &[&str] = &[
    "commy.chat",
    "google.com",
    "youtube.com",
    "youtu.be",
    "youtube-nocookie.com",
    "gmail.com",
    "apple.com",
    "icloud.com",
    "microsoft.com",
    "live.com",
    "outlook.com",
    "xbox.com",
    "github.com",
    "gitlab.com",
    "amazon.com",
    "paypal.com",
    "netflix.com",
    "spotify.com",
    "twitch.tv",
    "reddit.com",
    "wikipedia.org",
    "wikimedia.org",
    "x.com",
    "twitter.com",
    "facebook.com",
    "instagram.com",
    "whatsapp.com",
    "tiktok.com",
    "linkedin.com",
    "steampowered.com",
    "steamcommunity.com",
    "epicgames.com",
    "roblox.com",
    "minecraft.net",
    "playstation.com",
    "nintendo.com",
    "blizzard.com",
    "riotgames.com",
    "coinbase.com",
    "binance.com",
    "dropbox.com",
    "yahoo.com",
    "bing.com",
    "duckduckgo.com",
    "imgur.com",
    "stackoverflow.com",
    "mozilla.org",
    "signal.org",
];

const REGIONAL_SECOND_LEVELS: &[&str] = &["com", "co", "net", "org", "gov", "edu", "ac", "ne", "or", "me", "ltd", "plc"];

const NAMED_TLDS: &[&str] = &[
    "com", "net", "org", "info", "biz", "app", "dev", "xyz", "site", "online", "store", "shop", "live", "link", "club",
    "top", "fun", "pro", "gov", "edu", "chat", "zip", "mov", "page", "blog", "news", "gift", "gifts", "game", "games",
    "cloud", "tech", "click", "support", "help", "wiki", "social", "network", "email", "money", "bank", "login",
];

const FILE_EXTENSIONS_LIKE_TLDS: &[&str] = &["js", "rs", "md", "py", "ts", "go", "cs", "sh", "pl", "rb", "kt", "ps"];

const CONFUSABLE_SPELLINGS: &[(&str, &str)] = &[("rn", "m"), ("vv", "w"), ("cl", "d")];

const MIN_LABEL_FOR_LOOKALIKE: usize = 4;
const MIN_LABEL_FOR_HYPHEN_MATCH: usize = 6;
const MIN_LABEL_FOR_TYPO_MATCH: usize = 8;
const MAX_DOMAIN_LEN: usize = 253;
const PUNYCODE_PREFIX: &str = "xn--";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkLevel {
    Open,
    Ask,
    Warn,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkSign {
    KnownBad,
    Disguised,
    TextMismatch,
    Lookalike,
    BrandName,
    International,
    Address,
}

impl LinkSign {
    fn deceptive(self) -> bool {
        matches!(self, Self::Disguised | Self::TextMismatch | Self::Lookalike)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkCheck {
    pub url: String,
    pub host: String,
    pub shown_host: String,
    pub site: String,
    pub trust_key: String,
    pub level: LinkLevel,
    pub signs: Vec<LinkSign>,
    pub can_trust: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Blocklist {
    domains: HashSet<String>,
}

impl Blocklist {
    pub fn parse(text: &str) -> Self {
        let domains = text.lines().filter_map(listed_domain).collect();
        Self { domains }
    }

    pub fn len(&self) -> usize {
        self.domains.len()
    }

    pub fn is_empty(&self) -> bool {
        self.domains.is_empty()
    }

    pub fn lists(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        let listed = parent_domains(&host).any(|domain| self.domains.contains(domain));
        listed
    }
}

fn listed_domain(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with(['#', '!']) {
        return None;
    }
    let token = line.split_whitespace().last()?;
    let domain = token.trim_start_matches("||").trim_end_matches('^').trim_matches('.').to_ascii_lowercase();
    let plausible = domain.contains('.')
        && domain.len() <= MAX_DOMAIN_LEN
        && domain.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    plausible.then_some(domain)
}

fn parent_domains(host: &str) -> impl Iterator<Item = &str> {
    let mut rest = Some(host);
    std::iter::from_fn(move || {
        let current = rest?;
        rest = current.split_once('.').map(|(_, parent)| parent).filter(|parent| parent.contains('.'));
        Some(current)
    })
}

pub fn site_of(host: &str) -> &str {
    let dots: Vec<usize> = host.match_indices('.').map(|(at, _)| at).collect();
    if dots.len() < 2 {
        return host;
    }
    let (second, country) = host[dots[dots.len() - 2] + 1..].split_once('.').unwrap_or_default();
    let regional = country.len() == 2 && REGIONAL_SECOND_LEVELS.contains(&second);
    let keep = if regional { 3 } else { 2 };
    if dots.len() < keep {
        return host;
    }
    &host[dots[dots.len() - keep] + 1..]
}

fn site_label(site: &str) -> &str {
    site.split('.').next().unwrap_or(site)
}

pub fn trust_key(host: &str) -> String {
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

fn within(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn well_known(host: &str) -> bool {
    WELL_KNOWN_SITES.iter().any(|site| within(host, site))
}

fn trusted_by_user(host: &str, trusted: &[String]) -> bool {
    let key = trust_key(host);
    trusted.iter().map(|t| t.trim().trim_end_matches('.').to_ascii_lowercase()).any(|t| !t.is_empty() && within(&key, &t))
}

fn forwards_elsewhere(url: &Url) -> bool {
    let rest = format!("{}?{}", url.path(), url.query().unwrap_or("")).to_ascii_lowercase();
    rest.contains("http://") || rest.contains("https://") || rest.contains("http%3a") || rest.contains("https%3a")
}

pub fn unicode_host(host: &str) -> String {
    let (shown, result) = idna::domain_to_unicode(host);
    if result.is_ok() { shown } else { host.to_string() }
}

fn latin_twin(c: char) -> char {
    match c {
        'а' | 'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'α' => 'a',
        'Ь' | 'ь' | 'в' => 'b',
        'с' | 'ç' | 'ϲ' => 'c',
        'ԁ' | 'ď' => 'd',
        'е' | 'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ε' | '3' => 'e',
        'ɡ' | 'ġ' => 'g',
        'һ' => 'h',
        'i' | 'і' | 'ì' | 'í' | 'î' | 'ï' | 'ı' | 'ι' | 'l' | '1' | '|' | 'ӏ' | 'ł' => 'l',
        'ј' => 'j',
        'κ' | 'к' => 'k',
        'м' => 'm',
        'п' | 'ñ' | 'ń' | 'η' => 'n',
        'о' | 'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ο' | '0' => 'o',
        'р' | 'ρ' => 'p',
        'ԛ' => 'q',
        'ѕ' | 'ś' | 'š' | '5' => 's',
        'т' | 'τ' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'υ' => 'u',
        'ν' | 'ѵ' => 'v',
        'ԝ' | 'ω' => 'w',
        'х' | 'χ' => 'x',
        'у' | 'ý' | 'ÿ' | 'γ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        other => other,
    }
}

fn skeleton(label: &str) -> String {
    let mut flat: String = label.to_lowercase().chars().filter(|c| *c != '-').map(latin_twin).collect();
    for (pair, single) in CONFUSABLE_SPELLINGS {
        flat = flat.replace(pair, single);
    }
    flat
}

fn one_edit_apart(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (short, long) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };
    if long.len() - short.len() > 1 {
        return false;
    }
    let mut i = 0;
    while i < short.len() && short[i] == long[i] {
        i += 1;
    }
    if i == short.len() {
        return long.len() != short.len();
    }
    if short.len() == long.len() {
        short[i + 1..] == long[i + 1..]
    } else {
        short[i..] == long[i + 1..]
    }
}

fn known_labels() -> impl Iterator<Item = &'static str> {
    WELL_KNOWN_SITES.iter().map(|known| site_label(known))
}

fn imitates(host: &str, shown_label: &str) -> bool {
    if well_known(host) {
        return false;
    }
    let label = site_label(site_of(host));
    let embedded = WELL_KNOWN_SITES
        .iter()
        .any(|known| (host.starts_with(&format!("{known}.")) || host.contains(&format!(".{known}."))) && label != site_label(known));
    if embedded {
        return true;
    }
    if known_labels().any(|known| known == label) {
        return false;
    }
    let shown_skeleton = skeleton(shown_label);
    known_labels().any(|known| {
        let homoglyph = known.len() >= MIN_LABEL_FOR_LOOKALIKE && shown_label != known && shown_skeleton == skeleton(known);
        let typo = known.len() >= MIN_LABEL_FOR_TYPO_MATCH && one_edit_apart(label, known);
        homoglyph || typo
    })
}

fn borrows_brand_name(host: &str) -> bool {
    let label = site_label(site_of(host));
    !well_known(host)
        && label.contains('-')
        && known_labels().any(|known| known.len() >= MIN_LABEL_FOR_HYPHEN_MATCH && label.split('-').any(|part| part == known))
}

fn named_host(text: &str) -> Option<String> {
    let token = text.trim_matches(|c: char| !c.is_alphanumeric());
    let token = token.split_once("://").map(|(_, rest)| rest).unwrap_or(token);
    let token = token.split(['/', '?', '#']).next()?.trim_end_matches('.').to_lowercase();
    let tld = token.rsplit_once('.')?.1;
    let looks_like_tld = (tld.len() == 2 && tld.chars().all(|c| c.is_ascii_lowercase()) && !FILE_EXTENSIONS_LIKE_TLDS.contains(&tld))
        || NAMED_TLDS.contains(&tld);
    let host_chars = token.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '-'));
    (looks_like_tld && host_chars && !token.starts_with('.') && !token.contains("..")).then_some(token)
}

fn text_names_other_site(label: &str, site: &str) -> bool {
    label.split_whitespace().filter_map(named_host).any(|named| {
        let named = Url::parse(&format!("https://{named}/"))
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or(named);
        site_of(&named) != site
    })
}

pub fn check_link(raw: &str, label: Option<&str>, trusted: &[String], blocklist: &Blocklist) -> Option<LinkCheck> {
    let url = Url::parse(raw.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let (host, address) = match url.host()? {
        Host::Domain(domain) => (domain.trim_end_matches('.').to_ascii_lowercase(), false),
        Host::Ipv4(ip) => (ip.to_string(), true),
        Host::Ipv6(ip) => (ip.to_string(), true),
    };
    let shown_host = if address { host.clone() } else { unicode_host(&host) };
    let site = if address { host.clone() } else { site_of(&host).to_string() };
    let international = host.split('.').any(|part| part.starts_with(PUNYCODE_PREFIX));

    let mut signs = Vec::new();
    if blocklist.lists(&host) {
        signs.push(LinkSign::KnownBad);
    }
    if !url.username().is_empty() || url.password().is_some() {
        signs.push(LinkSign::Disguised);
    }
    if label.is_some_and(|text| text_names_other_site(text, &site)) {
        signs.push(LinkSign::TextMismatch);
    }
    if !address && imitates(&host, site_label(site_of(&shown_host))) {
        signs.push(LinkSign::Lookalike);
    }
    if !address && borrows_brand_name(&host) {
        signs.push(LinkSign::BrandName);
    }
    if international {
        signs.push(LinkSign::International);
    }
    if address {
        signs.push(LinkSign::Address);
    }

    let deceptive = signs.iter().any(|sign| sign.deceptive());
    let level = if signs.contains(&LinkSign::KnownBad) {
        LinkLevel::Block
    } else if deceptive {
        LinkLevel::Warn
    } else if (well_known(&host) && !forwards_elsewhere(&url)) || trusted_by_user(&host, trusted) {
        LinkLevel::Open
    } else if international || signs.contains(&LinkSign::BrandName) {
        LinkLevel::Warn
    } else {
        LinkLevel::Ask
    };
    Some(LinkCheck {
        url: url.to_string(),
        trust_key: trust_key(&host),
        host,
        shown_host,
        site,
        level,
        can_trust: matches!(level, LinkLevel::Ask | LinkLevel::Warn) && !deceptive,
        signs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(url: &str) -> LinkCheck {
        check_link(url, None, &[], &Blocklist::default()).unwrap()
    }

    fn labelled(url: &str, label: &str) -> LinkCheck {
        check_link(url, Some(label), &[], &Blocklist::default()).unwrap()
    }

    #[test]
    fn only_web_links_are_checked() {
        assert!(check_link("javascript:alert(1)", None, &[], &Blocklist::default()).is_none());
        assert!(check_link("file:///etc/passwd", None, &[], &Blocklist::default()).is_none());
        assert!(check_link("not a link", None, &[], &Blocklist::default()).is_none());
    }

    #[test]
    fn unfamiliar_sites_ask_first_and_can_be_trusted() {
        let link = check("https://www.example.org/page");
        assert_eq!(link.level, LinkLevel::Ask);
        assert!(link.can_trust);
        assert_eq!(link.host, "www.example.org");
        assert_eq!(link.site, "example.org");
        assert_eq!(link.trust_key, "example.org");
        assert!(link.signs.is_empty());
    }

    #[test]
    fn well_known_sites_open_straight_away() {
        assert_eq!(check("https://www.youtube.com/watch?v=abc").level, LinkLevel::Open);
        assert_eq!(check("https://en.wikipedia.org/wiki/Star").level, LinkLevel::Open);
        assert_eq!(check("https://commy.chat/docs").level, LinkLevel::Open);
    }

    #[test]
    fn well_known_sites_that_forward_elsewhere_still_ask() {
        assert_eq!(check("https://www.google.com/url?q=https://example.org").level, LinkLevel::Ask);
        assert_eq!(check("https://www.google.com/url?q=https%3A%2F%2Fexample.org").level, LinkLevel::Ask);
    }

    #[test]
    fn trusted_domains_open_with_their_subdomains() {
        let trusted = vec!["example.org".to_string()];
        let open = |url: &str| check_link(url, None, &trusted, &Blocklist::default()).unwrap().level;
        assert_eq!(open("https://example.org/"), LinkLevel::Open);
        assert_eq!(open("https://www.example.org/"), LinkLevel::Open);
        assert_eq!(open("https://blog.example.org/"), LinkLevel::Open);
        assert_eq!(open("https://notexample.org/"), LinkLevel::Ask);
        assert_eq!(open("https://example.org.evil.io/"), LinkLevel::Ask);
    }

    #[test]
    fn listed_sites_are_blocked_even_when_trusted() {
        let list = Blocklist::parse("# Title: list\n! comment\n0.0.0.0 bad.example\n||worse.example^\nplain.example\n\nnot-a-domain\n");
        assert_eq!(list.len(), 3);
        let trusted = vec!["bad.example".to_string()];
        let link = check_link("https://login.bad.example/x", None, &trusted, &list).unwrap();
        assert_eq!(link.level, LinkLevel::Block);
        assert!(!link.can_trust);
        assert_eq!(link.signs, vec![LinkSign::KnownBad]);
        assert!(list.lists("worse.example"));
        assert!(list.lists("a.b.plain.example"));
        assert!(!list.lists("example"));
        assert!(!list.lists("good.example"));
    }

    #[test]
    fn homoglyph_and_digit_swaps_warn() {
        let cyrillic = check("https://pаypal.com/login");
        assert_eq!(cyrillic.level, LinkLevel::Warn);
        assert!(cyrillic.signs.contains(&LinkSign::Lookalike));
        assert!(cyrillic.signs.contains(&LinkSign::International));
        assert_eq!(cyrillic.shown_host, "pаypal.com");
        assert!(cyrillic.host.starts_with(PUNYCODE_PREFIX));
        assert!(!cyrillic.can_trust);

        for url in ["https://paypa1.com/", "https://steamcommunlty.com/", "https://rnicrosoft.com/", "https://pay-pal.com/", "https://wikipedla.org/"] {
            let link = check(url);
            assert_eq!(link.level, LinkLevel::Warn, "{url}");
            assert_eq!(link.signs, vec![LinkSign::Lookalike], "{url}");
        }
    }

    #[test]
    fn brand_names_used_as_bait_warn() {
        let bait = check("https://paypal-login.com/");
        assert_eq!(bait.level, LinkLevel::Warn);
        assert_eq!(bait.signs, vec![LinkSign::BrandName]);
        assert!(bait.can_trust);
        assert!(check("https://paypal.com.secure-check.io/").signs.contains(&LinkSign::Lookalike));
        assert!(check("https://steamcomunity.com/").signs.contains(&LinkSign::Lookalike));
    }

    #[test]
    fn ordinary_sites_and_regional_brand_sites_are_not_lookalikes() {
        for url in ["https://example.org/", "https://google.de/", "https://amazon.co.uk/", "https://www.google.com.br/", "https://youtube-nocookie.com/", "https://upload.wikimedia.org/", "https://wikipedia.com/", "https://applepie.example/", "https://switch.ch/", "https://twitchy.example/"] {
            assert!(!check(url).signs.contains(&LinkSign::Lookalike), "{url}");
        }
    }

    #[test]
    fn international_names_warn_but_can_be_trusted() {
        let link = check("https://münchen.de/");
        assert_eq!(link.level, LinkLevel::Warn);
        assert_eq!(link.signs, vec![LinkSign::International]);
        assert!(link.can_trust);
        assert_eq!(link.shown_host, "münchen.de");
        let trusted = vec![link.trust_key.clone()];
        assert_eq!(check_link("https://münchen.de/", None, &trusted, &Blocklist::default()).unwrap().level, LinkLevel::Open);
    }

    #[test]
    fn text_naming_another_site_warns() {
        let link = labelled("https://evil.example/login", "paypal.com");
        assert_eq!(link.level, LinkLevel::Warn);
        assert_eq!(link.signs, vec![LinkSign::TextMismatch]);
        assert!(!link.can_trust);
        assert!(labelled("https://evil.example/", "sign in at https://www.paypal.com/signin").signs.contains(&LinkSign::TextMismatch));
        assert!(labelled("https://paypal.com.evil.example/", "paypal.com").signs.contains(&LinkSign::TextMismatch));
    }

    #[test]
    fn text_matching_the_site_or_naming_no_site_is_fine() {
        assert!(labelled("https://www.example.org/a", "example.org").signs.is_empty());
        assert!(labelled("https://docs.example.org/a", "example.org docs").signs.is_empty());
        assert!(labelled("https://example.org/a", "the release notes").signs.is_empty());
        assert!(labelled("https://example.org/a", "see main.rs and notes.md").signs.is_empty());
        assert!(labelled("https://example.org/a", "version 1.2").signs.is_empty());
    }

    #[test]
    fn hidden_sign_in_names_and_addresses_are_flagged() {
        let disguised = check("https://paypal.com@evil.example/");
        assert_eq!(disguised.host, "evil.example");
        assert_eq!(disguised.level, LinkLevel::Warn);
        assert!(disguised.signs.contains(&LinkSign::Disguised));

        let address = check("http://192.168.1.20:8080/");
        assert_eq!(address.level, LinkLevel::Ask);
        assert_eq!(address.signs, vec![LinkSign::Address]);
    }

    #[test]
    fn sites_drop_subdomains_but_keep_regional_suffixes() {
        assert_eq!(site_of("a.b.example.com"), "example.com");
        assert_eq!(site_of("shop.example.co.uk"), "example.co.uk");
        assert_eq!(site_of("example.co.uk"), "example.co.uk");
        assert_eq!(site_of("localhost"), "localhost");
        assert_eq!(site_of("www.google.com.hk"), "google.com.hk");
        assert_eq!(site_of("paypal.com.evil.io"), "evil.io");
    }

    #[test]
    fn one_edit_apart_counts_single_changes() {
        assert!(one_edit_apart("steamcommunity", "steamcomunity"));
        assert!(one_edit_apart("steamcommunity", "steamcommunitty"));
        assert!(one_edit_apart("microsoft", "microsoff"));
        assert!(!one_edit_apart("microsoft", "microsoft"));
        assert!(!one_edit_apart("microsoft", "macrosaft"));
    }
}
