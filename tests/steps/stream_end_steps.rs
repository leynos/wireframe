//! Steps for stream terminator behavioural tests.
use cucumber::{then, when};

use crate::world::StreamEndWorld;

#[when("a streaming response completes")]
async fn when_stream(world: &mut StreamEndWorld) { world.process().await; }

#[then("an end-of-stream frame is sent")]
fn then_end(world: &mut StreamEndWorld) { world.verify(); }
