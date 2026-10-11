//! Optional IGDB lookups (game names and cover art). IGDB authenticates
//! with Twitch developer credentials: a client id and secret exchanged
//! for an app access token. Everything works without them; lookups just
//! return nothing. Results are cached and requests are spaced to stay
//! under IGDB's limit of 4 requests per second.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const TOKEN_URL: &str = "https://id.twitch.tv/oauth2/token";
const GAMES_URL: &str = "https://api.igdb.com/v4/games";
const MIN_GAP: Duration = Duration::from_millis(260);
const SEARCH_CACHE_CAP: usize = 300;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IgdbCredentials {
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
}

impl IgdbCredentials {
    pub fn configured(&self) -> bool {
        !self.client_id.trim().is_empty() && !self.client_secret.trim().is_empty()
    }
}

/// One game as the app uses it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameHit {
    pub id: u64,
    pub name: String,
    /// IGDB image id of the cover, see [`cover_url`].
    pub cover: Option<String>,
    pub year: Option<i32>,
}

#[derive(Deserialize)]
struct ApiCover {
    image_id: Option<String>,
}

#[derive(Deserialize)]
struct ApiGame {
    id: u64,
    name: Option<String>,
    cover: Option<ApiCover>,
    first_release_date: Option<i64>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
}

/// Image ids are short alphanumeric strings; anything else is dropped so
/// a peer cannot smuggle a URL in place of a cover.
pub fn valid_image_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

/// "t_cover_small" (90x128) or "t_thumb" (90x90) sized cover.
pub fn cover_url(image_id: &str, size: &str) -> Option<String> {
    valid_image_id(image_id).then(|| format!("https://images.igdb.com/igdb/image/upload/{size}/{image_id}.jpg"))
}

/// Lowercase letters and digits only, for comparing names.
pub fn normalize(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Escapes a user query for IGDB's Apicalypse `search "..."` clause.
pub fn apicalypse_string(query: &str) -> String {
    query.chars().filter(|c| !c.is_control()).collect::<String>().replace('\\', "\\\\").replace('"', "\\\"")
}

/// The hit that is clearly the same game as `name`, if any. Exact name
/// matches only (ignoring punctuation and case), so an app called "Code"
/// never turns into a game.
pub fn best_match<'a>(name: &str, hits: &'a [GameHit]) -> Option<&'a GameHit> {
    let want = normalize(name);
    if want.len() < 3 {
        return None;
    }
    hits.iter().find(|h| normalize(&h.name) == want)
}

fn to_hit(game: ApiGame) -> Option<GameHit> {
    let name = game.name?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some(GameHit {
        id: game.id,
        name,
        cover: game.cover.and_then(|c| c.image_id).filter(|id| valid_image_id(id)),
        year: game
            .first_release_date
            .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
            .map(|d| chrono::Datelike::year(&d)),
    })
}

/// Live client: token, pacing and caches. Held behind a tokio mutex.
#[derive(Default)]
pub struct IgdbClient {
    token: Option<(String, Instant)>,
    last_request: Option<Instant>,
    searches: HashMap<String, Vec<GameHit>>,
    /// Activity names already resolved (None: not a known game).
    pub resolved: HashMap<String, Option<GameHit>>,
}

impl IgdbClient {
    /// Forget the token and caches, e.g. after the credentials change.
    pub fn reset(&mut self) {
        self.token = None;
        self.searches.clear();
        self.resolved.clear();
    }

    async fn pace(&mut self) {
        if let Some(last) = self.last_request {
            if let Some(wait) = MIN_GAP.checked_sub(last.elapsed()) {
                tokio::time::sleep(wait).await;
            }
        }
        self.last_request = Some(Instant::now());
    }

    async fn token(&mut self, http: &reqwest::Client, creds: &IgdbCredentials) -> Result<String, String> {
        if let Some((token, expires)) = &self.token {
            if Instant::now() < *expires {
                return Ok(token.clone());
            }
        }
        self.pace().await;
        let response = http
            .post(TOKEN_URL)
            .query(&[
                ("client_id", creds.client_id.trim()),
                ("client_secret", creds.client_secret.trim()),
                ("grant_type", "client_credentials"),
            ])
            .send()
            .await
            .map_err(|e| format!("could not reach Twitch: {e}"))?;
        if !response.status().is_success() {
            return Err("Twitch rejected the client id or secret".into());
        }
        let token: TokenResponse = response.json().await.map_err(|_| "unexpected reply from Twitch".to_string())?;
        // Refresh a minute early.
        let expires = Instant::now() + Duration::from_secs(token.expires_in.saturating_sub(60));
        self.token = Some((token.access_token.clone(), expires));
        Ok(token.access_token)
    }

