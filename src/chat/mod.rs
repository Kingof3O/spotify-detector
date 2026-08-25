use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::{Duration, Instant},
};

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::{
    sync::{watch, Mutex},
    time::timeout,
};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::{
    config::{ChatConfig, RequestRole},
    integration::{
        IntegrationState, SpotifyApi, SpotifyError, TwitchApi, TwitchError, TwitchToken,
    },
    media::MediaState,
    server::AppState,
};

const EVENTSUB_URL: &str = "wss://eventsub.wss.twitch.tv/ws";
const MAX_SEEN_MESSAGE_IDS: usize = 1024;
const MAX_CHAT_RESPONSE_LENGTH: usize = 480;

pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move { supervisor(state).await })
}

async fn supervisor(state: AppState) {
    let twitch = TwitchApi::new();
    let spotify = SpotifyApi::new();
    let cooldowns = std::sync::Arc::new(Mutex::new(CooldownState::default()));
    let mut config_rx = state.config.subscribe();
    let mut reconnect_delay = Duration::from_secs(2);

    loop {
        let config = config_rx.borrow().clone();
        if !config.integrations.chat.enabled {
            set_chat_status(&state.integration, false, None, "disabled").await;
            wait_for_change(&state.integration, &mut config_rx).await;
            reconnect_delay = Duration::from_secs(2);
            continue;
        }

        let credentials = match state.integration.credentials.load() {
            Ok(credentials) => credentials,
            Err(error) => {
                set_chat_status(
                    &state.integration,
                    false,
                    None,
                    &format!("error: credential storage: {error}"),
                )
                .await;
                wait_for_change(&state.integration, &mut config_rx).await;
                continue;
            }
        };
        if credentials.twitch.is_none() {
            set_chat_status(&state.integration, false, None, "twitch_not_connected").await;
            wait_for_change(&state.integration, &mut config_rx).await;
            reconnect_delay = Duration::from_secs(2);
            continue;
        }
        match run_session(
            &state,
            &config,
            &twitch,
            &spotify,
            &mut config_rx,
            cooldowns.clone(),
        )
        .await
        {
            SessionResult::ConfigChanged => {
                reconnect_delay = Duration::from_secs(2);
            }
            SessionResult::Stopped(error) => {
                if let Some(error) = error {
                    set_chat_status(&state.integration, false, None, &format!("error: {error}"))
                        .await;
                }
                tokio::select! {
                    _ = tokio::time::sleep(reconnect_delay) => {},
                    _ = state.integration.notify.notified() => {},
                    _ = config_rx.changed() => {},
                }
                reconnect_delay =
                    std::cmp::min(reconnect_delay.saturating_mul(2), Duration::from_secs(30));
            }
        }
    }
}

async fn wait_for_change(
    integration: &IntegrationState,
    config_rx: &mut watch::Receiver<crate::config::Config>,
) {
    tokio::select! {
        _ = integration.notify.notified() => {},
        _ = config_rx.changed() => {},
    }
}

async fn run_session(
    state: &AppState,
    config: &crate::config::Config,
    twitch: &TwitchApi,
    spotify: &SpotifyApi,
    config_rx: &mut watch::Receiver<crate::config::Config>,
    cooldowns: std::sync::Arc<Mutex<CooldownState>>,
) -> SessionResult {
    let client_id = config.integrations.twitch_client_id.trim();
    let channel = config.integrations.twitch_channel.trim();
    if client_id.is_empty() || channel.is_empty() {
        return SessionResult::Stopped(Some(
            "Twitch channel and Twitch Client ID are required".to_owned(),
        ));
    }

    let mut token = match current_twitch_token(state, twitch, client_id).await {
        Ok(token) => token,
        Err(error) => return SessionResult::Stopped(Some(error.to_string())),
    };
    let broadcaster_id = match twitch
        .broadcaster_id(&token.access_token, client_id, channel)
        .await
    {
        Ok(id) => id,
        Err(error) => return SessionResult::Stopped(Some(error.to_string())),
    };
    let mut websocket_url = EVENTSUB_URL.to_owned();
    let mut transferred_subscriptions = false;

    loop {
        let connection = match connect_async(&websocket_url).await {
            Ok((socket, _)) => socket,
            Err(error) => {
                return SessionResult::Stopped(Some(format!("Twitch EventSub connection: {error}")))
            }
        };
        match run_socket(
            state,
            config,
            twitch,
            spotify,
            config_rx,
            cooldowns.clone(),
            connection,
            &token,
            client_id,
            &broadcaster_id,
            transferred_subscriptions,
        )
        .await
        {
            SocketResult::ConfigChanged => return SessionResult::ConfigChanged,
            SocketResult::Reconnect(url) => {
                websocket_url = url;
                transferred_subscriptions = true;
                token = match current_twitch_token(state, twitch, client_id).await {
                    Ok(token) => token,
                    Err(error) => return SessionResult::Stopped(Some(error.to_string())),
                };
            }
            SocketResult::Stopped(error) => return SessionResult::Stopped(error),
        }
    }
}

