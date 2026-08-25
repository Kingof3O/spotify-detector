use tokio::sync::watch;

use crate::config::Config;
use crate::integration::IntegrationState;
use crate::media::{ArtworkStore, MediaState};

#[derive(Clone)]
pub struct AppState {
    pub(crate) media: watch::Receiver<MediaState>,
    pub(crate) artwork: ArtworkStore,
    pub(crate) shutdown: watch::Sender<bool>,
    pub(crate) config: watch::Sender<Config>,
    pub(crate) integration: IntegrationState,
    pub(crate) overlay_url: String,
}

impl AppState {
    pub fn new(
        media: watch::Receiver<MediaState>,
        artwork: ArtworkStore,
        shutdown: watch::Sender<bool>,
        config: watch::Sender<Config>,
        integration: IntegrationState,
        overlay_url: String,
    ) -> Self {
        Self {
            media,
            artwork,
            shutdown,
            config,
            integration,
            overlay_url,
        }
    }

    pub(crate) fn config_snapshot(&self) -> Config {
        self.config.borrow().clone()
    }
}
