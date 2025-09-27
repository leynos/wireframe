//! Steps for `correlation_id` behavioural tests.
use cucumber::{given, then, when};

use crate::world::CorrelationWorld;

#[given(expr = "a correlation id {int}")]
fn given_cid(world: &mut CorrelationWorld, id: u64) { world.set_cid(id); }

#[when("a stream of frames is processed")]
async fn when_process(world: &mut CorrelationWorld) { world.process().await; }

#[when("a multi-packet channel emits frames")]
async fn when_process_multi(world: &mut CorrelationWorld) { world.process_multi().await; }

#[then(expr = "each emitted frame uses correlation id {int}")]
fn then_verify(world: &mut CorrelationWorld, id: u64) {
    assert_eq!(world.cid(), id);
    world.verify();
}
