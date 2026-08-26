use std::time::{Duration, Instant};

use serde::Serialize;

use super::LeagueObservation;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LeagueState {
    #[default]
    Unknown,
    Idle,
    Client,
    Game,
    GameTransition,
    ClientTransition,
}

impl LeagueState {
    pub fn scene_state(self) -> Option<Self> {
        match self {
            Self::Unknown => None,
            Self::Game | Self::GameTransition => Some(Self::Game),
            Self::Client | Self::ClientTransition => Some(Self::Client),
            Self::Idle => Some(Self::Idle),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MachineUpdate {
    pub before: LeagueState,
    pub after: LeagueState,
    pub changed: bool,
    pub pending_deadline: Option<Instant>,
    pub pending_generation: Option<u64>,
}

impl MachineUpdate {
    fn unchanged(state: LeagueState, deadline: Option<Instant>, generation: Option<u64>) -> Self {
        Self {
            before: state,
            after: state,
            changed: false,
            pending_deadline: deadline,
            pending_generation: generation,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LeagueStateMachine {
    state: LeagueState,
    pending_deadline: Option<Instant>,
    pending_generation: u64,
}

impl Default for LeagueStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl LeagueStateMachine {
    pub fn new() -> Self {
        Self {
            state: LeagueState::Unknown,
            pending_deadline: None,
            pending_generation: 0,
        }
    }

    #[allow(dead_code)]
    pub fn state(&self) -> LeagueState {
        self.state
    }

    #[allow(dead_code)]
    pub fn pending(&self) -> Option<(Instant, u64)> {
        self.pending_deadline
            .map(|deadline| (deadline, self.pending_generation))
    }

    pub fn reset(&mut self) {
        self.state = LeagueState::Unknown;
        self.pending_deadline = None;
        self.pending_generation = self.pending_generation.wrapping_add(1);
    }

    pub fn observe(
        &mut self,
        observation: LeagueObservation,
        now: Instant,
        grace: Duration,
    ) -> MachineUpdate {
        if self
            .pending_deadline
            .is_some_and(|deadline| deadline <= now)
        {
            return self.expire(self.pending_generation, now, observation);
        }

        match self.state {
            LeagueState::Unknown => {
                self.set_stable(target_state(observation).unwrap_or(LeagueState::Idle))
            }
            LeagueState::Idle => match target_state(observation) {
                Some(target) => self.set_stable(target),
                None => MachineUpdate::unchanged(self.state, None, None),
            },
            LeagueState::Client => {
                if observation.game {
                    self.set_stable(LeagueState::Game)
                } else if observation.client {
                    MachineUpdate::unchanged(self.state, None, None)
                } else {
                    self.begin_transition(LeagueState::ClientTransition, now, grace, observation)
                }
            }
            LeagueState::Game => {
                if observation.game {
                    MachineUpdate::unchanged(self.state, None, None)
                } else {
                    self.begin_transition(LeagueState::GameTransition, now, grace, observation)
                }
            }
            LeagueState::GameTransition => {
                if observation.game {
                    self.set_stable(LeagueState::Game)
                } else if observation.client {
                    self.set_stable(LeagueState::Client)
                } else {
                    self.waiting_update()
                }
            }
            LeagueState::ClientTransition => {
                if observation.game {
                    self.set_stable(LeagueState::Game)
                } else if observation.client {
                    self.set_stable(LeagueState::Client)
                } else {
                    self.waiting_update()
                }
            }
        }
    }

    pub fn expire(
        &mut self,
        generation: u64,
        now: Instant,
        observation: LeagueObservation,
    ) -> MachineUpdate {
        if self.pending_generation != generation
            || self.pending_deadline.is_none_or(|deadline| deadline > now)
        {
            return self.waiting_update();
        }

        let target = target_state(observation).unwrap_or(LeagueState::Idle);
        self.set_stable(target)
    }

    fn begin_transition(
        &mut self,
        transition: LeagueState,
        now: Instant,
        grace: Duration,
        observation: LeagueObservation,
    ) -> MachineUpdate {
        if grace.is_zero() {
            return self.set_stable(target_state(observation).unwrap_or(LeagueState::Idle));
        }

        let before = self.state;
        self.pending_generation = self.pending_generation.wrapping_add(1);
        self.pending_deadline = Some(now + grace);
        self.state = transition;
        MachineUpdate {
            before,
            after: self.state,
            changed: before != self.state,
            pending_deadline: self.pending_deadline,
            pending_generation: Some(self.pending_generation),
        }
    }

    fn set_stable(&mut self, state: LeagueState) -> MachineUpdate {
        let before = self.state;
        self.state = state;
        self.pending_deadline = None;
        self.pending_generation = self.pending_generation.wrapping_add(1);
        MachineUpdate {
            before,
            after: state,
            changed: before != state,
            pending_deadline: None,
            pending_generation: None,
        }
    }

    fn waiting_update(&self) -> MachineUpdate {
        MachineUpdate {
            before: self.state,
            after: self.state,
            changed: false,
            pending_deadline: self.pending_deadline,
            pending_generation: Some(self.pending_generation),
        }
    }
}

fn target_state(observation: LeagueObservation) -> Option<LeagueState> {
    if observation.game {
        Some(LeagueState::Game)
    } else if observation.client {
        Some(LeagueState::Client)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{LeagueState, LeagueStateMachine};
    use crate::automation::LeagueObservation;
    use std::time::{Duration, Instant};

    fn game() -> LeagueObservation {
        LeagueObservation {
            game: true,
            ..Default::default()
        }
    }

    fn client() -> LeagueObservation {
        LeagueObservation {
            client: true,
            ..Default::default()
        }
    }

    #[test]
    fn game_has_priority_over_client() {
        let mut machine = LeagueStateMachine::new();
        let update = machine.observe(
            LeagueObservation {
                game: true,
                client: true,
                ..Default::default()
            },
            Instant::now(),
            Duration::from_secs(2),
        );
        assert_eq!(update.after, LeagueState::Game);
    }

    #[test]
    fn game_loss_enters_transition_and_expires_to_idle() {
        let mut machine = LeagueStateMachine::new();
        let start = Instant::now();
        machine.observe(game(), start, Duration::from_secs(2));
        let transition = machine.observe(
            LeagueObservation::default(),
            start + Duration::from_millis(10),
            Duration::from_secs(2),
        );
        assert_eq!(transition.after, LeagueState::GameTransition);
        let (deadline, generation) = machine.pending().expect("pending transition");
        let expired = machine.expire(
            generation,
            deadline + Duration::from_millis(1),
            LeagueObservation::default(),
        );
        assert_eq!(expired.after, LeagueState::Idle);
    }

    #[test]
    fn client_return_cancels_game_transition() {
        let mut machine = LeagueStateMachine::new();
        let start = Instant::now();
        machine.observe(game(), start, Duration::from_secs(2));
        machine.observe(
            LeagueObservation::default(),
            start + Duration::from_millis(10),
            Duration::from_secs(2),
        );
        let update = machine.observe(
            client(),
            start + Duration::from_millis(50),
            Duration::from_secs(2),
        );
        assert_eq!(update.after, LeagueState::Client);
        assert!(machine.pending().is_none());
    }

    #[test]
    fn client_loss_enters_grace_then_becomes_idle() {
        let mut machine = LeagueStateMachine::new();
        let start = Instant::now();
        machine.observe(client(), start, Duration::from_secs(2));
        let transition = machine.observe(
            LeagueObservation::default(),
            start + Duration::from_millis(10),
            Duration::from_secs(2),
        );
        assert_eq!(transition.after, LeagueState::ClientTransition);
        let (deadline, generation) = machine.pending().expect("pending transition");
        let expired = machine.expire(
            generation,
            deadline + Duration::from_millis(1),
            LeagueObservation::default(),
        );
        assert_eq!(expired.after, LeagueState::Idle);
    }

    #[test]
    fn stale_transition_generation_is_ignored() {
        let mut machine = LeagueStateMachine::new();
        let start = Instant::now();
        machine.observe(game(), start, Duration::from_secs(2));
        machine.observe(
            LeagueObservation::default(),
            start + Duration::from_millis(10),
            Duration::from_secs(2),
        );
        let (_, stale_generation) = machine.pending().expect("pending transition");
        machine.observe(
            game(),
            start + Duration::from_millis(20),
            Duration::from_secs(2),
        );
        let update = machine.expire(stale_generation, start + Duration::from_secs(3), game());
        assert_eq!(update.after, LeagueState::Game);
    }

    #[test]
    fn zero_grace_switches_without_pending_timer() {
        let mut machine = LeagueStateMachine::new();
        let start = Instant::now();
        machine.observe(game(), start, Duration::ZERO);
        let update = machine.observe(
            LeagueObservation::default(),
            start + Duration::from_millis(1),
            Duration::ZERO,
        );
        assert_eq!(update.after, LeagueState::Idle);
        assert!(machine.pending().is_none());
    }
}
