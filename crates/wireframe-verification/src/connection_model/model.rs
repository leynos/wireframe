//! Stateright model implementation for the placeholder connection actor.

use stateright::{Model, Property};

use super::{ConnectionAction, ConnectionState, properties::properties, state::ActiveOutput};

/// Small semantic model used to land the verification crate and shared harness.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaceholderConnectionModel {
    max_steps: u8,
}

impl Default for PlaceholderConnectionModel {
    /// Returns a model bounded to six steps, suitable for CI-time verification.
    fn default() -> Self { Self { max_steps: 6 } }
}

impl PlaceholderConnectionModel {
    /// Create a model with an explicit state-space depth bound.
    pub fn new(max_steps: u8) -> Self { Self { max_steps } }

    /// Clone `state` and advance the step counter by one.
    fn stepped(state: &ConnectionState) -> ConnectionState {
        let mut next = state.clone();
        next.steps = next.steps.saturating_add(1);
        next
    }

    /// Apply `mutate` to a stepped clone of `state` if `enabled`; return `None` otherwise.
    fn apply_if<F>(state: &ConnectionState, enabled: bool, mutate: F) -> Option<ConnectionState>
    where
        F: FnOnce(&mut ConnectionState),
    {
        if !enabled {
            return None;
        }
        let mut next = Self::stepped(state);
        mutate(&mut next);
        Some(next)
    }

    /// Transition: enqueue a high-priority output request.
    fn apply_enqueue_high(state: &ConnectionState) -> Option<ConnectionState> {
        Self::apply_if(state, !state.high_priority_queued, |next| {
            next.high_priority_queued = true;
        })
    }

    /// Transition: enqueue a low-priority output request.
    fn apply_enqueue_low(state: &ConnectionState) -> Option<ConnectionState> {
        Self::apply_if(state, !state.low_priority_queued, |next| {
            next.low_priority_queued = true;
        })
    }

    /// Transition: install a response stream as the active output.
    fn apply_install_response(state: &ConnectionState) -> Option<ConnectionState> {
        Self::apply_if(state, state.can_install_output(), |next| {
            next.active_output = ActiveOutput::Response;
        })
    }

    /// Transition: install a multi-packet stream as the active output.
    fn apply_install_multi_packet(state: &ConnectionState) -> Option<ConnectionState> {
        Self::apply_if(state, state.can_install_output(), |next| {
            next.active_output = ActiveOutput::MultiPacket;
            next.multi_packet_terminal_count = 0;
        })
    }

    /// Transition: emit the queued high-priority output.
    fn apply_emit_high(state: &ConnectionState) -> Option<ConnectionState> {
        Self::apply_if(state, state.high_priority_queued, |next| {
            next.high_priority_queued = false;
            next.emitted_high_priority = true;
            next.fairness_allows_low = true;
        })
    }

    /// Transition: emit the queued low-priority output under the fairness policy.
    fn apply_emit_low(state: &ConnectionState) -> Option<ConnectionState> {
        Self::apply_if(state, state.can_emit_low_priority(), |next| {
            next.low_priority_queued = false;
            next.emitted_low_priority = true;
            next.fairness_allows_low = false;
        })
    }

    /// Transition: emit one frame from the active output stream.
    fn apply_emit_active_frame(state: &ConnectionState) -> Option<ConnectionState> {
        Self::apply_if(state, !state.is_output_idle(), |next| {
            next.shutdown_during_output = state.shutdown_requested;
        })
    }

    /// Transition: complete a response output stream.
    fn apply_complete_response(state: &ConnectionState) -> Option<ConnectionState> {
        Self::apply_if(
            state,
            matches!(state.active_output, ActiveOutput::Response),
            |next| {
                next.active_output = ActiveOutput::Idle;
                next.response_completed = true;
            },
        )
    }

    /// Transition: complete a multi-packet output stream.
    fn apply_complete_multi_packet(state: &ConnectionState) -> Option<ConnectionState> {
        Self::apply_if(
            state,
            matches!(state.active_output, ActiveOutput::MultiPacket),
            |next| {
                next.active_output = ActiveOutput::Idle;
                next.multi_packet_completed = true;
                next.multi_packet_terminal_count =
                    next.multi_packet_terminal_count.saturating_add(1);
            },
        )
    }

    /// Transition: request connection shutdown.
    fn apply_shutdown(state: &ConnectionState) -> Option<ConnectionState> {
        Self::apply_if(state, !state.shutdown_requested, |next| {
            next.shutdown_requested = true;
            next.shutdown_during_output = !state.is_output_idle();
        })
    }

    /// Transition: advance the fairness timer to allow low-priority emission.
    fn apply_tick_fairness(state: &ConnectionState) -> Option<ConnectionState> {
        Self::apply_if(state, state.low_priority_queued, |next| {
            next.fairness_allows_low = true;
        })
    }
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
        match action {
            ConnectionAction::EnqueueHigh => Self::apply_enqueue_high(state),
            ConnectionAction::EnqueueLow => Self::apply_enqueue_low(state),
            ConnectionAction::InstallResponse => Self::apply_install_response(state),
            ConnectionAction::InstallMultiPacket => Self::apply_install_multi_packet(state),
            ConnectionAction::EmitQueued => {
                Self::apply_emit_high(state).or_else(|| Self::apply_emit_low(state))
            }
            ConnectionAction::EmitActiveFrame => Self::apply_emit_active_frame(state),
            ConnectionAction::CompleteActiveOutput => Self::apply_complete_response(state)
                .or_else(|| Self::apply_complete_multi_packet(state)),
            ConnectionAction::Shutdown => Self::apply_shutdown(state),
            ConnectionAction::TickFairness => Self::apply_tick_fairness(state),
        }
    }

    fn properties(&self) -> Vec<Property<Self>> { properties() }

    fn within_boundary(&self, state: &Self::State) -> bool { state.steps <= self.max_steps }
}
