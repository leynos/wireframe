//! Fixture world for workspace-manifest behavioural scenarios.

use rstest::fixture;

use crate::workspace_manifest_support::{
    WorkspaceManifestResult as FixtureResult,
    cargo_metadata,
    root_manifest,
    root_package_id,
};

pub type TestResult = FixtureResult<()>;

/// BDD world holding the root manifest and `cargo metadata` output.
#[derive(Debug, Default)]
pub struct WorkspaceManifestWorld {
    manifest: Option<String>,
    metadata: Option<String>,
}

#[fixture]
pub fn workspace_manifest_world() -> WorkspaceManifestWorld {
    std::hint::black_box(());
    WorkspaceManifestWorld::default()
}

impl WorkspaceManifestWorld {
    /// Load the repository manifest and workspace metadata for later checks.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest cannot be read or `cargo metadata`
    /// fails.
    pub fn load(&mut self) -> TestResult {
        self.manifest = Some(root_manifest()?);
        self.metadata = Some(cargo_metadata()?);
        Ok(())
    }

    fn manifest(&self) -> Result<&str, String> {
        self.manifest
            .as_deref()
            .ok_or_else(|| "workspace manifest not loaded".to_owned())
    }

    fn metadata(&self) -> Result<&str, String> {
        self.metadata
            .as_deref()
            .ok_or_else(|| "workspace metadata not loaded".to_owned())
    }

    /// Verify the root manifest declares the staged hybrid workspace contract.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest is missing an expected workspace
    /// clause.
    pub fn verify_staged_hybrid_workspace_manifest(&self) -> TestResult {
        let manifest = self.manifest()?;
        for expected in [
            "[workspace]",
            "members = [\".\"]",
            "default-members = [\".\"]",
            "resolver = \"3\"",
        ] {
            if !manifest.contains(expected) {
                return Err(format!("expected `{expected}` in root Cargo.toml").into());
            }
        }
        Ok(())
    }

    /// Verify the root package remains part of the workspace membership.
    ///
    /// # Errors
    ///
    /// Returns an error when the metadata omits the root package.
    pub fn verify_root_is_workspace_member(&self) -> TestResult {
        let metadata = self.metadata()?;
        if !metadata.contains(&root_package_id()?) {
            return Err("workspace metadata did not include the root package".into());
        }
        Ok(())
    }

    /// Verify the root package is the only default workspace member.
    ///
    /// # Errors
    ///
    /// Returns an error when the metadata widens default-member coverage.
    pub fn verify_root_is_only_default_member(&self) -> TestResult {
        let metadata = self.metadata()?;
        let expected = format!("\"workspace_default_members\":[\"{}\"]", root_package_id()?);
        if !metadata.contains(&expected) {
            return Err(
                "workspace metadata did not keep the root package as the only default member"
                    .into(),
            );
        }
        Ok(())
    }

    /// Verify the staged rollout has not yet added the verification crate.
    ///
    /// # Errors
    ///
    /// Returns an error when the 10.1.2 crate appears too early or the helper
    /// crate unexpectedly disappears from Cargo metadata.
    pub fn verify_verification_crate_is_absent(&self) -> TestResult {
        let metadata = self.metadata()?;
        if metadata.contains("wireframe-verification") {
            return Err(
                "verification crate should not join the workspace until roadmap item 10.1.2".into(),
            );
        }
        if !metadata.contains("\"wireframe_testing\"") {
            return Err(
                "workspace metadata should continue to report the in-repository helper crate"
                    .into(),
            );
        }
        Ok(())
    }
}
