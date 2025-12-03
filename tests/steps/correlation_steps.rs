//! Steps for `correlation_id` behavioural tests.
use cucumber::{given, then, when};

use crate::world::CorrelationWorld;
type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

#[given(expr = "a correlation id {int}")]
fn given_cid(world: &mut CorrelationWorld, id: u64) { world.set_expected(Some(id)); }

#[given("no correlation id")]
fn given_no_correlation(world: &mut CorrelationWorld) { world.set_expected(None); }

#[when("a stream of frames is processed")]
async fn when_process(world: &mut CorrelationWorld) -> TestResult { world.process().await }

#[when("a multi-packet channel emits frames")]
async fn when_process_multi(world: &mut CorrelationWorld) -> TestResult {
    world.process_multi().await
}

#[then(expr = "each emitted frame uses correlation id {int}")]
fn then_verify(world: &mut CorrelationWorld, id: u64) -> TestResult {
    if world.expected() != Some(id) {
        return Err("mismatched expected correlation id".into());
    }
    world.verify()
}

#[then("each emitted frame has no correlation id")]
fn then_verify_absent(world: &mut CorrelationWorld) -> TestResult {
    if world.expected().is_some() {
        return Err("expected correlation id should be cleared".into());
    }
    world.verify()
}
