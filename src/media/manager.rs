use crate::config::SourceMode;

use super::{ArtworkStore, MediaState};

pub fn spawn(
    state_tx: tokio::sync::watch::Sender<MediaState>,
    artwork: ArtworkStore,
    source_mode: SourceMode,
    specific_app_user_model_id: Option<String>,
) -> Option<std::thread::JoinHandle<()>> {
    #[cfg(windows)]
    {
        let builder = std::thread::Builder::new().name("spotify-media-monitor".to_owned());
        return match builder.spawn(move || {
            windows_impl::run_on_media_thread(
                state_tx,
                artwork,
                source_mode,
                specific_app_user_model_id,
            );
        }) {
            Ok(handle) => Some(handle),
            Err(error) => {
                tracing::error!(?error, "could not start Windows media monitor thread");
                None
            }
        };
    }

    #[cfg(not(windows))]
    {
        let _ = (state_tx, artwork, source_mode, specific_app_user_model_id);
        tracing::warn!("Windows media integration is unavailable on this platform");
        None
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::{
        collections::{HashMap, HashSet},
        time::Duration,
    };

    use tokio::sync::{mpsc, watch};
    use windows::{
        Foundation::TypedEventHandler,
        Media::Control::{
            CurrentSessionChangedEventArgs, GlobalSystemMediaTransportControlsSession,
            GlobalSystemMediaTransportControlsSessionManager,
            GlobalSystemMediaTransportControlsSessionMediaProperties,
            GlobalSystemMediaTransportControlsSessionPlaybackStatus,
            MediaPropertiesChangedEventArgs, PlaybackInfoChangedEventArgs,
            SessionsChangedEventArgs, TimelinePropertiesChangedEventArgs,
        },
        Storage::Streams::DataReader,
        Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
    };

    use crate::{
        config::SourceMode,
        media::{session::MediaSnapshot, spotify::source_matches, ArtworkStore, MediaState},
    };

    #[derive(Clone, Copy, Debug)]
    enum MediaSignal {
        SessionsChanged,
        CurrentSessionChanged,
        SessionPropertiesChanged,
    }

    pub fn run_on_media_thread(
        state_tx: watch::Sender<MediaState>,
        artwork: ArtworkStore,
        source_mode: SourceMode,
        specific_app_user_model_id: Option<String>,
    ) {
        let com_status = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if com_status.is_err() {
            tracing::error!(error = ?com_status, "could not initialize the Windows runtime");
            return;
        }

        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                tracing::error!(?error, "could not create media monitor runtime");
                unsafe { CoUninitialize() };
                return;
            }
        };

        runtime.block_on(async move {
            monitor_until_stopped(state_tx, artwork, source_mode, specific_app_user_model_id).await;
        });

        unsafe { CoUninitialize() };
    }

    async fn monitor_until_stopped(
        state_tx: watch::Sender<MediaState>,
        artwork: ArtworkStore,
        source_mode: SourceMode,
        specific_app_user_model_id: Option<String>,
    ) {
        loop {
            let manager = match GlobalSystemMediaTransportControlsSessionManager::RequestAsync() {
                Ok(operation) => match operation.await {
                    Ok(manager) => manager,
                    Err(error) => {
                        tracing::warn!(?error, "Windows media session manager is unavailable");
                        tokio::time::sleep(Duration::from_secs(20)).await;
                        continue;
                    }
                },
                Err(error) => {
                    tracing::warn!(?error, "could not request Windows media session manager");
                    tokio::time::sleep(Duration::from_secs(20)).await;
                    continue;
                }
            };

            tracing::info!("Windows media session manager connected");
            if let Err(error) = run_manager_loop(
                manager,
                state_tx.clone(),
                artwork.clone(),
                source_mode.clone(),
                specific_app_user_model_id.clone(),
            )
            .await
            {
                tracing::warn!(?error, "Windows media manager loop ended; retrying");
            }

            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    async fn run_manager_loop(
        manager: GlobalSystemMediaTransportControlsSessionManager,
        state_tx: watch::Sender<MediaState>,
        artwork: ArtworkStore,
        source_mode: SourceMode,
        specific_app_user_model_id: Option<String>,
    ) -> windows::core::Result<()> {
        let (signal_tx, mut signal_rx) = mpsc::unbounded_channel::<MediaSignal>();

        let sessions_handler = {
            let signal_tx = signal_tx.clone();
            TypedEventHandler::<
                GlobalSystemMediaTransportControlsSessionManager,
                SessionsChangedEventArgs,
            >::new(move |_, _| {
                let _ = signal_tx.send(MediaSignal::SessionsChanged);
                Ok(())
            })
        };
        let current_handler = {
            let signal_tx = signal_tx.clone();
            TypedEventHandler::<
                GlobalSystemMediaTransportControlsSessionManager,
                CurrentSessionChangedEventArgs,
            >::new(move |_, _| {
                let _ = signal_tx.send(MediaSignal::CurrentSessionChanged);
                Ok(())
            })
        };

        let sessions_token = manager.SessionsChanged(&sessions_handler)?;
        let current_token = match manager.CurrentSessionChanged(&current_handler) {
            Ok(token) => token,
            Err(error) => {
                let _ = manager.RemoveSessionsChanged(sessions_token);
                return Err(error);
            }
        };

        let mut subscriptions = HashMap::<String, SessionSubscription>::new();
        reconcile_sessions(
            &manager,
            &mut subscriptions,
            &signal_tx,
            &state_tx,
            &artwork,
            &source_mode,
            specific_app_user_model_id.as_deref(),
        )
        .await;

        let mut fallback = tokio::time::interval(Duration::from_secs(20));
        fallback.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        fallback.tick().await;

        loop {
            tokio::select! {
                signal = signal_rx.recv() => {
                    match signal {
                        Some(MediaSignal::SessionsChanged | MediaSignal::CurrentSessionChanged) => {
                            reconcile_sessions(
                                &manager,
                                &mut subscriptions,
                                &signal_tx,
                                &state_tx,
                                &artwork,
                                &source_mode,
                                specific_app_user_model_id.as_deref(),
                            ).await;
                        }
                        Some(MediaSignal::SessionPropertiesChanged) => {
                            refresh_selected_session(&subscriptions, &state_tx, &artwork).await;
                        }
                        None => break,
                    }
                }
                _ = fallback.tick() => {
                    reconcile_sessions(
                        &manager,
                        &mut subscriptions,
                        &signal_tx,
                        &state_tx,
                        &artwork,
                        &source_mode,
                        specific_app_user_model_id.as_deref(),
                    ).await;
                }
            }
        }

        subscriptions.clear();
        let _ = manager.RemoveSessionsChanged(sessions_token);
        let _ = manager.RemoveCurrentSessionChanged(current_token);
        Ok(())
    }

    async fn reconcile_sessions(
        manager: &GlobalSystemMediaTransportControlsSessionManager,
        subscriptions: &mut HashMap<String, SessionSubscription>,
        signal_tx: &mpsc::UnboundedSender<MediaSignal>,
        state_tx: &watch::Sender<MediaState>,
        artwork: &ArtworkStore,
        source_mode: &SourceMode,
        specific_app_user_model_id: Option<&str>,
    ) {
        let desired = match discover_sessions(manager, source_mode, specific_app_user_model_id) {
            Ok(desired) => desired,
            Err(error) => {
                tracing::warn!(?error, "could not enumerate Windows media sessions");
                return;
            }
        };

        let desired_keys = desired
            .iter()
            .map(|(source, _)| source.clone())
            .collect::<HashSet<_>>();
        subscriptions.retain(|source, _| desired_keys.contains(source));

        for (source, session) in desired {
            let needs_subscription = subscriptions
                .get(&source)
                .map(|subscription| subscription.session != session)
                .unwrap_or(true);

            if !needs_subscription {
                continue;
            }

            subscriptions.remove(&source);
            match SessionSubscription::new(session, signal_tx.clone()) {
                Ok(subscription) => {
                    tracing::info!(source = %source, "media session discovered");
                    subscriptions.insert(source, subscription);
                }
                Err(error) => {
                    tracing::warn!(source = %source, ?error, "could not subscribe to media session");
                }
            }
        }

        if subscriptions.is_empty() {
            let artwork_version = artwork.clear().await;
            publish_state(state_tx, MediaState::unavailable(artwork_version));
            return;
        }

        refresh_selected_session(subscriptions, state_tx, artwork).await;
    }

    fn discover_sessions(
        manager: &GlobalSystemMediaTransportControlsSessionManager,
        source_mode: &SourceMode,
        specific_app_user_model_id: Option<&str>,
    ) -> windows::core::Result<Vec<(String, GlobalSystemMediaTransportControlsSession)>> {
        if matches!(source_mode, SourceMode::CurrentMediaSession) {
            let session = match manager.GetCurrentSession() {
                Ok(session) => session,
                Err(_) => return Ok(Vec::new()),
            };
            let source = session.SourceAppUserModelId()?.to_string();
            return Ok(vec![(source, session)]);
        }

        let sessions = manager.GetSessions()?;
        let mut desired = Vec::new();
        for index in 0..sessions.Size()? {
            let session = sessions.GetAt(index)?;
            let source = session.SourceAppUserModelId()?.to_string();
            if source_matches(source_mode, &source, specific_app_user_model_id) {
                desired.push((source, session));
            }
        }
        Ok(desired)
    }

    async fn refresh_selected_session(
        subscriptions: &HashMap<String, SessionSubscription>,
        state_tx: &watch::Sender<MediaState>,
        artwork: &ArtworkStore,
    ) {
        let Some((source, subscription)) = subscriptions.iter().next() else {
            return;
        };

        refresh_session(source, &subscription.session, state_tx, artwork).await;
    }

    async fn refresh_session(
        source: &str,
        session: &GlobalSystemMediaTransportControlsSession,
        state_tx: &watch::Sender<MediaState>,
        artwork: &ArtworkStore,
    ) {
        let properties = match session.TryGetMediaPropertiesAsync() {
            Ok(operation) => match operation.await {
                Ok(properties) => properties,
                Err(error) => {
                    tracing::debug!(source = %source, ?error, "media properties are temporarily unavailable");
                    return;
                }
            },
            Err(error) => {
                tracing::debug!(source = %source, ?error, "could not request media properties");
                return;
            }
        };

        let previous = state_tx.borrow().clone();
        let snapshot = build_snapshot(source, session, &properties);
        let track_changed = previous.title != snapshot.title;

        match read_thumbnail(&properties).await {
            Ok(Some((bytes, content_type))) => {
                artwork.replace(bytes, content_type).await;
            }
            Ok(None) if track_changed => {
                artwork.clear().await;
            }
            Err(error) if track_changed => {
                tracing::debug!(source = %source, ?error, "could not read new artwork");
                artwork.clear().await;
            }
            Err(error) => {
                tracing::debug!(source = %source, ?error, "could not refresh artwork; keeping cached image");
            }
            Ok(None) => {}
        }

        let artwork_snapshot = artwork.snapshot().await;
        let state =
            MediaState::from_snapshot(snapshot, artwork_snapshot.map(|image| image.version));
        if state.title != previous.title {
            tracing::info!(
                source = %source,
                title = ?state.title,
                artist = ?state.artist,
                "track changed"
            );
        }
        publish_state(state_tx, state);
    }

    fn build_snapshot(
        source: &str,
        session: &GlobalSystemMediaTransportControlsSession,
        properties: &GlobalSystemMediaTransportControlsSessionMediaProperties,
    ) -> MediaSnapshot {
        let (duration_ms, position_ms) = session
            .GetTimelineProperties()
            .ok()
            .and_then(|timeline| {
                let start = timeline.StartTime().ok()?.Duration;
                let end = timeline.EndTime().ok()?.Duration;
                let position = timeline.Position().ok()?.Duration;
                let duration = (end > start).then(|| ((end - start) / 10_000) as u64);
                let position = (position >= start).then(|| ((position - start) / 10_000) as u64);
                Some((duration, position))
            })
            .unwrap_or((None, None));

        let playing = session
            .GetPlaybackInfo()
            .ok()
            .and_then(|info| info.PlaybackStatus().ok())
            .map(|status| {
                status == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing
            })
            .unwrap_or(false);

        MediaSnapshot {
            source_app_user_model_id: source.to_owned(),
            title: properties.Title().ok().map(|value| value.to_string()),
            artist: properties.Artist().ok().map(|value| value.to_string()),
            album: properties.AlbumTitle().ok().map(|value| value.to_string()),
            album_artist: properties.AlbumArtist().ok().map(|value| value.to_string()),
            subtitle: properties.Subtitle().ok().map(|value| value.to_string()),
            track_number: properties
                .TrackNumber()
                .ok()
                .and_then(|value| u32::try_from(value).ok()),
            duration_ms,
            position_ms,
            playing,
        }
    }

    async fn read_thumbnail(
        properties: &GlobalSystemMediaTransportControlsSessionMediaProperties,
    ) -> windows::core::Result<Option<(Vec<u8>, String)>> {
        let reference = match properties.Thumbnail() {
            Ok(reference) => reference,
            Err(_) => return Ok(None),
        };
        let stream = reference.OpenReadAsync()?.await?;
        let size = stream.Size()?;
        const MAX_ARTWORK_BYTES: u64 = 8 * 1024 * 1024;
        if size == 0 || size > MAX_ARTWORK_BYTES {
            return Ok(None);
        }

        let reader = DataReader::CreateDataReader(&stream)?;
        let loaded = reader.LoadAsync(size as u32)?.await?;
        let mut bytes = vec![0_u8; loaded as usize];
        reader.ReadBytes(&mut bytes)?;
        let content_type = stream
            .ContentType()
            .ok()
            .map(|value| value.to_string())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| sniff_content_type(&bytes).to_owned());

        Ok(Some((bytes, content_type)))
    }

    fn sniff_content_type(bytes: &[u8]) -> &'static str {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            "image/jpeg"
        } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            "image/webp"
        } else {
            "application/octet-stream"
        }
    }

    fn publish_state(state_tx: &watch::Sender<MediaState>, next: MediaState) {
        let current = state_tx.borrow().clone();
        if current.content_eq(&next) {
            return;
        }
        let _ = state_tx.send(next);
    }

    struct SessionSubscription {
        session: GlobalSystemMediaTransportControlsSession,
        media_properties_token: i64,
        playback_info_token: i64,
        timeline_properties_token: i64,
    }

    impl SessionSubscription {
        fn new(
            session: GlobalSystemMediaTransportControlsSession,
            signal_tx: mpsc::UnboundedSender<MediaSignal>,
        ) -> windows::core::Result<Self> {
            let media_handler = {
                let signal_tx = signal_tx.clone();
                TypedEventHandler::<
                    GlobalSystemMediaTransportControlsSession,
                    MediaPropertiesChangedEventArgs,
                >::new(move |_, _| {
                    let _ = signal_tx.send(MediaSignal::SessionPropertiesChanged);
                    Ok(())
                })
            };
            let playback_handler = {
                let signal_tx = signal_tx.clone();
                TypedEventHandler::<
                    GlobalSystemMediaTransportControlsSession,
                    PlaybackInfoChangedEventArgs,
                >::new(move |_, _| {
                    let _ = signal_tx.send(MediaSignal::SessionPropertiesChanged);
                    Ok(())
                })
            };
            let timeline_handler = TypedEventHandler::<
                GlobalSystemMediaTransportControlsSession,
                TimelinePropertiesChangedEventArgs,
            >::new(move |_, _| {
                let _ = signal_tx.send(MediaSignal::SessionPropertiesChanged);
                Ok(())
            });

            let media_properties_token = session.MediaPropertiesChanged(&media_handler)?;
            let playback_info_token = match session.PlaybackInfoChanged(&playback_handler) {
                Ok(token) => token,
                Err(error) => {
                    let _ = session.RemoveMediaPropertiesChanged(media_properties_token);
                    return Err(error);
                }
            };
            let timeline_properties_token =
                match session.TimelinePropertiesChanged(&timeline_handler) {
                    Ok(token) => token,
                    Err(error) => {
                        let _ = session.RemoveMediaPropertiesChanged(media_properties_token);
                        let _ = session.RemovePlaybackInfoChanged(playback_info_token);
                        return Err(error);
                    }
                };

            Ok(Self {
                session,
                media_properties_token,
                playback_info_token,
                timeline_properties_token,
            })
        }
    }

    impl Drop for SessionSubscription {
        fn drop(&mut self) {
            let _ = self
                .session
                .RemoveMediaPropertiesChanged(self.media_properties_token);
            let _ = self
                .session
                .RemovePlaybackInfoChanged(self.playback_info_token);
            let _ = self
                .session
                .RemoveTimelinePropertiesChanged(self.timeline_properties_token);
        }
    }
}
