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
            report_fatal_error(&format!("Failed to load configuration:\n\n{error}"));
            return ExitCode::FAILURE;
        }
    };

    init_logging(&config.log_level);

    if let Err(error) = app::run(config).await {
        tracing::error!(?error, "application stopped with an error");
        report_fatal_error(&format!(
            "Spotify OBS Overlay could not start:\n\n{error}\n\nRun restart-and-check.cmd for diagnostics."
        ));
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

fn report_fatal_error(message: &str) {
    eprintln!("{message}");

    #[cfg(all(windows, not(debug_assertions)))]
    {
        use windows::core::{w, PCWSTR};
        use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

        let message = message
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let _ = unsafe {
            MessageBoxW(
                None,
                PCWSTR(message.as_ptr()),
                w!("Spotify OBS Overlay"),
                MB_OK | MB_ICONERROR,
            )
        };
    }
}

fn init_logging(default_level: &str) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_level));

    #[cfg(all(windows, not(debug_assertions)))]
    if let Ok(file) = release_log_file() {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(file))
            .compact()
            .init();
        return;
    }

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .compact()
        .init();
}

#[cfg(all(windows, not(debug_assertions)))]
fn release_log_file() -> std::io::Result<std::fs::File> {
    const MAX_LOG_SIZE: u64 = 1024 * 1024;

    let executable = std::env::current_exe()?;
    let directory = executable
        .parent()
        .ok_or_else(|| std::io::Error::other("executable has no parent directory"))?;
    let path = directory.join("spotify-overlay.log");
    let should_rotate = std::fs::metadata(&path)
        .map(|metadata| metadata.len() >= MAX_LOG_SIZE)
        .unwrap_or(false);

    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true);
    if should_rotate {
        options.truncate(true);
    } else {
        options.append(true);
    }
    options.open(path)
}
