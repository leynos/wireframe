//! State for the placeholder connection-actor model.

/// Active output tracked by the placeholder model.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ActiveOutput {
    /// No response or multi-packet output is currently active.
    #[default]
    Idle,
    /// A response stream is active.
    Response,
    /// A multi-packet stream is active.
    MultiPacket,
}

/// Model state for the placeholder connection-actor abstraction.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct ConnectionState {
    /// Number of transitions taken so far; bounded by `PlaceholderConnectionModel::max_steps`.
    pub(crate) steps: u8,
    /// Queued-output state that governs priority selection and fairness.
    pub(super) pending_outputs: PendingOutputs,
    /// Output stream currently held by the connection.
    pub(crate) active_output: ActiveOutput,
    /// Shutdown request and its interaction with the active output.
    pub(super) shutdown: ShutdownState,
    /// Observable progress markers for priority emissions.
    pub(super) emissions: EmissionEvidence,
    /// Completion markers retained as liveness witnesses for both stream kinds.
    pub(super) completed_outputs: CompletedOutputs,
    /// Number of terminator frames appended to multi-packet streams.
    pub(crate) multi_packet_terminal_count: u8,
}

/// Queue facts kept together because fairness interprets them as one decision.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(super) struct PendingOutputs {
    /// Records that a high-priority item must be selected before ordinary work.
    pub(super) high_priority_queued: bool,
    /// Records that an ordinary item remains eligible once fairness permits it.
    pub(super) low_priority_queued: bool,
    /// Grants one low-priority turn after high-priority traffic has progressed.
    pub(super) fairness_allows_low: bool,
}

/// Shutdown facts retained so the checker can witness cancellation races.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(super) struct ShutdownState {
    /// Prevents installation of new output after connection teardown begins.
    pub(super) requested: bool,
    /// Captures that teardown overlapped an already-active output stream.
    pub(super) during_output: bool,
}

/// Emission witnesses used only to prove that each priority can make progress.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(super) struct EmissionEvidence {
    /// Shows the high-priority path reached an emission transition.
    pub(super) high_priority: bool,
    /// Shows the low-priority path reached an emission transition.
    pub(super) low_priority: bool,
}

/// Completion witnesses used to keep response and multi-packet liveness distinct.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(super) struct CompletedOutputs {
    /// Shows a response stream returned ownership of the active-output slot.
    pub(super) response: bool,
    /// Shows a multi-packet stream returned ownership of the active-output slot.
    pub(super) multi_packet: bool,
}

impl ConnectionState {
    /// Returns `true` when no output stream is currently active.
    pub(crate) fn is_output_idle(&self) -> bool { matches!(self.active_output, ActiveOutput::Idle) }

    /// Returns `true` when a new output stream may be installed (output idle and no shutdown
    /// pending).
    pub(crate) fn can_install_output(&self) -> bool {
        !self.shutdown.requested && self.is_output_idle()
    }

    /// Returns `true` when the fairness policy permits emitting the queued low-priority output.
    pub(crate) fn can_emit_low_priority(&self) -> bool {
        self.pending_outputs.low_priority_queued
            && (!self.pending_outputs.high_priority_queued
                || self.pending_outputs.fairness_allows_low)
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{ActiveOutput, ConnectionState, PendingOutputs, ShutdownState};

    #[rstest]
    #[case(ActiveOutput::Idle, true)]
    #[case(ActiveOutput::Response, false)]
    #[case(ActiveOutput::MultiPacket, false)]
    fn is_output_idle_reflects_active_output(#[case] active: ActiveOutput, #[case] expected: bool) {
        let state = ConnectionState {
            active_output: active,
            ..Default::default()
        };

        assert_eq!(state.is_output_idle(), expected);
    }

    #[rstest]
    #[case(false, ActiveOutput::Idle, true)]
    #[case(true, ActiveOutput::Idle, false)]
    #[case(false, ActiveOutput::Response, false)]
    #[case(true, ActiveOutput::Response, false)]
    fn can_install_output_requires_idle_and_no_shutdown(
        #[case] shutdown: bool,
        #[case] active: ActiveOutput,
        #[case] expected: bool,
    ) {
        let state = ConnectionState {
            shutdown: ShutdownState {
                requested: shutdown,
                ..Default::default()
            },
            active_output: active,
            ..Default::default()
        };

        assert_eq!(state.can_install_output(), expected);
    }

    #[rstest]
    #[case(true, false, false, true)]
    #[case(true, true, false, false)]
    #[case(true, true, true, true)]
    #[case(false, false, false, false)]
    #[case(false, true, true, false)]
    fn can_emit_low_priority_respects_fairness(
        #[case] low: bool,
        #[case] high: bool,
        #[case] fairness: bool,
        #[case] expected: bool,
    ) {
        let state = ConnectionState {
            pending_outputs: PendingOutputs {
                low_priority_queued: low,
                high_priority_queued: high,
                fairness_allows_low: fairness,
            },
            ..Default::default()
        };

        assert_eq!(state.can_emit_low_priority(), expected);
    }
}
