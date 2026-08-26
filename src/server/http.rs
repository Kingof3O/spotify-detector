use axum::{
    body::Body,
    extract::{ConnectInfo, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

use crate::{
    automation::AutomationStatus,
    config::{ChatConfig, Config, LeagueConfig, ObsConfig},
    integration::{
        CredentialError, DevicePoll, IntegrationStatus, PlaybackStatus, SpotifyApi, SpotifyError,
        TwitchApi, TwitchDeviceStatus, TwitchError,
    },
};

use super::{state::AppState, websocket};

const INDEX_HTML: &str = include_str!("../../overlay/index.html");
const TEST_HTML: &str = include_str!("../../overlay/test.html");
const TEST_ARTWORK_SVG: &str = include_str!("../../overlay/test-artwork.svg");
const SETUP_HTML: &str = include_str!("../../overlay/setup.html");
const CHECK_HTML: &str = include_str!("../../overlay/check.html");
const THEME_CSS: &str = include_str!("../../overlay/theme.css");
const SETUP_CSS: &str = include_str!("../../overlay/setup.css");
const SETUP_JS: &str = include_str!("../../overlay/setup.js");
const CHECK_CSS: &str = include_str!("../../overlay/check.css");
const CHECK_JS: &str = include_str!("../../overlay/check.js");
const OVERLAY_CSS: &str = include_str!("../../overlay/overlay.css");
const OVERLAY_JS: &str = include_str!("../../overlay/overlay.js");

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/test", get(test))
        .route("/setup", get(setup))
        .route("/check", get(check))
        .route("/test-artwork.svg", get(test_artwork))
        .route("/theme.css", get(theme_css))
        .route("/setup.css", get(setup_css))
        .route("/setup.js", get(setup_js))
        .route("/check.css", get(check_css))
        .route("/check.js", get(check_js))
        .route("/overlay.css", get(overlay_css))
        .route("/overlay.js", get(overlay_js))
        .route("/health", get(health))
        .route("/api/health/check", get(health_check))
        .route("/api/setup/status", get(setup_status))
        .route("/api/setup/settings", put(save_settings))
        .route("/api/auth/twitch/start", post(start_twitch_auth))
        .route("/api/auth/twitch/status", get(twitch_auth_status))
        .route("/api/auth/twitch/disconnect", post(disconnect_twitch))
        .route("/api/auth/spotify/start", post(start_spotify_auth))
        .route("/auth/spotify/callback", get(spotify_callback))
        .route("/api/auth/spotify/disconnect", post(disconnect_spotify))
        .route("/artwork", get(artwork))
        .route("/ws", get(websocket::upgrade))
        .route("/internal/shutdown", post(shutdown))
        .with_state(state)
}

async fn index() -> axum::response::Html<&'static str> {
    axum::response::Html(INDEX_HTML)
}

async fn test() -> axum::response::Html<&'static str> {
    axum::response::Html(TEST_HTML)
}

async fn setup() -> axum::response::Html<&'static str> {
    axum::response::Html(SETUP_HTML)
}

async fn check() -> axum::response::Html<&'static str> {
    axum::response::Html(CHECK_HTML)
}

async fn test_artwork() -> impl IntoResponse {
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("image/svg+xml; charset=utf-8"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-store, max-age=0"),
            ),
        ],
        TEST_ARTWORK_SVG,
    )
}

async fn theme_css() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/css; charset=utf-8"),
        )],
        THEME_CSS,
    )
}

async fn setup_css() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/css; charset=utf-8"),
        )],
        SETUP_CSS,
    )
}

async fn setup_js() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/javascript; charset=utf-8"),
        )],
        SETUP_JS,
    )
}

async fn check_css() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/css; charset=utf-8"),
        )],
        CHECK_CSS,
    )
}

async fn check_js() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/javascript; charset=utf-8"),
        )],
        CHECK_JS,
    )
}

async fn overlay_css() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/css; charset=utf-8"),
        )],
        OVERLAY_CSS,
    )
}

