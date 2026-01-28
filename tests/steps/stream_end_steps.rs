//! Step definitions for stream terminator behavioural tests.

use rstest_bdd_macros::{then, when};

use crate::fixtures::stream_end::{StreamEndWorld, TestResult};

#[when("a streaming response completes")]
fn when_stream(stream_end_world: &mut StreamEndWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(stream_end_world.process())
}

#[then("an end-of-stream frame is sent")]
fn then_end(stream_end_world: &mut StreamEndWorld) { stream_end_world.verify(); }

#[when("a multi-packet channel drains")]
fn when_multi_channel(stream_end_world: &mut StreamEndWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(stream_end_world.process_multi())
}

#[then("a multi-packet end-of-stream frame is sent")]
fn then_multi_end(stream_end_world: &mut StreamEndWorld) { stream_end_world.verify_multi(); }

#[when("a multi-packet channel disconnects abruptly")]
fn when_multi_disconnect(stream_end_world: &mut StreamEndWorld) -> TestResult {
    stream_end_world.process_multi_disconnect()
}

#[when("shutdown closes a multi-packet channel")]
fn when_multi_shutdown(stream_end_world: &mut StreamEndWorld) -> TestResult {
    stream_end_world.process_multi_shutdown()
}

#[then("no multi-packet terminator is sent")]
fn then_no_multi(stream_end_world: &mut StreamEndWorld) { stream_end_world.verify_no_multi(); }

#[then(expr = "the multi-packet termination reason is {reason:word}")]
fn then_reason(stream_end_world: &mut StreamEndWorld, reason: String) -> TestResult {
    stream_end_world.verify_reason(reason.as_str())
}
