//! Steps for correlation id behavioural tests.

use cucumber::{given, then, when};

use crate::correlation_world::CorrelationWorld;

#[given(regex = "a multi-packet response stream with correlation id (\\d+)")]
fn set_id(world: &mut CorrelationWorld, id: u32) {
    world.set_id(id);
}

#[when("the connection actor runs to completion")]
async fn run(world: &mut CorrelationWorld) {
    world.run_actor().await;
}

#[then(regex = "each emitted frame has correlation id (\\d+)")]
fn verify(world: &mut CorrelationWorld, id: u32) {
    assert_eq!(world.correlation_id, id);
    world.assert_ids();
}