    /// Games matching `query`, best matches first.
    pub async fn search(&mut self, http: &reqwest::Client, creds: &IgdbCredentials, query: &str) -> Result<Vec<GameHit>, String> {
        let query = query.trim();
        if !creds.configured() || query.chars().count() < 2 {
            return Ok(Vec::new());
        }
        let key = normalize(query);
        if let Some(hits) = self.searches.get(&key) {
            return Ok(hits.clone());
        }
        let token = self.token(http, creds).await?;
        self.pace().await;
        let body = format!(
            "search \"{}\"; fields name,cover.image_id,first_release_date; where version_parent = null; limit 8;",
            apicalypse_string(query)
        );
        let response = http
            .post(GAMES_URL)
            .header("Client-ID", creds.client_id.trim())
            .header("Authorization", format!("Bearer {token}"))
            .body(body)
            .send()
            .await
            .map_err(|e| format!("could not reach IGDB: {e}"))?;
        if response.status().as_u16() == 401 {
            self.token = None;
            return Err("IGDB rejected the token; check the credentials".into());
        }
        if !response.status().is_success() {
            return Err(format!("IGDB returned {}", response.status()));
        }
        let games: Vec<ApiGame> = response.json().await.map_err(|_| "unexpected reply from IGDB".to_string())?;
        let hits: Vec<GameHit> = games.into_iter().filter_map(to_hit).collect();
        if self.searches.len() >= SEARCH_CACHE_CAP {
            self.searches.clear();
        }
        self.searches.insert(key, hits.clone());
        Ok(hits)
    }

    /// Resolves a detected activity name to a game, caching misses too.
    pub async fn resolve(&mut self, http: &reqwest::Client, creds: &IgdbCredentials, name: &str) -> Option<GameHit> {
        let key = normalize(name);
        if let Some(hit) = self.resolved.get(&key) {
            return hit.clone();
        }
        let hit = match self.search(http, creds, name).await {
            Ok(hits) => best_match(name, &hits).cloned(),
            // Network trouble: try again next time instead of caching a miss.
            Err(_) => return None,
        };
        self.resolved.insert(key, hit.clone());
        hit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(name: &str) -> GameHit {
        GameHit { id: 1, name: name.into(), cover: None, year: None }
    }

    #[test]
    fn matches_only_the_same_name() {
        let hits = vec![hit("Hollow Knight: Silksong"), hit("Hollow Knight")];
        assert_eq!(best_match("hollow knight", &hits).map(|h| h.name.as_str()), Some("Hollow Knight"));
        assert_eq!(best_match("Hollow-Knight", &hits).map(|h| h.name.as_str()), Some("Hollow Knight"));
        assert!(best_match("Hollow", &hits).is_none());
        assert!(best_match("ab", &[hit("AB")]).is_none(), "too short to trust");
    }

    #[test]
    fn covers_only_from_safe_ids() {
        assert_eq!(
            cover_url("co1rgi", "t_thumb").as_deref(),
            Some("https://images.igdb.com/igdb/image/upload/t_thumb/co1rgi.jpg")
        );
        assert!(cover_url("../evil", "t_thumb").is_none());
        assert!(cover_url("", "t_thumb").is_none());
    }

    #[test]
    fn queries_are_escaped() {
        assert_eq!(apicalypse_string(r#"Tom "Clancy" \ x"#), r#"Tom \"Clancy\" \\ x"#);
        assert_eq!(apicalypse_string("a\nb"), "ab");
    }

    #[test]
    fn api_games_convert() {
        let game = ApiGame { id: 7, name: Some(" Celeste ".into()), cover: Some(ApiCover { image_id: Some("co3byy".into()) }), first_release_date: Some(1516665600) };
        assert_eq!(to_hit(game), Some(GameHit { id: 7, name: "Celeste".into(), cover: Some("co3byy".into()), year: Some(2018) }));
        let unnamed = ApiGame { id: 8, name: None, cover: None, first_release_date: None };
        assert_eq!(to_hit(unnamed), None);
    }

    #[test]
    fn credentials_need_both_parts() {
        assert!(!IgdbCredentials::default().configured());
        assert!(!IgdbCredentials { client_id: "x".into(), client_secret: " ".into() }.configured());
        assert!(IgdbCredentials { client_id: "x".into(), client_secret: "y".into() }.configured());
    }
}
