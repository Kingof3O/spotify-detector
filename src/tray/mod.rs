#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::spawn;

#[cfg(not(windows))]
pub fn spawn(
    _overlay_url: String,
    _setup_url: String,
    _shutdown_tx: tokio::sync::watch::Sender<bool>,
) -> Option<std::thread::JoinHandle<()>> {
    None
}
