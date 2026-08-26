use crate::config::Config;
use crate::integration::IntegrationState;
use crate::media::{ArtworkStore, MediaState};
use crate::server::{self, AppState};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("local server failed: {0}")]
    Server(#[from] server::ServerError),
}

pub async fn run(config: Config) -> Result<(), AppError> {
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "spotify overlay starting"
    );

    let artwork = ArtworkStore::new();
    let (state_tx, state_rx) = tokio::sync::watch::channel(MediaState::unavailable(0));
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let (config_tx, _config_rx) = tokio::sync::watch::channel(config.clone());
    let automation_state = crate::automation::AutomationState::new();
    let integration = IntegrationState::new();

    let _media_thread = crate::media::spawn_monitor(
        state_tx,
        artwork.clone(),
        config.source_mode.clone(),
        config.specific_app_user_model_id.clone(),
    );

    let overlay_host = match config.bind_address.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V6(address)) => format!("[{address}]"),
        _ => config.bind_address.clone(),
    };
    let overlay_url = format!("http://{overlay_host}:{}/", config.port);
    let setup_url = format!("{overlay_url}setup");
    let _tray_thread = crate::tray::spawn(overlay_url.clone(), setup_url, shutdown_tx.clone());

    let _automation_handles = crate::automation::spawn(
        config_tx.subscribe(),
        shutdown_tx.subscribe(),
        integration.credentials.clone(),
        automation_state.clone(),
    );

    let server_state = AppState::new(
        state_rx,
        artwork,
        shutdown_tx,
        config_tx,
        integration,
        automation_state,
        overlay_url,
    );
    let _chat_task = crate::chat::spawn(server_state.clone());
    server::run(&config, server_state, shutdown_rx).await?;

    Ok(())
}
