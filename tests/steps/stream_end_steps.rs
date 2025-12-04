//! Steps for stream terminator behavioural tests.
use cucumber::{then, when};

use crate::world::{StreamEndWorld, TestResult};

#[when("a streaming response completes")]
async fn when_stream(world: &mut StreamEndWorld) -> TestResult { world.process().await }

#[then("an end-of-stream frame is sent")]
fn then_end(world: &mut StreamEndWorld) { world.verify(); }

#[when("a multi-packet channel drains")]
async fn when_multi_channel(world: &mut StreamEndWorld) -> TestResult {
    world.process_multi().await
}

#[then("a multi-packet end-of-stream frame is sent")]
fn then_multi_end(world: &mut StreamEndWorld) { world.verify_multi(); }

#[when("a multi-packet channel disconnects abruptly")]
fn when_multi_disconnect(world: &mut StreamEndWorld) -> TestResult {
    world.process_multi_disconnect()
}

#[when("shutdown closes a multi-packet channel")]
fn when_multi_shutdown(world: &mut StreamEndWorld) -> TestResult { world.process_multi_shutdown() }

#[then("no multi-packet terminator is sent")]
fn then_no_multi(world: &mut StreamEndWorld) { world.verify_no_multi(); }

#[then(expr = "the multi-packet termination reason is {word}")]
fn then_reason(world: &mut StreamEndWorld, reason: String) -> TestResult {
    let reason = reason.into_boxed_str();
    world.verify_reason(reason.as_ref())
}
