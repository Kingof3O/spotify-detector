#![cfg_attr(not(windows), allow(dead_code))]

use crate::config::SourceMode;

pub fn is_spotify_source(source_app_user_model_id: &str) -> bool {
    source_app_user_model_id
        .to_ascii_lowercase()
        .contains("spotify")
}

pub fn source_label(source_app_user_model_id: &str) -> String {
    if is_spotify_source(source_app_user_model_id) {
        "spotify".to_owned()
    } else {
        source_app_user_model_id.to_owned()
    }
}

pub(crate) fn source_matches(
    source_mode: &SourceMode,
    source_app_user_model_id: &str,
    specific_app_user_model_id: Option<&str>,
) -> bool {
    match source_mode {
        SourceMode::SpotifyOnly => is_spotify_source(source_app_user_model_id),
        SourceMode::CurrentMediaSession => true,
        SourceMode::SpecificApplication => specific_app_user_model_id
            .is_some_and(|expected| expected.eq_ignore_ascii_case(source_app_user_model_id)),
    }
}

#[cfg(test)]
mod tests {
    use super::{is_spotify_source, source_label, source_matches};
    use crate::config::SourceMode;

    #[test]
    fn spotify_source_detection_is_case_insensitive() {
        assert!(is_spotify_source(
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"
        ));
        assert!(is_spotify_source("SPOTIFY.EXE"));
        assert!(!is_spotify_source("Chrome.exe"));
    }

    #[test]
    fn source_label_normalizes_spotify() {
        assert_eq!(source_label("Spotify.exe"), "spotify");
        assert_eq!(source_label("VLC.exe"), "VLC.exe");
    }

    #[test]
    fn source_modes_are_explicit() {
        assert!(source_matches(
            &SourceMode::SpotifyOnly,
            "Spotify.exe",
            None
        ));
        assert!(!source_matches(
            &SourceMode::SpotifyOnly,
            "Chrome.exe",
            None
        ));
        assert!(source_matches(
            &SourceMode::SpecificApplication,
            "VLC.exe",
            Some("vlc.exe")
        ));
    }
}
