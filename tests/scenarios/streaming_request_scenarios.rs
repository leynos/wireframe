//! Scenario tests for streaming request body behaviour.

use rstest_bdd_macros::scenario;

use crate::fixtures::streaming_request::*;

#[scenario(
    path = "tests/features/streaming_request.feature",
    name = "StreamingBody exposes an AsyncRead adapter"
)]
fn streaming_body_async_read(streaming_request_world: StreamingRequestWorld) {
    drop(streaming_request_world);
}

#[scenario(
    path = "tests/features/streaming_request.feature",
    name = "Bounded request body channels apply back-pressure"
)]
fn streaming_request_backpressure(streaming_request_world: StreamingRequestWorld) {
    drop(streaming_request_world);
}

#[scenario(
    path = "tests/features/streaming_request.feature",
    name = "Request body stream errors reach consumers"
)]
fn streaming_request_stream_error(streaming_request_world: StreamingRequestWorld) {
    drop(streaming_request_world);
}
