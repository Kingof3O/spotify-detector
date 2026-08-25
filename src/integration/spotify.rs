use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{now_seconds, IntegrationState, SpotifyToken};

const API_BASE: &str = "https://api.spotify.com/v1";
const ACCOUNTS_BASE: &str = "https://accounts.spotify.com";

#[derive(Clone)]
pub struct SpotifyApi {
    client: Client,
}

impl Default for SpotifyApi {
    fn default() -> Self {
        Self::new()
    }
}

impl SpotifyApi {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .user_agent("spotify-overlay/0.2")
                .timeout(Duration::from_secs(15))
                .build()
                .expect("Spotify HTTP client should build"),
        }
    }

    pub fn pkce_verifier() -> String {
        super::random_token()
    }

    pub fn pkce_challenge(verifier: &str) -> String {
        let digest = Sha256::digest(verifier.as_bytes());
        STANDARD
            .encode(digest)
            .replace('+', "-")
            .replace('/', "_")
            .trim_end_matches('=')
            .to_owned()
    }

    pub fn authorization_url(
        client_id: &str,
        redirect_uri: &str,
        state: &str,
        verifier: &str,
    ) -> String {
        let challenge = Self::pkce_challenge(verifier);
        let scope = "user-modify-playback-state user-read-playback-state";
        let query = [
            ("response_type", "code"),
            ("client_id", client_id),
            ("scope", scope),
            ("redirect_uri", redirect_uri),
            ("state", state),
            ("code_challenge_method", "S256"),
            ("code_challenge", challenge.as_str()),
        ];
        format!(
            "{ACCOUNTS_BASE}/authorize?{}",
            serde_urlencoded::to_string(query).unwrap_or_default()
        )
    }

    pub async fn exchange_code(
        &self,
        client_id: &str,
        code: &str,
        redirect_uri: &str,
        verifier: &str,
    ) -> Result<SpotifyToken, SpotifyError> {
        let response = self
            .client
            .post(format!("{ACCOUNTS_BASE}/api/token"))
            .form(&[
                ("client_id", client_id),
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect_uri),
                ("code_verifier", verifier),
            ])
            .send()
            .await
            .map_err(SpotifyError::Network)?;
        parse_token_response(response, None).await
    }

    pub async fn search_or_resolve(
        &self,
        state: &IntegrationState,
        client_id: &str,
        query: &str,
    ) -> Result<SpotifyTrack, SpotifyError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(SpotifyError::InvalidInput(
                "please provide a song title or Spotify track link".to_owned(),
            ));
        }

        if looks_like_spotify_reference(query) && parse_track_id(query).is_none() {
            return Err(SpotifyError::InvalidInput(
                "please provide a Spotify track link, not an album or playlist link".to_owned(),
            ));
        }

        if let Some(track_id) = parse_track_id(query) {
            let request = self.client.get(format!("{API_BASE}/tracks/{track_id}"));
            let response = self
                .send_authenticated(state, client_id, request, false)
                .await?;
            return parse_track(response).await;
        }

        let request = self.client.get(format!("{API_BASE}/search")).query(&[
            ("q", query),
            ("type", "track"),
            ("limit", "15"),
        ]);
        let response = self
            .send_authenticated(state, client_id, request, false)
            .await?;
        let body = response
            .json::<SearchResponse>()
            .await
            .map_err(SpotifyError::Network)?;
        select_best_track_match(query, body.tracks.items).ok_or(SpotifyError::NoMatch)
    }

    pub async fn add_to_queue(
        &self,
        state: &IntegrationState,
        client_id: &str,
        track_uri: &str,
    ) -> Result<(), SpotifyError> {
        let request = self
            .client
            .post(format!("{API_BASE}/me/player/queue"))
            .query(&[("uri", track_uri)])
            .header(reqwest::header::CONTENT_LENGTH, "0")
            .body(Vec::new());
        let response = self
            .send_authenticated(state, client_id, request, true)
            .await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(map_api_error(response, true).await)
        }
    }

    pub async fn playback_status(
        &self,
        state: &IntegrationState,
        client_id: &str,
    ) -> Result<PlaybackStatus, SpotifyError> {
        let request = self.client.get(format!("{API_BASE}/me/player"));
        let response = self
            .send_authenticated(state, client_id, request, false)
            .await?;
        if response.status() == StatusCode::NO_CONTENT {
            return Ok(PlaybackStatus::NoActiveDevice);
        }
        let body = response
            .json::<PlaybackStateResponse>()
            .await
            .map_err(SpotifyError::Network)?;
        let Some(device) = body.device else {
            return Ok(PlaybackStatus::NoActiveDevice);
        };
        Ok(PlaybackStatus::Active {
            name: device.name,
            playing: body.is_playing,
        })
    }

    async fn send_authenticated(
        &self,
        state: &IntegrationState,
        client_id: &str,
        request: RequestBuilder,
        queue_request: bool,
    ) -> Result<Response, SpotifyError> {
        let access_token = self.access_token(state, client_id).await?;
        let retry_request = request.try_clone().ok_or_else(|| {
            SpotifyError::InvalidInput("Spotify request could not be retried".to_owned())
        })?;
        let response = request
            .bearer_auth(&access_token)
            .send()
            .await
            .map_err(SpotifyError::Network)?;
        if response.status() != StatusCode::UNAUTHORIZED {
            if response.status().is_success() {
                return Ok(response);
            }
            return Err(map_api_error(response, queue_request).await);
        }

        let refreshed = self.refresh(state, client_id).await?;
        let response = retry_request
            .bearer_auth(refreshed.access_token)
            .send()
            .await
            .map_err(SpotifyError::Network)?;
        if response.status().is_success() {
            Ok(response)
        } else {
            Err(map_api_error(response, queue_request).await)
        }
    }

    async fn access_token(
        &self,
        state: &IntegrationState,
        client_id: &str,
    ) -> Result<String, SpotifyError> {
        let credentials = state
            .credentials
            .load()
            .map_err(|error| SpotifyError::Storage(error.to_string()))?;
        let token = credentials.spotify.ok_or(SpotifyError::NotConnected)?;
        if token.is_expired() {
            return Ok(self.refresh(state, client_id).await?.access_token);
        }
        Ok(token.access_token)
    }

    async fn refresh(
        &self,
        state: &IntegrationState,
        client_id: &str,
    ) -> Result<SpotifyToken, SpotifyError> {
        let credentials = state
            .credentials
            .load()
            .map_err(|error| SpotifyError::Storage(error.to_string()))?;
        let old = credentials
            .spotify
            .clone()
            .ok_or(SpotifyError::NotConnected)?;
        let response = self
            .client
            .post(format!("{ACCOUNTS_BASE}/api/token"))
            .form(&[
                ("client_id", client_id),
                ("grant_type", "refresh_token"),
                ("refresh_token", old.refresh_token.as_str()),
            ])
            .send()
            .await
            .map_err(SpotifyError::Network)?;
        let token = parse_token_response(response, Some(old.refresh_token)).await?;
        let mut updated = credentials;
        updated.spotify = Some(token.clone());
        state
            .credentials
            .save(&updated)
            .map_err(|error| SpotifyError::Storage(error.to_string()))?;
        state.signal_change();
        Ok(token)
    }
}