async fn run_socket(
    state: &AppState,
    config: &crate::config::Config,
    twitch: &TwitchApi,
    spotify: &SpotifyApi,
    config_rx: &mut watch::Receiver<crate::config::Config>,
    cooldowns: std::sync::Arc<Mutex<CooldownState>>,
    mut socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    token: &TwitchToken,
    client_id: &str,
    broadcaster_id: &str,
    transferred_subscriptions: bool,
) -> SocketResult {
    let welcome = loop {
        let incoming = timeout(Duration::from_secs(20), socket.next()).await;
        match incoming {
            Ok(Some(Ok(Message::Text(text)))) => break text.to_string(),
            Ok(Some(Ok(Message::Binary(bytes)))) => match String::from_utf8(bytes.to_vec()) {
                Ok(text) => break text,
                Err(_) => {
                    return SocketResult::Stopped(Some(
                        "Twitch sent a non-UTF-8 EventSub welcome".to_owned(),
                    ))
                }
            },
            Ok(Some(Ok(Message::Ping(payload)))) => {
                if socket.send(Message::Pong(payload)).await.is_err() {
                    return SocketResult::Stopped(Some(
                        "Twitch EventSub welcome ping failed".to_owned(),
                    ));
                }
            }
            Ok(Some(Ok(Message::Pong(_))) | Some(Ok(Message::Frame(_)))) => {}
            Ok(Some(Ok(Message::Close(_))) | None) => {
                return SocketResult::Stopped(Some(
                    "Twitch EventSub closed before welcome".to_owned(),
                ))
            }
            Ok(Some(Err(error))) => {
                return SocketResult::Stopped(Some(format!("Twitch EventSub read: {error}")))
            }
            Err(_) => {
                return SocketResult::Stopped(Some("Twitch EventSub welcome timed out".to_owned()))
            }
        }
    };
    let welcome: EventSubEnvelope = match serde_json::from_str::<EventSubEnvelope>(&welcome) {
        Ok(value) if value.metadata.message_type == "session_welcome" => value,
        Ok(value) => {
            return SocketResult::Stopped(Some(format!(
                "Twitch EventSub sent message type '{}' instead of session_welcome",
                value.metadata.message_type
            )))
        }
        Err(error) => {
            return SocketResult::Stopped(Some(format!(
                "Twitch EventSub welcome was not valid JSON: {error}"
            )))
        }
    };
    let session_id = welcome
        .payload
        .session
        .and_then(|session| session.id)
        .unwrap_or_default();
    if session_id.is_empty() {
        return SocketResult::Stopped(Some("Twitch EventSub welcome had no session ID".to_owned()));
    }
    if !transferred_subscriptions {
        if let Err(error) = twitch
            .subscribe_chat(
                &token.access_token,
                client_id,
                broadcaster_id,
                &token.user_id,
                &session_id,
            )
            .await
        {
            return SocketResult::Stopped(Some(format!(
                "could not subscribe to Twitch chat: {error}"
            )));
        }
    }
    set_chat_status(
        &state.integration,
        true,
        Some(token.display_name.clone()),
        "connected",
    )
    .await;

    let mut seen = SeenMessageIds::default();
    loop {
        tokio::select! {
            _ = state.integration.notify.notified() => return SocketResult::ConfigChanged,
            changed = config_rx.changed() => {
                if changed.is_ok() { return SocketResult::ConfigChanged; }
            }
            incoming = socket.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        match handle_event(state, config, twitch, spotify, cooldowns.clone(), token, client_id, broadcaster_id, &mut seen, &text).await {
                            EventResult::Continue => {},
                            EventResult::Reconnect(url) => return SocketResult::Reconnect(url),
                            EventResult::Error(error) => return SocketResult::Stopped(Some(error)),
                        }
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        if let Ok(text) = String::from_utf8(bytes.to_vec()) {
                            match handle_event(state, config, twitch, spotify, cooldowns.clone(), token, client_id, broadcaster_id, &mut seen, &text).await {
                                EventResult::Continue => {},
                                EventResult::Reconnect(url) => return SocketResult::Reconnect(url),
                                EventResult::Error(error) => return SocketResult::Stopped(Some(error)),
                            }
                        } else {
                            tracing::debug!("ignored non-UTF-8 Twitch EventSub binary frame");
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            return SocketResult::Stopped(Some("Twitch EventSub ping failed".to_owned()));
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        return SocketResult::Stopped(Some("Twitch EventSub disconnected".to_owned()));
                    }
                    Some(Ok(Message::Pong(_))) | Some(Ok(Message::Frame(_))) => {}
                    Some(Err(error)) => return SocketResult::Stopped(Some(format!("Twitch EventSub read: {error}"))),
                }
            }
        }
    }
}