async fn overlay_js() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/javascript; charset=utf-8"),
        )],
        OVERLAY_JS,
    )
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    available: bool,
    source: Option<String>,
    automation: AutomationStatus,
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let current = state.media.borrow().clone();
    Json(HealthResponse {
        status: "ok",
        available: current.available,
        source: current.source,
        automation: state.automation.snapshot().await,
    })
}

#[derive(Deserialize)]
struct HealthQuery {
    live: Option<u8>,
}

#[derive(Serialize)]
struct HealthCheckReport {
    version: &'static str,
    overall: &'static str,
    summary: String,
    checked_at: u64,
    checks: Vec<HealthCheckItem>,
}

#[derive(Serialize)]
struct HealthCheckItem {
    name: String,
    status: &'static str,
    detail: String,
    action: Option<String>,
}

async fn health_check(
    State(state): State<AppState>,
    Query(query): Query<HealthQuery>,
) -> Json<HealthCheckReport> {
    let live = query.live == Some(1);
    let config = state.config_snapshot();
    let current = state.media.borrow().clone();
    let mut checks = Vec::new();

    add_check(
        &mut checks,
        "Local server",
        "ok",
        format!(
            "Version {} is serving localhost:{}. ",
            env!("CARGO_PKG_VERSION"),
            config.port
        )
        .trim()
        .to_owned(),
        None,
    );
    if current.available {
        let title = current.title.as_deref().unwrap_or("Unknown track");
        add_check(
            &mut checks,
            "Media session",
            "ok",
            format!(
                "Detected {}: {}.",
                current.source.as_deref().unwrap_or("media"),
                title
            ),
            None,
        );
    } else {
        add_check(
            &mut checks,
            "Media session",
            "warning",
            "No usable Windows media metadata is currently available.".to_owned(),
            Some("Start a Spotify track before testing !song.".to_owned()),
        );
    }

    let credentials = state.integration.credentials.load();
    match credentials {
        Ok(credentials) => {
            let twitch_client_id = !config.integrations.twitch_client_id.trim().is_empty();
            let twitch_channel = !config.integrations.twitch_channel.trim().is_empty();
            if twitch_client_id && twitch_channel {
                add_check(
                    &mut checks,
                    "Twitch configuration",
                    "ok",
                    format!(
                        "Client ID and channel @{} are configured.",
                        config.integrations.twitch_channel
                    ),
                    None,
                );
            } else {
                add_check(
                    &mut checks,
                    "Twitch configuration",
                    "error",
                    "Twitch Client ID or target channel is missing.".to_owned(),
                    Some("Open setup and save both Twitch fields.".to_owned()),
                );
            }

            match credentials.twitch {
                Some(token) if live => match TwitchApi::new().validate(&token.access_token).await {
                    Ok(user) => add_check(
                        &mut checks,
                        "Twitch authorization",
                        "ok",
                        format!("Authorized as {}.", user.display_name),
                        None,
                    ),
                    Err(error) => add_check(
                        &mut checks,
                        "Twitch authorization",
                        "error",
                        twitch_health_error(&error),
                        Some("Reconnect the Twitch bot from setup.".to_owned()),
                    ),
                },
                Some(token) => add_check(
                    &mut checks,
                    "Twitch authorization",
                    "ok",
                    format!("Saved authorization for {}.", token.display_name),
                    None,
                ),
                None => add_check(
                    &mut checks,
                    "Twitch authorization",
                    "error",
                    "No Twitch bot authorization is stored.".to_owned(),
                    Some("Authorize the bot account from setup.".to_owned()),
                ),
            }

            if !config.integrations.chat.enabled {
                add_check(
                    &mut checks,
                    "Twitch chat listener",
                    "warning",
                    "Twitch commands are disabled in settings.".to_owned(),
                    Some("Enable Twitch commands and save settings.".to_owned()),
                );
            } else {
                let status = state.integration.status.read().await.clone();
                if status.twitch_connected {
                    add_check(
                        &mut checks,
                        "Twitch chat listener",
                        "ok",
                        format!(
                            "EventSub is connected as {}.",
                            status.twitch_user.unwrap_or_else(|| "bot".to_owned())
                        ),
                        None,
                    );
                } else if status.twitch_status.starts_with("error") {
                    add_check(
                        &mut checks,
                        "Twitch chat listener",
                        "error",
                        status.twitch_status,
                        Some(
                            "Check the error, then reconnect or make the bot a channel moderator."
                                .to_owned(),
                        ),
                    );
                } else {
                    add_check(
                        &mut checks,
                        "Twitch chat listener",
                        "warning",
                        format!(
                            "Listener status: {}.",
                            if status.twitch_status.is_empty() {
                                "starting"
                            } else {
                                &status.twitch_status
                            }
                        ),
                        Some("Wait a few seconds and refresh this page.".to_owned()),
                    );
                }
            }

            if config.integrations.spotify_client_id.trim().is_empty() {
                add_check(
                    &mut checks,
                    "Spotify configuration",
                    "error",
                    "Spotify Client ID is missing.".to_owned(),
                    Some("Enter and save the Spotify Client ID in setup.".to_owned()),
                );
            } else {
                add_check(
                    &mut checks,
                    "Spotify configuration",
                    "ok",
                    "Spotify Client ID is configured.".to_owned(),
                    None,
                );
            }

            match credentials.spotify {
                Some(_) if live => match SpotifyApi::new()
                    .playback_status(&state.integration, &config.integrations.spotify_client_id)
                    .await
                {
                    Ok(PlaybackStatus::Active { name, playing }) => add_check(
                        &mut checks,
                        "Spotify playback device",
                        "ok",
                        format!("Active device: {name} ({}).", if playing { "playing" } else { "paused" }),
                        None,
                    ),
                    Ok(PlaybackStatus::NoActiveDevice) => add_check(
                        &mut checks,
                        "Spotify playback device",
                        "warning",
                        "Spotify authorization works, but no active playback device was found.".to_owned(),
                        Some("Open Spotify Desktop and start playback.".to_owned()),
                    ),
                    Err(error) => add_check(
                        &mut checks,
                        "Spotify playback device",
                        "error",
                        spotify_health_error(&error),
                        Some("Reconnect Spotify to grant playback-read permission.".to_owned()),
                    ),
                },
                Some(_) => add_check(
                    &mut checks,
                    "Spotify authorization",
                    "ok",
                    "Spotify authorization is stored. Run a live check to test the playback device.".to_owned(),
                    None,
                ),
                None => add_check(
                    &mut checks,
                    "Spotify authorization",
                    "error",
                    "No Spotify authorization is stored.".to_owned(),
                    Some("Authorize Spotify from setup.".to_owned()),
                ),
            }
        }
        Err(error) => add_check(
            &mut checks,
            "Credential storage",
            "error",
            format!("Could not read protected credentials: {error}."),
            Some("Restart the app and reconnect the accounts.".to_owned()),
        ),
    }

    let automation = state.automation.snapshot().await;
    if !config.obs.enabled || !config.league.enabled {
        add_check(
            &mut checks,
            "League → OBS automation",
            "warning",
            "League scene automation is disabled in setup.".to_owned(),
            Some("Enable OBS and League automation in setup.".to_owned()),
        );
    } else {
        if automation.obs_connected {
            add_check(
                &mut checks,
                "OBS WebSocket",
                "ok",
                format!(
                    "Connected to OBS {}{}.",
                    automation
                        .obs_version
                        .as_deref()
                        .map(|version| format!("v{version}"))
                        .unwrap_or_else(|| "WebSocket v5".to_owned()),
                    automation
                        .obs_current_scene
                        .as_deref()
                        .map(|scene| format!("; current scene: {scene}"))
                        .unwrap_or_default()
                ),
                None,
            );
        } else {
            add_check(
                &mut checks,
                "OBS WebSocket",
                "error",
                automation
                    .obs_last_error
                    .clone()
                    .unwrap_or_else(|| "OBS WebSocket is not connected.".to_owned()),
                Some("Start OBS and check its WebSocket v5 settings.".to_owned()),
            );
        }

        let missing_scenes = [
            ("game", config.league.game_scene.as_str()),
            ("client", config.league.client_scene.as_str()),
            ("idle", config.league.idle_scene.as_str()),
        ]
        .into_iter()
        .filter(|(_, scene)| {
            !automation
                .obs_scenes
                .iter()
                .any(|candidate| candidate == *scene)
        })
        .map(|(kind, scene)| format!("{kind}='{scene}'"))
        .collect::<Vec<_>>();
        if !automation.obs_connected {
            add_check(
                &mut checks,
                "OBS scene names",
                "warning",
                "Scene list will be validated after OBS connects.".to_owned(),
                Some("Start OBS and refresh this check.".to_owned()),
            );
        } else if !missing_scenes.is_empty() {
            add_check(
                &mut checks,
                "OBS scene names",
                "error",
                format!(
                    "Configured scenes are missing: {}.",
                    missing_scenes.join(", ")
                ),
                Some("Create or rename the scenes in OBS, then refresh this check.".to_owned()),
            );
        } else {
            add_check(
                &mut checks,
                "OBS scene names",
                "ok",
                "All configured League scenes are present in OBS.".to_owned(),
                None,
            );
        }

        add_check(
            &mut checks,
            "League detector",
            if !cfg!(windows) {
                "error"
            } else if automation.league_state == crate::automation::LeagueState::Unknown {
                "warning"
            } else {
                "ok"
            },
            format!(
                "{}State: {}; game window {}; client window {}.",
                if !cfg!(windows) {
                    "Windows League detection is unavailable on this platform. "
                } else {
                    ""
                },
                automation.league_state,
                if automation.league_game_present {
                    "detected"
                } else {
                    "not detected"
                },
                if automation.league_client_present {
                    "detected"
                } else {
                    "not detected"
                }
            ),
            Some("Launch League or review the advanced process signatures in setup.".to_owned()),
        );
    }

    let overall = if checks.iter().any(|check| check.status == "error") {
        "error"
    } else if checks.iter().any(|check| check.status == "warning") {
        "warning"
    } else {
        "ok"
    };
    let summary = match overall {
        "ok" => "All health checks passed.".to_owned(),
        "warning" => "The app is running, but one or more checks need attention.".to_owned(),
        _ => "One or more required checks failed.".to_owned(),
    };
    Json(HealthCheckReport {
        version: env!("CARGO_PKG_VERSION"),
        overall,
        summary,
        checked_at: crate::integration::now_seconds(),
        checks,
    })
}

