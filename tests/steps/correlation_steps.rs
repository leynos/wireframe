//! Step definitions for `correlation_id` behavioural tests.
//!
//! Steps are synchronous but call async world methods via
//! `Runtime::new().block_on(...)`.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::correlation::{CorrelationWorld, TestResult};

#[given("a correlation id {id:u64}")]
fn given_cid(correlation_world: &mut CorrelationWorld, id: u64) {
    correlation_world.set_expected(Some(id));
}

#[given("no correlation id")]
fn given_no_correlation(correlation_world: &mut CorrelationWorld) {
    correlation_world.set_expected(None);
}

#[when("a stream of frames is processed")]
fn when_process(correlation_world: &mut CorrelationWorld) -> TestResult {
    // Create a new runtime for this step since we can't block_on within an
    // existing runtime
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(correlation_world.process())
}

#[when("a multi-packet channel emits frames")]
fn when_process_multi(correlation_world: &mut CorrelationWorld) -> TestResult {
    // Create a new runtime for this step since we can't block_on within an
    // existing runtime
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(correlation_world.process_multi())
}

#[then("each emitted frame uses correlation id {id:u64}")]
fn then_verify(correlation_world: &mut CorrelationWorld, id: u64) -> TestResult {
    if correlation_world.expected() != Some(id) {
        return Err("mismatched expected correlation id".into());
    }
    correlation_world.verify()
}

#[then("each emitted frame has no correlation id")]
fn then_verify_absent(correlation_world: &mut CorrelationWorld) -> TestResult {
    if correlation_world.expected().is_some() {
        return Err("expected correlation id should be cleared".into());
    }
    correlation_world.verify()
}
