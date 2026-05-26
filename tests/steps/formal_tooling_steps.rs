//! Step definitions for formal-tooling behavioural tests.

use rstest_bdd_macros::{given, then};

use crate::fixtures::formal_tooling::{FormalToolingWorld, TestResult};

#[given("the formal verification tooling metadata is loaded")]
fn given_formal_verification_tooling_metadata_is_loaded(
    formal_tooling_world: &mut FormalToolingWorld,
) -> TestResult {
    formal_tooling_world.load()
}

#[then("the metadata pins Kani, Verus, and rust-prover-tools")]
fn then_metadata_pins_kani_verus_and_rust_prover_tools(
    formal_tooling_world: &mut FormalToolingWorld,
) -> TestResult {
    formal_tooling_world.verify_tool_metadata_pins()
}

#[then("the Verus checksum manifest covers the configured Linux archive")]
fn then_verus_checksum_manifest_covers_configured_linux_archive(
    formal_tooling_world: &mut FormalToolingWorld,
) -> TestResult {
    formal_tooling_world.verify_verus_checksum_manifest()
}

#[then("the Makefile exposes the formal verification tool entry points")]
fn then_makefile_exposes_formal_verification_tool_entry_points(
    formal_tooling_world: &mut FormalToolingWorld,
) -> TestResult {
    formal_tooling_world.verify_makefile_targets()
}

#[then("the Makefile entry points delegate to prover-tools")]
fn then_makefile_entry_points_delegate_to_prover_tools(
    formal_tooling_world: &mut FormalToolingWorld,
) -> TestResult {
    formal_tooling_world.verify_makefile_targets_delegate_to_prover_tools()
}
