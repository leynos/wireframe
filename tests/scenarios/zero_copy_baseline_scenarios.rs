//! Scenario tests for the zero-copy baseline gate.

use rstest_bdd_macros::scenario;
use wireframe_testing::TestError;

use crate::fixtures::zero_copy_baseline::*;

#[scenario(
    path = "tests/features/zero_copy_baseline.feature",
    name = "A pass-through middleware leaves the payload in place"
)]
fn pass_through_middleware_preserves_payload(
    zero_copy_baseline_world: Result<ZeroCopyBaselineWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}

#[scenario(
    path = "tests/features/zero_copy_baseline.feature",
    name = "A copying middleware is detected"
)]
fn copying_middleware_is_detected(
    zero_copy_baseline_world: Result<ZeroCopyBaselineWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}

#[scenario(
    path = "tests/features/zero_copy_baseline.feature",
    name = "Inbound decode copies the payload exactly once"
)]
fn inbound_decode_copies_once(
    zero_copy_baseline_world: Result<ZeroCopyBaselineWorld, TestError>,
) -> Result<(), TestError> {
    Ok(())
}
