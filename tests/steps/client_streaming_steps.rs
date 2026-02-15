//! Step definitions for client streaming response behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::client_streaming::{ClientStreamingWorld, TestResult};

#[given("a streaming echo server")]
fn given_streaming_server(client_streaming_world: &mut ClientStreamingWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        // Default server sends frames based on the scenario's When step.
        // Start with a normal 3-frame server as default; specific scenarios
        // override via their own Given steps.
        client_streaming_world.start_normal_server(3).await?;
        client_streaming_world.connect_client().await
    })
}

#[given("a streaming server that returns mismatched correlation IDs")]
fn given_mismatch_server(client_streaming_world: &mut ClientStreamingWorld) -> TestResult {
    client_streaming_world.abort_server();
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        client_streaming_world.start_mismatch_server().await?;
        client_streaming_world.connect_client().await
    })
}

#[given("a streaming server that disconnects after {count:usize} frames")]
fn given_disconnect_server(
    client_streaming_world: &mut ClientStreamingWorld,
    count: usize,
) -> TestResult {
    client_streaming_world.abort_server();
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        client_streaming_world
            .start_disconnect_server(count)
            .await?;
        client_streaming_world.connect_client().await
    })
}

#[when("the client sends a streaming request with {count:usize} data frames")]
fn when_streaming_request_with_count(
    client_streaming_world: &mut ClientStreamingWorld,
    count: usize,
) -> TestResult {
    // Restart the server with the correct frame count.
    client_streaming_world.abort_server();
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        client_streaming_world.start_normal_server(count).await?;
        client_streaming_world.connect_client().await?;
        client_streaming_world.send_streaming_request().await
    })
}

#[when("the client sends a streaming request")]
fn when_streaming_request(client_streaming_world: &mut ClientStreamingWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_streaming_world.send_streaming_request())
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
