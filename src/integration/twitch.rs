use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::Deserialize;

use super::{now_seconds, TwitchToken};

const API_BASE: &str = "https://api.twitch.tv/helix";
const OAUTH_BASE: &str = "https://id.twitch.tv/oauth2";

#[derive(Clone)]
pub struct TwitchApi {
    client: Client,
}

impl Default for TwitchApi {
    fn default() -> Self {
        Self::new()
    }
}

impl TwitchApi {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .user_agent("spotify-overlay/0.2")
                .timeout(Duration::from_secs(15))
                .build()
                .expect("Twitch HTTP client should build"),
        }
    }

    pub async fn start_device(&self, client_id: &str) -> Result<TwitchDeviceCode, TwitchError> {
        let response = self
            .client
            .post(format!("{OAUTH_BASE}/device"))
            .form(&[
                ("client_id", client_id),
                ("scopes", "user:read:chat user:write:chat"),
            ])
            .send()
            .await
            .map_err(TwitchError::Network)?;
        if !response.status().is_success() {
            return Err(map_error(response).await);
        }
        response.json().await.map_err(TwitchError::Network)
    }

    pub async fn poll_device(
        &self,
        client_id: &str,
        device_code: &str,
    ) -> Result<DevicePoll, TwitchError> {
        let response = self
            .client
            .post(format!("{OAUTH_BASE}/token"))
            .form(&[
                ("client_id", client_id),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await
            .map_err(TwitchError::Network)?;
        if response.status().is_success() {
            let body = response
                .json::<TokenResponse>()
                .await
                .map_err(TwitchError::Network)?;
            let user = self.user(&body.access_token, client_id).await?;
            return Ok(DevicePoll::Complete(TwitchToken {
                access_token: body.access_token,
                refresh_token: Some(body.refresh_token),
                expires_at: Some(now_seconds().saturating_add(body.expires_in)),
                user_id: user.id,
                login: user.login,
                display_name: user.display_name,
            }));
        }

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status == StatusCode::BAD_REQUEST {
            if let Ok(error) = serde_json::from_str::<OAuthError>(&body) {
                return Ok(match error.message.as_str() {
                    "authorization_pending" => DevicePoll::Pending,
                    "slow_down" => DevicePoll::SlowDown,
                    "access_denied" => DevicePoll::Denied,
                    "expired_token" => DevicePoll::Expired,
                    _ => DevicePoll::Failed(error.message),
                });
            }
        }
        Err(TwitchError::Api(status.as_u16(), body))
    }

    pub async fn refresh(
        &self,
        client_id: &str,
        token: &TwitchToken,
    ) -> Result<TwitchToken, TwitchError> {
        let refresh_token = token
            .refresh_token
            .as_deref()
            .ok_or(TwitchError::Authentication(
                "Twitch token cannot be refreshed".to_owned(),
            ))?;
        let response = self
            .client
            .post(format!("{OAUTH_BASE}/token"))
            .form(&[
                ("client_id", client_id),
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ])
            .send()
            .await
            .map_err(TwitchError::Network)?;
        if !response.status().is_success() {
            return Err(map_error(response).await);
        }
        let body = response
            .json::<TokenResponse>()
            .await
            .map_err(TwitchError::Network)?;
        Ok(TwitchToken {
            access_token: body.access_token,
            refresh_token: Some(body.refresh_token),
            expires_at: Some(now_seconds().saturating_add(body.expires_in)),
            user_id: token.user_id.clone(),
            login: token.login.clone(),
            display_name: token.display_name.clone(),
        })
    }

    pub async fn user(
        &self,
        access_token: &str,
        client_id: &str,
    ) -> Result<TwitchUser, TwitchError> {
        let response = self
            .client
            .get(format!("{API_BASE}/users"))
            .header("Client-Id", client_id)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(TwitchError::Network)?;
        if !response.status().is_success() {
            return Err(map_error(response).await);
        }
        response
            .json::<UsersResponse>()
            .await
            .map_err(TwitchError::Network)?
            .data
            .into_iter()
            .next()
            .ok_or(TwitchError::Authentication(
                "Twitch did not return the bot account".to_owned(),
            ))
    }

    pub async fn broadcaster_id(
        &self,
        access_token: &str,
        client_id: &str,
        channel: &str,
    ) -> Result<String, TwitchError> {
        let response = self
            .client
            .get(format!("{API_BASE}/users"))
            .query(&[("login", channel)])
            .header("Client-Id", client_id)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(TwitchError::Network)?;
        if !response.status().is_success() {
            return Err(map_error(response).await);
        }
        response
            .json::<UsersResponse>()
            .await
            .map_err(TwitchError::Network)?
            .data
            .into_iter()
            .next()
            .map(|user| user.id)
            .ok_or_else(|| TwitchError::InvalidChannel(channel.to_owned()))
    }

    pub async fn subscribe_chat(
        &self,
        access_token: &str,
        client_id: &str,
        broadcaster_id: &str,
        bot_id: &str,
        session_id: &str,
    ) -> Result<(), TwitchError> {
        let response = self
            .client
            .post(format!("{API_BASE}/eventsub/subscriptions"))
            .header("Client-Id", client_id)
            .bearer_auth(access_token)
            .json(&serde_json::json!({
                "type": "channel.chat.message",
                "version": "1",
                "condition": {
                    "broadcaster_user_id": broadcaster_id,
                    "user_id": bot_id,
                },
                "transport": {
                    "method": "websocket",
                    "session_id": session_id,
                }
            }))
            .send()
            .await
            .map_err(TwitchError::Network)?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(map_error(response).await)
        }
    }

    pub async fn send_chat(
        &self,
        access_token: &str,
        client_id: &str,
        broadcaster_id: &str,
        sender_id: &str,
        message: &str,
        reply_parent_message_id: Option<&str>,
    ) -> Result<(), TwitchError> {
        let mut body = serde_json::json!({
            "broadcaster_id": broadcaster_id,
            "sender_id": sender_id,
            "message": message,
        });
        if let Some(parent) = reply_parent_message_id {
            body["reply_parent_message_id"] = serde_json::Value::String(parent.to_owned());
        }
        let response = self
            .client
            .post(format!("{API_BASE}/chat/messages"))
            .header("Client-Id", client_id)
            .bearer_auth(access_token)
            .json(&body)
            .send()
            .await
            .map_err(TwitchError::Network)?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(map_error(response).await)
        }
    }
}

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
pub struct TwitchDeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Clone, Debug)]
pub enum DevicePoll {
    Pending,
    SlowDown,
    Complete(TwitchToken),
    Denied,
    Expired,
    Failed(String),
}

#[derive(Clone, Debug, Deserialize)]
pub struct TwitchUser {
    pub id: String,
    pub login: String,
    pub display_name: String,
}

#[derive(Deserialize)]
struct UsersResponse {
    data: Vec<TwitchUser>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
}

#[derive(Deserialize)]
struct OAuthError {
    message: String,
}

async fn map_error(response: reqwest::Response) -> TwitchError {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    TwitchError::Api(status.as_u16(), body)
}

#[derive(Debug, thiserror::Error)]
pub enum TwitchError {
    #[error("Twitch authentication failed: {0}")]
    Authentication(String),
    #[error("Twitch channel was not found: {0}")]
    InvalidChannel(String),
    #[error("Twitch API returned HTTP {0}: {1}")]
    Api(u16, String),
    #[error("Twitch network request failed: {0}")]
    Network(reqwest::Error),
}
