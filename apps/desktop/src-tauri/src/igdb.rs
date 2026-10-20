use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const TOKEN_URL: &str = "https://id.twitch.tv/oauth2/token";
const GAMES_URL: &str = "https://api.igdb.com/v4/games";
const IGDB_MAX_REQUESTS_PER_SECOND: u128 = 4;
const MIN_REQUEST_GAP: Duration = Duration::from_millis(260);
const _: () = assert!(MIN_REQUEST_GAP.as_millis() * IGDB_MAX_REQUESTS_PER_SECOND > 1000);
const TOKEN_REFRESH_MARGIN_SECS: u64 = 60;
const SEARCH_CACHE_CAP: usize = 300;
const STEAM_SOURCE: u64 = 1;
const SEARCH_FIELDS: &str = "name,cover.image_id,first_release_date,external_games.external_game_source,external_games.uid,external_games.url";

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameHit {
    pub id: u64,
    pub name: String,
    pub cover: Option<String>,
    pub year: Option<i32>,
    #[serde(default)]
    pub steam: Option<u32>,
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
    #[serde(default)]
    external_games: Vec<ApiExternalGame>,
}

#[derive(Deserialize)]
struct ApiExternalGame {
    external_game_source: Option<u64>,
    uid: Option<String>,
    url: Option<String>,
}

fn steam_app_id(uid: &str) -> Option<u32> {
    let uid = uid.trim();
    let digits = !uid.is_empty() && uid.len() <= 10 && uid.bytes().all(|b| b.is_ascii_digit());
    digits.then(|| uid.parse().ok()).flatten().filter(|id| *id > 0)
}

fn steam_id_from_url(url: &str) -> Option<u32> {
    let parsed = reqwest::Url::parse(url).ok()?;
    if parsed.host_str()? != "store.steampowered.com" {
        return None;
    }
    let mut segments = parsed.path_segments()?;
    (segments.next()? == "app").then(|| segments.next().and_then(steam_app_id)).flatten()
}

fn steam_of(external: &[ApiExternalGame]) -> Option<u32> {
    let from_source = external
        .iter()
        .filter(|e| e.external_game_source == Some(STEAM_SOURCE))
        .find_map(|e| e.uid.as_deref().and_then(steam_app_id));
    from_source.or_else(|| external.iter().find_map(|e| e.url.as_deref().and_then(steam_id_from_url)))
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
}

pub fn valid_image_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

pub fn normalize(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

pub fn apicalypse_string(query: &str) -> String {
    query.chars().filter(|c| !c.is_control()).collect::<String>().replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn safe_hits(hits: Vec<GameHit>) -> Vec<GameHit> {
    hits.into_iter()
        .filter(|h| !h.name.trim().is_empty())
        .map(|h| GameHit { cover: h.cover.filter(|id| valid_image_id(id)), steam: h.steam.filter(|id| *id > 0), ..h })
        .collect()
}

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
        steam: steam_of(&game.external_games),
    })
}

#[derive(Default)]
pub struct IgdbClient {
    token: Option<(String, Instant)>,
    last_request: Option<Instant>,
    searches: HashMap<String, Vec<GameHit>>,
    pub resolved: HashMap<String, Option<GameHit>>,
}

impl IgdbClient {
    pub fn reset(&mut self) {
        self.token = None;
        self.searches.clear();
        self.resolved.clear();
    }

