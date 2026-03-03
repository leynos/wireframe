//! Step definitions for partial frame and fragment feeding behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::partial_frame_feeding::{PartialFrameFeedingWorld, TestResult};

#[given(
    "a wireframe app with a Hotline codec allowing {max_frame_length:usize}-byte frames for \
     partial feeding"
)]
fn given_app_with_codec(
    partial_frame_feeding_world: &mut PartialFrameFeedingWorld,
    max_frame_length: usize,
) -> TestResult {
    partial_frame_feeding_world.configure_app(max_frame_length)
}

#[given("a fragmenter capped at {max_payload:usize} bytes per fragment for partial feeding")]
fn given_fragmenter(
    partial_frame_feeding_world: &mut PartialFrameFeedingWorld,
    max_payload: usize,
) -> TestResult {
    partial_frame_feeding_world.configure_fragmenter(max_payload)
}

#[when("a test payload is fed in {chunk_size:usize}-byte chunks")]
fn when_single_payload_chunked(
    partial_frame_feeding_world: &mut PartialFrameFeedingWorld,
    chunk_size: usize,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(partial_frame_feeding_world.drive_chunked(chunk_size))
}

#[when("{count:usize} test payloads are fed in {chunk_size:usize}-byte chunks")]
fn when_multiple_payloads_chunked(
    partial_frame_feeding_world: &mut PartialFrameFeedingWorld,
    count: usize,
    chunk_size: usize,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(partial_frame_feeding_world.drive_chunked_multiple(count, chunk_size))
}

#[when("a {payload_len:usize}-byte payload is fragmented and fed through the app")]
fn when_fragmented(
    partial_frame_feeding_world: &mut PartialFrameFeedingWorld,
    payload_len: usize,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(partial_frame_feeding_world.drive_fragmented(payload_len))
}

#[when(
    "a {payload_len:usize}-byte payload is fragmented and fed in {chunk_size:usize}-byte chunks"
)]
fn when_partial_fragmented(
    partial_frame_feeding_world: &mut PartialFrameFeedingWorld,
    payload_len: usize,
    chunk_size: usize,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(partial_frame_feeding_world.drive_partial_fragmented(payload_len, chunk_size))
}

#[then("the partial feeding response payloads are non-empty")]
fn then_payloads_non_empty(
    partial_frame_feeding_world: &mut PartialFrameFeedingWorld,
) -> TestResult {
    partial_frame_feeding_world.assert_payloads_non_empty()
}

#[then("the partial feeding response contains {expected:usize} payloads")]
fn then_payload_count(
    partial_frame_feeding_world: &mut PartialFrameFeedingWorld,
    expected: usize,
) -> TestResult {
    partial_frame_feeding_world.assert_payload_count(expected)
}

#[then("the fragment feeding completes without error")]
fn then_fragment_completed(
    partial_frame_feeding_world: &mut PartialFrameFeedingWorld,
) -> TestResult {
    partial_frame_feeding_world.assert_fragment_feeding_completed()
}
