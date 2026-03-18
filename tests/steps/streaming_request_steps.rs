//! Step definitions for streaming request body scenarios.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::streaming_request::{ErrorKindArg, StreamingRequestWorld, TestResult};

#[given("a streaming request body channel with capacity {capacity:usize}")]
fn given_request_body_channel(
    streaming_request_world: &mut StreamingRequestWorld,
    capacity: usize,
) -> TestResult {
    streaming_request_world.create_channel(capacity)
}

#[when("body chunks {first:string} and {second:string} are sent")]
fn when_body_chunks_are_sent(
    streaming_request_world: &mut StreamingRequestWorld,
    first: String,
    second: String,
) -> TestResult {
    streaming_request_world.send_chunk(&first)?;
    streaming_request_world.send_chunk(&second)
}

#[when("body chunk {chunk:string} is sent")]
fn when_body_chunk_is_sent(
    streaming_request_world: &mut StreamingRequestWorld,
    chunk: String,
) -> TestResult {
    streaming_request_world.send_chunk(&chunk)
}

#[when("one body chunk {chunk:string} is buffered without draining the stream")]
fn when_chunk_is_buffered_without_draining(
    streaming_request_world: &mut StreamingRequestWorld,
    chunk: String,
) -> TestResult {
    streaming_request_world.send_chunk(&chunk)
}

#[when("another body chunk {chunk:string} is sent with a {timeout_ms:u64} millisecond timeout")]
fn when_body_chunk_is_sent_with_timeout(
    streaming_request_world: &mut StreamingRequestWorld,
    chunk: String,
    timeout_ms: u64,
) -> TestResult {
    streaming_request_world.send_chunk_with_timeout(&chunk, timeout_ms)
}

#[when("a request body error of kind {kind:string} is sent")]
fn when_request_body_error_is_sent(
    streaming_request_world: &mut StreamingRequestWorld,
    kind: ErrorKindArg,
) -> TestResult {
    streaming_request_world.send_error(kind.into())
}

#[when("the streaming body is read through the AsyncRead adapter")]
fn when_streaming_body_is_read(streaming_request_world: &mut StreamingRequestWorld) -> TestResult {
    streaming_request_world.drain_with_reader()
}

#[when("the request body stream is drained directly")]
fn when_request_body_stream_is_drained(
    streaming_request_world: &mut StreamingRequestWorld,
) -> TestResult {
    streaming_request_world.drain_stream()
}

#[then("the collected body is {expected:string}")]
fn then_collected_body_is(
    streaming_request_world: &mut StreamingRequestWorld,
    expected: String,
) -> TestResult {
    streaming_request_world.assert_collected_body(&expected)
}

#[then("the send is blocked by back-pressure")]
fn then_send_is_blocked_by_backpressure(
    streaming_request_world: &mut StreamingRequestWorld,
) -> TestResult {
    streaming_request_world.assert_send_blocked_by_backpressure()
}

#[then("one chunk is received before the error")]
fn then_one_chunk_is_received(streaming_request_world: &mut StreamingRequestWorld) -> TestResult {
    streaming_request_world.assert_collected_chunks(1)
}

#[then("the last stream error kind is {kind:string}")]
fn then_last_stream_error_kind_is(
    streaming_request_world: &mut StreamingRequestWorld,
    kind: ErrorKindArg,
) -> TestResult {
    streaming_request_world.assert_last_error_kind(kind.into())
}
