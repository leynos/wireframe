//! Scenario tests for client tracing span behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::client_tracing::*;

#[scenario(
    path = "tests/features/client_tracing.feature",
    name = "Connect emits a tracing span with the peer address"
)]
fn connect_span_with_peer_address(client_tracing_world: ClientTracingWorld) {
    let _ = client_tracing_world;
}

#[scenario(
    path = "tests/features/client_tracing.feature",
    name = "Send emits a tracing span with frame size"
)]
fn send_span_with_frame_size(client_tracing_world: ClientTracingWorld) {
    let _ = client_tracing_world;
}

#[scenario(
    path = "tests/features/client_tracing.feature",
    name = "Receive emits a tracing span recording result"
)]
fn receive_span_recording_result(client_tracing_world: ClientTracingWorld) {
    let _ = client_tracing_world;
}

#[scenario(
    path = "tests/features/client_tracing.feature",
    name = "Per-command timing emits elapsed microseconds"
)]
fn timing_emits_elapsed_us(client_tracing_world: ClientTracingWorld) {
    let _ = client_tracing_world;
}

#[scenario(
    path = "tests/features/client_tracing.feature",
    name = "Timing is not emitted when disabled"
)]
fn timing_not_emitted_when_disabled(client_tracing_world: ClientTracingWorld) {
    let _ = client_tracing_world;
}

#[scenario(
    path = "tests/features/client_tracing.feature",
    name = "Close emits a tracing span"
)]
fn close_emits_span(client_tracing_world: ClientTracingWorld) { let _ = client_tracing_world; }
