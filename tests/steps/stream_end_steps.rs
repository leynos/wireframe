//! Steps for stream terminator behavioural tests.
use cucumber::{then, when};

use crate::world::StreamEndWorld;

#[when("a streaming response completes")]
async fn when_stream(world: &mut StreamEndWorld) { world.process().await; }

#[then("an end-of-stream frame is sent")]
fn then_end(world: &mut StreamEndWorld) { world.verify(); }

#[when("a multi-packet channel drains")]
async fn when_multi_channel(world: &mut StreamEndWorld) { world.process_multi().await; }

#[then("a multi-packet end-of-stream frame is sent")]
fn then_multi_end(world: &mut StreamEndWorld) { world.verify_multi(); }
