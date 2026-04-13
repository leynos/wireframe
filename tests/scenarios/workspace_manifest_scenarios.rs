//! Scenario tests for workspace-manifest behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::workspace_manifest::{
    TestResult,
    WorkspaceManifestWorld,
    workspace_manifest_world,
};

fn assert_root_manifest_stages_hybrid_workspace(
    workspace_manifest_world: &mut WorkspaceManifestWorld,
) -> TestResult {
    workspace_manifest_world.load()?;
    workspace_manifest_world.verify_staged_hybrid_workspace_manifest()?;
    workspace_manifest_world.verify_root_is_workspace_member()?;
    workspace_manifest_world.verify_root_is_only_default_member()?;
    workspace_manifest_world.verify_verification_crate_is_absent()?;
    Ok(())
}

#[scenario(
    path = "tests/features/workspace_manifest.feature",
    name = "The root manifest stages the hybrid workspace conversion"
)]
fn root_manifest_stages_hybrid_workspace(workspace_manifest_world: WorkspaceManifestWorld) {
    let mut workspace_manifest_world = workspace_manifest_world;
    assert_root_manifest_stages_hybrid_workspace(&mut workspace_manifest_world)
        .expect("workspace manifest scenario should pass");
}
