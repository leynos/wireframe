//! Step definitions for codec-aware test harness behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::codec_test_harness::{CodecTestHarnessWorld, TestResult};

#[given(
    "a wireframe app configured with a Hotline codec allowing frames up to \
     {max_frame_length:usize} bytes"
)]
fn given_app_with_hotline_codec(
    codec_test_harness_world: &mut CodecTestHarnessWorld,
    max_frame_length: usize,
) -> TestResult {
    codec_test_harness_world.configure_app(max_frame_length)
}

#[when("a test payload is driven through the codec-aware payload driver")]
fn when_payload_driven(codec_test_harness_world: &mut CodecTestHarnessWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(codec_test_harness_world.drive_payload())
}

#[when("a test payload is driven through the codec-aware frame driver")]
fn when_frame_driven(codec_test_harness_world: &mut CodecTestHarnessWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(codec_test_harness_world.drive_frames())
}

#[then("the response payloads are non-empty")]
fn then_payloads_non_empty(codec_test_harness_world: &mut CodecTestHarnessWorld) -> TestResult {
    codec_test_harness_world.verify_payloads_non_empty()
}

#[then("the response frames contain valid transaction identifiers")]
fn then_frames_have_transaction_ids(
    codec_test_harness_world: &mut CodecTestHarnessWorld,
) -> TestResult {
    codec_test_harness_world.verify_frames_have_transaction_ids()
}