fn add_check(
    checks: &mut Vec<HealthCheckItem>,
    name: &str,
    status: &'static str,
    detail: String,
    action: Option<String>,
) {
    checks.push(HealthCheckItem {
        name: name.to_owned(),
        status,
        detail,
        action,
    });
}

fn twitch_health_error(error: &TwitchError) -> String {
    match error {
        TwitchError::Api(status, _) => {
            format!("Twitch returned HTTP {status} while validating the bot.")
        }
        TwitchError::Authentication(message) => message.clone(),
        TwitchError::InvalidChannel(channel) => format!("Twitch channel @{channel} was not found."),
        TwitchError::Network(_) => "Twitch could not be reached.".to_owned(),
        TwitchError::MessageDropped(message) => format!("Twitch dropped a chat reply: {message}"),
    }
}

fn spotify_health_error(error: &SpotifyError) -> String {
    match error {
        SpotifyError::Forbidden(_) => {
            "Spotify denied playback-state access; the stored token may need reconnecting."
                .to_owned()
        }
        SpotifyError::Authentication(_) => "Spotify authorization expired.".to_owned(),
        SpotifyError::NotConnected => "Spotify is not authorized.".to_owned(),
        SpotifyError::RateLimited(_) => "Spotify rate-limited the live check.".to_owned(),
        SpotifyError::Network(_) => "Spotify could not be reached.".to_owned(),
        _ => format!("Spotify live check failed: {error}."),
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SetupSettings {
    twitch_client_id: String,
    twitch_channel: String,
    spotify_client_id: String,
    chat: ChatConfig,
    #[serde(default)]
    obs: ObsConfig,
    #[serde(default)]
    league: LeagueConfig,
    #[serde(default, skip_serializing)]
    obs_password: Option<String>,
    #[serde(default, skip_serializing)]
    clear_obs_password: bool,
}

impl SetupSettings {
    fn from_config(config: &Config) -> Self {
        Self {
            twitch_client_id: config.integrations.twitch_client_id.clone(),
            twitch_channel: config.integrations.twitch_channel.clone(),
            spotify_client_id: config.integrations.spotify_client_id.clone(),
            chat: config.integrations.chat.clone(),
            obs: config.obs.clone(),
            league: config.league.clone(),
            obs_password: None,
            clear_obs_password: false,
        }
    }

    fn apply_to(&self, config: &mut Config) {
        config.integrations.twitch_client_id = self.twitch_client_id.trim().to_owned();
        config.integrations.twitch_channel = self.twitch_channel.trim().to_ascii_lowercase();
        config.integrations.spotify_client_id = self.spotify_client_id.trim().to_owned();
        config.integrations.chat = self.chat.clone();
        config.obs = self.obs.clone();
        config.obs.host = config.obs.host.trim().to_owned();
        config.league = self.league.clone();
        config.league.game_scene = config.league.game_scene.trim().to_owned();
        config.league.client_scene = config.league.client_scene.trim().to_owned();
        config.league.idle_scene = config.league.idle_scene.trim().to_owned();
        config.league.game_process_names = config
            .league
            .game_process_names
            .iter()
            .map(|value| value.trim().to_owned())
            .collect();
        config.league.client_process_names = config
            .league
            .client_process_names
            .iter()
            .map(|value| value.trim().to_owned())
            .collect();
        config.league.client_window_classes = config
            .league
            .client_window_classes
            .iter()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .collect();
        config.league.client_window_title_patterns = config
            .league
            .client_window_title_patterns
            .iter()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .collect();
    }
}

#[derive(Serialize)]
struct SetupStatusResponse {
    csrf_token: String,
    settings: SetupSettings,
    status: IntegrationStatus,
    twitch_device: Option<TwitchDeviceStatus>,
    automation: AutomationStatus,
    obs_password_set: bool,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Serialize)]
struct TwitchStartResponse {
    verification_uri: String,
    user_code: String,
}

#[derive(Serialize)]
struct SpotifyStartResponse {
    authorization_url: String,
}

#[derive(Deserialize)]
struct SpotifyCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

async fn setup_status(State(state): State<AppState>) -> Json<SetupStatusResponse> {
    let config = state.config_snapshot();
    let mut status = state.integration.status.read().await.clone();
    let obs_password_set = if let Ok(credentials) = state.integration.credentials.load() {
        if credentials.twitch.is_none() {
            status.twitch_connected = false;
            status.twitch_user = None;
            if !status.twitch_status.starts_with("error") {
                status.twitch_status = "not_authorized".to_owned();
            }
        } else if status.twitch_status.is_empty() {
            status.twitch_status = "authorized_waiting_for_chat".to_owned();
        }
        status.spotify_connected = credentials.spotify.is_some();
        credentials.obs_password.is_some()
    } else {
        false
    };
    let twitch_device = state
        .integration
        .twitch_device
        .lock()
        .ok()
        .and_then(|device| device.clone());
    Json(SetupStatusResponse {
        csrf_token: state.integration.csrf_token.clone(),
        settings: SetupSettings::from_config(&config),
        status,
        twitch_device,
        automation: state.automation.snapshot().await,
        obs_password_set,
    })
}

async fn twitch_auth_status(State(state): State<AppState>) -> Json<Option<TwitchDeviceStatus>> {
    Json(
        state
            .integration
            .twitch_device
            .lock()
            .ok()
            .and_then(|device| device.clone()),
    )
}

async fn save_settings(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(settings): Json<SetupSettings>,
) -> Result<Json<SetupSettings>, (StatusCode, Json<ErrorResponse>)> {
    ensure_setup_request(peer, &headers, &state)?;
    let mut config = state.config_snapshot();
    settings.apply_to(&mut config);
    config.validate_for_setup().map_err(config_error)?;
    config.save().map_err(config_error)?;
    if settings.clear_obs_password {
        state
            .integration
            .credentials
            .clear_obs_password()
            .map_err(storage_error)?;
    } else if let Some(password) = settings.obs_password.as_deref() {
        if !password.is_empty() {
            let mut credentials = state
                .integration
                .credentials
                .load()
                .map_err(storage_error)?;
            credentials.obs_password = Some(password.to_owned());
            state
                .integration
                .credentials
                .save(&credentials)
                .map_err(storage_error)?;
        }
    }
    let response = SetupSettings::from_config(&config);
    let _ = state.config.send(config);
    state.integration.signal_change();
    Ok(Json(response))
}

async fn start_twitch_auth(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<TwitchStartResponse>, (StatusCode, Json<ErrorResponse>)> {
    ensure_setup_request(peer, &headers, &state)?;
    let config = state.config_snapshot();
    let client_id = config.integrations.twitch_client_id.trim().to_owned();
    if client_id.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Enter and save a Twitch Client ID first.",
        ));
    }

    let api = TwitchApi::new();
    let device = api.start_device(&client_id).await.map_err(twitch_error)?;
    let status = TwitchDeviceStatus {
        state: "pending".to_owned(),
        verification_uri: Some(device.verification_uri.clone()),
        user_code: Some(device.user_code.clone()),
        error: None,
    };
    if let Ok(mut current) = state.integration.twitch_device.lock() {
        *current = Some(status);
    }
    state.integration.status.write().await.twitch_status = "waiting_for_authorization".to_owned();
    let integration = state.integration.clone();
    tokio::spawn(async move {
        let mut interval = device.interval.max(2);
        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_secs(device.expires_in);
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
            if tokio::time::Instant::now() >= deadline {
                set_twitch_device_error(&integration, "authorization_expired").await;
                return;
            }
            match api.poll_device(&client_id, &device.device_code).await {
                Ok(DevicePoll::Pending) => {}
                Ok(DevicePoll::SlowDown) => interval = interval.saturating_add(5),
                Ok(DevicePoll::Complete(token)) => match integration.credentials.load() {
                    Ok(mut credentials) => {
                        credentials.twitch = Some(token.clone());
                        if let Err(error) = integration.credentials.save(&credentials) {
                            set_twitch_device_error(
                                &integration,
                                &format!("storage_error: {error}"),
                            )
                            .await;
                            return;
                        }
                        if let Ok(mut current) = integration.twitch_device.lock() {
                            *current = Some(TwitchDeviceStatus {
                                state: "connected".to_owned(),
                                ..Default::default()
                            });
                        }
                        let mut status = integration.status.write().await;
                        status.twitch_connected = true;
                        status.twitch_user = Some(token.display_name);
                        status.twitch_status = "authorized".to_owned();
                        status.last_error = None;
                        integration.signal_change();
                        return;
                    }
                    Err(error) => {
                        set_twitch_device_error(&integration, &format!("storage_error: {error}"))
                            .await;
                        return;
                    }
                },
                Ok(DevicePoll::Denied) => {
                    set_twitch_device_error(&integration, "authorization_denied").await;
                    return;
                }
                Ok(DevicePoll::Expired) => {
                    set_twitch_device_error(&integration, "authorization_expired").await;
                    return;
                }
                Ok(DevicePoll::Failed(error)) => {
                    set_twitch_device_error(&integration, &error).await;
                    return;
                }
                Err(error) => {
                    set_twitch_device_error(&integration, &error.to_string()).await;
                    return;
                }
            }
        }
    });
    Ok(Json(TwitchStartResponse {
        verification_uri: device.verification_uri,
        user_code: device.user_code,
    }))
}

