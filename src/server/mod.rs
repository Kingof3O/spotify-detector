mod http;
mod state;
mod websocket;

pub use state::AppState;

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

pub async fn run(
    config: &Config,
    state: AppState,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> Result<(), ServerError> {
    let ip = config.bind_address.parse::<IpAddr>().map_err(|source| {
        ServerError::InvalidBindAddress {
            value: config.bind_address.clone(),
            source,
        }
    })?;
    let address = SocketAddr::from((ip, config.port));
    let router = http::router(state);
    let listener = bind_replacing_existing(address).await?;

    tracing::info!(%address, "local OBS overlay server listening");
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(shutdown_rx))
    .await?;

    tracing::info!("local OBS overlay server stopped");
    Ok(())
}

async fn bind_replacing_existing(address: SocketAddr) -> std::io::Result<tokio::net::TcpListener> {
    match tokio::net::TcpListener::bind(address).await {
        Ok(listener) => return Ok(listener),
        Err(error)
            if error.kind() == std::io::ErrorKind::AddrInUse && address.ip().is_loopback() =>
        {
            tracing::warn!(%address, "another overlay instance is using the local port");
        }
        Err(error) => return Err(error),
    }

    let replacement_requested = tokio::time::timeout(
        Duration::from_secs(1),
        request_existing_instance_shutdown(address),
    )
    .await
    .unwrap_or(false);

    if !replacement_requested {
        return tokio::net::TcpListener::bind(address).await;
    }

    tracing::info!("asked the previous overlay instance to stop; taking over");
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        match tokio::net::TcpListener::bind(address).await {
            Ok(listener) => return Ok(listener),
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(error) => return Err(error),
        }
    }

    tokio::net::TcpListener::bind(address).await
}

async fn request_existing_instance_shutdown(address: SocketAddr) -> bool {
    let Ok(mut stream) = tokio::net::TcpStream::connect(address).await else {
        return false;
    };

    let host = match address.ip() {
        IpAddr::V6(ip) => format!("[{ip}]:{}", address.port()),
        IpAddr::V4(ip) => format!("{ip}:{}", address.port()),
    };
    let request = format!(
        "POST /internal/shutdown HTTP/1.1\r\nHost: {host}\r\nX-Spotify-Overlay-Restart: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    if stream.write_all(request.as_bytes()).await.is_err() {
        return false;
    }

    let mut response = [0_u8; 64];
    let Ok(bytes_read) = stream.read(&mut response).await else {
        return false;
    };
    response[..bytes_read].starts_with(b"HTTP/1.1 202")
}

async fn shutdown_signal(mut shutdown_rx: tokio::sync::watch::Receiver<bool>) {
    tokio::select! {
        _ = ctrl_c_signal() => {}
        changed = shutdown_rx.changed() => {
            if changed.is_ok() && *shutdown_rx.borrow() {
                tracing::info!("shutdown requested");
            }
        }
    }
}

async fn ctrl_c_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::debug!(
            ?error,
            "Ctrl+C handler unavailable; server will stop with the process"
        );
        std::future::pending::<()>().await;
    }
}
