//! Step definitions for server-side streaming behavioural tests.
//!
//! Each step drives the paired server stored in [`ServerStreamingWorld`] on
//! its shared runtime, so the scenarios exercise the same connection actor
//! that production servers use. Parameter names must match the fixture name
//! declared in `tests/fixtures/server_streaming.rs`; `rstest_bdd` resolves
//! fixtures by parameter name.

use rstest_bdd_macros::{given, then, when};
use wireframe_testing::TestResult;

use crate::fixtures::server_streaming::{
    EMPTY_STREAM_ROUTE,
    MULTI_PACKET_ROUTE,
    STREAMED_ROUTE,
    ServerStreamingWorld,
};

/// Start a paired server whose routes respond with streams.
#[given("a server whose routes stream responses")]
fn given_streaming_server(server_streaming_world: &mut ServerStreamingWorld) -> TestResult {
    server_streaming_world.block_on(|state| Box::pin(state.start_server()))?
}

/// Request the route that streams three lazily produced frames.
#[when("the client requests the streamed response")]
fn when_streamed_response(server_streaming_world: &mut ServerStreamingWorld) -> TestResult {
    server_streaming_world.block_on(|state| Box::pin(state.request_route(STREAMED_ROUTE)))?
}

/// Request the route whose frames come from a spawned channel producer.
#[when("the client requests the multi-packet response")]
fn when_multi_packet_response(server_streaming_world: &mut ServerStreamingWorld) -> TestResult {
    server_streaming_world.block_on(|state| Box::pin(state.request_route(MULTI_PACKET_ROUTE)))?
}

/// Request the route whose stream yields no data frames.
#[when("the client requests the empty stream response")]
fn when_empty_stream_response(server_streaming_world: &mut ServerStreamingWorld) -> TestResult {
    server_streaming_world.block_on(|state| Box::pin(state.request_route(EMPTY_STREAM_ROUTE)))?
}

/// Assert how many data frames arrived before the terminator.
#[then("the server sends {count:usize} data frames")]
fn then_server_sends_frames(
    server_streaming_world: &ServerStreamingWorld,
    count: usize,
) -> TestResult {
    server_streaming_world.verify_frame_count(count)
}

/// Assert the data frames carry payload bytes one, two, three, and so on.
#[then("the data frames arrive in ascending order")]
fn then_frames_arrive_in_order(server_streaming_world: &ServerStreamingWorld) -> TestResult {
    server_streaming_world.verify_frame_order()
}

/// Assert the response stream ended at the protocol terminator.
#[then("the streamed response terminates cleanly")]
fn then_stream_terminates(server_streaming_world: &ServerStreamingWorld) -> TestResult {
    server_streaming_world.verify_clean_termination()
}
