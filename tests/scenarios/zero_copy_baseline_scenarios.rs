//! Scenario tests for the zero-copy baseline gate.

use rstest_bdd_macros::scenario;
use wireframe_testing::TestError;

use crate::fixtures::zero_copy_baseline::*;

#[scenario(
    path = "tests/features/zero_copy_baseline.feature",
    name = "The harness read-only chain leaves the payload in place"
)]
fn pass_through_middleware_preserves_payload(
    zero_copy_baseline_world: Result<ZeroCopyBaselineWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}

#[scenario(
    path = "tests/features/zero_copy_baseline.feature",
    name = "The harness copying chain is detected"
)]
fn copying_middleware_is_detected(
    zero_copy_baseline_world: Result<ZeroCopyBaselineWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}

#[scenario(
    path = "tests/features/zero_copy_baseline.feature",
    name = "The harness decode seam returns the framed payload"
)]
fn inbound_decode_returns_the_framed_payload(
    zero_copy_baseline_world: Result<ZeroCopyBaselineWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}