async fn current_twitch_token(
    state: &AppState,
    twitch: &TwitchApi,
    client_id: &str,
) -> Result<TwitchToken, TwitchError> {
    let credentials = state
        .integration
        .credentials
        .load()
        .map_err(|error| TwitchError::Authentication(error.to_string()))?;
    let token = credentials
        .twitch
        .clone()
        .ok_or(TwitchError::Authentication(
            "Twitch is not connected".to_owned(),
        ))?;
    if !token.is_expired() {
        return Ok(token);
    }
    let refreshed = twitch.refresh(client_id, &token).await?;
    let mut updated = credentials;
    updated.twitch = Some(refreshed.clone());
    state
        .integration
        .credentials
        .save(&updated)
        .map_err(|error| TwitchError::Authentication(error.to_string()))?;
    state.integration.signal_change();
    Ok(refreshed)
}

async fn handle_event(
    state: &AppState,
    config: &crate::config::Config,
    twitch: &TwitchApi,
    spotify: &SpotifyApi,
    cooldowns: std::sync::Arc<Mutex<CooldownState>>,
    token: &TwitchToken,
    client_id: &str,
    broadcaster_id: &str,
    seen: &mut SeenMessageIds,
    raw: &str,
) -> EventResult {
    let envelope: EventSubEnvelope = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(_) => return EventResult::Continue,
    };
    if envelope.metadata.message_type == "session_reconnect" {
        return envelope
            .payload
            .session
            .and_then(|session| session.reconnect_url)
            .map(EventResult::Reconnect)
            .unwrap_or_else(|| {
                EventResult::Error("Twitch requested reconnect without a URL".to_owned())
            });
    }
    if envelope.metadata.message_type != "notification"
        || envelope.metadata.subscription_type.as_deref() != Some("channel.chat.message")
    {
        return EventResult::Continue;
    }
    let Some(event) = envelope.payload.event else {
        return EventResult::Continue;
    };
    let Some(delivery_id) = envelope
        .metadata
        .message_id
        .clone()
        .or(event.message_id.clone())
    else {
        return EventResult::Continue;
    };
    if !seen.insert(delivery_id) {
        return EventResult::Continue;
    }
    let reply_message_id = event.message_id.clone();
    if event.chatter_user_id.as_deref() == Some(token.user_id.as_str()) {
        return EventResult::Continue;
    }
    let (command, args) = parse_command(&event.message.text);
    let role = user_role(&event, broadcaster_id);

    if config
        .integrations
        .chat
        .current_song_commands
        .iter()
        .any(|alias| alias.eq_ignore_ascii_case(&command))
    {
        if config.integrations.chat.enabled {
            let response = current_song_response(&state.media.borrow());
            send_chat(
                twitch,
                token,
                client_id,
                broadcaster_id,
                &response,
                reply_message_id.as_deref(),
            )
            .await;
        }
        return EventResult::Continue;
    }
    if !config.integrations.chat.requests_enabled
        || !config
            .integrations
            .chat
            .request_commands
            .iter()
            .any(|alias| alias.eq_ignore_ascii_case(&command))
    {
        return EventResult::Continue;
    }
    if !config.integrations.chat.request_role.allows(role) {
        send_chat(
            twitch,
            token,
            client_id,
            broadcaster_id,
            "Song requests are not available for your role.",
            reply_message_id.as_deref(),
        )
        .await;
        return EventResult::Continue;
    }
    let Some(user_id) = event.chatter_user_id.clone() else {
        return EventResult::Continue;
    };
    if args.is_empty() {
        send_chat(
            twitch,
            token,
            client_id,
            broadcaster_id,
            "Usage: !songrequest <Spotify track link or song search>",
            reply_message_id.as_deref(),
        )
        .await;
        return EventResult::Continue;
    }
    {
        let mut cooldowns = cooldowns.lock().await;
        if let Some(remaining) = cooldowns.reserve(&user_id, role, &config.integrations.chat) {
            send_chat(
                twitch,
                token,
                client_id,
                broadcaster_id,
                &format!(
                    "@{} please wait {}s before requesting another song.",
                    event.chatter_user_name.as_deref().unwrap_or("viewer"),
                    remaining
                ),
                reply_message_id.as_deref(),
            )
            .await;
            return EventResult::Continue;
        }
    }

    let result = async {
        let spotify_client_id = config.integrations.spotify_client_id.trim();
        let track = spotify
            .search_or_resolve(&state.integration, spotify_client_id, args)
            .await?;
        spotify
            .add_to_queue(&state.integration, spotify_client_id, &track.uri)
            .await?;
        Ok::<_, SpotifyError>(track)
    }
    .await;
    match result {
        Ok(track) => {
            cooldowns
                .lock()
                .await
                .commit(&user_id, &config.integrations.chat);
            let response = format!(
                "@{} queued: {} — {}",
                event.chatter_user_name.as_deref().unwrap_or("viewer"),
                track.name,
                track.artist
            );
            send_chat(
                twitch,
                token,
                client_id,
                broadcaster_id,
                &response,
                reply_message_id.as_deref(),
            )
            .await;
        }
        Err(error) => {
            cooldowns.lock().await.release(&user_id);
            send_chat(
                twitch,
                token,
                client_id,
                broadcaster_id,
                &spotify_error_message(error),
                reply_message_id.as_deref(),
            )
            .await;
        }
    }
    EventResult::Continue
}

