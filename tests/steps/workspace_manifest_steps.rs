//! Step definitions for workspace-manifest behavioural tests.

use rstest_bdd_macros::{given, then};

use crate::fixtures::workspace_manifest::{TestResult, WorkspaceManifestWorld};

#[given("the repository workspace metadata is loaded")]
fn given_repository_workspace_metadata_is_loaded(
    workspace_manifest_world: &mut WorkspaceManifestWorld,
) -> TestResult {
    workspace_manifest_world.load()
}

#[then("the root Cargo manifest declares the staged hybrid workspace")]
fn then_root_manifest_declares_staged_hybrid_workspace(
    workspace_manifest_world: &mut WorkspaceManifestWorld,
) -> TestResult {
    workspace_manifest_world.verify_staged_hybrid_workspace_manifest()
}

#[then("the workspace metadata reports the root package as a workspace member")]
fn then_workspace_metadata_reports_workspace_member(
    workspace_manifest_world: &mut WorkspaceManifestWorld,
) -> TestResult {
    workspace_manifest_world.verify_root_is_workspace_member()
}

#[then("the workspace metadata reports the root package as the only default member")]
fn then_workspace_metadata_reports_only_default_member(
    workspace_manifest_world: &mut WorkspaceManifestWorld,
) -> TestResult {
    workspace_manifest_world.verify_root_is_only_default_member()
}

#[then("the workspace metadata does not include the verification crate yet")]
fn then_workspace_metadata_excludes_verification_crate(
    workspace_manifest_world: &mut WorkspaceManifestWorld,
) -> TestResult {
    workspace_manifest_world.verify_verification_crate_is_absent()
}
