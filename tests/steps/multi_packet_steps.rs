//! Step definitions for multi-packet response behavioural tests.
//!
//! Steps are synchronous but call async World methods via
//! `Runtime::new().block_on()` (`current_thread` runtime doesn't support
//! `block_in_place`).

use rstest_bdd_macros::{then, when};

use crate::fixtures::multi_packet::{MultiPacketWorld, TestResult};

#[when("a handler uses the with_channel helper to emit messages")]
fn when_multi(multi_packet_world: &mut MultiPacketWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(multi_packet_world.process())
}

#[then("all messages are received in order")]
fn then_multi(multi_packet_world: &mut MultiPacketWorld) { multi_packet_world.verify(); }

#[when("a handler uses the with_channel helper to emit no messages")]
fn when_multi_empty(multi_packet_world: &mut MultiPacketWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(multi_packet_world.process_empty())
}

#[then("no messages are received")]
fn then_multi_empty(multi_packet_world: &mut MultiPacketWorld) {
    multi_packet_world.verify_empty();
}

#[when("a handler emits more messages than the channel capacity")]
fn when_multi_overflow(multi_packet_world: &mut MultiPacketWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(multi_packet_world.process_overflow())
}

#[then("overflow messages are handled according to channel policy")]
fn then_multi_overflow(multi_packet_world: &mut MultiPacketWorld) {
    multi_packet_world.verify_overflow();
}
