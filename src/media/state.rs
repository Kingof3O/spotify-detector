#![cfg_attr(not(windows), allow(dead_code))]

use serde::Serialize;

use super::{source_label, MediaSnapshot};

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct MediaState {
    #[serde(rename = "type")]
    pub message_type: &'static str,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_app_user_model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album_artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_number: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_ms: Option<u64>,
    pub playing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artwork_url: Option<String>,
    pub artwork_version: u64,
    pub timestamp: u64,
}

impl MediaState {
    pub fn unavailable(artwork_version: u64) -> Self {
        Self {
            message_type: "state",
            available: false,
            source: None,
            source_app_user_model_id: None,
            title: None,
            artist: None,
            album: None,
            album_artist: None,
            subtitle: None,
            track_number: None,
            duration_ms: None,
            position_ms: None,
            playing: false,
            artwork_url: None,
            artwork_version,
            timestamp: now_ms(),
        }
    }

    pub fn from_snapshot(snapshot: MediaSnapshot, artwork_version: Option<u64>) -> Self {
        let title = clean(snapshot.title);
        let artist = clean(snapshot.artist);
        let album = clean(snapshot.album);
        let album_artist = clean(snapshot.album_artist);
        let subtitle = clean(snapshot.subtitle);
        let available = title.is_some() || artist.is_some();

        let duration_ms = snapshot.duration_ms.filter(|duration| *duration > 0);
        let position_ms = snapshot.position_ms.map(|position| {
            duration_ms
                .map(|duration| position.min(duration))
                .unwrap_or(position)
        });
        let artwork_url = artwork_version
            .filter(|version| *version > 0)
            .map(|version| format!("/artwork?v={version}"));

        Self {
            message_type: "state",
            available,
            source: Some(source_label(&snapshot.source_app_user_model_id)),
            source_app_user_model_id: clean(Some(snapshot.source_app_user_model_id)),
            title,
            artist,
            album,
            album_artist,
            subtitle,
            track_number: snapshot.track_number,
            duration_ms,
            position_ms,
            playing: snapshot.playing && available,
            artwork_url,
            artwork_version: artwork_version.unwrap_or_default(),
            timestamp: now_ms(),
        }
    }

    #[cfg(windows)]
    pub fn content_eq(&self, other: &Self) -> bool {
        let mut left = self.clone();
        let mut right = other.clone();
        left.timestamp = 0;
        right.timestamp = 0;
        left == right
    }
}

fn clean(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::MediaState;
    use crate::media::MediaSnapshot;

    fn snapshot() -> MediaSnapshot {
        MediaSnapshot {
            source_app_user_model_id: "SpotifyAB.SpotifyMusic!Spotify".to_owned(),
            title: Some("  Track one  ".to_owned()),
            artist: Some("Artist".to_owned()),
            album: Some("Album".to_owned()),
            album_artist: Some("Album Artist".to_owned()),
            subtitle: Some("Subtitle".to_owned()),
            track_number: Some(4),
            duration_ms: Some(212_000),
            position_ms: Some(35_000),
            playing: true,
        }
    }

    #[test]
    fn new_track_state_contains_normalized_metadata() {
        let state = MediaState::from_snapshot(snapshot(), Some(25));

        assert!(state.available);
        assert_eq!(state.source.as_deref(), Some("spotify"));
        assert_eq!(state.title.as_deref(), Some("Track one"));
        assert_eq!(state.position_ms, Some(35_000));
        assert_eq!(state.artwork_url.as_deref(), Some("/artwork?v=25"));
    }

    #[test]
    fn serialized_state_uses_the_overlay_contract() {
        let value = serde_json::to_value(MediaState::from_snapshot(snapshot(), Some(25)))
            .expect("state serializes");

        assert_eq!(value["type"], "state");
        assert_eq!(value["duration_ms"], 212_000);
        assert_eq!(value["position_ms"], 35_000);
        assert_eq!(value["artwork_url"], "/artwork?v=25");
        assert!(value.get("durationMs").is_none());
    }

    #[test]
    fn pause_and_resume_preserve_timeline_position() {
        let mut paused = snapshot();
        paused.playing = false;
        let paused_state = MediaState::from_snapshot(paused, None);
        assert!(!paused_state.playing);
        assert_eq!(paused_state.position_ms, Some(35_000));

        let resumed_state = MediaState::from_snapshot(snapshot(), None);
        assert!(resumed_state.playing);
        assert_eq!(resumed_state.position_ms, Some(35_000));
    }

    #[test]
    fn seek_is_clamped_to_duration() {
        let mut seeked = snapshot();
        seeked.position_ms = Some(999_999);
        let state = MediaState::from_snapshot(seeked, None);
        assert_eq!(state.position_ms, Some(212_000));
    }

    #[test]
    fn missing_metadata_is_not_exposed_as_placeholder_text() {
        let state = MediaState::from_snapshot(
            MediaSnapshot {
                source_app_user_model_id: "Spotify.exe".to_owned(),
                title: Some("  ".to_owned()),
                artist: None,
                ..MediaSnapshot::default()
            },
            None,
        );

        assert!(!state.available);
        assert!(state.title.is_none());
        assert!(state.artist.is_none());
    }

    #[test]
    fn missing_duration_is_supported() {
        let mut without_duration = snapshot();
        without_duration.duration_ms = None;
        without_duration.position_ms = Some(35_000);
        let state = MediaState::from_snapshot(without_duration, None);

        assert!(state.available);
        assert!(state.duration_ms.is_none());
        assert_eq!(state.position_ms, Some(35_000));
    }

    #[test]
    fn session_removal_produces_unavailable_state() {
        let state = MediaState::unavailable(6);
        assert!(!state.available);
        assert!(!state.playing);
        assert!(state.title.is_none());
        assert_eq!(state.artwork_version, 6);
    }
}
