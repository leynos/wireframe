//! Steps for stateful codec behavioural tests.

use cucumber::{given, then, when};

use crate::world::{CodecStatefulWorld, TestResult};

#[given(expr = "a stateful wireframe server allowing frames up to {int} bytes")]
async fn given_server(world: &mut CodecStatefulWorld, max_frame_length: usize) -> TestResult {
    world.start_server(max_frame_length).await
}

#[when(expr = "the first client sends {int} requests")]
async fn when_first_client_sends(world: &mut CodecStatefulWorld, count: usize) -> TestResult {
    world.send_first_requests(count).await
}

#[when(expr = "the second client sends {int} request")]
async fn when_second_client_sends(world: &mut CodecStatefulWorld, count: usize) -> TestResult {
    world.send_second_requests(count).await
}

#[then(expr = "the first client observes sequence numbers {int} and {int}")]
async fn then_first_client_sequences(
    world: &mut CodecStatefulWorld,
    first: u64,
    second: u64,
) -> TestResult {
    world.verify_first_sequences(&[first, second]).await
}

#[then(expr = "the second client observes sequence number {int}")]
async fn then_second_client_sequence(world: &mut CodecStatefulWorld, seq: u64) -> TestResult {
    world.verify_second_sequences(&[seq]).await
}
