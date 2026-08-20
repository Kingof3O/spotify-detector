use tokio::sync::watch;

use crate::media::{ArtworkStore, MediaState};

#[derive(Clone)]
pub struct AppState {
    pub(crate) media: watch::Receiver<MediaState>,
    pub(crate) artwork: ArtworkStore,
    pub(crate) shutdown: watch::Sender<bool>,
}

impl AppState {
    pub fn new(
        media: watch::Receiver<MediaState>,
        artwork: ArtworkStore,
        shutdown: watch::Sender<bool>,
    ) -> Self {
        Self {
            media,
            artwork,
            shutdown,
        }
    }
}