    async fn pace(&mut self) {
        if let Some(last) = self.last_request {
            if let Some(wait) = MIN_REQUEST_GAP.checked_sub(last.elapsed()) {
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
        let expires = Instant::now() + Duration::from_secs(token.expires_in.saturating_sub(TOKEN_REFRESH_MARGIN_SECS));
        self.token = Some((token.access_token.clone(), expires));
        Ok(token.access_token)
    }

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
            "search \"{}\"; fields {SEARCH_FIELDS}; where version_parent = null; limit 8;",
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

    pub async fn resolve(&mut self, http: &reqwest::Client, creds: &IgdbCredentials, name: &str) -> Option<GameHit> {
        let key = normalize(name);
        if let Some(hit) = self.resolved.get(&key) {
            return hit.clone();
        }
        let hit = match self.search(http, creds, name).await {
            Ok(hits) => best_match(name, &hits).cloned(),
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
        GameHit { id: 1, name: name.into(), cover: None, year: None, steam: None }
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
        assert!(valid_image_id("co1rgi"));
        assert!(!valid_image_id("../evil"));
        assert!(!valid_image_id(""));
    }

    #[test]
    fn queries_are_escaped() {
        assert_eq!(apicalypse_string(r#"Tom "Clancy" \ x"#), r#"Tom \"Clancy\" \\ x"#);
        assert_eq!(apicalypse_string("a\nb"), "ab");
    }

    #[test]
    fn api_games_convert() {
        let game = ApiGame {
            id: 7,
            name: Some(" Celeste ".into()),
            cover: Some(ApiCover { image_id: Some("co3byy".into()) }),
            first_release_date: Some(1516665600),
            external_games: vec![],
        };
        assert_eq!(to_hit(game), Some(GameHit { id: 7, name: "Celeste".into(), cover: Some("co3byy".into()), year: Some(2018), steam: None }));
        let unnamed = ApiGame { id: 8, name: None, cover: None, first_release_date: None, external_games: vec![] };
        assert_eq!(to_hit(unnamed), None);
    }

    #[test]
    fn hits_from_the_server_are_checked() {
        let hits = vec![
            GameHit { id: 1, name: "Ok".into(), cover: Some("co1".into()), year: None, steam: Some(504230) },
            GameHit { id: 2, name: "Bad cover".into(), cover: Some("https://evil".into()), year: None, steam: Some(0) },
            GameHit { id: 3, name: " ".into(), cover: None, year: None, steam: None },
        ];
        let safe = safe_hits(hits);
        assert_eq!(safe.len(), 2);
        assert_eq!(safe[0].cover.as_deref(), Some("co1"));
        assert_eq!(safe[1].cover, None);
        assert_eq!(safe[0].steam, Some(504230));
        assert_eq!(safe[1].steam, None);
    }

    fn external(source: Option<u64>, uid: &str, url: &str) -> ApiExternalGame {
        ApiExternalGame { external_game_source: source, uid: Some(uid.into()), url: Some(url.into()) }
    }

    #[test]
    fn steam_store_id_comes_from_the_steam_entry() {
        let listed = vec![
            external(Some(5), "gog-celeste", "https://www.gog.com/game/celeste"),
            external(Some(STEAM_SOURCE), "504230", "https://store.steampowered.com/app/504230"),
        ];
        assert_eq!(steam_of(&listed), Some(504230));
        let url_only = vec![external(None, "x", "https://store.steampowered.com/app/367520/Hollow_Knight/")];
        assert_eq!(steam_of(&url_only), Some(367520));
        assert_eq!(steam_of(&[external(Some(STEAM_SOURCE), "12ab", "https://store.steampowered.com.evil/app/9")]), None);
        assert_eq!(steam_of(&[external(Some(STEAM_SOURCE), "99999999999", "")]), None);
        assert_eq!(steam_of(&[external(Some(STEAM_SOURCE), "0", "")]), None);
        assert_eq!(steam_of(&[]), None);
    }

    #[test]
    fn hits_without_a_steam_field_still_parse() {
        let hit: GameHit = serde_json::from_str(r#"{"id":1,"name":"Old","cover":null,"year":null}"#).unwrap();
        assert_eq!(hit.steam, None);
    }

    #[test]
    fn credentials_need_both_parts() {
        assert!(!IgdbCredentials::default().configured());
        assert!(!IgdbCredentials { client_id: "x".into(), client_secret: " ".into() }.configured());
        assert!(IgdbCredentials { client_id: "x".into(), client_secret: "y".into() }.configured());
    }
}
