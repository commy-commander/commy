use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use base64::Engine;
use serde::Deserialize;

const RELEASES_URL: &str = match option_env!("COMMY_RELEASES_URL") {
    Some(url) => url,
    None => "https://releases.commy.chat",
};
const CHANNEL: &str = match option_env!("COMMY_INSTALL_CHANNEL") {
    Some(channel) => channel,
    None => "stable",
};
const PLATFORM: &str = "windows-x86_64";
const RELEASE_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEYxODJBQkQwNzY3NTlBMTMKUldRVG1uVjIwS3VDOGF0TUM3ZlUvYzNwS3kyQVFrVlJXNGMwc3oyYUxqeDhJa2RlV3J2bnBaWksK"; // gitleaks:allow

#[derive(Debug, Clone)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub signature: String,
}

#[derive(Deserialize)]
struct Manifest {
    version: String,
    platforms: HashMap<String, PlatformFile>,
}

#[derive(Deserialize)]
struct PlatformFile {
    url: String,
    signature: String,
}

#[derive(Debug)]
pub enum Problem {
    Offline,
    NoRelease,
    Cancelled,
    NotGenuine,
    StillOpen,
    Disk(String),
    Setup(String),
}

impl Problem {
    pub fn message(&self) -> String {
        match self {
            Problem::Offline => "Check your internet connection and try again.".into(),
            Problem::NoRelease => "There is no Windows download right now. Try again in a little while.".into(),
            Problem::Cancelled => "Cancelled.".into(),
            Problem::NotGenuine => "The download did not match Commy's signature, so it was not installed. Try again.".into(),
            Problem::StillOpen => "Commy is still open. Quit it from its icon next to the clock, then try again.".into(),
            Problem::Disk(reason) => format!("Commy could not be saved to this computer: {reason}"),
            Problem::Setup(reason) => format!("Setup stopped before it finished: {reason}"),
        }
    }
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("commy-setup/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default()
}

pub fn manifest_url() -> String {
    format!("{RELEASES_URL}/{CHANNEL}/latest.json")
}

pub async fn latest(client: &reqwest::Client) -> Result<Release, Problem> {
    let response = client.get(manifest_url()).send().await.map_err(|_| Problem::Offline)?;
    if !response.status().is_success() {
        return Err(Problem::NoRelease);
    }
    let manifest: Manifest = response.json().await.map_err(|_| Problem::NoRelease)?;
    release_for_windows(manifest)
}

fn release_for_windows(mut manifest: Manifest) -> Result<Release, Problem> {
    let file = manifest.platforms.remove(PLATFORM).ok_or(Problem::NoRelease)?;
    Ok(Release { version: manifest.version, url: file.url, signature: file.signature })
}

pub async fn size(client: &reqwest::Client, release: &Release) -> Option<u64> {
    let response = client.head(&release.url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    declared_length(response.headers())
}

fn declared_length(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
        .filter(|bytes| *bytes > 0)
}

pub async fn download(
    client: &reqwest::Client,
    release: &Release,
    cancelled: &AtomicBool,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<Vec<u8>, Problem> {
    let mut response = client.get(&release.url).send().await.map_err(|_| Problem::Offline)?;
    if !response.status().is_success() {
        return Err(Problem::NoRelease);
    }
    let total = response.content_length();
    let mut bytes = Vec::with_capacity(total.unwrap_or(0) as usize);
    progress(0, total);
    while let Some(chunk) = response.chunk().await.map_err(|_| Problem::Offline)? {
        if cancelled.load(Ordering::Relaxed) {
            return Err(Problem::Cancelled);
        }
        bytes.extend_from_slice(&chunk);
        progress(bytes.len() as u64, total);
    }
    Ok(bytes)
}

pub fn check_signature(bytes: &[u8], signature: &str) -> Result<(), Problem> {
    check_with_key(bytes, signature, RELEASE_KEY)
}

fn check_with_key(bytes: &[u8], signature: &str, key: &str) -> Result<(), Problem> {
    let key = decode_text(key).ok_or(Problem::NotGenuine)?;
    let signature = decode_text(signature).ok_or(Problem::NotGenuine)?;
    let key = minisign_verify::PublicKey::decode(&key).map_err(|_| Problem::NotGenuine)?;
    let signature = minisign_verify::Signature::decode(&signature).map_err(|_| Problem::NotGenuine)?;
    key.verify(bytes, &signature, true).map_err(|_| Problem::NotGenuine)
}

fn decode_text(encoded: &str) -> Option<String> {
    let raw = base64::engine::general_purpose::STANDARD.decode(encoded.trim()).ok()?;
    String::from_utf8(raw).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIGNED_TEXT: &[u8] = b"Commy setup test file\n";

    fn manifest(json: &str) -> Manifest {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn picks_the_windows_installer_from_the_manifest() {
        let release = release_for_windows(manifest(
            r#"{"version":"1.0.2","platforms":{"darwin-aarch64":{"url":"https://example.test/mac","signature":"m"},"windows-x86_64":{"url":"https://example.test/Commy_1.0.2_x64-setup.exe","signature":"w"}}}"#,
        ))
        .unwrap();
        assert_eq!(release.version, "1.0.2");
        assert_eq!(release.url, "https://example.test/Commy_1.0.2_x64-setup.exe");
        assert_eq!(release.signature, "w");
    }

    #[test]
    fn a_manifest_without_windows_is_no_release() {
        let missing = release_for_windows(manifest(r#"{"version":"1.0.2","platforms":{}}"#));
        assert!(matches!(missing, Err(Problem::NoRelease)));
    }

    #[test]
    fn reads_the_size_from_a_head_reply() {
        let mut headers = reqwest::header::HeaderMap::new();
        assert_eq!(declared_length(&headers), None);
        headers.insert(reqwest::header::CONTENT_LENGTH, "24469451".parse().unwrap());
        assert_eq!(declared_length(&headers), Some(24_469_451));
        headers.insert(reqwest::header::CONTENT_LENGTH, "0".parse().unwrap());
        assert_eq!(declared_length(&headers), None);
    }

    #[test]
    fn reads_the_stable_channel_by_default() {
        if option_env!("COMMY_INSTALL_CHANNEL").is_none() && option_env!("COMMY_RELEASES_URL").is_none() {
            assert_eq!(manifest_url(), "https://releases.commy.chat/stable/latest.json");
        }
    }

    const TEST_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDg0N0U1NjkyNzIwNjIwMTgKUldRWUlBWnlrbForaE5IVU4rd0ljOXM4R1dRWlJPMmdsWE5WanZOcThFaUJUTW9tRTJnWmtjdkwK";
    const TEST_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVRWUlBWnlrbForaE1JTXVuZTRzTmJjQWpORGx2NHUvbWN2S1hreEVmMnBmTExWaENkZjFRQmh3dDZCMk1ZbWJaMlN6bUVoSStHcFZqRlN5dk9uWkVNZ0ZVUHd1MWYxaHdjPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzk1ODE0NDYxCWZpbGU6dGVzdC50eHQKTnJwZXNKNXVtYWJBY21ueFJLdHhMUEIyK3RIdUNvN1oxdE9za05Ya2FqL1cwdXZuSC9nUTRqMWROVDNVVkJSVmhVS2tpNEk0UlR4SnJYNkllSmV1QWc9PQo=";

    #[test]
    fn accepts_a_file_signed_with_the_matching_key() {
        assert!(check_with_key(SIGNED_TEXT, TEST_SIGNATURE, TEST_KEY).is_ok());
    }

    #[test]
    fn rejects_a_changed_file() {
        assert!(matches!(check_with_key(b"Commy setup test file, changed\n", TEST_SIGNATURE, TEST_KEY), Err(Problem::NotGenuine)));
    }

    #[test]
    fn rejects_a_file_signed_with_another_key() {
        assert!(matches!(check_with_key(SIGNED_TEXT, TEST_SIGNATURE, RELEASE_KEY), Err(Problem::NotGenuine)));
    }

    #[test]
    fn rejects_a_missing_or_broken_signature() {
        assert!(matches!(check_signature(SIGNED_TEXT, ""), Err(Problem::NotGenuine)));
        assert!(matches!(check_signature(SIGNED_TEXT, "bm90IGEgc2lnbmF0dXJl"), Err(Problem::NotGenuine)));
    }
}
