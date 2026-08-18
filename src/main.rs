#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod config;
mod media;
mod server;
mod tray;

use std::process::ExitCode;

use config::Config;

#[tokio::main]
async fn main() -> ExitCode {
    let config = match Config::load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("failed to load configuration: {error}");
            return ExitCode::FAILURE;
        }
    };

    init_logging(&config.log_level);

    if let Err(error) = app::run(config).await {
        tracing::error!(?error, "application stopped with an error");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

fn init_logging(default_level: &str) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .compact()
        .init();
}
