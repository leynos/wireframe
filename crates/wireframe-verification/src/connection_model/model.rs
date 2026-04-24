//! Stateright model implementation for the placeholder connection actor.

use stateright::{Model, Property};

use super::{ConnectionAction, ConnectionState, properties::properties, state::ActiveOutput};

/// Small semantic model used to land the verification crate and shared harness.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaceholderConnectionModel {
    max_steps: u8,
}

impl Default for PlaceholderConnectionModel {
    fn default() -> Self { Self { max_steps: 6 } }
}

impl PlaceholderConnectionModel {
    /// Create a model with an explicit state-space depth bound.
    pub fn new(max_steps: u8) -> Self { Self { max_steps } }
}

impl Model for PlaceholderConnectionModel {
    type State = ConnectionState;
    type Action = ConnectionAction;

    fn init_states(&self) -> Vec<Self::State> { vec![ConnectionState::default()] }

    fn actions(&self, _state: &Self::State, actions: &mut Vec<Self::Action>) {
        actions.extend([
            ConnectionAction::EnqueueHigh,
            ConnectionAction::EnqueueLow,
            ConnectionAction::InstallResponse,
            ConnectionAction::InstallMultiPacket,
            ConnectionAction::EmitQueued,
            ConnectionAction::EmitActiveFrame,
            ConnectionAction::CompleteActiveOutput,
            ConnectionAction::Shutdown,
            ConnectionAction::TickFairness,
        ]);
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        next.steps = next.steps.saturating_add(1);

        match action {
            ConnectionAction::EnqueueHigh if !state.high_priority_queued => {
                next.high_priority_queued = true;
                Some(next)
            }
            ConnectionAction::EnqueueLow if !state.low_priority_queued => {
                next.low_priority_queued = true;
                Some(next)
            }
            ConnectionAction::InstallResponse
                if !state.shutdown_requested
                    && matches!(state.active_output, ActiveOutput::Idle) =>
            {
                next.active_output = ActiveOutput::Response;
                Some(next)
            }
            ConnectionAction::InstallMultiPacket
                if !state.shutdown_requested
                    && matches!(state.active_output, ActiveOutput::Idle) =>
            {
                next.active_output = ActiveOutput::MultiPacket;
                next.multi_packet_terminal_count = 0;
                Some(next)
            }
            ConnectionAction::EmitQueued if state.high_priority_queued => {
                next.high_priority_queued = false;
                next.emitted_high_priority = true;
                next.fairness_allows_low = true;
                Some(next)
            }
            ConnectionAction::EmitQueued
                if state.low_priority_queued
                    && (!state.high_priority_queued || state.fairness_allows_low) =>
            {
                next.low_priority_queued = false;
                next.emitted_low_priority = true;
                next.fairness_allows_low = false;
                Some(next)
            }
            ConnectionAction::EmitActiveFrame
                if !matches!(state.active_output, ActiveOutput::Idle) =>
            {
                if state.shutdown_requested {
                    next.shutdown_during_output = true;
                }
                Some(next)
            }
            ConnectionAction::CompleteActiveOutput
                if matches!(state.active_output, ActiveOutput::Response) =>
            {
                next.active_output = ActiveOutput::Idle;
                next.response_completed = true;
                Some(next)
            }
            ConnectionAction::CompleteActiveOutput
                if matches!(state.active_output, ActiveOutput::MultiPacket) =>
            {
                next.active_output = ActiveOutput::Idle;
                next.multi_packet_completed = true;
                next.multi_packet_terminal_count =
                    next.multi_packet_terminal_count.saturating_add(1);
                Some(next)
            }
            ConnectionAction::Shutdown if !state.shutdown_requested => {
                next.shutdown_requested = true;
                if !matches!(state.active_output, ActiveOutput::Idle) {
                    next.shutdown_during_output = true;
                }
                Some(next)
            }
            ConnectionAction::TickFairness if state.low_priority_queued => {
                next.fairness_allows_low = true;
                Some(next)
            }
            _ => None,
        }
    }

    fn properties(&self) -> Vec<Property<Self>> { properties() }

    fn within_boundary(&self, state: &Self::State) -> bool { state.steps <= self.max_steps }
}
