//! Step definitions for outbound streaming send behavioural tests.
//!
//! All step phrases use a `send-streaming` prefix to avoid collisions
//! with existing `client_streaming` step definitions.

use std::time::Duration;

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::client_send_streaming::{ClientSendStreamingWorld, TestResult};

#[given("a send-streaming receiving server")]
fn given_receiving_server(
    client_send_streaming_world: &mut ClientSendStreamingWorld,
) -> TestResult {
    client_send_streaming_world.block_on(|w| {
        Box::pin(async {
            w.start_receiving_server().await?;
            w.connect_client().await
        })
    })?
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "step signature mandated by the #[given] proc macro"
)]
#[given("a send-streaming body reader that blocks indefinitely")]
fn given_blocking_reader(client_send_streaming_world: &mut ClientSendStreamingWorld) -> TestResult {
    client_send_streaming_world.set_blocking_reader();
    Ok(())
}

#[given("a send-streaming server that disconnects immediately")]
fn given_dropping_server(client_send_streaming_world: &mut ClientSendStreamingWorld) -> TestResult {
    client_send_streaming_world.abort_server();
    client_send_streaming_world.block_on(|w| {
        Box::pin(async {
            w.start_dropping_server().await?;
            w.connect_client().await
        })
    })?
}

#[when(
    "the client streams {body_size:usize} bytes with a {header_size:usize} byte header and \
     {chunk_size:usize} byte chunks"
)]
fn when_send_streaming(
    client_send_streaming_world: &mut ClientSendStreamingWorld,
    body_size: usize,
    _header_size: usize,
    chunk_size: usize,
) -> TestResult {
    client_send_streaming_world
        .block_on(|w| Box::pin(w.do_send_streaming(body_size, chunk_size)))?
}

#[when("the client streams with a {ms:u64} ms timeout")]
fn when_send_streaming_timeout(
    client_send_streaming_world: &mut ClientSendStreamingWorld,
    ms: u64,
) -> TestResult {
    client_send_streaming_world
        .block_on(|w| Box::pin(w.do_send_streaming_with_timeout(Duration::from_millis(ms))))?
}

#[then("the send-streaming server receives {count:usize} frames")]
fn then_server_frame_count(
    client_send_streaming_world: &mut ClientSendStreamingWorld,
    count: usize,
) -> TestResult {
    // Drop the client and collect frames from the server task.
    client_send_streaming_world.block_on(|w| Box::pin(w.collect_server_frames()))??;
    client_send_streaming_world.verify_server_frame_count(count)
}

#[then("each send-streaming frame starts with the protocol header")]
fn then_frames_start_with_header(
    client_send_streaming_world: &mut ClientSendStreamingWorld,
) -> TestResult {
    client_send_streaming_world.verify_frames_start_with_header()
}

#[then("a send-streaming TimedOut error is returned")]
fn then_timed_out_error(client_send_streaming_world: &mut ClientSendStreamingWorld) -> TestResult {
    client_send_streaming_world.verify_timed_out_error()
}

#[then("a send-streaming transport error is returned")]
fn then_transport_error(client_send_streaming_world: &mut ClientSendStreamingWorld) -> TestResult {
    client_send_streaming_world.verify_transport_error()
}
