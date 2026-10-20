use commy_core::link_preview::{read_page_meta, LinkPreview, PreviewImage};
use image::{DynamicImage, ImageFormat};
use reqwest::header::{ACCEPT, LOCATION};
use reqwest::Url;
use serde::Serialize;
use std::io::Cursor;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::Duration;

const MAX_PAGE_BYTES: usize = 768 * 1024;
const MAX_IMAGE_BYTES: usize = 6 * 1024 * 1024;
const MAX_REDIRECTS: usize = 4;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(6);
const PREPARE_TIMEOUT: Duration = Duration::from_secs(10);
const THUMBNAIL_SIDES: [u32; 3] = [640, 400, 240];
const MAX_THUMBNAIL_BYTES: usize = 200 * 1024;
const MAX_DECODED_SIDE: u32 = 8192;
const MAX_DECODE_ALLOCATION: u64 = 160 * 1024 * 1024;
const REMEMBERED_PREVIEWS: usize = 24;
const USER_AGENT: &str = "Mozilla/5.0 (compatible; Commy/1.0; +https://commy.chat)";

#[derive(Clone)]
pub struct Prepared {
    pub preview: LinkPreview,
    pub picture: Option<Vec<u8>>,
}

static PREPARED: Mutex<Vec<(String, Prepared)>> = Mutex::new(Vec::new());

pub fn prepared(url: &str) -> Option<Prepared> {
    PREPARED.lock().unwrap().iter().find(|(u, _)| u == url).map(|(_, p)| p.clone())
}

fn remember(url: &str, prepared: Prepared) {
    let mut all = PREPARED.lock().unwrap();
    all.retain(|(u, _)| u != url);
    all.push((url.to_string(), prepared));
    let extra = all.len().saturating_sub(REMEMBERED_PREVIEWS);
    all.drain(..extra);
}

#[derive(Debug, Serialize)]
pub struct DraftPreview {
    pub url: String,
    pub site: String,
    pub title: String,
    pub description: String,
    pub image: Option<String>,
    pub width: u32,
    pub height: u32,
}

fn draft(prepared: &Prepared) -> DraftPreview {
    let p = &prepared.preview;
    let image = p.image.as_ref().zip(prepared.picture.as_ref());
    DraftPreview {
        url: p.url.clone(),
        site: p.site.clone(),
        title: p.title.clone(),
        description: p.description.clone(),
        image: image.map(|(i, bytes)| crate::data_url(&i.mime, bytes)),
        width: image.map_or(0, |(i, _)| i.width),
        height: image.map_or(0, |(i, _)| i.height),
    }
}

#[tauri::command]
pub async fn prepare_link_preview(url: String) -> Result<DraftPreview, String> {
    if let Some(known) = prepared(&url) {
        return Ok(draft(&known));
    }
    let built = tokio::time::timeout(PREPARE_TIMEOUT, build(&url))
        .await
        .map_err(|_| "the site took too long to answer".to_string())??;
    remember(&url, built.clone());
    Ok(draft(&built))
}

async fn build(raw: &str) -> Result<Prepared, String> {
    let url = Url::parse(raw).map_err(|_| "not a link".to_string())?;
    let page = fetch(&url, "text/html,application/xhtml+xml", MAX_PAGE_BYTES).await?;
    if !is_web_page(&page.content_type) {
        return Err("that link is not a web page".into());
    }
    let meta = read_page_meta(&String::from_utf8_lossy(&page.body));
    let picture = match page.url.join(&meta.image) {
        Ok(image_url) if !meta.image.is_empty() => thumbnail(&image_url).await.ok(),
        _ => None,
    };
    let site = if meta.site.is_empty() { site_from_host(&page.url) } else { meta.site };
    let preview = LinkPreview {
        url: raw.to_string(),
        site,
        title: meta.title,
        description: meta.description,
        image: picture.as_ref().map(|t| PreviewImage { mime: t.mime.to_string(), width: t.width, height: t.height }),
    }
    .tidy()
    .ok_or_else(|| "that page has nothing to preview".to_string())?;
    let picture = picture.filter(|_| preview.image.is_some()).map(|t| t.bytes);
    Ok(Prepared { preview, picture })
}

fn is_web_page(content_type: &str) -> bool {
    content_type.starts_with("text/html") || content_type.starts_with("application/xhtml")
}

fn site_from_host(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

struct Fetched {
    url: Url,
    content_type: String,
    body: Vec<u8>,
    cut_short: bool,
}

async fn fetch(start: &Url, accept: &str, max: usize) -> Result<Fetched, String> {
    let mut url = start.clone();
    for _ in 0..=MAX_REDIRECTS {
        let client = client_for(&url).await?;
        let mut response = client.get(url.clone()).header(ACCEPT, accept).send().await.map_err(|e| e.to_string())?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| "the site redirected nowhere".to_string())?;
            url = url.join(location).map_err(|_| "the site redirected to a broken link".to_string())?;
            continue;
        }
        if !response.status().is_success() {
            return Err(format!("the site answered {}", response.status()));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let mut body = Vec::new();
        let mut cut_short = false;
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            let room = max - body.len();
            body.extend_from_slice(&chunk[..chunk.len().min(room)]);
            if chunk.len() >= room {
                cut_short = chunk.len() > room || response.chunk().await.ok().flatten().is_some();
                break;
            }
        }
        return Ok(Fetched { url, content_type, body, cut_short });
    }
    Err("the site redirected too many times".into())
}

