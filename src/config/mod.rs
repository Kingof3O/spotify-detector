use std::{fs, net::IpAddr, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceMode {
    SpotifyOnly,
    CurrentMediaSession,
    SpecificApplication,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RequestRole {
    Everyone,
    Subscriber,
    Vip,
    Moderator,
    Broadcaster,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct ChatMessages {
    pub now_playing: String,
    pub paused: String,
    pub nothing_playing: String,
    pub queued: String,
    pub usage: String,
    pub permission_denied: String,
    pub cooldown: String,
    pub request_error: String,
    pub no_match: String,
    pub no_device: String,
    pub spotify_not_connected: String,
    pub spotify_auth_expired: String,
    pub spotify_denied: String,
    pub rate_limited: String,
    pub quota_exceeded: String,
}

impl Default for ChatMessages {
    fn default() -> Self {
        Self {
            now_playing: "Now playing: {track}".to_owned(),
            paused: "Paused: {track}".to_owned(),
            nothing_playing: "Nothing is playing right now.".to_owned(),
            queued: "@{user} queued: {title} — {artist}".to_owned(),
            usage: "Usage: {command} <Spotify track link or song search>".to_owned(),
            permission_denied: "Song requests are not available for your role.".to_owned(),
            cooldown: "@{user} please wait {seconds}s before requesting another song.".to_owned(),
            request_error: "@{user} could not add that song right now.".to_owned(),
            no_match: "No matching Spotify track was found.".to_owned(),
            no_device: "Spotify has no active playback device.".to_owned(),
            spotify_not_connected: "Spotify is not connected. Ask the streamer to open setup."
                .to_owned(),
            spotify_auth_expired:
                "Spotify authorization expired. Ask the streamer to reconnect it.".to_owned(),
            spotify_denied:
                "Spotify rejected the request. Ask the streamer to check Premium and app access."
                    .to_owned(),
            rate_limited: "Spotify is rate-limited; try again in {seconds}s.".to_owned(),
            quota_exceeded: "Spotify request quota is exhausted; try again later.".to_owned(),
        }
    }
}

impl ChatMessages {
    fn validate(&self) -> Result<(), ConfigError> {
        for (field, value) in [
            ("now_playing", &self.now_playing),
            ("paused", &self.paused),
            ("nothing_playing", &self.nothing_playing),
            ("queued", &self.queued),
            ("usage", &self.usage),
            ("permission_denied", &self.permission_denied),
            ("cooldown", &self.cooldown),
            ("request_error", &self.request_error),
            ("no_match", &self.no_match),
            ("no_device", &self.no_device),
            ("spotify_not_connected", &self.spotify_not_connected),
            ("spotify_auth_expired", &self.spotify_auth_expired),
            ("spotify_denied", &self.spotify_denied),
            ("rate_limited", &self.rate_limited),
            ("quota_exceeded", &self.quota_exceeded),
        ] {
            if value.chars().count() > 480 {
                return Err(ConfigError::MessageTooLong {
                    field: field.to_owned(),
                });
            }
        }
        Ok(())
    }
}

impl Default for RequestRole {
    fn default() -> Self {
        Self::Everyone
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct ChatConfig {
    pub enabled: bool,
    pub requests_enabled: bool,
    pub current_song_commands: Vec<String>,
    pub request_commands: Vec<String>,
    pub request_role: RequestRole,
    pub request_user_cooldown_secs: u64,
    pub request_global_cooldown_secs: u64,
    pub messages: ChatMessages,
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            requests_enabled: true,
            current_song_commands: vec!["!song".to_owned(), "!playingnow".to_owned()],
            request_commands: vec!["!sr".to_owned(), "!songrequest".to_owned()],
            request_role: RequestRole::Everyone,
            request_user_cooldown_secs: 300,
            request_global_cooldown_secs: 10,
            messages: ChatMessages::default(),
        }
    }
}

impl ChatConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_aliases(&self.current_song_commands, "current_song_commands")?;
        validate_aliases(&self.request_commands, "request_commands")?;
        if self.request_user_cooldown_secs > 86_400 || self.request_global_cooldown_secs > 86_400 {
            return Err(ConfigError::CooldownOutOfRange);
        }
        self.messages.validate()?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct IntegrationConfig {
    pub twitch_client_id: String,
    pub twitch_channel: String,
    pub spotify_client_id: String,
    pub chat: ChatConfig,
}

impl Default for IntegrationConfig {
    fn default() -> Self {
        Self {
            twitch_client_id: String::new(),
            twitch_channel: String::new(),
            spotify_client_id: String::new(),
            chat: ChatConfig::default(),
        }
    }
}

impl Default for SourceMode {
    fn default() -> Self {
        Self::SpotifyOnly
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub bind_address: String,
    pub port: u16,
    pub source_mode: SourceMode,
    pub specific_app_user_model_id: Option<String>,
    pub log_level: String,
    pub integrations: IntegrationConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1".to_owned(),
            port: 18_923,
            source_mode: SourceMode::SpotifyOnly,
            specific_app_user_model_id: None,
            log_level: "info".to_owned(),
            integrations: IntegrationConfig::default(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self, ConfigError> {
        let Some(path) = find_config_file() else {
            return Ok(Self::default());
        };

        let contents = fs::read_to_string(&path)?;
        let config = serde_json::from_str::<Self>(&contents)
            .map_err(|source| ConfigError::InvalidJson { path, source })?;
        config.validate_for_setup()?;
        Ok(config)
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        self.validate_for_setup()?;
        let path = config_path_for_save();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let contents =
            serde_json::to_string_pretty(self).map_err(|source| ConfigError::Serialize {
                path: path.clone(),
                source,
            })?;
        fs::write(path, format!("{contents}\n"))?;
        Ok(())
    }

    pub(crate) fn validate_for_setup(&self) -> Result<(), ConfigError> {
        self.bind_address
            .parse::<IpAddr>()
            .map_err(|source| ConfigError::InvalidBindAddress {
                value: self.bind_address.clone(),
                source,
            })?;

        if self.port == 0 {
            return Err(ConfigError::InvalidPort);
        }

        if matches!(self.source_mode, SourceMode::SpecificApplication)
            && self
                .specific_app_user_model_id
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
        {
            return Err(ConfigError::MissingSpecificApplication);
        }

        self.integrations.chat.validate()?;

        Ok(())
    }
}

fn find_config_file() -> Option<PathBuf> {
    let mut candidates = Vec::with_capacity(3);

    if let Some(directory) = app_data_directory() {
        candidates.push(directory.join("config.json"));
    }

    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            candidates.push(parent.join("config.json"));
        }
    }

    if let Ok(current_dir) = std::env::current_dir() {
        let current_config = current_dir.join("config.json");
        if !candidates.iter().any(|path| path == &current_config) {
            candidates.push(current_config);
        }
    }

    candidates.into_iter().find(|path| path.is_file())
}

fn config_path_for_save() -> PathBuf {
    if let Some(directory) = app_data_directory() {
        return directory.join("config.json");
    }

    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            return parent.join("config.json");
        }
    }

    PathBuf::from("config.json")
}

fn app_data_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        return std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("SpotifyOverlay"));
    }

    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .map(|path| path.join("spotify-overlay"))
    }
}

