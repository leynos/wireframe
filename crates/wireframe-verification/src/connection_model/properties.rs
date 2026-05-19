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
