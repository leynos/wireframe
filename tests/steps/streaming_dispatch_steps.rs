//! Step definitions for streaming request-body dispatch scenarios.

use std::io;

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::streaming_dispatch::{ContinuationSpec, StreamingDispatchWorld, TestResult};

/// Map a scenario's body-error description onto the I/O error kind the
/// runtime reports.
fn body_error_kind(text: &str) -> TestResult<io::ErrorKind> {
    match text {
        "unexpected end of file" => Ok(io::ErrorKind::UnexpectedEof),
        "invalid data" => Ok(io::ErrorKind::InvalidData),
        "timed out" => Ok(io::ErrorKind::TimedOut),
        other => Err(format!("unknown body error {other:?}").into()),
    }
}

#[given("an app with a streaming route and a buffered route")]
fn given_streaming_and_buffered_app(
    streaming_dispatch_world: &mut StreamingDispatchWorld,
) -> TestResult {
    streaming_dispatch_world.start_consuming()
}

#[given("an app whose streaming handler holds its body")]
fn given_holding_app(streaming_dispatch_world: &mut StreamingDispatchWorld) -> TestResult {
    streaming_dispatch_world.start_holding()
}

#[given("an app whose streaming handler abandons its body")]
fn given_abandoning_app(streaming_dispatch_world: &mut StreamingDispatchWorld) -> TestResult {
    streaming_dispatch_world.start_abandoning()
}

#[given("an app with a streaming route and a {millis:u64} millisecond assembly timeout")]
fn given_app_with_timeout(
    streaming_dispatch_world: &mut StreamingDispatchWorld,
    millis: u64,
) -> TestResult {
    streaming_dispatch_world.start_with_timeout(millis)
}

#[when("a streaming first frame for key {key:u64} with body {body:string} arrives")]
fn when_streaming_first_frame(
    streaming_dispatch_world: &mut StreamingDispatchWorld,
    key: u64,
    body: String,
) -> TestResult {
    streaming_dispatch_world.send_streaming_first(key, &body, None)
}

#[when(
    "a streaming first frame for key {key:u64} with body {body:string} and declared total \
     {total:u32} arrives"
)]
fn when_streaming_first_frame_with_total(
    streaming_dispatch_world: &mut StreamingDispatchWorld,
    key: u64,
    body: String,
    total: u32,
) -> TestResult {
    streaming_dispatch_world.send_streaming_first(key, &body, Some(total))
}

#[when(
    "a final streaming continuation frame for key {key:u64} sequence {sequence:u32} with body \
     {body:string} arrives"
)]
fn when_final_streaming_continuation(
    streaming_dispatch_world: &mut StreamingDispatchWorld,
    key: u64,
    sequence: u32,
    body: String,
) -> TestResult {
    streaming_dispatch_world.send_streaming_continuation(ContinuationSpec::new(
        key.into(),
        sequence.into(),
        &body,
        true,
    ))
}

#[when("a complete buffered message for key {key:u64} with body {body:string} arrives")]
fn when_complete_buffered_message(
    streaming_dispatch_world: &mut StreamingDispatchWorld,
    key: u64,
    body: String,
) -> TestResult {
    streaming_dispatch_world.send_buffered_first(key, &body, true)
}

#[when("a non-final buffered frame for key {key:u64} with body {body:string} arrives")]
fn when_non_final_buffered_frame(
    streaming_dispatch_world: &mut StreamingDispatchWorld,
    key: u64,
    body: String,
) -> TestResult {
    streaming_dispatch_world.send_buffered_first(key, &body, false)
}

#[when(
    "a final buffered continuation frame for key {key:u64} sequence {sequence:u32} with body \
     {body:string} arrives"
)]
fn when_final_buffered_continuation(
    streaming_dispatch_world: &mut StreamingDispatchWorld,
    key: u64,
    sequence: u32,
    body: String,
) -> TestResult {
    streaming_dispatch_world.send_buffered_continuation(ContinuationSpec::new(
        key.into(),
        sequence.into(),
        &body,
        true,
    ))
}

#[when(
    "the first part of a complete buffered message for key {key:u64} with body {body:string} \
     arrives"
)]
fn when_first_part_of_buffered_message(
    streaming_dispatch_world: &mut StreamingDispatchWorld,
    key: u64,
    body: String,
) -> TestResult {
    streaming_dispatch_world.send_split_buffered_first(key, &body)
}

#[when("the rest of the buffered message arrives")]
fn when_rest_of_buffered_message(
    streaming_dispatch_world: &mut StreamingDispatchWorld,
) -> TestResult {
    streaming_dispatch_world.send_pending_remainder()
}

#[when("the handler is released")]
fn when_handler_released(streaming_dispatch_world: &mut StreamingDispatchWorld) {
    streaming_dispatch_world.release_handler();
}

#[when("the connection closes")]
fn when_connection_closes(streaming_dispatch_world: &mut StreamingDispatchWorld) -> TestResult {
    streaming_dispatch_world.close_connection()
}

#[when("time passes for {millis:u64} milliseconds")]
fn when_time_passes(
    streaming_dispatch_world: &mut StreamingDispatchWorld,
    millis: u64,
) -> TestResult {
    streaming_dispatch_world.wait_millis(millis)
}

#[then("the handler receives chunk {body:string}")]
fn then_handler_receives_chunk(
    streaming_dispatch_world: &mut StreamingDispatchWorld,
    body: String,
) -> TestResult {
    streaming_dispatch_world.assert_chunk_received(&body)
}

#[then("the handler is still reading its body")]
fn then_handler_still_reading(streaming_dispatch_world: &mut StreamingDispatchWorld) -> TestResult {
    streaming_dispatch_world.assert_body_still_open()
}

#[then("the handler observes the body completed")]
fn then_body_completed(streaming_dispatch_world: &mut StreamingDispatchWorld) -> TestResult {
    streaming_dispatch_world.assert_body_completed()
}

#[then("the handler observes body error {kind:string}")]
fn then_body_error(
    streaming_dispatch_world: &mut StreamingDispatchWorld,
    kind: String,
) -> TestResult {
    streaming_dispatch_world.assert_body_error(body_error_kind(&kind)?)
}

#[then("the buffered handler receives payload {body:string}")]
fn then_buffered_payload(
    streaming_dispatch_world: &mut StreamingDispatchWorld,
    body: String,
) -> TestResult {
    streaming_dispatch_world.assert_buffered_payload(&body)
}

#[then("the buffered handler receives no payload")]
fn then_no_buffered_payload(streaming_dispatch_world: &mut StreamingDispatchWorld) -> TestResult {
    streaming_dispatch_world.assert_no_buffered_payload()
}

#[then("the reply payload is {body:string}")]
fn then_reply_payload(
    streaming_dispatch_world: &mut StreamingDispatchWorld,
    body: String,
) -> TestResult {
    streaming_dispatch_world.assert_reply(&body)
}

#[then("the connection ends without error")]
fn then_connection_ends_cleanly(
    streaming_dispatch_world: &mut StreamingDispatchWorld,
) -> TestResult {
    streaming_dispatch_world.assert_server_ok()
}
