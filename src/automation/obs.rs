use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::{
    sync::{mpsc, watch, Mutex},
    time::timeout,
};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

use crate::{
    config::{Config, ObsConfig, ObsManualScenePolicy},
    integration::CredentialStore,
};

use super::{AutomationState, DesiredScene};

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Clone)]
pub struct ObsHandle {
    tx: mpsc::UnboundedSender<DesiredScene>,
}

impl ObsHandle {
    pub fn request_scene(&self, scene: String, force: bool) {
        let _ = self.tx.send(DesiredScene { scene, force });
    }
}

#[derive(Debug, thiserror::Error)]
enum ObsError {
    #[error("OBS connection failed: {0}")]
    Connection(String),
    #[error("OBS protocol error: {0}")]
    Protocol(String),
    #[error("configured OBS scene is missing: {0}")]
    MissingScene(String),
    #[error("OBS authentication failed: {0}")]
    Authentication(String),
    #[error("OBS request failed (HTTP-like code {code}): {comment}")]
    Request { code: u64, comment: String },
    #[error("OBS response timed out")]
    Timeout,
}

#[derive(Clone, Debug)]
enum PendingRequest {
    SetScene(String),
    SceneList,
}

struct ObsConnection {
    sink: Arc<Mutex<SplitSink<Socket, Message>>>,
    stream: SplitStream<Socket>,
    pending: Arc<Mutex<HashMap<String, PendingRequest>>>,
    next_request_id: AtomicU64,
}

enum ConnectedResult {
    ConfigChanged,
    Stopped(ObsError),
}

pub fn spawn(
    config_rx: watch::Receiver<Config>,
    shutdown_rx: watch::Receiver<bool>,
    credentials: Arc<CredentialStore>,
    automation: AutomationState,
) -> (ObsHandle, tokio::task::JoinHandle<()>) {
    let (tx, commands) = mpsc::unbounded_channel();
    let handle = ObsHandle { tx };
    let task = tokio::spawn(async move {
        supervisor(config_rx, shutdown_rx, credentials, automation, commands).await;
    });
    (handle, task)
}

async fn supervisor(
    mut config_rx: watch::Receiver<Config>,
    mut shutdown_rx: watch::Receiver<bool>,
    credentials: Arc<CredentialStore>,
    automation: AutomationState,
    mut commands: mpsc::UnboundedReceiver<DesiredScene>,
) {
    let mut desired_scene: Option<DesiredScene> = None;
    let mut reconnect_delay = Duration::from_millis(500);

    loop {
        if *shutdown_rx.borrow() {
            break;
        }

        let config = config_rx.borrow().clone();
        let automation_enabled = config.obs.enabled && config.league.enabled;
        update_disabled_status(&automation, &config, automation_enabled).await;

        if !cfg!(windows) && automation_enabled {
            tracing::warn!("League → OBS automation requires Windows and has been disabled");
            let mut status = automation.status.write().await;
            status.enabled = false;
            status.obs_connected = false;
            status.obs_status = "unsupported_platform".to_owned();
            drop(status);
            wait_for_shutdown(&mut shutdown_rx).await;
            break;
        }

        if !automation_enabled {
            if !wait_until_enabled_or_stopped(
                &mut config_rx,
                &mut shutdown_rx,
                &mut commands,
                &mut desired_scene,
            )
            .await
            {
                break;
            }
            reconnect_delay = Duration::from_millis(config.obs.reconnect_min_ms.max(100));
            continue;
        }

        let password = match credentials.load() {
            Ok(credentials) => credentials.obs_password,
            Err(error) => {
                set_obs_error(&automation, &format!("credential storage: {error}")).await;
                if !wait_after_failure(
                    &config.obs,
                    reconnect_delay,
                    &mut config_rx,
                    &mut shutdown_rx,
                    &mut commands,
                    &mut desired_scene,
                )
                .await
                {
                    break;
                }
                reconnect_delay = next_delay(reconnect_delay, &config.obs);
                continue;
            }
        };

        match ObsConnection::connect(&config.obs, password.as_deref()).await {
            Ok(mut connection) => {
                reconnect_delay = Duration::from_millis(config.obs.reconnect_min_ms.max(100));
                match bootstrap(&mut connection).await {
                    Ok((version, scenes, current_scene)) => {
                        set_obs_connected(
                            &automation,
                            version,
                            scenes.clone(),
                            current_scene.clone(),
                        )
                        .await;
                        let result = connected_loop(
                            connection,
                            &config,
                            &automation,
                            &mut config_rx,
                            &mut shutdown_rx,
                            &mut commands,
                            &mut desired_scene,
                            scenes,
                            current_scene,
                        )
                        .await;
                        match result {
                            ConnectedResult::ConfigChanged => {
                                reconnect_delay =
                                    Duration::from_millis(config.obs.reconnect_min_ms.max(100));
                                continue;
                            }
                            ConnectedResult::Stopped(error) => {
                                if *shutdown_rx.borrow() {
                                    break;
                                }
                                set_obs_error(&automation, &error.to_string()).await;
                            }
                        }
                    }
                    Err(error) => set_obs_error(&automation, &error.to_string()).await,
                }
            }
            Err(error) => set_obs_error(&automation, &error.to_string()).await,
        }

        if *shutdown_rx.borrow() {
            break;
        }

        if !wait_after_failure(
            &config.obs,
            reconnect_delay,
            &mut config_rx,
            &mut shutdown_rx,
            &mut commands,
            &mut desired_scene,
        )
        .await
        {
            break;
        }
        reconnect_delay = next_delay(reconnect_delay, &config.obs);
    }

    let mut status = automation.status.write().await;
    status.obs_connected = false;
    status.obs_status = "stopped".to_owned();
}

