//! Scenario tests for formal-tooling behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::formal_tooling::{FormalToolingWorld, TestResult, formal_tooling_world};

fn assert_repository_declares_pinned_prover_tooling(
    formal_tooling_world: &mut FormalToolingWorld,
) -> TestResult {
    formal_tooling_world.load()?;
    formal_tooling_world.verify_tool_metadata_pins()?;
    formal_tooling_world.verify_verus_checksum_manifest()?;
    formal_tooling_world.verify_makefile_targets()?;
    formal_tooling_world.verify_makefile_targets_delegate_to_prover_tools()?;
    Ok(())
}

#[scenario(
    path = "tests/features/formal_tooling.feature",
    name = "The repository declares pinned prover tooling"
)]
fn repository_declares_pinned_prover_tooling(formal_tooling_world: FormalToolingWorld) {
    let mut formal_tooling_world = formal_tooling_world;
    assert_repository_declares_pinned_prover_tooling(&mut formal_tooling_world)
        .expect("formal tooling scenario should pass");
}