fn validate_aliases(aliases: &[String], field: &str) -> Result<(), ConfigError> {
    if aliases.is_empty() || aliases.len() > 8 {
        return Err(ConfigError::InvalidAliases {
            field: field.to_owned(),
        });
    }
    for alias in aliases {
        let normalized = alias.trim();
        if normalized.len() < 2
            || normalized.len() > 32
            || !normalized.starts_with('!')
            || normalized.chars().any(char::is_whitespace)
        {
            return Err(ConfigError::InvalidAlias {
                field: field.to_owned(),
                value: alias.clone(),
            });
        }
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read configuration: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid JSON in {path}: {source}")]
    InvalidJson {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("invalid bind address '{value}': {source}")]
    InvalidBindAddress {
        value: String,
        source: std::net::AddrParseError,
    },
    #[error("port must be between 1 and 65535")]
    InvalidPort,
    #[error("specific_application mode requires specific_app_user_model_id")]
    MissingSpecificApplication,
    #[error("{field} must contain between 1 and 8 commands")]
    InvalidAliases { field: String },
    #[error("invalid command alias '{value}' in {field}")]
    InvalidAlias { field: String, value: String },
    #[error("cooldowns must be between 0 and 86400 seconds")]
    CooldownOutOfRange,
    #[error("bot message '{field}' must be 480 characters or fewer")]
    MessageTooLong { field: String },
    #[error("could not serialize configuration for {path}: {source}")]
    Serialize {
        path: PathBuf,
        source: serde_json::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::{ChatConfig, Config, RequestRole};

    #[test]
    fn defaults_have_expected_chat_commands() {
        let config = Config::default();
        assert_eq!(
            config.integrations.chat.current_song_commands,
            vec!["!song", "!playingnow"]
        );
        assert_eq!(config.integrations.chat.request_role, RequestRole::Everyone);
    }

    #[test]
    fn invalid_aliases_are_rejected() {
        let config = ChatConfig {
            current_song_commands: vec!["song now".to_owned()],
            ..ChatConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn oversized_bot_messages_are_rejected() {
        let mut config = ChatConfig::default();
        config.messages.now_playing = "x".repeat(481);
        assert!(config.validate().is_err());
    }
}
