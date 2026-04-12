//! Scenario tests for workspace-manifest behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::workspace_manifest::*;

#[scenario(
    path = "tests/features/workspace_manifest.feature",
    name = "The root manifest stages the hybrid workspace conversion"
)]
fn root_manifest_stages_hybrid_workspace(workspace_manifest_world: WorkspaceManifestWorld) {
    let _ = workspace_manifest_world;
}
