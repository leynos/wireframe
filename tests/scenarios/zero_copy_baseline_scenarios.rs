//! Scenario tests for the zero-copy baseline gate.
//!
//! Each scenario binds one named case from
//! `tests/features/zero_copy_baseline.feature` to the step sequence in
//! `tests/steps/zero_copy_baseline_steps.rs`, whose fixture is
//! `ZeroCopyBaselineWorld` in `tests/fixtures/zero_copy_baseline.rs`. The
//! scenarios restate the allocator-level invariants behaviourally, so a
//! regression a maintainer would describe in user-facing terms still has a
//! failing test; the byte-level authority remains the
//! `pipeline_allocation_invariants` binary.

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
