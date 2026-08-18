mod http;
mod state;
mod websocket;

pub use state::AppState;

use std::net::{IpAddr, SocketAddr};

use crate::config::Config;

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("invalid bind address '{value}': {source}")]
    InvalidBindAddress {
        value: String,
        source: std::net::AddrParseError,
    },
    #[error("local server I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

pub async fn run(config: &Config, state: AppState) -> Result<(), ServerError> {
    let ip = config.bind_address.parse::<IpAddr>().map_err(|source| {
        ServerError::InvalidBindAddress {
            value: config.bind_address.clone(),
            source,
        }
    })?;
    let address = SocketAddr::from((ip, config.port));
    let router = http::router(state);
    let listener = tokio::net::TcpListener::bind(address).await?;

    tracing::info!(%address, "local OBS overlay server listening");
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("local OBS overlay server stopped");
    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::debug!(
            ?error,
            "Ctrl+C handler unavailable; server will stop with the process"
        );
        std::future::pending::<()>().await;
    }
}
