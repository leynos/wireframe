//! Step definitions for stateful codec behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::codec_stateful::{CodecStatefulWorld, TestResult};

#[given("a stateful wireframe server allowing frames up to {max_frame_length:usize} bytes")]
fn given_server(
    codec_stateful_world: &mut CodecStatefulWorld,
    max_frame_length: usize,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(codec_stateful_world.start_server(max_frame_length))
}

#[when("the first client sends {count:usize} requests")]
fn when_first_client_sends(
    codec_stateful_world: &mut CodecStatefulWorld,
    count: usize,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(codec_stateful_world.send_first_requests(count))
}

#[when("the second client sends {count:usize} request")]
fn when_second_client_sends(
    codec_stateful_world: &mut CodecStatefulWorld,
    count: usize,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(codec_stateful_world.send_second_requests(count))
}

#[then("the first client observes sequence numbers {first:u64} and {second:u64}")]
fn then_first_client_sequences(
    codec_stateful_world: &mut CodecStatefulWorld,
    first: u64,
    second: u64,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(codec_stateful_world.verify_first_sequences(&[first, second]))
}

#[then("the second client observes sequence number {seq:u64}")]
fn then_second_client_sequence(
    codec_stateful_world: &mut CodecStatefulWorld,
    seq: u64,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(codec_stateful_world.verify_second_sequences(&[seq]))
}
