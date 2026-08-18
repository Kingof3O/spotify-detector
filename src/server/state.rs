use tokio::sync::watch;

use crate::media::{ArtworkStore, MediaState};

#[derive(Clone)]
pub struct AppState {
    pub(crate) media: watch::Receiver<MediaState>,
    pub(crate) artwork: ArtworkStore,
}

impl AppState {
    pub fn new(media: watch::Receiver<MediaState>, artwork: ArtworkStore) -> Self {
        Self { media, artwork }
    }
}
