//! Scenario tests for streaming request-body dispatch.

use rstest_bdd_macros::scenario;
use wireframe_testing::TestError;

use crate::fixtures::streaming_dispatch::*;

#[scenario(
    path = "tests/features/streaming_request_dispatch.feature",
    name = "A streaming body is delivered while its frames arrive"
)]
fn streaming_body_is_delivered_incrementally(
    streaming_dispatch_world: Result<StreamingDispatchWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}

#[scenario(
    path = "tests/features/streaming_request_dispatch.feature",
    name = "A complete buffered message keeps its reassembly guarantees"
)]
fn buffered_message_keeps_reassembly(
    streaming_dispatch_world: Result<StreamingDispatchWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}

#[scenario(
    path = "tests/features/streaming_request_dispatch.feature",
    name = "A paused frame header resumes without losing the message"
)]
fn paused_frame_header_resumes(
    streaming_dispatch_world: Result<StreamingDispatchWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}

#[scenario(
    path = "tests/features/streaming_request_dispatch.feature",
    name = "A held consumer parks later messages and replies in order"
)]
fn held_consumer_parks_later_messages(
    streaming_dispatch_world: Result<StreamingDispatchWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}

#[scenario(
    path = "tests/features/streaming_request_dispatch.feature",
    name = "An abandoned body does not block the next message"
)]
fn abandoned_body_does_not_block(
    streaming_dispatch_world: Result<StreamingDispatchWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}

#[scenario(
    path = "tests/features/streaming_request_dispatch.feature",
    name = "End of stream reports an unexpected end to the body"
)]
fn end_of_stream_reports_unexpected_eof(
    streaming_dispatch_world: Result<StreamingDispatchWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}

#[scenario(
    path = "tests/features/streaming_request_dispatch.feature",
    name = "A declared total overshoot fails the body"
)]
fn declared_total_overshoot_fails_body(
    streaming_dispatch_world: Result<StreamingDispatchWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}

#[scenario(
    path = "tests/features/streaming_request_dispatch.feature",
    name = "A short final frame fails the body"
)]
fn short_final_frame_fails_body(
    streaming_dispatch_world: Result<StreamingDispatchWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}

#[scenario(
    path = "tests/features/streaming_request_dispatch.feature",
    name = "An idle body expires and reports a timeout"
)]
fn idle_body_expires(
    streaming_dispatch_world: Result<StreamingDispatchWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}
