//! Properties checked against the placeholder connection model.

use stateright::Property;

use super::{ConnectionState, PlaceholderConnectionModel};

/// Returns the full set of Stateright properties checked against [`PlaceholderConnectionModel`].
pub(super) fn properties() -> Vec<Property<PlaceholderConnectionModel>> {
    vec![
        Property::always(
            "multi-packet terminator stays single",
            multi_packet_terminator_is_single,
        ),
        Property::sometimes(
            "high-priority output can make progress",
            high_priority_progress,
        ),
        Property::sometimes(
            "low-priority output can make progress",
            low_priority_progress,
        ),
        Property::sometimes("response output can complete", response_completion),
        Property::sometimes("multi-packet output can complete", multi_packet_completion),
        Property::sometimes(
            "shutdown can race an active output",
            shutdown_races_active_output,
        ),
    ]
}

/// Invariant: the multi-packet terminator count never exceeds 1.
fn multi_packet_terminator_is_single(
    _model: &PlaceholderConnectionModel,
    state: &ConnectionState,
) -> bool {
    state.multi_packet_terminal_count <= 1
}

/// Witness: a high-priority frame has been emitted.
fn high_priority_progress(_model: &PlaceholderConnectionModel, state: &ConnectionState) -> bool {
    state.emitted_high_priority
}

/// Witness: a low-priority frame has been emitted.
fn low_priority_progress(_model: &PlaceholderConnectionModel, state: &ConnectionState) -> bool {
    state.emitted_low_priority
}

/// Witness: a response output stream has completed.
fn response_completion(_model: &PlaceholderConnectionModel, state: &ConnectionState) -> bool {
    state.response_completed
}

/// Witness: a multi-packet output stream has completed.
fn multi_packet_completion(_model: &PlaceholderConnectionModel, state: &ConnectionState) -> bool {
    state.multi_packet_completed
}

/// Witness: shutdown was requested while an output was active.
fn shutdown_races_active_output(
    _model: &PlaceholderConnectionModel,
    state: &ConnectionState,
) -> bool {
    state.shutdown_during_output
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(0, true)]
    #[case(1, true)]
    #[case(2, false)]
    fn multi_packet_terminator_predicate_rejects_counts_above_one(
        #[case] terminal_count: u8,
        #[case] expected: bool,
    ) {
        let model = PlaceholderConnectionModel::default();
        let state = ConnectionState {
            multi_packet_terminal_count: terminal_count,
            ..Default::default()
        };

        assert_eq!(multi_packet_terminator_is_single(&model, &state), expected);
    }

    #[rstest]
    #[case(false, false)]
    #[case(true, true)]
    fn high_priority_progress_reflects_emitted_flag(
        #[case] emitted_high_priority: bool,
        #[case] expected: bool,
    ) {
        let model = PlaceholderConnectionModel::default();
        let state = ConnectionState {
            emitted_high_priority,
            ..Default::default()
        };

        assert_eq!(high_priority_progress(&model, &state), expected);
    }

    #[rstest]
    #[case(false, false)]
    #[case(true, true)]
    fn low_priority_progress_reflects_emitted_flag(
        #[case] emitted_low_priority: bool,
        #[case] expected: bool,
    ) {
        let model = PlaceholderConnectionModel::default();
        let state = ConnectionState {
            emitted_low_priority,
            ..Default::default()
        };

        assert_eq!(low_priority_progress(&model, &state), expected);
    }

    #[rstest]
    #[case(false, false)]
    #[case(true, true)]
    fn response_completion_reflects_completion_flag(
        #[case] response_completed: bool,
        #[case] expected: bool,
    ) {
        let model = PlaceholderConnectionModel::default();
        let state = ConnectionState {
            response_completed,
            ..Default::default()
        };

        assert_eq!(response_completion(&model, &state), expected);
    }

    #[rstest]
    #[case(false, false)]
    #[case(true, true)]
    fn multi_packet_completion_reflects_completion_flag(
        #[case] multi_packet_completed: bool,
        #[case] expected: bool,
    ) {
        let model = PlaceholderConnectionModel::default();
        let state = ConnectionState {
            multi_packet_completed,
            ..Default::default()
        };

        assert_eq!(multi_packet_completion(&model, &state), expected);
    }

    #[rstest]
    #[case(false, false)]
    #[case(true, true)]
    fn shutdown_race_predicate_reflects_race_flag(
        #[case] shutdown_during_output: bool,
        #[case] expected: bool,
    ) {
        let model = PlaceholderConnectionModel::default();
        let state = ConnectionState {
            shutdown_during_output,
            ..Default::default()
        };

        assert_eq!(shutdown_races_active_output(&model, &state), expected);
    }
}
