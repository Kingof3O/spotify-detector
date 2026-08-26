mod league;
mod obs;
mod state;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use serde::Serialize;
use tokio::sync::{mpsc, watch, RwLock};

use crate::{config::Config, integration::CredentialStore};

pub use obs::ObsHandle;
pub use state::{LeagueState, LeagueStateMachine};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LeagueObservation {
    pub game: bool,
    pub client: bool,
    pub game_foreground: bool,
    pub client_foreground: bool,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub enum WindowSignalKind {
    Created,
    Destroyed,
    Show,
    Hide,
    Cloaked,
    Foreground,
}

impl WindowSignalKind {
    #[cfg(windows)]
    fn from_event(event: u32) -> Option<Self> {
        use windows::Win32::UI::WindowsAndMessaging::{
            EVENT_OBJECT_CLOAKED, EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE,
            EVENT_OBJECT_SHOW, EVENT_SYSTEM_FOREGROUND,
        };
        match event {
            EVENT_OBJECT_CREATE => Some(Self::Created),
            EVENT_OBJECT_DESTROY => Some(Self::Destroyed),
            EVENT_OBJECT_SHOW => Some(Self::Show),
            EVENT_OBJECT_HIDE => Some(Self::Hide),
            EVENT_OBJECT_CLOAKED => Some(Self::Cloaked),
            EVENT_SYSTEM_FOREGROUND => Some(Self::Foreground),
            _ => None,
        }
    }
}

impl std::fmt::Display for WindowSignalKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Created => "window_created",
            Self::Destroyed => "window_destroyed",
            Self::Show => "window_shown",
            Self::Hide => "window_hidden",
            Self::Cloaked => "window_cloaked",
            Self::Foreground => "foreground_changed",
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WindowSignal {
    pub kind: WindowSignalKind,
    pub hwnd: usize,
}

#[derive(Clone, Debug)]
pub struct DesiredScene {
    pub scene: String,
    pub force: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct AutomationStatus {
    pub enabled: bool,
    pub obs_enabled: bool,
    pub obs_connected: bool,
    pub obs_status: String,
    pub obs_version: Option<String>,
    pub obs_current_scene: Option<String>,
    pub obs_scenes: Vec<String>,
    pub obs_last_error: Option<String>,
    pub league_state: LeagueState,
    pub league_game_present: bool,
    pub league_client_present: bool,
    pub league_game_foreground: bool,
    pub league_client_foreground: bool,
    pub league_last_signal: Option<String>,
    pub league_pending_transition_ms: Option<u64>,
    pub league_last_transition: Option<String>,
    pub league_last_error: Option<String>,
}

impl Default for AutomationStatus {
    fn default() -> Self {
        Self {
            enabled: false,
            obs_enabled: false,
            obs_connected: false,
            obs_status: "disabled".to_owned(),
            obs_version: None,
            obs_current_scene: None,
            obs_scenes: Vec::new(),
            obs_last_error: None,
            league_state: LeagueState::Unknown,
            league_game_present: false,
            league_client_present: false,
            league_game_foreground: false,
            league_client_foreground: false,
            league_last_signal: None,
            league_pending_transition_ms: None,
            league_last_transition: None,
            league_last_error: None,
        }
    }
}

#[derive(Clone)]
pub struct AutomationState {
    pub(crate) status: Arc<RwLock<AutomationStatus>>,
}

impl AutomationState {
    pub fn new() -> Self {
        Self {
            status: Arc::new(RwLock::new(AutomationStatus::default())),
        }
    }

    pub async fn snapshot(&self) -> AutomationStatus {
        self.status.read().await.clone()
    }
}

impl Default for AutomationState {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
pub struct AutomationHandles {
    pub(crate) controller: tokio::task::JoinHandle<()>,
    pub(crate) obs: tokio::task::JoinHandle<()>,
    pub(crate) detector: Option<std::thread::JoinHandle<()>>,
}

pub fn spawn(
    config_rx: watch::Receiver<Config>,
    shutdown_rx: watch::Receiver<bool>,
    credentials: Arc<CredentialStore>,
    state: AutomationState,
) -> AutomationHandles {
    let (signal_tx, signal_rx) = mpsc::unbounded_channel();
    let signal_keepalive = signal_tx.clone();
    let detector = league::spawn(signal_tx);
    let (obs_handle, obs_task) = obs::spawn(
        config_rx.clone(),
        shutdown_rx.clone(),
        credentials,
        state.clone(),
    );
    let controller = tokio::spawn(run_controller(
        config_rx,
        shutdown_rx,
        signal_rx,
        signal_keepalive,
        state,
        obs_handle,
    ));
    AutomationHandles {
        controller,
        obs: obs_task,
        detector,
    }
}

async fn run_controller(
    mut config_rx: watch::Receiver<Config>,
    mut shutdown_rx: watch::Receiver<bool>,
    mut signal_rx: mpsc::UnboundedReceiver<WindowSignal>,
    _signal_keepalive: mpsc::UnboundedSender<WindowSignal>,
    automation: AutomationState,
    obs: ObsHandle,
) {
    let mut config = config_rx.borrow().clone();
    let mut machine = LeagueStateMachine::new();
    let mut observation = LeagueObservation::default();
    let mut debounce_deadline: Option<tokio::time::Instant> = Some(tokio::time::Instant::now());
    let mut transition_deadline: Option<(tokio::time::Instant, u64)> = None;
    let mut force_scene_on_next_observation = true;

    loop {
        let debounce_wait = async {
            match debounce_deadline {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                None => std::future::pending::<()>().await,
            }
        };
        let transition_wait = async {
            match transition_deadline {
                Some((deadline, _)) => tokio::time::sleep_until(deadline).await,
                None => std::future::pending::<()>().await,
            }
        };

        tokio::select! {
            _ = debounce_wait => {
                debounce_deadline = None;
                if config.league.enabled {
                    observation = league::snapshot(&config.league);
                    let force_scene = force_scene_on_next_observation;
                    force_scene_on_next_observation = false;
                    apply_observation(
                        &mut machine,
                        &mut transition_deadline,
                        observation,
                        &config,
                        &automation,
                        &obs,
                        force_scene,
                    ).await;
                }
            }
            _ = transition_wait => {
                if let Some((_, generation)) = transition_deadline.take() {
                    let update = machine.expire(
                        generation,
                        Instant::now(),
                        observation,
                    );
                    apply_update(
                        update,
                        &mut transition_deadline,
                        observation,
                        &config,
                        &automation,
                        &obs,
                        false,
                    ).await;
                }
            }
            signal = signal_rx.recv() => {
                let Some(signal) = signal else { break; };
                {
                    let mut status = automation.status.write().await;
                    status.league_last_signal = Some(format!(
                        "{} ({:#x})",
                        signal.kind, signal.hwnd
                    ));
                }
                debounce_deadline = Some(tokio::time::Instant::now() + Duration::from_millis(75));
            }
            changed = config_rx.changed() => {
                if changed.is_err() { break; }
                config = config_rx.borrow().clone();
                debounce_deadline = Some(tokio::time::Instant::now());
                force_scene_on_next_observation = true;
                machine.reset();
                transition_deadline = None;
                if !config.league.enabled {
                    let mut status = automation.status.write().await;
                    status.league_state = LeagueState::Unknown;
                    status.league_pending_transition_ms = None;
                    status.enabled = false;
                } else {
                    let mut status = automation.status.write().await;
                    status.enabled = config.obs.enabled;
                }
            }
            changed = shutdown_rx.changed() => {
                if changed.is_ok() && *shutdown_rx.borrow() { break; }
            }
        }
    }

    let mut status = automation.status.write().await;
    status.enabled = false;
    status.league_state = LeagueState::Unknown;
    status.league_pending_transition_ms = None;
}

async fn apply_observation(
    machine: &mut LeagueStateMachine,
    transition_deadline: &mut Option<(tokio::time::Instant, u64)>,
    observation: LeagueObservation,
    config: &Config,
    automation: &AutomationState,
    obs: &ObsHandle,
    force_scene: bool,
) {
    let update = machine.observe(
        observation,
        Instant::now(),
        Duration::from_millis(config.league.transition_grace_ms),
    );
    apply_update(
        update,
        transition_deadline,
        observation,
        config,
        automation,
        obs,
        force_scene,
    )
    .await;
}

async fn apply_update(
    update: state::MachineUpdate,
    transition_deadline: &mut Option<(tokio::time::Instant, u64)>,
    observation: LeagueObservation,
    config: &Config,
    automation: &AutomationState,
    obs: &ObsHandle,
    force_scene: bool,
) {
    *transition_deadline = update
        .pending_deadline
        .zip(update.pending_generation)
        .map(|(deadline, generation)| (tokio::time::Instant::from_std(deadline), generation));

    let pending_ms = transition_deadline.map(|(deadline, _)| {
        deadline
            .saturating_duration_since(tokio::time::Instant::now())
            .as_millis() as u64
    });

    {
        let mut status = automation.status.write().await;
        status.enabled = config.obs.enabled && config.league.enabled;
        status.league_state = update.after;
        status.league_game_present = observation.game;
        status.league_client_present = observation.client;
        status.league_game_foreground = observation.game_foreground;
        status.league_client_foreground = observation.client_foreground;
        status.league_pending_transition_ms = pending_ms;
        if update.changed {
            status.league_last_transition = Some(format!("{} → {}", update.before, update.after));
            tracing::info!(
                from = %update.before,
                to = %update.after,
                game = observation.game,
                client = observation.client,
                "League automation state changed"
            );
        }
    }

    if !config.obs.enabled || !config.league.enabled {
        return;
    }

    let old_scene = update.before.scene_state();
    let new_scene = update.after.scene_state();
    let scene_changed = old_scene != new_scene;
    if (update.changed && scene_changed) || force_scene {
        if let Some(scene_state) = new_scene {
            if let Some(scene) = scene_name(scene_state, config) {
                obs.request_scene(scene.to_owned(), true);
            }
        }
    }
}

fn scene_name<'a>(state: LeagueState, config: &'a Config) -> Option<&'a str> {
    match state {
        LeagueState::Game => Some(config.league.game_scene.as_str()),
        LeagueState::Client => Some(config.league.client_scene.as_str()),
        LeagueState::Idle => Some(config.league.idle_scene.as_str()),
        _ => None,
    }
}

impl std::fmt::Display for LeagueState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Unknown => "unknown",
            Self::Idle => "idle",
            Self::Client => "client",
            Self::Game => "game",
            Self::GameTransition => "game_transition",
            Self::ClientTransition => "client_transition",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{scene_name, LeagueState};
    use crate::config::Config;

    #[test]
    fn scene_mapping_uses_configured_names() {
        let mut config = Config::default();
        config.league.game_scene = "Game".to_owned();
        config.league.client_scene = "Client".to_owned();
        config.league.idle_scene = "Idle".to_owned();
        assert_eq!(scene_name(LeagueState::Game, &config), Some("Game"));
        assert_eq!(scene_name(LeagueState::Client, &config), Some("Client"));
        assert_eq!(scene_name(LeagueState::Idle, &config), Some("Idle"));
        assert_eq!(scene_name(LeagueState::Unknown, &config), None);
    }
}