#[derive(Clone, Debug)]
pub struct SpotifyTrack {
    pub uri: String,
    pub name: String,
    pub artist: String,
}

#[derive(Clone, Debug)]
pub enum PlaybackStatus {
    Active { name: String, playing: bool },
    NoActiveDevice,
}

impl From<TrackResponse> for SpotifyTrack {
    fn from(track: TrackResponse) -> Self {
        Self {
            uri: track.uri,
            name: track.name,
            artist: track
                .artists
                .into_iter()
                .map(|artist| artist.name)
                .collect::<Vec<_>>()
                .join(", "),
        }
    }
}

#[derive(Deserialize)]
struct SearchResponse {
    tracks: TrackPage,
}

#[derive(Deserialize)]
struct TrackPage {
    items: Vec<TrackResponse>,
}

#[derive(Deserialize, Clone)]
struct TrackResponse {
    uri: String,
    name: String,
    #[serde(default)]
    artists: Vec<ArtistResponse>,
    #[serde(default)]
    popularity: Option<u32>,
}

#[derive(Deserialize, Clone)]
struct ArtistResponse {
    name: String,
}

fn normalize_for_matching(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c.is_whitespace() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn strip_version_noise(title: &str) -> &str {
    let lower = title.to_lowercase();
    let noise_markers = [
        " - remastered", " - live", " (remastered", " (live",
        " (feat.", " (feat ", " (with ", " - feat.", " [feat.",
    ];
    let mut cut_idx = title.len();
    for marker in noise_markers {
        if let Some(pos) = lower.find(marker) {
            if pos < cut_idx {
                cut_idx = pos;
            }
        }
    }
    title[..cut_idx].trim()
}

fn score_track_match(query: &str, track: &TrackResponse, index: usize, total_items: usize) -> i32 {
    let norm_query = normalize_for_matching(query);
    if norm_query.is_empty() {
        return 0;
    }

    let norm_title = normalize_for_matching(&track.name);
    let base_title = normalize_for_matching(strip_version_noise(&track.name));
    let artists_str = track
        .artists
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let norm_artists = normalize_for_matching(&artists_str);
    let full_string = format!("{norm_title} {norm_artists}");

    let mut score = 0i32;

    // 1. Exact Title match (Highest priority)
    if norm_title == norm_query || base_title == norm_query {
        score += 10000;
    }
    // 2. Exact Title + Artist match (user typed song and artist)
    else if full_string == norm_query || format!("{norm_artists} {norm_title}") == norm_query {
        score += 9000;
    }
    // 3. Title starts with query or query starts with title
    else if norm_title.starts_with(&norm_query) || base_title.starts_with(&norm_query) {
        let diff = (norm_title.len() as i32 - norm_query.len() as i32).abs();
        score += (8000 - diff * 20).max(5000);
    } else if norm_query.starts_with(&norm_title) || norm_query.starts_with(&base_title) {
        score += 7500;
    }
    // 4. Title contains query or query contains title
    else if norm_title.contains(&norm_query) || base_title.contains(&norm_query) {
        let diff = (norm_title.len() as i32 - norm_query.len() as i32).abs();
        score += (7000 - diff * 20).max(4000);
    } else if norm_query.contains(&norm_title) && !norm_title.is_empty() {
        score += 6500;
    }

    // 5. Word token matching across title & artist
    let query_tokens: Vec<&str> = norm_query.split_whitespace().collect();
    if !query_tokens.is_empty() {
        let title_tokens: Vec<&str> = norm_title.split_whitespace().collect();
        let full_tokens: Vec<&str> = full_string.split_whitespace().collect();

        let mut matched_title_tokens = 0;
        let mut matched_full_tokens = 0;

        for token in &query_tokens {
            if title_tokens.contains(token) {
                matched_title_tokens += 1;
            }
            if full_tokens.contains(token) {
                matched_full_tokens += 1;
            }
        }

        let full_match_ratio = matched_full_tokens as f32 / query_tokens.len() as f32;
        let title_match_ratio = matched_title_tokens as f32 / query_tokens.len() as f32;

        if title_match_ratio >= 0.99 {
            score += 4000;
        } else if full_match_ratio >= 0.99 {
            score += 3500;
        } else {
            score += (full_match_ratio * 2000.0) as i32;
        }
    }

    // Add popularity bonus if available
    if let Some(pop) = track.popularity {
        score += pop.min(100) as i32;
    }

    // Minor position tie-breaker for Spotify's top rankings
    score += (total_items.saturating_sub(index)) as i32 * 5;

    score
}

fn select_best_track_match(query: &str, items: Vec<TrackResponse>) -> Option<SpotifyTrack> {
    if items.is_empty() {
        return None;
    }

    let total = items.len();
    items
        .into_iter()
        .enumerate()
        .max_by_key(|(idx, track)| score_track_match(query, track, *idx, total))
        .map(|(_, track)| SpotifyTrack::from(track))
}

#[derive(Deserialize)]
struct PlaybackStateResponse {
    #[serde(default)]
    is_playing: bool,
    device: Option<PlaybackDevice>,
}

#[derive(Deserialize)]
struct PlaybackDevice {
    name: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    scope: String,
}

async fn parse_token_response(
    response: Response,
    previous_refresh_token: Option<String>,
) -> Result<SpotifyToken, SpotifyError> {
    if !response.status().is_success() {
        return Err(map_api_error(response, false).await);
    }
    let body = response
        .json::<TokenResponse>()
        .await
        .map_err(SpotifyError::Network)?;
    Ok(SpotifyToken {
        access_token: body.access_token,
        refresh_token: body
            .refresh_token
            .or(previous_refresh_token)
            .ok_or_else(|| {
                SpotifyError::Authentication("Spotify did not return a refresh token".to_owned())
            })?,
        expires_at: now_seconds().saturating_add(body.expires_in),
        scope: body.scope,
    })
}

async fn parse_track(response: Response) -> Result<SpotifyTrack, SpotifyError> {
    let body = if response.status().is_success() {
        response
            .json::<TrackResponse>()
            .await
            .map_err(SpotifyError::Network)?
    } else {
        return Err(map_api_error(response, false).await);
    };
    Ok(SpotifyTrack::from(body))
}

async fn map_api_error(response: Response, queue_request: bool) -> SpotifyError {
    let status = response.status();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let body = response.text().await.unwrap_or_default();
    let reason = serde_json::from_str::<ApiErrorBody>(&body)
        .ok()
        .map(|body| body.error.reason)
        .unwrap_or_default();
    let logged_body = body.chars().take(1024).collect::<String>();
    match status {
        StatusCode::UNAUTHORIZED => {
            SpotifyError::Authentication("Spotify authorization expired".to_owned())
        }
        StatusCode::FORBIDDEN => SpotifyError::Forbidden(
            "Spotify rejected the request; Premium and app access may be required".to_owned(),
        ),
        StatusCode::NOT_FOUND if queue_request => SpotifyError::NoDevice,
        StatusCode::NOT_FOUND => SpotifyError::NoMatch,
        StatusCode::TOO_MANY_REQUESTS if reason == "QUOTA_EXCEEDED" => SpotifyError::QuotaExceeded,
        StatusCode::TOO_MANY_REQUESTS => SpotifyError::RateLimited(retry_after),
        _ => SpotifyError::Api(status.as_u16(), logged_body),
    }
}

#[derive(Deserialize)]
struct ApiErrorBody {
    error: ApiError,
}

#[derive(Deserialize)]
struct ApiError {
    #[serde(default)]
    reason: String,
}

fn parse_track_id(value: &str) -> Option<String> {
    if let Some(id) = value.strip_prefix("spotify:track:") {
        return valid_track_id(id);
    }
    let without_query = value.split(['?', '#']).next().unwrap_or(value);
    let marker = "open.spotify.com/track/";
    let start = without_query
        .find(marker)
        .map(|index| index + marker.len())?;
    valid_track_id(without_query[start..].split('/').next().unwrap_or_default())
}

fn looks_like_spotify_reference(value: &str) -> bool {
    value.starts_with("spotify:") || value.contains("spotify.com/")
}

fn valid_track_id(value: &str) -> Option<String> {
    (value.len() >= 10
        && value.len() <= 64
        && value.chars().all(|char| char.is_ascii_alphanumeric()))
    .then(|| value.to_owned())
}

#[derive(Debug, thiserror::Error)]
pub enum SpotifyError {
    #[error("Spotify is not connected")]
    NotConnected,
    #[error("invalid Spotify request: {0}")]
    InvalidInput(String),
    #[error("no matching Spotify track was found")]
    NoMatch,
    #[error("Spotify has no active playback device")]
    NoDevice,
    #[error("Spotify authorization failed: {0}")]
    Authentication(String),
    #[error("Spotify rejected the request: {0}")]
    Forbidden(String),
    #[error("Spotify rate limit reached; retry in {0:?} seconds")]
    RateLimited(Option<u64>),
    #[error("Spotify development quota has been exceeded")]
    QuotaExceeded,
    #[error("Spotify API returned HTTP {0}: {1}")]
    Api(u16, String),
    #[error("Spotify network request failed: {0}")]
    Network(reqwest::Error),
    #[error("Spotify credential storage failed: {0}")]
    Storage(String),
}

#[cfg(test)]
mod tests {
    use super::{looks_like_spotify_reference, parse_track_id, SpotifyApi};

    #[test]
    fn parses_spotify_track_links_and_uris() {
        assert_eq!(
            parse_track_id("spotify:track:1234567890"),
            Some("1234567890".to_owned())
        );
        assert_eq!(
            parse_track_id("https://open.spotify.com/track/1234567890?si=abc"),
            Some("1234567890".to_owned())
        );
        assert_eq!(
            parse_track_id("https://open.spotify.com/album/1234567890"),
            None
        );
        assert!(looks_like_spotify_reference("spotify:album:1234567890"));
    }

    #[test]
    fn pkce_challenge_is_stable() {
        assert_eq!(
            SpotifyApi::pkce_challenge("test-verifier"),
            "JBbiqONGWPaAmwXk_8bT6UnlPfrn65D32eZlJS-zGG0"
        );
    }

    #[test]
    fn selects_accurate_track_over_unrelated_popular_results() {
        use super::{select_best_track_match, ArtistResponse, TrackResponse};

        let items = vec![
            TrackResponse {
                uri: "spotify:track:trending1".to_owned(),
                name: "back to friends".to_owned(),
                artists: vec![ArtistResponse {
                    name: "sombr".to_owned(),
                }],
                popularity: Some(90),
            },
            TrackResponse {
                uri: "spotify:track:target2".to_owned(),
                name: "The Cut That Always Bleeds".to_owned(),
                artists: vec![ArtistResponse {
                    name: "Conan Gray".to_owned(),
                }],
                popularity: Some(75),
            },
            TrackResponse {
                uri: "spotify:track:other3".to_owned(),
                name: "Always Bleeds".to_owned(),
                artists: vec![ArtistResponse {
                    name: "Various Artists".to_owned(),
                }],
                popularity: Some(20),
            },
        ];

        let matched = select_best_track_match("the cut that always bleeds", items).unwrap();
        assert_eq!(matched.uri, "spotify:track:target2");
        assert_eq!(matched.name, "The Cut That Always Bleeds");
        assert_eq!(matched.artist, "Conan Gray");
    }
}