#[allow(clippy::too_many_arguments)]
async fn connected_loop(
    mut connection: ObsConnection,
    config: &Config,
    automation: &AutomationState,
    config_rx: &mut watch::Receiver<Config>,
    shutdown_rx: &mut watch::Receiver<bool>,
    commands: &mut mpsc::UnboundedReceiver<DesiredScene>,
    desired_scene: &mut Option<DesiredScene>,
    mut scenes: Vec<String>,
    mut current_scene: Option<String>,
) -> ConnectedResult {
    if let Some(desired) = desired_scene.clone() {
        if let Err(error) =
            maybe_send_scene(&connection, &scenes, &mut current_scene, desired).await
        {
            if matches!(error, ObsError::MissingScene(_)) {
                set_obs_warning(automation, &error.to_string()).await;
            } else {
                return ConnectedResult::Stopped(error);
            }
        } else if let Some(current) = desired_scene.as_mut() {
            current.force = false;
        }
    }

    loop {
        tokio::select! {
            changed = config_rx.changed() => {
                if changed.is_ok() {
                    return ConnectedResult::ConfigChanged;
                }
                return ConnectedResult::Stopped(ObsError::Protocol("configuration channel closed".to_owned()));
            }
            changed = shutdown_rx.changed() => {
                if changed.is_ok() && *shutdown_rx.borrow() {
                    return ConnectedResult::Stopped(ObsError::Protocol("shutdown requested".to_owned()));
                }
            }
            command = commands.recv() => {
                let Some(command) = command else {
                    return ConnectedResult::Stopped(ObsError::Protocol("scene command channel closed".to_owned()));
                };
                *desired_scene = Some(command.clone());
                if let Err(error) = maybe_send_scene(&connection, &scenes, &mut current_scene, command).await {
                    if matches!(error, ObsError::MissingScene(_)) {
                        set_obs_warning(automation, &error.to_string()).await;
                    } else {
                        return ConnectedResult::Stopped(error);
                    }
                } else if let Some(current) = desired_scene.as_mut() {
                    current.force = false;
                }
            }
            incoming = connection.next_json() => {
                let value = match incoming {
                    Ok(value) => value,
                    Err(error) => return ConnectedResult::Stopped(error),
                };
                if let Err(error) = handle_incoming(
                    &connection,
                    config,
                    automation,
                    desired_scene,
                    &mut scenes,
                    &mut current_scene,
                    value,
                ).await {
                    if matches!(error, ObsError::Connection(_) | ObsError::Timeout) {
                        return ConnectedResult::Stopped(error);
                    }
                    set_obs_warning(automation, &error.to_string()).await;
                }
                update_scene_status(automation, current_scene.clone(), scenes.clone()).await;
            }
        }
    }
}

