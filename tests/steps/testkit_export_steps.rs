//! Step definitions for `wireframe::testkit` export behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::testkit_export::{TestResult, TestkitExportWorld};

#[given(
    "a wireframe app with a Hotline codec allowing {max_frame_length:usize}-byte frames for \
     testkit export"
)]
fn given_app_with_codec(
    testkit_export_world: &mut TestkitExportWorld,
    max_frame_length: usize,
) -> TestResult {
    testkit_export_world.configure_app(max_frame_length)
}

#[when("a payload is driven through `wireframe::testkit` in {chunk_size:usize}-byte chunks")]
fn when_payload_is_driven(
    testkit_export_world: &mut TestkitExportWorld,
    chunk_size: usize,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(testkit_export_world.drive_chunked(chunk_size))
}

#[then("the testkit export returns a non-empty response payload")]
fn then_non_empty_payload(testkit_export_world: &mut TestkitExportWorld) -> TestResult {
    testkit_export_world.assert_response_payloads_non_empty()
}

#[given("a completed message-assembly snapshot for key {key:u64} in the testkit export world")]
fn given_completed_snapshot(testkit_export_world: &mut TestkitExportWorld, key: u64) -> TestResult {
    let _ = usize::try_from(key)?;
    testkit_export_world.seed_completed_message_snapshot(key);
    Ok(())
}

#[when("the snapshot is asserted through `wireframe::testkit`")]
fn when_snapshot_is_asserted(testkit_export_world: &mut TestkitExportWorld) -> TestResult {
    testkit_export_world.assert_root_reassembly_helper()
}

#[then("the testkit reassembly assertion succeeds")]
fn then_reassembly_assertion_succeeds(testkit_export_world: &mut TestkitExportWorld) -> TestResult {
    testkit_export_world.assert_reassembly_assertion_passed()
}
