use crate::message::Attachment;
use serde::{Deserialize, Serialize};

pub const LINK_PREVIEW_MIME: &str = "application/vnd.commy.link-preview+json";
pub const MAX_TITLE_CHARS: usize = 120;
pub const MAX_DESCRIPTION_CHARS: usize = 220;
pub const MAX_SITE_CHARS: usize = 48;
pub const MAX_URL_CHARS: usize = 600;
pub const MAX_IMAGE_SIDE: u32 = 4096;
const PREVIEW_IMAGE_MIMES: &[&str] = &["image/jpeg", "image/png"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewImage {
    pub mime: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPreview {
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub site: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<PreviewImage>,
}

pub struct PreviewPicture {
    pub blob: String,
    pub size_bytes: u64,
    pub provider: Option<String>,
}

impl LinkPreview {
    pub fn tidy(mut self) -> Option<Self> {
        if !is_web_link(&self.url) || self.url.chars().count() > MAX_URL_CHARS {
            return None;
        }
        self.site = clip(&squash(&self.site), MAX_SITE_CHARS);
        self.title = clip(&squash(&self.title), MAX_TITLE_CHARS);
        self.description = clip(&squash(&self.description), MAX_DESCRIPTION_CHARS);
        self.image = self.image.filter(|i| {
            PREVIEW_IMAGE_MIMES.contains(&i.mime.as_str())
                && (1..=MAX_IMAGE_SIDE).contains(&i.width)
                && (1..=MAX_IMAGE_SIDE).contains(&i.height)
        });
        let says_something = !self.title.is_empty() || !self.description.is_empty() || self.image.is_some();
        says_something.then_some(self)
    }

    pub fn into_attachment(self, picture: Option<PreviewPicture>) -> Attachment {
        let (preview, picture) = match picture {
            Some(picture) if self.image.is_some() => (self, Some(picture)),
            _ => (LinkPreview { image: None, ..self }, None),
        };
        Attachment {
            blob: picture.as_ref().map(|p| p.blob.clone()).unwrap_or_default(),
            filename: serde_json::to_string(&preview).expect("plain strings and numbers encode"),
            size_bytes: picture.as_ref().map_or(0, |p| p.size_bytes),
            mime_type: LINK_PREVIEW_MIME.to_string(),
            provider: picture.and_then(|p| p.provider),
        }
    }
}

impl Attachment {
    pub fn is_link_preview(&self) -> bool {
        self.mime_type == LINK_PREVIEW_MIME
    }

    pub fn link_preview(&self) -> Option<LinkPreview> {
        if !self.is_link_preview() {
            return None;
        }
        let mut preview = serde_json::from_str::<LinkPreview>(&self.filename).ok()?.tidy()?;
        if self.blob.is_empty() {
            preview.image = None;
        }
        Some(preview)
    }

    pub fn carries_a_file(&self) -> bool {
        !self.is_link_preview() || !self.blob.is_empty()
    }
}

pub fn is_web_link(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://")) && !url.chars().any(char::is_whitespace)
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max - 1).collect();
    format!("{}…", kept.trim_end())
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PageMeta {
    pub title: String,
    pub description: String,
    pub site: String,
    pub image: String,
    pub large_image: bool,
}

pub fn read_page_meta(html: &str) -> PageMeta {
    let mut found = Found::default();
    let lower = html.to_ascii_lowercase();
    let mut at = 0;
    while let Some(offset) = lower[at..].find('<') {
        let start = at + offset;
        let rest = &lower[start..];
        if rest.starts_with("</head") || rest.starts_with("<body") {
            break;
        }
        if rest.starts_with("<!--") {
            at = lower[start..].find("-->").map_or(lower.len(), |end| start + end + 3);
            continue;
        }
        let Some(close) = tag_end(&lower, start) else { break };
        if is_tag(rest, "meta") {
            found.meta(&attributes(&html[start + 5..close]));
        } else if is_tag(rest, "title") && found.page_title.is_none() {
            let text_start = close + 1;
            let text_end = lower[text_start..].find("</title").map_or(lower.len(), |end| text_start + end);
            found.page_title = Some(decode_entities(&html[text_start..text_end]));
            at = text_end;
            continue;
        } else if is_tag(rest, "script") || is_tag(rest, "style") {
            let name = if is_tag(rest, "script") { "</script" } else { "</style" };
            at = lower[close..].find(name).map_or(lower.len(), |end| close + end);
            continue;
        }
        at = close + 1;
    }
    found.into_meta()
}

fn is_tag(rest: &str, name: &str) -> bool {
    rest[1..].starts_with(name) && rest[1 + name.len()..].starts_with(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
}

fn tag_end(lower: &str, start: usize) -> Option<usize> {
    let bytes = lower.as_bytes();
    let mut quote: Option<u8> = None;
    for (i, &b) in bytes.iter().enumerate().skip(start + 1) {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'>' => return Some(i),
            None => {}
        }
    }
    None
}

fn attributes(source: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut chars = source.char_indices().peekable();
    while let Some(&(i, c)) = chars.peek() {
        if c.is_whitespace() || c == '/' {
            chars.next();
            continue;
        }
        let mut name_end = i;
        while let Some(&(j, c)) = chars.peek() {
            if c.is_whitespace() || c == '=' || c == '/' {
                break;
            }
            name_end = j + c.len_utf8();
            chars.next();
        }
        let name = source[i..name_end].to_ascii_lowercase();
        while chars.peek().is_some_and(|&(_, c)| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none_or(|&(_, c)| c != '=') {
            out.push((name, String::new()));
            continue;
        }
        chars.next();
        while chars.peek().is_some_and(|&(_, c)| c.is_whitespace()) {
            chars.next();
        }
        let value = match chars.peek().copied() {
            Some((j, q)) if q == '"' || q == '\'' => {
                chars.next();
                let begin = j + 1;
                let mut end = source.len();
                for (k, c) in chars.by_ref() {
                    if c == q {
                        end = k;
                        break;
                    }
                }
                &source[begin..end.max(begin)]
            }
            Some((j, _)) => {
                let mut end = source.len();
                while let Some(&(k, c)) = chars.peek() {
                    if c.is_whitespace() {
                        end = k;
                        break;
                    }
                    chars.next();
                }
                &source[j..end]
            }
            None => "",
        };
        out.push((name, decode_entities(value)));
    }
    out
}

#[derive(Default)]
struct Found {
    page_title: Option<String>,
    og_title: Option<String>,
    twitter_title: Option<String>,
    og_description: Option<String>,
    twitter_description: Option<String>,
    description: Option<String>,
    site: Option<String>,
    og_image: Option<String>,
    twitter_image: Option<String>,
    large_card: bool,
}

impl Found {
    fn meta(&mut self, attrs: &[(String, String)]) {
        let get = |key: &str| attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
        let Some(key) = get("property").or_else(|| get("name")).map(str::to_ascii_lowercase) else { return };
        let Some(content) = get("content").map(str::trim).filter(|c| !c.is_empty()) else { return };
        let slot = match key.as_str() {
            "og:title" => &mut self.og_title,
            "twitter:title" => &mut self.twitter_title,
            "og:description" => &mut self.og_description,
            "twitter:description" => &mut self.twitter_description,
            "description" => &mut self.description,
            "og:site_name" => &mut self.site,
            "og:image" | "og:image:url" | "og:image:secure_url" => &mut self.og_image,
            "twitter:image" | "twitter:image:src" => &mut self.twitter_image,
            "twitter:card" => {
                self.large_card = content.eq_ignore_ascii_case("summary_large_image");
                return;
            }
            _ => return,
        };
        if slot.is_none() {
            *slot = Some(content.to_string());
        }
    }

    fn into_meta(self) -> PageMeta {
        PageMeta {
            title: self.og_title.or(self.twitter_title).or(self.page_title).unwrap_or_default(),
            description: self.og_description.or(self.twitter_description).or(self.description).unwrap_or_default(),
            site: self.site.unwrap_or_default(),
            image: self.og_image.or(self.twitter_image).unwrap_or_default(),
            large_image: self.large_card,
        }
    }
}

pub fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let end = after.find(';').filter(|&e| e <= 10);
        let decoded = end.and_then(|e| entity(&after[..e]));
        match (end, decoded) {
            (Some(e), Some(c)) => {
                out.push(c);
                rest = &after[e + 1..];
            }
            _ => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
        return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
    }
    if let Some(dec) = name.strip_prefix('#') {
        return dec.parse().ok().and_then(char::from_u32);
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        "rsquo" => '’',
        "lsquo" => '‘',
        "rdquo" => '”',
        "ldquo" => '“',
        "middot" => '·',
        "copy" => '©',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview(url: &str) -> LinkPreview {
        LinkPreview {
            url: url.into(),
            site: "Example".into(),
            title: "A page".into(),
            description: "About it".into(),
            image: Some(PreviewImage { mime: "image/jpeg".into(), width: 640, height: 336 }),
        }
    }

    #[test]
    fn open_graph_tags_win_over_the_page_title() {
        let html = r#"<!doctype html><html><head>
            <title>Plain &amp; simple</title>
            <meta property="og:title" content="Rich &quot;title&quot;">
            <meta name='description' content='Fallback'>
            <meta property=og:description content="Told &#8220;well&#8221;">
            <meta content="https://example.org/card.png" property="og:image" />
            <meta name="twitter:card" content="summary_large_image">
            <meta property="og:site_name" content="Example">
            </head><body><meta property="og:title" content="ignored"></body>"#;
        let meta = read_page_meta(html);
        assert_eq!(meta.title, "Rich \"title\"");
        assert_eq!(meta.description, "Told \u{201c}well\u{201d}");
        assert_eq!(meta.image, "https://example.org/card.png");
        assert_eq!(meta.site, "Example");
        assert!(meta.large_image);
    }

    #[test]
    fn pages_without_open_graph_fall_back_to_title_and_description() {
        let html = "<HTML><HEAD><TITLE>\n  Old   school\n</TITLE><META NAME=\"Description\" CONTENT=\"Hand made\"><META NAME=\"twitter:image\" CONTENT=\"/a.png\"></HEAD>";
        let meta = read_page_meta(html);
        assert_eq!(meta.title.split_whitespace().collect::<Vec<_>>().join(" "), "Old school");
        assert_eq!(meta.description, "Hand made");
        assert_eq!(meta.image, "/a.png");
        assert!(!meta.large_image);
    }

    #[test]
    fn scripts_comments_and_quoted_brackets_do_not_confuse_the_reader() {
        let html = r#"<head><script>var s = "<meta property='og:title' content='nope'>";</script>
            <!-- <meta property="og:title" content="also nope"> -->
            <style>a > b {}</style>
            <meta property="og:title" content="a > b">"#;
        assert_eq!(read_page_meta(html).title, "a > b");
    }

    #[test]
    fn broken_html_reads_what_it_can_without_panicking() {
        for html in ["", "<", "<meta", "<meta property=\"og:title\" content=\"x", "<title>never closed", "&#xZZ; <meta name=a content=b c=>", "<meta property=og:title content=héllo>"] {
            let _ = read_page_meta(html);
        }
        assert_eq!(read_page_meta("<meta property=og:title content=héllo>").title, "héllo");
    }

    #[test]
    fn entities_decode_and_unknown_ones_stay_as_written() {
        assert_eq!(decode_entities("Tom &amp; Jerry&#39;s &#x1F600; &bogus; & more"), "Tom & Jerry's \u{1F600} &bogus; & more");
    }

    #[test]
    fn a_preview_round_trips_through_an_attachment() {
        let p = preview("https://example.org/post");
        let att = p.clone().into_attachment(Some(PreviewPicture { blob: "abc".into(), size_bytes: 1234, provider: Some("node".into()) }));
        assert!(att.is_link_preview());
        assert!(!att.is_image());
        assert!(att.carries_a_file());
        assert_eq!(att.size_bytes, 1234);
        assert_eq!(att.link_preview(), Some(p));
    }

    #[test]
    fn a_preview_without_its_picture_is_text_only_and_carries_no_file() {
        let att = preview("https://example.org/post").into_attachment(None);
        assert!(att.blob.is_empty());
        assert!(!att.carries_a_file());
        assert_eq!(att.link_preview().unwrap().image, None);
    }

    #[test]
    fn tidy_trims_long_text_and_refuses_odd_links_and_images() {
        let mut p = preview("https://example.org/post");
        p.title = "word ".repeat(100);
        p.description = "  lots\n\nof   space  ".into();
        p.image = Some(PreviewImage { mime: "image/svg+xml".into(), width: 10, height: 10 });
        let tidy = p.clone().tidy().unwrap();
        assert_eq!(tidy.title.chars().count(), MAX_TITLE_CHARS);
        assert!(tidy.title.ends_with('…'));
        assert_eq!(tidy.description, "lots of space");
        assert_eq!(tidy.image, None);
        assert_eq!(LinkPreview { url: "javascript:alert(1)".into(), ..p.clone() }.tidy(), None);
        assert_eq!(LinkPreview { url: "https://a b".into(), ..p.clone() }.tidy(), None);
        let empty = LinkPreview { url: "https://example.org".into(), site: "x".into(), title: " ".into(), description: String::new(), image: None };
        assert_eq!(empty.tidy(), None);
    }

    #[test]
    fn ordinary_attachments_are_not_previews() {
        let att = Attachment { blob: "b".into(), filename: "{}".into(), size_bytes: 1, mime_type: "image/png".into(), provider: None };
        assert!(!att.is_link_preview());
        assert!(att.carries_a_file());
        assert_eq!(att.link_preview(), None);
        let junk = Attachment { mime_type: LINK_PREVIEW_MIME.into(), filename: "not json".into(), ..att };
        assert_eq!(junk.link_preview(), None);
    }
}
