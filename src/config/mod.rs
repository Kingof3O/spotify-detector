use std::{fs, net::IpAddr, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceMode {
    SpotifyOnly,
    CurrentMediaSession,
    SpecificApplication,
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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1".to_owned(),
            port: 18_923,
            source_mode: SourceMode::SpotifyOnly,
            specific_app_user_model_id: None,
            log_level: "info".to_owned(),
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
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
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

        Ok(())
    }
}

fn find_config_file() -> Option<PathBuf> {
    let mut candidates = Vec::with_capacity(2);

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
}
