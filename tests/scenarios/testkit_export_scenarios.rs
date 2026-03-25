//! Scenario tests for `wireframe::testkit` export behaviour.

use rstest_bdd_macros::scenario;

use crate::fixtures::testkit_export::*;

#[scenario(
    path = "tests/features/testkit_export.feature",
    name = "Chunked partial-frame driving is available from the root crate"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn root_testkit_partial_frame_export(testkit_export_world: TestkitExportWorld) {}

#[scenario(
    path = "tests/features/testkit_export.feature",
    name = "Reassembly assertions are available from the root crate"
)]
#[expect(
    unused_variables,
    reason = "rstest-bdd wires steps via parameters without using them directly"
)]
fn root_testkit_reassembly_export(testkit_export_world: TestkitExportWorld) {}