async fn send_chat(
    twitch: &TwitchApi,
    token: &TwitchToken,
    client_id: &str,
    broadcaster_id: &str,
    response: &str,
    parent: Option<&str>,
) {
    let response = truncate_response(response);
    if let Err(error) = twitch
        .send_chat(
            &token.access_token,
            client_id,
            broadcaster_id,
            &token.user_id,
            &response,
            parent,
        )
        .await
    {
        tracing::warn!(?error, "could not send Twitch chat response");
    }
}

fn current_song_response(state: &MediaState) -> String {
    let Some(title) = state.title.as_deref() else {
        return "Nothing is playing right now.".to_owned();
    };
    let track = match state.artist.as_deref() {
        Some(artist) if !artist.is_empty() => format!("{title} — {artist}"),
        _ => title.to_owned(),
    };
    if state.playing {
        format!("Now playing: {track}")
    } else {
        format!("Paused: {track}")
    }
}

fn spotify_error_message(error: SpotifyError) -> String {
    match error {
        SpotifyError::NotConnected => {
            "Spotify is not connected. Open Setup Twitch & Spotify.".to_owned()
        }
        SpotifyError::InvalidInput(message) => message,
        SpotifyError::NoMatch => "No matching Spotify track was found.".to_owned(),
        SpotifyError::NoDevice => "Spotify has no active playback device.".to_owned(),
        SpotifyError::Authentication(_) => {
            "Spotify authorization expired. Reconnect it in setup.".to_owned()
        }
        SpotifyError::Forbidden(_) => {
            "Spotify rejected the request. Check Premium status and app access.".to_owned()
        }
        SpotifyError::RateLimited(seconds) => format!(
            "Spotify is rate-limited; try again in {}s.",
            seconds.unwrap_or(30)
        ),
        SpotifyError::QuotaExceeded => {
            "Spotify development quota is exhausted; try again later.".to_owned()
        }
        SpotifyError::Api(_, _) | SpotifyError::Network(_) | SpotifyError::Storage(_) => {
            "Spotify could not process that request right now.".to_owned()
        }
    }
}