async fn disconnect_twitch(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    ensure_setup_request(peer, &headers, &state)?;
    state
        .integration
        .credentials
        .clear_twitch()
        .map_err(storage_error)?;
    if let Ok(mut device) = state.integration.twitch_device.lock() {
        *device = None;
    }
    state.integration.status.write().await.twitch_connected = false;
    state.integration.status.write().await.twitch_status = "disconnected".to_owned();
    state.integration.signal_change();
    Ok(StatusCode::NO_CONTENT)
}

async fn start_spotify_auth(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SpotifyStartResponse>, (StatusCode, Json<ErrorResponse>)> {
    ensure_setup_request(peer, &headers, &state)?;
    let config = state.config_snapshot();
    let client_id = config.integrations.spotify_client_id.trim().to_owned();
    if client_id.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Enter and save a Spotify Client ID first.",
        ));
    }
    let state_token = crate::integration::random_token();
    let verifier = SpotifyApi::pkce_verifier();
    let redirect_uri = format!(
        "{}/auth/spotify/callback",
        state.overlay_url.trim_end_matches('/')
    );
    let pending = crate::integration::SpotifyPending {
        state: state_token.clone(),
        verifier: verifier.clone(),
        client_id: client_id.clone(),
        redirect_uri: redirect_uri.clone(),
        created_at: crate::integration::now_seconds(),
    };
    if let Ok(mut current) = state.integration.spotify_pending.lock() {
        *current = Some(pending);
    }
    state.integration.status.write().await.spotify_status = "waiting_for_authorization".to_owned();
    Ok(Json(SpotifyStartResponse {
        authorization_url: SpotifyApi::authorization_url(
            &client_id,
            &redirect_uri,
            &state_token,
            &verifier,
        ),
    }))
}

