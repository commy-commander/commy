use serde::{Deserialize, Serialize};

use crate::{truncate, valid_image_id};

pub const EMOJIGG_IMAGES: &str = "https://cdn3.emoji.gg/emojis/";
pub const KLIPY_IMAGES: &str = "https://static.klipy.com/";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GifHit {
    pub preview: String,
    pub url: String,
    #[serde(default)]
    pub ratio: f64,
    #[serde(default)]
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameHit {
    pub id: u64,
    pub name: String,
    pub cover: Option<String>,
    pub year: Option<i32>,
    #[serde(default)]
    pub steam: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickerHit {
    pub id: String,
    pub preview: String,
    pub url: String,
    #[serde(default)]
    pub ratio: f64,
    #[serde(default)]
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickerPage {
    pub results: Vec<StickerHit>,
    #[serde(default)]
    pub more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmojiHit {
    pub id: u64,
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub animated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmojiCategory {
    pub id: u32,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmojiPage {
    pub results: Vec<EmojiHit>,
    #[serde(default)]
    pub more: bool,
    #[serde(default)]
    pub categories: Vec<EmojiCategory>,
}

#[derive(Debug, Deserialize)]
pub struct Found<T> {
    pub results: Vec<T>,
}

pub fn source(url: &str) -> Option<&'static str> {
    let (source, path) = match (url.strip_prefix(EMOJIGG_IMAGES), url.strip_prefix(KLIPY_IMAGES)) {
        (Some(path), _) if !path.contains('/') => ("emoji.gg", path),
        (_, Some(path)) => ("KLIPY", path),
        _ => return None,
    };
    if path.is_empty() || path.contains(['?', '#', '\\']) || path.contains("..") {
        return None;
    }
    Some(source)
}

pub fn image_failure(source: &str, status: u16) -> String {
    match status {
        429 => "Too many images at once, try again in a minute".into(),
        _ => format!("{source} could not be reached right now ({status})"),
    }
}

pub fn gif_query(query: &str) -> String {
    truncate(query.trim(), 100)
}

pub fn sticker_params(query: &str, from: Option<&str>, page: Option<u32>) -> [(&'static str, String); 3] {
    let from = if from == Some("gifs") { "gifs" } else { "stickers" };
    let page = page.unwrap_or(1).clamp(1, 20);
    [("q", gif_query(query)), ("from", from.to_string()), ("page", page.to_string())]
}

pub fn emoji_params(query: &str, category: Option<u32>, page: Option<u32>) -> [(&'static str, String); 3] {
    let category = category.map(|c| c.to_string()).unwrap_or_default();
    [("q", query.trim().to_string()), ("category", category), ("page", page.unwrap_or(0).to_string())]
}

pub fn safe_gifs(hits: Vec<GifHit>) -> Vec<GifHit> {
    hits.into_iter()
        .filter(|g| g.preview.starts_with("https://") && g.url.starts_with("https://"))
        .map(|g| GifHit { title: truncate(&g.title, 200), ..g })
        .collect()
}

pub fn safe_games(hits: Vec<GameHit>) -> Vec<GameHit> {
    hits.into_iter()
        .filter(|h| !h.name.trim().is_empty())
        .map(|h| GameHit { cover: h.cover.filter(|id| valid_image_id(id)), steam: h.steam.filter(|id| *id > 0), ..h })
        .collect()
}

pub fn safe_stickers(page: StickerPage) -> StickerPage {
    StickerPage {
        results: page
            .results
            .into_iter()
            .filter(|s| source(&s.url) == Some("KLIPY") && source(&s.preview) == Some("KLIPY"))
            .map(|s| StickerHit { id: truncate(&s.id, 120), title: truncate(&s.title, 200), ..s })
            .collect(),
        more: page.more,
    }
}

pub fn safe_emojis(mut page: EmojiPage) -> EmojiPage {
    page.results.retain(|hit| hit.url.starts_with(EMOJIGG_IMAGES));
    page
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_images_come_from_emoji_gg_or_klipy_only() {
        assert_eq!(source("https://cdn3.emoji.gg/emojis/12_cat.png"), Some("emoji.gg"));
        assert_eq!(source("https://static.klipy.com/ii/ab12/e8/3a/5LM0jRpL.webp"), Some("KLIPY"));
        for url in [
            "https://cdn3.emoji.gg/emojis/a/b.png",
            "https://cdn3.emoji.gg/emojis/",
            "https://static.klipy.com/ii/../x.gif",
            "https://static.klipy.com/a.gif?x=1",
            "https://static.klipy.com.evil.example/a.gif",
            "http://static.klipy.com/a.gif",
            "https://example.com/a.gif",
        ] {
            assert_eq!(source(url), None, "{url}");
        }
    }

    #[test]
    fn a_busy_account_server_is_not_blamed_on_the_image_source() {
        assert_eq!(image_failure("KLIPY", 429), "Too many images at once, try again in a minute");
        assert_eq!(image_failure("KLIPY", 502), "KLIPY could not be reached right now (502)");
    }

    #[test]
    fn only_https_gifs_and_klipy_stickers_pass() {
        let gif = |preview: &str| GifHit { preview: preview.into(), url: "https://static.klipy.com/a.gif".into(), ratio: 1.0, title: "t".repeat(300) };
        let kept = safe_gifs(vec![gif("https://static.klipy.com/p.gif"), gif("http://static.klipy.com/p.gif")]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].title.chars().count(), 200);
        let sticker = |url: &str| StickerHit { id: "1".into(), preview: url.into(), url: url.into(), ratio: 1.0, title: String::new() };
        let page = safe_stickers(StickerPage { results: vec![sticker("https://static.klipy.com/s.webp"), sticker("https://example.com/s.webp")], more: true });
        assert_eq!(page.results.len(), 1);
        assert!(page.more);
    }
}
