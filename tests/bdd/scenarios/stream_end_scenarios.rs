//! Scenario tests for stream terminator features.

use rstest_bdd_macros::scenario;

use crate::fixtures::stream_end::*;

#[scenario(
    path = "tests/features/stream_end.feature",
    name = "Connection actor emits terminator after stream"
)]
fn stream_terminator(stream_end_world: StreamEndWorld) { let _ = stream_end_world; }

#[scenario(
    path = "tests/features/stream_end.feature",
    name = "Multi-packet channel emits terminator after completion"
)]
fn multi_packet_completion(stream_end_world: StreamEndWorld) { let _ = stream_end_world; }

#[scenario(
    path = "tests/features/stream_end.feature",
    name = "Multi-packet channel disconnect logs termination"
)]
fn multi_packet_disconnect(stream_end_world: StreamEndWorld) { let _ = stream_end_world; }

#[scenario(
    path = "tests/features/stream_end.feature",
    name = "Shutdown closes a multi-packet channel"
)]
fn multi_packet_shutdown(stream_end_world: StreamEndWorld) { let _ = stream_end_world; }