async fn spotify_callback(
    State(state): State<AppState>,
    Query(query): Query<SpotifyCallbackQuery>,
) -> axum::response::Html<String> {
    if let Some(error) = query.error {
        return oauth_result_page(&format!("Spotify authorization failed: {error}"), true);
    }
    let Some(code) = query.code else {
        return oauth_result_page("Spotify did not return an authorization code.", true);
    };
    let Some(returned_state) = query.state else {
        return oauth_result_page("Spotify authorization state was missing.", true);
    };
    let pending = state
        .integration
        .spotify_pending
        .lock()
        .ok()
        .and_then(|mut value| value.take());
    let Some(pending) = pending else {
        return oauth_result_page(
            "Spotify authorization expired. Start again from setup.",
            true,
        );
    };
    if pending.state != returned_state
        || crate::integration::now_seconds().saturating_sub(pending.created_at) > 600
    {
        return oauth_result_page("Spotify authorization state was invalid or expired.", true);
    }
    match SpotifyApi::new()
        .exchange_code(
            &pending.client_id,
            &code,
            &pending.redirect_uri,
            &pending.verifier,
        )
        .await
    {
        Ok(token) => match state.integration.credentials.load() {
            Ok(mut credentials) => {
                credentials.spotify = Some(token);
                match state.integration.credentials.save(&credentials) {
                    Ok(()) => {
                        let mut status = state.integration.status.write().await;
                        status.spotify_connected = true;
                        status.spotify_status = "authorized".to_owned();
                        status.last_error = None;
                        state.integration.signal_change();
                        oauth_result_page(
                            "Spotify connected. You can close this tab and return to setup.",
                            false,
                        )
                    }
                    Err(error) => oauth_result_page(
                        &format!("Could not store Spotify credentials: {error}"),
                        true,
                    ),
                }
            }
            Err(error) => {
                oauth_result_page(&format!("Could not load credential storage: {error}"), true)
            }
        },
        Err(error) => oauth_result_page(&format!("Spotify authorization failed: {error}"), true),
    }
}

