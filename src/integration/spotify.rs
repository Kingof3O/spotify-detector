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
            ("limit", "1"),
        ]);
        let response = self
            .send_authenticated(state, client_id, request, false)
            .await?;
        let body = response
            .json::<SearchResponse>()
            .await
            .map_err(SpotifyError::Network)?;
        body.tracks
            .items
            .into_iter()
            .next()
            .map(SpotifyTrack::from)
            .ok_or(SpotifyError::NoMatch)
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
            .query(&[("uri", track_uri)]);
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

#[derive(Deserialize)]
struct TrackResponse {
    uri: String,
    name: String,
    artists: Vec<ArtistResponse>,
}

#[derive(Deserialize)]
struct ArtistResponse {
    name: String,
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
        _ => SpotifyError::Api(status.as_u16(), body),
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
}
