//! Step definitions for client streaming response behavioural tests.
//!
//! Steps run async world methods on the shared runtime stored in
//! `ClientStreamingWorld` rather than constructing a per-step runtime.

use std::{future::Future, pin::Pin};

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::client_streaming::{ClientStreamingWorld, TestResult};

/// Helper to abort existing server, restart with new configuration, and
/// reconnect client.
///
/// The `server_starter` closure receives the world and returns a pinned
/// future so the borrow can outlive the closure boundary.
fn with_server_restart(
    world: &mut ClientStreamingWorld,
    server_starter: impl for<'a> FnOnce(
        &'a mut ClientStreamingWorld,
    ) -> Pin<Box<dyn Future<Output = TestResult> + 'a>>,
) -> TestResult {
    world.abort_server();
    world.block_on(|w| server_starter(w))?;
    world.block_on(|w| Box::pin(w.connect_client()))
}

#[given("a streaming echo server")]
fn given_streaming_server(client_streaming_world: &mut ClientStreamingWorld) -> TestResult {
    client_streaming_world.block_on(|w| {
        Box::pin(async {
            // Default server sends frames based on the scenario's When step.
            // Start with a normal 3-frame server as default; specific scenarios
            // override via their own Given steps.
            w.start_normal_server(3).await?;
            w.connect_client().await
        })
    })
}

#[given("a streaming server that returns mismatched correlation IDs")]
fn given_mismatch_server(client_streaming_world: &mut ClientStreamingWorld) -> TestResult {
    with_server_restart(client_streaming_world, |world| {
        Box::pin(world.start_mismatch_server())
    })
}

#[given("a streaming server that disconnects after {count:usize} frames")]
fn given_disconnect_server(
    client_streaming_world: &mut ClientStreamingWorld,
    count: usize,
) -> TestResult {
    with_server_restart(client_streaming_world, |world| {
        Box::pin(world.start_disconnect_server(count))
    })
}

#[when("the client sends a streaming request with {count:usize} data frames")]
fn when_streaming_request_with_count(
    client_streaming_world: &mut ClientStreamingWorld,
    count: usize,
) -> TestResult {
    with_server_restart(client_streaming_world, |world| {
        Box::pin(world.start_normal_server(count))
    })?;
    client_streaming_world.block_on(|w| Box::pin(w.send_streaming_request()))
}

#[when("the client sends a streaming request")]
fn when_streaming_request(client_streaming_world: &mut ClientStreamingWorld) -> TestResult {
    client_streaming_world.block_on(|w| Box::pin(w.send_streaming_request()))
}

#[then("all {count:usize} data frames are received in order")]
fn then_frames_received_in_order(
    client_streaming_world: &mut ClientStreamingWorld,
    count: usize,
) -> TestResult {
    client_streaming_world.verify_frame_count(count)?;
    client_streaming_world.verify_frame_order()
}

#[then("no data frames are received")]
fn then_no_frames(client_streaming_world: &mut ClientStreamingWorld) -> TestResult {
    client_streaming_world.verify_frame_count(0)
}

#[then("the stream terminates cleanly")]
fn then_clean_termination(client_streaming_world: &mut ClientStreamingWorld) -> TestResult {
    client_streaming_world.verify_clean_termination()
}

#[then("a StreamCorrelationMismatch error is returned")]
fn then_correlation_mismatch(client_streaming_world: &mut ClientStreamingWorld) -> TestResult {
    client_streaming_world.verify_correlation_mismatch_error()
}

#[then("{count:usize} data frames are received")]
fn then_n_frames_received(
    client_streaming_world: &mut ClientStreamingWorld,
    count: usize,
) -> TestResult {
    client_streaming_world.verify_frame_count(count)
}

#[then("a disconnection error is returned")]
fn then_disconnect_error(client_streaming_world: &mut ClientStreamingWorld) -> TestResult {
    client_streaming_world.verify_disconnect_error()
}