async fn disconnect_spotify(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    ensure_setup_request(peer, &headers, &state)?;
    state
        .integration
        .credentials
        .clear_spotify()
        .map_err(storage_error)?;
    let mut status = state.integration.status.write().await;
    status.spotify_connected = false;
    status.spotify_status = "disconnected".to_owned();
    state.integration.signal_change();
    Ok(StatusCode::NO_CONTENT)
}

async fn set_twitch_device_error(state: &crate::integration::IntegrationState, error: &str) {
    if let Ok(mut device) = state.twitch_device.lock() {
        *device = Some(TwitchDeviceStatus {
            state: "error".to_owned(),
            error: Some(error.to_owned()),
            ..Default::default()
        });
    }
    let mut status = state.status.write().await;
    status.twitch_connected = false;
    status.twitch_status = format!("error: {error}");
    status.last_error = Some(error.to_owned());
    state.signal_change();
}

fn ensure_setup_request(
    peer: SocketAddr,
    headers: &HeaderMap,
    state: &AppState,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if !peer.ip().is_loopback() {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "Setup is available only from this computer.",
        ));
    }
    let csrf_valid = headers
        .get("x-spotify-overlay-csrf")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == state.integration.csrf_token);
    if !csrf_valid {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "The setup page session expired. Reload it and try again.",
        ));
    }
    let host_valid = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.starts_with("127.0.0.1:")
                || value.starts_with("localhost:")
                || value.starts_with("[::1]:")
        });
    if !host_valid {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "Invalid local setup host.",
        ));
    }
    if let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        if !(origin.starts_with("http://127.0.0.1:")
            || origin.starts_with("http://localhost:")
            || origin.starts_with("http://[::1]:"))
        {
            return Err(error_response(
                StatusCode::FORBIDDEN,
                "Invalid setup origin.",
            ));
        }
    }
    Ok(())
}

