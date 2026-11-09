//! Scenario tests for server-side streaming through the connection actor.

use rstest_bdd_macros::scenario;

use crate::fixtures::server_streaming::*;

#[scenario(
    path = "tests/features/server_streaming.feature",
    name = "Server streams an ordered response"
)]
fn streams_ordered_response(server_streaming_world: ServerStreamingWorld) {
    let _ = server_streaming_world;
}

#[scenario(
    path = "tests/features/server_streaming.feature",
    name = "Server streams a spawned multi-packet response"
)]
fn streams_multi_packet_response(server_streaming_world: ServerStreamingWorld) {
    let _ = server_streaming_world;
}

#[scenario(
    path = "tests/features/server_streaming.feature",
    name = "Server streams an empty response"
)]
fn streams_empty_response(server_streaming_world: ServerStreamingWorld) {
    let _ = server_streaming_world;
}