async fn handle_incoming(
    connection: &ObsConnection,
    config: &Config,
    automation: &AutomationState,
    desired_scene: &Option<DesiredScene>,
    scenes: &mut Vec<String>,
    current_scene: &mut Option<String>,
    value: Value,
) -> Result<(), ObsError> {
    match value.get("op").and_then(Value::as_u64) {
        Some(5) => {
            let data = value.get("d").cloned().unwrap_or(Value::Null);
            match data.get("eventType").and_then(Value::as_str) {
                Some("CurrentProgramSceneChanged") => {
                    *current_scene = data
                        .get("eventData")
                        .and_then(|event| event.get("sceneName"))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned);
                    if matches!(
                        config.obs.manual_scene_policy,
                        ObsManualScenePolicy::Enforce
                    ) {
                        if let Some(desired) = desired_scene.clone() {
                            if desired.scene != current_scene.clone().unwrap_or_default() {
                                if let Err(error) = maybe_send_scene(
                                    connection,
                                    &*scenes,
                                    current_scene,
                                    DesiredScene {
                                        scene: desired.scene,
                                        force: false,
                                    },
                                )
                                .await
                                {
                                    if matches!(error, ObsError::MissingScene(_)) {
                                        set_obs_warning(automation, &error.to_string()).await;
                                    } else {
                                        return Err(error);
                                    }
                                }
                            }
                        }
                    }
                }
                Some("SceneCreated") | Some("SceneRemoved") => {
                    let _ = connection
                        .send_request("GetSceneList", Value::Null, PendingRequest::SceneList)
                        .await?;
                }
                _ => {}
            }
        }
        Some(7) => {
            let data = value.get("d").cloned().unwrap_or(Value::Null);
            let request_id = data
                .get("requestId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let pending = connection.pending.lock().await.remove(&request_id);
            let Some(pending) = pending else {
                return Ok(());
            };
            let response_status = data.get("requestStatus").cloned().unwrap_or(Value::Null);
            if !response_status
                .get("result")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return Err(ObsError::Request {
                    code: response_status
                        .get("code")
                        .and_then(Value::as_u64)
                        .unwrap_or_default(),
                    comment: response_status
                        .get("comment")
                        .and_then(Value::as_str)
                        .unwrap_or("OBS rejected the request")
                        .to_owned(),
                });
            }
            let response_data = data.get("responseData").cloned().unwrap_or(Value::Null);
            match pending {
                PendingRequest::SetScene(scene) => {
                    if desired_scene
                        .as_ref()
                        .is_none_or(|desired| desired.scene == scene)
                    {
                        *current_scene = Some(scene);
                    }
                    let mut status = automation.status.write().await;
                    status.obs_last_error = None;
                    status.obs_status = "connected".to_owned();
                }
                PendingRequest::SceneList => {
                    *scenes = parse_scene_names(&response_data);
                    if let Some(desired) = desired_scene.clone() {
                        if !scenes.is_empty() && !scenes.iter().any(|scene| scene == &desired.scene)
                        {
                            set_obs_warning(
                                automation,
                                &format!(
                                    "configured scene '{}' is not present in OBS",
                                    desired.scene
                                ),
                            )
                            .await;
                        } else if !scenes.is_empty() {
                            maybe_send_scene(connection, &*scenes, current_scene, desired).await?;
                        }
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

async fn maybe_send_scene(
    connection: &ObsConnection,
    scenes: &[String],
    current_scene: &mut Option<String>,
    desired: DesiredScene,
) -> Result<(), ObsError> {
    if !scenes.is_empty() && !scenes.iter().any(|scene| scene == &desired.scene) {
        return Err(ObsError::MissingScene(format!(
            "configured scene '{}' is not present in OBS",
            desired.scene
        )));
    }
    if !desired.force && current_scene.as_deref() == Some(desired.scene.as_str()) {
        return Ok(());
    }
    tracing::info!(scene = %desired.scene, forced = desired.force, "requesting OBS scene change");
    connection
        .send_request(
            "SetCurrentProgramScene",
            json!({ "sceneName": desired.scene }),
            PendingRequest::SetScene(desired.scene.clone()),
        )
        .await?;
    *current_scene = Some(desired.scene);
    Ok(())
}

async fn bootstrap(
    connection: &mut ObsConnection,
) -> Result<(String, Vec<String>, Option<String>), ObsError> {
    let version_value = connection.request_wait("GetVersion", Value::Null).await?;
    let version = version_value
        .get("obsVersion")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let scenes_value = connection.request_wait("GetSceneList", Value::Null).await?;
    let scenes = parse_scene_names(&scenes_value);
    let current_value = connection
        .request_wait("GetCurrentProgramScene", Value::Null)
        .await?;
    let current_scene = current_value
        .get("currentProgramSceneName")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    Ok((version, scenes, current_scene))
}

impl ObsConnection {
    async fn connect(config: &ObsConfig, password: Option<&str>) -> Result<Self, ObsError> {
        let url = format!("ws://{}:{}/", config.host.trim(), config.port);
        let (mut socket, _) = timeout(Duration::from_secs(10), connect_async(&url))
            .await
            .map_err(|_| ObsError::Timeout)?
            .map_err(|error| ObsError::Connection(error.to_string()))?;

        let hello = read_socket_json(&mut socket).await?;
        if hello.get("op").and_then(Value::as_u64) != Some(0) {
            return Err(ObsError::Protocol("OBS did not send Hello".to_owned()));
        }
        let mut identify_data = json!({ "rpcVersion": 1 });
        if let Some(authentication) = hello.get("d").and_then(|data| data.get("authentication")) {
            let challenge = authentication
                .get("challenge")
                .and_then(Value::as_str)
                .ok_or_else(|| ObsError::Protocol("OBS Hello had no challenge".to_owned()))?;
            let salt = authentication
                .get("salt")
                .and_then(Value::as_str)
                .ok_or_else(|| ObsError::Protocol("OBS Hello had no salt".to_owned()))?;
            let Some(password) = password else {
                return Err(ObsError::Authentication(
                    "OBS requires a WebSocket password; save it in setup".to_owned(),
                ));
            };
            identify_data["authentication"] =
                Value::String(obs_authentication(password, salt, challenge));
        }
        socket
            .send(Message::Text(
                json!({ "op": 1, "d": identify_data }).to_string(),
            ))
            .await
            .map_err(|error| ObsError::Connection(error.to_string()))?;
        loop {
            let identified = read_socket_json(&mut socket).await?;
            match identified.get("op").and_then(Value::as_u64) {
                Some(2) => break,
                Some(5) => continue,
                _ => {
                    return Err(ObsError::Authentication(
                        identified
                            .get("d")
                            .and_then(|data| data.get("error"))
                            .and_then(Value::as_str)
                            .unwrap_or("OBS rejected Identify")
                            .to_owned(),
                    ))
                }
            }
        }
        let (sink, stream) = socket.split();
        Ok(Self {
            sink: Arc::new(Mutex::new(sink)),
            stream,
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_request_id: AtomicU64::new(1),
        })
    }

    async fn send_request(
        &self,
        request_type: &str,
        request_data: Value,
        pending: PendingRequest,
    ) -> Result<String, ObsError> {
        let request_id = self
            .next_request_id
            .fetch_add(1, Ordering::Relaxed)
            .to_string();
        self.pending
            .lock()
            .await
            .insert(request_id.clone(), pending);
        let payload = json!({
            "op": 6,
            "d": {
                "requestType": request_type,
                "requestId": request_id,
                "requestData": request_data
            }
        });
        if let Err(error) = self
            .sink
            .lock()
            .await
            .send(Message::Text(payload.to_string()))
            .await
        {
            self.pending.lock().await.remove(&request_id);
            return Err(ObsError::Connection(error.to_string()));
        }
        Ok(request_id)
    }

    async fn request_wait(
        &mut self,
        request_type: &str,
        request_data: Value,
    ) -> Result<Value, ObsError> {
        let request_id = self
            .send_request(request_type, request_data, PendingRequest::SceneList)
            .await?;
        loop {
            let value = timeout(Duration::from_secs(8), self.next_json())
                .await
                .map_err(|_| ObsError::Timeout)??;
            if value.get("op").and_then(Value::as_u64) != Some(7) {
                continue;
            }
            let data = value.get("d").cloned().unwrap_or(Value::Null);
            if data.get("requestId").and_then(Value::as_str) != Some(request_id.as_str()) {
                continue;
            }
            let response_status = data.get("requestStatus").cloned().unwrap_or(Value::Null);
            let pending = self.pending.lock().await.remove(&request_id);
            let _ = pending;
            if !response_status
                .get("result")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return Err(ObsError::Request {
                    code: response_status
                        .get("code")
                        .and_then(Value::as_u64)
                        .unwrap_or_default(),
                    comment: response_status
                        .get("comment")
                        .and_then(Value::as_str)
                        .unwrap_or("OBS rejected the request")
                        .to_owned(),
                });
            }
            return Ok(data.get("responseData").cloned().unwrap_or(Value::Null));
        }
    }

    async fn next_json(&mut self) -> Result<Value, ObsError> {
        loop {
            let Some(message) = self.stream.next().await else {
                return Err(ObsError::Connection("OBS WebSocket closed".to_owned()));
            };
            match message.map_err(|error| ObsError::Connection(error.to_string()))? {
                Message::Text(text) => {
                    return serde_json::from_str(text.as_ref())
                        .map_err(|error| ObsError::Protocol(error.to_string()));
                }
                Message::Binary(bytes) => {
                    return serde_json::from_slice(&bytes)
                        .map_err(|error| ObsError::Protocol(error.to_string()));
                }
                Message::Ping(payload) => {
                    self.sink
                        .lock()
                        .await
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|error| ObsError::Connection(error.to_string()))?;
                }
                Message::Pong(_) | Message::Frame(_) => {}
                Message::Close(_) => {
                    return Err(ObsError::Connection("OBS WebSocket closed".to_owned()));
                }
            }
        }
    }
}

async fn read_socket_json(socket: &mut Socket) -> Result<Value, ObsError> {
    loop {
        let message = timeout(Duration::from_secs(10), socket.next())
            .await
            .map_err(|_| ObsError::Timeout)?
            .ok_or_else(|| ObsError::Connection("OBS WebSocket closed".to_owned()))?
            .map_err(|error| ObsError::Connection(error.to_string()))?;
        match message {
            Message::Text(text) => {
                return serde_json::from_str(text.as_ref())
                    .map_err(|error| ObsError::Protocol(error.to_string()));
            }
            Message::Binary(bytes) => {
                return serde_json::from_slice(&bytes)
                    .map_err(|error| ObsError::Protocol(error.to_string()));
            }
            Message::Ping(payload) => {
                socket
                    .send(Message::Pong(payload))
                    .await
                    .map_err(|error| ObsError::Connection(error.to_string()))?;
            }
            Message::Pong(_) | Message::Frame(_) => {}
            Message::Close(_) => {
                return Err(ObsError::Connection("OBS WebSocket closed".to_owned()));
            }
        }
    }
}

fn obs_authentication(password: &str, salt: &str, challenge: &str) -> String {
    let mut secret_hash = Sha256::new();
    secret_hash.update(password.as_bytes());
    secret_hash.update(salt.as_bytes());
    let secret = STANDARD.encode(secret_hash.finalize());

    let mut auth_hash = Sha256::new();
    auth_hash.update(secret.as_bytes());
    auth_hash.update(challenge.as_bytes());
    STANDARD.encode(auth_hash.finalize())
}

fn parse_scene_names(value: &Value) -> Vec<String> {
    value
        .get("scenes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|scene| scene.get("sceneName").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .collect()
}

async fn update_disabled_status(automation: &AutomationState, config: &Config, enabled: bool) {
    let mut status = automation.status.write().await;
    status.enabled = enabled;
    status.obs_enabled = config.obs.enabled;
    if !config.obs.enabled {
        status.obs_connected = false;
        status.obs_status = "disabled".to_owned();
        status.obs_version = None;
        status.obs_current_scene = None;
        status.obs_scenes.clear();
    } else if !config.league.enabled {
        status.obs_connected = false;
        status.obs_status = "waiting_for_league".to_owned();
        status.obs_version = None;
        status.obs_current_scene = None;
        status.obs_scenes.clear();
    } else if !status.obs_connected {
        status.obs_status = "connecting".to_owned();
    }
}

async fn set_obs_connected(
    automation: &AutomationState,
    version: String,
    scenes: Vec<String>,
    current_scene: Option<String>,
) {
    let mut status = automation.status.write().await;
    status.obs_connected = true;
    status.obs_status = "connected".to_owned();
    status.obs_version = Some(version);
    status.obs_scenes = scenes;
    status.obs_current_scene = current_scene;
    status.obs_last_error = None;
}

async fn update_scene_status(
    automation: &AutomationState,
    current_scene: Option<String>,
    scenes: Vec<String>,
) {
    let mut status = automation.status.write().await;
    status.obs_current_scene = current_scene;
    status.obs_scenes = scenes;
}

async fn set_obs_error(automation: &AutomationState, error: &str) {
    tracing::warn!(error, "OBS automation unavailable");
    let mut status = automation.status.write().await;
    status.obs_connected = false;
    status.obs_status = format!("error: {error}");
    status.obs_version = None;
    status.obs_current_scene = None;
    status.obs_scenes.clear();
    status.obs_last_error = Some(error.to_owned());
}

async fn set_obs_warning(automation: &AutomationState, error: &str) {
    tracing::warn!(error, "OBS request was rejected");
    let mut status = automation.status.write().await;
    status.obs_last_error = Some(error.to_owned());
    if status.obs_connected {
        status.obs_status = "connected_with_warning".to_owned();
    }
}

fn next_delay(current: Duration, config: &ObsConfig) -> Duration {
    let minimum = Duration::from_millis(config.reconnect_min_ms.max(100));
    let maximum = Duration::from_millis(config.reconnect_max_ms.max(minimum.as_millis() as u64));
    current.saturating_mul(2).clamp(minimum, maximum)
}

async fn wait_until_enabled_or_stopped(
    config_rx: &mut watch::Receiver<Config>,
    shutdown_rx: &mut watch::Receiver<bool>,
    commands: &mut mpsc::UnboundedReceiver<DesiredScene>,
    desired_scene: &mut Option<DesiredScene>,
) -> bool {
    loop {
        tokio::select! {
            changed = config_rx.changed() => {
                if changed.is_err() { return false; }
                return true;
            }
            changed = shutdown_rx.changed() => {
                return changed.is_ok() && !*shutdown_rx.borrow();
            }
            command = commands.recv() => {
                let Some(command) = command else { return false; };
                *desired_scene = Some(command);
            }
        }
    }
}

async fn wait_for_shutdown(shutdown_rx: &mut watch::Receiver<bool>) {
    while !*shutdown_rx.borrow() {
        if shutdown_rx.changed().await.is_err() {
            break;
        }
    }
}

async fn wait_after_failure(
    config: &ObsConfig,
    delay: Duration,
    config_rx: &mut watch::Receiver<Config>,
    shutdown_rx: &mut watch::Receiver<bool>,
    commands: &mut mpsc::UnboundedReceiver<DesiredScene>,
    desired_scene: &mut Option<DesiredScene>,
) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(delay) => true,
        changed = config_rx.changed() => changed.is_ok() && !*shutdown_rx.borrow(),
        changed = shutdown_rx.changed() => changed.is_ok() && !*shutdown_rx.borrow(),
        command = commands.recv() => {
            let Some(command) = command else { return false; };
            *desired_scene = Some(command);
            let _ = config;
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{obs_authentication, parse_scene_names};
    use serde_json::json;

    #[test]
    fn obs_authentication_is_deterministic() {
        assert_eq!(
            obs_authentication("secret", "salt", "challenge"),
            "39cfhx7et2iyoMZvoQ6o3OPLNSKgtMmy48GQ7jnvsdE="
        );
    }

    #[test]
    fn scene_list_parser_keeps_scene_names_only() {
        let scenes = parse_scene_names(&json!({
            "scenes": [
                {"sceneName": "League Game"},
                {"sceneName": "League Client", "sceneIndex": 1}
            ]
        }));
        assert_eq!(scenes, ["League Game", "League Client"]);
    }
}
