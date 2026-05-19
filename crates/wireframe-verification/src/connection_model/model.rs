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

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use stateright::Model;

    use super::*;

    #[rstest]
    #[case(ConnectionAction::EnqueueHigh, true, false)]
    #[case(ConnectionAction::EnqueueLow, false, true)]
    fn next_state_enqueues_requested_priority(
        #[case] action: ConnectionAction,
        #[case] expected_high: bool,
        #[case] expected_low: bool,
    ) {
        let model = PlaceholderConnectionModel::default();
        let next = model
            .next_state(&ConnectionState::default(), action)
            .expect("enqueue transition should be enabled");

        assert_eq!(next.steps, 1);
        assert_eq!(next.high_priority_queued, expected_high);
        assert_eq!(next.low_priority_queued, expected_low);
    }

    #[rstest]
    #[case(ConnectionAction::InstallResponse, ActiveOutput::Response)]
    #[case(ConnectionAction::InstallMultiPacket, ActiveOutput::MultiPacket)]
    fn next_state_installs_requested_output(
        #[case] action: ConnectionAction,
        #[case] expected_output: ActiveOutput,
    ) {
        let model = PlaceholderConnectionModel::default();
        let next = model
            .next_state(&ConnectionState::default(), action)
            .expect("install transition should be enabled");

        assert_eq!(next.steps, 1);
        assert_eq!(next.active_output, expected_output);
    }

    #[rstest]
    #[case(ConnectionAction::EnqueueHigh, ConnectionState { high_priority_queued: true, ..Default::default() })]
    #[case(ConnectionAction::EnqueueLow, ConnectionState { low_priority_queued: true, ..Default::default() })]
    fn next_state_rejects_duplicate_enqueue(
        #[case] action: ConnectionAction,
        #[case] state: ConnectionState,
    ) {
        let model = PlaceholderConnectionModel::default();

        assert_eq!(model.next_state(&state, action), None);
    }

    #[rstest]
    #[case(ConnectionState { active_output: ActiveOutput::Response, ..Default::default() })]
    #[case(ConnectionState { shutdown_requested: true, ..Default::default() })]
    fn install_transitions_require_idle_output_and_no_shutdown(#[case] state: ConnectionState) {
        let model = PlaceholderConnectionModel::default();

        assert_eq!(
            model.next_state(&state, ConnectionAction::InstallResponse),
            None
        );
        assert_eq!(
            model.next_state(&state, ConnectionAction::InstallMultiPacket),
            None
        );
    }

    #[rstest]
    #[case(ConnectionState::default())]
    #[case(ConnectionState {
        low_priority_queued: true,
        high_priority_queued: true,
        fairness_allows_low: false,
        ..Default::default()
    })]
    fn emit_low_priority_requires_queue_and_fairness(#[case] state: ConnectionState) {
        assert_eq!(PlaceholderConnectionModel::apply_emit_low(&state), None);
    }

    #[rstest]
    fn emit_queued_rejects_empty_queues() {
        let model = PlaceholderConnectionModel::default();

        assert_eq!(
            model.next_state(&ConnectionState::default(), ConnectionAction::EmitQueued),
            None
        );
    }

    #[rstest]
    #[case(ConnectionAction::EmitActiveFrame)]
    #[case(ConnectionAction::CompleteActiveOutput)]
    fn active_output_transitions_reject_idle_output(#[case] action: ConnectionAction) {
        let model = PlaceholderConnectionModel::default();

        assert_eq!(model.next_state(&ConnectionState::default(), action), None);
    }

    #[rstest]
    #[case(ActiveOutput::MultiPacket)]
    #[case(ActiveOutput::Idle)]
    fn complete_response_requires_response_output(#[case] active_output: ActiveOutput) {
        let state = ConnectionState {
            active_output,
            ..Default::default()
        };

        assert_eq!(
            PlaceholderConnectionModel::apply_complete_response(&state),
            None
        );
    }

    #[rstest]
    #[case(ActiveOutput::Response)]
    #[case(ActiveOutput::Idle)]
    fn complete_multi_packet_requires_multi_packet_output(#[case] active_output: ActiveOutput) {
        let state = ConnectionState {
            active_output,
            ..Default::default()
        };

        assert_eq!(
            PlaceholderConnectionModel::apply_complete_multi_packet(&state),
            None
        );
    }

    #[rstest]
    #[case(2, true)]
    #[case(3, true)]
    #[case(4, false)]
    fn within_boundary_respects_max_steps(#[case] steps: u8, #[case] expected: bool) {
        let model = PlaceholderConnectionModel::new(3);
        let state = ConnectionState {
            steps,
            ..Default::default()
        };

        assert_eq!(model.within_boundary(&state), expected);
    }
}
