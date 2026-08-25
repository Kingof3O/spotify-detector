mod credentials;
mod spotify;
mod twitch;

use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::{rngs::OsRng, RngCore};
use serde::Serialize;
use tokio::sync::{Notify, RwLock};

pub use credentials::{CredentialError, CredentialStore, SpotifyToken, TwitchToken};
pub use spotify::{PlaybackStatus, SpotifyApi, SpotifyError};
pub use twitch::{DevicePoll, TwitchApi, TwitchError};

#[derive(Clone)]
pub struct IntegrationState {
    pub credentials: Arc<CredentialStore>,
    pub notify: Arc<Notify>,
    pub csrf_token: String,
    pub status: Arc<RwLock<IntegrationStatus>>,
    pub spotify_pending: Arc<Mutex<Option<SpotifyPending>>>,
    pub twitch_device: Arc<Mutex<Option<TwitchDeviceStatus>>>,
}

impl IntegrationState {
    pub fn new() -> Self {
        Self {
            credentials: Arc::new(CredentialStore::new()),
            notify: Arc::new(Notify::new()),
            csrf_token: random_token(),
            status: Arc::new(RwLock::new(IntegrationStatus::default())),
            spotify_pending: Arc::new(Mutex::new(None)),
            twitch_device: Arc::new(Mutex::new(None)),
        }
    }

    pub fn signal_change(&self) {
        self.notify.notify_waiters();
    }
}

impl Default for IntegrationState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct IntegrationStatus {
    pub twitch_connected: bool,
    pub twitch_user: Option<String>,
    pub twitch_status: String,
    pub spotify_connected: bool,
    pub spotify_status: String,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SpotifyPending {
    pub state: String,
    pub verifier: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub created_at: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct TwitchDeviceStatus {
    pub state: String,
    pub verification_uri: Option<String>,
    pub user_code: Option<String>,
    pub error: Option<String>,
}

pub fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
