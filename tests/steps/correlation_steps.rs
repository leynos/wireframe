//! Steps for `correlation_id` behavioural tests.
use cucumber::{given, then, when};

use crate::world::CorrelationWorld;

#[given(expr = "a correlation id {int}")]
fn given_cid(world: &mut CorrelationWorld, id: u64) { world.set_expected(Some(id)); }

#[given("no correlation id")]
fn given_no_correlation(world: &mut CorrelationWorld) { world.set_expected(None); }

#[when("a stream of frames is processed")]
async fn when_process(world: &mut CorrelationWorld) { world.process().await; }

#[when("a multi-packet channel emits frames")]
async fn when_process_multi(world: &mut CorrelationWorld) { world.process_multi().await; }

#[then(expr = "each emitted frame uses correlation id {int}")]
fn then_verify(world: &mut CorrelationWorld, id: u64) {
    assert_eq!(world.expected(), Some(id));
    world.verify();
}

#[then("each emitted frame has no correlation id")]
fn then_verify_absent(world: &mut CorrelationWorld) {
    assert_eq!(world.expected(), None);
    world.verify();
}