fn config_error(error: crate::config::ConfigError) -> (StatusCode, Json<ErrorResponse>) {
    error_response(StatusCode::BAD_REQUEST, &error.to_string())
}

fn storage_error(error: CredentialError) -> (StatusCode, Json<ErrorResponse>) {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string())
}

fn twitch_error(error: crate::integration::TwitchError) -> (StatusCode, Json<ErrorResponse>) {
    error_response(StatusCode::BAD_GATEWAY, &error.to_string())
}

fn error_response(status: StatusCode, message: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        status,
        Json(ErrorResponse {
            error: message.to_owned(),
        }),
    )
}

fn oauth_result_page(message: &str, error: bool) -> axum::response::Html<String> {
    let color = if error { "#f09b9b" } else { "#9bd5a6" };
    axum::response::Html(format!("<!doctype html><meta charset=\"utf-8\"><title>Spotify Overlay</title><body style=\"background:#111214;color:#f0ddd1;font:16px Segoe UI,Arial;padding:32px\"><p style=\"color:{color}\">{}</p><a href=\"/setup\" style=\"color:#e7bf9c\">Return to setup</a></body>", html_escape(message)))
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

async fn artwork(State(state): State<AppState>) -> Response {
    let Some(snapshot) = state.artwork.snapshot().await else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let mut response = Response::new(Body::from(snapshot.bytes.as_ref().to_vec()));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&snapshot.content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
    response
}

async fn shutdown(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> StatusCode {
    let restart_header_is_valid = headers
        .get("x-spotify-overlay-restart")
        .and_then(|value| value.to_str().ok())
        == Some("1");
    if !peer.ip().is_loopback() || !restart_header_is_valid {
        return StatusCode::FORBIDDEN;
    }

    match state.shutdown.send(true) {
        Ok(()) => StatusCode::ACCEPTED,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}
