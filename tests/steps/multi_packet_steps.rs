//! Steps for multi-packet response behavioural tests.
use cucumber::{then, when};

use crate::world::MultiPacketWorld;

#[when("a multi-packet response emits messages")]
async fn when_multi(world: &mut MultiPacketWorld) { world.process().await; }

#[then("all messages are received in order")]
fn then_multi(world: &mut MultiPacketWorld) { world.verify(); }

#[when("a multi-packet response emits no messages")]
async fn when_multi_empty(world: &mut MultiPacketWorld) { world.process_empty().await; }

#[then("no messages are received")]
fn then_multi_empty(world: &mut MultiPacketWorld) { world.verify_empty(); }
