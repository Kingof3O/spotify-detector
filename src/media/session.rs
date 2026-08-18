#![cfg_attr(not(windows), allow(dead_code))]

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaSnapshot {
    pub source_app_user_model_id: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub subtitle: Option<String>,
    pub track_number: Option<u32>,
    pub duration_ms: Option<u64>,
    pub position_ms: Option<u64>,
    pub playing: bool,
}
