//! Steps for multi-packet response behavioural tests.
use cucumber::{then, when};

use crate::world::MultiPacketWorld;
type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

#[when("a handler uses the with_channel helper to emit messages")]
async fn when_multi(world: &mut MultiPacketWorld) -> TestResult { world.process().await }

#[then("all messages are received in order")]
fn then_multi(world: &mut MultiPacketWorld) { world.verify(); }

#[when("a handler uses the with_channel helper to emit no messages")]
async fn when_multi_empty(world: &mut MultiPacketWorld) -> TestResult {
    world.process_empty().await
}

#[then("no messages are received")]
fn then_multi_empty(world: &mut MultiPacketWorld) { world.verify_empty(); }

#[when("a handler emits more messages than the channel capacity")]
async fn when_multi_overflow(world: &mut MultiPacketWorld) -> TestResult {
    world.process_overflow().await
}

#[then("overflow messages are handled according to channel policy")]
fn then_multi_overflow(world: &mut MultiPacketWorld) { world.verify_overflow(); }
