mod artwork;
mod session;
mod spotify;
mod state;

mod manager;

pub use artwork::ArtworkStore;
pub use session::MediaSnapshot;
pub use state::MediaState;

pub(crate) use spotify::source_label;

pub use crate::config::SourceMode;

/// Starts the platform media monitor. The returned thread handle is deliberately
/// owned by the application scope so the monitor stays alive for the process.
pub fn spawn_monitor(
    state_tx: tokio::sync::watch::Sender<MediaState>,
    artwork: ArtworkStore,
    source_mode: SourceMode,
    specific_app_user_model_id: Option<String>,
) -> Option<std::thread::JoinHandle<()>> {
    manager::spawn(state_tx, artwork, source_mode, specific_app_user_model_id)
}
