//! Properties checked against the placeholder connection model.

use stateright::Property;

use super::{ConnectionState, PlaceholderConnectionModel};

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

fn multi_packet_terminator_is_single(
    _model: &PlaceholderConnectionModel,
    state: &ConnectionState,
) -> bool {
    state.multi_packet_terminal_count <= 1
}

fn high_priority_progress(_model: &PlaceholderConnectionModel, state: &ConnectionState) -> bool {
    state.emitted_high_priority
}

fn low_priority_progress(_model: &PlaceholderConnectionModel, state: &ConnectionState) -> bool {
    state.emitted_low_priority
}

fn response_completion(_model: &PlaceholderConnectionModel, state: &ConnectionState) -> bool {
    state.response_completed
}

fn multi_packet_completion(_model: &PlaceholderConnectionModel, state: &ConnectionState) -> bool {
    state.multi_packet_completed
}

fn shutdown_races_active_output(
    _model: &PlaceholderConnectionModel,
    state: &ConnectionState,
) -> bool {
    state.shutdown_during_output
}