async fn client_for(url: &Url) -> Result<reqwest::Client, String> {
    if !matches!(url.scheme(), "https" | "http") {
        return Err("only web links get previews".into());
    }
    let port = url.port_or_known_default().ok_or_else(|| "that link has no port".to_string())?;
    let addresses: Vec<SocketAddr> = match url.host() {
        Some(url::Host::Ipv4(ip)) => vec![SocketAddr::new(ip.into(), port)],
        Some(url::Host::Ipv6(ip)) => vec![SocketAddr::new(ip.into(), port)],
        Some(url::Host::Domain(domain)) => tokio::net::lookup_host((domain, port)).await.map_err(|e| e.to_string())?.collect(),
        None => return Err("that link has no host".into()),
    };
    if addresses.is_empty() || !addresses.iter().all(|a| is_public(a.ip())) {
        return Err("that link points inside a private network".into());
    }
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(REQUEST_TIMEOUT)
        .user_agent(USER_AGENT);
    if let Some(url::Host::Domain(domain)) = url.host() {
        builder = builder.resolve_to_addrs(domain, &addresses);
    }
    builder.build().map_err(|e| e.to_string())
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            let shared = a == 100 && (b & 0xc0) == 64;
            let benchmarking = a == 198 && (b & 0xfe) == 18;
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || a == 0
                || a >= 240
                || shared
                || benchmarking)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let [first, second, ..] = v6.segments();
            let unique_local = (first & 0xfe00) == 0xfc00;
            let link_local = (first & 0xffc0) == 0xfe80;
            let documentation = first == 0x2001 && second == 0x0db8;
            !(v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || unique_local || link_local || documentation)
        }
    }
}

struct Thumbnail {
    bytes: Vec<u8>,
    mime: &'static str,
    width: u32,
    height: u32,
}

async fn thumbnail(url: &Url) -> Result<Thumbnail, String> {
    let picture = fetch(url, "image/*", MAX_IMAGE_BYTES).await?;
    if picture.cut_short {
        return Err("that picture is too large".into());
    }
    tokio::task::spawn_blocking(move || shrink(&picture.body)).await.map_err(|e| e.to_string())?
}

fn shrink(bytes: &[u8]) -> Result<Thumbnail, String> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODED_SIDE);
    limits.max_image_height = Some(MAX_DECODED_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOCATION);
    reader.limits(limits);
    let full = reader.decode().map_err(|e| e.to_string())?;
    for side in THUMBNAIL_SIDES {
        let small = if full.width() > side || full.height() > side { full.thumbnail(side, side) } else { full.clone() };
        let (bytes, mime) = encode(&small)?;
        if bytes.len() <= MAX_THUMBNAIL_BYTES {
            return Ok(Thumbnail { bytes, mime, width: small.width(), height: small.height() });
        }
    }
    Err("that picture does not shrink small enough".into())
}

fn encode(image: &DynamicImage) -> Result<(Vec<u8>, &'static str), String> {
    let mut out = Vec::new();
    if image.color().has_alpha() {
        image.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).map_err(|e| e.to_string())?;
        return Ok((out, "image/png"));
    }
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 82)
        .encode_image(&image.to_rgb8())
        .map_err(|e| e.to_string())?;
    Ok((out, "image/jpeg"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_and_special_addresses_are_never_fetched() {
        for ip in ["127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.169.254", "100.64.0.1", "0.0.0.0", "255.255.255.255", "198.18.0.1", "::1", "fd00::1", "fe80::1", "::ffff:192.168.0.1", "::"] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "93.184.216.34", "2606:4700::1111", "::ffff:8.8.8.8"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn site_names_drop_the_www() {
        assert_eq!(site_from_host(&Url::parse("https://www.example.org/a").unwrap()), "example.org");
        assert_eq!(site_from_host(&Url::parse("https://news.example.org").unwrap()), "news.example.org");
    }

    #[test]
    fn pictures_shrink_to_a_small_jpeg_or_png() {
        let photo = DynamicImage::new_rgb8(1600, 900);
        let mut source = Vec::new();
        photo.write_to(&mut Cursor::new(&mut source), ImageFormat::Png).unwrap();
        let thumb = shrink(&source).unwrap();
        assert_eq!((thumb.width, thumb.height, thumb.mime), (640, 360, "image/jpeg"));
        assert!(thumb.bytes.len() <= MAX_THUMBNAIL_BYTES);

        let logo = DynamicImage::new_rgba8(64, 64);
        let mut source = Vec::new();
        logo.write_to(&mut Cursor::new(&mut source), ImageFormat::Png).unwrap();
        let thumb = shrink(&source).unwrap();
        assert_eq!((thumb.width, thumb.height, thumb.mime), (64, 64, "image/png"));
    }

    #[test]
    fn junk_is_not_a_picture() {
        assert!(shrink(b"<html>not a picture</html>").is_err());
    }

    #[test]
    fn prepared_previews_are_remembered_newest_last_and_capped() {
        let make = |n: usize| Prepared {
            preview: LinkPreview { url: format!("https://e.org/{n}"), site: String::new(), title: "t".into(), description: String::new(), image: None },
            picture: None,
        };
        for n in 0..REMEMBERED_PREVIEWS + 5 {
            remember(&format!("https://e.org/{n}"), make(n));
        }
        assert!(prepared("https://e.org/0").is_none());
        assert!(prepared(&format!("https://e.org/{}", REMEMBERED_PREVIEWS + 4)).is_some());
        assert_eq!(PREPARED.lock().unwrap().len(), REMEMBERED_PREVIEWS);
    }
}
