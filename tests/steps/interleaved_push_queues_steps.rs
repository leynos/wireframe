//! Step definitions for interleaved push queue behavioural tests.
//!
//! Steps are synchronous but call async world methods via
//! `Runtime::new().block_on()`.

use rstest_bdd_macros::{then, when};

use crate::fixtures::interleaved_push_queues::{InterleavedPushWorld, TestResult};

#[when("both queues carry traffic with fairness disabled")]
fn when_strict(interleaved_push_world: &mut InterleavedPushWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(interleaved_push_world.run_strict_priority())
}

#[then("all high-priority frames precede all low-priority frames")]
fn then_strict(interleaved_push_world: &mut InterleavedPushWorld) {
    interleaved_push_world.verify_strict_priority();
}

#[when("both queues carry traffic with a fairness burst threshold")]
fn when_fairness(interleaved_push_world: &mut InterleavedPushWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(interleaved_push_world.run_fairness_threshold())
}

#[then("low-priority frames are interleaved at the configured interval")]
fn then_fairness(interleaved_push_world: &mut InterleavedPushWorld) {
    interleaved_push_world.verify_fairness_threshold();
}

#[when("a high-priority push exhausts the rate limit")]
fn when_rate_limit(interleaved_push_world: &mut InterleavedPushWorld) -> TestResult {
    // time::pause() requires the current_thread runtime.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(interleaved_push_world.run_rate_limit_symmetry())
}

#[then("a low-priority push is also blocked until the next window")]
fn then_rate_limit(interleaved_push_world: &mut InterleavedPushWorld) {
    interleaved_push_world.verify_rate_limit_blocked();
}

#[when("both queues carry interleaved traffic with fairness enabled")]
fn when_delivery(interleaved_push_world: &mut InterleavedPushWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(interleaved_push_world.run_interleaved_delivery())
}

#[then("every enqueued frame is present in the output")]
fn then_delivery(interleaved_push_world: &mut InterleavedPushWorld) {
    interleaved_push_world.verify_all_delivered();
}
