//! Scenario tests for outbound streaming send behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::client_send_streaming::*;

#[scenario(
    path = "tests/features/client_send_streaming.feature",
    name = "Client sends a multi-chunk body"
)]
fn multi_chunk_body(client_send_streaming_world: ClientSendStreamingWorld) {
    let _ = client_send_streaming_world;
}

#[scenario(
    path = "tests/features/client_send_streaming.feature",
    name = "Client sends an empty body"
)]
fn empty_body(client_send_streaming_world: ClientSendStreamingWorld) {
    let _ = client_send_streaming_world;
}

#[scenario(
    path = "tests/features/client_send_streaming.feature",
    name = "Client send operation times out on a slow body reader"
)]
fn timeout_on_slow_reader(client_send_streaming_world: ClientSendStreamingWorld) {
    let _ = client_send_streaming_world;
}

#[scenario(
    path = "tests/features/client_send_streaming.feature",
    name = "Client handles transport failure during streaming send"
)]
fn transport_failure(client_send_streaming_world: ClientSendStreamingWorld) {
    let _ = client_send_streaming_world;
}
