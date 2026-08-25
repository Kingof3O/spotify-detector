use std::{
    fs,
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Credentials {
    #[serde(default)]
    pub twitch: Option<TwitchToken>,
    #[serde(default)]
    pub spotify: Option<SpotifyToken>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TwitchToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<u64>,
    pub user_id: String,
    pub login: String,
    pub display_name: String,
}

impl TwitchToken {
    pub fn is_expired(&self) -> bool {
        self.expires_at
            .is_some_and(|expires_at| now_seconds().saturating_add(60) >= expires_at)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SpotifyToken {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
    pub scope: String,
}

impl SpotifyToken {
    pub fn is_expired(&self) -> bool {
        now_seconds().saturating_add(60) >= self.expires_at
    }
}

#[derive(Debug)]
pub struct CredentialStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl CredentialStore {
    pub fn new() -> Self {
        Self {
            path: credentials_path(),
            lock: Mutex::new(()),
        }
    }

    pub fn load(&self) -> Result<Credentials, CredentialError> {
        let _guard = self.lock.lock().map_err(|_| CredentialError::Poisoned)?;
        if !self.path.is_file() {
            return Ok(Credentials::default());
        }

        let encrypted = fs::read(&self.path)?;
        let plaintext = unprotect(&encrypted)?;
        serde_json::from_slice(&plaintext).map_err(CredentialError::InvalidJson)
    }

    pub fn save(&self, credentials: &Credentials) -> Result<(), CredentialError> {
        let _guard = self.lock.lock().map_err(|_| CredentialError::Poisoned)?;
        let plaintext = serde_json::to_vec(credentials).map_err(CredentialError::Serialize)?;
        let encrypted = protect(&plaintext)?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, encrypted)?;
        Ok(())
    }

    pub fn clear_twitch(&self) -> Result<(), CredentialError> {
        let mut credentials = self.load()?;
        credentials.twitch = None;
        self.save(&credentials)
    }

    pub fn clear_spotify(&self) -> Result<(), CredentialError> {
        let mut credentials = self.load()?;
        credentials.spotify = None;
        self.save(&credentials)
    }
}

impl Default for CredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

fn credentials_path() -> PathBuf {
    #[cfg(windows)]
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local_app_data)
            .join("SpotifyOverlay")
            .join("credentials.bin");
    }

    #[cfg(not(windows))]
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(config_home)
            .join("spotify-overlay")
            .join("credentials.bin");
    }

    PathBuf::from("credentials.bin")
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(windows)]
fn protect(plaintext: &[u8]) -> Result<Vec<u8>, CredentialError> {
    use windows::Win32::{
        Foundation::{LocalFree, HLOCAL},
        Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB},
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: plaintext.len() as u32,
        pbData: plaintext.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe { CryptProtectData(&input, None, None, None, None, 0, &mut output) }
        .map_err(|error| CredentialError::Protection(error.to_string()))?;
    if output.cbData == 0 || output.pbData.is_null() {
        return Err(CredentialError::Protection("empty DPAPI output".to_owned()));
    }

    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe { LocalFree(Some(HLOCAL(output.pbData.cast()))) };
    Ok(result)
}

#[cfg(not(windows))]
fn protect(plaintext: &[u8]) -> Result<Vec<u8>, CredentialError> {
    // The real released application is Windows-only and uses DPAPI. Keeping a
    // plain test store on development hosts lets the local server compile and
    // exercise its setup routes without pretending macOS has DPAPI.
    Ok(plaintext.to_vec())
}

#[cfg(windows)]
fn unprotect(encrypted: &[u8]) -> Result<Vec<u8>, CredentialError> {
    use windows::Win32::{
        Foundation::{LocalFree, HLOCAL},
        Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB},
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: encrypted.len() as u32,
        pbData: encrypted.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe { CryptUnprotectData(&input, None, None, None, None, 0, &mut output) }
        .map_err(|error| CredentialError::Protection(error.to_string()))?;
    if output.cbData == 0 || output.pbData.is_null() {
        return Err(CredentialError::Protection("empty DPAPI output".to_owned()));
    }

    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe { LocalFree(Some(HLOCAL(output.pbData.cast()))) };
    Ok(result)
}

#[cfg(not(windows))]
fn unprotect(encrypted: &[u8]) -> Result<Vec<u8>, CredentialError> {
    Ok(encrypted.to_vec())
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("credential storage I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("credential storage mutex was poisoned")]
    Poisoned,
    #[error("credential storage contains invalid JSON: {0}")]
    InvalidJson(serde_json::Error),
    #[error("could not serialize credentials: {0}")]
    Serialize(serde_json::Error),
    #[allow(dead_code)]
    #[error("DPAPI credential protection failed: {0}")]
    Protection(String),
}