fn truncate_response(response: &str) -> String {
    if response.len() <= MAX_CHAT_RESPONSE_LENGTH {
        return response.to_owned();
    }
    let mut output = response
        .chars()
        .take(MAX_CHAT_RESPONSE_LENGTH.saturating_sub(1))
        .collect::<String>();
    output.push('…');
    output
}

fn parse_command(text: &str) -> (String, &str) {
    let mut parts = text.trim().splitn(2, char::is_whitespace);
    (
        parts.next().unwrap_or_default().to_ascii_lowercase(),
        parts.next().unwrap_or_default().trim(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum UserRole {
    Everyone,
    Subscriber,
    Vip,
    Moderator,
    Broadcaster,
}

impl RequestRole {
    fn allows(&self, role: UserRole) -> bool {
        let minimum = match self {
            RequestRole::Everyone => UserRole::Everyone,
            RequestRole::Subscriber => UserRole::Subscriber,
            RequestRole::Vip => UserRole::Vip,
            RequestRole::Moderator => UserRole::Moderator,
            RequestRole::Broadcaster => UserRole::Broadcaster,
        };
        role >= minimum
    }
}

fn user_role(event: &ChatEvent, broadcaster_id: &str) -> UserRole {
    if event.chatter_user_id.as_deref() == Some(broadcaster_id) {
        return UserRole::Broadcaster;
    }
    if event.badges.iter().any(|badge| badge.set_id == "moderator") {
        return UserRole::Moderator;
    }
    if event.badges.iter().any(|badge| badge.set_id == "vip") {
        return UserRole::Vip;
    }
    if event
        .badges
        .iter()
        .any(|badge| badge.set_id == "subscriber")
    {
        return UserRole::Subscriber;
    }
    UserRole::Everyone
}

#[derive(Default)]
struct CooldownState {
    global_until: Option<Instant>,
    user_until: HashMap<String, Instant>,
    in_flight: HashSet<String>,
}

impl CooldownState {
    fn reserve(&mut self, user_id: &str, role: UserRole, config: &ChatConfig) -> Option<u64> {
        let now = Instant::now();
        if let Some(until) = self.global_until {
            if until > now {
                return Some(until.saturating_duration_since(now).as_secs().max(1));
            }
        }
        if self.in_flight.contains(user_id) {
            return Some(1);
        }
        if role < UserRole::Moderator {
            if let Some(until) = self.user_until.get(user_id) {
                if *until > now {
                    return Some(until.saturating_duration_since(now).as_secs().max(1));
                }
            }
        }
        self.in_flight.insert(user_id.to_owned());
        if config.request_global_cooldown_secs > 0 {
            self.global_until =
                Some(now + Duration::from_secs(config.request_global_cooldown_secs));
        }
        None
    }

    fn commit(&mut self, user_id: &str, config: &ChatConfig) {
        self.in_flight.remove(user_id);
        if config.request_user_cooldown_secs > 0 {
            self.user_until.insert(
                user_id.to_owned(),
                Instant::now() + Duration::from_secs(config.request_user_cooldown_secs),
            );
        }
    }

    fn release(&mut self, user_id: &str) {
        self.in_flight.remove(user_id);
    }
}

#[derive(Default)]
struct SeenMessageIds {
    ids: HashSet<String>,
    order: VecDeque<String>,
}

impl SeenMessageIds {
    fn insert(&mut self, id: String) -> bool {
        if !self.ids.insert(id.clone()) {
            return false;
        }
        self.order.push_back(id);
        if self.order.len() > MAX_SEEN_MESSAGE_IDS {
            if let Some(old) = self.order.pop_front() {
                self.ids.remove(&old);
            }
        }
        true
    }
}

#[derive(Default, Deserialize)]
struct EventSubEnvelope {
    #[serde(default)]
    metadata: EventSubMetadata,
    #[serde(default)]
    payload: EventSubPayload,
}

#[derive(Default, Deserialize)]
struct EventSubMetadata {
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    message_type: String,
    #[serde(default)]
    subscription_type: Option<String>,
}

#[derive(Default, Deserialize)]
struct EventSubPayload {
    #[serde(default)]
    session: Option<SessionInfo>,
    #[serde(default)]
    event: Option<ChatEvent>,
}

#[derive(Deserialize)]
struct SessionInfo {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    reconnect_url: Option<String>,
}

#[derive(Default, Deserialize)]
struct ChatEvent {
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    chatter_user_id: Option<String>,
    #[serde(default)]
    chatter_user_name: Option<String>,
    #[serde(default)]
    message: ChatMessage,
    #[serde(default)]
    badges: Vec<Badge>,
}

#[derive(Default, Deserialize)]
struct ChatMessage {
    #[serde(default)]
    text: String,
}

#[derive(Default, Deserialize)]
struct Badge {
    #[serde(default)]
    set_id: String,
}

enum EventResult {
    Continue,
    Reconnect(String),
    Error(String),
}

enum SocketResult {
    ConfigChanged,
    Reconnect(String),
    Stopped(Option<String>),
}

enum SessionResult {
    ConfigChanged,
    Stopped(Option<String>),
}

async fn set_chat_status(
    integration: &IntegrationState,
    connected: bool,
    user: Option<String>,
    status: &str,
) {
    let mut value = integration.status.write().await;
    value.twitch_connected = connected;
    value.twitch_user = user;
    value.twitch_status = status.to_owned();
    if !status.starts_with("error") {
        value.last_error = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        current_song_response, parse_command, ChatConfig, CooldownState, EventSubEnvelope,
        SeenMessageIds, UserRole,
    };
    use crate::media::MediaState;

    #[test]
    fn current_song_response_handles_playing_and_paused() {
        let mut state = MediaState::unavailable(0);
        state.title = Some("Song".to_owned());
        state.artist = Some("Artist".to_owned());
        state.playing = true;
        assert_eq!(current_song_response(&state), "Now playing: Song — Artist");
        state.playing = false;
        assert_eq!(current_song_response(&state), "Paused: Song — Artist");
    }

    #[test]
    fn event_ids_are_deduplicated() {
        let mut seen = SeenMessageIds::default();
        assert!(seen.insert("one".to_owned()));
        assert!(!seen.insert("one".to_owned()));
    }

    #[test]
    fn command_parser_is_case_insensitive_and_preserves_query() {
        assert_eq!(
            parse_command("  !SongRequest   Daft Punk  "),
            ("!songrequest".to_owned(), "Daft Punk")
        );
    }

    #[test]
    fn cooldown_allows_moderators_to_bypass_user_timer() {
        let config = ChatConfig {
            request_global_cooldown_secs: 0,
            request_user_cooldown_secs: 300,
            ..ChatConfig::default()
        };
        let mut cooldowns = CooldownState::default();
        assert_eq!(
            cooldowns.reserve("viewer", UserRole::Everyone, &config),
            None
        );
        cooldowns.commit("viewer", &config);
        assert!(cooldowns
            .reserve("viewer", UserRole::Everyone, &config)
            .is_some());
        cooldowns.release("viewer");
        assert_eq!(cooldowns.reserve("mod", UserRole::Moderator, &config), None);
    }

    #[test]
    fn eventsub_notification_reads_nested_subscription_type() {
        let envelope: EventSubEnvelope = serde_json::from_str(
            r#"{"metadata":{"message_id":"delivery-123","message_type":"notification","subscription_type":"channel.chat.message"},"payload":{"subscription":{"type":"channel.chat.message"},"event":{"message_id":"chat-456"}}}"#,
        )
        .expect("notification parses");
        assert_eq!(
            envelope.metadata.subscription_type,
            Some("channel.chat.message".to_owned())
        );
        assert_eq!(
            envelope.metadata.message_id.as_deref(),
            Some("delivery-123")
        );
        assert_eq!(
            envelope
                .payload
                .event
                .and_then(|event| event.message_id)
                .as_deref(),
            Some("chat-456")
        );
    }

    #[test]
    fn eventsub_welcome_fixture_has_session_id() {
        let envelope: EventSubEnvelope = serde_json::from_str(
            r#"{"metadata":{"message_type":"session_welcome"},"payload":{"session":{"id":"session-123","reconnect_url":null}}}"#,
        )
        .expect("welcome parses");
        assert_eq!(
            envelope.payload.session.and_then(|session| session.id),
            Some("session-123".to_owned())
        );
    }
}
